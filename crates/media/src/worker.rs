//! The worker thread: reads samples from the file in decode order, decodes
//! them, and queues the pictures and sound for the player, a bounded way
//! ahead. A seek empties the queues and starts it again from the keyframe
//! before the target.

use crate::MediaError;
use crate::audio::{AudioBlock, AudioDecoder};
use crate::container::{Container, Sample, nanos, panic_message};
use crate::schedule::{SeekMode, SeekPlan, plan_seek};
use crate::video::{Reorder, VideoDecoder, VideoFrame};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Something to call when the worker has queued what the app may want to
/// draw (the first picture after a seek or a stall), or when it has
/// finished or failed: an app wakes its event loop with it.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// How far ahead the worker decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ahead {
    /// Pictures decoded ahead of the one shown. Each is a whole picture
    /// (about 3 MB at 1080p).
    pub frames: usize,
    /// Sound decoded ahead.
    pub sound: Duration,
}

impl Default for Ahead {
    /// Eight pictures (about 25 MB at 1080p) and a second of sound.
    fn default() -> Self {
        Self {
            frames: 8,
            sound: Duration::from_secs(1),
        }
    }
}

/// What the player and the worker share, behind one lock.
pub(crate) struct Queues {
    /// Counts seeks: what the worker decoded for an earlier one is thrown
    /// away.
    pub generation: u64,
    /// A seek for the worker to start, with the generation it belongs to.
    pub seek: Option<(i64, SeekMode)>,
    pub frames: VecDeque<VideoFrame>,
    pub sound: VecDeque<AudioBlock>,
    /// How far the worker has decoded since the last seek (nanoseconds),
    /// for the scrubber's "buffered" layer.
    pub decoded_to: i64,
    /// Where the last seek landed (nanoseconds).
    pub landed_at: i64,
    /// Every picture since the last seek is queued.
    pub video_done: bool,
    /// Every sample of both tracks since the last seek is decoded.
    pub done: bool,
    /// Why decoding stopped, until the player takes it.
    pub error: Option<MediaError>,
    /// Decoding stopped at an error, taken or not: the worker decodes
    /// nothing more until a seek.
    pub failed: bool,
    /// The worker thread is gone (it panicked): not even a seek restarts it.
    pub exited: bool,
    /// The player is gone.
    pub stop: bool,
}

impl Queues {
    fn sound_ahead(&self) -> i64 {
        match (self.sound.front(), self.sound.back()) {
            (Some(first), Some(last)) => last.ends_at() - first.starts_at,
            _ => 0,
        }
    }

    fn fail(&mut self, error: MediaError) {
        self.error = Some(error);
        self.failed = true;
    }

    /// Whether the worker has nothing to do until the player takes from the
    /// queues or seeks.
    fn full(&self, ahead: &Ahead, has_video: bool) -> bool {
        self.done
            || self.failed
            || self.frames.len() >= ahead.frames
            || (self.sound_ahead() >= nanos(ahead.sound)
                // With video, never wait on sound while no picture is queued
                // and more are to come: the player may be waiting for one.
                && (!has_video || self.video_done || !self.frames.is_empty()))
    }
}

pub(crate) struct Shared {
    pub queues: Mutex<Queues>,
    /// Wakes the worker: a seek, the player taking from the queues, or
    /// stopping.
    pub wake: Condvar,
    pub waker: Mutex<Option<Waker>>,
}

