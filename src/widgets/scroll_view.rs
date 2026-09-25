//! General-purpose scrollable viewport widget.
//!
//! `ScrollView` clips its content to a fixed viewport `Rect`, applies a
//! caller-owned `ScrollState` offset to the content via the existing transform
//! stack, and draws minimal scrollbars (vertical + horizontal) when the
//! content overflows. Scrollbar thumbs can be dragged; the wheel is consumed
//! when the cursor is over the viewport.
//!
//! # Scrolling is smooth by default
//!
//! Input never moves the drawn offset directly. It moves the *target*
//! ([`ScrollState::target`]) — one target step per wheel notch — and the drawn
//! offset ([`ScrollState::offset`], the value the transform and the thumb use)
//! eases toward it, so a burst of notches reads as one continuous glide instead
//! of a series of 20px snaps. `scroll_range_into_view` (keyboard/selection
//! reveal) goes through the same path.
//!
//! The easing is exponential rather than a fixed-duration tween: each frame
//! closes a constant fraction of the remaining distance
//! ([`ScrollSmoothing::settle_seconds`], 0.22s by default), which (a) makes the
//! glide's speed follow the wheel cadence instead of restarting on every event,
//! and (b) is frame-rate independent — the step is derived from the frame
//! delta, so 60 Hz and 144 Hz hosts travel the same distance in the same
//! wall-clock time. It snaps to the target once the remainder is sub-pixel, so
//! an idle scroll is bit-exactly at its target and reports no pending repaint.
//!
//! The frame delta comes from [`InputState::frame_dt`] (stamped once per frame
//! by [`UiState::begin_frame`](crate::UiState::begin_frame), or set directly by
//! a hand-rolled host); a host that never sets one gets a nominal 60 Hz frame,
//! and `0.0` — a paused or one-shot static frame — applies the target at once,
//! so a single rendered frame always shows the offset that was asked for. While
//! a glide is in flight [`ScrollState::pending_deadline`] asks the host for the
//! next frame; see `frame_result` for how that reaches an event-driven loop.
//!
//! One deliberate exception: dragging the scrollbar thumb is 1:1 (direct
//! manipulation should not lag the pointer — the drag writes offset and target
//! together).
//!
//! The glide is governed only by [`ScrollState::smoothing`]; the theme's
//! `animation_duration` (hover/press fades, `0.0` by default) does not touch
//! it. Use [`ScrollSmoothing::INSTANT`] to opt a region out.
//!
//! # Docked and overlay bars
//!
//! By default a bar is docked (Forge `ScrollArea`): a 13 px gutter the content
//! gives up, with step keys at each end that scroll by
//! [`STEP_SCROLL`](ScrollView::STEP_SCROLL) and a thumb between them.
//! [`overlay`](ScrollView::overlay) bars take no space: a 9 px thumb floats
//! 2 px inside the viewport's edge, over the content, and nothing else is
//! drawn. While the pointer is on an overlay thumb it belongs to the thumb:
//! [`begin`](ScrollView::begin) marks the mouse consumed, so the content under
//! it doesn't hover or take the click. Either thumb is at least
//! [`MIN_THUMB`](ScrollView::MIN_THUMB) long.
//!
//! State is **caller-owned** so the widget remains a transient struct that
//! can be re-built every frame, matching the rest of this crate's
//! immediate-mode style.
//!
//! ```ignore
//! let mut scroll = ScrollState::default();
//! scroll.content_size = [200.0, 800.0];
//!
//! ScrollView::new(viewport_rect)
//!     .draw(&mut scroll, list, &style, input, |list, content_origin| {
//!         // Draw your content here. The transform stack has already been
//!         // translated by `-offset`, so draw in content-local coordinates
//!         // anchored at `content_origin` (which equals the viewport's top-left
//!         // in world space).
//!     });
//! ```
//!
//! `content_origin` is the viewport's world-space top-left after the active
//! transform; it's what `(0,0)` inside the closure now maps to. Most callers
//! draw at world-space rects derived from `viewport.x + col, viewport.y + row`.

use crate::NOMINAL_FRAME_DT;
use crate::chrome::SurfacePainter;
use crate::layout::Rect;
use crate::{InputState, StyleKey, StyleResolver};

use super::DrawList;

/// How close to its target a gliding offset must get before it is snapped onto
/// it, in logical pixels. A quarter pixel is below the visible threshold at any
/// realistic DPI, and terminating the glide exactly is what lets an idle scroll
/// stop asking the host for frames.
const SNAP_EPSILON: f32 = 0.25;

/// Easing applied to a [`ScrollState`]'s drawn offset as it chases its target.
///
/// The offset approaches the target *exponentially* — each frame closes a
/// constant fraction of what remains — rather than along a fixed timeline. See
/// the module docs for why (cadence-following, retarget-safe, frame-rate
/// independent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollSmoothing {
    /// Seconds to close ~99% of the distance to the target; the `e`-folding
    /// time constant is this divided by `ln 100`. `0.0` (or non-finite)
    /// disables easing — the offset lands on the target in a single frame.
    pub settle_seconds: f32,
}

impl ScrollSmoothing {
    /// Default settle time: long enough to read as motion, short enough that
    /// the content still feels attached to the wheel.
    pub const DEFAULT_SETTLE_SECONDS: f32 = 0.22;

    /// No easing: the offset lands on the target in one frame.
    pub const INSTANT: ScrollSmoothing = ScrollSmoothing {
        settle_seconds: 0.0,
    };

    /// Smoothing with the given settle time. Non-finite or non-positive values
    /// collapse to [`INSTANT`](Self::INSTANT), so a misconfigured knob degrades
    /// to a jump rather than to a NaN offset.
    pub fn new(settle_seconds: f32) -> Self {
        if settle_seconds.is_finite() && settle_seconds > 0.0 {
            Self { settle_seconds }
        } else {
            Self::INSTANT
        }
    }

    /// Whether this configuration eases at all.
    pub fn is_instant(&self) -> bool {
        !(self.settle_seconds.is_finite() && self.settle_seconds > 0.0)
    }

    /// The easing time constant: seconds in which the offset closes `1 - 1/e`
    /// of the remaining distance. `0.0` when this configuration does not ease.
    pub fn tau(&self) -> f32 {
        if self.is_instant() {
            0.0
        } else {
            self.settle_seconds / 100.0_f32.ln()
        }
    }
}

impl Default for ScrollSmoothing {
    fn default() -> Self {
        Self {
            settle_seconds: Self::DEFAULT_SETTLE_SECONDS,
        }
    }
}

/// Caller-owned scroll state.
///
/// `offset` is the *drawn* scroll offset (positive = scrolled right/down) — the
/// value the content transform and the scrollbar thumb use. `target` is the
/// offset the user has asked for: wheel notches and
/// [`scroll_range_into_view`](Self::scroll_range_into_view) move it, and
/// `offset` eases toward it (see the module docs). `content_size` is what the
/// most recent draw reported as the natural content extent — used for clamping
/// and scrollbar sizing on subsequent frames.
///
/// `_drag_*` fields track the currently-dragged scrollbar thumb.
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    /// Current scroll offset `[x, y]` (positive = scrolled right/down) — what
    /// is drawn this frame, eased toward [`target`](Self::target).
    pub offset: [f32; 2],
    /// Requested scroll offset `[x, y]` (positive = scrolled right/down).
    ///
    /// Always clamped to `[0, max_offset]` against the live content/viewport
    /// sizes, so it is reachable and a glide toward it always terminates.
    pub target: [f32; 2],
    /// Natural content extent `[w, h]` reported by the most recent draw.
    pub content_size: [f32; 2],
    /// Easing preference for this scroll region; the only thing that decides
    /// whether [`ScrollView`] glides ([`ScrollSmoothing::INSTANT`] opts out).
    pub smoothing: ScrollSmoothing,
    /// Frame delta of the most recent [`advance`](Self::advance), used to derive
    /// [`pending_deadline`](Self::pending_deadline)'s "next frame" cadence.
    last_dt: f32,
    /// Easing time constant actually used by the most recent advance (the
    /// theme-resolved one, which may differ from `smoothing.tau()`).
    last_tau: f32,
    /// Which scrollbar is being dragged this frame (None when not dragging).
    drag_axis: Option<ScrollAxis>,
    /// Mouse position at drag start (world coords).
    drag_start_mouse: f32,
    /// Scroll offset at drag start (along drag_axis).
    drag_start_offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}

