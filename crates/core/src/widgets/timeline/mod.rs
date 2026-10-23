//! Timeline: a keyframe dope sheet for a bottom dock (Forge `Timeline`).
//!
//! A transport strip on top (first frame, previous key, play, next key, last
//! frame, the frame field, timecode and rate, markers, loop, zoom, fit), then
//! a ruler with the playhead, the play range and the markers, then one lane
//! per visible track beside a label column. The ruler and the label column
//! stay put while the lanes scroll under them.
//!
//! Track data and markers are the caller's and are edited **in place**; every
//! change also comes back in [`TimelineOutput::edits`], with what an undo
//! needs to reverse it. The view — frame, playback, play range, zoom, scroll,
//! selection, collapsed groups — lives in the caller-owned [`TimelineState`].
//!
//! ```ignore
//! let out = Timeline::new(TIMELINE).length(240).fps(24).draw(
//!     rect, &mut tracks, &mut markers, &mut timeline, &mut drag, &mut ctx,
//! );
//! for edit in out.edits {
//!     undo.push(edit);
//! }
//! if out.frame_changed {
//!     scene.seek(timeline.frame);
//! }
//! ```
//!
//! Mouse: click or drag the ruler to scrub. Click a key to select it (Shift or
//! Ctrl toggles it), drag to move every selected key, snapped to frames and
//! clamped to 0…length. Drag empty lane space to marquee-select; double-click
//! a lane to add a key. The wheel scrolls the rows, Shift + wheel scrolls
//! time, Ctrl + wheel zooms around the pointer. Drag a marker's flag to move
//! it, click it to jump there, double-click it to rename it. Drag a range
//! bracket to move that end.
//!
//! Keys, while the timeline has focus: ← → step a frame (Shift: ten), Home and
//! End jump to the range ends, `,` and `.` to the previous and next key, Space
//! plays and pauses, Delete removes the selected keys (or else the selected
//! marker), Ctrl+A selects every key, `=` and `-` zoom, `M` adds a marker at
//! the playhead (or renames the one there), Esc clears the selection. The
//! letter and punctuation shortcuts read
//! [`InputState::keys`](crate::InputState::keys).
//!
//! Each frame is a cell: grid lines sit on cell edges, keys and markers at
//! cell centres, and the current frame lights its whole column. The play
//! range is inclusive.

mod model;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

pub use model::{Interp, KeyRef, Keyframe, Marker, TimelineEdit, Track, TrackId, timecode};
use model::{Row, Rows};

use crate::keys::Key;
use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{
    BoxShadow, CornerRadii, CursorIcon, Edge, Ink, NumberInput, PhosphorIcon, StyleKey,
    StyleResolver, SurfacePainter, TextSize, TimelineChrome, Tracking,
};

use super::material::{Tone, latch_ink};
use super::tree::{CARET_HALF, draw_disclosure};
use super::{DragCapture, DrawContext, DrawList, Pressable, ScrollView, Stroke, TextInput};

/// Height of one track row.
const ROW: f32 = 20.0;
/// Height of the ruler.
const RULER: f32 = 24.0;
/// Room before frame 0 and after the last frame.
const LPAD: f32 = 10.0;
/// Indent per nesting level in the label column.
const INDENT: f32 = 12.0;
/// Face of a transport key.
const KEY: f32 = 19.0;
/// Width of the play key.
const PLAY_W: f32 = 26.0;
/// Gap between transport keys.
const KEY_GAP: f32 = 3.0;
/// Transport strip padding: top, right, bottom, left.
const STRIP_PAD: [f32; 4] = [4.0, 6.0, 4.0, 8.0];
/// Horizontal padding of a transport text key.
const TEXT_KEY_PAD: f32 = 7.0;
/// Size of a transport text key's label.
const TEXT_KEY_SIZE: f32 = 10.5;
/// Width of the frame field.
const FIELD_W: f32 = 60.0;
/// Gap between the transport's readouts.
const READOUT_GAP: f32 = 7.0;
/// Side of a track's mute and lock keys.
const TRACK_KEY: f32 = 15.0;
/// Size of a track key's letter.
const TRACK_KEY_SIZE: f32 = 8.5;
/// Half the width of a key's hit cell.
const KEY_HIT: f32 = 6.0;
/// Size of a marker's label.
const MARKER_SIZE: f32 = 8.5;
/// Height of a marker's flag row.
const MARKER_H: f32 = 12.0;
/// Height of a marker's rename field (the frame field's, so the theme's
/// field text fits).
const RENAME_H: f32 = KEY;
/// Minimum width of a marker's rename field.
const RENAME_MIN_W: f32 = 64.0;
/// Height of a range bracket.
const BRACKET_H: f32 = 14.0;
/// Half the width of a range bracket's hit area.
const BRACKET_HIT: f32 = 5.0;
/// Pixels per frame, least and most.
const ZOOM_RANGE: [f32; 2] = [0.5, 40.0];
/// Zoom factor of a key or button press.
const ZOOM_STEP: f32 = 1.25;
/// Zoom factor of a wheel notch.
const WHEEL_ZOOM: f32 = 1.15;
/// Pixels scrolled per wheel notch.
const WHEEL_PX: f32 = 20.0;
/// Alpha of a muted track's keys.
const MUTED_ALPHA: f32 = 0.45;
/// The carve under label text (`--carve`).
const CARVE_ALPHA: u8 = 128;
/// What the timeline says when there are no tracks.
const EMPTY: &str = "No tracks · select an object to see its keys";

/// Pixels per frame of a new [`TimelineState`].
pub const TIMELINE_DEFAULT_ZOOM: f32 = 6.0;

/// A keyframe dope sheet. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    id: u64,
    length: u32,
    fps: u32,
    header_width: f32,
    editable: bool,
    markers_editable: bool,
    range_editable: bool,
}

impl Timeline {
    /// A timeline with focus and drag id `id`: 240 frames at 24 fps, every
    /// part editable.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            length: 240,
            fps: 24,
            header_width: 176.0,
            editable: true,
            markers_editable: true,
            range_editable: true,
        }
    }

    /// The last frame; frames run 0…`length`, inclusive.
    pub fn length(mut self, length: u32) -> Self {
        self.length = length;
        self
    }

    /// Frames per second, for playback and the timecode.
    pub fn fps(mut self, fps: u32) -> Self {
        self.fps = fps.max(1);
        self
    }

    /// Width of the label column.
    pub fn header_width(mut self, width: f32) -> Self {
        self.header_width = width.max(0.0);
        self
    }

    /// Whether keys can be moved, added and deleted, and tracks muted and
    /// locked. Selection works either way.
    pub fn editable(mut self, editable: bool) -> Self {
        self.editable = editable;
        self
    }

    /// Whether markers can be added, moved, renamed and deleted.
    pub fn markers_editable(mut self, editable: bool) -> Self {
        self.markers_editable = editable;
        self
    }

    /// Whether the play range's brackets can be dragged.
    pub fn range_editable(mut self, editable: bool) -> Self {
        self.range_editable = editable;
        self
    }
}

/// The caller-owned view of a [`Timeline`]: where the playhead is, whether it
/// plays, the play range, zoom and scroll, the selection and the collapsed
/// groups. Keep one per timeline, across frames.
pub struct TimelineState {
    /// The current frame.
    pub frame: u32,
    /// Playing: the frame advances with [`InputState::frame_dt`](crate::InputState::frame_dt).
    pub playing: bool,
    /// Loop within the play range (else stop at its end).
    pub looping: bool,
    /// The play range, inclusive; `None` plays the whole length.
    pub range: Option<[u32; 2]>,
    /// Pixels per frame.
    pub zoom: f32,
    /// Scroll offset: time (px), rows (px).
    pub scroll: [f32; 2],
    /// The selected keys.
    pub selection: HashSet<KeyRef>,
    /// Collapsed groups.
    pub collapsed: HashSet<TrackId>,
    /// The selected marker's index.
    pub marker_selected: Option<usize>,
    gesture: Gesture,
    clock: Option<Clock>,
    /// The frame the playhead was last kept in view at.
    seen_frame: Option<u32>,
    frame_field: TextInput,
    rename: Option<Rename>,
    rows: Rows,
    scratch: Vec<KeyRef>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineState {
    /// At frame 0, paused, looping, no range, default zoom.
    pub fn new() -> Self {
        Self {
            frame: 0,
            playing: false,
            looping: true,
            range: None,
            zoom: TIMELINE_DEFAULT_ZOOM,
            scroll: [0.0; 2],
            selection: HashSet::new(),
            collapsed: HashSet::new(),
            marker_selected: None,
            gesture: Gesture::Idle,
            clock: None,
            seen_frame: None,
            frame_field: TextInput::new(0.0, 0.0, 0.0, 0.0),
            rename: None,
            rows: Rows::default(),
            scratch: Vec::new(),
        }
    }

    /// The index of the marker whose name is being edited.
    pub fn renaming(&self) -> Option<usize> {
        self.rename.as_ref().map(|r| r.index)
    }

    /// True while a pointer gesture (scrub, key drag, marquee, …) is held.
    pub fn is_dragging(&self) -> bool {
        !matches!(self.gesture, Gesture::Idle)
    }

    /// The play range, or the whole length.
    fn bounds(&self, length: u32) -> [u32; 2] {
        self.range.unwrap_or([0, length])
    }
}

/// What a [`Timeline`] did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineOutput {
    /// Changes made to the tracks, markers and play range, in order.
    pub edits: Vec<TimelineEdit>,
    /// The frame moved (scrubbed, stepped, played).
    pub frame_changed: bool,
    /// The pointer is over the timeline.
    pub hovered: bool,
}

