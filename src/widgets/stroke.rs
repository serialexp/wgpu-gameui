//! Strokes: lines and polylines drawn as SDF segment instances, with SVG's
//! caps, joins and dashes (see `docs/design/sdf-lines.md`).
//!
//! A stroke is cut into one analytic record per segment. Each record carries
//! its neighbouring points, so the shader can draw the joins and decide which
//! segment owns each pixel near a corner: a translucent stroke covers every
//! pixel once.

use crate::affine::Affine2;

/// How an open stroke's ends are drawn (SVG `stroke-linecap`). Each dash of a
/// dashed stroke gets them too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cap {
    /// Ends flat at the end point.
    #[default]
    Butt,
    /// A half disc past the end point.
    Round,
    /// A half square past the end point.
    Square,
}

/// How a stroke's segments meet (SVG `stroke-linejoin`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Join {
    /// The outer edges meet in a point, unless that point is further from the
    /// corner than the [miter limit](Stroke::miter_limit) allows; then it is a
    /// bevel.
    #[default]
    Miter,
    /// A disc around the corner.
    Round,
    /// The outer corners joined by a straight edge.
    Bevel,
}

/// A dash pattern (SVG `stroke-dasharray` with two values, and
/// `stroke-dashoffset`): `on` px drawn, `off` px skipped, starting `offset` px
/// into the pattern. The pattern runs on round corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dash {
    /// Length of each dash.
    pub on: f32,
    /// Length of each gap.
    pub off: f32,
    /// How far into the pattern the stroke starts.
    pub offset: f32,
}

/// How a line or polyline is stroked: SVG's stroke properties, with its
/// defaults (butt caps, miter joins, a miter limit of 4, no dashes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// Width across the line, in local px.
    pub width: f32,
    /// The ends of an open stroke, and of each dash.
    pub cap: Cap,
    /// Where segments meet.
    pub join: Join,
    /// A miter join becomes a bevel when its point would be further than
    /// `miter_limit × width / 2` from the corner (SVG `stroke-miterlimit`).
    pub miter_limit: f32,
    /// Dashes, or `None` for a solid stroke.
    pub dash: Option<Dash>,
}

impl Stroke {
    /// A solid stroke `width` px wide, with SVG's defaults.
    pub const fn new(width: f32) -> Self {
        Self {
            width,
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 4.0,
            dash: None,
        }
    }

    /// With `cap` at the ends (and at each dash's ends).
    pub const fn cap(self, cap: Cap) -> Self {
        Self { cap, ..self }
    }

    /// With `join` at the corners.
    pub const fn join(self, join: Join) -> Self {
        Self { join, ..self }
    }

    /// With miter joins becoming bevels past `limit` (see
    /// [`miter_limit`](Self::miter_limit)).
    pub const fn miter_limit(self, limit: f32) -> Self {
        Self {
            miter_limit: limit,
            ..self
        }
    }

    /// Dashed: `on` px drawn, `off` px skipped, from the start of the pattern.
    pub const fn dashed(self, on: f32, off: f32) -> Self {
        Self {
            dash: Some(Dash {
                on,
                off,
                offset: 0.0,
            }),
            ..self
        }
    }

    /// Dashed with the pattern starting `offset` px in.
    pub const fn dashed_from(self, on: f32, off: f32, offset: f32) -> Self {
        Self {
            dash: Some(Dash { on, off, offset }),
            ..self
        }
    }
}

