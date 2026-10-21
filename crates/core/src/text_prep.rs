//! The CPU half of drawing text and MSDF icons: placing every glyph into the
//! atlases, then building every unit's vertices against the atlases' final
//! size. Nothing here needs a GPU, so a list's text can be prepared, and its
//! draw boxes taken, in a plain test. [`TextRenderer`] mirrors the atlases to
//! the GPU and uploads the vertices kept in [`MsdfPrepared`].
//!
//! A unit (`docs/design/draw-call-batching.md`, "New: units") is what draws
//! as one: a text command whose blocks have a shadow or glow draws all its
//! shadows, then all its glows, then all its fills, so it is one unit; without
//! those effects the sweeps reduce to one fill pass in block order, and each
//! block is a unit of its own.

use std::ops::Range;

use super::*;

/// The glyph and icon atlases as pixels, the font keys and the font system:
/// everything preparing a list's text needs, with no device.
pub(crate) struct MsdfCpu {
    pub(super) font_system: FontSystemHandle,
    pub(super) atlas: MsdfGlyphAtlas,
    #[cfg(feature = "phosphor-icons")]
    pub(super) icon_atlas: MsdfGlyphAtlas,
    /// Stable per-font keys for the atlas, assigned on first sighting. Decouples
    /// the atlas from cosmic-text's `fontdb::ID`.
    font_keys: HashMap<fontdb::ID, u64>,
    next_font_key: u64,
    /// The registered icon fonts' bytes, refreshed per list.
    #[cfg(feature = "phosphor-icons")]
    icon_fonts: Vec<&'static [u8]>,
}

/// Blocks of text that draw together, and their vertices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextUnit {
    /// Into the list's text blocks.
    pub blocks: Range<u32>,
    /// Into [`MsdfPrepared::text_vertices`].
    pub vertices: Range<u32>,
}

/// A list's text and icons as prepared for drawing, kept until they are
/// uploaded. Reused from list to list: cleared, never freed.
#[derive(Default)]
pub(crate) struct MsdfPrepared {
    placements: Vec<GlyphPlacement>,
    /// Each text block's glyphs in `placements`.
    block_glyphs: Vec<Range<u32>>,
    /// Every text unit's vertices, one unit after another.
    pub(crate) text_vertices: Vec<MsdfVertex>,
    /// Each text block's vertices from its shadow, glow and fill sweeps, in
    /// `text_vertices`; empty where a sweep drew nothing.
    pub(crate) block_sweeps: Vec<[Range<u32>; 3]>,
    /// The text units, in paint order.
    pub(crate) text_units: Vec<TextUnit>,
    /// Each text command's vertices, in paint order: its units, one after
    /// another.
    pub(crate) text_commands: Vec<Range<u32>>,
    /// Every drawn icon's six vertices, in icon order.
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icon_vertices: Vec<MsdfVertex>,
    /// Icon `i`'s vertices are `icon_starts[i]..icon_starts[i + 1]`; none for
    /// an icon that doesn't draw.
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icon_starts: Vec<u32>,
}

impl MsdfPrepared {
    /// Block `block`'s vertices from each of its sweeps.
    pub(crate) fn block_vertices(&self, block: usize) -> impl Iterator<Item = &[MsdfVertex]> {
        self.block_sweeps[block]
            .iter()
            .map(|range| &self.text_vertices[range.start as usize..range.end as usize])
    }

    /// The vertices of icons `icons`.
    #[cfg(feature = "phosphor-icons")]
    pub(crate) fn icon_range(&self, icons: Range<u32>) -> Range<u32> {
        self.icon_starts[icons.start as usize]..self.icon_starts[icons.end as usize]
    }
}

/// The box a glyph of `block` has to reach into to show: its clip, widened
/// by how far a glyph draws past its line box and cell (ink above the line or
/// left of the pen, an italic's overhang) and by the block's effects. `None`
/// for a block without a clip, all of which can show.
pub(super) fn cull_bounds(block: &TextBlock) -> Option<[f32; 4]> {
    let clip = block.clip?;
    let mut reach = block.font_size.max(block.line_height);
    if let Some(outline) = &block.outline {
        reach += outline.width_px;
    }
    if let Some(shadow) = &block.shadow {
        reach += shadow.offset[0].abs().max(shadow.offset[1].abs()) + shadow.softness;
    }
    if let Some(glow) = &block.glow {
        reach += glow.radius_px;
    }
    Some([
        clip.x - reach,
        clip.y - reach,
        clip.x + clip.width + reach,
        clip.y + clip.height + reach,
    ])
}

