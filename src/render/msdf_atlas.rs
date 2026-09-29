//! Glyph MSDF atlas: lazily generates a multi-channel signed distance field for
//! each glyph on first sighting, packs it into a single CPU-side RGBA8 buffer with
//! a shelf packer, and hands out uv rects + placement metrics. The GPU texture is
//! owned by [`crate::render::UiRenderer`] (mirroring how [`super::atlas::SpriteAtlas`]
//! is driven) and re-uploaded from [`MsdfGlyphAtlas::build_pixel_buffer`] whenever
//! [`MsdfGlyphAtlas::take_dirty`] reports a change.
//!
//! ## Why a separate atlas from `SpriteAtlas`
//!
//! * **Format must be linear, not sRGB.** MSDF texels are distances, not colors —
//!   sampling them through an sRGB-decoding view would warp the field. The renderer
//!   creates this atlas's texture as `Rgba8Unorm` (linear) with `FilterMode::Linear`
//!   (MSDF *requires* bilinear).
//! * **No edge-replication halo.** A glyph's MSDF already carries a saturated
//!   "outside" margin (the padding baked in by [`super::glyph_msdf`]); a plain 1px
//!   zero gutter between tiles is enough to stop cross-tile bilinear bleed, since
//!   the gutter value (far-outside) matches the tile edge.
//!
//! ## Lazy generation & caching
//!
//! [`MsdfGlyphAtlas::glyph`] is the only entry point. It caches every lookup —
//! including misses (outline-less glyphs like space) as `None` — so generation
//! happens exactly once per `(font, glyph)` and never on a frame's hot path. Callers
//! should pre-warm the printable-ASCII set at init to avoid first-sighting hitches.

use std::collections::HashMap;

use ttf_parser::{Face, GlyphId};

use super::atlas::AtlasRegion;
use super::glyph_msdf::{
    GlyphMetrics, HINTED_OVERSAMPLE, generate_glyph_msdf, generate_hinted_glyph_msdf,
};

/// Initial atlas dimensions.
pub(crate) const INITIAL_MSDF_ATLAS_SIZE: u32 = 1024;
/// Maximum atlas dimensions before allocation fails.
pub(crate) const MAX_MSDF_ATLAS_SIZE: u32 = 4096;
/// Zero gutter (in pixels) reserved on each side of a tile to prevent cross-tile
/// bilinear bleed. No replication needed — see module docs.
const GLYPH_GUTTER: u32 = 1;

/// Default reference EM size (pixels) the distance fields are generated at. Higher
/// = crisper at large display sizes, more atlas space.
///
/// Raising this does **not** fix soft UI text, despite the intuition that small
/// text is starved of texels. Measured on the gallery, 40 → 64 (with `px_range`
/// raised to match, so effect reach held constant) left the proportion of
/// mid-tone text pixels unchanged and narrowed horizontal edges by only ~6%, for
/// ~1.6x the tile area and ~20% more glyph generation time — a difference not
/// visible at 6x zoom. Softness at 11-14px is a *positioning* problem (stems
/// landing between pixel columns), not a sampling-rate one; see
/// [`GlyphSnap`](crate::GlyphSnap).
pub const DEFAULT_REF_PX: f32 = 40.0;
/// Default distance-ramp width (tile pixels). The shader scales screen-space AA by
/// this; also sets the tile padding and — crucially — the *effect reach*: outlines
/// and shadow/glow blur are only valid within `~(px_range/2) * (font_size/ref_px)`
/// screen px of the edge. Reach depends on the *ratio*, so this must track
/// [`DEFAULT_REF_PX`]: `12 / 40` gives ~1.9px reach at a 16px UI font (and ~5.5px
/// at 40px), enough for 1–2px outlines, soft shadows, and small glow halos without
/// bloating tiles. Fill AA is unaffected by this value (it's computed in screen space).
pub const DEFAULT_PX_RANGE: f32 = 12.0;