impl ScrollState {
    /// Create a fresh state scrolled to the origin.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when content overflows in the given axis (0 = X, 1 = Y).
    pub fn overflows(&self, axis: usize, viewport_size: f32) -> bool {
        self.content_size[axis] > viewport_size + 0.5
    }

    /// Maximum legal offset along axis (>= 0).
    pub fn max_offset(&self, axis: usize, viewport_size: f32) -> f32 {
        (self.content_size[axis] - viewport_size).max(0.0)
    }

    /// Clamp the drawn offset **and** the target against the latest
    /// content/viewport sizes.
    ///
    /// The target is clamped too, deliberately: an unclamped target beyond
    /// `max_offset` would leave the glide asymptotically chasing a position the
    /// offset can never legally occupy — i.e. permanently in flight, and asking
    /// the host for a frame every frame, forever.
    pub fn clamp(&mut self, viewport: [f32; 2]) {
        self.clamp_axis(0, viewport[0]);
        self.clamp_axis(1, viewport[1]);
    }

    fn clamp_axis(&mut self, axis: usize, viewport_size: f32) {
        let max = self.max_offset(axis, viewport_size);
        self.offset[axis] = self.offset[axis].clamp(0.0, max);
        self.target[axis] = self.target[axis].clamp(0.0, max);
    }

    /// Ask to scroll to `value` along `axis`; the drawn offset eases toward it
    /// over the following frames.
    ///
    /// The value is floored at `0` and bounded properly by the next
    /// [`clamp`](Self::clamp)/[`ScrollView`] pass (which knows the live
    /// viewport). Invalid axes and non-finite values are ignored.
    pub fn scroll_to(&mut self, axis: usize, value: f32) {
        if axis > 1 || !value.is_finite() {
            return;
        }
        self.target[axis] = value.max(0.0);
    }

    /// Move the target by `delta` along `axis` (positive = right/down), the way
    /// one wheel notch does. Invalid axes and non-finite deltas are ignored.
    pub fn scroll_by(&mut self, axis: usize, delta: f32) {
        if axis > 1 || !delta.is_finite() {
            return;
        }
        self.target[axis] = (self.target[axis] + delta).max(0.0);
    }

    /// Jump both the drawn offset and the target to `value` along `axis`, with
    /// no easing — for moments where there is nothing to glide from (fresh
    /// content, a "jump to top" command, a keyboard page jump).
    pub fn snap_to(&mut self, axis: usize, value: f32) {
        if axis > 1 || !value.is_finite() {
            return;
        }
        let value = value.max(0.0);
        self.offset[axis] = value;
        self.target[axis] = value;
    }

    /// Whether the drawn offset is still visibly short of the target.
    pub fn is_gliding(&self) -> bool {
        self.gap() > SNAP_EPSILON
    }

    /// Largest remaining distance to the target across both axes.
    fn gap(&self) -> f32 {
        let dx = (self.target[0] - self.offset[0]).abs();
        let dy = (self.target[1] - self.offset[1]).abs();
        dx.max(dy)
    }

    /// Seconds until this scroll region's appearance next changes: the shorter
    /// of one frame (the cadence the host is running at — a glide changes every
    /// frame) and the time left before the offset snaps onto the target. `None`
    /// when the offset is at the target.
    ///
    /// A still-moving scroll is a repaint source in its own right, but it cannot
    /// report "my settle time" the way a hover fade does — an event-driven host
    /// that only woke at the settle time would draw a single jump instead of a
    /// glide. Hand the value to
    /// [`UiState::request_repaint_after`](crate::UiState::request_repaint_after)
    /// (which `UiState::end_frame` does for `UiState::scroll`) to keep the loop
    /// awake for exactly as long as the motion lasts.
    pub fn pending_deadline(&self) -> Option<f32> {
        let gap = self.gap();
        if gap <= SNAP_EPSILON {
            return None;
        }
        let tau = if self.last_tau > 0.0 {
            self.last_tau
        } else {
            self.smoothing.tau()
        };
        if tau <= 0.0 {
            // Not easing: the target applies on the next drawn frame.
            return Some(0.0);
        }
        let cadence = if self.last_dt > 0.0 {
            self.last_dt
        } else {
            NOMINAL_FRAME_DT
        };
        Some(cadence.min(tau * (gap / SNAP_EPSILON).ln()))
    }

    /// Ease the drawn offset toward the target by one frame of `dt` seconds.
    ///
    /// `smoothing` is the *effective* configuration (the caller/widget has
    /// already folded in any theme- or caller-level "no motion" override).
    /// `dt` is sanitized here (non-finite/negative → `0.0`, huge deltas clamped
    /// to [`MAX_DT`](crate::MAX_DT)); a `dt` of `0.0` — a paused or one-shot
    /// static frame, where no time passes to animate in — applies the target at
    /// once, so a single rendered frame shows the offset that was asked for.
    pub fn advance(&mut self, smoothing: ScrollSmoothing, dt: f32) {
        let dt = crate::frame_result::sanitize_dt(dt);
        let tau = if dt > 0.0 { smoothing.tau() } else { 0.0 };
        self.last_dt = dt;
        self.last_tau = tau;
        for axis in 0..2 {
            let gap = self.target[axis] - self.offset[axis];
            if gap == 0.0 {
                continue;
            }
            if tau <= 0.0 {
                self.offset[axis] = self.target[axis];
                continue;
            }
            // Exponential step: frame-rate independent, and recomputed from the
            // live gap each frame, so a wheel notch landing mid-glide re-aims
            // the motion instead of restarting it.
            let step = 1.0 - (-dt / tau).exp();
            let next = self.offset[axis] + gap * step;
            self.offset[axis] = if (self.target[axis] - next).abs() <= SNAP_EPSILON {
                self.target[axis]
            } else {
                next
            };
        }
    }

    /// Move one axis by the minimum amount needed to reveal `[start, end]`.
    ///
    /// Visibility is judged against the **target**, not the mid-glide offset, so
    /// re-issuing the same reveal every frame is a no-op once the target already
    /// covers the range instead of nudging it forward each time. The target
    /// (and therefore the eventual offset) moves by the minimum amount, eased
    /// like any other scroll.
    ///
    /// Oversized ranges align their leading edge. Non-finite coordinates,
    /// invalid axes, and non-positive/non-finite viewport sizes are ignored.
    pub fn scroll_range_into_view(
        &mut self,
        axis: usize,
        start: f32,
        end: f32,
        viewport_size: f32,
    ) {
        if axis > 1
            || !start.is_finite()
            || !end.is_finite()
            || !viewport_size.is_finite()
            || viewport_size <= 0.0
        {
            return;
        }
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        if end - start > viewport_size || start < self.target[axis] {
            self.target[axis] = start;
        } else if end > self.target[axis] + viewport_size {
            self.target[axis] = end - viewport_size;
        }
        self.target[axis] = self.target[axis].clamp(0.0, self.max_offset(axis, viewport_size));
    }

