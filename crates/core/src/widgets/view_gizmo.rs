//! The orientation gizmo for a viewport corner (Forge `ViewGizmo`), and the
//! view rotation it and [`TransformGizmo`](crate::TransformGizmo) project
//! through.
//!
//! Six knobs sit on the ends of the world axes as the camera sees them: a
//! filled, lettered knob on each positive axis, a hollow one on each negative.
//! Click a knob to look down that axis; drag the disc to orbit. Below it a
//! glass caption names the view ("Top · Ortho") and, when the host allows it,
//! toggles orthographic projection.
//!
//! The gizmo draws and reports; the camera is the caller's. Feed it the
//! camera's [`ViewRotation`] each frame, and apply what
//! [`ViewGizmoOutput`] asks for: an orbit in degrees, a snap to an axis view,
//! an ortho toggle.

use crate::layout::Rect;
use crate::shadow::BoxShadow;
use crate::text::TextBlock;
use crate::{CornerRadii, CursorIcon, Ink, StyleKey, StyleResolver};

use super::hud::hud_glass;
use super::{DragCapture, DragId, DrawContext, DrawList, Stroke};

/// A camera's rotation, world to view, as three rows: the view's right, up,
/// and toward-the-viewer axes in world space. World space is Y-up.
///
/// Projecting a world direction gives `[x, y, depth]`: `x` to the right and
/// `y` down the screen (so it adds straight onto pixel coordinates), and
/// `depth` positive toward the viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRotation {
    rows: [[f32; 3]; 3],
}