/// The glyphs of `layout` on the lines whose box meets `bounds`
/// ([`cull_bounds`]). Lines run top to bottom, so the first and last are
/// found by search, and a long text clipped to a screenful places a
/// screenful. All of them without bounds, and for vertical text, whose
/// lines are its stacked clusters rather than rows.
pub(super) fn glyphs_in_clip(
    block: &TextBlock,
    layout: &crate::shaping::ShapedLayout,
    bounds: Option<[f32; 4]>,
) -> Range<usize> {
    let all = 0..layout.glyphs.len();
    let Some([_, top, _, bottom]) = bounds else {
        return all;
    };
    if block.vertical || layout.lines.is_empty() {
        return all;
    }
    let lines = &layout.lines;
    let first = lines.partition_point(|line| block.y + line.top + line.height < top);
    let end = lines.partition_point(|line| block.y + line.top <= bottom);
    if first >= end {
        return 0..0;
    }
    lines[first].glyphs.start as usize..lines[end - 1].glyphs.end as usize
}

impl MsdfCpu {
    pub(crate) fn new(font_system: FontSystemHandle) -> Self {
        Self {
            font_system,
            atlas: MsdfGlyphAtlas::new(),
            // Icons use the same MSDF machinery, generated at a higher
            // reference size (they render anywhere from ~16px steppers to
            // ~48px gallery cells) and the standard distance ramp, so the
            // shared shader's AA math is unchanged.
            #[cfg(feature = "phosphor-icons")]
            icon_atlas: MsdfGlyphAtlas::with_params(ICON_REF_PX, DEFAULT_PX_RANGE),
            font_keys: HashMap::new(),
            next_font_key: 0,
            #[cfg(feature = "phosphor-icons")]
            icon_fonts: Vec::new(),
        }
    }

    pub(super) fn font_key(&mut self, id: fontdb::ID) -> u64 {
        resolve_font_key(&mut self.font_keys, &mut self.next_font_key, id)
    }

    /// Prepare `texts` for drawing as `commands` (each a range of blocks drawn
    /// as one paint command), placed on `grid`. Every glyph goes into the
    /// atlas before any vertex is built: generating a glyph can grow the
    /// atlas, and a UV has to divide by its final size.
    pub(crate) fn prepare_texts(
        &mut self,
        out: &mut MsdfPrepared,
        grid: PixelGrid,
        texts: &[TextBlock],
        commands: impl IntoIterator<Item = Range<u32>>,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_shape").entered();

        out.placements.clear();
        out.block_glyphs.clear();
        out.text_vertices.clear();
        out.block_sweeps.clear();
        out.text_units.clear();
        out.text_commands.clear();

        // Place every block. The layouts come from the font system shared with
        // the measurers, which shapes through cosmic-text only for a layout it
        // doesn't keep — so a block measured for layout earlier this frame, or
        // drawn on an earlier one, is not shaped again.
        //
        // The font system stays locked for the whole pass, including drawing
        // new glyphs into the atlas (~1.5 ms each, once per glyph): a placed
        // glyph borrows its layout from the shared cache, and its outline
        // comes from the font system. Measuring and drawing run on one thread,
        // so nothing waits on it; a renderer on another thread sharing this
        // handle would wait for the pass.
        let fs_handle = Arc::clone(&self.font_system);
        let mut shared = fs_handle.lock().expect("FontSystem poisoned");
        for block in texts {
            let start = out.placements.len() as u32;
            if !block.content.is_empty() {
                let (layout, fs) =
                    shared.layout_and_fonts(&LayoutSpec::of_block(block), &block.content);
                let bounds = cull_bounds(block);
                append_placements(
                    &mut self.atlas,
                    &mut self.font_keys,
                    &mut self.next_font_key,
                    fs,
                    block,
                    &block.spans,
                    &layout.glyphs[glyphs_in_clip(block, layout, bounds)],
                    bounds,
                    grid,
                    &mut out.placements,
                );
            }
            out.block_glyphs.push(start..out.placements.len() as u32);
        }
        drop(shared);

        // Every unit's vertices, against the final atlas size.
        let sweep = Sweep {
            atlas_w: self.atlas.width(),
            atlas_h: self.atlas.height(),
            px_range: self.atlas.px_range(),
        };
        out.block_sweeps.resize(texts.len(), [0..0, 0..0, 0..0]);
        for blocks in commands {
            let start = out.text_vertices.len() as u32;
            let effects = texts[blocks.start as usize..blocks.end as usize]
                .iter()
                .any(|b| b.shadow.is_some() || b.glow.is_some());
            if effects {
                out.build_unit(&sweep, blocks);
            } else {
                for block in blocks {
                    out.build_unit(&sweep, block..block + 1);
                }
            }
            out.text_commands
                .push(start..out.text_vertices.len() as u32);
        }
    }