    /// Reset offset and target to (0, 0) — immediately, not gliding.
    pub fn reset(&mut self) {
        self.offset = [0.0, 0.0];
        self.target = [0.0, 0.0];
    }
}

/// Configuration for a single ScrollView call.
#[derive(Clone, Copy, Debug)]
pub struct ScrollView {
    viewport: Rect,
    /// Width of the scrollbar (track + thumb), in pixels.
    bar_thickness: f32,
    /// Minimum thumb extent so a tiny content/viewport ratio still produces a
    /// grabbable target.
    min_thumb: f32,
    /// Pixels of scroll per wheel notch.
    wheel_speed: f32,
    /// If false, vertical scrolling is disabled (content is clipped vertically
    /// but the offset.y is forced to 0). Same for horizontal.
    enable_vertical: bool,
    enable_horizontal: bool,
    /// Bars float over the content instead of docking beside it.
    overlay: bool,
}

/// Geometry returned by [`ScrollView::begin`] and handed back to
/// [`ScrollView::end`]. Holds the inner viewport rect plus which scrollbars are
/// visible and the inner extents (so `end` can place the bars without
/// recomputing visibility).
#[derive(Debug, Clone, Copy)]
pub struct ScrollBegin {
    /// The scrollable region in world space (viewport minus any visible bars).
    pub inner: Rect,
    v_visible: bool,
    h_visible: bool,
    inner_w: f32,
    inner_h: f32,
    /// The pointer is on the vertical / horizontal overlay thumb (or dragging
    /// it); `begin` consumed the mouse for it.
    v_hot: bool,
    h_hot: bool,
    /// Debug-scope depth before `begin` opened the `ScrollView` scope, so `end`
    /// can close back to it even if the caller's content closure leaked a scope.
    scope_depth: usize,
}

impl ScrollView {
    /// The shortest a thumb gets, in pixels.
    pub const MIN_THUMB: f32 = 22.0;
    /// How far one click on a docked bar's step key scrolls, in pixels.
    pub const STEP_SCROLL: f32 = 34.0;
    /// Thickness of an overlay bar's thumb.
    pub const OVERLAY_THICKNESS: f32 = 9.0;
    /// Gap between an overlay thumb and the viewport's edges.
    pub const OVERLAY_INSET: f32 = 2.0;

    /// Create a ScrollView over `viewport` with default bar/wheel settings.
    pub fn new(viewport: Rect) -> Self {
        Self {
            viewport,
            // The docked 4a scrollbar is a 13px control: two 13px stepper
            // keys bookend a sunken track and a face-plate scrubber.
            bar_thickness: 13.0,
            min_thumb: Self::MIN_THUMB,
            wheel_speed: 20.0,
            enable_vertical: true,
            enable_horizontal: true,
            overlay: false,
        }
    }

    /// Float the bars over the content (see the module docs): the content
    /// keeps the whole viewport, and only a thin thumb is drawn.
    pub fn overlay(mut self) -> Self {
        self.overlay = true;
        self.bar_thickness = Self::OVERLAY_THICKNESS;
        self
    }

    /// Space a visible bar takes from the content: its thickness when
    /// docked, none as an overlay.
    fn reserve(&self) -> f32 {
        if self.overlay {
            0.0
        } else {
            self.bar_thickness
        }
    }

    /// Where a thumb runs along an axis `inner` px long: its start from the
    /// viewport's edge, and the length it travels in. Docked, the step keys
    /// take a bar's thickness at each end; an overlay keeps its inset.
    fn track(&self, inner: f32) -> (f32, f32) {
        let end = if self.overlay {
            Self::OVERLAY_INSET.min(inner * 0.5)
        } else {
            self.bar_thickness.min(inner * 0.5)
        };
        (end, (inner - 2.0 * end).max(0.0))
    }

    /// The thumb's thickness and its gap from the viewport edge across the
    /// bar: the design's docked thumb sits 2 px inside its gutter.
    fn thumb_across(&self) -> (f32, f32) {
        if self.overlay {
            (self.bar_thickness, Self::OVERLAY_INSET)
        } else {
            ((self.bar_thickness - 4.0).max(1.0), 2.0)
        }
    }

    /// The vertical thumb for `state` in a viewport whose content area is
    /// `inner_h` tall.
    fn v_thumb(&self, state: &ScrollState, inner_h: f32) -> Rect {
        let (start, travel) = self.track(inner_h);
        let (thick, gap) = self.thumb_across();
        let len = thumb_extent(travel, inner_h, state.content_size[1], self.min_thumb);
        let max_off = state.max_offset(1, inner_h).max(1e-6);
        let t = (state.offset[1] / max_off).clamp(0.0, 1.0);
        Rect::new(
            self.viewport.x + self.viewport.width - gap - thick,
            self.viewport.y + start + (travel - len) * t,
            thick,
            len,
        )
    }

    /// The horizontal thumb for `state` in a viewport whose content area is
    /// `inner_w` wide.
    fn h_thumb(&self, state: &ScrollState, inner_w: f32) -> Rect {
        let (start, travel) = self.track(inner_w);
        let (thick, gap) = self.thumb_across();
        let len = thumb_extent(travel, inner_w, state.content_size[0], self.min_thumb);
        let max_off = state.max_offset(0, inner_w).max(1e-6);
        let t = (state.offset[0] / max_off).clamp(0.0, 1.0);
        Rect::new(
            self.viewport.x + start + (travel - len) * t,
            self.viewport.y + self.viewport.height - gap - thick,
            len,
            thick,
        )
    }

    /// Set the scrollbar thickness in pixels.
    pub fn with_bar_thickness(mut self, t: f32) -> Self {
        self.bar_thickness = t;
        self
    }

    /// Set the pixels scrolled per wheel notch.
    pub fn with_wheel_speed(mut self, s: f32) -> Self {
        self.wheel_speed = s;
        self
    }

    /// Disable horizontal scrolling (vertical only).
    pub fn vertical_only(mut self) -> Self {
        self.enable_horizontal = false;
        self
    }

    /// Disable vertical scrolling (horizontal only).
    pub fn horizontal_only(mut self) -> Self {
        self.enable_vertical = false;
        self
    }

    /// Draw the ScrollView and run `content` to populate it.
    ///
    /// The closure is called with the `DrawList` already translated by the
    /// negative scroll offset and clipped to the viewport. The closure receives
    /// the world-space `Rect` representing the *scrollable region* in
    /// content-local coordinates — i.e. its `x`/`y` are `viewport.x/y` and its
    /// `width`/`height` are the viewport size; widgets inside should draw at
    /// rects starting at `(viewport.x, viewport.y)` and any content beyond
    /// the viewport extents is clipped/scrolled automatically.
    ///
    /// `state.content_size` must be set by the caller *before* calling `draw`
    /// (the ScrollView cannot know how tall arbitrary content is until it has
    /// been measured). A common pattern is to compute it once based on item
    /// counts, then pass it in.
    ///
    /// The drawn offset is eased toward `state.target` using the frame delta in
    /// [`InputState::frame_dt`]; see the module docs.
    ///
    /// This is a thin wrapper over [`begin`](Self::begin) + [`end`](Self::end)
    /// for callers that draw their content in a Rust closure. Immediate-mode
    /// callers (e.g. a scripting binding) can use `begin`/`end` directly.
    pub fn draw<F>(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        mut content: F,
    ) where
        F: FnMut(&mut DrawList, Rect),
    {
        let begun = self.begin(state, list, input);
        content(list, begun.inner);
        self.end(state, list, style, input, begun);
    }