/// A glyph's location and placement, handed back from [`MsdfGlyphAtlas::glyph`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphTile {
    /// Pixel rect of the tile content within the atlas.
    pub region: AtlasRegion,
    /// EM-space placement metrics (see [`GlyphMetrics`]).
    pub metrics: GlyphMetrics,
    /// Texels per EM this tile's field was generated at.
    ///
    /// Per tile rather than per atlas because hinted entries are generated at a
    /// multiple of the size they are hinted for, so they differ from each other
    /// and from [`MsdfGlyphAtlas::ref_px`]. Effect clamping needs the tile's own
    /// value — see `field_reach` in `crate::text`.
    pub ref_px: f32,
}

/// Whether a glyph's field is generated size-independently or hinted for one
/// specific device pixel size.
///
/// A distance field scales to any size; hinting does not, because it moves
/// control points onto one pixel grid. So hinted entries are keyed by their
/// size, and a UI with four text sizes holds four entries per glyph.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum GlyphSizing {
    /// One size-independent entry, unhinted. Used for icons, for text above
    /// [`MAX_HINTED_PX`], and whenever hinting is turned off or unavailable.
    #[default]
    Scalable,
    /// Hinted and generated for this **device** pixel EM size.
    Hinted(u16),
}

/// Largest device pixel size text is hinted at.
///
/// Above this, grid fitting stops paying for itself: a stem is several pixels
/// wide, so a half-pixel misalignment is no longer visible, while the per-size
/// tiles keep costing atlas space and generation time. Large text also tends to
/// carry the wide outlines and glows whose reach a hinted tile's higher
/// texels-per-EM would shrink.
pub const MAX_HINTED_PX: u16 = 20;

#[derive(Clone)]
struct Shelf {
    y: u32,
    /// Cell height (tile height + 2 * GLYPH_GUTTER).
    height: u32,
    cursor_x: u32,
}

#[derive(Clone)]
struct StoredGlyph {
    region: AtlasRegion,
    metrics: GlyphMetrics,
    /// Texels per EM this entry was generated at (see [`GlyphTile::ref_px`]).
    ref_px: f32,
    /// RGBA8 pixels (region.w * region.h * 4). MSDF RGB with alpha forced to 255.
    rgba: Vec<u8>,
}

/// CPU-side glyph MSDF atlas. The GPU texture is owned and uploaded by the renderer.
pub struct MsdfGlyphAtlas {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    glyphs: Vec<StoredGlyph>,
    /// `(font_id, glyph_id, sizing)` → stored-glyph index, or `None` for
    /// outline-less glyphs (whitespace) and generation failures. Caches misses so
    /// we never retry — including a font whose hinting `skrifa` declines, which
    /// would otherwise re-parse on every frame.
    lookup: HashMap<(u64, u16, GlyphSizing), Option<u32>>,
    dirty: bool,
    ref_px: f32,
    px_range: f32,
}

impl MsdfGlyphAtlas {
    /// Create an empty atlas using the default reference size and ramp width.
    pub fn new() -> Self {
        Self::with_params(DEFAULT_REF_PX, DEFAULT_PX_RANGE)
    }

    /// Create an empty atlas generating fields at `ref_px` with a `px_range`-wide
    /// distance ramp (see [`DEFAULT_REF_PX`] / [`DEFAULT_PX_RANGE`]).
    pub fn with_params(ref_px: f32, px_range: f32) -> Self {
        Self {
            width: INITIAL_MSDF_ATLAS_SIZE,
            height: INITIAL_MSDF_ATLAS_SIZE,
            shelves: Vec::new(),
            next_shelf_y: 0,
            glyphs: Vec::new(),
            lookup: HashMap::new(),
            dirty: true,
            ref_px,
            px_range,
        }
    }

    /// Current atlas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Current atlas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The distance-ramp width (tile pixels) the fields were generated with. The
    /// shader needs this to scale screen-space AA: `screen_px_range = px_range *
    /// font_size / ref_px`.
    pub fn px_range(&self) -> f32 {
        self.px_range
    }

