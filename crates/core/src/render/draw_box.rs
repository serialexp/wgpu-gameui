//! Draw boxes: the physical pixels a primitive could change.
//!
//! Each function here mirrors its kind's shader in `ui.wgsl`: the quad the
//! vertex shader rasterizes, cut where the fragment shader's coverage is
//! exactly zero, then by the primitive's clip. A pixel outside the box is
//! never written, which is what lets primitives that don't share a box be
//! drawn in any order (`docs/design/draw-call-batching.md`, "New: draw
//! boxes").
//!
//! Two kinds of bound:
//! - A hard edge is all-or-nothing: a triangle's edge, a clip test (which
//!   keeps a pixel exactly on its edge), a crisp stripe. The rasterizer can
//!   move a triangle edge by about 1/32 px, so a hard bound is widened by
//!   [`HARD_EDGE`].
//! - A soft edge fades out through anti-aliasing. Its bound is where the
//!   coverage formula reaches exactly 0, which leaves the target unchanged.
//!
//! Every bound is also widened by [`float_margin`], for the error of the f32
//! steps a position goes through on its way to the target.

use super::QUAD_INDICES;
use crate::layout::Rect;
use crate::text::{MsdfInstance, MsdfPrepared};
use crate::widgets::{
    ANALYTIC_KIND_CHROME, ANALYTIC_KIND_SEGMENT, ANALYTIC_KIND_SHADOW, ANALYTIC_KIND_STRIPES,
    AnalyticInstance, CircleInstance, IconDraw, NineSliceDraw, Vertex,
};

/// How far a hard edge can move, in physical px: the rasterizer snaps
/// triangle edges to a sub-pixel grid, and Vulkan only promises 4 bits of it.
pub(crate) const HARD_EDGE: f32 = 1.0 / 16.0;

/// How far an untransformed chrome or stripe record's anti-aliasing reaches
/// past its rect, in physical px. `edge_coverage` reaches half a pixel along
/// a straight edge; near a corner `box_sdf`'s diagonal gradient takes it to
/// (1 + √2) / 4 ≈ 0.604 px at radius 0, and less for any larger radius.
pub(crate) const CHROME_REACH: f32 = 0.61;

/// The target a list is drawn onto, as the renderer projects it
/// (`UiRenderer::prepare_pass`): logical point `p` lands on physical
/// `(p - origin) · scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    /// Physical size in pixels.
    pub size: (u32, u32),
    /// Physical px per logical px.
    pub scale: f32,
    /// The logical point at the target's top-left corner.
    pub origin: [f32; 2],
}

impl Target {
    /// The target for a pass of `size` physical px at `scale_factor`, with the
    /// view origin `origin`. A scale factor that isn't positive is taken as 1,
    /// as the renderer does.
    pub(crate) fn new(size: (u32, u32), scale_factor: f32, origin: [f32; 2]) -> Self {
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        Self {
            size,
            scale,
            origin,
        }
    }

    fn whole(&self) -> PixelBox {
        PixelBox {
            x0: 0,
            y0: 0,
            x1: self.size.0,
            y1: self.size.1,
        }
    }
}

/// A range of whole pixels on the target, `x0..x1` by `y0..y1`. Empty when
/// either range is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelBox {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl PixelBox {
    /// No pixels.
    pub(crate) const EMPTY: Self = Self {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    pub(crate) fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    /// Whether the two boxes share a pixel.
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x0 < other.x1
            && other.x0 < self.x1
            && self.y0 < other.y1
            && other.y0 < self.y1
    }

    /// The smallest box holding both.
    pub(crate) fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    /// Whether `(x, y)` is one of the box's pixels.
    #[cfg(test)]
    pub(crate) fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// A bound in physical px, before it is turned into whole pixels. `None` in
/// [`Bound::Whole`]'s place would lose the difference between "nothing" and
/// "can't tell", which matters: a bound that can't be computed covers the
/// whole target, never nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bound {
    /// From `lo` to `hi`, inclusive.
    Span { lo: [f32; 2], hi: [f32; 2] },
    /// Couldn't be computed (a NaN or infinite input): the whole target.
    Whole,
    /// Nothing: a clip that holds no pixels.
    Nothing,
}

