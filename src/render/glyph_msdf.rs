//! MSDF (multi-channel signed distance field) generation for a single glyph.
//!
//! This is the **swappable generator seam**. Everything that knows about `fdsm`
//! lives here behind one function — [`generate_glyph_msdf`] — so that if `fdsm`
//! ever produces artifacts on real strings we can swap in the `msdf` C++ binding
//! crate or a hand-rolled generator by rewriting this one file, with no churn in
//! the atlas or render layers.
//!
//! ## Pipeline
//!
//! 1. `fdsm_ttf_parser::load_shape_from_face` lifts the glyph outline into an
//!    `fdsm::shape::Shape<Contour>` in **font units** (Y-up, origin at the glyph
//!    pen position on the baseline).
//! 2. We build an affine that maps font units → **tile pixels** (Y-down, with a
//!    `padding` margin around the glyph bbox so the distance ramp doesn't clip at
//!    the tile edge) and apply it to the shape via `fdsm::transform::Transform`.
//! 3. The outline is cleaned up for fdsm, which (unlike msdfgen) takes it as
//!    it comes: points are snapped to a fine grid so corner ties are exact,
//!    zero-length segments and hairpin reversals are removed, and contours are
//!    turned to one winding direction. Each step's doc says what it prevents;
//!    Phosphor's outlines need all of them.
//! 4. `edge_coloring_simple` assigns R/G/B channels to edges, `.prepare()` builds
//!    the acceleration structure, `generate_msdf` fills an f32 image,
//!    `correct_error_msdf` evens out texels whose channels would interpolate to
//!    ink off the outline, and `correct_sign_msdf` fixes the inside/outside sign
//!    (median > 0.5 == inside) before the field is quantised to RGB8.
//!
//! The returned [`GlyphMetrics`] carries the tile's bounds in **EM fractions**
//! (units of `font_size`) relative to the pen-on-baseline origin, x rightward and
//! y upward, so the render layer can place a quad at any `font_size`:
//!
//! ```text
//! x_left   = pen_x    + left_em   * font_size
//! x_right  = pen_x    + right_em  * font_size
//! y_top    = baseline - top_em    * font_size   // top_em > 0 (above baseline)
//! y_bottom = baseline - bottom_em * font_size   // bottom_em < 0 for descenders
//! ```
//!
//! These bounds INCLUDE the SDF padding, so the quad covers the whole tile and the
//! uv rect maps 1:1.

use fdsm::bezier::scanline::FillRule;
use fdsm::bezier::{Order, Point, Segment, Vect};
use fdsm::correct_error::{ErrorCorrectionConfig, correct_error_msdf};
use fdsm::generate::generate_msdf;
use fdsm::render::correct_sign_msdf;
use fdsm::shape::{ColoredContour, Contour, Shape};
use fdsm::transform::Transform;
use image::{Rgb32FImage, RgbImage};
use nalgebra::{Affine2, Matrix3};
use ttf_parser::{Face, GlyphId};

/// Sine of the edge-coloring angle threshold (3°, matching msdfgen's default).
/// Edges meeting at a sharper corner than this get distinct color channels.
const EDGE_COLORING_SIN_ALPHA: f64 = 0.052_335_956; // (3°).to_radians().sin()
/// Deterministic seed for `edge_coloring_simple` so generation is reproducible
/// (important for tests and for stable atlas contents across runs).
const EDGE_COLORING_SEED: u64 = 0;

/// Placement metrics for a generated glyph tile, in EM fractions (units of
/// `font_size`) relative to the pen-on-baseline origin. x rightward, y **upward**
/// (font convention). Bounds include the SDF padding margin, so the quad they
/// describe covers the full tile.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlyphMetrics {
    /// Tile width in pixels (at the generation reference size).
    pub width_px: u32,
    /// Tile height in pixels (at the generation reference size).
    pub height_px: u32,
    /// Left edge of the tile, EM fraction (typically slightly negative due to padding/bearing).
    pub left_em: f32,
    /// Right edge of the tile, EM fraction.
    pub right_em: f32,
    /// Top edge of the tile, EM fraction (positive == above baseline).
    pub top_em: f32,
    /// Bottom edge of the tile, EM fraction (negative for descenders).
    pub bottom_em: f32,
}

