//! Frame-level repaint scheduling — what a host needs to know to decide whether
//! to draw another frame, and when.
//!
//! The library's timing sources live in caller-owned [`UiState`] parts: the
//! hover/press animation clock ([`AnimationState`](crate::AnimationState)), the
//! toast stack, the tooltip hover-delay layer, the scroll glide
//! ([`ScrollState::pending_deadline`](crate::ScrollState::pending_deadline) —
//! a still-moving scroll changes what is drawn *every* frame, so it asks for
//! the next frame rather than for its own settle time), and a set of deadlines
//! registered by the application itself (caret blink, spinner phase — anything
//! the app animates on its own clock). [`UiState::end_frame`] aggregates them
//! into a [`UiFrameResult`]; [`Frame::run`]/[`Frame::run_layers`] return it
//! alongside the build closure's value so an event-driven host can schedule its
//! next redraw instead of relying on incidental mouse movement.
//!
//! # Contract
//!
//! - `next_deadline` is the *earliest* instant what is drawn changes: `0.0`
//!   (the next frame) while a hover/press transition is in flight, a toast
//!   entering its fade (or expiring), a tooltip's delay elapsing, the next
//!   frame a gliding scroll needs, or a registered app deadline. It is `None`
//!   when nothing is pending, so a host can schedule on it alone.
//! - `needs_repaint` is `next_deadline.is_some()`: some source is unsettled,
//!   whether it changes on the next frame or later.

/// Aggregated per-frame timing outcome, returned by [`UiState::end_frame`]
/// (`crate::UiState::end_frame`) and [`Frame::run`]/[`Frame::run_layers`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiFrameResult {
    /// At least one timing source is unsettled (an in-flight hover/press
    /// transition, a visible toast, a pending tooltip hover, a registered
    /// deadline). An event-driven host should schedule another frame, at
    /// [`next_deadline`](Self::next_deadline).
    pub needs_repaint: bool,
    /// Earliest instant at which the UI's appearance changes, in seconds from
    /// *now* (the moment `end_frame` ran); `0.0` means the next frame. `None`
    /// = nothing pending; the host may idle indefinitely.
    pub next_deadline: Option<f32>,
}

impl UiFrameResult {
    /// A settled frame: nothing animating, nothing scheduled.
    pub const IDLE: UiFrameResult = UiFrameResult {
        needs_repaint: false,
        next_deadline: None,
    };

    /// Fold another source's contribution into this result.
    ///
    /// `needs_repaint` is an OR; `next_deadline` keeps the minimum. Useful on
    /// the host side for combining the UI frame's result with the
    /// application's own timers before scheduling the next redraw.
    pub fn merge(&mut self, other: UiFrameResult) {
        self.needs_repaint |= other.needs_repaint;
        self.next_deadline = match (self.next_deadline, other.next_deadline) {
            (None, d) => d,
            (d, None) => d,
            (Some(a), Some(b)) => Some(a.min(b)),
        };
    }
}

/// Per-frame timing scratch, owned by [`UiState`](crate::UiState). Sources
/// contribute during a frame; [`UiState::end_frame`](crate::UiState::end_frame)
/// converts the accumulation into the frame's [`UiFrameResult`].
#[derive(Debug, Default)]
pub struct FrameTimings {
    /// Earliest registered change, in seconds from frame start; `0.0` for
    /// the next frame.
    next_deadline: Option<f32>,
    /// Seconds of delta-time this frame's ticks consumed (diagnostics/tests).
    pub dt_seconds: f32,
}

impl FrameTimings {
    /// Reset for a new frame.
    pub fn reset(&mut self, dt: f32) {
        self.next_deadline = None;
        self.dt_seconds = dt;
    }

    /// Record an unsettled source whose next visible change comes `delay_s`
    /// seconds from now. Non-positive (or NaN) values mean the next frame.
    pub fn mark_after(&mut self, delay_s: f32) {
        let delay_s = delay_s.max(0.0);
        self.next_deadline = Some(match self.next_deadline {
            Some(d) => d.min(delay_s),
            None => delay_s,
        });
    }

    /// Convert the accumulation into the frame's result: any registered
    /// deadline implies `needs_repaint`, because the host must draw again at
    /// that instant.
    pub fn finish(&self) -> UiFrameResult {
        UiFrameResult {
            needs_repaint: self.next_deadline.is_some(),
            next_deadline: self.next_deadline,
        }
    }
}

/// Sanitize a caller-supplied frame delta: NaN/negative become `0.0`, and
/// deltas above [`MAX_DT`] are clamped so a paused/backgrounded host resuming
/// with a huge delta advances animations by at most one large step instead of
/// teleporting them and instantly expiring timed widgets (toasts).
pub fn sanitize_dt(dt: f32) -> f32 {
    if !dt.is_finite() || dt <= 0.0 {
        0.0
    } else {
        dt.min(MAX_DT)
    }
}

/// Upper clamp for a caller-supplied frame delta, in seconds (100 ms).
pub const MAX_DT: f32 = 0.1;

/// Frame delta assumed when a host supplies no clock at all: one 60 Hz frame.
///
/// This is the default for [`InputState::frame_dt`](crate::InputState::frame_dt),
/// so time-based widgets ([`ScrollView`](crate::ScrollView)'s easing) still
/// animate smoothly in a host that never stamps a real delta, instead of either
/// freezing or snapping. A host that does know its delta should write it.
pub const NOMINAL_FRAME_DT: f32 = 1.0 / 60.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_dt_rejects_invalid_and_clamps_large() {
        assert_eq!(sanitize_dt(0.016), 0.016);
        assert_eq!(sanitize_dt(-0.5), 0.0);
        assert_eq!(sanitize_dt(f32::NAN), 0.0);
        // Non-finite deltas mean a broken clock: freeze rather than jump.
        assert_eq!(sanitize_dt(f32::INFINITY), 0.0);
        assert_eq!(sanitize_dt(5.0), MAX_DT);
        assert_eq!(sanitize_dt(MAX_DT), MAX_DT);
    }

    #[test]
    fn merge_keeps_earliest_deadline_and_ors_activity() {
        let mut r = UiFrameResult::IDLE;
        r.merge(UiFrameResult {
            needs_repaint: false,
            next_deadline: Some(2.0),
        });
        r.merge(UiFrameResult {
            needs_repaint: true,
            next_deadline: Some(0.5),
        });
        assert!(r.needs_repaint);
        assert_eq!(r.next_deadline, Some(0.5));
    }

    #[test]
    fn overdue_means_the_next_frame() {
        let mut t = FrameTimings::default();
        t.mark_after(-1.0);
        assert!(t.finish().needs_repaint);
        assert_eq!(t.finish().next_deadline, Some(0.0));
        t.mark_after(f32::NAN);
        assert_eq!(t.finish().next_deadline, Some(0.0));
        t.reset(0.016);
        t.mark_after(0.25);
        assert_eq!(t.finish().next_deadline, Some(0.25));
    }

    #[test]
    fn finish_reflects_accumulation() {
        let mut t = FrameTimings::default();
        assert_eq!(t.finish(), UiFrameResult::IDLE);
        t.reset(0.016);
        t.mark_after(0.4);
        assert_eq!(t.finish().next_deadline, Some(0.4));
        // A source that changes on the next frame wins, so a host scheduling
        // on the deadline alone doesn't wait for the later one.
        t.mark_after(0.0);
        let r = t.finish();
        assert!(r.needs_repaint);
        assert_eq!(r.next_deadline, Some(0.0));
    }
}