    /// Begin a scroll region: handle wheel + thumb-drag input, ease the drawn
    /// offset toward the target, push the clip and the `-offset` transform, and
    /// return the viewport geometry. The caller then draws content (in
    /// world-space pre-offset coords anchored at `ScrollBegin::inner`) and
    /// **must** call [`end`](Self::end) with the returned value to pop the
    /// clip/transform and draw the scrollbars.
    ///
    /// `state.content_size` must be set before calling (see [`draw`](Self::draw)).
    pub fn begin(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        input: &mut InputState,
    ) -> ScrollBegin {
        // Force-disable axes where content fits.
        if !self.enable_vertical {
            state.snap_to(1, 0.0);
        }
        if !self.enable_horizontal {
            state.snap_to(0, 0.0);
        }

        // Reserve space for visible scrollbars so content doesn't slide under them.
        let v_visible = self.enable_vertical && state.overflows(1, self.viewport.height);
        let h_visible = self.enable_horizontal && state.overflows(0, self.viewport.width);
        let inner_w = self.viewport.width - if v_visible { self.reserve() } else { 0.0 };
        let inner_h = self.viewport.height - if h_visible { self.reserve() } else { 0.0 };
        let inner = Rect::new(self.viewport.x, self.viewport.y, inner_w, inner_h);

        // Re-clamp offset *and* target against the inner viewport (now that we
        // know which bars take space) — before anything reads them, so an eased
        // glide always chases a reachable position.
        state.clamp([inner_w, inner_h]);

        let mouse_over_inner =
            inner.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;

        // Wheel input moves the *target*, not the drawn offset: the offset
        // catches up in `advance` below, which turns a burst of notches into one
        // glide. Consumed when over the inner viewport, regardless of whether the
        // target actually changed (e.g. at a clamp boundary the wheel is still
        // claimed so it doesn't bubble to an outer scrollable).
        if mouse_over_inner
            && input.scroll_delta != 0.0
            && !input.scroll_consumed
            && self.enable_vertical
        {
            state.target[1] = (state.target[1] - input.scroll_delta * self.wheel_speed)
                .clamp(0.0, state.max_offset(1, inner_h));
            input.scroll_consumed = true;
            input.scroll_delta = 0.0;
        }

        // Handle thumb drag for both axes. Dragging is direct manipulation: it
        // sets the drawn offset and the target together (no easing), so the
        // thumb tracks the pointer 1:1 and any glide still in flight is
        // cancelled by the grab.
        if let Some(axis) = state.drag_axis {
            if !input.mouse_down {
                state.drag_axis = None;
            } else {
                match axis {
                    ScrollAxis::Vertical => {
                        let (_, track_h) = self.track(inner_h);
                        let thumb_h =
                            thumb_extent(track_h, inner_h, state.content_size[1], self.min_thumb);
                        let drag_range = (track_h - thumb_h).max(1.0);
                        let max_off = state.max_offset(1, inner_h);
                        let dy = input.mouse_y - state.drag_start_mouse;
                        let dragged = (state.drag_start_offset + dy * (max_off / drag_range))
                            .clamp(0.0, max_off);
                        state.offset[1] = dragged;
                        state.target[1] = dragged;
                    }
                    ScrollAxis::Horizontal => {
                        let (_, track_w) = self.track(inner_w);
                        let thumb_w =
                            thumb_extent(track_w, inner_w, state.content_size[0], self.min_thumb);
                        let drag_range = (track_w - thumb_w).max(1.0);
                        let max_off = state.max_offset(0, inner_w);
                        let dx = input.mouse_x - state.drag_start_mouse;
                        let dragged = (state.drag_start_offset + dx * (max_off / drag_range))
                            .clamp(0.0, max_off);
                        state.offset[0] = dragged;
                        state.target[0] = dragged;
                    }
                }
            }
        }

        // Ease the drawn offset the rest of the way toward its target with this
        // frame's clock, before the transform below reads it.
        state.advance(state.smoothing, input.frame_dt);

        // An overlay thumb lies over the content: while the pointer is on it
        // (or dragging it) it takes the mouse, so the content drawn next
        // neither hovers nor takes the click.
        let pointer_on =
            |thumb: Rect| !input.mouse_consumed && thumb.contains(input.mouse_x, input.mouse_y);
        let v_hot = self.overlay
            && v_visible
            && (state.drag_axis == Some(ScrollAxis::Vertical)
                || pointer_on(self.v_thumb(state, inner_h)));
        let h_hot = self.overlay
            && h_visible
            && (state.drag_axis == Some(ScrollAxis::Horizontal)
                || pointer_on(self.h_thumb(state, inner_w)));
        if v_hot || h_hot {
            input.mouse_consumed = true;
        }

        // Opened *before* the clip and transform below, deliberately: the scope
        // records the clip in force at push time and applies the active
        // transform to its declared rect, so pushing it after would declare the
        // viewport shifted by -offset and record the inner clip as its own —
        // which would then wrongly excuse a genuinely mislaid scrollbar.
        let scope_depth = list.debug_scope_depth();
        list.push_debug_scope_rect("ScrollView", self.viewport);

        // Set up clip + transform for the content the caller is about to draw.
        // A viewport, not a boundary: rows either side of `inner` are supposed
        // to be clipped away, so the debug report must not call that a defect.
        list.push_clip_viewport(inner);
        list.push_transform();
        list.translate(-state.offset[0], -state.offset[1]);

        ScrollBegin {
            inner,
            v_visible,
            h_visible,
            inner_w,
            inner_h,
            v_hot,
            h_hot,
            scope_depth,
        }
    }

    /// Finish a scroll region opened by [`begin`](Self::begin): pop the
    /// transform + clip and draw the scrollbars.
    pub fn end(
        &self,
        state: &mut ScrollState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &InputState,
        begun: ScrollBegin,
    ) {
        list.pop_transform();
        list.pop_clip();

        // Draw scrollbars.
        if begun.v_visible {
            self.draw_bar(ScrollAxis::Vertical, state, list, style, input, &begun);
        }
        if begun.h_visible {
            self.draw_bar(ScrollAxis::Horizontal, state, list, style, input, &begun);
        }

        // Fill the bottom-right corner gap when both docked scrollbars are
        // visible so the content underneath doesn't show through.
        if begun.v_visible && begun.h_visible && !self.overlay {
            let corner = Rect::new(
                self.viewport.x + self.viewport.width - self.bar_thickness,
                self.viewport.y + self.viewport.height - self.bar_thickness,
                self.bar_thickness,
                self.bar_thickness,
            );
            list.quad(
                corner.x,
                corner.y,
                corner.width,
                corner.height,
                style.color(StyleKey::InputBackground),
            );
        }

        // `truncate_debug_scopes` rather than a bare pop: arbitrary caller code
        // ran between `begin` and here, and a closure that leaked a scope would
        // otherwise make this pop close the wrong one.
        list.truncate_debug_scopes(begun.scope_depth);
    }