/// A generated glyph: its MSDF tile (RGB8) plus placement metrics.
pub struct GlyphMsdf {
    /// The generated MSDF tile (RGB8, distances in each channel).
    pub image: RgbImage,
    /// EM-fraction placement metrics for positioning the tile's quad.
    pub metrics: GlyphMetrics,
}

/// Lift `glyph`'s outline into its tile (see [`generate_glyph_msdf`] for the
/// parameters) and colour its edges. `None` for outline-less glyphs.
fn tile_shape(face: &Face, glyph: GlyphId, ref_px: f32, px_range: f32) -> Option<TileShape> {
    // Whitespace and other outline-less glyphs have no shape → no tile.
    let shape = fdsm_ttf_parser::load_shape_from_face(face, glyph)?;
    let bbox = face.glyph_bounding_box(glyph)?;
    let upm = face.units_per_em() as f64;
    if upm <= 0.0 {
        return None;
    }

    let ref_px = ref_px as f64;
    let px_range = px_range.max(1.0) as f64;
    // Padding (in tile pixels) on every side so the full distance ramp fits. The
    // ramp reaches +-px_range/2 around the outline, so padding >= px_range/2; we
    // use the full px_range plus a pixel for safety.
    let padding = px_range.ceil() + 1.0;

    let scale = ref_px / upm; // font units -> pixels

    let x_min = bbox.x_min as f64;
    let y_min = bbox.y_min as f64;
    let x_max = bbox.x_max as f64;
    let y_max = bbox.y_max as f64;

    // Degenerate bbox (e.g. a zero-area control glyph) — nothing to render.
    if x_max <= x_min || y_max <= y_min {
        return None;
    }

    let glyph_w_px = (x_max - x_min) * scale;
    let glyph_h_px = (y_max - y_min) * scale;
    let width_px = (glyph_w_px + 2.0 * padding).ceil() as u32;
    let height_px = (glyph_h_px + 2.0 * padding).ceil() as u32;
    if width_px == 0 || height_px == 0 {
        return None;
    }

    // Affine: font units (Y-up) -> tile pixels (Y-down).
    //   px = scale * fx + tx           with tx = padding - x_min * scale
    //   py = -scale * fy + ty          with ty = padding + y_max * scale
    // So the glyph's top-left (x_min, y_max) maps to (padding, padding).
    let tx = padding - x_min * scale;
    let ty = padding + y_max * scale;
    #[rustfmt::skip]
    let affine = Affine2::from_matrix_unchecked(Matrix3::new(
        scale, 0.0,    tx,
        0.0,   -scale, ty,
        0.0,   0.0,    1.0,
    ));

    let mut shape = shape;
    shape.transform(&affine);
    snap_to_grid(&mut shape);
    drop_degenerate_segments(&mut shape);
    remove_hairpins(&mut shape);
    orient_contours(&mut shape);

    let colored = Shape::<ColoredContour>::edge_coloring_simple(
        shape,
        EDGE_COLORING_SIN_ALPHA,
        EDGE_COLORING_SEED,
    );

    // Tile-corner -> font units (invert the affine analytically), then -> EM
    // fraction. Computing from the actual tile dimensions (which were ceil'd)
    // keeps the quad's uv mapping exact.
    //   top-left  pixel (0, 0):        fx = -tx/scale,             fy = ty/scale
    //   bot-right pixel (W, H): fx = (W - tx)/scale,  fy = (ty - H)/scale
    let left_fu = -tx / scale;
    let right_fu = (width_px as f64 - tx) / scale;
    let top_fu = ty / scale;
    let bottom_fu = (ty - height_px as f64) / scale;

    let metrics = GlyphMetrics {
        width_px,
        height_px,
        left_em: (left_fu / upm) as f32,
        right_em: (right_fu / upm) as f32,
        top_em: (top_fu / upm) as f32,
        bottom_em: (bottom_fu / upm) as f32,
    };

    Some(TileShape {
        colored,
        px_range,
        metrics,
    })
}

