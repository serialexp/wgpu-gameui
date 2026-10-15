//! The playback clock for a video without sound: a media time it was at, the
//! moment it was there, and how fast it runs. Every reading takes the
//! moment as an argument (`Instant::now()` in an app, a made-up one in
//! tests), so the clock itself never looks at the wall.
//!
//! With sound, the sound device's position becomes the clock (phase 3), and
//! this one is what a silent video plays by.

use std::time::{Duration, Instant};

/// A media time that runs with the wall clock while started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Clock {
    /// The media time it was at, in nanoseconds, when it was last set,
    /// started or stopped.
    at: i64,
    /// When it was at `at`, while it runs.
    since: Option<Instant>,
    /// Media seconds a wall second.
    speed: f64,
}

impl Clock {
    /// A stopped clock at `at` (nanoseconds), running at normal speed once
    /// started.
    pub fn stopped_at(at: i64) -> Self {
        Self {
            at,
            since: None,
            speed: 1.0,
        }
    }

    /// The media time at `now`, in nanoseconds.
    pub fn time(&self, now: Instant) -> i64 {
        match self.since {
            Some(since) => self.at + scaled(now.saturating_duration_since(since), self.speed),
            None => self.at,
        }
    }

    pub fn is_running(&self) -> bool {
        self.since.is_some()
    }

    /// Run from `now` on, from where it is.
    pub fn start(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    /// Stop where it is at `now`.
    pub fn stop(&mut self, now: Instant) {
        self.at = self.time(now);
        self.since = None;
    }

    /// Jump to `at` (nanoseconds); a running clock runs on from there.
    pub fn set(&mut self, at: i64, now: Instant) {
        self.at = at;
        if self.since.is_some() {
            self.since = Some(now);
        }
    }

    /// Run at `speed` from `now` on (media seconds a wall second, more
    /// than zero).
    pub fn set_speed(&mut self, speed: f64, now: Instant) {
        let at = self.time(now);
        self.set(at, now);
        self.speed = speed;
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// When a running clock reaches media time `at` (nanoseconds): `None`
    /// while stopped, or if it is past already.
    pub fn when(&self, at: i64, now: Instant) -> Option<Instant> {
        self.since?;
        let ahead = at - self.time(now);
        if ahead <= 0 {
            return None;
        }
        // Rounded up, so the moment asked for has the time reached.
        let wall = (ahead as f64 / self.speed).ceil();
        Some(now + Duration::from_nanos(wall as u64))
    }
}

/// `wall` at `speed`, in media nanoseconds.
fn scaled(wall: Duration, speed: f64) -> i64 {
    let nanos = wall.as_nanos() as f64 * speed;
    nanos as i64
}

#[cfg(test)]
#[path = "clock_tests.rs"]
mod tests;
