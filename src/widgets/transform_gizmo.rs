//! The manipulator drawn at a selection's pivot (Forge `TransformGizmo`):
//! move arrows and plane squares, rotate rings, or scale cubes, projected
//! through the camera's [`ViewRotation`].
//!
//! The gizmo owns its hit-testing and drag maths and reports
//! [`TransformEvent`]s: a start, a drag per frame the pointer moves, and an
//! end (or a cancel on Escape). Every value is the **total since the drag
//! started**, so the caller applies it to the transform it saved at the start
//! — no accumulated rounding — and a cancel restores that saved transform.
//!
//! Values are in the gizmo's frame: an axis or plane handle moves, scales
//! or rotates along the local [`basis`](TransformGizmo::basis) axes, while
//! the free-move centre reports a world-space delta and the view ring a turn
//! about the view axis.

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{CursorIcon, StyleKey, StyleResolver};

use super::stroke::Cap;
use super::view_gizmo::ViewRotation;
use super::{DragCapture, DragId, DrawContext, DrawList, Stroke};

/// What the gizmo manipulates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GizmoMode {
    /// Arrows along the axes, squares on the planes, a free-move centre.
    #[default]
    Move,
    /// A ring about each axis and an outer ring about the view axis.
    Rotate,
    /// Cubes on the axes and a uniform-scale centre.
    Scale,
}

/// A grabbable part of the gizmo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GizmoHandle {
    /// The X axis: its arrow, ring or cube.
    X,
    /// The Y axis.
    Y,
    /// The Z axis.
    Z,
    /// The XY plane square (move).
    XY,
    /// The XZ plane square (move).
    XZ,
    /// The YZ plane square (move).
    YZ,
    /// The centre square in move mode: move in the view plane.
    Free,
    /// The centre square in scale mode: scale all axes together.
    Uniform,
    /// The outer ring in rotate mode: turn about the view axis.
    View,
}

impl GizmoHandle {
    const AXES: [GizmoHandle; 3] = [GizmoHandle::X, GizmoHandle::Y, GizmoHandle::Z];
    const PLANES: [GizmoHandle; 3] = [GizmoHandle::XY, GizmoHandle::XZ, GizmoHandle::YZ];

    /// The axis (0 X, 1 Y, 2 Z) of an axis handle.
    pub const fn axis(self) -> Option<usize> {
        match self {
            GizmoHandle::X => Some(0),
            GizmoHandle::Y => Some(1),
            GizmoHandle::Z => Some(2),
            _ => None,
        }
    }

    /// The two axes of a plane handle.
    pub const fn plane(self) -> Option<[usize; 2]> {
        match self {
            GizmoHandle::XY => Some([0, 1]),
            GizmoHandle::XZ => Some([0, 2]),
            GizmoHandle::YZ => Some([1, 2]),
            _ => None,
        }
    }

    /// The axes this handle constrains to, as a mask (`[x, y, z]`): one for
    /// an axis, two for a plane, none for the free, uniform and view
    /// handles. What a readout shows as axis chips.
    pub const fn axes(self) -> [bool; 3] {
        match (self.axis(), self.plane()) {
            (Some(0), _) => [true, false, false],
            (Some(1), _) => [false, true, false],
            (Some(_), _) => [false, false, true],
            (None, Some([a, b])) => [a == 0 || b == 0, a == 1 || b == 1, a == 2 || b == 2],
            (None, None) => [false; 3],
        }
    }
}

/// A transform amount, in total since the drag started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GizmoValue {
    /// A move: per local axis for an axis or plane handle, in world space
    /// for [`GizmoHandle::Free`]. In scene units.
    Translate([f32; 3]),
    /// A turn about the handle's axis (or the view axis), in degrees,
    /// counter-clockwise looking down the axis toward the pivot.
    Rotate(f32),
    /// Scale factors per local axis.
    Scale([f32; 3]),
}

impl GizmoValue {
    /// The identity value of `mode`: no move, no turn, unit scale.
    pub const fn identity(mode: GizmoMode) -> Self {
        match mode {
            GizmoMode::Move => GizmoValue::Translate([0.0; 3]),
            GizmoMode::Rotate => GizmoValue::Rotate(0.0),
            GizmoMode::Scale => GizmoValue::Scale([1.0; 3]),
        }
    }
}

/// Where a [`TransformEvent`] falls in its drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformPhase {
    /// A handle was grabbed; the value is the identity. Save the transform.
    Start,
    /// The pointer moved; apply the value to the saved transform.
    Drag,
    /// The handle was released; the value is final. Commit (and record an
    /// undo step).
    End,
    /// Escape during the drag: restore the saved transform.
    Cancel,
}

/// One step of a gizmo drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformEvent {
    /// Where in the drag this is.
    pub phase: TransformPhase,
    /// The mode the drag started in.
    pub mode: GizmoMode,
    /// The handle being dragged.
    pub handle: GizmoHandle,
    /// The total since the drag started.
    pub value: GizmoValue,
}

/// Snap increments; `None` drags smoothly.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransformSnap {
    /// Move increment, in scene units.
    pub translate: Option<f32>,
    /// Rotate increment, in degrees.
    pub rotate: Option<f32>,
    /// Scale-factor increment.
    pub scale: Option<f32>,
}

/// What a [`TransformGizmo`] reports for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransformGizmoOutput {
    /// The handle under the pointer (none while a drag is on).
    pub hovered: Option<GizmoHandle>,
    /// The handle being dragged.
    pub dragging: Option<GizmoHandle>,
    /// This frame's step of a drag.
    pub event: Option<TransformEvent>,
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GizmoDrag {
    handle: GizmoHandle,
    mode: GizmoMode,
    /// Pointer and pivot at the press.
    press: [f32; 2],
    pivot: [f32; 2],
    /// The pointer last frame.
    last_pointer: [f32; 2],
    /// Rotate: the grab angle about the pivot, the angle last frame, the
    /// unwrapped turn so far, the turn's sign and the grabbed ring angle.
    a0: f32,
    prev: f32,
    unwrap: f32,
    sign: f32,
    t0: f32,
    value: GizmoValue,
}

/// A [`TransformGizmo`]'s drag. Caller-owned; persists across frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TransformGizmoState {
    drag: Option<GizmoDrag>,
}