/// The pointer gesture in progress.
#[derive(Clone, Debug, Default)]
enum Gesture {
    #[default]
    Idle,
    /// Dragging the playhead along the ruler.
    Scrub,
    /// Dragging the selected keys; `delta` is clamped to `[lo, hi]`.
    Keys {
        x0: f32,
        delta: i64,
        lo: i64,
        hi: i64,
    },
    /// Marquee from `from` to `to`, in lane content space; the selection is
    /// `base` plus what it covers.
    Marquee {
        from: [f32; 2],
        to: [f32; 2],
        base: HashSet<KeyRef>,
        moved: bool,
    },
    /// Dragging one end of the play range.
    Range { edge: usize, from: [u32; 2] },
    /// Dragging marker `index`, now shown at `t`.
    Marker {
        index: usize,
        x0: f32,
        t: u32,
        moved: bool,
    },
    /// Dragging a scroll thumb; `grab` is the pointer's offset into it.
    Thumb { axis: usize, grab: f32 },
}

/// Playback's clock: `elapsed` frames (fractional) since leaving frame
/// `from`. `shown` is the frame it last set, so a frame set from elsewhere
/// restarts it. Counting frames rather than seconds keeps a wrap exact.
#[derive(Clone, Copy, Debug)]
struct Clock {
    from: u32,
    elapsed: f32,
    shown: u32,
}

/// A marker's name being edited.
struct Rename {
    index: usize,
    input: TextInput,
    /// Focus has been handed to the field.
    armed: bool,
}

/// Where everything is this frame.
#[derive(Clone, Copy, Debug)]
struct Layout {
    strip: Rect,
    body: Rect,
    /// The ruler above the lanes.
    ruler: Rect,
    /// The ruler above the labels.
    corner: Rect,
    labels: Rect,
    lanes: Rect,
    ppf: f32,
    scroll: [f32; 2],
    /// Width and height of everything that scrolls.
    content: [f32; 2],
}

impl Layout {
    /// Screen x of frame cell `t`'s left edge.
    fn x(&self, t: f32) -> f32 {
        self.lanes.x - self.scroll[0] + LPAD + t * self.ppf
    }

    /// Screen x of frame `t`'s centre.
    fn xc(&self, t: u32) -> f32 {
        self.x(t as f32 + 0.5)
    }

    /// The frame under screen x, fractional.
    fn frame_at(&self, x: f32) -> f32 {
        (x - self.lanes.x + self.scroll[0] - LPAD) / self.ppf
    }

    /// Screen y of row `i`'s top.
    fn row_y(&self, i: usize) -> f32 {
        self.lanes.y - self.scroll[1] + i as f32 * ROW
    }

    /// The row under screen y.
    fn row_at(&self, y: f32, rows: usize) -> Option<usize> {
        let i = ((y - self.lanes.y + self.scroll[1]) / ROW).floor();
        (i >= 0.0 && (i as usize) < rows).then_some(i as usize)
    }

    /// Rows that are on screen.
    fn visible_rows(&self, rows: usize) -> std::ops::Range<usize> {
        let first = (self.scroll[1] / ROW).floor().max(0.0) as usize;
        let last = ((self.scroll[1] + self.lanes.height) / ROW).ceil().max(0.0) as usize;
        first.min(rows)..last.min(rows)
    }

    fn max_scroll(&self) -> [f32; 2] {
        [
            (self.content[0] - self.lanes.width).max(0.0),
            (self.content[1] - self.lanes.height).max(0.0),
        ]
    }

    /// A pointer position in lane content space.
    fn content_point(&self, x: f32, y: f32) -> [f32; 2] {
        [
            x - self.lanes.x + self.scroll[0],
            y - self.lanes.y + self.scroll[1],
        ]
    }

    /// Scroll thumb `axis` (0 time, 1 rows): its track and thumb, when the
    /// content overflows.
    fn thumb(&self, axis: usize) -> Option<(Rect, Rect)> {
        let max = self.max_scroll()[axis];
        if max <= 0.0 {
            return None;
        }
        let t = ScrollView::OVERLAY_THICKNESS;
        let inset = ScrollView::OVERLAY_INSET;
        let both = self.max_scroll()[1 - axis] > 0.0;
        let track = if axis == 0 {
            Rect::new(
                self.lanes.x + inset,
                self.lanes.bottom() - t - inset,
                self.lanes.width - 2.0 * inset - if both { t + inset } else { 0.0 },
                t,
            )
        } else {
            Rect::new(
                self.lanes.right() - t - inset,
                self.lanes.y + inset,
                t,
                self.lanes.height - 2.0 * inset - if both { t + inset } else { 0.0 },
            )
        };
        let (len, view) = if axis == 0 {
            (track.width, self.lanes.width)
        } else {
            (track.height, self.lanes.height)
        };
        if len <= 0.0 {
            return None;
        }
        let size = (len * view / self.content[axis])
            .max(ScrollView::MIN_THUMB)
            .min(len);
        let at = (len - size) * (self.scroll[axis] / max);
        let thumb = if axis == 0 {
            Rect::new(track.x + at, track.y, size, t)
        } else {
            Rect::new(track.x, track.y + at, t, size)
        };
        Some((track, thumb))
    }
}