impl Default for ViewRotation {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl ViewRotation {
    /// Looking down −Z from the front: +X right, +Y up.
    pub const IDENTITY: Self = Self {
        rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    /// A rotation from its rows: the view's right, up and toward-the-viewer
    /// axes in world coordinates (the rows of a world-to-view matrix). They
    /// should be orthonormal.
    pub const fn from_rows(right: [f32; 3], up: [f32; 3], toward: [f32; 3]) -> Self {
        Self {
            rows: [right, up, toward],
        }
    }

    /// An orbit camera's rotation: `yaw` spins about world Y, then `pitch`
    /// tilts about the camera's X, both in degrees. `(0, 0)` is the front
    /// view; positive pitch looks down from above, positive yaw swings the
    /// camera toward +X.
    pub fn from_yaw_pitch(yaw: f32, pitch: f32) -> Self {
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().sin_cos();
        Self::from_rows(
            [cy, 0.0, -sy],
            [-sp * sy, cp, -sp * cy],
            [cp * sy, sp, cp * cy],
        )
    }

    /// The view's right axis in world space.
    pub fn right(&self) -> [f32; 3] {
        self.rows[0]
    }

    /// The view's up axis in world space.
    pub fn up(&self) -> [f32; 3] {
        self.rows[1]
    }

    /// The axis from the scene toward the viewer, in world space.
    pub fn toward(&self) -> [f32; 3] {
        self.rows[2]
    }

    /// Project world direction `v`: `[x right, y down, depth toward viewer]`.
    pub fn project(&self, v: [f32; 3]) -> [f32; 3] {
        let [r, u, t] = self.rows;
        [dot(r, v), -dot(u, v), dot(t, v)]
    }

    /// The axis view this rotation looks down, if it is within `tolerance`
    /// (radians) of one: for naming the view ("Top") in the caption.
    pub fn aligned_axis(&self, tolerance: f32) -> Option<AxisDir> {
        let cos = tolerance.cos();
        AxisDir::ALL
            .into_iter()
            .find(|axis| dot(self.toward(), axis.vector()) >= cos)
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// One end of a world axis: what a view gizmo knob stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AxisDir {
    /// +X: the right view.
    PosX,
    /// −X: the left view.
    NegX,
    /// +Y: the top view.
    PosY,
    /// −Y: the bottom view.
    NegY,
    /// +Z: the front view.
    PosZ,
    /// −Z: the back view.
    NegZ,
}

impl AxisDir {
    /// All six, positive before negative per axis.
    pub const ALL: [AxisDir; 6] = [
        AxisDir::PosX,
        AxisDir::NegX,
        AxisDir::PosY,
        AxisDir::NegY,
        AxisDir::PosZ,
        AxisDir::NegZ,
    ];

    /// The world axis: 0 for X, 1 for Y, 2 for Z.
    pub const fn axis(self) -> usize {
        match self {
            AxisDir::PosX | AxisDir::NegX => 0,
            AxisDir::PosY | AxisDir::NegY => 1,
            AxisDir::PosZ | AxisDir::NegZ => 2,
        }
    }

    /// True for the positive end.
    pub const fn is_positive(self) -> bool {
        matches!(self, AxisDir::PosX | AxisDir::PosY | AxisDir::PosZ)
    }

    /// The unit vector.
    pub const fn vector(self) -> [f32; 3] {
        let sign = if self.is_positive() { 1.0 } else { -1.0 };
        match self.axis() {
            0 => [sign, 0.0, 0.0],
            1 => [0.0, sign, 0.0],
            _ => [0.0, 0.0, sign],
        }
    }

    /// The name of the view from this end ("Right", "Top", …).
    pub const fn view_name(self) -> &'static str {
        match self {
            AxisDir::PosX => "Right",
            AxisDir::NegX => "Left",
            AxisDir::PosY => "Top",
            AxisDir::NegY => "Bottom",
            AxisDir::PosZ => "Front",
            AxisDir::NegZ => "Back",
        }
    }

    /// The orbit angles, `(yaw, pitch)` in degrees, of the view from this
    /// end, for [`ViewRotation::from_yaw_pitch`].
    pub const fn yaw_pitch(self) -> (f32, f32) {
        match self {
            AxisDir::PosX => (90.0, 0.0),
            AxisDir::NegX => (-90.0, 0.0),
            AxisDir::PosY => (0.0, 90.0),
            AxisDir::NegY => (0.0, -90.0),
            AxisDir::PosZ => (0.0, 0.0),
            AxisDir::NegZ => (180.0, 0.0),
        }
    }

    /// The rotation of the view from this end, looking back at the origin.
    pub fn rotation(self) -> ViewRotation {
        let (yaw, pitch) = self.yaw_pitch();
        ViewRotation::from_yaw_pitch(yaw, pitch)
    }

    fn letter(self) -> &'static str {
        match self.axis() {
            0 => "X",
            1 => "Y",
            _ => "Z",
        }
    }
}

/// The default disc diameter, in px.
const SIZE: f32 = 84.0;
/// Knob centres sit this far inside the disc's edge.
const KNOB_INSET: f32 = 11.0;
/// Positive and negative knob radii.
const KNOB_POS: f32 = 8.0;
const KNOB_NEG: f32 = 6.0;
/// A knob behind the centre (depth below this) fades.
const BACK_DEPTH: f32 = -0.05;
const BACK_ALPHA: f32 = 0.6;
/// A positive axis's stem, from the centre to its knob.
const STEM_WIDTH: f32 = 2.0;
const STEM_BACK_ALPHA: f32 = 0.55;
/// The knob letters: mono 9 bold on a positive knob, 8 on a hovered negative.
const LETTER_POS: f32 = 9.0;
const LETTER_NEG: f32 = 8.0;
/// Ink of a positive knob's letter (`#0a0d0f`).
const KNOB_INK: [f32; 4] = [0.039, 0.051, 0.059, 1.0];
/// A negative knob's fill (`rgba(10,13,15,.75)`) and ring width.
const NEG_FILL: [f32; 4] = [0.039, 0.051, 0.059, 0.75];
const NEG_RING: f32 = 1.5;
/// The hover ring (`0 0 0 2px #f1f5f9`) and glow (`0 0 8px` white .4).
const HOVER_RING: [f32; 4] = [0.945, 0.961, 0.976, 1.0];
const HOVER_GLOW: [f32; 4] = [1.0, 1.0, 1.0, 0.4];
/// The disc under the pointer (`rgba(16,20,24,.5)`, a white .06 rim).
const DISC_FILL: [f32; 4] = [0.063, 0.078, 0.094, 0.5];
const DISC_RIM: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// Pointer travel, in px (|dx| + |dy|), before a press becomes an orbit.
const ORBIT_SLOP: f32 = 3.0;
/// Degrees of orbit per px of drag.
const ORBIT_RATE: f32 = 0.6;
/// Gap between the disc and the caption.
const CAPTION_GAP: f32 = 4.0;
/// The caption: mono 8.5, letter-spaced 0.12em, padded `1px 6px`.
const CAPTION_SIZE: f32 = 8.5;
const CAPTION_TRACKING: f32 = 0.12;
const CAPTION_PAD: [f32; 2] = [6.0, 1.0];

/// The orientation gizmo for a viewport corner (Forge `ViewGizmo`). Built
/// fresh each frame; its press state lives in a caller-owned
/// [`ViewGizmoState`].
#[derive(Clone, Copy, Debug)]
pub struct ViewGizmo<'a> {
    id: DragId,
    rotation: ViewRotation,
    size: f32,
    ortho: bool,
    view: Option<&'a str>,
    ortho_toggle: bool,
}

