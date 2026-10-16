//! The player: opens a file, starts the worker, and keeps the playback
//! state, the clock and the picture to show.
//!
//! Everything that depends on the time takes the moment as an argument
//! (`Instant::now()` in an app), so a test drives it with made-up moments.
//! Call [`Player::update`] before drawing: it moves the picture on to the
//! clock, and says when the next one is due so the app can ask for a frame
//! then, rather than drawing every frame the display could show.

use crate::MediaError;
use crate::audio::{AudioBlock, AudioDecoder};
use crate::clock::Clock;
use crate::container::{Container, MediaReader, duration, nanos};
use crate::convert::Format;
use crate::device::{OpenDevice, default_device};
use crate::schedule::{SeekMode, advance};
use crate::sound::{Hearing, Segment, Sound};
use crate::video::{VideoDecoder, VideoFrame};
use crate::worker::{self, Ahead, Shared, Waker};
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long the clock waits at the start of the sound for the device to play
/// it (opening, or its output latency) before running without it.
const START_GRACE: Duration = Duration::from_secs(1);

/// The speeds the sound can be stretched to with its pitch kept.
const SPEEDS: std::ops::RangeInclusive<f64> = 0.25..=4.0;

/// What a player is doing, as the Forge media components show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    /// Stopped where it is.
    Paused,
    /// Playing.
    Playing,
    /// Asked to play, but waiting for pictures (after a seek, or on a
    /// machine that can't decode fast enough); the clock waits too.
    Loading,
    /// Played to the end. Playing again starts from the beginning.
    Ended,
    /// Can't play: see [`Player::error`].
    Error,
}

/// What a file holds, from its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaInfo {
    /// How long it plays.
    pub duration: Duration,
    /// The picture's size in pixels, if it has video.
    pub video_size: Option<(u32, u32)>,
    /// Samples a second and channels, if it has sound.
    pub sound: Option<(u32, u16)>,
}

/// What [`Player::update`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Update {
    /// The picture to show changed: upload [`Player::frame`] again, or show
    /// none if it has none (after a seek to before the video starts).
    pub new_picture: bool,
    /// When to update again: the next picture's time, or the end. `None`
    /// while nothing will change by itself (paused, or waiting on the
    /// worker, which calls the [waker](Player::set_waker) when it has
    /// something).
    pub next_at: Option<Instant>,
}

/// How a player is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// How far ahead to decode.
    pub ahead: Ahead,
    /// Play the sound on the system's default output. Without, or without a
    /// device to play on, the sound is let go as its time passes and the
    /// clock runs on the wall.
    pub sound: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ahead: Ahead::default(),
            sound: true,
        }
    }
}

/// Plays one file: its pictures and its sound.
pub struct Player {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    /// Plays the sound, if the file has any and the player was opened to.
    /// Dropped before the worker is stopped (it reads the worker's queues).
    sound: Option<Sound>,
    /// When the sound's segment was asked for.
    sound_since: Option<Instant>,
    volume: f32,
    muted: bool,
    info: MediaInfo,
    clock: Clock,
    /// Asked to play (it may be [loading](PlaybackState::Loading)).
    playing: bool,
    ended: bool,
    /// Waiting for the first picture of a seek.
    seeking: bool,
    /// Waiting for pictures with the clock held.
    stalled: bool,
    error: Option<MediaError>,
    shown: Option<VideoFrame>,
    /// Where the last block of sound let go ends (nanoseconds).
    heard_to: i64,
    dropped: u64,
    buffered: i64,
}