/// Grid the outline's points are snapped to, in tile pixels (a power of two,
/// so snapped coordinates are exact in `f64`).
const SNAP_GRID: f64 = 1.0 / 1024.0;

/// Round every control point onto [`SNAP_GRID`] so the arithmetic between
/// them is exact.
///
/// fdsm stores a straight segment as its start plus an offset, so its end
/// comes back as `start + (end - start)`, which rounding can move off the next
/// segment's start by a unit in the last place. The two edges meeting at a
/// corner should then be exactly as far from every point whose nearest point
/// is that corner, with the tie going to the edge the point lies square to,
/// but the stray bit decides it instead. Wherever it lands on the wrong edge,
/// that channel takes the edge's extended line as its distance, which draws
/// thin lines and stair-steps of ink running away from corners (Plex's `|`,
/// Phosphor's gear and chat bubble). On the grid, sums and differences of
/// points are exact, so the tie is real again. The move is at most 1/2048 px.
fn snap_to_grid(shape: &mut Shape<Contour>) {
    let snap = |p: Point| {
        Point::new(
            (p.x / SNAP_GRID).round() * SNAP_GRID,
            (p.y / SNAP_GRID).round() * SNAP_GRID,
        )
    };
    for contour in &mut shape.contours {
        for segment in &mut contour.segments {
            *segment = map_points(segment, |_, p| snap(p));
        }
    }
}

/// Index of `segment`'s last control point (its end): 1, 2 or 3.
fn last_point(segment: &Segment) -> usize {
    match segment.order() {
        Order::Linear => 1,
        Order::Quadratic => 2,
        Order::Cubic => 3,
    }
}

/// `segment` with each control point `i` replaced by `f(i, point)`.
fn map_points(segment: &Segment, f: impl Fn(usize, Point) -> Point) -> Segment {
    let p = |i| f(i, segment.control_point(i));
    match segment.order() {
        Order::Linear => Segment::line(p(0), p(1)),
        Order::Quadratic => Segment::quad(p(0), p(1), p(2)),
        Order::Cubic => Segment::cubic(p(0), p(1), p(2), p(3)),
    }
}

/// `segment` run backwards.
fn reversed(segment: &Segment) -> Segment {
    let last = last_point(segment);
    map_points(segment, |i, _| segment.control_point(last - i))
}

/// Remove segments that don't go anywhere: every control point on the start
/// (the snap has already merged points closer than the grid). Fonts repeat
/// points (TrueType closing lines, Phosphor's joins), and such a segment has
/// no direction: fdsm's orthogonality for it is NaN, which wins every
/// distance tie and signs the field from a NaN tangent. Dropping one leaves
/// the contour closed, since it starts where it ends. Contours left empty are
/// removed.
fn drop_degenerate_segments(shape: &mut Shape<Contour>) {
    for contour in &mut shape.contours {
        contour
            .segments
            .retain(|s| (1..=last_point(s)).any(|i| s.control_point(i) != s.start()));
    }
    shape.contours.retain(|c| !c.segments.is_empty());
}

/// Longest hairpin [`remove_hairpins`] takes out, in tile pixels (a sixty-
/// fourth of an em for icons, an eightieth for text).
const HAIRPIN_MAX: f64 = 1.0;
/// Two segments meeting at a sharper turn than this (the cosine between
/// their directions, about 155°) double back on each other.
const HAIRPIN_COS: f64 = -0.9;