impl Bound {
    fn intersect(self, other: Self) -> Self {
        match (self, other) {
            (Self::Nothing, _) | (_, Self::Nothing) => Self::Nothing,
            (Self::Whole, b) | (b, Self::Whole) => b,
            (Self::Span { lo: a0, hi: a1 }, Self::Span { lo: b0, hi: b1 }) => Self::Span {
                lo: [a0[0].max(b0[0]), a0[1].max(b0[1])],
                hi: [a1[0].min(b1[0]), a1[1].min(b1[1])],
            },
        }
    }

    /// The pixels whose centres lie in the bound, cut to the target.
    fn pixels(self, target: &Target) -> PixelBox {
        let (lo, hi) = match self {
            Self::Nothing => return PixelBox::EMPTY,
            Self::Whole => return target.whole(),
            Self::Span { lo, hi } => (lo, hi),
        };
        // Pixel `i`'s centre is `i + 0.5`, so the centres in `lo..=hi` are
        // `ceil(lo - 0.5)..=floor(hi - 0.5)`.
        let axis = |lo: f32, hi: f32, size: u32| -> (u32, u32) {
            let size = size as f32;
            let first = (lo - 0.5).ceil().clamp(0.0, size);
            let end = ((hi - 0.5).floor() + 1.0).clamp(0.0, size);
            (first as u32, end as u32)
        };
        let (x0, x1) = axis(lo[0], hi[0], target.size.0);
        let (y0, y1) = axis(lo[1], hi[1], target.size.1);
        if x0 >= x1 || y0 >= y1 {
            return PixelBox::EMPTY;
        }
        PixelBox { x0, y0, x1, y1 }
    }
}

/// The float margin for a bound whose inputs reach `magnitude` (the largest
/// absolute coordinate that went into it, local or world), in physical px:
/// several times the error of the f32 steps between a position and its pixel
/// (the transform, the view-origin projection, the viewport).
pub(crate) fn float_margin(target: &Target, magnitude: f32) -> f32 {
    let reach = magnitude
        .max(target.origin[0].abs())
        .max(target.origin[1].abs())
        .max(target.size.0 as f32)
        .max(target.size.1 as f32);
    8.0 * f32::EPSILON * reach * target.scale
}

/// Logical points, gathered into a bound on the target.
struct Points {
    lo: [f32; 2],
    hi: [f32; 2],
    /// The largest absolute value among the points and whatever they were
    /// computed from.
    magnitude: f32,
    finite: bool,
}

impl Points {
    fn new() -> Self {
        Self {
            lo: [f32::INFINITY; 2],
            hi: [f32::NEG_INFINITY; 2],
            magnitude: 0.0,
            finite: true,
        }
    }

    /// Note values a point was computed from, for the float margin.
    fn inputs(&mut self, values: &[f32]) {
        for v in values {
            self.finite &= v.is_finite();
            self.magnitude = self.magnitude.max(v.abs());
        }
    }

    fn add(&mut self, p: [f32; 2]) {
        self.inputs(&p);
        self.lo = [self.lo[0].min(p[0]), self.lo[1].min(p[1])];
        self.hi = [self.hi[0].max(p[0]), self.hi[1].max(p[1])];
    }

    /// The bound on the target, widened by `reach` physical px past the
    /// points (an edge's own reach) and by the float margin.
    fn bound(&self, target: &Target, reach: f32) -> Bound {
        if !self.finite || self.lo[0] > self.hi[0] {
            return Bound::Whole;
        }
        let widen = reach + float_margin(target, self.magnitude);
        let project = |p: [f32; 2], by: f32| {
            [
                (p[0] - target.origin[0]) * target.scale + by,
                (p[1] - target.origin[1]) * target.scale + by,
            ]
        };
        let (lo, hi) = (project(self.lo, -widen), project(self.hi, widen));
        if !(lo.iter().chain(&hi).all(|v| v.is_finite())) {
            return Bound::Whole;
        }
        Bound::Span { lo, hi }
    }
}