impl TransformGizmoState {
    /// No drag.
    pub fn new() -> Self {
        Self::default()
    }

    /// The handle being dragged.
    pub fn dragging(&self) -> Option<GizmoHandle> {
        self.drag.map(|d| d.handle)
    }
}

/// The default arm length, in px.
const SIZE: f32 = 92.0;
/// Room round the arms for caps, labels and the view ring.
const PAD: f32 = 26.0;
/// Scene units per px of drag at the default size.
const UNITS_PER_PX: f32 = 0.02;
/// An axis this short on screen (as a fraction of the arm) points at the
/// camera: it fades. Shorter than [`DEGENERATE`] it stops moving.
const FORESHORTENED: f32 = 0.12;
const DEGENERATE: f32 = 0.05;
const FORESHORTENED_ALPHA: f32 = 0.35;
/// A plane square smaller than this (as a fraction of the arm squared) is
/// edge-on: hidden.
const EDGE_ON: f32 = 0.08;
/// The plane squares span this range along both of their axes.
const PLANE_SPAN: [f32; 2] = [0.22, 0.42];
const PLANE_FILL: f32 = 0.28;
const PLANE_FILL_HOT: f32 = 0.55;
/// The axis arms start this far out from the pivot.
const ARM_START: f32 = 0.16;
/// Arm stroke widths, and the hit width round them.
const ARM: f32 = 2.0;
const ARM_HOT: f32 = 3.0;
const ARM_HIT: f32 = 7.0;
/// The move arrowhead's length and half-width; the scale cube's half-side.
const ARROW_LENGTH: f32 = 15.0;
const ARROW_HALF: f32 = 5.0;
const CUBE_HALF: f32 = 5.0;
/// Outline of the arrowheads and cubes (`rgba(0,0,0,.5)`, 0.75 px).
const CAP_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
const CAP_EDGE_WIDTH: f32 = 0.75;
/// Axis labels sit this far past the tip, mono 10 bold.
const LABEL_OFFSET: f32 = 24.0;
const LABEL_SIZE: f32 = 10.0;
/// The centre square's half-side and edge width.
const CENTRE_HALF: f32 = 6.0;
const CENTRE_EDGE: f32 = 1.25;
/// Handles other than the active one fade to these while a drag is on.
const IDLE_ALPHA: f32 = 0.25;
const IDLE_AXIS_ALPHA: f32 = 0.2;
/// `rgba(241,245,249,…)`: the gizmo's white.
const WHITE: [f32; 3] = [0.945, 0.961, 0.976];
/// Rotate: ring samples round a full turn, and a visible side's tolerance.
const RING_SAMPLES: usize = 72;
const PICK_SAMPLES: usize = 144;
const FRONT_DEPTH: f32 = 0.02;
const BACK_RING_ALPHA: f32 = 0.28;
const BACK_RING: f32 = 1.5;
const RING_HIT: f32 = 6.0;
/// The sweep wedge's arc: a sample every 5°, at most this many (past two
/// turns the samples spread out).
const WEDGE_STEP: f32 = std::f32::consts::PI / 36.0;
const WEDGE_MAX: usize = 144;
/// The view ring sits this far outside the arm length.
const VIEW_RING_GAP: f32 = 14.0;
/// The drop shadow under everything (`drop-shadow(0 1px 1px rgba(0,0,0,.7))`).
const SHADOW_OFFSET: [f32; 2] = [0.0, 1.0];
const SHADOW_ALPHA: f32 = 0.7;

/// The manipulator drawn at the selection's pivot (Forge `TransformGizmo`).
/// Built fresh each frame; its drag lives in a caller-owned
/// [`TransformGizmoState`].
#[derive(Clone, Copy, Debug)]
pub struct TransformGizmo {
    id: DragId,
    mode: GizmoMode,
    rotation: ViewRotation,
    basis: [[f32; 3]; 3],
    size: f32,
    units_per_px: f32,
    snap: TransformSnap,
    active: Option<GizmoHandle>,
}

/// One frame's projection of the gizmo.
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// The pivot on screen.
    c: [f32; 2],
    /// Each local axis on screen at full arm length, px.
    arm: [[f32; 2]; 3],
    /// Each local axis's depth (toward the viewer +).
    depth: [f32; 3],
    /// Each local axis's on-screen length as a fraction of the arm.
    len: [f32; 3],
    /// Each world axis on screen, per unit (for the free move).
    world: [[f32; 2]; 3],
}

/// How a paint pass colours: the gizmo itself, or its drop shadow.
#[derive(Clone, Copy, Debug)]
struct Pass {
    offset: [f32; 2],
    shadow: bool,
}

impl Pass {
    fn ink(&self, color: [f32; 4]) -> [f32; 4] {
        if self.shadow {
            [0.0, 0.0, 0.0, color[3] * SHADOW_ALPHA]
        } else {
            color
        }
    }

    fn at(&self, p: [f32; 2]) -> [f32; 2] {
        [p[0] + self.offset[0], p[1] + self.offset[1]]
    }
}

fn white(alpha: f32) -> [f32; 4] {
    [WHITE[0], WHITE[1], WHITE[2], alpha]
}

fn with_alpha(mut c: [f32; 4], alpha: f32) -> [f32; 4] {
    c[3] *= alpha;
    c
}

fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn add(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + b[0] * t, a[1] + b[1] * t]
}