    /// Prepare `icons` for drawing. Like text, every icon's glyph goes into the
    /// icon atlas before any quad is built against its size.
    #[cfg(feature = "phosphor-icons")]
    pub(crate) fn prepare_icons(&mut self, out: &mut MsdfPrepared, icons: &[IconMsdf]) {
        out.icon_vertices.clear();
        out.icon_starts.clear();
        // One lock acquisition for the whole list; the bytes are `'static`, so
        // MSDF generation below runs with the registry unlocked.
        icon_font_snapshot(&mut self.icon_fonts);
        for icon in icons {
            // An icon from an unregistered font simply doesn't draw.
            if let Some(data) = self.icon_fonts.get(icon.glyph.font.index() as usize) {
                self.icon_atlas.glyph(
                    icon.glyph.font.index() as u64,
                    icon.glyph.glyph_id,
                    GlyphSizing::Scalable,
                    data,
                );
            }
        }
        let (aw, ah) = (self.icon_atlas.width(), self.icon_atlas.height());
        let px_range = self.icon_atlas.px_range();
        for icon in icons {
            out.icon_starts.push(out.icon_vertices.len() as u32);
            let tile = self.icon_atlas.cached(
                icon.glyph.font.index() as u64,
                icon.glyph.glyph_id,
                GlyphSizing::Scalable,
            );
            if let Some(Some(tile)) = tile {
                push_icon_quad(&mut out.icon_vertices, &tile, icon, aw, ah, px_range);
            }
        }
        out.icon_starts.push(out.icon_vertices.len() as u32);
    }
}

/// What every glyph quad of one list is built against.
struct Sweep {
    atlas_w: u32,
    atlas_h: u32,
    px_range: f32,
}

impl MsdfPrepared {
    /// Build the unit of blocks `blocks`: every shadow, then every glow, then
    /// every fill (and outline), so each glyph's shadow and glow sit behind
    /// all the unit's fills.
    fn build_unit(&mut self, sweep: &Sweep, blocks: Range<u32>) {
        let start = self.text_vertices.len() as u32;
        for kind in 0..3 {
            for block in blocks.clone() {
                let from = self.text_vertices.len() as u32;
                let glyphs = self.block_glyphs[block as usize].clone();
                for p in &self.placements[glyphs.start as usize..glyphs.end as usize] {
                    push_sweep_quad(&mut self.text_vertices, sweep, p, kind);
                }
                self.block_sweeps[block as usize][kind] = from..self.text_vertices.len() as u32;
            }
        }
        self.text_units.push(TextUnit {
            blocks,
            vertices: start..self.text_vertices.len() as u32,
        });
    }
}

/// One glyph's quad for sweep `kind`: 0 its shadow, 1 its glow, 2 its fill
/// and outline. Nothing for a shadow or glow the glyph doesn't have.
fn push_sweep_quad(out: &mut Vec<MsdfVertex>, sweep: &Sweep, p: &GlyphPlacement, kind: usize) {
    let (aw, ah, px_range) = (sweep.atlas_w, sweep.atlas_h, sweep.px_range);
    // Effects are capped to the field's reach, so they follow the glyph
    // instead of filling its tile.
    let safe = field_reach(p.font_size, px_range, p.tile.ref_px);
    let style = match kind {
        0 => {
            let Some((color, offset, softness)) = p.shadow else {
                return;
            };
            // A shadow is a fill with widened AA.
            QuadStyle {
                fill: color,
                outline: [0.0; 4],
                outline_width: 0.0,
                softness: softness.min(safe),
                offset,
            }
        }
        1 => {
            let Some((color, radius)) = p.glow else {
                return;
            };
            // The glow is a grown, soft, fill-less halo; its band (width +
            // softness) stays within the field's reach (graceful degradation
            // at small sizes).
            let radius = radius.min(safe / 1.5);
            let softness = (radius * 0.5).max(0.5).min((safe - radius).max(0.0));
            QuadStyle {
                fill: [0.0; 4],
                outline: color,
                outline_width: radius,
                softness,
                offset: [0.0, 0.0],
            }
        }
        _ => {
            let (outline, outline_width) = p.outline.unwrap_or(([0.0; 4], 0.0));
            QuadStyle {
                fill: p.fill,
                outline,
                outline_width: outline_width.min(safe),
                softness: 0.0,
                offset: [0.0, 0.0],
            }
        }
    };
    push_glyph_quad(out, p, aw, ah, px_range, &style);
}

#[cfg(all(test, feature = "bundled-font"))]
#[path = "text_prep_tests.rs"]
mod tests;
