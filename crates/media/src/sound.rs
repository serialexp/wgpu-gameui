//! The sound thread: one per player with sound, beside the decoding worker.
//! It opens the device, then, while the player's clock runs, reads the
//! decoded blocks from where the sound has got to, converts them for the
//! device and keeps its ring a little ahead of what it plays. The blocks
//! stay queued until the player lets them go as they are heard.
//!
//! What plays is a *segment*: from a media time, at a speed. Each time the
//! clock starts (play, a seek landing, a stall ending, a new speed) the
//! player starts one; when it stops, the stream pauses and the ring is
//! emptied. While a segment plays, [`Sound::hearing`] says which media
//! time the device is playing, and the player's clock follows it.

use crate::MediaError;
use crate::audio::{AudioBlock, frames_to_nanos};
use crate::convert::{Converter, Format};
use crate::device::{Failed, OpenDevice, Stream};
use crate::ring::{Gauge, Writer, ring};
use crate::worker::Shared;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Playing from a media time at a speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Segment {
    /// Where it starts, in nanoseconds.
    pub at: i64,
    pub speed: f64,
}

/// What the device is playing at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hearing {
    /// This media time (nanoseconds).
    At(i64),
    /// Nothing yet of the segment that starts at this media time: the
    /// device is opening, or hasn't been handed any of it.
    Starting(i64),
    /// Nothing: no segment, no working device, or its sound has all been
    /// heard.
    Nothing,
}

/// The ring holds this much of the device's time…
const RING: Duration = Duration::from_secs(1);
/// …and the sound thread keeps this much of it queued (or twice the
/// device's largest callback, if more): enough to ride out the thread being
/// late, little enough that a pause or a seek has little to throw away.
const QUEUED: Duration = Duration::from_millis(200);
/// How often the sound thread looks at the ring while playing.
const LOOK: Duration = Duration::from_millis(10);
/// Frames of sound (at the file's rate) converted at a time.
const PIECE: usize = 1024;

/// What the player and the sound thread share.
#[derive(Default)]
struct Control {
    /// Counts the player's asks: a segment the thread hasn't seen yet has a
    /// newer one.
    generation: u64,
    /// What the player asks to play: `None` while its clock is stopped.
    segment: Option<Segment>,
    stop: bool,
    /// The gain asked for.
    gain: f32,
    /// Once the device is open: its ring's gauge, and its rate.
    device: Option<(Arc<Gauge>, u32)>,
    /// Why there is no sound, if the device couldn't be opened or failed.
    error: Option<Arc<MediaError>>,
    /// The generation of the segment playing, and the frame count it
    /// started at.
    started: Option<(u64, u64)>,
    /// The frame count the playing segment's sound ends at, once the thread
    /// has written the end.
    ends: Option<u64>,
}

/// The player's handle on its sound thread.
pub(crate) struct Sound {
    control: Arc<Mutex<Control>>,
    thread: Option<JoinHandle<()>>,
}

impl Sound {
    /// Start a sound thread for sound in `file`'s format, decoded into
    /// `shared`'s queues, on the device `open` finds.
    pub fn start(shared: Arc<Shared>, file: Format, open: OpenDevice) -> std::io::Result<Self> {
        let control = Arc::new(Mutex::new(Control {
            gain: 1.0,
            ..Control::default()
        }));
        let thread = std::thread::Builder::new()
            .name("gameui-sound".into())
            .spawn({
                let control = control.clone();
                move || run(&shared, control, file, open)
            })?;
        Ok(Self {
            control,
            thread: Some(thread),
        })
    }