/// Take out hairpins: a short segment that the outline runs along and then
/// straight back over. Phosphor's outlines (strokes merged into fills) are
/// full of them, 0.03 to 0.25 px long at the icon atlas size. To a distance
/// field a hairpin is a spike of zero width, and the MSDF draws such a spike
/// as a thin line running off to the tile's edge (the gear's stair-steps, the
/// chat bubble's tail). The short segment goes and its neighbour across the
/// turn is stretched to meet the rest of the outline, which moves that end by
/// the hairpin's length at most.
fn remove_hairpins(shape: &mut Shape<Contour>) {
    for contour in &mut shape.contours {
        loop {
            let segments = &mut contour.segments;
            let n = segments.len();
            if n < 3 {
                break;
            }
            let hairpin = (0..n).find_map(|i| {
                let j = (i + 1) % n;
                let turn = end_direction(&segments[i])?.dot(&start_direction(&segments[j])?);
                let (short, long) = if outline_length(&segments[i]) <= outline_length(&segments[j])
                {
                    (i, j)
                } else {
                    (j, i)
                };
                (turn < HAIRPIN_COS && outline_length(&segments[short]) <= HAIRPIN_MAX)
                    .then_some((short, long))
            });
            let Some((short, long)) = hairpin else {
                break;
            };
            let gone = segments[short];
            let long_segment = segments[long];
            segments[long] = if long == (short + 1) % n {
                // The long segment came back over the short one: start it
                // where the short one started.
                map_points(&long_segment, |i, p| if i == 0 { gone.start() } else { p })
            } else {
                // The short segment ran back over the long one: end the long
                // one where the short one ended.
                let last = last_point(&long_segment);
                map_points(&long_segment, |i, p| if i == last { gone.end() } else { p })
            };
            segments.remove(short);
        }
    }
    drop_degenerate_segments(shape);
}

/// Length of `segment`'s control polygon: at least its arc length, so a
/// segment short by this measure is short.
fn outline_length(segment: &Segment) -> f64 {
    (1..=last_point(segment))
        .map(|i| (segment.control_point(i) - segment.control_point(i - 1)).norm())
        .sum()
}

/// Unit direction `segment` leaves its start in: toward the first control
/// point off the start. `None` when every point is on the start.
fn start_direction(segment: &Segment) -> Option<Vect> {
    (1..=last_point(segment))
        .map(|i| segment.control_point(i) - segment.start())
        .find(|v| v.norm() > 0.0)
        .map(|v| v.normalize())
}

/// Unit direction `segment` arrives at its end in; see [`start_direction`].
fn end_direction(segment: &Segment) -> Option<Vect> {
    start_direction(&reversed(segment)).map(|v| -v)
}

/// Reverse every contour whose fill (under the nonzero rule, judged by the
/// whole shape) is on its left, so that all of them run clockwise on screen
/// with the fill on their right: the direction a TrueType outline has in tile
/// space (its outer contours run clockwise with Y up, and the Y flip mirrors
/// both the outline and the meaning of "clockwise"), so a well-formed font
/// passes through untouched. The MSDF's channel signs follow each contour's
/// direction, so a font that draws some parts the other way round (Phosphor
/// does: arrows, the palette's dots, the eye's slash) otherwise gets fields
/// that disagree between contours, which the renderer shows as specks and
/// smears away from the outline. msdfgen calls this `orientContours`; fdsm has
/// no equivalent.
fn orient_contours(shape: &mut Shape<Contour>) {
    /// How far to either side of the outline to probe the fill, in tile px.
    const PROBE: f64 = 0.05;
    let prepared = shape.prepare();
    let filled_at = |p: Point| {
        prepared
            .scanline(p.y)
            .cursor()
            .filled(p.x, FillRule::Nonzero)
    };
    for contour in &mut shape.contours {
        // Each segment votes from its midpoint; ties and unclear probes
        // (a segment on another contour's edge) leave the contour alone.
        let mut votes = 0i32;
        for segment in &contour.segments {
            let dir = segment.direction_at(0.5);
            let len = dir.norm();
            if len <= f64::EPSILON {
                continue;
            }
            let p = segment.get(0.5);
            // Right of travel on screen (Y down).
            let right = Vect::new(-dir.y, dir.x) / len * PROBE;
            match (filled_at(p + right), filled_at(p - right)) {
                (true, false) => votes += 1,
                (false, true) => votes -= 1,
                _ => {}
            }
        }
        if votes < 0 {
            contour.segments = contour.segments.iter().rev().map(reversed).collect();
        }
    }
}