/// One stroke segment as the GPU gets it (`shade_segment` in `ui.wgsl`). The
/// points are in the space the linear part and translation map to the screen.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SegmentInstance {
    /// Forward affine linear part `[a, b, c, d]`.
    pub linear: [f32; 4],
    /// `[tx, ty, clip_enabled, half_width]`.
    pub translation: [f32; 4],
    /// `[ax, ay, bx, by]`: the segment, from `a` to `b`.
    pub ends: [f32; 4],
    /// The colour, tint applied.
    pub color: [f32; 4],
    /// `[prev_x, prev_y, next_x, next_y]`: the point before `a` and the one
    /// after `b`, when the ends are joins.
    pub neighbours: [f32; 4],
    /// `[start, end, start_reach, end_reach]`: each end's style (a `STYLE_*`
    /// code) and how far it reaches past its end point, in local px.
    pub styles: [f32; 4],
    /// `[on, off, offset, length]`: the dash pattern (both zero when solid)
    /// and the whole stroke's length.
    pub dash: [f32; 4],
    /// `[before, dash_cap, closed, 0]`: the stroke's length before `a`, the
    /// dash ends' cap (a `CAP_*` code) and whether the stroke is closed.
    pub phase: [f32; 4],
    /// World-space clip rect `[x, y, width, height]`.
    pub clip: [f32; 4],
}

// End styles, as `shade_segment` reads them. Below `STYLE_ROUND_JOIN` the end
// is open (a cap); from it on, the end is a corner with a neighbour.
pub(crate) const STYLE_BUTT: f32 = 0.0;
pub(crate) const STYLE_SQUARE: f32 = 1.0;
pub(crate) const STYLE_ROUND_CAP: f32 = 2.0;
pub(crate) const STYLE_ROUND_JOIN: f32 = 3.0;
pub(crate) const STYLE_MITER: f32 = 4.0;
pub(crate) const STYLE_BEVEL: f32 = 5.0;

// Dash caps.
const CAP_BUTT: f32 = 0.0;
const CAP_SQUARE: f32 = 1.0;
const CAP_ROUND: f32 = 2.0;

/// Segments shorter than this are dropped: their direction is noise.
const MIN_SEGMENT: f32 = 1e-5;

/// Why a stroke drew nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Skip {
    /// Bad input: a non-finite or non-positive width, a non-finite point or
    /// transform, a singular transform, or fewer than two distinct points.
    Degenerate,
    /// Nothing to draw by design: zero-length dashes with butt caps.
    Invisible,
}

/// Where a stroke's segments land and how they are coloured.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paint {
    /// Local space to world.
    pub transform: Affine2,
    /// Straight alpha.
    pub color: [f32; 4],
    /// World-space `[x, y, width, height]`.
    pub clip: Option<[f32; 4]>,
}