/// An id for part `part` of timeline `id` (its frame field, a marker's
/// rename field).
fn part_id(id: u64, part: u64) -> u64 {
    id.rotate_left(17) ^ part.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

const FRAME_FIELD: u64 = 1;
const RENAME_FIELD: u64 = 2;

impl Timeline {
    /// Height of the transport strip under `s`.
    fn strip_height(s: &StyleResolver) -> f32 {
        STRIP_PAD[0] + KEY + Pressable::new().travel_px(s) + STRIP_PAD[2]
    }

    fn layout(&self, rect: Rect, state: &TimelineState, rows: usize, s: &StyleResolver) -> Layout {
        let strip = Rect::new(rect.x, rect.y, rect.width, Self::strip_height(s));
        let body = Rect::new(
            rect.x,
            strip.bottom(),
            rect.width,
            (rect.bottom() - strip.bottom()).max(0.0),
        );
        let header = self.header_width.min(body.width);
        let ruler_h = RULER.min(body.height);
        let corner = Rect::new(body.x, body.y, header, ruler_h);
        let ruler = Rect::new(body.x + header, body.y, body.width - header, ruler_h);
        let labels = Rect::new(body.x, body.y + ruler_h, header, body.height - ruler_h);
        let lanes = Rect::new(ruler.x, labels.y, ruler.width, labels.height);
        let ppf = state.zoom.clamp(ZOOM_RANGE[0], ZOOM_RANGE[1]);
        let content = [
            2.0 * LPAD + (self.length as f32 + 1.0) * ppf,
            rows as f32 * ROW,
        ];
        let mut layout = Layout {
            strip,
            body,
            ruler,
            corner,
            labels,
            lanes,
            ppf,
            scroll: state.scroll,
            content,
        };
        let max = layout.max_scroll();
        for (scroll, max) in layout.scroll.iter_mut().zip(max) {
            *scroll = scroll.clamp(0.0, max);
        }
        layout
    }

    /// Draw the timeline in `rect`, editing `tracks` and `markers` in place.
    /// `drag` arbitrates the pointer with the surface's other draggables.
    pub fn draw(
        &self,
        rect: Rect,
        tracks: &mut [Track],
        markers: &mut Vec<Marker>,
        state: &mut TimelineState,
        drag: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> TimelineOutput {
        ctx.push_debug_scope_rect("Timeline", rect);
        let mut out = TimelineOutput::default();
        let start_frame = state.frame;
        self.sanitize(state, markers);

        let mut rows = std::mem::take(&mut state.rows);
        rows.build(tracks, &state.collapsed);

        let s = ctx.styles();
        let input = ctx.input;
        out.hovered = !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        ctx.register_focus(self.id);

        self.advance(state, input.frame_dt);
        let l = self.layout(rect, state, rows.rows.len(), &s);
        state.scroll = l.scroll;
        self.pointer(&l, tracks, markers, &rows, state, drag, ctx, &mut out);
        let l = self.layout(rect, state, rows.rows.len(), &s);
        self.transport(&l, tracks, markers, state, ctx, &mut out);
        self.keyboard(&l, tracks, markers, state, ctx, &mut out);

        // Edits may have changed which rows there are and what they sum up.
        if !out.edits.is_empty() {
            rows.build(tracks, &state.collapsed);
        }
        let l = self.layout(rect, state, rows.rows.len(), &s);
        state.scroll = l.scroll;
        self.follow(&l, state);
        let l = self.layout(rect, state, rows.rows.len(), &s);
        state.scroll = l.scroll;

        self.paint(&l, tracks, markers, &rows, state, ctx, &mut out);
        self.rename_field(&l, markers, state, ctx, &mut out);
        if ctx.focus.is_focused(self.id) {
            ctx.draw_focus_ring(l.body.inset(2.0));
        }

        state.rows = rows;
        out.frame_changed = state.frame != start_frame;
        ctx.pop_debug_scope();
        out
    }

    /// Bring the state back inside this timeline's bounds.
    fn sanitize(&self, state: &mut TimelineState, markers: &[Marker]) {
        state.frame = state.frame.min(self.length);
        if let Some([lo, hi]) = state.range {
            let hi = hi.min(self.length).max(1);
            state.range = Some([lo.min(hi - 1), hi]);
        }
        state.zoom = state.zoom.clamp(ZOOM_RANGE[0], ZOOM_RANGE[1]);
        if state.marker_selected.is_some_and(|i| i >= markers.len()) {
            state.marker_selected = None;
        }
        if state
            .rename
            .as_ref()
            .is_some_and(|r| r.index >= markers.len())
        {
            state.rename = None;
        }
    }

    /// Advance playback by `dt` seconds.
    fn advance(&self, state: &mut TimelineState, dt: f32) {
        if !state.playing {
            state.clock = None;
            return;
        }
        let [lo, hi] = state.bounds(self.length);
        let fps = self.fps.max(1) as f32;
        let mut clock = match state.clock {
            Some(clock) if clock.shown == state.frame => clock,
            _ => {
                let from = if state.frame >= hi { lo } else { state.frame };
                Clock {
                    from,
                    elapsed: 0.0,
                    shown: from,
                }
            }
        };
        clock.elapsed += dt.max(0.0) * fps;
        let mut frame = clock.from as f32 + clock.elapsed.floor();
        if frame > hi as f32 {
            if !state.looping {
                state.frame = hi;
                state.playing = false;
                state.clock = None;
                return;
            }
            let past = clock.elapsed - (hi + 1 - clock.from) as f32;
            clock.from = lo;
            clock.elapsed = past.rem_euclid((hi + 1 - lo) as f32);
            frame = lo as f32 + clock.elapsed.floor();
        }
        state.frame = (frame as u32).min(hi);
        clock.shown = state.frame;
        state.clock = Some(clock);
    }

    fn set_frame(&self, state: &mut TimelineState, frame: i64) {
        state.frame = frame.clamp(0, self.length as i64) as u32;
    }

    /// Zoom to `ppf`, keeping the frame under lane-local `anchor` in place.
    fn zoom_to(&self, l: &Layout, state: &mut TimelineState, ppf: f32, anchor: Option<f32>) {
        let next = ppf.clamp(ZOOM_RANGE[0], ZOOM_RANGE[1]);
        let ax = anchor.unwrap_or(l.lanes.width * 0.5);
        let frame = (l.scroll[0] + ax - LPAD) / l.ppf;
        state.zoom = next;
        state.scroll[0] = (LPAD + frame * next - ax).max(0.0);
    }

    /// Zoom so the play range fills the lanes.
    fn fit(&self, l: &Layout, state: &mut TimelineState) {
        let [lo, hi] = state.bounds(self.length);
        let ppf = ((l.lanes.width - 2.0 * LPAD) / (hi - lo + 1) as f32)
            .clamp(ZOOM_RANGE[0], ZOOM_RANGE[1]);
        state.zoom = ppf;
        state.scroll[0] = lo as f32 * ppf;
    }

    /// Keep the playhead in view when the frame moved.
    fn follow(&self, l: &Layout, state: &mut TimelineState) {
        if state.seen_frame == Some(state.frame) {
            return;
        }
        state.seen_frame = Some(state.frame);
        let view = l.lanes.width;
        if view <= 0.0 {
            return;
        }
        let x = LPAD + state.frame as f32 * l.ppf;
        let left = l.scroll[0];
        if x > left + view - 24.0 {
            let lead = if state.playing {
                40.0
            } else {
                (view - 60.0).max(40.0)
            };
            state.scroll[0] = (x - lead).max(0.0);
        } else if x < left + 8.0 {
            state.scroll[0] = (x - 40.0).max(0.0);
        }
    }

    // ---- Pointer ----

    #[allow(clippy::too_many_arguments)]
    fn pointer(
        &self,
        l: &Layout,
        tracks: &mut [Track],
        markers: &mut [Marker],
        rows: &Rows,
        state: &mut TimelineState,
        drag: &mut DragCapture,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let input = ctx.input;
        let (mx, my) = (input.mouse_x, input.mouse_y);
        let over = !input.mouse_consumed && l.body.contains(mx, my);

        if over && !input.scroll_consumed {
            if input.ctrl_pressed && input.scroll_delta != 0.0 {
                let factor = if input.scroll_delta > 0.0 {
                    WHEEL_ZOOM
                } else {
                    1.0 / WHEEL_ZOOM
                };
                let anchor = (mx - l.lanes.x).clamp(0.0, l.lanes.width);
                self.zoom_to(l, state, l.ppf * factor, Some(anchor));
            } else {
                let (dx, dy) = if input.shift_pressed {
                    (input.scroll_delta, 0.0)
                } else {
                    (input.scroll_delta_x, input.scroll_delta)
                };
                state.scroll[0] -= dx * WHEEL_PX;
                state.scroll[1] -= dy * WHEEL_PX;
            }
        }

        if drag.is_active(self.id) {
            self.update_gesture(l, tracks, markers, rows, state, drag, ctx, out);
            return;
        }
        if !matches!(state.gesture, Gesture::Idle) {
            // Another widget cleared the capture under us.
            state.gesture = Gesture::Idle;
        }
        if !over || !drag.is_free() {
            if over {
                self.hover_cursor(l, markers, rows, tracks, state, ctx);
            }
            return;
        }
        if self
            .rename_rect(l, markers, state, ctx.draw_list, &ctx.styles())
            .is_some_and(|r| r.contains(mx, my))
        {
            return;
        }
        if !input.mouse_clicked {
            self.hover_cursor(l, markers, rows, tracks, state, ctx);
            return;
        }
        ctx.focus.request(self.id);
        self.press(l, tracks, markers, rows, state, drag, ctx, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn press(
        &self,
        l: &Layout,
        tracks: &mut [Track],
        markers: &mut [Marker],
        rows: &Rows,
        state: &mut TimelineState,
        drag: &mut DragCapture,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let input = ctx.input;
        let (mx, my) = (input.mouse_x, input.mouse_y);

        for axis in 0..2 {
            if let Some((_, thumb)) = l.thumb(axis)
                && thumb.contains(mx, my)
                && drag.try_begin(self.id)
            {
                let at = if axis == 0 {
                    mx - thumb.x
                } else {
                    my - thumb.y
                };
                state.gesture = Gesture::Thumb { axis, grab: at };
                return;
            }
        }

        if l.ruler.contains(mx, my) {
            if let Some(edge) = self.bracket_at(l, state, mx, my)
                && drag.try_begin(self.id)
            {
                let from = state.bounds(self.length);
                state.gesture = Gesture::Range { edge, from };
                return;
            }
            let s = ctx.styles();
            if let Some(index) = self.flag_at(l, markers, state, ctx.draw_list, &s, mx, my) {
                if input.mouse_double_clicked && self.markers_editable {
                    start_rename(state, index, &markers[index].label);
                    return;
                }
                if drag.try_begin(self.id) {
                    state.marker_selected = Some(index);
                    state.selection.clear();
                    state.gesture = Gesture::Marker {
                        index,
                        x0: mx,
                        t: markers[index].t,
                        moved: false,
                    };
                }
                return;
            }
            if drag.try_begin(self.id) {
                state.marker_selected = None;
                self.set_frame(state, l.frame_at(mx).floor() as i64);
                state.gesture = Gesture::Scrub;
            }
            return;
        }

        if l.labels.contains(mx, my) {
            if let Some(i) = l.row_at(my, rows.rows.len()) {
                let row = &rows.rows[i];
                let track = &tracks[row.track];
                let caret_x = l.labels.x + 6.0 + track.depth as f32 * INDENT;
                if track.group
                    && mx >= caret_x - 3.0
                    && mx < caret_x + 12.0
                    && !state.collapsed.remove(&track.id)
                {
                    state.collapsed.insert(track.id);
                }
            }
            return;
        }

        if !l.lanes.contains(mx, my) {
            return;
        }
        state.marker_selected = None;
        let row_index = l.row_at(my, rows.rows.len());
        if let Some(i) = row_index
            && let Some(t) = self.key_at(l, rows, &rows.rows[i], tracks, mx)
        {
            self.press_key(rows, &rows.rows[i], tracks, t, state, drag, input);
            return;
        }
        if input.mouse_double_clicked {
            if let Some(i) = row_index
                && self.editable
            {
                let t = (l.frame_at(mx).floor() as i64).clamp(0, self.length as i64) as u32;
                let row = &rows.rows[i];
                if !row.summary && !tracks[row.track].locked {
                    if let Some(edit) = model::add_key(tracks, row.track, t) {
                        out.edits.push(edit);
                    }
                    state.selection.clear();
                    state.selection.insert(KeyRef::new(tracks[row.track].id, t));
                }
            }
            return;
        }
        if drag.try_begin(self.id) {
            let add = input.shift_pressed || input.ctrl_pressed;
            let base = if add {
                state.selection.clone()
            } else {
                HashSet::new()
            };
            state.selection.clone_from(&base);
            let from = l.content_point(mx, my);
            state.gesture = Gesture::Marquee {
                from,
                to: from,
                base,
                moved: false,
            };
        }
    }

    /// A press on the key at frame `t` of `row`: select (or toggle), and
    /// start dragging the selection.
    #[allow(clippy::too_many_arguments)]
    fn press_key(
        &self,
        rows: &Rows,
        row: &Row,
        tracks: &[Track],
        t: u32,
        state: &mut TimelineState,
        drag: &mut DragCapture,
        input: &crate::InputState,
    ) {
        let mut ids = std::mem::take(&mut state.scratch);
        rows.keys_at(row, tracks, t, &mut ids);
        let all_in = ids.iter().all(|k| state.selection.contains(k));
        if input.shift_pressed || input.ctrl_pressed {
            for k in &ids {
                if all_in {
                    state.selection.remove(k);
                } else {
                    state.selection.insert(*k);
                }
            }
        } else if !all_in {
            state.selection.clear();
            state.selection.extend(ids.iter().copied());
        }
        state.scratch = ids;
        if !self.editable || (input.shift_pressed && all_in) {
            return;
        }
        let mut span: Option<[u32; 2]> = None;
        for k in &state.selection {
            if rows.is_locked(tracks, k.track) {
                continue;
            }
            span = Some(match span {
                None => [k.t, k.t],
                Some([a, b]) => [a.min(k.t), b.max(k.t)],
            });
        }
        if let Some([min_t, max_t]) = span
            && drag.try_begin(self.id)
        {
            state.gesture = Gesture::Keys {
                x0: input.mouse_x,
                delta: 0,
                lo: -(min_t as i64),
                hi: self.length as i64 - max_t as i64,
            };
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_gesture(
        &self,
        l: &Layout,
        tracks: &mut [Track],
        markers: &mut [Marker],
        rows: &Rows,
        state: &mut TimelineState,
        drag: &mut DragCapture,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let input = ctx.input;
        let (mx, my) = (input.mouse_x, input.mouse_y);
        let released = !input.mouse_down;
        let cancel = input.key_escape;
        let mut gesture = std::mem::take(&mut state.gesture);
        let mut done = released || cancel;
        match &mut gesture {
            Gesture::Idle => done = true,
            Gesture::Scrub => {
                ctx.request_cursor(CursorIcon::EwResize);
                self.set_frame(state, l.frame_at(mx).floor() as i64);
            }
            Gesture::Keys { x0, delta, lo, hi } => {
                ctx.request_cursor(CursorIcon::EwResize);
                *delta = (((mx - *x0) / l.ppf).round() as i64).clamp(*lo, *hi);
                if cancel {
                    *delta = 0;
                }
                if done && let Some(edit) = model::move_keys(tracks, &mut state.selection, *delta) {
                    out.edits.push(edit);
                }
            }
            Gesture::Marquee {
                from,
                to,
                base,
                moved,
            } => {
                *to = l.content_point(mx, my);
                *moved |= (to[0] - from[0]).abs().max((to[1] - from[1]).abs()) >= crate::CLICK_SLOP;
                state.selection.clone_from(base);
                if *moved && !cancel {
                    marquee_hits(l, rows, tracks, *from, *to, &mut state.selection);
                }
            }
            Gesture::Range { edge, from } => {
                ctx.request_cursor(CursorIcon::EwResize);
                let f = (l.frame_at(mx).floor() as i64).clamp(0, self.length as i64) as u32;
                let [lo, hi] = state.bounds(self.length);
                let next = if cancel {
                    *from
                } else if *edge == 0 {
                    [f.min(hi.saturating_sub(1)), hi]
                } else {
                    [lo, f.max(lo + 1).min(self.length.max(1))]
                };
                state.range = Some(next);
                if done && next != *from {
                    out.edits.push(TimelineEdit::RangeChanged {
                        from: *from,
                        to: next,
                    });
                }
            }
            Gesture::Marker {
                index,
                x0,
                t,
                moved,
            } => {
                let Some(original) = markers.get(*index).map(|m| m.t) else {
                    drag.release(self.id);
                    return;
                };
                if self.markers_editable && !cancel {
                    ctx.request_cursor(CursorIcon::Grabbing);
                    let d = ((mx - *x0) / l.ppf).round() as i64;
                    *moved |= d != 0;
                    *t = (original as i64 + d).clamp(0, self.length as i64) as u32;
                } else {
                    *t = original;
                }
                if released {
                    if *moved && *t != original {
                        markers[*index].t = *t;
                        out.edits.push(TimelineEdit::MarkerMoved {
                            index: *index,
                            from: original,
                            to: *t,
                        });
                    } else if !*moved && !cancel {
                        state.frame = original;
                    }
                }
            }
            Gesture::Thumb { axis, grab } => {
                if let Some((track, thumb)) = l.thumb(*axis) {
                    let (pointer, start, len, size) = if *axis == 0 {
                        (mx, track.x, track.width, thumb.width)
                    } else {
                        (my, track.y, track.height, thumb.height)
                    };
                    let room = (len - size).max(1.0);
                    let at = (pointer - *grab - start).clamp(0.0, room);
                    state.scroll[*axis] = at / room * l.max_scroll()[*axis];
                }
            }
        }
        if done {
            drag.release(self.id);
        } else {
            state.gesture = gesture;
        }
    }

    /// The cursor for what the pointer is over, with no gesture held.
    fn hover_cursor(
        &self,
        l: &Layout,
        markers: &[Marker],
        rows: &Rows,
        tracks: &[Track],
        state: &TimelineState,
        ctx: &mut DrawContext,
    ) {
        let (mx, my) = (ctx.input.mouse_x, ctx.input.mouse_y);
        if l.ruler.contains(mx, my) {
            let s = ctx.styles();
            let icon = if self.bracket_at(l, state, mx, my).is_some() {
                CursorIcon::EwResize
            } else if self.markers_editable
                && self
                    .flag_at(l, markers, state, ctx.draw_list, &s, mx, my)
                    .is_some()
            {
                CursorIcon::Grab
            } else {
                CursorIcon::Pointer
            };
            ctx.request_cursor(icon);
        } else if self.editable
            && l.lanes.contains(mx, my)
            && let Some(i) = l.row_at(my, rows.rows.len())
        {
            let row = &rows.rows[i];
            if !row.summary
                && !tracks[row.track].locked
                && self.key_at(l, rows, row, tracks, mx).is_some()
            {
                ctx.request_cursor(CursorIcon::EwResize);
            }
        }
    }

    /// The range bracket under the pointer: 0 the start, 1 the end.
    fn bracket_at(&self, l: &Layout, state: &TimelineState, mx: f32, my: f32) -> Option<usize> {
        let [lo, hi] = state.range.filter(|_| self.range_editable)?;
        if my < l.ruler.bottom() - BRACKET_H {
            return None;
        }
        [hi + 1, lo]
            .into_iter()
            .zip([1, 0])
            .find(|&(f, _)| (mx - l.x(f as f32)).abs() <= BRACKET_HIT)
            .map(|(_, edge)| edge)
    }

    /// The marker whose flag is under the pointer; the selected one wins,
    /// then the later ones (drawn on top).
    #[allow(clippy::too_many_arguments)]
    fn flag_at(
        &self,
        l: &Layout,
        markers: &[Marker],
        state: &TimelineState,
        list: &mut DrawList,
        s: &StyleResolver,
        mx: f32,
        my: f32,
    ) -> Option<usize> {
        let hit = |i: usize, list: &mut DrawList| {
            flag_rect(l, list, s, markers[i].t, &markers[i].label).contains(mx, my)
        };
        if let Some(i) = state.marker_selected
            && hit(i, list)
        {
            return Some(i);
        }
        (0..markers.len()).rev().find(|&i| hit(i, list))
    }

    /// The frame of the key of `row` under screen x.
    fn key_at(&self, l: &Layout, rows: &Rows, row: &Row, tracks: &[Track], mx: f32) -> Option<u32> {
        let times = rows.row_times(row, tracks);
        let from = l.frame_at(mx - KEY_HIT).floor().max(0.0) as u32;
        let to = l.frame_at(mx + KEY_HIT).ceil().max(0.0) as u32;
        let mut best: Option<(f32, u32)> = None;
        for j in times.lower_bound(from)..times.len() {
            let t = times.t(j);
            if t > to {
                break;
            }
            let d = (l.xc(t) - mx).abs();
            if d <= KEY_HIT && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, t));
            }
        }
        best.map(|(_, t)| t)
    }

    // ---- Keyboard ----

    fn keyboard(
        &self,
        l: &Layout,
        tracks: &mut [Track],
        markers: &mut Vec<Marker>,
        state: &mut TimelineState,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        if !ctx.focus.is_focused(self.id) {
            return;
        }
        let input = ctx.input;
        let keys = &input.keys;
        let [lo, hi] = state.bounds(self.length);
        let step = if input.shift_pressed { 10 } else { 1 };
        let frame = state.frame as i64;
        if input.key_left {
            self.set_frame(state, frame - step);
        } else if input.key_right {
            self.set_frame(state, frame + step);
        } else if input.key_home {
            state.frame = lo;
        } else if input.key_end {
            state.frame = hi;
        } else if keys.was_pressed(Key::Char(',')) {
            state.frame = model::prev_key(tracks, state.frame).unwrap_or(lo);
        } else if keys.was_pressed(Key::Char('.')) {
            state.frame = model::next_key(tracks, state.frame).unwrap_or(hi);
        } else if input.key_space {
            state.playing = !state.playing;
        } else if input.key_delete || input.backspace_pressed {
            if !state.selection.is_empty() {
                if self.editable
                    && let Some(edit) = model::delete_keys(tracks, &mut state.selection)
                {
                    out.edits.push(edit);
                }
            } else if let Some(index) = state.marker_selected
                && self.markers_editable
            {
                let marker = markers.remove(index);
                state.marker_selected = None;
                state.rename = None;
                out.edits
                    .push(TimelineEdit::MarkerDeleted { index, marker });
            }
        } else if input.key_escape {
            state.selection.clear();
            state.marker_selected = None;
        } else if keys.was_pressed(Key::Char('M')) && !input.ctrl_pressed && !input.alt_down {
            self.add_marker(markers, state, out);
        } else if input.key_select_all {
            state.selection = model::all_keys(tracks);
        } else if keys.was_pressed(Key::Char('=')) || keys.was_pressed(Key::Char('+')) {
            self.zoom_to(l, state, l.ppf * ZOOM_STEP, None);
        } else if keys.was_pressed(Key::Char('-')) {
            self.zoom_to(l, state, l.ppf / ZOOM_STEP, None);
        }
    }

    /// Add a marker at the playhead and open its name, or open the name of
    /// the one already there.
    fn add_marker(
        &self,
        markers: &mut Vec<Marker>,
        state: &mut TimelineState,
        out: &mut TimelineOutput,
    ) {
        if !self.markers_editable {
            return;
        }
        if let Some(index) = markers.iter().position(|m| m.t == state.frame) {
            state.marker_selected = Some(index);
            start_rename(state, index, &markers[index].label);
            return;
        }
        let index = markers.len();
        markers.push(Marker::new(state.frame, format!("Marker {}", index + 1)));
        out.edits.push(TimelineEdit::MarkerAdded { index });
        state.marker_selected = Some(index);
        state.selection.clear();
        start_rename(state, index, &markers[index].label);
    }

    // ---- Transport ----

    fn transport(
        &self,
        l: &Layout,
        tracks: &[Track],
        markers: &mut Vec<Marker>,
        state: &mut TimelineState,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let s = ctx.styles();
        let chrome = s.timeline();
        let strip = l.strip;
        ctx.draw_list.edge_line(
            strip,
            Edge::Bottom,
            chrome.transport_rule.thickness,
            chrome.transport_rule.color,
        );
        let below = Rect::new(strip.x, strip.bottom(), strip.width, 1.0);
        ctx.draw_list.edge_line(
            below,
            Edge::Top,
            chrome.transport_highlight.thickness,
            chrome.transport_highlight.color,
        );
        ctx.draw_list.push_clip(strip);

        let travel = Pressable::new().travel_px(&s);
        let y = strip.y + STRIP_PAD[0];
        let h = KEY + travel;
        let [lo, hi] = state.bounds(self.length);
        let mut x = strip.x + STRIP_PAD[3];
        let key = |x: &mut f32, w: f32| {
            let r = Rect::new(*x, y, w, h);
            *x += w + KEY_GAP;
            r
        };
        let first = key(&mut x, KEY);
        let prev = key(&mut x, KEY);
        let play = key(&mut x, PLAY_W);
        let next = key(&mut x, KEY);
        let last = key(&mut x, KEY);
        if icon_key(ctx, first, PhosphorIcon::SkipBack, false) {
            state.frame = lo;
        }
        if icon_key(ctx, prev, PhosphorIcon::CaretLeft, false) {
            state.frame = model::prev_key(tracks, state.frame).unwrap_or(lo);
        }
        let glyph = if state.playing {
            PhosphorIcon::Pause
        } else {
            PhosphorIcon::Play
        };
        if icon_key(ctx, play, glyph, state.playing) {
            state.playing = !state.playing;
        }
        if icon_key(ctx, next, PhosphorIcon::CaretRight, false) {
            state.frame = model::next_key(tracks, state.frame).unwrap_or(hi);
        }
        if icon_key(ctx, last, PhosphorIcon::SkipForward, false) {
            state.frame = hi;
        }

        // Right cluster, laid out from the right edge first: the readouts on
        // the left give way to it when the strip is narrow.
        let mut right = strip.right() - STRIP_PAD[1];
        let rkey = |x: &mut f32, w: f32| {
            *x -= w;
            let r = Rect::new(*x, y, w, h);
            *x -= KEY_GAP;
            r
        };
        let fit_w = text_key_width(ctx.draw_list, &s, "Fit");
        let fit = rkey(&mut right, fit_w);
        let zoom_in = rkey(&mut right, KEY);
        let zoom_out = rkey(&mut right, KEY);
        right -= 6.0;
        let looping = rkey(&mut right, KEY);
        let marker = self.markers_editable.then(|| {
            let w = text_key_width(ctx.draw_list, &s, "+ Marker");
            rkey(&mut right, w)
        });
        if text_key(ctx, fit, "Fit") {
            self.fit(l, state);
        }
        if icon_key(ctx, zoom_in, PhosphorIcon::Plus, false) {
            self.zoom_to(l, state, l.ppf * ZOOM_STEP, None);
        }
        if icon_key(ctx, zoom_out, PhosphorIcon::Minus, false) {
            self.zoom_to(l, state, l.ppf / ZOOM_STEP, None);
        }
        if icon_key(ctx, looping, PhosphorIcon::Repeat, state.looping) {
            state.looping = !state.looping;
        }
        if let Some(marker) = marker
            && text_key(ctx, marker, "+ Marker")
        {
            self.add_marker(markers, state, out);
        }

        // Frame field and readouts.
        x += 8.0 - KEY_GAP;
        let field = Rect::new(x, y, FIELD_W, KEY);
        let number = NumberInput::new()
            .with_range(0.0, self.length as f64)
            .with_step(1.0)
            .with_decimals(0)
            .draw(
                state.frame as f64,
                part_id(self.id, FRAME_FIELD),
                &mut state.frame_field,
                field,
                ctx,
            );
        if number.changed {
            self.set_frame(state, number.value.round() as i64);
        }
        x = field.right() + READOUT_GAP;
        let list = &mut *ctx.draw_list;
        let mono = s.theme().mono_font.clone();
        // A readout is drawn only while it fits left of the right cluster;
        // the first that doesn't ends the row.
        let limit = right - READOUT_GAP;
        let mut fits = true;
        let mut readout = |list: &mut DrawList, x: &mut f32, text: String, size: f32, ink: Ink| {
            let ty = list.vcentered_text_y(y, KEY, size, mono.as_ref(), "0");
            let mut block = TextBlock::new(text, *x, ty)
                .with_size(size)
                .with_color_f32(s.ink(ink))
                .with_font_opt(mono.clone());
            if size < s.text_size(TextSize::Meta) {
                block = block.with_letter_spacing(size * 0.08);
            }
            let w = list.measure_block(&block).0;
            fits &= *x + w <= limit;
            if fits {
                list.text(block);
                *x += w + READOUT_GAP;
            }
        };
        readout(
            list,
            &mut x,
            format!("/ {}", self.length),
            s.text_size(TextSize::Meta),
            Ink::Dim,
        );
        x += 4.0;
        readout(
            list,
            &mut x,
            timecode(state.frame, self.fps),
            s.text_size(TextSize::Meta),
            Ink::Value,
        );
        readout(
            list,
            &mut x,
            format!("{} FPS", self.fps),
            s.text_size(TextSize::Caption),
            Ink::Caption,
        );
        let left_end = x;

        let count = state.selection.len();
        if count > 0 {
            let list = &mut *ctx.draw_list;
            let text = format!("{count} {}", if count == 1 { "key" } else { "keys" });
            let size = 9.5;
            let w = list
                .measure_text_with_font(&text, size, None, mono.as_ref())
                .0;
            let tx = right - 6.0 - w;
            if tx > left_end {
                let ty = list.vcentered_text_y(y, KEY, size, mono.as_ref(), "0");
                list.text(
                    TextBlock::new(text, tx, ty)
                        .with_size(size)
                        .with_color_f32(s.ink(Ink::Muted))
                        .with_font_opt(mono.clone()),
                );
            }
        }
        ctx.draw_list.pop_clip();
    }

    // ---- Painting ----

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        l: &Layout,
        tracks: &mut [Track],
        markers: &[Marker],
        rows: &Rows,
        state: &TimelineState,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let s = ctx.styles();
        let chrome = s.timeline();
        let [lo, hi] = state.bounds(self.length);
        let delta = match state.gesture {
            Gesture::Keys { delta, .. } => delta,
            _ => 0,
        };
        let marker_t = |i: usize| match state.gesture {
            Gesture::Marker { index, t, .. } if index == i => t,
            _ => markers[i].t,
        };
        let visible = l.visible_rows(rows.rows.len());
        let f_lo = l.frame_at(l.lanes.x).floor().max(0.0) as u32;
        let f_hi = (l.frame_at(l.lanes.right()).ceil().max(0.0) as u32).min(self.length);
        let [major, _, fine] = model::tick_steps(l.ppf);

        // Ruler background, across the corner too.
        {
            let list = &mut *ctx.draw_list;
            let band = Rect::new(l.body.x, l.body.y, l.body.width, l.ruler.height);
            list.paint_quad_background(band, chrome.ruler, CornerRadii::uniform(0.0));
            list.edge_line(
                band,
                Edge::Bottom,
                chrome.ruler_rule.thickness,
                chrome.ruler_rule.color,
            );
        }

        // Lanes: grid, rows, then the overlay of range, markers and playhead.
        {
            let list = &mut *ctx.draw_list;
            list.push_clip(l.lanes);
            let grid_from = f_lo - f_lo % fine;
            let mut f = grid_from;
            while f <= f_hi + 1 {
                let major_line = f.is_multiple_of(major);
                if major_line || fine != major {
                    let color = chrome.grid[usize::from(!major_line)];
                    list.quad(l.x(f as f32), l.lanes.y, 1.0, l.lanes.height, color);
                }
                f += fine;
            }
            for i in visible.clone() {
                let row = &rows.rows[i];
                let track = &tracks[row.track];
                let y = l.row_y(i);
                let band = Rect::new(l.lanes.x, y, l.lanes.width, ROW);
                if track.group {
                    list.quad(band.x, y, band.width, ROW, chrome.group_lane);
                } else if i % 2 == 1 {
                    list.quad(band.x, y, band.width, ROW, s.color(StyleKey::RowZebra));
                }
                list.edge_line(
                    band,
                    Edge::Bottom,
                    chrome.row_rule.thickness,
                    chrome.row_rule.color,
                );
                let keys = RowPaint {
                    row,
                    cy: y + ROW * 0.5,
                    delta,
                    frames: [f_lo, f_hi],
                };
                paint_keys(l, list, &s, &chrome, rows, tracks, state, keys);
            }
            if state.range.is_some() {
                let left = l.lanes.x - l.scroll[0];
                let right = l.lanes.x - l.scroll[0] + l.content[0];
                let shade = chrome.out_of_range;
                list.quad(
                    left,
                    l.lanes.y,
                    l.x(lo as f32) - left,
                    l.lanes.height,
                    shade,
                );
                let end = l.x((hi + 1) as f32);
                list.quad(end, l.lanes.y, right - end, l.lanes.height, shade);
                let mut cut = s.color(StyleKey::AccentGlow);
                cut[3] *= 0.55;
                list.quad(l.x(lo as f32) - 1.0, l.lanes.y, 1.0, l.lanes.height, cut);
                list.quad(end, l.lanes.y, 1.0, l.lanes.height, cut);
            }
            for i in 0..markers.len() {
                let x = l.xc(marker_t(i)).floor() + 0.5;
                let color = if state.marker_selected == Some(i) {
                    s.color(StyleKey::WarnRule)
                } else {
                    s.color(StyleKey::WarnSoft)
                };
                list.stroke_line(
                    [x, l.lanes.y],
                    [x, l.lanes.bottom()],
                    &Stroke::new(1.0).dashed(3.0, 3.0),
                    color,
                );
            }
            list.pop_clip();
        }

        // Ruler: ticks, range, playhead column, its badge, markers.
        {
            let list = &mut *ctx.draw_list;
            list.push_clip(l.ruler);
            let mono = s.theme().mono_font.clone();
            let size = s.text_size(TextSize::Caption);
            let label_y =
                list.vcentered_text_y(l.ruler.y + 2.0, RULER - 10.0, size, mono.as_ref(), "0");
            let tick_from = f_lo - f_lo % fine;
            let mut f = tick_from;
            while f <= f_hi {
                let major_tick = f.is_multiple_of(major);
                let x = l.x(f as f32);
                let (h, color) = if major_tick {
                    (9.0, chrome.ticks[0])
                } else {
                    (4.0, chrome.ticks[1])
                };
                list.quad(x, l.ruler.bottom() - h, 1.0, h, color);
                if major_tick {
                    let ink = if (lo..=hi).contains(&f) {
                        Ink::Muted
                    } else {
                        Ink::Disabled
                    };
                    list.text(
                        TextBlock::new(f.to_string(), x + 4.0, label_y)
                            .with_size(size)
                            .with_color_f32(s.ink(ink))
                            .with_font_opt(mono.clone()),
                    );
                }
                f += fine;
            }
            if state.range.is_some() {
                paint_range(list, &s, l, lo, hi);
            }
            list.pop_clip();

            let column = Rect::new(
                l.ruler.x,
                l.ruler.y,
                l.ruler.width,
                l.ruler.height + l.lanes.height,
            );
            list.push_clip(column);
            paint_frame_column(list, &s, l, state.frame, column);
            list.pop_clip();

            list.push_clip(l.ruler);
            paint_badge(list, &s, l, state.frame);
            let order = (0..markers.len())
                .filter(|&i| state.marker_selected != Some(i))
                .chain(state.marker_selected);
            for i in order {
                if state.renaming() == Some(i) {
                    paint_flag_mark(list, &s, l, marker_t(i), true);
                    continue;
                }
                paint_flag(
                    list,
                    &s,
                    &chrome,
                    l,
                    marker_t(i),
                    &markers[i].label,
                    state.marker_selected == Some(i),
                );
            }
            list.pop_clip();
        }

        self.paint_labels(l, tracks, rows, visible, ctx, out);

        // The corner above the labels.
        {
            let list = &mut *ctx.draw_list;
            list.edge_line(
                l.corner,
                Edge::Right,
                chrome.label_rule.thickness,
                chrome.label_rule.color,
            );
            list.push_clip(l.corner);
            let mono = s.theme().mono_font.clone();
            let size = s.text_size(TextSize::Caption);
            let spacing = size * s.scalar(StyleKey::Tracking(Tracking::Caption));
            let ty = list.vcentered_text_y(l.corner.y, l.corner.height, size, mono.as_ref(), "T");
            let block = TextBlock::new("TRACKS", l.corner.x + 9.0, ty)
                .with_size(size)
                .with_letter_spacing(spacing)
                .with_color_f32(s.ink(Ink::Glyph))
                .with_font_opt(mono.clone());
            let w = list.measure_block(&block).0;
            list.text(block);
            let count = tracks.iter().filter(|t| !t.group).count();
            list.text(
                TextBlock::new(count.to_string(), l.corner.x + 9.0 + w + 6.0, ty)
                    .with_size(size)
                    .with_letter_spacing(spacing)
                    .with_color_f32(s.ink(Ink::Caption))
                    .with_font_opt(mono),
            );
            list.pop_clip();
        }

        let list = &mut *ctx.draw_list;
        if let Gesture::Marquee {
            from,
            to,
            moved: true,
            ..
        } = &state.gesture
        {
            list.push_clip(l.lanes);
            let x0 = from[0].min(to[0]) + l.lanes.x - l.scroll[0];
            let y0 = from[1].min(to[1]) + l.lanes.y - l.scroll[1];
            let r = Rect::new(x0, y0, (to[0] - from[0]).abs(), (to[1] - from[1]).abs());
            list.chrome_rect(
                r,
                0.0,
                1.0,
                s.color(StyleKey::AccentRing),
                s.color(StyleKey::Accent),
            );
            list.pop_clip();
        }

        for axis in 0..2 {
            if let Some((_, thumb)) = l.thumb(axis) {
                let active = matches!(state.gesture, Gesture::Thumb { axis: a, .. } if a == axis);
                let hovered = !ctx.input.mouse_consumed
                    && thumb.contains(ctx.input.mouse_x, ctx.input.mouse_y);
                let face = if active {
                    2
                } else if hovered {
                    1
                } else {
                    0
                };
                let bar = s.scrollbar();
                let padding = thumb.inset(bar.thumb[face].border_widths.left);
                let mut painter = SurfacePainter::new(
                    list,
                    thumb,
                    padding,
                    bar.thumb[face].corner_radii,
                    bar.thumb[face],
                    &bar.thumb_insets[face],
                    &[],
                );
                painter.paint_pre_content();
                painter.paint_post_content();
            }
        }

        if rows.rows.is_empty() {
            let area = Rect::new(
                l.body.x,
                l.lanes.y,
                l.body.width,
                80.0_f32.min(l.lanes.height),
            );
            let size = s.text_size(TextSize::Row);
            let font = s.theme().font.clone();
            let w = list
                .measure_text_with_font(EMPTY, size, None, font.as_ref())
                .0;
            let ty = list.vcentered_text_y(area.y, area.height, size, font.as_ref(), EMPTY);
            list.push_clip(l.body);
            list.text(
                TextBlock::new(EMPTY, area.x + (area.width - w) * 0.5, ty)
                    .with_size(size)
                    .with_color_f32(s.ink(Ink::Dim))
                    .with_font_opt(font),
            );
            list.pop_clip();
        }
    }

    /// The label column: each visible row's name, disclosure, swatch, and
    /// mute and lock keys.
    fn paint_labels(
        &self,
        l: &Layout,
        tracks: &mut [Track],
        rows: &Rows,
        visible: std::ops::Range<usize>,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let s = ctx.styles();
        let chrome = s.timeline();
        let input = ctx.input;
        let hovered_row = (!input.mouse_consumed
            && Rect::new(l.labels.x, l.labels.y, l.body.width, l.labels.height)
                .contains(input.mouse_x, input.mouse_y))
        .then(|| l.row_at(input.mouse_y, rows.rows.len()))
        .flatten();
        ctx.draw_list.push_clip(l.labels);
        for i in visible {
            let row = &rows.rows[i];
            let y = l.row_y(i);
            let cell = Rect::new(l.labels.x, y, l.labels.width, ROW);
            let (id, depth, group, color, muted, locked) = {
                let t = &tracks[row.track];
                (t.id, t.depth, t.group, t.color, t.muted, t.locked)
            };
            {
                let list = &mut *ctx.draw_list;
                list.paint_quad_background(
                    cell,
                    chrome.label[usize::from(group)],
                    CornerRadii::uniform(0.0),
                );
                if hovered_row == Some(i) {
                    list.quad(cell.x, y, cell.width, ROW, s.color(StyleKey::RowHover));
                }
                list.edge_line(
                    cell,
                    Edge::Bottom,
                    chrome.row_rule.thickness,
                    chrome.row_rule.color,
                );
            }
            let cy = y + ROW * 0.5;
            let mut x = cell.x + 6.0 + depth as f32 * INDENT;
            if group {
                let open = !row.collapsed;
                draw_disclosure(
                    ctx.draw_list,
                    x + 4.5,
                    cy,
                    CARET_HALF,
                    open,
                    s.ink(Ink::Body2),
                );
            }
            x += 9.0 + 5.0;
            if let Some(color) = color {
                ctx.draw_list.chrome_rect(
                    Rect::new(x, cy - 5.5, 3.0, 11.0),
                    1.0,
                    0.0,
                    color,
                    [0.0; 4],
                );
                x += 3.0 + 5.0;
            }

            // Mute and lock keys, right-aligned.
            let lock_rect = Rect::new(
                cell.right() - 4.0 - TRACK_KEY,
                cy - TRACK_KEY * 0.5,
                TRACK_KEY,
                TRACK_KEY,
            );
            let mute_rect = Rect::new(
                lock_rect.x - 5.0 - TRACK_KEY,
                lock_rect.y,
                TRACK_KEY,
                TRACK_KEY,
            );
            let reveal = self.editable && hovered_row == Some(i);
            if self.editable && (reveal || muted) && track_key(ctx, mute_rect, "M", muted) {
                tracks[row.track].muted = !muted;
                out.edits.push(TimelineEdit::TrackMuted {
                    track: id,
                    muted: !muted,
                });
            }
            if self.editable && (reveal || locked) && track_key(ctx, lock_rect, "L", locked) {
                tracks[row.track].locked = !locked;
                out.edits.push(TimelineEdit::TrackLocked {
                    track: id,
                    locked: !locked,
                });
            }

            let list = &mut *ctx.draw_list;
            let size = s.text_size(TextSize::Row);
            let font = s.theme().font.clone();
            let label = &tracks[row.track].label;
            let ink = if muted {
                Ink::Disabled
            } else if group {
                Ink::Title
            } else {
                Ink::Row
            };
            let ty = list.vcentered_text_y(y, ROW, size, font.as_ref(), label);
            let mut block = TextBlock::new(label.as_str(), x, ty)
                .with_size(size)
                .with_color_f32(s.ink(ink))
                .with_font_opt(font)
                .with_max_width((mute_rect.x - 5.0 - x).max(0.0))
                .with_ellipsis()
                .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0);
            if group {
                block = block.with_weight(crate::Weight::SEMIBOLD);
            }
            list.text(block);
        }
        let list = &mut *ctx.draw_list;
        list.pop_clip();
        list.edge_line(
            l.labels,
            Edge::Right,
            chrome.label_rule.thickness,
            chrome.label_rule.color,
        );
    }

    /// Where the rename field goes, while a marker is being renamed.
    fn rename_rect(
        &self,
        l: &Layout,
        markers: &[Marker],
        state: &TimelineState,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> Option<Rect> {
        let rename = state.rename.as_ref()?;
        let marker = markers.get(rename.index)?;
        let size = s.scalar(StyleKey::FontSize);
        let w = list
            .measure_text_with_font(&rename.input.value, size, None, s.theme().font.as_ref())
            .0;
        let x = l.xc(marker.t) - 4.0 + 8.0 + 3.0;
        Some(Rect::new(
            x,
            l.ruler.y + 1.0,
            (w + 16.0).max(RENAME_MIN_W),
            RENAME_H,
        ))
    }

    /// The marker rename field: Enter or a click elsewhere commits, Esc
    /// cancels.
    fn rename_field(
        &self,
        l: &Layout,
        markers: &mut [Marker],
        state: &mut TimelineState,
        ctx: &mut DrawContext,
        out: &mut TimelineOutput,
    ) {
        let s = ctx.styles();
        let Some(rect) = self.rename_rect(l, markers, state, ctx.draw_list, &s) else {
            return;
        };
        let field = part_id(self.id, RENAME_FIELD);
        let Some(rename) = state.rename.as_mut() else {
            return;
        };
        let input = ctx.input;
        let mut commit = false;
        if !rename.armed {
            ctx.focus.focus(field);
            rename.armed = true;
        } else if !ctx.focus.is_focused(field) {
            commit = true;
        } else if input.enter_pressed {
            commit = true;
            ctx.focus.focus(self.id);
        } else if input.key_escape {
            state.rename = None;
            ctx.focus.focus(self.id);
            return;
        }
        if commit {
            let index = rename.index;
            let name = rename.input.value.trim();
            if !name.is_empty() && name != markers[index].label {
                let to = name.to_string();
                let from = std::mem::replace(&mut markers[index].label, to.clone());
                out.edits
                    .push(TimelineEdit::MarkerRenamed { index, from, to });
            }
            state.rename = None;
            return;
        }
        ctx.draw_list.push_clip(l.ruler);
        rename.input.x = rect.x;
        rename.input.y = rect.y;
        rename.input.width = rect.width;
        rename.input.height = rect.height;
        rename.input.draw(field, ctx);
        ctx.draw_list.pop_clip();
    }
}

/// What [`paint_keys`] draws: a row, its centre line, how far the dragged
/// keys have moved, and the frames on screen.
struct RowPaint<'r> {
    row: &'r Row,
    cy: f32,
    delta: i64,
    frames: [u32; 2],
}

/// One row's keys: the line from first to last, hold bars, the keys. Only
/// the keys on screen (and the hold bars reaching onto it) are drawn.
#[allow(clippy::too_many_arguments)]
fn paint_keys(
    l: &Layout,
    list: &mut DrawList,
    s: &StyleResolver,
    chrome: &TimelineChrome,
    rows: &Rows,
    tracks: &[Track],
    state: &TimelineState,
    p: RowPaint,
) {
    let RowPaint {
        row,
        cy,
        delta,
        frames: [f_lo, f_hi],
    } = p;
    let track = &tracks[row.track];
    let times = rows.row_times(row, tracks);
    let n = times.len();
    if n == 0 {
        return;
    }
    let shifts = delta != 0 && !row.summary && !track.locked;
    let shown = |t: u32| -> u32 {
        if shifts && state.selection.contains(&KeyRef::new(track.id, t)) {
            (t as i64 + delta).max(0) as u32
        } else {
            t
        }
    };
    let dim = track.muted || track.locked;
    if n > 1 {
        let span = if dim {
            chrome.key_span[2]
        } else if row.summary {
            chrome.key_span[1]
        } else {
            chrome.key_span[0]
        };
        let a = l.xc(shown(times.t(0)));
        let b = l.xc(shown(times.t(n - 1)));
        list.quad(a, cy - 0.5, b - a, 1.0, span);
    }
    let pad = delta.unsigned_abs() as u32 + 1;
    let first = times
        .lower_bound(f_lo.saturating_sub(pad))
        .saturating_sub(1);
    let last = (times.lower_bound(f_hi.saturating_add(pad + 1)) + 1).min(n);
    let hold = chrome.hold_bar[usize::from(dim)];
    for j in first..last {
        if times.interp(j) == Interp::Hold && j + 1 < n {
            let a = l.xc(shown(times.t(j)));
            let b = l.xc(shown(times.t(j + 1)));
            if b > a {
                list.chrome_rect(Rect::new(a, cy - 1.5, b - a, 3.0), 1.0, 0.0, hold, [0.0; 4]);
            }
        }
    }
    if track.muted {
        list.push_tint();
        list.multiply_tint([1.0, 1.0, 1.0, MUTED_ALPHA]);
    }
    for j in first..last {
        let t = times.t(j);
        let selected = if row.summary {
            tracks[row.kids.clone()]
                .iter()
                .any(|k| !k.group && state.selection.contains(&KeyRef::new(k.id, t)))
        } else {
            state.selection.contains(&KeyRef::new(track.id, t))
        };
        let shape = if row.summary {
            KeyShape::Summary
        } else {
            KeyShape::Key(times.interp(j))
        };
        paint_key(
            list,
            s,
            chrome,
            l.xc(shown(t)),
            cy,
            shape,
            selected,
            track.locked,
        );
    }
    if track.muted {
        list.pop_tint();
    }
}

/// Start editing marker `index`'s name.
fn start_rename(state: &mut TimelineState, index: usize, label: &str) {
    let mut input = TextInput::new(0.0, 0.0, 0.0, 0.0);
    input.set_value(label);
    input.select_all();
    state.rename = Some(Rename {
        index,
        input,
        armed: false,
    });
}

/// Add every key inside the marquee from `from` to `to` (lane content space)
/// to `selection`.
fn marquee_hits(
    l: &Layout,
    rows: &Rows,
    tracks: &[Track],
    from: [f32; 2],
    to: [f32; 2],
    selection: &mut HashSet<KeyRef>,
) {
    let (x0, x1) = (from[0].min(to[0]), from[0].max(to[0]));
    let (y0, y1) = (from[1].min(to[1]), from[1].max(to[1]));
    // Rows whose centre is inside, and frames whose centre is inside.
    let first = ((y0 - ROW * 0.5) / ROW).ceil().max(0.0) as usize;
    let last = (((y1 - ROW * 0.5) / ROW).floor() + 1.0).max(0.0) as usize;
    let t0 = ((x0 - LPAD) / l.ppf - 0.5).ceil().max(0.0) as u32;
    let t1 = (x1 - LPAD) / l.ppf - 0.5;
    if t1 < 0.0 {
        return;
    }
    let t1 = t1.floor() as u32;
    for row in rows.rows.iter().take(last).skip(first) {
        let times = rows.row_times(row, tracks);
        for j in times.lower_bound(t0)..times.len() {
            let t = times.t(j);
            if t > t1 {
                break;
            }
            if row.summary {
                for kid in &tracks[row.kids.clone()] {
                    if !kid.group && kid.key_at(t).is_some() {
                        selection.insert(KeyRef::new(kid.id, t));
                    }
                }
            } else {
                selection.insert(KeyRef::new(tracks[row.track].id, t));
            }
        }
    }
}

/// The rect of a marker's flag and label, for hit testing.
fn flag_rect(l: &Layout, list: &mut DrawList, s: &StyleResolver, t: u32, label: &str) -> Rect {
    let w = list
        .measure_text_with_font(label, MARKER_SIZE, None, s.theme().mono_font.as_ref())
        .0;
    let x = l.xc(t) - 4.0;
    Rect::new(x, l.ruler.y + 1.0, 8.0 + 3.0 + w + 6.0, MARKER_H)
}

/// A marker's triangle.
fn paint_flag_mark(list: &mut DrawList, s: &StyleResolver, l: &Layout, t: u32, selected: bool) {
    let x = l.xc(t);
    let ty = l.ruler.y + 1.0 + (MARKER_H - 6.0) * 0.5;
    let color = if selected {
        s.color(StyleKey::WarnRule)
    } else {
        s.color(StyleKey::WarnMeta)
    };
    list.triangle((x - 4.0, ty), (x + 4.0, ty), (x, ty + 6.0), color);
}

/// A marker's flag: its triangle and its labelled tag.
fn paint_flag(
    list: &mut DrawList,
    s: &StyleResolver,
    chrome: &TimelineChrome,
    l: &Layout,
    t: u32,
    label: &str,
    selected: bool,
) {
    paint_flag_mark(list, s, l, t, selected);
    let mono = s.theme().mono_font.clone();
    let w = list
        .measure_text_with_font(label, MARKER_SIZE, None, mono.as_ref())
        .0;
    let tag = Rect::new(
        l.xc(t) - 4.0 + 8.0 + 3.0,
        l.ruler.y + 1.0,
        w + 6.0,
        MARKER_H,
    );
    let radii = CornerRadii::uniform(2.0);
    if selected {
        list.paint_quad_background(
            tag,
            crate::Background::Solid(s.color(StyleKey::WarnSoft)),
            radii,
        );
    } else {
        list.paint_quad_background(tag, chrome.ruler, radii);
    }
    let ink = if selected {
        s.color(StyleKey::WarnBannerInk)
    } else {
        s.color(StyleKey::WarnMeta)
    };
    let ty = list.vcentered_text_y(tag.y, tag.height, MARKER_SIZE, mono.as_ref(), label);
    list.text(
        TextBlock::new(label, tag.x + 3.0, ty)
            .with_size(MARKER_SIZE)
            .with_color_f32(ink)
            .with_font_opt(mono),
    );
}

/// The play range on the ruler: a bar under it and a bracket at each end.
fn paint_range(list: &mut DrawList, s: &StyleResolver, l: &Layout, lo: u32, hi: u32) {
    let accent = s.color(StyleKey::Accent);
    let mut bar = accent;
    bar[3] *= 0.75;
    let start = l.x(lo as f32);
    let end = l.x((hi + 1) as f32);
    list.quad(start, l.ruler.bottom() - 2.0, end - start, 2.0, bar);
    let glow = BoxShadow {
        blur: 3.0,
        color: s.color(StyleKey::AccentGlow),
        ..BoxShadow::default()
    };
    for (x, into_range) in [(start, 1.0), (end, -1.0)] {
        let post = Rect::new(x - 1.0, l.ruler.bottom() - BRACKET_H, 2.0, BRACKET_H);
        list.box_shadow_outset(post, CornerRadii::uniform(0.0), glow);
        list.quad(post.x, post.y, post.width, post.height, accent);
        let lip_x = if into_range > 0.0 { x - 1.0 } else { x - 5.0 };
        let mut radii = CornerRadii::uniform(0.0);
        if into_range > 0.0 {
            radii.top_right = 2.0;
        } else {
            radii.top_left = 2.0;
        }
        list.paint_quad_background(
            Rect::new(lip_x, l.ruler.bottom() - 4.0, 6.0, 4.0),
            crate::Background::Solid(accent),
            radii,
        );
    }
}

/// The current frame's column through the ruler and lanes: a lit cell, or a
/// solid line once cells are too thin to read as a column.
fn paint_frame_column(
    list: &mut DrawList,
    s: &StyleResolver,
    l: &Layout,
    frame: u32,
    column: Rect,
) {
    let x = l.x(frame as f32);
    if l.ppf >= 4.0 {
        let r = Rect::new(x, column.y, l.ppf + 1.0, column.height);
        list.quad(r.x, r.y, r.width, r.height, s.color(StyleKey::AccentRing));
        let edge = s.color(StyleKey::AccentGlow);
        list.quad(r.x, r.y, 1.0, r.height, edge);
        list.quad(r.right() - 1.0, r.y, 1.0, r.height, edge);
    } else {
        let mut c = s.color(StyleKey::Accent);
        c[3] *= 0.8;
        list.quad(x, column.y, l.ppf.max(1.0), column.height, c);
    }
}

/// The playhead's frame number, on an accent tag at the top of the ruler.
fn paint_badge(list: &mut DrawList, s: &StyleResolver, l: &Layout, frame: u32) {
    let mono = s.theme().mono_font.clone();
    let size = s.text_size(TextSize::Caption);
    let text = frame.to_string();
    let w = list
        .measure_text_with_font(&text, size, None, mono.as_ref())
        .0;
    let tag = Rect::new(
        l.xc(frame) - (w + 8.0) * 0.5,
        l.ruler.y + 3.0,
        w + 8.0,
        13.0,
    );
    list.box_shadow_outset(
        tag,
        CornerRadii::uniform(2.0),
        BoxShadow {
            offset: [0.0, 1.0],
            blur: 3.0,
            color: [0.0, 0.0, 0.0, 0.6],
            ..BoxShadow::default()
        },
    );
    list.chrome_rect(tag, 2.0, 0.0, s.color(StyleKey::Accent), [0.0; 4]);
    let ty = list.vcentered_text_y(tag.y, tag.height, size, mono.as_ref(), &text);
    list.text(
        TextBlock::new(text, tag.x + 4.0, ty)
            .with_size(size)
            .with_weight(crate::Weight::SEMIBOLD)
            .with_color_f32(s.color(StyleKey::OnAccent))
            .with_font_opt(mono),
    );
}

#[derive(Clone, Copy, PartialEq)]
enum KeyShape {
    Key(Interp),
    /// A summary row's small diamond.
    Summary,
}

/// One key at (`cx`, `cy`): a diamond (ease), circle (linear) or square
/// (hold); lit when selected, greyed when its track is locked.
#[allow(clippy::too_many_arguments)]
fn paint_key(
    list: &mut DrawList,
    s: &StyleResolver,
    chrome: &TimelineChrome,
    cx: f32,
    cy: f32,
    shape: KeyShape,
    selected: bool,
    locked: bool,
) {
    let fill = if selected {
        s.color(StyleKey::AccentTick)
    } else if locked || shape == KeyShape::Summary {
        s.ink(Ink::Caption)
    } else {
        s.ink(Ink::Cell)
    };
    let (side, radius, turn) = match shape {
        KeyShape::Summary => (5.0, 0.0, true),
        KeyShape::Key(Interp::Ease) => (7.0, 1.0, true),
        KeyShape::Key(Interp::Linear) => (8.0, 4.0, false),
        KeyShape::Key(Interp::Hold) => (7.0, 1.0, false),
    };
    let border = if shape == KeyShape::Summary {
        let mut c = chrome.key_edge;
        c[3] = c[3].min(0.75);
        c
    } else if selected {
        s.color(StyleKey::LatchShade)
    } else {
        chrome.key_edge
    };
    list.push_transform();
    list.translate(cx, cy);
    if turn {
        list.rotate(std::f32::consts::FRAC_PI_4);
    }
    let r = Rect::new(-side * 0.5, -side * 0.5, side, side);
    let radii = CornerRadii::uniform(radius);
    if selected && shape != KeyShape::Summary {
        list.box_shadow_outset(
            r,
            radii,
            BoxShadow {
                blur: 6.0,
                color: s.color(StyleKey::AccentGlow),
                ..BoxShadow::default()
            },
        );
    }
    list.chrome_rect(r, radius, 1.0, fill, border);
    if !selected && shape != KeyShape::Summary {
        list.box_shadow_inset(
            r.inset(1.0),
            CornerRadii::uniform((radius - 1.0).max(0.0)),
            BoxShadow {
                offset: [0.0, 1.0],
                color: chrome.key_highlight,
                inset: true,
                ..BoxShadow::default()
            },
        );
    }
    list.pop_transform();
}

/// A transport key with a Phosphor glyph; true when clicked.
fn icon_key(ctx: &mut DrawContext, rect: Rect, icon: PhosphorIcon, latched: bool) -> bool {
    transport_pressable(latched)
        .draw(rect, ctx, |state, ctx| {
            let s = ctx.styles();
            let ink = key_ink(&s, latched, state.hovered, state.down);
            let face = state.face;
            let side = 10.0;
            let r = Rect::new(
                (face.x + (face.width - side) * 0.5).round(),
                (face.y + (face.height - side) * 0.5).round(),
                side,
                side,
            );
            ctx.draw_list.phosphor_icon(r, icon, ink);
        })
        .clicked
}

/// A transport key with a text label; true when clicked.
fn text_key(ctx: &mut DrawContext, rect: Rect, label: &str) -> bool {
    transport_pressable(false)
        .draw(rect, ctx, |state, ctx| {
            let s = ctx.styles();
            let ink = key_ink(&s, false, state.hovered, state.down);
            let font = s.theme().font.clone();
            let list = &mut *ctx.draw_list;
            let face = state.face;
            let w = list
                .measure_text_with_font(label, TEXT_KEY_SIZE, None, font.as_ref())
                .0;
            let y = list.vcentered_text_y(face.y, face.height, TEXT_KEY_SIZE, font.as_ref(), label);
            list.text(
                TextBlock::new(label, face.x + (face.width - w) * 0.5, y)
                    .with_size(TEXT_KEY_SIZE)
                    .with_weight(crate::Weight::MEDIUM)
                    .with_color_f32(ink)
                    .with_font_opt(font),
            );
        })
        .clicked
}

fn text_key_width(list: &mut DrawList, s: &StyleResolver, label: &str) -> f32 {
    list.measure_text_with_font(label, TEXT_KEY_SIZE, None, s.theme().font.as_ref())
        .0
        + 2.0 * TEXT_KEY_PAD
}

/// A track's mute or lock key; true when clicked.
fn track_key(ctx: &mut DrawContext, rect: Rect, letter: &str, on: bool) -> bool {
    transport_pressable(on)
        .travel(1.0)
        .draw(rect, ctx, |state, ctx| {
            let s = ctx.styles();
            let ink = key_ink(&s, on, state.hovered, state.down);
            let mono = s.theme().mono_font.clone();
            let list = &mut *ctx.draw_list;
            let face = state.face;
            let w = list
                .measure_text_with_font(letter, TRACK_KEY_SIZE, None, mono.as_ref())
                .0;
            let y =
                list.vcentered_text_y(face.y, face.height, TRACK_KEY_SIZE, mono.as_ref(), letter);
            list.text(
                TextBlock::new(letter, face.x + (face.width - w) * 0.5, y)
                    .with_size(TRACK_KEY_SIZE)
                    .with_color_f32(ink)
                    .with_font_opt(mono),
            );
        })
        .clicked
}

fn transport_pressable(latched: bool) -> Pressable {
    Pressable::new()
        .tone(if latched { Tone::Latch } else { Tone::Ghost })
        .held(latched)
}

/// A key's ink: the latch ink while latched, else the ghost key's.
fn key_ink(s: &StyleResolver, latched: bool, hovered: bool, down: bool) -> [f32; 4] {
    if latched {
        latch_ink(s, hovered, down)
    } else if down {
        s.ink(Ink::Glyph)
    } else {
        s.ink(Ink::Cell)
    }
}
