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
use crate::schedule::{SeekMode, advance};
use crate::video::{VideoDecoder, VideoFrame};
use crate::worker::{self, Ahead, Shared, Waker};
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

/// Plays one file: its pictures, and (from phase 3) its sound.
pub struct Player {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
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
    /// Open the MP4 at `path`. A file this player can't play is an error
    /// here, before any playing: another codec, interlaced or 10-bit H.264,
    /// or not an MP4.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MediaError> {
        Self::from_reader(Box::new(std::fs::File::open(path)?))
    }

    /// Open the MP4 read by `reader`.
    pub fn from_reader(reader: Box<dyn MediaReader>) -> Result<Self, MediaError> {
        Self::with_ahead(reader, Ahead::default())
    }

    /// Open the MP4 read by `reader`, decoding as far ahead as `ahead` says.
    pub fn with_ahead(reader: Box<dyn MediaReader>, ahead: Ahead) -> Result<Self, MediaError> {
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
        let worker = std::thread::Builder::new()
            .name("gameui-media".into())
            .spawn({
                let shared = shared.clone();
                move || worker::run(container, shared, ahead)
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
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
    }

    /// Stop where it is.
    pub fn pause(&mut self, now: Instant) {
        self.playing = false;
        self.clock.stop(now);
        self.stalled = false;
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
    }

    /// Play at `speed` times normal (more than zero; the Forge player offers
    /// 0.5, 1, 1.5 and 2).
    pub fn set_speed(&mut self, speed: f64, now: Instant) {
        if speed > 0.0 && speed.is_finite() {
            self.clock.set_speed(speed, now);
        }
    }

    /// Move playback on to `now`: take the picture due from the worker's
    /// queue, notice the end, a stall or an error, and say when to update
    /// next.
    pub fn update(&mut self, now: Instant) -> Update {
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
            // Without a sound device (phase 3) the sound due is let go.
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
            // it is due, so the worker can decode on (phase 3's sound device
            // takes it instead), notices it running out, and the end.
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