impl Player {
    /// Open the MP4 at `path`, to play its sound on the default output. A
    /// file this player can't play is an error here, before any playing:
    /// another codec, interlaced or 10-bit H.264, or not an MP4. Not having
    /// a sound device isn't: see [`sound_error`](Self::sound_error).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MediaError> {
        Self::from_reader(Box::new(std::fs::File::open(path)?))
    }

    /// Open the MP4 read by `reader`, to play its sound on the default
    /// output.
    pub fn from_reader(reader: Box<dyn MediaReader>) -> Result<Self, MediaError> {
        Self::with_options(reader, Options::default())
    }

    /// Open the MP4 read by `reader` as `options` say.
    pub fn with_options(
        reader: Box<dyn MediaReader>,
        options: Options,
    ) -> Result<Self, MediaError> {
        let open: Option<OpenDevice> = options.sound.then(|| Box::new(default_device) as _);
        Self::with_device(reader, options.ahead, open)
    }

    /// Open the MP4 read by `reader`, decoding as far ahead as `ahead` says,
    /// and playing its sound on the device `open` finds (none without).
    pub(crate) fn with_device(
        reader: Box<dyn MediaReader>,
        ahead: Ahead,
        open: Option<OpenDevice>,
    ) -> Result<Self, MediaError> {
        let container = Container::open(reader)?;
        // Make the decoders once here so what they refuse is refused now;
        // the worker makes its own.
        if let Some(track) = &container.video {
            VideoDecoder::new(track)?;
        }
        if let Some(track) = &container.audio {
            AudioDecoder::new(track)?;
        }
        let info = MediaInfo {
            duration: container.duration,
            video_size: container
                .video
                .as_ref()
                .map(|track| (track.width, track.height)),
            sound: container
                .audio
                .as_ref()
                .map(|track| (track.sample_rate, track.channels)),
        };
        let ahead = Ahead {
            // At least the picture after the one shown, or none is decoded.
            frames: ahead.frames.max(1),
            ..ahead
        };
        let shared = Arc::new(Shared::new(0));
        // The sound thread first: if the worker then fails to start, the
        // sound thread is stopped as it drops, while a worker left behind
        // would wait for its queues forever.
        let sound = match (info.sound, open) {
            (Some((rate, channels)), Some(open)) => Some(Sound::start(
                shared.clone(),
                Format { rate, channels },
                open,
            )?),
            _ => None,
        };
        let worker = std::thread::Builder::new()
            .name("gameui-media".into())
            .spawn({
                let shared = shared.clone();
                move || worker::run(container, shared, ahead)
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
            sound,
            sound_since: None,
            volume: 1.0,
            muted: false,
            info,
            clock: Clock::stopped_at(0),
            playing: false,
            ended: false,
            seeking: true,
            stalled: false,
            error: None,
            shown: None,
            heard_to: 0,
            dropped: 0,
            buffered: 0,
        })
    }

    /// What the file holds.
    pub fn info(&self) -> MediaInfo {
        self.info
    }

    /// Call `waker` (on the worker thread) when the worker has something
    /// the app should draw without waiting for its next frame: the first
    /// picture after a seek or a stall, the end, or an error. An app wakes
    /// its event loop with it.
    ///
    /// Dropping the player waits for the worker, which may be inside the
    /// waker, so the waker must not wait on anything the thread dropping the
    /// player holds. A waker that panics stops playback with
    /// [`MediaError::Panicked`].
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        let waker: Waker = Arc::new(waker);
        *self
            .shared
            .waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(waker);
    }

    /// The state the Forge components show.
    pub fn state(&self) -> PlaybackState {
        if self.error.is_some() {
            PlaybackState::Error
        } else if self.playing && (self.seeking || self.stalled) {
            PlaybackState::Loading
        } else if self.ended {
            PlaybackState::Ended
        } else if self.playing {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }

    /// Why it can't play, once it can't.
    pub fn error(&self) -> Option<&MediaError> {
        self.error.as_ref()
    }

    /// Where playback is at `now`.
    pub fn time(&self, now: Instant) -> Duration {
        duration(self.time_nanos(now))
    }

    fn time_nanos(&self, now: Instant) -> i64 {
        self.clock.time(now).min(nanos(self.info.duration))
    }

    /// How far ahead of where playback is the worker has decoded: what the
    /// scrubber's dim "buffered" layer shows.
    pub fn buffered(&self) -> Duration {
        duration(self.buffered)
    }

    /// Pictures skipped because they were due before they could be shown.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped
    }

    /// The picture to show, once there is one.
    pub fn frame(&self) -> Option<&VideoFrame> {
        self.shown.as_ref()
    }

    /// The playback speed (1.0 is normal).
    pub fn speed(&self) -> f64 {
        self.clock.speed()
    }

    /// How loud the sound plays, from 0.0 (silent) to 1.0 (as decoded).
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Play the sound at `volume` (clamped to 0.0 to 1.0, a multiple of
    /// the decoded samples), from the device's next callback.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 1.0)
        };
        self.apply_gain();
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// Silence the sound, or let it be heard at its volume again. The
    /// picture plays on by the sound either way.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_gain();
    }

    fn apply_gain(&mut self) {
        let gain = if self.muted { 0.0 } else { self.volume };
        if let Some(sound) = &mut self.sound {
            sound.set_gain(gain);
        }
    }

    /// Why the sound isn't playing, when the file has sound and the player
    /// was opened to play it: there's no sound device, or it failed. The
    /// picture plays on, by the wall clock.
    pub fn sound_error(&self) -> Option<Arc<MediaError>> {
        self.sound.as_ref().and_then(Sound::error)
    }

    /// Start, stop or restart the sound to match the clock: playing from
    /// where the clock is while it runs, at its speed.
    fn follow_clock(&mut self, now: Instant) {
        let Some(sound) = &mut self.sound else {
            return;
        };
        let wanted = self.clock.is_running().then(|| Segment {
            at: self.clock.time(now),
            speed: self.clock.speed(),
        });
        let playing = sound.segment();
        let same = match (playing, wanted) {
            (None, None) => true,
            // The sound leads a running clock: it only restarts at a new speed.
            (Some(playing), Some(wanted)) => playing.speed == wanted.speed,
            _ => false,
        };
        if !same {
            sound.set_segment(wanted);
            self.sound_since = wanted.map(|_| now);
        }
    }

    /// Play from where it is; from the start once it has ended.
    pub fn play(&mut self, now: Instant) {
        if self.error.is_some() || self.playing {
            return;
        }
        if self.ended {
            self.seek(Duration::ZERO, SeekMode::Exact, now);
        }
        self.playing = true;
        if !self.seeking && !self.stalled {
            self.clock.start(now);
        }
        self.follow_clock(now);
    }

    /// Stop where it is.
    pub fn pause(&mut self, now: Instant) {
        self.playing = false;
        self.clock.stop(now);
        self.stalled = false;
        self.follow_clock(now);
    }

    /// Play if paused (or ended), pause if playing.
    pub fn toggle(&mut self, now: Instant) {
        if self.playing {
            self.pause(now);
        } else {
            self.play(now);
        }
    }

    /// Go to `to` (clamped to the length). Playing goes on from there once
    /// its picture is decoded; paused shows that picture once it is.
    pub fn seek(&mut self, to: Duration, mode: SeekMode, now: Instant) {
        if self.error.is_some() {
            return;
        }
        let target = nanos(to.min(self.info.duration));
        {
            let mut queues = self.shared.lock();
            queues.generation += 1;
            queues.seek = Some((target, mode));
            queues.frames.clear();
            queues.sound.clear();
            queues.decoded_to = target;
            queues.landed_at = target;
            queues.video_done = false;
            queues.done = false;
            queues.sound_known_to = target;
            queues.played_to = target;
            // Decoding failed further on, and the player hadn't got there
            // yet: from the new place it may not. A worker that panicked is
            // gone, though, and its error stays.
            if !queues.exited {
                queues.error = None;
                queues.failed = false;
            }
        }
        self.shared.wake.notify_one();
        // Held at the target until its picture comes; a keyframe seek may
        // land elsewhere, which the first update after the worker plans it
        // picks up.
        self.clock.stop(now);
        self.clock.set(target, now);
        self.seeking = true;
        self.stalled = false;
        self.ended = false;
        self.buffered = target;
        self.heard_to = target;
        self.follow_clock(now);
    }

    /// Play at `speed` times normal, from a quarter to four times (what the
    /// sound can be stretched to with its pitch kept; the Forge player
    /// offers 0.5, 1, 1.5 and 2). Others are clamped to that; not a number
    /// changes nothing.
    pub fn set_speed(&mut self, speed: f64, now: Instant) {
        if !speed.is_nan() {
            let speed = speed.clamp(*SPEEDS.start(), *SPEEDS.end());
            self.clock.set_speed(speed, now);
            self.follow_clock(now);
        }
    }

    /// Move playback on to `now`: take the picture due from the worker's
    /// queue, notice the end, a stall or an error, and say when to update
    /// next.
    pub fn update(&mut self, now: Instant) -> Update {
        let update = self.step(now);
        self.follow_clock(now);
        update
    }

    fn step(&mut self, now: Instant) -> Update {
        // While the device plays the sound, the clock is where it has got
        // to: the picture follows the sound. Until it plays any, the clock
        // waits where the sound starts (for a while: a device that never
        // plays doesn't hold the picture for ever).
        if self.clock.is_running()
            && let Some(sound) = &self.sound
        {
            let waited = self
                .sound_since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
            let mut restart = None;
            match sound.hearing(now) {
                Hearing::At(at) => self.clock.set(at, now),
                Hearing::Starting(at) if waited <= START_GRACE => self.clock.set(at, now),
                // The clock went on without the device; once it is ready,
                // the sound starts again from where the clock has got to.
                Hearing::Starting(at) => {
                    let time = self.clock.time(now);
                    if sound.is_ready() && time > at {
                        restart = Some(Segment {
                            at: time,
                            speed: self.clock.speed(),
                        });
                    }
                }
                Hearing::Nothing => {}
            }
            if let Some(segment) = restart
                && let Some(sound) = &mut self.sound
            {
                sound.set_segment(Some(segment));
                self.sound_since = Some(now);
                self.clock.set(segment.at, now);
            }
        }
        let has_video = self.info.video_size.is_some();
        let mut new_picture = false;
        let next_picture_at;
        let next_sound_ends_at;
        let video_done;
        let done;
        {
            let mut queues = self.shared.lock();
            if self.seeking {
                // A keyframe seek lands on the keyframe; the clock goes with it.
                let landed = queues.landed_at;
                if queues.seek.is_none() && self.clock.time(now) != landed {
                    self.clock.set(landed, now);
                }
            }
            let time = self.time_nanos(now);
            let (dropped, changed) = advance(&mut queues.frames, &mut self.shown, time);
            self.dropped += dropped as u64;
            new_picture |= changed;
            // The worker stopped at a sample it couldn't decode: what it
            // queued before that plays first.
            if queues.frames.is_empty() && queues.error.is_some() {
                self.error = queues.error.take();
            }
            // The sound heard (or, without a device, due) is let go.
            let mut took = changed;
            while let Some(block) = queues.sound.pop_front_if(|block| block.ends_at() <= time) {
                self.heard_to = block.ends_at();
                took = true;
            }
            if took {
                // Room was made: the worker can decode on.
                self.shared.wake.notify_one();
            }
            next_picture_at = queues.frames.front().map(|frame| frame.shown_at);
            next_sound_ends_at = queues.sound.front().map(AudioBlock::ends_at);
            video_done = queues.video_done;
            done = queues.done;
            self.buffered = queues.decoded_to;
            queues.played_to = queues.played_to.max(time);
            // A seek has landed once its picture is here (shown now, or
            // queued for a moment after the target when the video starts
            // later) or there is none to come. Sound can be decoded before
            // the picture it goes with, so with video it doesn't count.
            let arrived = if has_video {
                changed || !queues.frames.is_empty() || video_done
            } else {
                !queues.sound.is_empty() || done
            };
            if self.seeking && queues.seek.is_none() && arrived {
                self.seeking = false;
                // The picture from before the seek stayed up while it
                // decoded; if the target has none yet (the video starts
                // later), nothing is shown until it does.
                if !changed && self.shown.take().is_some() {
                    new_picture = true;
                }
            }
        }
        if self.error.is_some() {
            self.playing = false;
            self.clock.stop(now);
            return Update {
                new_picture,
                next_at: None,
            };
        }
        let time = self.time_nanos(now);
        let end = nanos(self.info.duration);
        let shown_to = self.shown.as_ref().map(VideoFrame::ends_at);
        // Every picture decoded isn't every picture shown: up to `Ahead`'s
        // worth are still queued.
        let no_more_pictures = !has_video || (video_done && next_picture_at.is_none());
        // Out of pictures before the end, with more to come: hold the clock
        // at the end of the one shown until the next arrives.
        let out_of_pictures = !no_more_pictures
            && next_picture_at.is_none()
            && shown_to.is_none_or(|shown_to| shown_to <= time);
        // With no pictures left, the sound is what plays: out of it with
        // more to come, hold the clock where the last of it ends (or the last
        // picture does, if that runs on further: the clock never goes back).
        let waiting_on_sound =
            no_more_pictures && self.info.sound.is_some() && !done && next_sound_ends_at.is_none();
        let sound_runs_out_at = self.heard_to.max(shown_to.unwrap_or(i64::MIN));
        let out_of_sound = waiting_on_sound && sound_runs_out_at <= time;
        let starved = out_of_pictures || out_of_sound;
        if self.playing {
            if self.seeking || starved {
                if self.clock.is_running() {
                    let hold = if out_of_pictures {
                        shown_to.unwrap_or(time)
                    } else if out_of_sound {
                        sound_runs_out_at
                    } else {
                        time
                    }
                    .min(time);
                    self.clock.stop(now);
                    self.clock.set(hold, now);
                }
                self.stalled = starved;
            } else {
                self.stalled = false;
                self.clock.start(now);
            }
        }
        let time = self.time_nanos(now);
        // Paused at the end (a seek there) is ended too: playing starts over.
        if done && next_picture_at.is_none() && time >= end && !self.ended {
            self.playing = false;
            self.ended = true;
            self.clock.stop(now);
            self.clock.set(end, now);
        }
        let next_at = if self.playing && self.clock.is_running() {
            let picture = match next_picture_at {
                Some(at) => Some(at),
                // Out of pictures with more to come: the stall starts when
                // the one shown ends, unless the next comes first (the
                // worker wakes the app for it).
                None if !no_more_pictures => shown_to,
                None => None,
            };
            // Without pictures to move it on, the player lets the sound go as
            // it is heard, so the worker can decode on, notices it running
            // out, and the end.
            let sound = next_sound_ends_at
                .or(waiting_on_sound.then_some(sound_runs_out_at))
                .filter(|_| no_more_pictures);
            let end = no_more_pictures.then_some(end);
            [picture, sound, end]
                .into_iter()
                .flatten()
                .min()
                .map(|at| self.clock.when(at, now).unwrap_or(now))
        } else {
            None
        };
        Update {
            new_picture,
            next_at,
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.sound = None;
        self.shared.lock().stop = true;
        self.shared.wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