/// A glyph outline placed in its tile: tile pixels, Y down, edges coloured.
struct TileShape {
    colored: Shape<ColoredContour>,
    /// The distance ramp width in tile pixels (at least 1).
    px_range: f64,
    metrics: GlyphMetrics,
}

/// Generate an MSDF tile for one glyph.
///
/// * `face` — the parsed font face (from `cosmic_text::Font::data()`).
/// * `glyph` — the glyph id to render (post-shaping, from cosmic-text layout).
/// * `ref_px` — the reference EM size in pixels the tile is generated at. Larger
///   gives more distance-field resolution (crisper at large display sizes) at the
///   cost of atlas space. ~48–64 is typical.
/// * `px_range` — the width of the distance ramp in tile pixels. The shader uses
///   this to scale screen-space AA. ~4–6 is typical. Also sets the tile padding.
///
/// Returns `None` for glyphs with no outline (whitespace) — the caller advances
/// the pen without drawing a tile.
pub fn generate_glyph_msdf(
    face: &Face,
    glyph: GlyphId,
    ref_px: f32,
    px_range: f32,
) -> Option<GlyphMsdf> {
    let tile = tile_shape(face, glyph, ref_px, px_range)?;
    let (width_px, height_px) = (tile.metrics.width_px, tile.metrics.height_px);
    let prepared = tile.colored.prepare();

    // Generated in f32 because the error correction needs it: without that
    // pass, texels where two channels' false edges meet interpolate to stray
    // ink outside the outline (short lines and dots beside Phosphor's house,
    // palette and angle icons). Correction runs before the sign fix, as fdsm
    // requires.
    let mut field = Rgb32FImage::new(width_px, height_px);
    generate_msdf(&prepared, tile.px_range, &mut field);
    correct_error_msdf(
        &mut field,
        &tile.colored,
        &prepared,
        tile.px_range,
        &ErrorCorrectionConfig::default(),
    );
    correct_sign_msdf(&mut field, &prepared, FillRule::Nonzero);
    // Quantize the way fdsm's own u8 output does (scale and truncate).
    let image = RgbImage::from_fn(width_px, height_px, |x, y| {
        image::Rgb(
            field
                .get_pixel(x, y)
                .0
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8),
        )
    });

    Some(GlyphMsdf {
        image,
        metrics: tile.metrics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_face() -> Face<'static> {
        Face::parse(
            include_bytes!("../../assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf"),
            0,
        )
        .expect("parse IBM Plex Sans")
    }

    fn glyph_for(face: &Face, c: char) -> GlyphId {
        face.glyph_index(c).expect("glyph present")
    }

    /// median of an RGB pixel, normalized to 0..1. The MSDF reconstruction uses
    /// the median channel; > 0.5 == inside the glyph, < 0.5 == outside.
    fn median01(p: &image::Rgb<u8>) -> f32 {
        let mut v = [p.0[0], p.0[1], p.0[2]];
        v.sort_unstable();
        v[1] as f32 / 255.0
    }

    /// The field's distance (tile pixels, positive inside) at tile point
    /// `(x, y)`, sampled the way the shader does: bilinear per channel, then
    /// the median.
    fn sampled_distance(img: &RgbImage, px_range: f64, x: f64, y: f64) -> f64 {
        let (w, h) = (img.width() as i64, img.height() as i64);
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let texel = |i: i64, j: i64| {
            let p = img.get_pixel(i.clamp(0, w - 1) as u32, j.clamp(0, h - 1) as u32);
            p.0.map(|c| c as f64 / 255.0)
        };
        let (i, j) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (
            texel(i, j),
            texel(i + 1, j),
            texel(i, j + 1),
            texel(i + 1, j + 1),
        );
        let mut v = [0.0; 3];
        for k in 0..3 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            v[k] = top + (bottom - top) * ty;
        }
        v.sort_by(f64::total_cmp);
        (v[1] - 0.5) * px_range
    }

    /// Where the glyph, drawn at each of `scales` (screen px per tile texel)
    /// with the shader's anti-aliasing, puts visible ink (over 10 % coverage)
    /// more than two screen pixels outside the true outline, or a visible hole
    /// more than two pixels inside it. Closer to the outline nothing is
    /// judged: an MSDF keeps corners sharp where the true distance rounds
    /// them, so a sharp tip's anti-aliasing legitimately reaches a little past
    /// the rounded outline. Returns `(scale, fault count, first fault in tile
    /// px)` for each scale with faults.
    fn msdf_faults(
        face: &Face,
        glyph: GlyphId,
        ref_px: f32,
        px_range: f32,
        scales: &[f64],
    ) -> Vec<(f64, usize, (f64, f64))> {
        let tile = tile_shape(face, glyph, ref_px, px_range).expect("glyph has an outline");
        let img = generate_glyph_msdf(face, glyph, ref_px, px_range)
            .expect("glyph generates")
            .image;
        let prepared = tile.colored.prepare();
        let (w, h) = (img.width() as f64, img.height() as f64);
        let mut report = Vec::new();
        for &scale in scales {
            // Sample twice per screen pixel, in tile texels.
            let step = 0.5 / scale;
            let far = 2.0 / scale;
            let mut faults = Vec::new();
            let mut y = step * 0.5;
            while y < h {
                let scanline = prepared.scanline(y);
                let mut x = step * 0.5;
                while x < w {
                    let screen = sampled_distance(&img, tile.px_range, x, y) * scale;
                    let coverage = (screen + 0.5).clamp(0.0, 1.0);
                    let inside = scanline.cursor().filled(x, FillRule::Nonzero);
                    // The exact distance is the slow part, so it's only
                    // taken where the ink disagrees with the fill.
                    if (if inside {
                        coverage < 0.9
                    } else {
                        coverage > 0.1
                    }) && prepared.distance4(Point::new(x, y))[3]
                        .value
                        .distance()
                        .abs()
                        > far
                    {
                        faults.push((x, y));
                    }
                    x += step;
                }
                y += step;
            }
            if let Some(&first) = faults.first() {
                report.push((scale, faults.len(), first));
            }
        }
        report
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn icon_fields_have_no_stray_ink_or_holes() {
        use crate::render::PhosphorIcon;
        let face = Face::parse(crate::render::phosphor::PHOSPHOR_TTF, 0).expect("parse Phosphor");
        // The icon atlas's parameters (`ICON_REF_PX`, `DEFAULT_PX_RANGE`),
        // drawn from a 9 px header-key icon up to 64 px. Each defect this
        // guards against showed at only some of these sizes.
        let scales = [0.12, 0.14, 0.16, 0.2, 0.25, 0.3, 0.4, 0.5, 0.7, 1.0];
        let bad: Vec<_> = PhosphorIcon::ALL
            .iter()
            .flat_map(|&icon| {
                let glyph = GlyphId(icon.glyph().expect("icon resolves").glyph_id);
                msdf_faults(&face, glyph, 64.0, 12.0, &scales)
                    .into_iter()
                    .map(move |fault| (icon.name(), fault))
            })
            .collect();
        assert!(
            bad.is_empty(),
            "icons with faults (name, (scale, count, first)): {bad:?}"
        );
    }

    #[test]
    fn text_fields_have_no_stray_ink_or_holes() {
        let face = test_face();
        // The text atlas's parameters (`DEFAULT_REF_PX`, `DEFAULT_PX_RANGE`),
        // drawn from 9 px captions up to 40 px.
        let scales = [0.22, 0.25, 0.3, 0.4, 0.5, 0.7, 1.0];
        let bad: Vec<_> = (0x21u8..=0x7e)
            .map(char::from)
            .flat_map(|c| {
                msdf_faults(&face, glyph_for(&face, c), 40.0, 12.0, &scales)
                    .into_iter()
                    .map(move |fault| (c, fault))
            })
            .collect();
        assert!(
            bad.is_empty(),
            "glyphs with faults (char, (scale, count, first)): {bad:?}"
        );
    }

    #[test]
    fn a_well_formed_fonts_contours_keep_their_direction() {
        let face = test_face();
        let flip = Affine2::from_matrix_unchecked(Matrix3::new(
            1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0,
        ));
        for c in (0x21u8..=0x7e).map(char::from) {
            let mut shape = fdsm_ttf_parser::load_shape_from_face(&face, glyph_for(&face, c))
                .expect("glyph has an outline");
            shape.transform(&flip);
            let before: Vec<_> = shape.contours.iter().map(ends).collect();
            orient_contours(&mut shape);
            let after: Vec<_> = shape.contours.iter().map(ends).collect();
            assert_eq!(before, after, "'{c}' had a contour reversed");
        }
    }

    fn line(a: (f64, f64), b: (f64, f64)) -> Segment {
        Segment::line(Point::new(a.0, a.1), Point::new(b.0, b.1))
    }

    /// A closed contour through `points` in order.
    fn polygon(points: &[(f64, f64)]) -> Contour {
        let n = points.len();
        Contour {
            segments: (0..n)
                .map(|i| line(points[i], points[(i + 1) % n]))
                .collect(),
        }
    }

    fn ends(contour: &Contour) -> Vec<(f64, f64)> {
        contour
            .segments
            .iter()
            .map(|s| (s.start().x, s.start().y))
            .collect()
    }

    #[test]
    fn snapping_lands_every_point_on_the_grid() {
        let mut shape = Shape {
            contours: vec![polygon(&[(0.1, 0.2), (10.3337, 0.2), (10.3337, 7.00049)])],
        };
        snap_to_grid(&mut shape);
        for s in &shape.contours[0].segments {
            for p in [s.start(), s.end()] {
                assert_eq!((p.x / SNAP_GRID).fract(), 0.0, "{p:?}");
                assert_eq!((p.y / SNAP_GRID).fract(), 0.0, "{p:?}");
            }
        }
    }

    #[test]
    fn zero_length_segments_and_empty_contours_are_dropped() {
        let mut shape = Shape {
            contours: vec![
                Contour {
                    segments: vec![
                        line((0.0, 0.0), (4.0, 0.0)),
                        line((4.0, 0.0), (4.0, 0.0)),
                        line((4.0, 0.0), (4.0, 4.0)),
                        line((4.0, 4.0), (0.0, 0.0)),
                    ],
                },
                Contour {
                    segments: vec![line((9.0, 9.0), (9.0, 9.0))],
                },
            ],
        };
        drop_degenerate_segments(&mut shape);
        assert_eq!(shape.contours.len(), 1);
        assert_eq!(
            ends(&shape.contours[0]),
            [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)]
        );
    }

    #[test]
    fn hairpins_come_out_and_the_contour_stays_closed() {
        // A square whose top edge overshoots its corner by 0.25 px and comes
        // back, and whose right edge starts with a 0.1 px step back up.
        let mut shape = Shape {
            contours: vec![Contour {
                segments: vec![
                    line((0.0, 0.0), (10.25, 0.0)),
                    line((10.25, 0.0), (10.0, 0.0)),
                    line((10.0, 0.0), (10.0, -0.1)),
                    line((10.0, -0.1), (10.0, 10.0)),
                    line((10.0, 10.0), (0.0, 10.0)),
                    line((0.0, 10.0), (0.0, 0.0)),
                ],
            }],
        };
        remove_hairpins(&mut shape);
        let contour = &shape.contours[0];
        assert_eq!(contour.segments.len(), 4, "{:?}", ends(contour));
        let n = contour.segments.len();
        for i in 0..n {
            assert_eq!(
                contour.segments[i].end(),
                contour.segments[(i + 1) % n].start()
            );
        }
        for (x, y) in ends(contour) {
            assert!(
                (0.0..=10.0).contains(&x) && (-0.1..=10.0).contains(&y),
                "{x},{y}"
            );
        }
    }

    #[test]
    fn long_reversals_are_left_alone() {
        // A 5 px spike is part of the design, not a hairpin.
        let mut shape = Shape {
            contours: vec![polygon(&[
                (0.0, 0.0),
                (15.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
            ])],
        };
        remove_hairpins(&mut shape);
        assert_eq!(shape.contours[0].segments.len(), 5);
    }

    #[test]
    fn contours_are_turned_to_run_clockwise_on_screen() {
        // Tile space is Y down: (0,0) -> (0,10) -> (10,10) runs down the left
        // side first, anticlockwise on screen, with the fill on its left.
        let wrong = polygon(&[(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)]);
        let right = polygon(&[(20.0, 0.0), (30.0, 0.0), (30.0, 10.0), (20.0, 10.0)]);
        let mut shape = Shape {
            contours: vec![wrong, right.clone()],
        };
        orient_contours(&mut shape);
        assert_eq!(
            ends(&shape.contours[0]),
            [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
        );
        assert_eq!(ends(&shape.contours[1]), ends(&right));
    }

    #[test]
    fn whitespace_glyph_has_no_tile() {
        let face = test_face();
        let space = glyph_for(&face, ' ');
        assert!(
            generate_glyph_msdf(&face, space, 48.0, 4.0).is_none(),
            "space should have no outline tile"
        );
    }

    #[test]
    fn glyph_generates_nonempty_tile() {
        let face = test_face();
        let a = glyph_for(&face, 'A');
        let g = generate_glyph_msdf(&face, a, 48.0, 4.0).expect("A generates");
        assert!(g.metrics.width_px > 0 && g.metrics.height_px > 0);
        assert_eq!(g.image.width(), g.metrics.width_px);
        assert_eq!(g.image.height(), g.metrics.height_px);
        // EM metrics should be sane: the glyph is above the baseline and has
        // positive horizontal extent.
        assert!(g.metrics.right_em > g.metrics.left_em);
        assert!(g.metrics.top_em > g.metrics.bottom_em);
        assert!(g.metrics.top_em > 0.0, "uppercase A extends above baseline");
    }

    #[test]
    fn sign_is_correct_inside_vs_outside() {
        // The tile corners are in the padding margin → always outside (median < 0.5).
        // At least one interior pixel must be inside (median > 0.5). A sign
        // inversion or empty output fails both checks.
        let face = test_face();
        for c in ['A', 'M', 'H', 'g', '0', '@'] {
            let gid = glyph_for(&face, c);
            let g = generate_glyph_msdf(&face, gid, 48.0, 4.0)
                .unwrap_or_else(|| panic!("'{c}' generates"));
            let img = &g.image;
            let (w, h) = (img.width(), img.height());

            // All four corners sit in padding → outside.
            for &(x, y) in &[(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                let m = median01(img.get_pixel(x, y));
                assert!(
                    m < 0.5,
                    "'{c}' corner ({x},{y}) median {m} should be < 0.5 (outside)"
                );
            }

            // Some pixel is inside.
            let any_inside = img.pixels().any(|p| median01(p) > 0.5);
            assert!(any_inside, "'{c}' has no interior pixels (median > 0.5)");
        }
    }

    /// Eyeball check for fdsm quality across the printable-ASCII set — the
    /// maturity risk gate from the plan. Writes per-glyph MSDF PNGs (RGB encoded
    /// directly, so you see the raw 3-channel field) into `test_output/msdf/`.
    /// Run with: `cargo test -p wgpu-gameui dump_ascii_msdf -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes PNG files for manual inspection"]
    fn dump_ascii_msdf() {
        let face = test_face();
        let dir = std::path::Path::new("test_output/msdf");
        std::fs::create_dir_all(dir).expect("create test_output/msdf");
        let mut generated = 0usize;
        for code in 0x21u8..=0x7e {
            let c = code as char;
            let gid = match face.glyph_index(c) {
                Some(g) => g,
                None => continue,
            };
            let Some(g) = generate_glyph_msdf(&face, gid, 48.0, 4.0) else {
                continue;
            };
            let safe = format!("{:02x}_{}", code, if c.is_alphanumeric() { c } else { '_' });
            let path = dir.join(format!("{safe}.png"));
            g.image.save(&path).expect("save png");
            generated += 1;
        }
        eprintln!("wrote {generated} glyph MSDFs to {}", dir.display());
        assert!(
            generated > 90,
            "expected most of printable ASCII to generate"
        );
    }
}