    /// The reference EM size (pixels) the fields were generated at.
    pub fn ref_px(&self) -> f32 {
        self.ref_px
    }

    /// Look up (and lazily generate) the MSDF tile for a glyph.
    ///
    /// * `font_id` — a stable per-font key (the caller maps cosmic-text's
    ///   `fontdb::ID` to this).
    /// * `glyph_id` — the shaped glyph index.
    /// * `font_data` — the raw font face bytes (`cosmic_text::Font::data()`); only
    ///   touched on a cache miss.
    ///
    /// * `sizing` — [`GlyphSizing::Scalable`] for one shared size-independent
    ///   entry, or [`GlyphSizing::Hinted`] to grid-fit the outline for a device
    ///   pixel size first. A hinted request whose font `skrifa` cannot hint
    ///   silently falls back to a scalable field of the same size, so callers
    ///   never have to handle that case.
    ///
    /// Returns `None` for outline-less glyphs (whitespace) — the caller advances the
    /// pen without emitting a quad. Generation happens at most once per `(font_id,
    /// glyph_id, sizing)`.
    pub fn glyph(
        &mut self,
        font_id: u64,
        glyph_id: u16,
        sizing: GlyphSizing,
        font_data: &[u8],
    ) -> Option<GlyphTile> {
        if let Some(cached) = self.lookup.get(&(font_id, glyph_id, sizing)) {
            return cached.map(|idx| self.tile(idx));
        }

        let hinted = match sizing {
            GlyphSizing::Hinted(size_px) => {
                generate_hinted_glyph_msdf(font_data, glyph_id, size_px as f32, self.px_range)
                    .map(|g| (g, size_px as f32 * HINTED_OVERSAMPLE))
            }
            GlyphSizing::Scalable => None,
        };
        let generated = hinted.or_else(|| {
            Face::parse(font_data, 0)
                .ok()
                .and_then(|face| {
                    generate_glyph_msdf(&face, GlyphId(glyph_id), self.ref_px, self.px_range)
                })
                .map(|g| (g, self.ref_px))
        });

        match generated {
            Some((g, ref_px)) => {
                let region = self.pack(g.metrics.width_px, g.metrics.height_px);
                let idx = self.glyphs.len() as u32;
                self.glyphs.push(StoredGlyph {
                    region,
                    metrics: g.metrics,
                    ref_px,
                    rgba: rgb_to_rgba(&g.image),
                });
                self.dirty = true;
                self.lookup.insert((font_id, glyph_id, sizing), Some(idx));
                Some(GlyphTile {
                    region,
                    metrics: g.metrics,
                    ref_px,
                })
            }
            None => {
                self.lookup.insert((font_id, glyph_id, sizing), None);
                None
            }
        }
    }

    /// What [`glyph`](Self::glyph) would return, if the glyph was already
    /// looked up: `Some(None)` for an outline-less one, `None` when it hasn't
    /// been generated yet and needs its font data.
    pub fn cached(
        &self,
        font_id: u64,
        glyph_id: u16,
        sizing: GlyphSizing,
    ) -> Option<Option<GlyphTile>> {
        self.lookup
            .get(&(font_id, glyph_id, sizing))
            .map(|cached| cached.map(|idx| self.tile(idx)))
    }

    fn tile(&self, idx: u32) -> GlyphTile {
        let g = &self.glyphs[idx as usize];
        GlyphTile {
            region: g.region,
            metrics: g.metrics,
            ref_px: g.ref_px,
        }
    }