    fn control(&self) -> MutexGuard<'_, Control> {
        lock(&self.control)
    }

    fn wake(&self) {
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }

    /// The segment asked for.
    pub fn segment(&self) -> Option<Segment> {
        self.control().segment
    }

    /// Play `segment`, or (`None`) stop.
    pub fn set_segment(&mut self, segment: Option<Segment>) {
        {
            let mut control = self.control();
            control.generation += 1;
            control.segment = segment;
            control.started = None;
            control.ends = None;
        }
        self.wake();
    }

    /// Play at `gain` (0.0 silent, 1.0 as decoded).
    pub fn set_gain(&mut self, gain: f32) {
        let mut control = self.control();
        control.gain = gain;
        if let Some((gauge, _)) = &control.device {
            gauge.set_gain(gain);
        }
    }

    /// Whether the device is open and working.
    pub fn is_ready(&self) -> bool {
        let control = self.control();
        control.device.is_some() && control.error.is_none()
    }

    /// Why there is no sound, if the device couldn't be opened or failed.
    pub fn error(&self) -> Option<Arc<MediaError>> {
        self.control().error.clone()
    }

    /// What the device plays at `now`.
    pub fn hearing(&self, now: Instant) -> Hearing {
        let control = self.control();
        let Some(segment) = control.segment else {
            return Hearing::Nothing;
        };
        if control.error.is_some() {
            return Hearing::Nothing;
        }
        let starting = Hearing::Starting(segment.at);
        let Some((gauge, rate)) = control.device.as_ref() else {
            return starting;
        };
        let Some((generation, first)) = control.started else {
            return starting;
        };
        if generation != control.generation {
            return starting;
        }
        let Some(heard) = gauge.heard(now, *rate) else {
            return starting;
        };
        if heard.handed <= first {
            return starting;
        }
        if control.ends.is_some_and(|ends| heard.frames >= ends as f64) {
            return Hearing::Nothing;
        }
        let seconds = (heard.frames - first as f64).max(0.0) / f64::from(*rate);
        Hearing::At(segment.at + (seconds * segment.speed * 1e9).round() as i64)
    }

    /// Frames written to the ring for the playing segment, once it has
    /// started.
    #[cfg(test)]
    pub fn written(&self) -> Option<u64> {
        let control = self.control();
        let (gauge, _) = control.device.as_ref()?;
        let (generation, first) = control.started?;
        (generation == control.generation).then(|| gauge.written() - first)
    }

    /// Whether the playing segment's sound has all been written.
    #[cfg(test)]
    pub fn ended(&self) -> bool {
        self.control().ends.is_some()
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        let opening = {
            let mut control = self.control();
            control.stop = true;
            control.device.is_none() && control.error.is_none()
        };
        self.wake();
        if let Some(thread) = self.thread.take() {
            // A device can take a while to open (a Bluetooth one waking up):
            // rather than hold up whoever drops the player, the thread is
            // left to see `stop` once it has, and end.
            if !opening {
                let _ = thread.join();
            }
        }
    }
}

fn lock(control: &Mutex<Control>) -> MutexGuard<'_, Control> {
    control.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the thread is playing.
struct Playing {
    generation: u64,
    /// The worker's generation it started at: after a seek, its blocks are
    /// another place's.
    queues_generation: u64,
    converter: Converter,
    /// How far the sound has been converted, in nanoseconds.
    to: i64,
    /// Converted and not yet in the ring, and how much of that is.
    pending: Vec<f32>,
    written: usize,
    /// The sound's end has been converted…
    finished: bool,
    /// …and written.
    ended: bool,
}

fn run(shared: &Shared, control: Arc<Mutex<Control>>, file: Format, open: OpenDevice) {
    // The stream failing (the device unplugged, say) ends the thread, which
    // drops the stream; the player plays on by the wall clock.
    let failed: Failed = {
        let control = control.clone();
        let thread = std::thread::current();
        Box::new(move |error| {
            lock(&control).error.get_or_insert(Arc::new(error));
            thread.unpark();
        })
    };
    let control = &*control;
    let opened = open().and_then(|device| {
        let format = device.format();
        let frames = (u64::from(format.rate) * RING.as_millis() as u64 / 1000) as usize;
        let gain = lock(control).gain;
        let (writer, reader) = ring(format.channels, frames.max(1), gain);
        let stream = device.open(reader, failed)?;
        Ok((format, writer, stream))
    });
    let (device, mut writer, mut stream) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            lock(control).error.get_or_insert(Arc::new(error));
            return;
        }
    };
    {
        // The gain may have changed while the device opened.
        let mut control = lock(control);
        writer.gauge().set_gain(control.gain);
        control.device = Some((writer.gauge().clone(), device.rate));
    }
    let least = (u64::from(device.rate) * QUEUED.as_millis() as u64 / 1000).max(1);
    let mut playing: Option<Playing> = None;
    let mut seen = 0;
    loop {
        let asked = {
            let control = lock(control);
            if control.stop || control.error.is_some() {
                return;
            }
            (control.generation != seen).then_some((control.generation, control.segment))
        };
        if let Some((generation, segment)) = asked {
            seen = generation;
            let outcome = restart(
                shared,
                control,
                &mut writer,
                &mut *stream,
                &mut playing,
                generation,
                segment,
                file,
                device,
            );
            if let Err(error) = outcome {
                lock(control).error.get_or_insert(Arc::new(error));
                return;
            }
        }
        match &mut playing {
            Some(now) if !now.ended => {
                let queued = least.max(2 * writer.gauge().largest_callback());
                fill(shared, control, &mut writer, now, file, queued);
                std::thread::park_timeout(LOOK);
            }
            _ => std::thread::park(),
        }
    }
}