/// Cut a stroke through `points` (local space) into segment records for
/// `paint`'s transform, each passed to `emit` with its colour and clip filled
/// in. `scratch` holds the cleaned-up points, so a reused one costs no
/// allocation.
///
/// Under a translate-only transform the points are moved into world space;
/// there, an axis-aligned stroke a whole number of px wide is snapped to the
/// pixel grid so it comes out crisp, covering exactly the pixels a hard-edged
/// quad of the same size would.
pub(crate) fn build_segments(
    points: &[[f32; 2]],
    closed: bool,
    stroke: &Stroke,
    paint: Paint,
    scratch: &mut Vec<[f32; 2]>,
    mut emit: impl FnMut(SegmentInstance),
) -> Result<(), Skip> {
    let Paint {
        transform,
        color,
        clip,
    } = paint;
    let width = stroke.width;
    let affine = [
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.tx,
        transform.ty,
    ];
    if !(width.is_finite() && width > 0.0)
        || affine.iter().any(|v| !v.is_finite())
        || transform.try_inverse().is_none()
    {
        return Err(Skip::Degenerate);
    }
    let dash = stroke.dash.and_then(usable_dash);
    if let Some(dash) = dash
        && dash.on == 0.0
        && stroke.cap == Cap::Butt
    {
        return Err(Skip::Invisible);
    }

    let translate_only = transform.is_translate_only();
    let shift = if translate_only {
        [transform.tx, transform.ty]
    } else {
        [0.0, 0.0]
    };
    scratch.clear();
    for p in points {
        if !(p[0].is_finite() && p[1].is_finite()) {
            return Err(Skip::Degenerate);
        }
        scratch.push([p[0] + shift[0], p[1] + shift[1]]);
    }
    dedupe(scratch, closed);
    if translate_only && let Some(grid_width) = whole_px(width) {
        snap(scratch, closed, grid_width, stroke.cap);
        dedupe(scratch, closed);
    }
    let n = scratch.len();
    if n < 2 {
        return Err(Skip::Degenerate);
    }

    let half = width * 0.5;
    let segments = if closed { n } else { n - 1 };
    let total: f32 = (0..segments)
        .map(|i| distance(scratch[i], scratch[(i + 1) % n]))
        .sum();
    let (linear, translation) = if translate_only {
        ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0])
    } else {
        (
            [transform.a, transform.b, transform.c, transform.d],
            [transform.tx, transform.ty],
        )
    };
    let (clip, clip_enabled) = clip.map_or(([0.0; 4], 0.0), |c| (c, 1.0));
    let dash_values = dash.map_or([0.0, 0.0, 0.0, total], |d| [d.on, d.off, d.offset, total]);
    let dash_cap = match stroke.cap {
        Cap::Butt => CAP_BUTT,
        Cap::Square => CAP_SQUARE,
        Cap::Round => CAP_ROUND,
    };
    let cap = cap_style(stroke.cap, half);

    let mut before = 0.0;
    for i in 0..segments {
        let a = scratch[i];
        let b = scratch[(i + 1) % n];
        let prev = (closed || i > 0).then(|| scratch[(i + n - 1) % n]);
        let next = (closed || i + 2 < n).then(|| scratch[(i + 2) % n]);
        let (start, start_reach) = prev.map_or(cap, |p| corner_style(p, a, b, stroke, half));
        let (end, end_reach) = next.map_or(cap, |q| corner_style(a, b, q, stroke, half));
        let prev = prev.unwrap_or_default();
        let next = next.unwrap_or_default();
        emit(SegmentInstance {
            linear,
            translation: [translation[0], translation[1], clip_enabled, half],
            ends: [a[0], a[1], b[0], b[1]],
            color,
            neighbours: [prev[0], prev[1], next[0], next[1]],
            styles: [start, end, start_reach, end_reach],
            dash: dash_values,
            phase: [before, dash_cap, f32::from(u8::from(closed)), 0.0],
            clip,
        });
        before += distance(a, b);
    }
    Ok(())
}