    /// Take the dirty flag; returns whether a GPU re-upload is needed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// Render the full atlas as a single packed RGBA8 buffer of `width*height*4`.
    /// Tiles are written into their content rect; gutters stay zero (far-outside),
    /// which is the correct neutral value for MSDF bilinear sampling.
    pub fn build_pixel_buffer(&self) -> Vec<u8> {
        let mut buf = vec![0u8; (self.width * self.height * 4) as usize];
        let stride = (self.width * 4) as usize;
        for g in &self.glyphs {
            let r = g.region;
            let row_bytes = (r.w * 4) as usize;
            for row in 0..r.h {
                let src_off = (row * r.w * 4) as usize;
                let dst_off = ((r.y + row) as usize) * stride + (r.x as usize) * 4;
                buf[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&g.rgba[src_off..src_off + row_bytes]);
            }
        }
        buf
    }

    fn pack(&mut self, w: u32, h: u32) -> AtlasRegion {
        loop {
            if let Some(r) = self.try_place(w, h) {
                return r;
            }
            if !self.try_grow() {
                panic!(
                    "glyph {}x{} doesn't fit in MSDF atlas at max size {}",
                    w, h, MAX_MSDF_ATLAS_SIZE
                );
            }
        }
    }

    fn try_place(&mut self, w: u32, h: u32) -> Option<AtlasRegion> {
        let cell_w = w + 2 * GLYPH_GUTTER;
        let cell_h = h + 2 * GLYPH_GUTTER;
        if cell_w > self.width {
            return None;
        }
        for shelf in &mut self.shelves {
            if shelf.height >= cell_h && shelf.cursor_x + cell_w <= self.width {
                let region = AtlasRegion {
                    x: shelf.cursor_x + GLYPH_GUTTER,
                    y: shelf.y + GLYPH_GUTTER,
                    w,
                    h,
                };
                shelf.cursor_x += cell_w;
                return Some(region);
            }
        }
        if self.next_shelf_y + cell_h <= self.height {
            let shelf = Shelf {
                y: self.next_shelf_y,
                height: cell_h,
                cursor_x: cell_w,
            };
            let region = AtlasRegion {
                x: GLYPH_GUTTER,
                y: shelf.y + GLYPH_GUTTER,
                w,
                h,
            };
            self.next_shelf_y += cell_h;
            self.shelves.push(shelf);
            return Some(region);
        }
        None
    }

    fn try_grow(&mut self) -> bool {
        let new_size = (self.width.max(self.height) * 2).min(MAX_MSDF_ATLAS_SIZE);
        if new_size == self.width && new_size == self.height {
            return false;
        }
        self.width = new_size;
        self.height = new_size;
        self.dirty = true;
        true
    }
}

impl Default for MsdfGlyphAtlas {
    fn default() -> Self {
        Self::new()
    }
}