/// Start what the player asked for: empty the ring, and play `segment` from
/// its start, or pause.
#[allow(clippy::too_many_arguments)]
fn restart(
    shared: &Shared,
    control: &Mutex<Control>,
    writer: &mut Writer,
    stream: &mut dyn Stream,
    playing: &mut Option<Playing>,
    generation: u64,
    segment: Option<Segment>,
    file: Format,
    device: Format,
) -> Result<(), MediaError> {
    *playing = None;
    let Some(segment) = segment else {
        stream.pause()?;
        writer.flush();
        return Ok(());
    };
    let converter = Converter::new(file, device, segment.speed)?;
    let first = writer.flush();
    *playing = Some(Playing {
        generation,
        queues_generation: shared.lock().generation,
        converter,
        to: segment.at,
        pending: Vec::new(),
        written: 0,
        finished: false,
        ended: false,
    });
    {
        let mut control = lock(control);
        if control.generation == generation {
            control.started = Some((generation, first));
        }
    }
    stream.play()
}

/// Write to the ring until `queued` frames are waiting in it, converting
/// the sound from where it has got to, while there is sound to convert.
fn fill(
    shared: &Shared,
    control: &Mutex<Control>,
    writer: &mut Writer,
    playing: &mut Playing,
    file: Format,
    queued: u64,
) {
    let channels = usize::from(file.channels);
    let mut input = Vec::new();
    loop {
        playing.written += writer.write(&playing.pending[playing.written..]);
        if playing.written < playing.pending.len() {
            return;
        }
        playing.pending.clear();
        playing.written = 0;
        if playing.finished {
            playing.ended = true;
            let mut control = lock(control);
            if control.generation == playing.generation {
                control.ends = Some(writer.written());
            }
            return;
        }
        if writer.queued() >= queued {
            return;
        }
        input.clear();
        match next_piece(shared, playing, file, &mut input) {
            Piece::Sound => {}
            Piece::Silence(frames) => input.resize(frames * channels, 0.0),
            Piece::End => {
                playing.converter.finish(&mut playing.pending);
                playing.finished = true;
                continue;
            }
            Piece::Wait => return,
        }
        playing.converter.push(&input, &mut playing.pending);
    }
}

enum Piece {
    /// Sound was copied in.
    Sound,
    /// This many frames of silence come next.
    Silence(usize),
    /// The sound has ended.
    End,
    /// Nothing is known there yet: the worker hasn't got that far.
    Wait,
}

/// The sound from `playing.to` on, a piece at a time: the decoded block
/// there (from where it has got to), the silence before the next block or
/// up to where the sound is known, or the end.
fn next_piece(shared: &Shared, playing: &mut Playing, file: Format, input: &mut Vec<f32>) -> Piece {
    let queues = shared.lock();
    if queues.generation != playing.queues_generation || queues.seek.is_some() {
        return Piece::Wait;
    }
    let mut queues = queues;
    if playing.to > queues.played_to {
        queues.played_to = playing.to;
    }
    let block = queues
        .sound
        .iter()
        .find(|block| block.ends_at() > playing.to);
    // Silence runs to the next block, or as far as the sound is known.
    let silent_to = match block {
        Some(block) => block.starts_at,
        None if queues.sound_known_to == i64::MAX => return Piece::End,
        None => queues.sound_known_to,
    };
    // Block times are whole nanoseconds, so one can start a nanosecond after
    // the last ended: less than half a frame apart is no gap.
    let gap = nearest_frames(silent_to - playing.to, file.rate);
    if gap > 0 {
        let frames = gap.min(PIECE as i64) as usize;
        playing.to = (playing.to + frames_to_nanos(frames as i64, file.rate)).min(silent_to);
        return Piece::Silence(frames);
    }
    let Some(block) = block else {
        // The worker may be waiting with its pictures queued: what it
        // counts as short is measured from here.
        drop(queues);
        shared.wake.notify_one();
        return Piece::Wait;
    };
    let mut piece: AudioBlock = block.clone();
    drop(queues);
    piece.trim_before(playing.to);
    input.extend_from_slice(&piece.samples);
    playing.to = piece.ends_at();
    Piece::Sound
}

/// The whole frames nearest to `nanos` at `rate` (none for less).
fn nearest_frames(nanos: i64, rate: u32) -> i64 {
    if nanos <= 0 {
        return 0;
    }
    ((i128::from(nanos) * i128::from(rate) + 500_000_000) / 1_000_000_000) as i64
}

#[cfg(test)]
#[path = "sound_tests.rs"]
mod tests;
