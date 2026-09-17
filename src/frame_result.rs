//! Frame-level repaint scheduling — what a host needs to know to decide whether
//! to draw another frame, and when.
//!
//! The library's timing sources live in caller-owned [`UiState`] parts: the
//! hover/press animation clock ([`AnimationState`](crate::AnimationState)), the
//! toast stack, the tooltip hover-delay layer, and a set of deadlines
//! registered by the application itself (caret blink, spinner phase — anything
//! the app animates on its own clock). [`UiState::end_frame`] aggregates them
//! into a [`UiFrameResult`]; [`Frame::run`]/[`Frame::run_layers`] return it
//! alongside the build closure's value so an event-driven host can schedule its
//! next redraw instead of relying on incidental mouse movement.
//!
//! # Contract
//!
//! - `needs_repaint` is `true` whenever **any** source is unsettled or any
//!   deadline was registered this frame — including sources that were not
//!   ticked (the host may tick toasts/tooltips itself via
//!   `begin_manual`; the flag stays conservative so it can never tell a host
//!   to stop while something is visibly in flight).
//! - `next_deadline` is the *earliest* instant any source becomes visible to
//!   the user: an in-flight transition finishing, a toast entering its fade
//!   (or expiring), a tooltip's delay elapsing, or a registered app deadline.
//!   It is `None` when nothing is pending.
//! - `changed` mirrors `!needs_repaint` for bandwidth-style frame accounting.

/// Aggregated per-frame timing outcome, returned by [`UiState::end_frame`]
/// (`crate::UiState::end_frame`) and [`Frame::run`]/[`Frame::run_layers`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiFrameResult {
    /// At least one timing source is unsettled (an in-flight hover/press
    /// transition, a visible toast, a pending tooltip hover, a registered
    /// deadline). An event-driven host should schedule another frame.
    pub needs_repaint: bool,
    /// Earliest future instant at which the UI's appearance changes, in
    /// seconds from *now* (the moment `end_frame` ran). `None` = nothing
    /// pending; the host may idle indefinitely.
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
    /// Earliest registered future transition, in seconds from frame start.
    next_deadline: Option<f32>,
    /// Set when any source reported unfinished visible work.
    active: bool,
    /// Seconds of delta-time this frame's ticks consumed (diagnostics/tests).
    pub dt_seconds: f32,
}

impl FrameTimings {
    /// Reset for a new frame.
    pub fn reset(&mut self, dt: f32) {
        self.next_deadline = None;
        self.active = false;
        self.dt_seconds = dt;
    }

    /// Record an in-flight (unsettled) source whose next visible change is
    /// already due, or whose transition completes `delay_s` seconds from now.
    /// Non-positive values record activity only (already-due changes can't
    /// contribute a *future* deadline, but they do require a repaint).
    pub fn mark_after(&mut self, delay_s: f32) {
        if delay_s <= 0.0 {
            self.active = true;
            return;
        }
        self.next_deadline = Some(match self.next_deadline {
            Some(d) => d.min(delay_s),
            None => delay_s,
        });
    }

    /// Convert the accumulation into the frame's result. Conservative: any
    /// registered deadline implies `needs_repaint`, because a scheduled
    /// transition means the host must draw again at (or before) that instant.
    pub fn finish(&self) -> UiFrameResult {
        UiFrameResult {
            needs_repaint: self.active || self.next_deadline.is_some(),
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
    fn after_records_overdue_as_activity_only() {
        let mut t = FrameTimings::default();
        t.mark_after(-1.0);
        assert!(t.finish().needs_repaint);
        assert_eq!(t.finish().next_deadline, None);
        t.mark_after(0.25);
        assert_eq!(t.finish().next_deadline, Some(0.25));
    }

    #[test]
    fn finish_reflects_accumulation() {
        let mut t = FrameTimings::default();
        assert_eq!(t.finish(), UiFrameResult::IDLE);
        t.reset(0.016);
        t.mark_after(0.4);
        t.mark_after(0.0); // an already-due source
        let r = t.finish();
        assert!(r.needs_repaint);
        assert_eq!(r.next_deadline, Some(0.4));
    }
}