impl Shared {
    pub fn new(seek_to: i64) -> Self {
        Self {
            queues: Mutex::new(Queues {
                generation: 0,
                seek: Some((seek_to, SeekMode::Exact)),
                frames: VecDeque::new(),
                sound: VecDeque::new(),
                decoded_to: seek_to,
                landed_at: seek_to,
                video_done: false,
                done: false,
                error: None,
                failed: false,
                exited: false,
                stop: false,
            }),
            wake: Condvar::new(),
            waker: Mutex::new(None),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, Queues> {
        // A panic while holding the lock leaves the queues as they were
        // between steps; nothing is half-written.
        self.queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn call_waker(&self) {
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(waker) = waker {
            waker();
        }
    }
}

/// Where the worker is in the file since the last seek.
struct Position {
    generation: u64,
    target: i64,
    next_video: usize,
    next_sound: usize,
    video: Option<VideoDecoder>,
    sound: Option<AudioDecoder>,
}

/// Run the worker until the player stops it. A panic anywhere on this
/// thread (a decoder on input it didn't expect, the app's waker) ends it
/// with an error for the player, instead of leaving the player waiting on a
/// worker that's gone.
pub(crate) fn run(container: Container, shared: Arc<Shared>, ahead: Ahead) {
    let outcome = catch_unwind(AssertUnwindSafe(|| decode(container, &shared, ahead)));
    if let Err(panic) = outcome {
        {
            let mut queues = shared.lock();
            queues.fail(MediaError::Panicked(panic_message(&*panic)));
            queues.exited = true;
        }
        // The waker may be what panicked; the player sees the error at its
        // next update either way.
        let _ = catch_unwind(AssertUnwindSafe(|| shared.call_waker()));
    }
}

fn decode(mut container: Container, shared: &Shared, ahead: Ahead) {
    // The sample tables move out of the container, which only reads bytes
    // from here on.
    let video_samples = container
        .video
        .as_mut()
        .map(|track| std::mem::take(&mut track.samples))
        .unwrap_or_default();
    let sound_samples = container
        .audio
        .as_mut()
        .map(|track| std::mem::take(&mut track.samples))
        .unwrap_or_default();
    let has_video = !video_samples.is_empty();
    // The last picture is kept even when a seek lands past its end, so
    // there is a picture to show at the end.
    let last_picture_at = video_samples.iter().map(|sample| sample.shown_at).max();
    // Built once: a seek only empties the reorder.
    let mut reorder = Reorder::new(&video_samples);
    let keyframes = crate::schedule::keyframes(&video_samples);
    let mut position: Option<Position> = None;
    let mut bytes = Vec::new();
    let mut ready = Vec::new();
    loop {
        // Wait for room in the queues or a seek.
        let seek = {
            let mut queues = shared.lock();
            loop {
                if queues.stop {
                    return;
                }
                if let Some(seek) = queues.seek.take() {
                    break Some((seek, queues.generation));
                }
                if position.is_some() && !queues.full(&ahead, has_video) {
                    break None;
                }
                queues = shared
                    .wake
                    .wait(queues)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        if let Some(((target, mode), generation)) = seek {
            // Planned and started without the lock: making the decoders
            // parses the parameter sets, which the player shouldn't wait on.
            let plan = plan_seek(&video_samples, &keyframes, &sound_samples, target, mode);
            reorder.clear();
            let started = start(&container, generation, plan);
            let mut queues = shared.lock();
            if queues.generation != generation {
                // Another seek came in meanwhile; it is waiting in `seek`.
                position = None;
                continue;
            }
            queues.landed_at = plan.target;
            queues.decoded_to = plan.target;
            match started {
                Ok(started) => position = Some(started),
                Err(error) => {
                    position = None;
                    queues.fail(error);
                    drop(queues);
                    shared.call_waker();
                }
            }
            continue;
        }
        let Some(at) = position.as_mut() else {
            continue;
        };
        // The next sample in decode order, from whichever track is behind.
        let next_video = video_samples.get(at.next_video);
        let next_sound = sound_samples.get(at.next_sound);
        let outcome = match (next_video, next_sound) {
            (Some(video), Some(sound)) if sound.decode_at < video.decode_at => {
                decode_sound(&mut container, at, sound, &mut bytes)
            }
            (Some(video), _) => decode_video(
                &mut container,
                at,
                video,
                &mut bytes,
                &mut reorder,
                &mut ready,
            ),
            (None, Some(sound)) => decode_sound(&mut container, at, sound, &mut bytes),
            (None, None) => Ok(Decoded::Nothing),
        };
        let finished_video = at.next_video >= video_samples.len();
        let finished = finished_video && at.next_sound >= sound_samples.len();
        let mut wake_app = false;
        {
            let mut queues = shared.lock();
            if queues.generation != at.generation {
                // A seek came in while decoding: this belongs to the old one.
                ready.clear();
                continue;
            }
            match outcome {
                Ok(Decoded::Sound(block)) => {
                    queues.decoded_to = queues.decoded_to.max(block.ends_at());
                    // With no pictures left to show, the player may be
                    // waiting on sound.
                    let no_more_pictures =
                        !has_video || (queues.video_done && queues.frames.is_empty());
                    wake_app |= queues.sound.is_empty() && no_more_pictures;
                    queues.sound.push_back(block);
                }
                Ok(Decoded::Pictures) => {
                    for frame in ready.drain(..) {
                        // Pictures over before the target only lead up to it.
                        if frame.ends_at() <= at.target && Some(frame.shown_at) != last_picture_at {
                            continue;
                        }
                        queues.decoded_to = queues.decoded_to.max(frame.ends_at());
                        wake_app |= queues.frames.is_empty();
                        queues.frames.push_back(frame);
                    }
                }
                Ok(Decoded::Nothing) => {}
                Err(error) => {
                    queues.fail(error);
                    wake_app = true;
                }
            }
            if finished_video && !queues.video_done {
                queues.video_done = true;
                wake_app = true;
            }
            if finished && !queues.done {
                queues.done = true;
                wake_app = true;
            }
        }
        if wake_app {
            shared.call_waker();
        }
    }
}

/// Decoders ready to decode from where `plan` says. Opening the file
/// already made them once, so failing here is unexpected, but it is still
/// an error for the player rather than a worker that silently stops.
fn start(container: &Container, generation: u64, plan: SeekPlan) -> Result<Position, MediaError> {
    let video = container
        .video
        .as_ref()
        .map(VideoDecoder::new)
        .transpose()?;
    let sound = container
        .audio
        .as_ref()
        .map(AudioDecoder::new)
        .transpose()?;
    Ok(Position {
        generation,
        target: plan.target,
        next_video: plan.video_from,
        next_sound: plan.sound_from,
        video,
        sound,
    })
}

enum Decoded {
    /// Pictures are in the reorder's output.
    Pictures,
    Sound(AudioBlock),
    Nothing,
}

fn decode_video(
    container: &mut Container,
    at: &mut Position,
    sample: &Sample,
    bytes: &mut Vec<u8>,
    reorder: &mut Reorder,
    ready: &mut Vec<VideoFrame>,
) -> Result<Decoded, MediaError> {
    let index = at.next_video;
    at.next_video += 1;
    let Some(decoder) = at.video.as_mut() else {
        return Ok(Decoded::Nothing);
    };
    container.read_sample(sample, bytes)?;
    let frame = match decoder.decode(sample, bytes) {
        Ok(frame) => Some(frame),
        // A picture that only leads up to the target can be skipped: after
        // a seek into an open GOP, the pictures before its keyframe refer
        // to ones that weren't decoded.
        Err(MediaError::Damaged(_)) if sample.ends_at() <= at.target => None,
        Err(error) => return Err(error),
    };
    reorder.decoded(index, frame, ready);
    Ok(Decoded::Pictures)
}

/// Decode a block of sound. Each block is a new buffer: it moves into the
/// queue and on to the player, one allocation per block (about 47 a second
/// at 48 kHz), not per sample.
fn decode_sound(
    container: &mut Container,
    at: &mut Position,
    sample: &Sample,
    bytes: &mut Vec<u8>,
) -> Result<Decoded, MediaError> {
    at.next_sound += 1;
    let Some(decoder) = at.sound.as_mut() else {
        return Ok(Decoded::Nothing);
    };
    container.read_sample(sample, bytes)?;
    let mut block = AudioBlock {
        starts_at: 0,
        sample_rate: 0,
        channels: 0,
        samples: Vec::new(),
    };
    decoder.decode(sample, bytes, &mut block)?;
    // The block before the target only rebuilds the first one heard, and
    // the encoder's priming block is before zero.
    block.trim_before(at.target.max(0));
    if block.samples.is_empty() {
        return Ok(Decoded::Nothing);
    }
    Ok(Decoded::Sound(block))
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