    /// Draw one axis's bar: the docked gutter, step keys and thumb, or an
    /// overlay's thumb alone. Starts a thumb drag or steps on a click.
    fn draw_bar(
        &self,
        axis: ScrollAxis,
        state: &mut ScrollState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &InputState,
        begun: &ScrollBegin,
    ) {
        let vertical = axis == ScrollAxis::Vertical;
        let thumb = if vertical {
            self.v_thumb(state, begun.inner_h)
        } else {
            self.h_thumb(state, begun.inner_w)
        };
        let pointer = |r: Rect| !input.mouse_consumed && r.contains(input.mouse_x, input.mouse_y);
        let hovered = if self.overlay {
            if vertical { begun.v_hot } else { begun.h_hot }
        } else {
            pointer(thumb)
        };

        if !self.overlay {
            let t = self.bar_thickness;
            let track = if vertical {
                Rect::new(
                    self.viewport.x + self.viewport.width - t,
                    self.viewport.y,
                    t,
                    begun.inner_h,
                )
            } else {
                Rect::new(
                    self.viewport.x,
                    self.viewport.y + self.viewport.height - t,
                    begun.inner_w,
                    t,
                )
            };
            // Sunken channel bookended by two step keys.
            list.chrome_rect(
                track,
                0.0,
                1.0,
                style.color(StyleKey::InputBackground),
                style.color(StyleKey::PanelBorder),
            );
            super::material::draw_inset_shadow(
                list,
                style,
                track,
                style.scalar(StyleKey::InnerShadowDepth),
                1.0,
            );
            let (step, _) = self.track(if vertical { track.height } else { track.width });
            let (back, forward) = if vertical {
                (
                    Rect::new(track.x, track.y, t, step),
                    Rect::new(track.x, track.y + track.height - step, t, step),
                )
            } else {
                (
                    Rect::new(track.x, track.y, step, t),
                    Rect::new(track.x + track.width - step, track.y, step, t),
                )
            };
            let carets = if vertical {
                [StepperDirection::Up, StepperDirection::Down]
            } else {
                [StepperDirection::Left, StepperDirection::Right]
            };
            for ((key, caret), sign) in [back, forward].into_iter().zip(carets).zip([-1.0, 1.0]) {
                let over = pointer(key);
                let m = super::material::Material::new(super::material::Tone::Default)
                    .hovered(over)
                    .pressed(over && input.mouse_down);
                super::material::draw(list, style, key, &m);
                draw_stepper_caret(list, style, key, caret);
                if over && input.mouse_clicked && state.drag_axis.is_none() {
                    state.scroll_by(if vertical { 1 } else { 0 }, sign * Self::STEP_SCROLL);
                }
            }
        }

        let active = state.drag_axis == Some(axis);
        let face = if active {
            2
        } else if hovered {
            1
        } else {
            0
        };
        let chrome = style.scrollbar();
        let padding = thumb.inset(chrome.thumb[face].border_widths.left);
        let mut painter = SurfacePainter::new(
            list,
            thumb,
            padding,
            chrome.thumb[face].corner_radii,
            chrome.thumb[face],
            &chrome.thumb_insets[face],
            &[],
        );
        painter.paint_pre_content();
        painter.paint_post_content();

        if hovered && input.mouse_clicked && state.drag_axis.is_none() {
            state.drag_axis = Some(axis);
            state.drag_start_mouse = if vertical {
                input.mouse_y
            } else {
                input.mouse_x
            };
            state.drag_start_offset = state.offset[if vertical { 1 } else { 0 }];
        }
    }
}

/// Direction for a scrollbar stepper caret.
#[derive(Clone, Copy)]
enum StepperDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Paint the reference design's compact 6px triangular arrow in a stepper key.
/// The glyph is geometry rather than font text, so it remains centered and crisp
/// in the 13px square at every font configuration.
fn draw_stepper_caret(
    list: &mut DrawList,
    style: &StyleResolver,
    rect: Rect,
    direction: StepperDirection,
) {
    let cx = rect.x + rect.width * 0.5;
    let cy = rect.y + rect.height * 0.5;
    let half = (rect.width.min(rect.height) * 0.23).min(3.0);
    let color = style.color(StyleKey::TextDim);
    match direction {
        StepperDirection::Up => list.triangle(
            (cx, cy - half),
            (cx - half, cy + half),
            (cx + half, cy + half),
            color,
        ),
        StepperDirection::Down => list.triangle(
            (cx - half, cy - half),
            (cx + half, cy - half),
            (cx, cy + half),
            color,
        ),
        StepperDirection::Left => list.triangle(
            (cx - half, cy),
            (cx + half, cy - half),
            (cx + half, cy + half),
            color,
        ),
        StepperDirection::Right => list.triangle(
            (cx - half, cy - half),
            (cx + half, cy),
            (cx - half, cy + half),
            color,
        ),
    }
}

