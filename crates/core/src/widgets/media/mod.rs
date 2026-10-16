//! The media widgets (Forge `components/media`): [`Scrubber`],
//! [`PlayButton`] and [`VideoPlayer`].
//!
//! They decode nothing. A [`MediaSource`] the app implements says where
//! playback is (its [`PlayState`], time, length, how much is buffered, the
//! volume and speed) and which [`Picture`] to show, and is told what the
//! user did: play, pause, seek, change the volume or the speed. The
//! `wgpu-gameui-media` crate has one over its `Player`; a test or a gallery
//! has its own.
//!
//! A picture is a sprite the app keeps current (a YUV sprite updated in
//! place as frames arrive): the widgets draw whatever it holds, and ask for
//! a frame when [`MediaSource::next_change`] says something shown changes.

use std::time::Duration;

use crate::SpriteId;

mod scrubber;
pub use scrubber::{Scrubber, ScrubberOutput};

#[cfg(feature = "phosphor-icons")]
mod play_button;
#[cfg(feature = "phosphor-icons")]
pub use play_button::PlayButton;

#[cfg(feature = "phosphor-icons")]
mod video_player;
#[cfg(feature = "phosphor-icons")]
pub use video_player::{VideoPlayer, VideoPlayerOutput, VideoPlayerState};

/// The speeds the speed key steps through, as the Forge player's.
pub const SPEEDS: [f64; 4] = [0.5, 1.0, 1.5, 2.0];

/// What a source's playback is doing, as the Forge media components show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    /// Stopped where it is.
    Paused,
    /// Playing.
    Playing,
    /// Asked to play, and waiting for what plays next.
    Loading,
    /// Played to the end.
    Ended,
    /// Can't play: [`MediaSource::error`] says why.
    Error,
}

impl PlayState {
    /// Playing, or asked to and waiting: a play key is held in.
    pub fn is_playing(self) -> bool {
        matches!(self, Self::Playing | Self::Loading)
    }
}

/// The picture to show: a sprite the source keeps current, and its size in
/// pixels (the shape it is fitted by).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    /// The sprite holding it.
    pub sprite: SpriteId,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
}

/// Where the media widgets' playback comes from, and where what the user did
/// goes. See the [module docs](self).
pub trait MediaSource {
    /// What playback is doing.
    fn state(&self) -> PlayState;
    /// Where playback is.
    fn time(&self) -> Duration;
    /// How long it plays; `None` while that isn't known.
    fn length(&self) -> Option<Duration>;
    /// How far from the start it can play without waiting; `None` when
    /// that isn't known.
    fn buffered(&self) -> Option<Duration> {
        None
    }
    /// The volume, `0.0..=1.0`, as set (muting doesn't change it).
    fn volume(&self) -> f32;
    /// Whether it is muted.
    fn muted(&self) -> bool;
    /// The speed it plays at: 1.0 as recorded.
    fn speed(&self) -> f64;
    /// Why it can't play, when its state is [`PlayState::Error`].
    fn error(&self) -> Option<String> {
        None
    }
    /// The picture to show now, for video.
    fn picture(&self) -> Option<Picture> {
        None
    }
    /// How long until something shown changes by itself (the next picture,
    /// the time readout), so the widget asks for a frame then; `None` while
    /// nothing will.
    fn next_change(&self) -> Option<Duration>;

    /// Play from where it is.
    fn play(&mut self);
    /// Stop where it is.
    fn pause(&mut self);
    /// Go to `to`, which is within the length.
    fn seek(&mut self, to: Duration);
    /// Go to near `to` quickly, while the timeline is dragged: a source may
    /// land on a picture it can show at once (a keyframe). The drag's end
    /// seeks exactly. Seeks exactly unless a source says otherwise.
    fn scrub(&mut self, to: Duration) {
        self.seek(to);
    }
    /// Set the volume, `0.0..=1.0`.
    fn set_volume(&mut self, volume: f32);
    /// Mute or unmute, keeping the volume.
    fn set_muted(&mut self, muted: bool);
    /// Set the speed it plays at.
    fn set_speed(&mut self, speed: f64);
}

/// Play or pause, as the play key and a click on the picture do: an ended
/// source plays again from the start, and one that can't play is left be.
pub fn toggle(source: &mut dyn MediaSource) {
    match source.state() {
        PlayState::Playing | PlayState::Loading => source.pause(),
        PlayState::Paused => source.play(),
        PlayState::Ended => {
            source.seek(Duration::ZERO);
            source.play();
        }
        PlayState::Error => {}
    }
}

/// The speed after `speed` in [`SPEEDS`], round to the first; a speed not
/// in the list goes to 1×.
pub fn next_speed(speed: f64) -> f64 {
    match SPEEDS.iter().position(|&s| (s - speed).abs() < 1e-6) {
        Some(at) => SPEEDS[(at + 1) % SPEEDS.len()],
        None => 1.0,
    }
}

/// A time as the players show it: `m:ss`, or `h:mm:ss` from an hour.
/// Whole seconds, rounded down.
pub fn format_time(time: Duration) -> String {
    let seconds = time.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A speed as the speed key shows it: `0.5×`, `1×`, `1.5×`.
pub fn format_speed(speed: f64) -> String {
    let rounded = (speed * 100.0).round() / 100.0;
    format!("{rounded}×")
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