/// The dash pattern to draw, or `None` for a solid stroke: SVG draws a
/// pattern with a negative, non-finite or all-zero length as solid, and one
/// without gaps is solid anyway. The offset is brought into one period.
fn usable_dash(dash: Dash) -> Option<Dash> {
    let Dash { on, off, offset } = dash;
    let valid = [on, off, offset].iter().all(|v| v.is_finite()) && on >= 0.0 && off >= 0.0;
    if !valid || off == 0.0 {
        return None;
    }
    Some(Dash {
        on,
        off,
        offset: offset.rem_euclid(on + off),
    })
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Drop points too close to the one before (and, when closed, a last point
/// back on the first), so every segment has a direction.
fn dedupe(points: &mut Vec<[f32; 2]>, closed: bool) {
    let mut kept = 0;
    for i in 0..points.len() {
        if kept == 0 || distance(points[kept - 1], points[i]) > MIN_SEGMENT {
            points[kept] = points[i];
            kept += 1;
        }
    }
    points.truncate(kept);
    if closed {
        while points.len() > 1 && distance(points[0], points[points.len() - 1]) <= MIN_SEGMENT {
            points.pop();
        }
    }
}

/// `width` as a whole number of px when it is one, for grid snapping.
fn whole_px(width: f32) -> Option<f32> {
    let whole = width.round();
    (whole >= 1.0 && (width - whole).abs() <= 1e-4).then_some(whole)
}

/// Snap an axis-aligned stroke `width` px wide onto the pixel grid, so it
/// covers exactly the pixels whose centres a hard-edged quad of its outline
/// would: its centre lines move onto pixel centres (odd widths) or edges (even
/// widths), and butt or square open ends onto pixel edges. Leaves any stroke
/// with a diagonal segment alone.
fn snap(points: &mut [[f32; 2]], closed: bool, width: f32, cap: Cap) {
    let n = points.len();
    let segments = if closed { n } else { n.saturating_sub(1) };
    let axis_aligned = (0..segments).all(|i| {
        let (a, b) = (points[i], points[(i + 1) % n]);
        a[0] == b[0] || a[1] == b[1]
    });
    if n < 2 || !axis_aligned {
        return;
    }
    let half = width * 0.5;
    // A hard-edged edge at `e` covers the pixel centres from `e` on: the edge
    // rounds half down onto the grid, like a rasterized quad's.
    let edge = |e: f32| (e - 0.5).ceil();
    let centre = |c: f32| edge(c - half) + half;
    // The axis a segment runs along: 0 for x, 1 for y.
    let along = |a: [f32; 2], b: [f32; 2]| usize::from(a[0] == b[0]);
    let open_end = |at: usize, inner: usize| -> Option<(usize, f32)> {
        let axis = along(points[at], points[inner]);
        let outward = (points[at][axis] - points[inner][axis]).signum();
        let reach = match cap {
            Cap::Butt => 0.0,
            Cap::Square => half,
            Cap::Round => return None,
        };
        let c = points[at][axis];
        Some((axis, edge(c + outward * reach) - outward * reach))
    };
    let first = (!closed).then(|| open_end(0, 1)).flatten();
    let last = (!closed).then(|| open_end(n - 1, n - 2)).flatten();
    for p in points.iter_mut() {
        *p = [centre(p[0]), centre(p[1])];
    }
    if let Some((axis, value)) = first {
        points[0][axis] = value;
    }
    if let Some((axis, value)) = last {
        points[n - 1][axis] = value;
    }
}

/// An open end's style and reach.
fn cap_style(cap: Cap, half: f32) -> (f32, f32) {
    match cap {
        Cap::Butt => (STYLE_BUTT, 0.0),
        Cap::Square => (STYLE_SQUARE, half),
        Cap::Round => (STYLE_ROUND_CAP, half),
    }
}

/// The style of the corner at `at`, between the segments from `prev` and to
/// `next`, and how far past `at` either segment's share of it reaches.
fn corner_style(
    prev: [f32; 2],
    at: [f32; 2],
    next: [f32; 2],
    stroke: &Stroke,
    half: f32,
) -> (f32, f32) {
    let incoming = unit(prev, at);
    let outgoing = unit(at, next);
    // The cosine of the turn, and of half of it.
    let turn = (incoming[0] * outgoing[0] + incoming[1] * outgoing[1]).clamp(-1.0, 1.0);
    let half_turn = ((1.0 + turn) * 0.5).sqrt();
    match stroke.join {
        Join::Round => (STYLE_ROUND_JOIN, half),
        // Each segment's half of the bevel reaches at most half a width back.
        Join::Bevel => (STYLE_BEVEL, half * 0.5),
        Join::Miter => {
            // SVG: the point is 1 / cos(turn / 2) half-widths from the corner.
            if half_turn * stroke.miter_limit.max(1.0) < 1.0 || half_turn <= 0.0 {
                (STYLE_BEVEL, half * 0.5)
            } else {
                let tan = (1.0 - half_turn * half_turn).sqrt() / half_turn;
                (STYLE_MITER, half * tan)
            }
        }
    }
}

fn unit(from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
    let d = [to[0] - from[0], to[1] - from[1]];
    let length = d[0].hypot(d[1]);
    [d[0] / length, d[1] / length]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(
        points: &[[f32; 2]],
        closed: bool,
        stroke: Stroke,
        transform: Affine2,
    ) -> Result<Vec<SegmentInstance>, Skip> {
        let mut out = Vec::new();
        let mut scratch = Vec::new();
        let paint = Paint {
            transform,
            color: [1.0; 4],
            clip: None,
        };
        build_segments(points, closed, &stroke, paint, &mut scratch, |s| {
            out.push(s)
        })
        .map(|()| out)
    }

    fn open(points: &[[f32; 2]], stroke: Stroke) -> Vec<SegmentInstance> {
        build(points, false, stroke, Affine2::IDENTITY).expect("drawn")
    }

    #[test]
    fn a_polyline_is_one_record_a_segment_each_knowing_its_neighbours() {
        let s = open(
            &[[0.0, 0.0], [10.5, 0.0], [10.5, 10.5], [20.0, 10.5]],
            Stroke::new(1.5),
        );
        assert_eq!(s.len(), 3);
        assert_eq!(s[1].ends, [10.5, 0.0, 10.5, 10.5]);
        assert_eq!(s[1].neighbours, [0.0, 0.0, 20.0, 10.5]);
        assert_eq!(s[0].styles[0], STYLE_BUTT, "an open start is a cap");
        assert_eq!(s[0].styles[1], STYLE_MITER);
        assert_eq!(s[2].styles[1], STYLE_BUTT);
        // The stroke's length before each segment carries the dash pattern on.
        let before: Vec<_> = s.iter().map(|s| s.phase[0]).collect();
        assert_eq!(before, [0.0, 10.5, 21.0]);
        assert!(s.iter().all(|s| s.dash[3] == 30.5));
        assert!(s.iter().all(|s| s.translation[3] == 0.75));
    }

    #[test]
    fn both_segments_at_a_corner_agree_on_its_style() {
        let points = [[0.0, 0.0], [30.0, 0.0], [0.0, 3.0]];
        // A hairpin turn: its miter point is far past the limit.
        let s = open(&points, Stroke::new(4.0));
        assert_eq!(s[0].styles[1], STYLE_BEVEL);
        assert_eq!(s[1].styles[0], STYLE_BEVEL);
        let s = open(&points, Stroke::new(4.0).miter_limit(100.0));
        assert_eq!((s[0].styles[1], s[1].styles[0]), (STYLE_MITER, STYLE_MITER));
        assert_eq!(s[0].styles[3], s[1].styles[2], "both reach as far");
    }

    #[test]
    fn a_right_angle_miter_reaches_one_half_width() {
        let s = open(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], Stroke::new(3.0));
        assert_eq!(s[0].styles[1], STYLE_MITER);
        assert!((s[0].styles[3] - 1.5).abs() < 1e-5);
        // √2 is under the default limit of 4, but not under 1.4.
        let s = open(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            Stroke::new(3.0).miter_limit(1.4),
        );
        assert_eq!(s[0].styles[1], STYLE_BEVEL);
        let s = open(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            Stroke::new(3.0).join(Join::Round),
        );
        assert_eq!((s[0].styles[1], s[0].styles[3]), (STYLE_ROUND_JOIN, 1.5));
    }

    #[test]
    fn caps_reach_past_open_ends() {
        for (cap, style, reach) in [
            (Cap::Butt, STYLE_BUTT, 0.0),
            (Cap::Square, STYLE_SQUARE, 2.5),
            (Cap::Round, STYLE_ROUND_CAP, 2.5),
        ] {
            let s = open(&[[0.3, 0.3], [9.7, 4.1]], Stroke::new(5.0).cap(cap));
            assert_eq!(s[0].styles, [style, style, reach, reach], "{cap:?}");
        }
    }

    #[test]
    fn a_closed_stroke_joins_its_last_point_to_its_first() {
        let square = [[0.5, 0.5], [9.5, 0.5], [9.5, 9.5], [0.5, 9.5], [0.5, 0.5]];
        let s = build(&square, true, Stroke::new(1.0), Affine2::IDENTITY).unwrap();
        assert_eq!(s.len(), 4, "the repeated first point is dropped");
        assert!(
            s.iter()
                .all(|s| s.styles[0] == STYLE_MITER && s.styles[1] == STYLE_MITER)
        );
        assert_eq!(s[0].neighbours[..2], [0.5, 9.5]);
        assert_eq!(s[3].ends, [0.5, 9.5, 0.5, 0.5]);
        assert_eq!(s[3].neighbours[2..], [9.5, 0.5]);
        assert!(s.iter().all(|s| s.phase[2] == 1.0));
    }

    #[test]
    fn repeated_points_are_dropped_and_too_few_draw_nothing() {
        let s = open(
            &[[0.0, 0.0], [0.0, 0.0], [5.5, 0.0], [5.5, 0.0], [5.5, 5.0]],
            Stroke::new(1.5),
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].styles[1], STYLE_MITER);
        let one = build(
            &[[1.0, 1.0], [1.0, 1.0]],
            false,
            Stroke::new(1.0),
            Affine2::IDENTITY,
        );
        assert_eq!(one, Err(Skip::Degenerate));
        assert_eq!(
            build(&[], false, Stroke::new(1.0), Affine2::IDENTITY),
            Err(Skip::Degenerate)
        );
    }

    #[test]
    fn bad_input_is_degenerate() {
        let line = [[0.0, 0.0], [5.0, 5.0]];
        for stroke in [
            Stroke::new(0.0),
            Stroke::new(-1.0),
            Stroke::new(f32::NAN),
            Stroke::new(f32::INFINITY),
        ] {
            assert_eq!(
                build(&line, false, stroke, Affine2::IDENTITY),
                Err(Skip::Degenerate)
            );
        }
        let nan = [[0.0, 0.0], [f32::NAN, 5.0]];
        assert_eq!(
            build(&nan, false, Stroke::new(1.0), Affine2::IDENTITY),
            Err(Skip::Degenerate)
        );
        let flat = Affine2::scale(1.0, 0.0);
        assert_eq!(
            build(&line, false, Stroke::new(1.0), flat),
            Err(Skip::Degenerate)
        );
        let far = Affine2::translation(f32::INFINITY, 0.0);
        assert_eq!(
            build(&line, false, Stroke::new(1.0), far),
            Err(Skip::Degenerate)
        );
    }

    #[test]
    fn dashes_are_kept_only_when_they_have_gaps() {
        let line = [[0.0, 0.5], [20.0, 0.5]];
        let s = open(&line, Stroke::new(1.0).dashed_from(4.0, 3.0, 9.0));
        assert_eq!(
            s[0].dash,
            [4.0, 3.0, 2.0, 20.0],
            "the offset is brought into one period"
        );
        for solid in [
            Stroke::new(1.0).dashed(4.0, 0.0),
            Stroke::new(1.0).dashed(-4.0, 3.0),
            Stroke::new(1.0).dashed(f32::NAN, 3.0),
            Stroke::new(1.0).dashed_from(4.0, 3.0, f32::INFINITY),
        ] {
            assert_eq!(
                open(&line, solid)[0].dash,
                [0.0, 0.0, 0.0, 20.0],
                "{solid:?}"
            );
        }
        // Zero-length dashes are dots with round or square caps, and nothing
        // with butt caps.
        let dots = Stroke::new(2.0).dashed(0.0, 4.0);
        assert_eq!(
            build(&line, false, dots, Affine2::IDENTITY),
            Err(Skip::Invisible)
        );
        assert_eq!(open(&line, dots.cap(Cap::Round))[0].phase[1], CAP_ROUND);
    }

    #[test]
    fn a_translation_is_folded_into_the_points() {
        let s = build(
            &[[1.0, 2.0], [7.0, 9.0]],
            false,
            Stroke::new(1.5),
            Affine2::translation(10.0, 20.0),
        )
        .unwrap();
        assert_eq!(s[0].ends, [11.0, 22.0, 17.0, 29.0]);
        assert_eq!(s[0].linear, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s[0].translation[..2], [0.0, 0.0]);
        let turned = Affine2::translation(10.0, 20.0).compose(&Affine2::rotation(0.5));
        let s = build(&[[1.0, 2.0], [7.0, 9.0]], false, Stroke::new(1.0), turned).unwrap();
        assert_eq!(s[0].ends, [1.0, 2.0, 7.0, 9.0], "kept local under rotation");
        assert_eq!(s[0].linear, [turned.a, turned.b, turned.c, turned.d]);
        assert_eq!(s[0].translation[..2], [10.0, 20.0]);
    }

    #[test]
    fn whole_px_axis_aligned_lines_land_on_the_pixels_a_quad_would_cover() {
        // A 1 px rule at y = 10 covered the pixel row whose centre is 9.5 as
        // a quad (centres in [9.5, 10.5)), from x = 3 to 7.
        let s = open(&[[3.0, 10.0], [7.0, 10.0]], Stroke::new(1.0));
        assert_eq!(s[0].ends, [3.0, 9.5, 7.0, 9.5]);
        // Fractional positions round the same way, ends onto pixel edges.
        let s = open(&[[3.4, 10.3], [7.6, 10.3]], Stroke::new(1.0));
        assert_eq!(s[0].ends, [3.0, 10.5, 8.0, 10.5]);
        // Even widths centre on a pixel edge.
        let s = open(&[[2.0, 4.0], [2.0, 9.0]], Stroke::new(2.0));
        assert_eq!(s[0].ends, [2.0, 4.0, 2.0, 9.0]);
        let s = open(&[[2.4, 4.0], [2.4, 9.0]], Stroke::new(2.0));
        assert_eq!(s[0].ends, [2.0, 4.0, 2.0, 9.0]);
        // A square cap's outer edge lands on the grid.
        let s = open(
            &[[3.0, 10.0], [7.0, 10.0]],
            Stroke::new(1.0).cap(Cap::Square),
        );
        assert_eq!(s[0].ends, [2.5, 9.5, 6.5, 9.5]);
        // Corners sit on pixel centres, so the joins fill whole pixels.
        let s = open(&[[1.0, 1.0], [9.0, 1.0], [9.0, 9.0]], Stroke::new(1.0));
        assert_eq!(s[0].ends, [1.0, 0.5, 8.5, 0.5]);
        assert_eq!(s[1].ends, [8.5, 0.5, 8.5, 9.0]);
    }

    #[test]
    fn diagonal_fractional_or_transformed_lines_are_not_snapped() {
        let s = open(&[[3.0, 10.0], [7.0, 10.0], [9.0, 12.0]], Stroke::new(1.0));
        assert_eq!(s[0].ends, [3.0, 10.0, 7.0, 10.0]);
        let s = open(&[[3.0, 10.0], [7.0, 10.0]], Stroke::new(1.5));
        assert_eq!(s[0].ends, [3.0, 10.0, 7.0, 10.0]);
        let scaled = build(
            &[[3.0, 10.0], [7.0, 10.0]],
            false,
            Stroke::new(1.0),
            Affine2::scale(2.0, 2.0),
        )
        .unwrap();
        assert_eq!(scaled[0].ends, [3.0, 10.0, 7.0, 10.0]);
    }

    #[test]
    fn a_reused_scratch_buffer_does_not_grow() {
        let mut scratch = Vec::with_capacity(8);
        let capacity = scratch.capacity();
        for _ in 0..3 {
            build_segments(
                &[[0.0, 0.0], [4.0, 1.0], [8.0, 0.0]],
                false,
                &Stroke::new(1.0),
                Paint {
                    transform: Affine2::IDENTITY,
                    color: [1.0; 4],
                    clip: None,
                },
                &mut scratch,
                |_| {},
            )
            .unwrap();
        }
        assert_eq!(scratch.capacity(), capacity);
    }
}