/// A clip `[x, y, width, height]` as the shaders test it (`outside_clip`): a
/// pixel exactly on its edge is kept, and a clip of zero or negative size
/// holds nothing. A clip that isn't finite is tested as the shader would: a
/// NaN fails every comparison and so discards nothing.
fn clip_bound(target: &Target, clip: [f32; 4]) -> Bound {
    if clip[2] <= 0.0 || clip[3] <= 0.0 {
        return Bound::Nothing;
    }
    let mut points = Points::new();
    points.inputs(&clip);
    points.add([clip[0], clip[1]]);
    points.add([clip[0] + clip[2], clip[1] + clip[3]]);
    points.bound(target, HARD_EDGE)
}

fn optional_clip(target: &Target, clip: Option<Rect>) -> Bound {
    match clip {
        Some(c) => clip_bound(target, [c.x, c.y, c.width, c.height]),
        None => Bound::Whole,
    }
}

/// `x' = a·x + b·y + tx`, `y' = c·x + d·y + ty`, as `vs_analytic` and
/// `vs_nine_slice` apply it.
fn apply(linear: [f32; 4], translation: [f32; 2], p: [f32; 2]) -> [f32; 2] {
    [
        linear[0] * p[0] + linear[1] * p[1] + translation[0],
        linear[2] * p[0] + linear[3] * p[1] + translation[1],
    ]
}

/// The bound of a local quad's four corners through an affine.
fn quad_through(linear: [f32; 4], translation: [f32; 2], corners: [[f32; 2]; 4]) -> Points {
    let mut points = Points::new();
    points.inputs(&linear);
    points.inputs(&translation);
    for c in corners {
        points.inputs(&c);
        points.add(apply(linear, translation, c));
    }
    points
}