/// What a [`ViewGizmo`] reports for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewGizmoOutput {
    /// The disc's bounds.
    pub disc: Rect,
    /// The pointer is over the disc or the caption: the scene under them
    /// should not take the pointer.
    pub hovered: bool,
    /// A knob was clicked: snap the camera to look down this axis (see
    /// [`AxisDir::rotation`]).
    pub snap: Option<AxisDir>,
    /// Orbit the camera by `[yaw, pitch]` degrees: the disc was dragged.
    pub orbit: [f32; 2],
    /// The caption was clicked: toggle orthographic projection.
    pub toggle_ortho: bool,
    /// The disc is being dragged.
    pub dragging: bool,
}

/// A [`ViewGizmo`]'s press, between the frames of a click or a drag.
/// Caller-owned; persists across frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewGizmoState {
    start: [f32; 2],
    last: [f32; 2],
    moved: bool,
    knob: Option<AxisDir>,
}

impl ViewGizmoState {
    /// No press.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A knob laid out for this frame.
#[derive(Clone, Copy, Debug)]
struct Knob {
    dir: AxisDir,
    centre: [f32; 2],
    radius: f32,
    depth: f32,
}

impl<'a> ViewGizmo<'a> {
    /// A gizmo under drag id `id` showing `rotation`.
    pub const fn new(id: DragId, rotation: ViewRotation) -> Self {
        Self {
            id,
            rotation,
            size: SIZE,
            ortho: false,
            view: None,
            ortho_toggle: false,
        }
    }

    /// The disc's diameter, in px (84 by default).
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// The camera is orthographic: the caption says "Ortho", else "Persp".
    pub const fn ortho(mut self, ortho: bool) -> Self {
        self.ortho = ortho;
        self
    }

    /// Name the view in the caption ("Top · Ortho"), for a camera looking
    /// down an axis (see [`ViewRotation::aligned_axis`]).
    pub const fn view(mut self, view: Option<&'a str>) -> Self {
        self.view = view;
        self
    }

    /// Let a click on the caption toggle orthographic projection.
    pub const fn ortho_toggle(mut self) -> Self {
        self.ortho_toggle = true;
        self
    }

    fn caption_text(&self) -> String {
        let projection = if self.ortho { "ORTHO" } else { "PERSP" };
        match self.view {
            Some(view) => format!("{} · {projection}", view.to_uppercase()),
            None => projection.to_string(),
        }
    }

    fn caption_block(&self, s: &StyleResolver, x: f32, y: f32) -> TextBlock {
        TextBlock::new(self.caption_text(), x, y)
            .with_size(CAPTION_SIZE)
            .with_letter_spacing(CAPTION_SIZE * CAPTION_TRACKING)
            .with_color_f32(s.ink(Ink::Muted))
            .with_font_opt(s.theme().mono_font.clone())
    }