/// Round `v` to a multiple of `step`, if any.
fn quantize(v: f32, step: Option<f32>) -> f32 {
    match step {
        Some(s) if s > 0.0 => (v / s).round() * s,
        _ => v,
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let len2 = dot2(ab, ab);
    let t = if len2 > 0.0 {
        (dot2(ap, ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = add(a, ab, t);
    (p[0] - q[0]).hypot(p[1] - q[1])
}

/// True when `p` is inside the convex quad `q` (either winding).
fn in_quad(p: [f32; 2], q: &[[f32; 2]; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

impl TransformGizmo {
    /// A move gizmo under drag id `id`, seen through `rotation`.
    pub const fn new(id: DragId, rotation: ViewRotation) -> Self {
        Self {
            id,
            mode: GizmoMode::Move,
            rotation,
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            size: SIZE,
            units_per_px: UNITS_PER_PX,
            snap: TransformSnap {
                translate: None,
                rotate: None,
                scale: None,
            },
            active: None,
        }
    }

    /// Switch to `mode`.
    pub const fn mode(mut self, mode: GizmoMode) -> Self {
        self.mode = mode;
        self
    }

    /// The gizmo's axes in world space (rows), for a local-space gizmo on a
    /// rotated object. World-aligned by default.
    pub const fn basis(mut self, basis: [[f32; 3]; 3]) -> Self {
        self.basis = basis;
        self
    }

    /// The arm length, in px (92 by default).
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Scene units per px of pointer travel along an arm (0.02 by default).
    pub const fn units_per_px(mut self, units: f32) -> Self {
        self.units_per_px = units;
        self
    }

    /// Snap increments.
    pub const fn snap(mut self, snap: TransformSnap) -> Self {
        self.snap = snap;
        self
    }

    /// Light `handle` as active although no drag is on, for a transform
    /// driven from the keyboard ("G X").
    pub const fn active(mut self, handle: Option<GizmoHandle>) -> Self {
        self.active = handle;
        self
    }

    /// The area the gizmo can paint round `pivot`: arms, caps, labels and
    /// the view ring.
    pub fn bounds(&self, pivot: [f32; 2]) -> Rect {
        let half = self.size + PAD;
        Rect::new(pivot[0] - half, pivot[1] - half, half * 2.0, half * 2.0)
    }

    fn frame(&self, c: [f32; 2]) -> Frame {
        let mut arm = [[0.0; 2]; 3];
        let mut depth = [0.0; 3];
        let mut len = [0.0; 3];
        let mut world = [[0.0; 2]; 3];
        for i in 0..3 {
            let [x, y, d] = self.rotation.project(self.basis[i]);
            arm[i] = [x * self.size, y * self.size];
            depth[i] = d;
            len[i] = x.hypot(y);
            let mut unit = [0.0; 3];
            unit[i] = 1.0;
            let [wx, wy, _] = self.rotation.project(unit);
            world[i] = [wx, wy];
        }
        Frame {
            c,
            arm,
            depth,
            len,
            world,
        }
    }

    /// The point on axis `i`'s ring at in-plane angle `t`: `[x, y, depth]`.
    /// The ring's plane is spanned by the next two axes, so +t turns
    /// right-handed about axis `i`.
    fn ring_at(&self, fr: &Frame, i: usize, t: f32) -> [f32; 3] {
        let u = self.basis[(i + 1) % 3];
        let v = self.basis[(i + 2) % 3];
        let (sin, cos) = t.sin_cos();
        let w = [
            u[0] * cos + v[0] * sin,
            u[1] * cos + v[1] * sin,
            u[2] * cos + v[2] * sin,
        ];
        let [x, y, d] = self.rotation.project(w);
        [fr.c[0] + x * self.size, fr.c[1] + y * self.size, d]
    }

    fn ring(&self, fr: &Frame, i: usize) -> [[f32; 3]; RING_SAMPLES + 1] {
        std::array::from_fn(|k| {
            self.ring_at(
                fr,
                i,
                k as f32 / RING_SAMPLES as f32 * std::f32::consts::TAU,
            )
        })
    }

    /// The plane square of `handle` (two axes), or `None` while edge-on.
    fn plane_quad(&self, fr: &Frame, [m, n]: [usize; 2]) -> Option<[[f32; 2]; 4]> {
        let (a, b) = (fr.arm[m], fr.arm[n]);
        let area = (a[0] * b[1] - a[1] * b[0]).abs() / (self.size * self.size);
        if area < EDGE_ON {
            return None;
        }
        let [k, l] = PLANE_SPAN;
        let pt = |u: f32, v: f32| [fr.c[0] + a[0] * u + b[0] * v, fr.c[1] + a[1] * u + b[1] * v];
        Some([pt(k, k), pt(l, k), pt(l, l), pt(k, l)])
    }

    /// The axes back to front.
    fn depth_order(fr: &Frame) -> [usize; 3] {
        let mut order = [0, 1, 2];
        order.sort_by(|&a, &b| fr.depth[a].total_cmp(&fr.depth[b]));
        order
    }

    /// The handle under `p`, in paint order from the top.
    fn hit(&self, fr: &Frame, p: [f32; 2]) -> Option<GizmoHandle> {
        match self.mode {
            GizmoMode::Move | GizmoMode::Scale => {
                let centre = Rect::new(
                    fr.c[0] - CENTRE_HALF,
                    fr.c[1] - CENTRE_HALF,
                    CENTRE_HALF * 2.0,
                    CENTRE_HALF * 2.0,
                );
                if p[0] >= centre.x
                    && p[0] <= centre.right()
                    && p[1] >= centre.y
                    && p[1] <= centre.bottom()
                {
                    return Some(if self.mode == GizmoMode::Move {
                        GizmoHandle::Free
                    } else {
                        GizmoHandle::Uniform
                    });
                }
                let cap = if self.mode == GizmoMode::Move {
                    ARROW_LENGTH
                } else {
                    CUBE_HALF
                };
                for &i in Self::depth_order(fr).iter().rev() {
                    let arm = fr.arm[i];
                    let len = arm[0].hypot(arm[1]);
                    let start = add(fr.c, arm, ARM_START);
                    let end = if len > 0.0 {
                        add(add(fr.c, arm, 1.0), arm, cap / len)
                    } else {
                        fr.c
                    };
                    if segment_distance(p, start, end) <= ARM_HIT {
                        return Some(GizmoHandle::AXES[i]);
                    }
                }
                if self.mode == GizmoMode::Move {
                    for handle in GizmoHandle::PLANES.iter().rev() {
                        let quad = self.plane_quad(fr, handle.plane().unwrap_or([0, 1]));
                        if quad.is_some_and(|q| in_quad(p, &q)) {
                            return Some(*handle);
                        }
                    }
                }
                None
            }
            GizmoMode::Rotate => {
                for i in (0..3).rev() {
                    let ring = self.ring(fr, i);
                    let near = ring.windows(2).any(|w| {
                        w[0][2] >= -FRONT_DEPTH
                            && w[1][2] >= -FRONT_DEPTH
                            && segment_distance(p, [w[0][0], w[0][1]], [w[1][0], w[1][1]])
                                <= RING_HIT
                    });
                    if near {
                        return Some(GizmoHandle::AXES[i]);
                    }
                }
                let r = (p[0] - fr.c[0]).hypot(p[1] - fr.c[1]);
                ((r - (self.size + VIEW_RING_GAP)).abs() <= RING_HIT).then_some(GizmoHandle::View)
            }
        }
    }

    /// The value of a drag to pointer `p`, updating the rotate unwrap.
    fn drag_value(&self, fr: &Frame, drag: &mut GizmoDrag, p: [f32; 2]) -> GizmoValue {
        let d = [p[0] - drag.press[0], p[1] - drag.press[1]];
        let snap = self.snap;
        match drag.mode {
            GizmoMode::Rotate => {
                let a = (p[1] - drag.pivot[1]).atan2(p[0] - drag.pivot[0]);
                let mut da = a - drag.prev;
                if da > std::f32::consts::PI {
                    da -= std::f32::consts::TAU;
                }
                if da < -std::f32::consts::PI {
                    da += std::f32::consts::TAU;
                }
                drag.unwrap += da;
                drag.prev = a;
                GizmoValue::Rotate(quantize(drag.unwrap.to_degrees() * drag.sign, snap.rotate))
            }
            GizmoMode::Move => {
                let mut value = [0.0; 3];
                let upp = self.units_per_px;
                match drag.handle {
                    GizmoHandle::Free => {
                        for (i, w) in fr.world.iter().enumerate() {
                            value[i] = quantize(dot2(*w, d) * upp, snap.translate);
                        }
                    }
                    handle => {
                        if let Some(i) = handle.axis() {
                            let s = fr.arm[i];
                            let t = if fr.len[i] < DEGENERATE {
                                0.0
                            } else {
                                dot2(d, s) / dot2(s, s) * self.size * upp
                            };
                            value[i] = quantize(t, snap.translate);
                        } else if let Some([m, n]) = handle.plane() {
                            let (a, b) = (fr.arm[m], fr.arm[n]);
                            let det = a[0] * b[1] - a[1] * b[0];
                            if det.abs() > 1.0 {
                                let k = self.size * upp / det;
                                value[m] =
                                    quantize((d[0] * b[1] - d[1] * b[0]) * k, snap.translate);
                                value[n] =
                                    quantize((a[0] * d[1] - a[1] * d[0]) * k, snap.translate);
                            }
                        }
                    }
                }
                GizmoValue::Translate(value)
            }
            GizmoMode::Scale => {
                if drag.handle == GizmoHandle::Uniform {
                    let f = quantize((1.0 + (d[0] - d[1]) / self.size).max(0.01), snap.scale);
                    GizmoValue::Scale([f; 3])
                } else {
                    let i = drag.handle.axis().unwrap_or(0);
                    let s = fr.arm[i];
                    let along = if fr.len[i] < DEGENERATE {
                        0.0
                    } else {
                        dot2(d, s) / dot2(s, s)
                    };
                    let mut value = [1.0; 3];
                    value[i] = quantize((1.0 + along).max(0.01), snap.scale);
                    GizmoValue::Scale(value)
                }
            }
        }
    }

    /// Begin a drag on `handle` at pointer `p`.
    fn begin(&self, fr: &Frame, handle: GizmoHandle, p: [f32; 2]) -> GizmoDrag {
        let a0 = (p[1] - fr.c[1]).atan2(p[0] - fr.c[0]);
        let mut drag = GizmoDrag {
            handle,
            mode: self.mode,
            press: p,
            pivot: fr.c,
            last_pointer: p,
            a0,
            prev: a0,
            unwrap: 0.0,
            sign: 1.0,
            t0: 0.0,
            value: GizmoValue::identity(self.mode),
        };
        if self.mode == GizmoMode::Rotate {
            if let Some(i) = handle.axis() {
                // The ring's sign follows which way its axis faces, so a
                // turn reads the same from either side.
                let d = fr.depth[i];
                drag.sign = -if d == 0.0 { 1.0 } else { d.signum() };
                let mut best = f32::INFINITY;
                for k in 0..PICK_SAMPLES {
                    let t = k as f32 / PICK_SAMPLES as f32 * std::f32::consts::TAU;
                    let q = self.ring_at(fr, i, t);
                    if q[2] < -FRONT_DEPTH {
                        continue;
                    }
                    let dd = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                    if dd < best {
                        best = dd;
                        drag.t0 = t;
                    }
                }
            } else {
                drag.sign = -1.0;
            }
        }
        drag
    }

    /// Draw the gizmo round `pivot` (screen px) and handle its drag.
    pub fn draw(
        &self,
        pivot: [f32; 2],
        state: &mut TransformGizmoState,
        drag: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> TransformGizmoOutput {
        let input = ctx.input;
        let p = [input.mouse_x, input.mouse_y];
        let fr = self.frame(pivot);
        let mut out = TransformGizmoOutput::default();

        // A drag that lost its capture (cleared by the host) is over.
        if state.drag.is_some() && !drag.is_active(self.id) {
            state.drag = None;
        }

        if let Some(mut current) = state.drag {
            if input.key_escape {
                out.event = Some(TransformEvent {
                    phase: TransformPhase::Cancel,
                    mode: current.mode,
                    handle: current.handle,
                    value: current.value,
                });
                state.drag = None;
                drag.release(self.id);
            } else if !input.mouse_down {
                out.event = Some(TransformEvent {
                    phase: TransformPhase::End,
                    mode: current.mode,
                    handle: current.handle,
                    value: current.value,
                });
                state.drag = None;
                drag.release(self.id);
            } else {
                if p != current.last_pointer {
                    current.value = self.drag_value(&fr, &mut current, p);
                    current.last_pointer = p;
                    out.event = Some(TransformEvent {
                        phase: TransformPhase::Drag,
                        mode: current.mode,
                        handle: current.handle,
                        value: current.value,
                    });
                }
                state.drag = Some(current);
            }
        } else if !input.mouse_consumed && (drag.is_free() || drag.is_active(self.id)) {
            out.hovered = self.hit(&fr, p);
            if let Some(handle) = out.hovered
                && input.mouse_clicked
                && drag.try_begin(self.id)
            {
                let started = self.begin(&fr, handle, p);
                out.event = Some(TransformEvent {
                    phase: TransformPhase::Start,
                    mode: started.mode,
                    handle,
                    value: started.value,
                });
                state.drag = Some(started);
                out.hovered = None;
            }
        }
        out.dragging = state.dragging();

        if out.dragging.is_some() {
            ctx.request_cursor(CursorIcon::Grabbing);
        } else if out.hovered.is_some() {
            ctx.request_cursor(CursorIcon::Pointer);
        }

        let s = ctx.styles();
        let active = self.active.or(out.dragging);
        let hot = active.or(out.hovered);
        ctx.push_debug_scope_rect("TransformGizmo", self.bounds(pivot));
        for pass in [
            Pass {
                offset: SHADOW_OFFSET,
                shadow: true,
            },
            Pass {
                offset: [0.0; 2],
                shadow: false,
            },
        ] {
            self.paint(
                ctx.draw_list,
                &s,
                &fr,
                state.drag.as_ref(),
                active,
                hot,
                pass,
            );
        }
        ctx.pop_debug_scope();
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        fr: &Frame,
        drag: Option<&GizmoDrag>,
        active: Option<GizmoHandle>,
        hot: Option<GizmoHandle>,
        pass: Pass,
    ) {
        match self.mode {
            GizmoMode::Move | GizmoMode::Scale => {
                self.paint_arms(list, s, fr, drag, active, hot, pass)
            }
            GizmoMode::Rotate => self.paint_rings(list, s, fr, drag, active, hot, pass),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_arms(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        fr: &Frame,
        drag: Option<&GizmoDrag>,
        active: Option<GizmoHandle>,
        hot: Option<GizmoHandle>,
        pass: Pass,
    ) {
        let accent = s.color(StyleKey::AccentMatch);
        let c = pass.at(fr.c);

        // The start ghost: where the pivot was when the move began.
        if let Some(GizmoDrag {
            handle,
            value: GizmoValue::Translate(v),
            mode: GizmoMode::Move,
            ..
        }) = drag
        {
            let off = self.screen_offset(fr, *handle, *v);
            let ghost = [c[0] - off[0], c[1] - off[1]];
            list.stroke_line(
                ghost,
                c,
                &Stroke::new(1.0).dashed(3.0, 3.0),
                pass.ink(white(0.5)),
            );
            list.chrome_rect(
                Rect::new(ghost[0] - 3.5, ghost[1] - 3.5, 7.0, 7.0),
                3.5,
                1.0,
                [0.0; 4],
                pass.ink(white(0.6)),
            );
        }

        if self.mode == GizmoMode::Move {
            for handle in GizmoHandle::PLANES {
                let Some(plane) = handle.plane() else {
                    continue;
                };
                let Some(quad) = self.plane_quad(fr, plane) else {
                    continue;
                };
                let on = hot == Some(handle);
                let third = 3 - plane[0] - plane[1];
                let tint = if on {
                    accent
                } else {
                    s.color(StyleKey::AXES[third])
                };
                let fade = if active.is_some() && !on {
                    IDLE_ALPHA
                } else {
                    1.0
                };
                let fill = with_alpha(tint, if on { PLANE_FILL_HOT } else { PLANE_FILL } * fade);
                let points = quad.map(|q| pass.at(q));
                list.filled_polygon(&points.map(|q| (q[0], q[1])), pass.ink(fill));
                list.stroke_closed(&points, &Stroke::new(1.0), pass.ink(with_alpha(tint, fade)));
            }
        }

        let mono = s.theme().mono_font.clone();
        for i in Self::depth_order(fr) {
            let handle = GizmoHandle::AXES[i];
            let on = hot == Some(handle);
            let off = active.is_some() && !on;
            let tint = if on {
                accent
            } else {
                s.color(StyleKey::AXES[i])
            };
            let alpha = if off {
                IDLE_AXIS_ALPHA
            } else if fr.len[i] < FORESHORTENED {
                FORESHORTENED_ALPHA
            } else {
                1.0
            };
            let col = pass.ink(with_alpha(tint, alpha));
            let arm = fr.arm[i];
            let tip = add(c, arm, 1.0);
            list.stroke_line(
                add(c, arm, ARM_START),
                tip,
                &Stroke::new(if on { ARM_HOT } else { ARM }).cap(Cap::Round),
                col,
            );
            let len = arm[0].hypot(arm[1]);
            let dir = if len > 1e-3 {
                [arm[0] / len, arm[1] / len]
            } else {
                [1.0, 0.0]
            };
            let perp = [-dir[1], dir[0]];
            let local = |x: f32, y: f32| {
                [
                    tip[0] + dir[0] * x + perp[0] * y,
                    tip[1] + dir[1] * x + perp[1] * y,
                ]
            };
            let edge = pass.ink(with_alpha(CAP_EDGE, alpha));
            if self.mode == GizmoMode::Move {
                let head = [
                    local(0.0, -ARROW_HALF),
                    local(ARROW_LENGTH, 0.0),
                    local(0.0, ARROW_HALF),
                ];
                list.filled_polygon(&head.map(|q| (q[0], q[1])), col);
                list.stroke_closed(&head, &Stroke::new(CAP_EDGE_WIDTH), edge);
            } else {
                let cube = [
                    local(-CUBE_HALF, -CUBE_HALF),
                    local(CUBE_HALF, -CUBE_HALF),
                    local(CUBE_HALF, CUBE_HALF),
                    local(-CUBE_HALF, CUBE_HALF),
                ];
                list.filled_polygon(&cube.map(|q| (q[0], q[1])), col);
                list.stroke_closed(&cube, &Stroke::new(CAP_EDGE_WIDTH), edge);
            }
            let label = ["X", "Y", "Z"][i];
            let at = add(tip, dir, LABEL_OFFSET);
            let width = list
                .measure_text_with_font(label, LABEL_SIZE, None, mono.as_ref())
                .0;
            let y = list.vcentered_text_y(
                at[1] - LABEL_SIZE * 0.5,
                LABEL_SIZE,
                LABEL_SIZE,
                mono.as_ref(),
                label,
            );
            list.text(
                TextBlock::new(label, at[0] - width * 0.5, y)
                    .with_size(LABEL_SIZE)
                    .with_weight(crate::Weight::BOLD)
                    .with_color_f32(col)
                    .with_font_opt(mono.clone()),
            );
        }

        let centre = if self.mode == GizmoMode::Move {
            GizmoHandle::Free
        } else {
            GizmoHandle::Uniform
        };
        let on = hot == Some(centre);
        let fade = if active.is_some() && !on {
            IDLE_ALPHA
        } else {
            1.0
        };
        let fill = if on {
            accent
        } else if self.mode == GizmoMode::Scale {
            white(0.85)
        } else {
            white(0.12)
        };
        let edge = if on { accent } else { white(0.85) };
        list.chrome_rect(
            Rect::new(
                c[0] - CENTRE_HALF - CENTRE_EDGE * 0.5,
                c[1] - CENTRE_HALF - CENTRE_EDGE * 0.5,
                (CENTRE_HALF + CENTRE_EDGE * 0.5) * 2.0,
                (CENTRE_HALF + CENTRE_EDGE * 0.5) * 2.0,
            ),
            0.0,
            CENTRE_EDGE,
            pass.ink(with_alpha(fill, fade)),
            pass.ink(with_alpha(edge, fade)),
        );
    }

    /// The screen offset of a move by `v` on `handle`: where the start
    /// ghost sits behind the pivot.
    fn screen_offset(&self, fr: &Frame, handle: GizmoHandle, v: [f32; 3]) -> [f32; 2] {
        let mut off = [0.0; 2];
        if handle == GizmoHandle::Free {
            for (i, w) in fr.world.iter().enumerate() {
                off = add(off, *w, v[i] / self.units_per_px);
            }
        } else {
            for (i, a) in fr.arm.iter().enumerate() {
                off = add(off, *a, v[i] / (self.size * self.units_per_px));
            }
        }
        off
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_rings(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        fr: &Frame,
        drag: Option<&GizmoDrag>,
        active: Option<GizmoHandle>,
        hot: Option<GizmoHandle>,
        pass: Pass,
    ) {
        let accent = s.color(StyleKey::AccentMatch);
        let c = pass.at(fr.c);
        let r = self.size;
        let mut run: [[f32; 2]; RING_SAMPLES + 1] = [[0.0; 2]; RING_SAMPLES + 1];

        // The sweep wedge, under everything.
        if let Some(GizmoDrag {
            handle,
            value: GizmoValue::Rotate(sweep),
            a0,
            sign,
            t0,
            ..
        }) = drag
            && *sweep != 0.0
        {
            let sw = sweep.to_radians();
            let n = ((sw.abs() / WEDGE_STEP).ceil() as usize).clamp(2, WEDGE_MAX);
            let col = match handle.axis() {
                Some(i) => s.color(StyleKey::AXES[i]),
                None => white(1.0),
            };
            // A fan from the pivot: C, then the arc.
            let mut fan = [(0.0f32, 0.0f32); WEDGE_MAX + 2];
            let mut outline = [[0.0f32; 2]; WEDGE_MAX + 2];
            fan[0] = (c[0], c[1]);
            for k in 0..=n {
                let f = k as f32 / n as f32;
                let q = match handle.axis() {
                    Some(i) => {
                        let q = self.ring_at(fr, i, t0 + sw * f);
                        pass.at([q[0], q[1]])
                    }
                    None => {
                        let t = a0 + sw * sign * f;
                        let rr = r + VIEW_RING_GAP;
                        [c[0] + t.cos() * rr, c[1] + t.sin() * rr]
                    }
                };
                fan[k + 1] = (q[0], q[1]);
            }
            let fan = &fan[..n + 2];
            for (o, &(x, y)) in outline.iter_mut().zip(fan) {
                *o = [x, y];
            }
            let outline = &outline[..n + 2];
            list.filled_polygon(fan, pass.ink(with_alpha(col, 0.16)));
            list.stroke_closed(outline, &Stroke::new(1.0), pass.ink(with_alpha(col, 0.6)));
            let p0 = outline[1];
            let p1 = outline[n + 1];
            list.stroke_line(
                c,
                p0,
                &Stroke::new(1.0).dashed(3.0, 3.0),
                pass.ink(white(0.45)),
            );
            list.stroke_line(c, p1, &Stroke::new(1.0), pass.ink(white(0.9)));
        }

        // Back halves: dim, not grabbable.
        for i in 0..3 {
            let handle = GizmoHandle::AXES[i];
            if active.is_some() && hot != Some(handle) {
                continue;
            }
            let tint = if hot == Some(handle) {
                accent
            } else {
                s.color(StyleKey::AXES[i])
            };
            let ring = self.ring(fr, i);
            stroke_runs(
                list,
                &ring,
                &mut run,
                |d| d <= FRONT_DEPTH,
                pass,
                &Stroke::new(BACK_RING),
                pass.ink(with_alpha(tint, BACK_RING_ALPHA)),
            );
        }

        // The occluder disc, then the view ring.
        list.chrome_rect(
            Rect::new(c[0] - r, c[1] - r, r * 2.0, r * 2.0),
            r,
            1.0,
            pass.ink([0.039, 0.051, 0.059, 0.12]),
            pass.ink(white(0.14)),
        );
        let view_on = hot == Some(GizmoHandle::View);
        let view_fade = if active.is_some() && !view_on {
            IDLE_AXIS_ALPHA
        } else {
            1.0
        };
        let (view_col, view_w) = if view_on {
            (accent, 2.5)
        } else {
            (white(0.7), 1.5)
        };
        let vr = r + VIEW_RING_GAP + view_w * 0.5;
        list.chrome_rect(
            Rect::new(c[0] - vr, c[1] - vr, vr * 2.0, vr * 2.0),
            vr,
            view_w,
            [0.0; 4],
            pass.ink(with_alpha(view_col, view_fade)),
        );

        // Front halves: the grabbable arcs.
        for i in 0..3 {
            let handle = GizmoHandle::AXES[i];
            let on = hot == Some(handle);
            let fade = if active.is_some() && !on {
                IDLE_AXIS_ALPHA
            } else {
                1.0
            };
            let tint = if on {
                accent
            } else {
                s.color(StyleKey::AXES[i])
            };
            let ring = self.ring(fr, i);
            stroke_runs(
                list,
                &ring,
                &mut run,
                |d| d >= -FRONT_DEPTH,
                pass,
                &Stroke::new(if on { ARM_HOT } else { ARM }).cap(Cap::Round),
                pass.ink(with_alpha(tint, fade)),
            );
        }

        // The pivot.
        list.chrome_rect(
            Rect::new(c[0] - 2.5, c[1] - 2.5, 5.0, 5.0),
            2.5,
            0.0,
            pass.ink(white(0.9)),
            [0.0; 4],
        );
    }
}

/// Stroke each run of `ring` samples whose depth passes `visible`, as its
/// own polyline (the pen lifts across the hidden side).
fn stroke_runs(
    list: &mut DrawList,
    ring: &[[f32; 3]],
    run: &mut [[f32; 2]],
    visible: impl Fn(f32) -> bool,
    pass: Pass,
    stroke: &Stroke,
    color: [f32; 4],
) {
    let mut n = 0;
    for q in ring {
        if visible(q[2]) {
            run[n] = pass.at([q[0], q[1]]);
            n += 1;
        } else {
            if n >= 2 {
                list.stroke_polyline(&run[..n], stroke, color);
            }
            n = 0;
        }
    }
    if n >= 2 {
        list.stroke_polyline(&run[..n], stroke, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    const C: [f32; 2] = [300.0, 300.0];

    struct Rig {
        theme: Theme,
        state: TransformGizmoState,
        drag: DragCapture,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                theme: Theme::default(),
                state: TransformGizmoState::new(),
                drag: DragCapture::new(),
            }
        }

        fn frame(&mut self, gizmo: &TransformGizmo, input: &InputState) -> TransformGizmoOutput {
            let mut list = DrawList::new();
            let mut focus = FocusState::new();
            let mut ctx = DrawContext::new(&mut list, &mut focus, &self.theme, input, 800.0, 600.0);
            gizmo.draw(C, &mut self.state, &mut self.drag, &mut ctx)
        }

        /// Press at `from`, move through `path`, release; returns every
        /// event in order.
        fn gesture(
            &mut self,
            gizmo: &TransformGizmo,
            from: [f32; 2],
            path: &[[f32; 2]],
        ) -> Vec<TransformEvent> {
            let mut events = Vec::new();
            events.extend(self.frame(gizmo, &pointer(from, true, true)).event);
            for &p in path {
                events.extend(self.frame(gizmo, &pointer(p, true, false)).event);
            }
            let end = path.last().copied().unwrap_or(from);
            events.extend(self.frame(gizmo, &pointer(end, false, false)).event);
            events
        }
    }

    fn pointer(at: [f32; 2], down: bool, clicked: bool) -> InputState {
        InputState {
            mouse_x: at[0],
            mouse_y: at[1],
            mouse_down: down,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    fn at(dx: f32, dy: f32) -> [f32; 2] {
        [C[0] + dx, C[1] + dy]
    }

    fn front() -> TransformGizmo {
        TransformGizmo::new(1, ViewRotation::IDENTITY)
    }

    fn translate(e: &TransformEvent) -> [f32; 3] {
        match e.value {
            GizmoValue::Translate(v) => v,
            other => panic!("{other:?}"),
        }
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn a_move_along_an_axis_reports_start_drags_and_end_as_totals() {
        let mut rig = Rig::new();
        let events = rig.gesture(&front(), at(50.0, 0.0), &[at(60.0, 4.0), at(96.0, -3.0)]);
        let phases: Vec<_> = events.iter().map(|e| e.phase).collect();
        assert_eq!(
            phases,
            [
                TransformPhase::Start,
                TransformPhase::Drag,
                TransformPhase::Drag,
                TransformPhase::End
            ]
        );
        assert!(events.iter().all(|e| e.handle == GizmoHandle::X));
        assert_eq!(translate(&events[0]), [0.0; 3]);
        // 10 px along X, then 46: px × 0.02 units, off-axis motion ignored.
        assert!(close3(translate(&events[1]), [0.2, 0.0, 0.0]));
        assert!(close3(translate(&events[2]), [0.92, 0.0, 0.0]));
        assert_eq!(
            events[3].value, events[2].value,
            "end repeats the last total"
        );
        assert!(rig.drag.is_free());
    }

    #[test]
    fn a_move_snaps_to_its_increment() {
        let gizmo = front().snap(TransformSnap {
            translate: Some(0.5),
            ..TransformSnap::default()
        });
        let events = Rig::new().gesture(&gizmo, at(50.0, 0.0), &[at(96.0, 0.0)]);
        assert!(close3(translate(&events[1]), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn a_plane_square_moves_on_both_of_its_axes() {
        // The XY square spans 0.22–0.42 of the arm; Y runs up the screen.
        let grab = at(0.3 * SIZE, -0.3 * SIZE);
        let mut rig = Rig::new();
        assert_eq!(
            rig.frame(&front(), &pointer(grab, false, false)).hovered,
            Some(GizmoHandle::XY)
        );
        let events = rig.gesture(&front(), grab, &[[grab[0] + 10.0, grab[1] - 20.0]]);
        assert_eq!(events[0].handle, GizmoHandle::XY);
        assert!(close3(translate(&events[1]), [0.2, 0.4, 0.0]));
    }

    #[test]
    fn an_edge_on_plane_is_hidden_and_not_grabbable() {
        let gizmo = front();
        let fr = gizmo.frame(C);
        assert!(gizmo.plane_quad(&fr, [0, 1]).is_some());
        assert!(
            gizmo.plane_quad(&fr, [0, 2]).is_none(),
            "XZ is edge-on from the front"
        );
        assert!(gizmo.plane_quad(&fr, [1, 2]).is_none());
    }

    #[test]
    fn the_free_centre_moves_in_the_view_plane_in_world_units() {
        let gizmo = TransformGizmo::new(1, ViewRotation::from_yaw_pitch(90.0, 0.0));
        let events = Rig::new().gesture(&gizmo, at(0.0, 0.0), &[at(10.0, -5.0)]);
        assert_eq!(events[0].handle, GizmoHandle::Free);
        // From the right (+X), screen right is world −Z and up is +Y.
        assert!(
            close3(translate(&events[1]), [0.0, 0.1, -0.2]),
            "{:?}",
            events[1]
        );
    }

    #[test]
    fn the_centre_wins_over_the_arms_and_hit_order_runs_front_to_back() {
        let gizmo = front();
        let fr = gizmo.frame(C);
        assert_eq!(gizmo.hit(&fr, at(2.0, 2.0)), Some(GizmoHandle::Free));
        assert_eq!(gizmo.hit(&fr, at(80.0, 3.0)), Some(GizmoHandle::X));
        assert_eq!(gizmo.hit(&fr, at(0.0, -80.0)), Some(GizmoHandle::Y));
        // The arrowhead past the tip is part of the arm.
        assert_eq!(gizmo.hit(&fr, at(SIZE + 12.0, 0.0)), Some(GizmoHandle::X));
        assert_eq!(gizmo.hit(&fr, at(-80.0, 0.0)), None);
        let scale = front().mode(GizmoMode::Scale);
        assert_eq!(scale.hit(&fr, at(0.0, 0.0)), Some(GizmoHandle::Uniform));
        assert_eq!(
            scale.hit(&fr, at(SIZE + 15.0, 0.0)),
            None,
            "a cube is shorter than an arrow"
        );
        assert_eq!(front().hit(&fr, at(SIZE + 15.0, 0.0)), Some(GizmoHandle::X));
    }

    #[test]
    fn scale_is_uniform_from_the_centre_and_per_axis_on_a_cube() {
        let gizmo = front().mode(GizmoMode::Scale);
        let events = Rig::new().gesture(&gizmo, at(0.0, 0.0), &[at(46.0, 0.0)]);
        assert_eq!(events[0].value, GizmoValue::Scale([1.0; 3]));
        assert_eq!(events[1].value, GizmoValue::Scale([1.5; 3]));

        let events = Rig::new().gesture(&gizmo, at(0.0, -SIZE), &[at(0.0, -SIZE * 1.5)]);
        assert_eq!(events[0].handle, GizmoHandle::Y);
        assert_eq!(events[1].value, GizmoValue::Scale([1.0, 1.5, 1.0]));

        // Never through zero.
        let events = Rig::new().gesture(&gizmo, at(0.0, 0.0), &[at(-400.0, 0.0)]);
        assert_eq!(events[1].value, GizmoValue::Scale([0.01; 3]));
    }

    #[test]
    fn rotating_a_face_on_ring_turns_with_the_pointer_and_unwraps() {
        let gizmo = front().mode(GizmoMode::Rotate);
        let r = SIZE * std::f32::consts::FRAC_1_SQRT_2;
        let mut rig = Rig::new();
        let grab = at(r, -r);
        assert_eq!(
            rig.frame(&gizmo, &pointer(grab, false, false)).hovered,
            Some(GizmoHandle::Z)
        );
        // A quarter turn counter-clockwise on screen, in two steps, then
        // three quarters more to pass the ±180° seam.
        let path = [
            at(0.0, -SIZE),
            at(-r, -r),
            at(-SIZE, 0.0),
            at(0.0, SIZE),
            at(r, r),
        ];
        let events = rig.gesture(&gizmo, grab, &path);
        let turn = |e: &TransformEvent| match e.value {
            GizmoValue::Rotate(a) => a,
            other => panic!("{other:?}"),
        };
        assert!((turn(&events[2]) - 90.0).abs() < 1e-3, "{:?}", events[2]);
        assert!((turn(&events[5]) - 270.0).abs() < 1e-3, "{:?}", events[5]);
    }

    #[test]
    fn the_outer_ring_turns_about_the_view_axis() {
        let gizmo = TransformGizmo::new(1, ViewRotation::from_yaw_pitch(35.0, 25.0))
            .mode(GizmoMode::Rotate);
        let rr = SIZE + VIEW_RING_GAP;
        let mut rig = Rig::new();
        let events = rig.gesture(&gizmo, at(rr, 0.0), &[at(0.0, -rr)]);
        assert_eq!(events[0].handle, GizmoHandle::View);
        assert_eq!(events[1].value, GizmoValue::Rotate(90.0));
    }

    #[test]
    fn rotation_snaps_to_its_increment() {
        let gizmo = front().mode(GizmoMode::Rotate).snap(TransformSnap {
            rotate: Some(15.0),
            ..TransformSnap::default()
        });
        let rr = SIZE + VIEW_RING_GAP;
        let a = 50.0f32.to_radians();
        let events = Rig::new().gesture(&gizmo, at(rr, 0.0), &[at(rr * a.cos(), -rr * a.sin())]);
        assert_eq!(events[1].value, GizmoValue::Rotate(45.0));
    }

    #[test]
    fn escape_cancels_with_the_last_total() {
        let mut rig = Rig::new();
        let gizmo = front();
        rig.frame(&gizmo, &pointer(at(50.0, 0.0), true, true));
        rig.frame(&gizmo, &pointer(at(60.0, 0.0), true, false));
        let mut esc = pointer(at(60.0, 0.0), true, false);
        esc.key_escape = true;
        let out = rig.frame(&gizmo, &esc);
        let event = out.event.expect("cancel");
        assert_eq!(event.phase, TransformPhase::Cancel);
        assert!(close3(translate(&event), [0.2, 0.0, 0.0]));
        assert!(rig.drag.is_free() && rig.state.dragging().is_none());
        // Releasing afterwards does nothing more.
        assert_eq!(
            rig.frame(&gizmo, &pointer(at(60.0, 0.0), false, false))
                .event,
            None
        );
    }

    #[test]
    fn no_hover_or_grab_under_a_layer_or_another_drag() {
        let gizmo = front();
        let mut rig = Rig::new();
        let mut consumed = pointer(at(50.0, 0.0), true, true);
        consumed.mouse_consumed = true;
        let out = rig.frame(&gizmo, &consumed);
        assert_eq!((out.hovered, out.event), (None, None));

        rig.drag.try_begin(99);
        let out = rig.frame(&gizmo, &pointer(at(50.0, 0.0), true, true));
        assert_eq!((out.hovered, out.event), (None, None));
    }

    #[test]
    fn a_still_pointer_reports_no_drag_event() {
        let mut rig = Rig::new();
        let gizmo = front();
        rig.frame(&gizmo, &pointer(at(50.0, 0.0), true, true));
        let out = rig.frame(&gizmo, &pointer(at(50.0, 0.0), true, false));
        assert_eq!(out.event, None);
        assert_eq!(out.dragging, Some(GizmoHandle::X));
    }

    #[test]
    fn handle_axes_name_the_constraint() {
        assert_eq!(GizmoHandle::Y.axes(), [false, true, false]);
        assert_eq!(GizmoHandle::XZ.axes(), [true, false, true]);
        assert_eq!(GizmoHandle::Free.axes(), [false; 3]);
    }
}