fn rect_corners(x: f32, y: f32, w: f32, h: f32) -> [[f32; 2]; 4] {
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

/// An analytic record: chrome, a shadow, stripes or a stroke segment.
pub(crate) fn analytic(target: &Target, record: &AnalyticInstance) -> PixelBox {
    let p = &record.payload;
    let (linear, translation) = (p[0], [p[1][0], p[1][1]]);
    let rect = p[2];
    debug_assert!(
        record.kind <= ANALYTIC_KIND_SEGMENT,
        "unknown analytic kind {}",
        record.kind
    );
    let content = match record.kind {
        // Chrome and stripes, with no transform: the rect, plus how far the
        // anti-aliasing reaches. `vs_analytic` tests the linear part exactly.
        ANALYTIC_KIND_CHROME | ANALYTIC_KIND_STRIPES
            if linear == [1.0, 0.0, 0.0, 1.0] && reach_holds(record) =>
        {
            let mut points = Points::new();
            points.inputs(&rect);
            for c in rect_corners(rect[0], rect[1], rect[2], rect[3]) {
                points.add([c[0] + translation[0], c[1] + translation[1]]);
            }
            points.inputs(&translation);
            points.bound(target, CHROME_REACH)
        }
        // A shadow's quad is its raster rect.
        ANALYTIC_KIND_SHADOW => quad_through(
            linear,
            translation,
            rect_corners(rect[0], rect[1], rect[2], rect[3]),
        )
        .bound(target, HARD_EDGE),
        ANALYTIC_KIND_SEGMENT => segment_quad(p).bound(target, HARD_EDGE),
        // Chrome and stripes under a transform, or whose fields the reach
        // above doesn't cover: the padded quad, 2 screen px per axis. An
        // untransformed record's quad pads only 1 logical px, so this is
        // larger than the GPU draws, never smaller.
        _ => {
            let axis_scale = [
                linear[0].hypot(linear[2]).max(1e-4),
                linear[1].hypot(linear[3]).max(1e-4),
            ];
            let pad = [2.0 / axis_scale[0], 2.0 / axis_scale[1]];
            let corners = rect_corners(
                rect[0] - pad[0],
                rect[1] - pad[1],
                rect[2] + 2.0 * pad[0],
                rect[3] + 2.0 * pad[1],
            );
            quad_through(linear, translation, corners).bound(target, HARD_EDGE)
        }
    };
    let clip = if p[1][2] > 0.5 {
        clip_bound(target, p[8])
    } else {
        Bound::Whole
    };
    content.intersect(clip).pixels(target)
}

/// Whether [`CHROME_REACH`] bounds what a chrome or stripe record draws past
/// its rect. It holds for corner radii of 0 or more, which `DrawList` clamps
/// to; a negative radius pulls the corner's field outwards and reaches
/// further. A non-finite field can turn the fragment's alpha into NaN, which
/// `alpha <= 0` doesn't discard, so the whole padded quad may be written.
/// `analytic_instances` is public, so a record edited by hand can be either.
fn reach_holds(record: &AnalyticInstance) -> bool {
    let p = &record.payload;
    p.iter().flatten().all(|v| v.is_finite())
        && (record.kind != ANALYTIC_KIND_CHROME || p[6].iter().all(|&r| r >= 0.0))
}

/// A stroke segment's quad, as `vs_analytic` builds it: along the segment,
/// past each end by its reach, the half width to either side, and the
/// anti-aliasing pad (1 logical px, or 2 screen px through a transform).
fn segment_quad(p: &[[f32; 4]; 10]) -> Points {
    let linear = p[0];
    let (a, b) = ([p[2][0], p[2][1]], [p[2][2], p[2][3]]);
    let span = [b[0] - a[0], b[1] - a[1]];
    let seg_len = span[0].hypot(span[1]).max(1e-6);
    let along = [span[0] / seg_len, span[1] / seg_len];
    let across = [-along[1], along[0]];
    let pad = if linear == [1.0, 0.0, 0.0, 1.0] {
        1.0
    } else {
        let det = linear[0] * linear[3] - linear[1] * linear[2];
        let norm = (linear.iter().map(|v| v * v).sum::<f32>()).sqrt();
        2.0 * norm / det.abs().max(1e-8)
    };
    let (start_reach, end_reach, half) = (p[5][2], p[5][3], p[1][3]);
    let us = [-start_reach - pad, seg_len + end_reach + pad];
    let vs = [-half - pad, half + pad];
    let local = |u: f32, v: f32| {
        [
            a[0] + along[0] * u + across[0] * v,
            a[1] + along[1] * u + across[1] * v,
        ]
    };
    let mut points = quad_through(
        linear,
        [p[1][0], p[1][1]],
        [
            local(us[0], vs[0]),
            local(us[1], vs[0]),
            local(us[1], vs[1]),
            local(us[0], vs[1]),
        ],
    );
    points.inputs(&p[2]);
    points.inputs(&[start_reach, end_reach, half, pad]);
    points
}

/// A circle: the quad `vs_circle` draws, the centre ± (radius + half the
/// ring's thickness + 2).
pub(crate) fn circle(target: &Target, c: &CircleInstance) -> PixelBox {
    let [cx, cy, radius, thickness] = c.center;
    let mut points = Points::new();
    points.inputs(&c.center);
    let extent = radius + thickness.max(0.0) * 0.5 + 2.0;
    points.add([cx - extent, cy - extent]);
    points.add([cx + extent, cy + extent]);
    let clip = if c.params[0] > 0.5 {
        clip_bound(target, c.clip)
    } else {
        Bound::Whole
    };
    points
        .bound(target, HARD_EDGE)
        .intersect(clip)
        .pixels(target)
}

/// A sprite, image or video frame: its four corners, as given.
pub(crate) fn icon(target: &Target, draw: &IconDraw) -> PixelBox {
    let mut points = Points::new();
    for c in draw.corners {
        points.add(c);
    }
    points
        .bound(target, HARD_EDGE)
        .intersect(optional_clip(target, draw.clip))
        .pixels(target)
}

/// A nine-slice panel: its local rect through its transform.
pub(crate) fn nine_slice(target: &Target, draw: &NineSliceDraw) -> PixelBox {
    let t = &draw.transform;
    let r = draw.local;
    quad_through(
        [t.a, t.b, t.c, t.d],
        [t.tx, t.ty],
        rect_corners(r.x, r.y, r.width, r.height),
    )
    .bound(target, HARD_EDGE)
    .intersect(optional_clip(target, draw.clip))
    .pixels(target)
}

/// The soup triangles `indices` (a `Soup` command's range) into `vertices`:
/// the union of each triangle's box.
///
/// A soup clip is interpolated across its triangle, so a triangle whose three
/// vertices all clip is cut by the bounding box of their clips: an
/// interpolated clip's edges lie between its vertices' edges. A triangle
/// where only some vertices clip is left uncut. An index past the end of
/// `vertices` gives the whole target, since what the GPU would draw isn't
/// known.
pub(crate) fn soup(target: &Target, vertices: &[Vertex], indices: &[u32]) -> PixelBox {
    let mut out = PixelBox::EMPTY;
    for tri in indices.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            vertices.get(tri[0] as usize),
            vertices.get(tri[1] as usize),
            vertices.get(tri[2] as usize),
        ) else {
            return target.whole();
        };
        let corner = |v: &Vertex| (v.position, v.clip, v.clip_enabled);
        out = out.union(&triangle(target, [corner(a), corner(b), corner(c)]));
    }
    out
}