    /// The caption pill's size.
    fn caption_size(&self, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        let (w, h) = list.measure_block(&self.caption_block(s, 0.0, 0.0));
        [
            w + CAPTION_PAD[0] * 2.0 + 2.0,
            h.max(CAPTION_SIZE) + CAPTION_PAD[1] * 2.0 + 2.0,
        ]
    }

    /// The gizmo's size, in px: the disc over its caption.
    pub fn outer_size(&self, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        let [cw, ch] = self.caption_size(list, s);
        [self.size.max(cw), self.size + CAPTION_GAP + ch]
    }

    /// The six knobs, back to front.
    fn knobs(&self, centre: [f32; 2]) -> [Knob; 6] {
        let reach = self.size * 0.5 - KNOB_INSET;
        let mut knobs = AxisDir::ALL.map(|dir| {
            let [x, y, depth] = self.rotation.project(dir.vector());
            Knob {
                dir,
                centre: [centre[0] + x * reach, centre[1] + y * reach],
                radius: if dir.is_positive() {
                    KNOB_POS
                } else {
                    KNOB_NEG
                },
                depth,
            }
        });
        knobs.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        knobs
    }

    /// Draw the gizmo in `rect` (sized by [`outer_size`](Self::outer_size);
    /// the disc centres along its top) and handle its pointer.
    pub fn draw(
        &self,
        rect: Rect,
        state: &mut ViewGizmoState,
        drag: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> ViewGizmoOutput {
        let s = ctx.styles();
        let input = ctx.input;
        let disc = Rect::new(
            rect.x + (rect.width - self.size) * 0.5,
            rect.y,
            self.size,
            self.size,
        );
        let centre = [disc.x + self.size * 0.5, disc.y + self.size * 0.5];
        let mouse = [input.mouse_x, input.mouse_y];
        let pointer = !input.mouse_consumed;
        let in_disc = pointer && distance(mouse, centre) <= self.size * 0.5;
        let knobs = self.knobs(centre);
        let mut out = ViewGizmoOutput {
            disc,
            ..ViewGizmoOutput::default()
        };

        // The front-most knob under the pointer, while no other drag owns it.
        let free = drag.is_free() || drag.is_active(self.id);
        let hot = (pointer && free)
            .then(|| {
                knobs
                    .iter()
                    .rev()
                    .find(|k| distance(mouse, k.centre) <= k.radius)
                    .map(|k| k.dir)
            })
            .flatten();

        if in_disc && input.mouse_clicked && drag.try_begin(self.id) {
            *state = ViewGizmoState {
                start: mouse,
                last: mouse,
                moved: false,
                knob: hot,
            };
        }
        if drag.is_active(self.id) {
            if input.mouse_down {
                let from_start =
                    (mouse[0] - state.start[0]).abs() + (mouse[1] - state.start[1]).abs();
                if state.moved || from_start >= ORBIT_SLOP {
                    state.moved = true;
                    out.orbit = [
                        (mouse[0] - state.last[0]) * ORBIT_RATE,
                        (mouse[1] - state.last[1]) * ORBIT_RATE,
                    ];
                    state.last = mouse;
                }
                out.dragging = state.moved;
            } else {
                drag.release(self.id);
                if !state.moved {
                    out.snap = state.knob.filter(|&k| hot == Some(k));
                }
                *state = ViewGizmoState::default();
            }
        }

        // A knob under the pointer stays lit through an orbit only if the
        // press started on it; a drag lights the disc instead.
        let lit = if out.dragging { None } else { hot };
        let active = in_disc || out.dragging;
        if out.dragging {
            ctx.request_cursor(CursorIcon::Grabbing);
        } else if lit.is_some() {
            ctx.request_cursor(CursorIcon::Pointer);
        } else if in_disc {
            ctx.request_cursor(CursorIcon::Grab);
        }

        ctx.push_debug_scope_rect("ViewGizmo", rect);
        let list = &mut *ctx.draw_list;
        if active {
            list.chrome_rect(disc, self.size * 0.5, 1.0, DISC_FILL, DISC_RIM);
        }
        for knob in &knobs {
            draw_knob(list, &s, knob, centre, lit == Some(knob.dir));
        }

        // The caption pill.
        let [cw, ch] = self.caption_size(list, &s);
        let pill = Rect::new(
            rect.x + (rect.width - cw) * 0.5,
            disc.bottom() + CAPTION_GAP,
            cw,
            ch,
        );
        hud_glass(list, &s, pill);
        let mono = s.theme().mono_font.clone();
        let y = list.vcentered_text_y(pill.y, pill.height, CAPTION_SIZE, mono.as_ref(), "P");
        list.text(self.caption_block(&s, pill.x + 1.0 + CAPTION_PAD[0], y));
        let over_pill = pointer && pill.contains(mouse[0], mouse[1]);
        if self.ortho_toggle && over_pill && free {
            ctx.request_cursor(CursorIcon::Pointer);
            out.toggle_ortho = input.mouse_clicked;
        }
        ctx.pop_debug_scope();

        out.hovered = in_disc || over_pill;
        out
    }
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn faded(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

/// Draw one knob, with a positive knob's stem from the centre.
fn draw_knob(list: &mut DrawList, s: &StyleResolver, knob: &Knob, centre: [f32; 2], hot: bool) {
    let tint = s.color(StyleKey::AXES[knob.dir.axis()]);
    let back = knob.depth < BACK_DEPTH;
    let alpha = if back && !hot { BACK_ALPHA } else { 1.0 };
    let [x, y] = knob.centre;
    let r = knob.radius;
    let positive = knob.dir.is_positive();

    if positive {
        let (dx, dy) = (x - centre[0], y - centre[1]);
        let len = dx.hypot(dy);
        if len > r {
            let end = [
                centre[0] + dx / len * (len - r),
                centre[1] + dy / len * (len - r),
            ];
            let stem_alpha = if back { STEM_BACK_ALPHA } else { 1.0 };
            list.stroke_line(
                centre,
                end,
                &Stroke::new(STEM_WIDTH),
                faded(tint, stem_alpha),
            );
        }
    }

    let bounds = Rect::new(x - r, y - r, r * 2.0, r * 2.0);
    let radii = CornerRadii::uniform(r);
    if hot {
        list.box_shadow_outset(
            bounds,
            radii,
            BoxShadow {
                blur: 8.0,
                spread: 2.0,
                color: HOVER_GLOW,
                ..BoxShadow::default()
            },
        );
        // Only outside the knob, like a CSS spread shadow: a negative knob's
        // fill is translucent.
        list.chrome_rect(bounds.inset(-2.0), r + 2.0, 2.0, [0.0; 4], HOVER_RING);
    } else {
        list.box_shadow_outset(
            bounds,
            radii,
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 2.0,
                color: faded([0.0, 0.0, 0.0, 0.6], alpha),
                ..BoxShadow::default()
            },
        );
    }
    if positive {
        list.chrome_rect(
            bounds,
            r,
            1.0,
            faded(tint, alpha),
            faded([0.0, 0.0, 0.0, 0.55], alpha),
        );
    } else {
        list.chrome_rect(
            bounds,
            r,
            NEG_RING,
            faded(NEG_FILL, alpha),
            faded(tint, alpha),
        );
    }

    let (label, size, color) = if positive {
        (knob.dir.letter(), LETTER_POS, faded(KNOB_INK, alpha))
    } else if hot {
        (
            match knob.dir.axis() {
                0 => "−X",
                1 => "−Y",
                _ => "−Z",
            },
            LETTER_NEG,
            tint,
        )
    } else {
        return;
    };
    let mono = s.theme().mono_font.clone();
    let width = list
        .measure_text_with_font(label, size, None, mono.as_ref())
        .0;
    let ty = list.vcentered_text_y(bounds.y, bounds.height, size, mono.as_ref(), label);
    list.text(
        TextBlock::new(label, x - width * 0.5, ty)
            .with_size(size)
            .with_weight(crate::Weight::BOLD)
            .with_color_f32(color)
            .with_font_opt(mono),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// Forge's `project`, transcribed.
    fn forge_project([x, y, z]: [f32; 3], yaw: f32, pitch: f32) -> [f32; 3] {
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().sin_cos();
        let (x1, z1) = (x * cy - z * sy, x * sy + z * cy);
        let (y2, z2) = (y * cp - z1 * sp, y * sp + z1 * cp);
        [x1, -y2, z2]
    }

    #[test]
    fn yaw_pitch_projects_like_forge() {
        for (yaw, pitch) in [(0.0, 0.0), (35.0, 25.0), (-120.0, 60.0), (200.0, -45.0)] {
            let rot = ViewRotation::from_yaw_pitch(yaw, pitch);
            for v in [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.3, -0.7, 0.2],
            ] {
                assert!(
                    close(rot.project(v), forge_project(v, yaw, pitch)),
                    "{yaw}/{pitch} {v:?}"
                );
            }
        }
    }

    #[test]
    fn the_front_view_is_the_identity_with_y_up_the_screen() {
        let rot = ViewRotation::from_yaw_pitch(0.0, 0.0);
        assert!(close(rot.project([1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]));
        assert!(close(rot.project([0.0, 1.0, 0.0]), [0.0, -1.0, 0.0]));
        assert!(close(rot.project([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]));
        assert_eq!(
            ViewRotation::IDENTITY.project([0.0, 1.0, 0.0]),
            [0.0, -1.0, 0.0]
        );
    }

    #[test]
    fn each_axis_view_looks_down_its_axis() {
        for dir in AxisDir::ALL {
            let rot = dir.rotation();
            assert!(close(rot.toward(), dir.vector()), "{dir:?}");
            assert_eq!(rot.aligned_axis(0.01), Some(dir));
            let [x, y, depth] = rot.project(dir.vector());
            assert!(x.abs() < 1e-5 && y.abs() < 1e-5 && (depth - 1.0).abs() < 1e-5);
        }
        assert_eq!(
            ViewRotation::from_yaw_pitch(35.0, 25.0).aligned_axis(0.05),
            None
        );
    }

    #[test]
    fn the_knobs_sort_back_to_front() {
        let gizmo = ViewGizmo::new(1, ViewRotation::from_yaw_pitch(35.0, 25.0));
        let knobs = gizmo.knobs([0.0, 0.0]);
        assert!(knobs.windows(2).all(|w| w[0].depth <= w[1].depth));
        // The default disc puts knob centres 31 px out, at most.
        for k in knobs {
            assert!(k.centre[0].hypot(k.centre[1]) <= SIZE * 0.5 - KNOB_INSET + 1e-4);
        }
    }

    struct Rig {
        theme: Theme,
        state: ViewGizmoState,
        drag: DragCapture,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                theme: Theme::default(),
                state: ViewGizmoState::new(),
                drag: DragCapture::new(),
            }
        }

        fn frame(&mut self, gizmo: &ViewGizmo, input: &InputState) -> (DrawList, ViewGizmoOutput) {
            let mut list = DrawList::new();
            let mut focus = FocusState::new();
            let mut ctx = DrawContext::new(&mut list, &mut focus, &self.theme, input, 800.0, 600.0);
            let [w, h] = gizmo.outer_size(ctx.draw_list, &ctx.styles());
            let out = gizmo.draw(
                Rect::new(0.0, 0.0, w, h),
                &mut self.state,
                &mut self.drag,
                &mut ctx,
            );
            (list, out)
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

    /// The front view: +X sits 31 px right of the 42 px centre.
    const PLUS_X: [f32; 2] = [42.0 + 31.0, 42.0];

    #[test]
    fn a_click_on_a_knob_snaps_to_its_view() {
        let gizmo = ViewGizmo::new(1, ViewRotation::IDENTITY);
        let mut rig = Rig::new();
        let (_, out) = rig.frame(&gizmo, &pointer(PLUS_X, true, true));
        assert_eq!(out.snap, None, "snaps on release");
        assert!(out.hovered);
        let (_, out) = rig.frame(&gizmo, &pointer([PLUS_X[0] + 1.0, PLUS_X[1]], false, false));
        assert_eq!(out.snap, Some(AxisDir::PosX));
        assert!(rig.drag.is_free());
    }

    #[test]
    fn a_drag_past_the_slop_orbits_and_does_not_snap() {
        let gizmo = ViewGizmo::new(1, ViewRotation::IDENTITY);
        let mut rig = Rig::new();
        rig.frame(&gizmo, &pointer(PLUS_X, true, true));
        let (_, out) = rig.frame(&gizmo, &pointer([PLUS_X[0] + 2.0, PLUS_X[1]], true, false));
        assert_eq!(out.orbit, [0.0, 0.0], "inside the slop");
        assert!(!out.dragging);
        let (_, out) = rig.frame(
            &gizmo,
            &pointer([PLUS_X[0] + 10.0, PLUS_X[1] - 5.0], true, false),
        );
        assert_eq!(out.orbit, [10.0 * ORBIT_RATE, -5.0 * ORBIT_RATE]);
        assert!(out.dragging);
        let (_, out) = rig.frame(
            &gizmo,
            &pointer([PLUS_X[0] + 12.0, PLUS_X[1] - 5.0], true, false),
        );
        assert_eq!(out.orbit, [2.0 * ORBIT_RATE, 0.0], "then frame by frame");
        let (_, out) = rig.frame(&gizmo, &pointer(PLUS_X, false, false));
        assert_eq!(out.snap, None);
        assert_eq!(out.orbit, [0.0, 0.0]);
    }

    #[test]
    fn a_press_owned_elsewhere_or_under_a_layer_does_nothing() {
        let gizmo = ViewGizmo::new(1, ViewRotation::IDENTITY);
        let mut rig = Rig::new();
        rig.drag.try_begin(99);
        rig.frame(&gizmo, &pointer(PLUS_X, true, true));
        let (_, out) = rig.frame(&gizmo, &pointer(PLUS_X, false, false));
        assert_eq!(out.snap, None);

        let mut rig = Rig::new();
        let mut consumed = pointer(PLUS_X, true, true);
        consumed.mouse_consumed = true;
        let (_, out) = rig.frame(&gizmo, &consumed);
        assert!(!out.hovered);
        assert!(rig.drag.is_free());
    }

    #[test]
    fn the_caption_names_the_view_and_toggles_only_when_allowed() {
        let gizmo = ViewGizmo::new(1, ViewRotation::IDENTITY);
        assert_eq!(gizmo.caption_text(), "PERSP");
        assert_eq!(
            gizmo.ortho(true).view(Some("Top")).caption_text(),
            "TOP · ORTHO"
        );

        let mut rig = Rig::new();
        let theme = Theme::default();
        let mut list = DrawList::new();
        let [w, h] = gizmo.outer_size(&mut list, &StyleResolver::new(&theme));
        let on_caption = [w * 0.5, h - 3.0];
        let (_, out) = rig.frame(&gizmo, &pointer(on_caption, true, true));
        assert!(!out.toggle_ortho && out.hovered);
        let (_, out) = rig.frame(&gizmo.ortho_toggle(), &pointer(on_caption, true, true));
        assert!(out.toggle_ortho);
    }

    #[test]
    fn a_hovered_negative_knob_shows_its_name() {
        let gizmo = ViewGizmo::new(1, ViewRotation::IDENTITY);
        let mut rig = Rig::new();
        let minus_x = [42.0 - 31.0, 42.0];
        let (list, _) = rig.frame(&gizmo, &pointer(minus_x, false, false));
        assert!(list.texts.iter().any(|t| t.content == "−X"));
        let (list, _) = rig.frame(&gizmo, &pointer([200.0, 200.0], false, false));
        assert!(!list.texts.iter().any(|t| t.content.starts_with('−')));
        let letters: Vec<_> = list.texts.iter().map(|t| t.content.as_str()).collect();
        for l in ["X", "Y", "Z"] {
            assert!(letters.contains(&l), "{letters:?}");
        }
    }
}