/// Expand an MSDF `RgbImage` to RGBA8 with alpha forced to 255.
fn rgb_to_rgba(img: &image::RgbImage) -> Vec<u8> {
    let mut out = Vec::with_capacity((img.width() * img.height() * 4) as usize);
    for p in img.pixels() {
        out.extend_from_slice(&[p.0[0], p.0[1], p.0[2], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf");

    /// Hinting multiplies atlas entries by the number of distinct text sizes, so
    /// the thing that could quietly break is capacity. A realistic UI — printable
    /// ASCII at four sizes — costs one growth step, to 2048² (16 MiB), and must
    /// stay well clear of [`MAX_MSDF_ATLAS_SIZE`] so real text with accents and
    /// punctuation still has room.
    #[test]
    fn a_full_ascii_set_at_every_ui_size_fits_one_atlas_page() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let sizes = [10u16, 11, 13, 16];

        let mut tiles = 0;
        for code in 0x21u8..=0x7e {
            let Some(g) = face.glyph_index(code as char) else {
                continue;
            };
            for size in sizes {
                if atlas
                    .glyph(1, g.0, GlyphSizing::Hinted(size), FONT)
                    .is_some()
                {
                    tiles += 1;
                }
            }
        }

        assert_eq!(tiles, 94 * sizes.len(), "every printable ASCII glyph packs");
        assert!(
            atlas.width() <= 2048 && atlas.height() <= 2048,
            "four sizes of ASCII grew the atlas to {}x{}, past the 2048 budget",
            atlas.width(),
            atlas.height()
        );
    }

    #[test]
    fn a_hinted_glyph_is_a_separate_entry_per_size() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;

        let scalable = atlas
            .glyph(1, a, GlyphSizing::Scalable, FONT)
            .expect("scalable tile");
        let at_11 = atlas
            .glyph(1, a, GlyphSizing::Hinted(11), FONT)
            .expect("11px tile");
        let at_13 = atlas
            .glyph(1, a, GlyphSizing::Hinted(13), FONT)
            .expect("13px tile");

        // Hinting is fitted to one pixel grid, so each size needs its own field;
        // sharing one would put the fit on the wrong grid at every other size.
        for (x, y) in [
            (scalable.region, at_11.region),
            (scalable.region, at_13.region),
            (at_11.region, at_13.region),
        ] {
            assert_ne!(x, y, "each sizing must pack its own tile");
        }
        assert_eq!(atlas.glyphs.len(), 3);

        // Re-asking is cached, not repacked.
        let again = atlas.glyph(1, a, GlyphSizing::Hinted(11), FONT).unwrap();
        assert_eq!(again, at_11);
        assert_eq!(atlas.glyphs.len(), 3);
    }

    #[test]
    fn a_hinted_tile_reports_the_resolution_it_was_generated_at() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;

        // Effect reach is derived from the tile's own texels-per-EM, so a hinted
        // tile has to carry its own rather than the atlas-wide default.
        let scalable = atlas.glyph(1, a, GlyphSizing::Scalable, FONT).unwrap();
        assert_eq!(scalable.ref_px, atlas.ref_px());

        let hinted = atlas.glyph(1, a, GlyphSizing::Hinted(11), FONT).unwrap();
        assert_eq!(hinted.ref_px, 11.0 * HINTED_OVERSAMPLE);
    }

    #[test]
    fn hinting_moves_the_outline_without_changing_its_em_scale() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let x = face.glyph_index('x').unwrap().0;

        let scalable = atlas.glyph(1, x, GlyphSizing::Scalable, FONT).unwrap();
        let hinted = atlas.glyph(1, x, GlyphSizing::Hinted(11), FONT).unwrap();

        // Both describe the same glyph in EM fractions, so the quad the renderer
        // builds stays roughly the same box — hinting nudges edges onto the pixel
        // grid, it does not rescale the glyph. A whole pixel at 11px is ~0.09 EM,
        // and the fit moves edges by at most half of that per side.
        for (a, b, what) in [
            (scalable.metrics.left_em, hinted.metrics.left_em, "left"),
            (scalable.metrics.right_em, hinted.metrics.right_em, "right"),
            (scalable.metrics.top_em, hinted.metrics.top_em, "top"),
            (
                scalable.metrics.bottom_em,
                hinted.metrics.bottom_em,
                "bottom",
            ),
        ] {
            assert!(
                (a - b).abs() < 0.15,
                "{what}_em moved {a} -> {b}, more than grid fitting can explain"
            );
        }
    }

    /// The whole point: hinting should pull the glyph's horizontal edges onto
    /// whole pixels. Measured on the tile itself, the top and bottom of a hinted
    /// `x` should sit closer to a pixel boundary than the unhinted one's.
    #[test]
    fn hinting_lands_horizontal_edges_nearer_the_pixel_grid() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let size = 11.0f32;

        // `x` has a flat top and bottom at the x-height and baseline — the two
        // zones an autohinter aligns first.
        let g = face.glyph_index('x').unwrap().0;
        let scalable = atlas.glyph(1, g, GlyphSizing::Scalable, FONT).unwrap();
        let hinted = atlas.glyph(1, g, GlyphSizing::Hinted(11), FONT).unwrap();

        // Tile bounds include the SDF padding, and that margin is a whole number
        // of *tile* pixels — which is a fraction of a display pixel, different
        // per tile because the two have different texels-per-EM. Subtract it, or
        // the measurement reads the padding's offset rather than the outline's.
        let padding_tile_px = DEFAULT_PX_RANGE.ceil() + 1.0;
        let off_grid = |edge_em: f32, ref_px: f32, toward_ink: f32| {
            let ink_px = edge_em * size - toward_ink * padding_tile_px * size / ref_px;
            (ink_px - ink_px.round()).abs()
        };
        // `top_em` measures up from the baseline and `bottom_em` down, so the
        // padding is subtracted from the top and added back at the bottom.
        let edges = |t: &GlyphTile| {
            off_grid(t.metrics.top_em, t.ref_px, 1.0)
                + off_grid(t.metrics.bottom_em, t.ref_px, -1.0)
        };
        let before = edges(&scalable);
        let after = edges(&hinted);
        // The x-height and baseline are the first two zones an autohinter fits,
        // so both should land essentially exactly on a pixel row.
        assert!(
            after < 0.05,
            "hinted x sits {after:.3}px off the pixel grid; grid fitting did not take"
        );
        assert!(
            before > after,
            "unhinted {before:.3}px should be further off the grid than hinted {after:.3}px"
        );
    }

    #[test]
    fn glyph_generates_caches_and_packs() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;

        let t1 = atlas
            .glyph(1, a, GlyphSizing::Scalable, FONT)
            .expect("A tile");
        assert!(t1.region.w > 0 && t1.region.h > 0);
        // Second lookup is cached — same region, no new stored glyph.
        let before = atlas.glyphs.len();
        let t2 = atlas
            .glyph(1, a, GlyphSizing::Scalable, FONT)
            .expect("A tile cached");
        assert_eq!(t1, t2);
        assert_eq!(atlas.glyphs.len(), before, "cached lookup must not re-pack");
    }

    #[test]
    fn whitespace_is_cached_as_miss() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let space = face.glyph_index(' ').unwrap().0;
        assert!(atlas.glyph(1, space, GlyphSizing::Scalable, FONT).is_none());
        // Cached as a miss — no stored glyph, and a repeat is still None.
        assert_eq!(atlas.glyphs.len(), 0);
        assert!(atlas.glyph(1, space, GlyphSizing::Scalable, FONT).is_none());
        assert_eq!(atlas.lookup.len(), 1);
    }

    #[test]
    fn distinct_fonts_keyed_separately() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;
        let t_font1 = atlas.glyph(1, a, GlyphSizing::Scalable, FONT).unwrap();
        let t_font2 = atlas.glyph(2, a, GlyphSizing::Scalable, FONT).unwrap();
        // Same glyph id, different font key → two separate tiles.
        assert_ne!(t_font1.region, t_font2.region);
        assert_eq!(atlas.glyphs.len(), 2);
    }

    #[test]
    fn tiles_do_not_overlap_including_gutter() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let mut regions = Vec::new();
        for code in 0x21u8..=0x7e {
            let c = code as char;
            if let Some(g) = face.glyph_index(c)
                && let Some(t) = atlas.glyph(1, g.0, GlyphSizing::Scalable, FONT)
            {
                regions.push(t.region);
            }
        }
        assert!(regions.len() > 90);
        for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                let a = regions[i];
                let b = regions[j];
                let overlap_x = a.x < b.x + b.w && b.x < a.x + a.w;
                let overlap_y = a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(
                    !(overlap_x && overlap_y),
                    "tiles {i} and {j} overlap: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn pixel_buffer_matches_atlas_size() {
        let mut atlas = MsdfGlyphAtlas::new();
        let face = Face::parse(FONT, 0).unwrap();
        let a = face.glyph_index('A').unwrap().0;
        atlas.glyph(1, a, GlyphSizing::Scalable, FONT);
        let buf = atlas.build_pixel_buffer();
        assert_eq!(buf.len(), (atlas.width() * atlas.height() * 4) as usize);
        assert!(atlas.take_dirty());
        // Dirty consumed.
        assert!(!atlas.take_dirty());
    }
}