/// A thumb's length on a `track` px track when `visible` px of `content`
/// px show: the visible fraction of the track, at least `min_thumb` (and never
/// longer than the track).
fn thumb_extent(track: f32, visible: f32, content: f32, min_thumb: f32) -> f32 {
    if content <= 0.0 {
        return track;
    }
    let ratio = (visible / content).clamp(0.0, 1.0);
    (track * ratio).max(min_thumb).min(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    /// One frame of a 100×100 vertical-only view over 400 px of content;
    /// returns the content rect and the input after `begin`.
    fn tall_frame(
        view: ScrollView,
        state: &mut ScrollState,
        input: InputState,
    ) -> (Rect, InputState) {
        let th = theme();
        let style = StyleResolver::new(&th);
        let mut list = DrawList::new();
        let mut input = input;
        state.content_size = [100.0, 400.0];
        let view = view.vertical_only();
        let begun = view.begin(state, &mut list, &mut input);
        let inner = begun.inner;
        let after_begin = input.clone();
        view.end(state, &mut list, &style, &input, begun);
        (inner, after_begin)
    }

    const VIEW: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };

    #[test]
    fn overlay_bars_take_no_space_and_float_inside_the_edge() {
        let mut state = ScrollState::default();
        let view = ScrollView::new(VIEW).overlay();
        let (inner, _) = tall_frame(view, &mut state, input_at(-1.0, -1.0));
        assert_eq!(inner, VIEW, "the content keeps the whole viewport");
        let thumb = view.v_thumb(&state, 100.0);
        // 9 px wide, 2 px in from the right and top; 96 px of travel shows a
        // quarter of the content.
        assert_eq!((thumb.x, thumb.y, thumb.width), (89.0, 2.0, 9.0));
        assert!((thumb.height - 24.0).abs() < 1e-3);

        let (inner, _) = tall_frame(ScrollView::new(VIEW), &mut state, input_at(-1.0, -1.0));
        assert_eq!(inner.width, 87.0, "a docked bar keeps its 13 px gutter");
    }

    #[test]
    fn an_overlay_thumb_takes_the_pointer_and_starts_a_drag() {
        let mut state = ScrollState::default();
        let view = ScrollView::new(VIEW).overlay();
        let press = InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..input_at(93.0, 10.0)
        };
        let (_, after_begin) = tall_frame(view, &mut state, press);
        assert!(
            after_begin.mouse_consumed,
            "content under the thumb must not react"
        );
        assert_eq!(state.drag_axis, Some(ScrollAxis::Vertical));

        // Beside the thumb the content keeps the pointer.
        let mut state = ScrollState::default();
        let (_, after_begin) = tall_frame(view, &mut state, input_at(50.0, 10.0));
        assert!(!after_begin.mouse_consumed);
    }

    #[test]
    fn docked_step_keys_scroll_by_a_step() {
        let mut state = ScrollState::default();
        let click = |x, y| InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..input_at(x, y)
        };
        // The bottom step key fills the gutter's last 13 px.
        tall_frame(ScrollView::new(VIEW), &mut state, click(94.0, 94.0));
        assert_eq!(state.target[1], ScrollView::STEP_SCROLL);
        tall_frame(ScrollView::new(VIEW), &mut state, click(94.0, 5.0));
        assert_eq!(state.target[1], 0.0);
        assert_eq!(state.drag_axis, None, "a step key is not the thumb");
    }

    #[test]
    fn the_thumb_face_follows_hover_and_drag() {
        let th = theme();
        let faces = th.chrome.scrollbar.thumb;
        let face_of = |state: &mut ScrollState, input: InputState| {
            let style = StyleResolver::new(&th);
            let mut list = DrawList::new();
            let mut input = input;
            state.content_size = [100.0, 400.0];
            let view = ScrollView::new(VIEW).overlay().vertical_only();
            let begun = view.begin(state, &mut list, &mut input);
            view.end(state, &mut list, &style, &input, begun);
            // The face is the last filled instance (the border follows it).
            list.chrome_instances()
                .filter(|c| c.bg[3] > 0.0)
                .last()
                .map(|c| c.bg)
                .unwrap()
        };
        let start = |q: crate::QuadStyle| match q.background {
            crate::Background::LinearGradient { start, .. } => start,
            crate::Background::Solid(c) => c,
        };
        let mut state = ScrollState::default();
        assert_eq!(face_of(&mut state, input_at(-1.0, -1.0)), start(faces[0]));
        assert_eq!(face_of(&mut state, input_at(93.0, 10.0)), start(faces[1]));
        let press = InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..input_at(93.0, 10.0)
        };
        face_of(&mut state, press);
        let held = InputState {
            mouse_down: true,
            ..input_at(93.0, 30.0)
        };
        assert_eq!(face_of(&mut state, held), start(faces[2]));
    }

    #[test]
    fn offset_is_clamped_to_max() {
        let mut s = ScrollState {
            offset: [9999.0, -5.0],
            content_size: [100.0, 200.0],
            ..ScrollState::default()
        };
        s.clamp([50.0, 80.0]);
        // X: clamp(9999, 0, 100-50=50) = 50
        // Y: clamp(-5, 0, 200-80=120) = 0
        assert_eq!(s.offset, [50.0, 0.0]);
    }

    #[test]
    fn no_overflow_when_content_fits() {
        let s = ScrollState {
            offset: [0.0, 0.0],
            content_size: [50.0, 80.0],
            ..ScrollState::default()
        };
        assert!(!s.overflows(0, 100.0));
        assert!(!s.overflows(1, 100.0));
    }

    #[test]
    fn scroll_range_into_view_moves_only_when_needed() {
        // The reveal target is what is seeded and asserted: visibility is judged
        // against the *target* (the offset is only mid-glide toward it), so a
        // reveal that repeats every frame must be a no-op rather than nudging
        // the scroll forward each time.
        let mut s = ScrollState {
            content_size: [100.0, 500.0],
            ..ScrollState::default()
        };
        s.snap_to(1, 100.0);

        s.scroll_range_into_view(1, 120.0, 140.0, 100.0);
        assert_eq!(s.target[1], 100.0, "an already-visible range must not move");

        s.scroll_range_into_view(1, 40.0, 60.0, 100.0);
        assert_eq!(s.target[1], 40.0, "a range above aligns its leading edge");

        s.scroll_range_into_view(1, 180.0, 200.0, 100.0);
        assert_eq!(
            s.target[1], 100.0,
            "a range below moves by the minimum amount"
        );

        // ... and the drawn offset follows the target over the next frames
        // instead of jumping there with it: aiming above the offset leaves the
        // offset partway on the following frame.
        s.scroll_range_into_view(1, 0.0, 40.0, 100.0);
        assert_eq!(s.target[1], 0.0, "a range above moves the target up");
        assert_eq!(s.offset[1], 100.0, "the drawn offset has not moved yet");
        s.advance(ScrollSmoothing::default(), 1.0 / 60.0);
        assert!(
            s.offset[1] > 0.0 && s.offset[1] < 100.0,
            "offset {} should be easing toward 0",
            s.offset[1]
        );
    }

    #[test]
    fn scroll_range_into_view_clamps_and_handles_oversized_ranges() {
        let mut s = ScrollState {
            content_size: [100.0, 500.0],
            ..ScrollState::default()
        };
        s.snap_to(1, 300.0);

        s.scroll_range_into_view(1, -20.0, 10.0, 100.0);
        assert_eq!(s.target[1], 0.0);

        s.scroll_range_into_view(1, 480.0, 500.0, 100.0);
        assert_eq!(s.target[1], 400.0);

        s.scroll_range_into_view(1, 150.0, 300.0, 100.0);
        assert_eq!(
            s.target[1], 150.0,
            "oversized ranges align their leading edge"
        );
    }

    #[test]
    fn clamp_bounds_the_target_as_well_as_the_offset() {
        // Content shrinks under a pending glide: both fields come back into
        // range, so the glide lands instead of asymptotically chasing a
        // position the offset can never occupy (which would keep the region
        // permanently "in flight" and asking the host for frames).
        let mut s = ScrollState {
            content_size: [100.0, 500.0],
            ..ScrollState::default()
        };
        s.snap_to(1, 400.0);
        s.scroll_to(1, 500.0);
        assert!(s.is_gliding());

        s.content_size = [100.0, 200.0];
        s.clamp([100.0, 100.0]);
        assert_eq!(s.target[1], 100.0, "target pulled back to the new maximum");
        assert_eq!(s.offset[1], 100.0, "offset pulled back with it");

        s.advance(ScrollSmoothing::default(), 1.0 / 60.0);
        assert!(!s.is_gliding());
        assert_eq!(s.pending_deadline(), None);
    }

    #[test]
    fn overflow_when_content_exceeds_viewport() {
        let s = ScrollState {
            content_size: [100.0, 800.0],
            ..ScrollState::default()
        };
        assert!(s.overflows(1, 200.0));
        assert!(!s.overflows(0, 200.0));
    }

    #[test]
    fn max_offset_zero_when_content_fits() {
        let s = ScrollState {
            content_size: [50.0, 80.0],
            ..ScrollState::default()
        };
        assert_eq!(s.max_offset(0, 100.0), 0.0);
        assert_eq!(s.max_offset(1, 100.0), 0.0);
    }

    #[test]
    fn wheel_input_updates_vertical_offset() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0; // wheel down

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_l, _r| {},
        );
        assert!(state.offset[1] > 0.0, "wheel down should scroll content");
    }

    #[test]
    fn wheel_does_not_scroll_outside_viewport() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(500.0, 500.0); // outside
        input.scroll_delta = -3.0;

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert_eq!(state.offset[1], 0.0);
    }

    #[test]
    fn scrollbar_hidden_when_content_smaller_than_viewport() {
        let mut state = ScrollState {
            content_size: [50.0, 50.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(0.0, 0.0);

        // Track approximate vertex count: a hidden bar means no extra rounded
        // rect geometry beyond what the (empty) content closure adds.
        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert!(list.vertices.is_empty());
    }

    #[test]
    fn docked_steppers_paint_compact_caret_triangles() {
        let mut state = ScrollState {
            content_size: [100.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(-1.0, -1.0);
        ScrollView::new(Rect::new(0.0, 0.0, 100.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        // Two triangles per visible vertical scrollbar; their 3 vertices live
        // in the soup buffer, independent of chrome instance batching.
        assert!(
            list.vertices.len() >= 6,
            "stepper caret triangles are emitted"
        );
    }

    #[test]
    fn thumb_extent_proportional_to_visible_fraction() {
        // 200px viewport over 1000px content -> 20% -> 40px thumb (above min).
        assert!((thumb_extent(200.0, 200.0, 1000.0, 16.0) - 40.0).abs() < 1e-3);
        // The fraction is of the viewport, applied to the track: step keys
        // shorten the track (200 - 2 × 13), not the share shown.
        assert!((thumb_extent(174.0, 200.0, 1000.0, 16.0) - 34.8).abs() < 1e-3);
        // Tiny content fraction clamped to min_thumb.
        assert!((thumb_extent(200.0, 200.0, 100000.0, 16.0) - 16.0).abs() < 1e-3);
        // Content fits — thumb spans entire track.
        assert!((thumb_extent(200.0, 200.0, 0.0, 16.0) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn thumb_drag_updates_offset() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let viewport = Rect::new(0.0, 0.0, 200.0, 200.0);

        // The default docked bar is 13px, so its vertical channel is x=187..200.
        // Stepper keys consume y=0..13 and y=187..200; the initial scrubber
        // starts at y=13. Click inside it to start a drag.
        let mut input = InputState {
            mouse_x: 193.0,
            mouse_y: 20.0,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        };
        ScrollView::new(viewport).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert!(state.drag_axis.is_some());

        // Now drag down by 80 pixels with mouse held.
        list.clear();
        input.mouse_clicked = false;
        input.mouse_y = 100.0;
        ScrollView::new(viewport).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );

        // The content is exactly as wide as the viewport, so there is no
        // horizontal bar and the view is 200px tall. The step keys leave a
        // 174px channel; showing 200 of 1000px gives a 34.8px thumb, so 80px
        // of thumb travel (of 139.2) maps to 80 × 800 / 139.2 ≈ 459.8px.
        assert!(
            (state.offset[1] - 459.77).abs() < 1.0,
            "offset {} expected ~460",
            state.offset[1]
        );
    }

    #[test]
    fn drag_releases_on_mouse_up() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            drag_axis: Some(ScrollAxis::Vertical),
            drag_start_mouse: 0.0,
            drag_start_offset: 0.0,
            ..ScrollState::default()
        };
        state.snap_to(1, 100.0);
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = InputState {
            mouse_down: false,
            ..InputState::default()
        };
        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert!(state.drag_axis.is_none());
    }

    #[test]
    fn consumed_input_blocks_wheel() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        input.mouse_consumed = true;

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert_eq!(state.offset[1], 0.0);
    }

    #[test]
    fn wheel_marks_scroll_consumed_when_applied() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert!(input.scroll_consumed);
        assert_eq!(input.scroll_delta, 0.0);
    }

    #[test]
    fn outer_scroll_skipped_when_inner_consumes() {
        // Two ScrollViews. Cursor sits over both. Inner runs first and absorbs
        // the wheel; outer should see scroll_delta = 0 / scroll_consumed = true
        // and not move.
        let mut inner_state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut outer_state = ScrollState {
            content_size: [400.0, 4000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;

        // Inner viewport is fully inside outer.
        ScrollView::new(Rect::new(0.0, 0.0, 100.0, 100.0)).draw(
            &mut inner_state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert!(inner_state.offset[1] > 0.0, "inner should have scrolled");

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut outer_state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert_eq!(
            outer_state.offset[1], 0.0,
            "outer must not steal scroll when inner consumed it"
        );
    }

    #[test]
    fn outer_scrolls_when_inner_not_under_cursor() {
        // Inner is in a different region than the cursor — its draw shouldn't
        // claim the wheel, leaving the outer free to scroll.
        let mut inner_state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        let mut outer_state = ScrollState {
            content_size: [400.0, 4000.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(150.0, 150.0);
        input.scroll_delta = -3.0;

        // Inner viewport at (0,0..50,50) — cursor (150,150) is outside it.
        ScrollView::new(Rect::new(0.0, 0.0, 50.0, 50.0)).draw(
            &mut inner_state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert_eq!(inner_state.offset[1], 0.0);
        assert!(!input.scroll_consumed);

        ScrollView::new(Rect::new(100.0, 100.0, 200.0, 200.0)).draw(
            &mut outer_state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );
        assert!(outer_state.offset[1] > 0.0, "outer should have scrolled");
    }

    #[test]
    fn corner_quad_drawn_when_both_axes_visible() {
        let mut state = ScrollState {
            content_size: [800.0, 800.0],
            ..ScrollState::default()
        };
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(-10.0, -10.0);

        let viewport = Rect::new(0.0, 0.0, 100.0, 100.0);
        ScrollView::new(viewport).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );

        // The corner quad sits at the 13px docked scrollbar intersection.
        let bar = 13.0_f32;
        let cx = viewport.x + viewport.width - bar;
        let cy = viewport.y + viewport.height - bar;
        let found = list
            .chrome_instances()
            .any(|i| (i.rect[0] - cx).abs() < 1e-3 && (i.rect[1] - cy).abs() < 1e-3);
        assert!(found, "expected a quad at corner ({}, {})", cx, cy);
    }

    #[test]
    fn content_translated_by_negative_offset() {
        let mut state = ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        };
        // `snap_to`, not a bare `offset` write: the drawn offset eases toward
        // the target, so a pre-scrolled state must be pre-scrolled in both.
        state.snap_to(1, 50.0);
        let mut list = DrawList::new();
        let theme = theme();
        let mut input = input_at(-10.0, -10.0);

        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |l, _vp| {
                // Quad at (0, 100) — should appear in world at (0, 50) due to
                // -50 vertical scroll. Translate-only, so it records a chrome
                // instance with the translated rect.
                l.quad(0.0, 100.0, 10.0, 10.0, [1.0; 4]);
            },
        );
        let found = list
            .chrome_instances()
            .any(|i| i.rect == [0.0, 50.0, 10.0, 10.0]);
        assert!(found, "content quad should be translated to world (0, 50)");
    }

    // ---- smooth scrolling -------------------------------------------------

    /// Draw one frame of the standard 200x200 scroll view (content 1000px tall,
    /// so the vertical bar is visible and the inner viewport is 187x200).
    ///
    /// Ends with [`InputState::end_frame`], the way a host does: per-frame edges
    /// (the wheel delta and `scroll_consumed`) last exactly one frame, so a test
    /// that wheels twice must do so on two frames.
    fn draw_frame(state: &mut ScrollState, input: &mut InputState, theme: &Theme) {
        let mut list = DrawList::new();
        ScrollView::new(Rect::new(0.0, 0.0, 200.0, 200.0)).draw(
            state,
            &mut list,
            &StyleResolver::new(theme),
            input,
            |_, _| {},
        );
        input.end_frame();
    }

    fn scroll_state() -> ScrollState {
        ScrollState {
            content_size: [200.0, 1000.0],
            ..ScrollState::default()
        }
    }

    #[test]
    fn wheel_sets_the_target_and_eases_the_offset_toward_it() {
        let mut state = scroll_state();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0; // wheel down = 3 * 20px

        draw_frame(&mut state, &mut input, &theme);

        // Default 60Hz clock: the target takes the whole notch, the drawn offset
        // only part of it — which is exactly what makes the motion smooth.
        assert_eq!(state.target[1], 60.0, "the notch moves the target in full");
        assert!(
            state.offset[1] > 0.0 && state.offset[1] < state.target[1],
            "offset {} should be partway to 60",
            state.offset[1]
        );
        assert!(state.is_gliding());
        let deadline = state.pending_deadline().expect("a glide is in flight");
        assert!(
            deadline > 0.0 && deadline <= 1.0 / 60.0,
            "a glide asks for the next frame, got {deadline}"
        );
    }

    #[test]
    fn a_glide_converges_and_snaps_onto_the_target() {
        let mut state = scroll_state();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        draw_frame(&mut state, &mut input, &theme);

        // Let the glide run on a 60Hz clock; it must land bit-exactly, not
        // asymptotically — that is what lets an idle UI report no repaint.
        for _ in 0..120 {
            input.scroll_delta = 0.0;
            draw_frame(&mut state, &mut input, &theme);
        }
        assert_eq!(state.offset[1], state.target[1]);
        assert_eq!(state.offset[1], 60.0);
        assert!(!state.is_gliding());
        assert_eq!(state.pending_deadline(), None);
    }

    #[test]
    fn easing_is_frame_rate_independent() {
        // The step is derived from the frame delta, so one 100ms frame and six
        // 16.7ms frames must land in the same place. (A per-frame fraction —
        // `offset += gap * 0.25` — would not, and would scroll two and a half
        // times faster on a 144Hz display than on a 60Hz one.)
        let theme = theme();
        let mut coarse = scroll_state();
        let mut fine = scroll_state();
        coarse.scroll_to(1, 400.0);
        fine.scroll_to(1, 400.0);

        let mut input = input_at(50.0, 50.0);
        input.frame_dt = 0.1;
        draw_frame(&mut coarse, &mut input, &theme);

        input.frame_dt = 0.1 / 6.0;
        for _ in 0..6 {
            draw_frame(&mut fine, &mut input, &theme);
        }

        assert!(
            (coarse.offset[1] - fine.offset[1]).abs() < 0.5,
            "{} vs {} — the step must depend on elapsed time, not frame count",
            coarse.offset[1],
            fine.offset[1]
        );
    }

    #[test]
    fn retargeting_mid_glide_continues_from_the_current_offset() {
        let mut state = scroll_state();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        draw_frame(&mut state, &mut input, &theme);
        let after_first = state.offset[1];

        // A second notch a frame later must re-aim the motion, not restart it:
        // the offset keeps moving from where it is (a restart would show up as a
        // velocity discontinuity, i.e. the jerkiness this change is about).
        input.scroll_delta = -3.0;
        draw_frame(&mut state, &mut input, &theme);
        assert_eq!(state.target[1], 120.0);
        assert!(
            state.offset[1] > after_first && state.offset[1] < state.target[1],
            "offset {} should continue past {after_first}",
            state.offset[1]
        );

        // Neither notch teleported the content: after two frames of a 60px and
        // then a 120px target the offset is still well short of either.
        assert!(
            state.offset[1] < 60.0,
            "two frames must not reach one notch"
        );
    }

    #[test]
    fn a_glide_keeps_running_when_the_cursor_leaves_the_viewport() {
        let mut state = scroll_state();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        draw_frame(&mut state, &mut input, &theme);
        let after_wheel = state.offset[1];

        // Cursor moved far away and the wheel is quiet: the glide must finish.
        let mut away = input_at(5000.0, 5000.0);
        draw_frame(&mut state, &mut away, &theme);
        assert!(
            state.offset[1] > after_wheel,
            "offset {} should still be moving",
            state.offset[1]
        );
    }

    #[test]
    fn zero_dt_applies_the_target_immediately() {
        // A paused or one-shot static frame: no time passes, so there is nothing
        // to animate in, and the frame must render the offset that was asked
        // for (a screenshot, not a mid-glide frame).
        let mut state = scroll_state();
        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        input.frame_dt = 0.0;

        draw_frame(&mut state, &mut input, &theme);

        assert_eq!(state.offset[1], 60.0);
        assert_eq!(state.offset[1], state.target[1]);
        assert_eq!(state.pending_deadline(), None);
    }

    #[test]
    fn instant_smoothing_jumps_to_the_target() {
        let mut state = scroll_state();
        state.smoothing = ScrollSmoothing::INSTANT;
        // A caller-authored knob must not be able to produce a NaN offset.
        assert!(ScrollSmoothing::new(f32::NAN).is_instant());
        assert_eq!(ScrollSmoothing::new(-1.0), ScrollSmoothing::INSTANT);

        let theme = theme();
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        draw_frame(&mut state, &mut input, &theme);

        assert_eq!(state.offset[1], 60.0);
        assert_eq!(state.pending_deadline(), None);
    }

    #[test]
    fn scroll_easing_ignores_the_theme_animation_duration() {
        // Forge: hover/press changes are instant, smooth scrolling stays.
        let mut state = scroll_state();
        let mut theme = theme();
        theme.animation_duration = 0.0;
        let mut input = input_at(50.0, 50.0);
        input.scroll_delta = -3.0;
        input.frame_dt = 1.0 / 60.0;

        draw_frame(&mut state, &mut input, &theme);

        assert_eq!(state.target[1], 60.0);
        assert!(
            state.offset[1] > 0.0 && state.offset[1] < 60.0,
            "still gliding: {}",
            state.offset[1]
        );
        assert!(state.pending_deadline().is_some());
    }

    #[test]
    fn dragging_the_thumb_is_one_to_one_and_moves_the_target_with_it() {
        let mut state = scroll_state();
        let theme = theme();
        let viewport = Rect::new(0.0, 0.0, 200.0, 200.0);

        // Grab the thumb (vertical channel is x=187..200, first key 0..13).
        let mut input = InputState {
            mouse_x: 193.0,
            mouse_y: 20.0,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        };
        let mut list = DrawList::new();
        ScrollView::new(viewport).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );

        input.mouse_clicked = false;
        input.mouse_y = 60.0;
        list.clear();
        ScrollView::new(viewport).draw(
            &mut state,
            &mut list,
            &StyleResolver::new(&theme),
            &mut input,
            |_, _| {},
        );

        // Direct manipulation: the drawn offset follows the pointer in the same
        // frame (no lag), and the target is dragged along so the next eased step
        // does not undo it.
        assert!(state.offset[1] > 0.0);
        assert_eq!(state.offset[1], state.target[1]);
        assert!(!state.is_gliding());
    }

    #[test]
    fn snap_to_and_reset_land_immediately() {
        let mut state = scroll_state();
        state.scroll_to(1, 400.0);
        assert_eq!(state.offset[1], 0.0, "scroll_to only aims");
        state.snap_to(1, 400.0);
        assert_eq!((state.offset[1], state.target[1]), (400.0, 400.0));
        assert!(!state.is_gliding());

        state.reset();
        assert_eq!(state.offset, [0.0, 0.0]);
        assert_eq!(state.target, [0.0, 0.0]);
    }

    #[test]
    fn scroll_by_accumulates_and_ignores_junk() {
        let mut state = scroll_state();
        state.scroll_by(1, 40.0);
        state.scroll_by(1, 20.0);
        assert_eq!(state.target[1], 60.0);
        state.scroll_by(1, -1000.0);
        assert_eq!(state.target[1], 0.0, "never negative");
        state.scroll_by(9, 10.0);
        state.scroll_by(1, f32::NAN);
        assert_eq!(state.target[1], 0.0);
    }
}