/// MSDF quads, text glyphs or icons: each quad's two triangles, as the base
/// mesh splits it, their boxes cut by the quad's clip. The MSDF shader draws
/// its quads exactly as given.
pub(crate) fn msdf(target: &Target, instances: &[MsdfInstance]) -> PixelBox {
    let mut out = PixelBox::EMPTY;
    for quad in instances {
        let clip_on = if quad.clip_enabled() { 1.0 } else { 0.0 };
        for tri in QUAD_INDICES.chunks_exact(3) {
            let corner = |i: u16| (quad.corners[i as usize], quad.clip, clip_on);
            out = out.union(&triangle(
                target,
                [corner(tri[0]), corner(tri[1]), corner(tri[2])],
            ));
        }
    }
    out
}

/// Text block `block` as prepared: its glyph quads from every sweep (shadow,
/// glow, fill and outline).
pub(crate) fn text_block(target: &Target, prepared: &MsdfPrepared, block: usize) -> PixelBox {
    prepared
        .block_instances(block)
        .fold(PixelBox::EMPTY, |b, instances| {
            b.union(&msdf(target, instances))
        })
}

/// MSDF icon `icon` as prepared: the quad its glyph tile was fitted into.
#[cfg(feature = "phosphor-icons")]
pub(crate) fn icon_msdf(target: &Target, prepared: &MsdfPrepared, icon: u32) -> PixelBox {
    let range = prepared.icon_range(icon..icon + 1);
    msdf(
        target,
        &prepared.icon_instances[range.start as usize..range.end as usize],
    )
}

/// One triangle with its corners' positions and clips (`clip`, and whether
/// it's on).
fn triangle(target: &Target, corners: [([f32; 2], [f32; 4], f32); 3]) -> PixelBox {
    let mut points = Points::new();
    for (position, _, _) in corners {
        points.add(position);
    }
    let clip = if corners.iter().all(|&(_, _, on)| on > 0.5) {
        triangle_clip(target, corners.map(|(_, clip, _)| clip))
    } else {
        Bound::Whole
    };
    points
        .bound(target, HARD_EDGE)
        .intersect(clip)
        .pixels(target)
}

/// The bound of the clips a triangle interpolates between. When all three
/// are the same, the shader sees exactly that clip. Otherwise each pixel sees
/// a weighted mean of them: when all three widths, or all three heights, are
/// zero or negative, so is every mean, and the triangle holds nothing, as
/// `outside_clip` has it (a NaN fails the test, as it does there). Any other
/// mean lies inside the box around all three.
fn triangle_clip(target: &Target, clips: [[f32; 4]; 3]) -> Bound {
    if clips[0] == clips[1] && clips[1] == clips[2] {
        return clip_bound(target, clips[0]);
    }
    if clips.iter().all(|c| c[2] <= 0.0) || clips.iter().all(|c| c[3] <= 0.0) {
        return Bound::Nothing;
    }
    let mut points = Points::new();
    for c in clips {
        points.inputs(&c);
        points.add([c[0], c[1]]);
        points.add([c[0] + c[2], c[1] + c[3]]);
    }
    points.bound(target, HARD_EDGE)
}

#[cfg(test)]
#[path = "draw_box_tests.rs"]
mod tests;
