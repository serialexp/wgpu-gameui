//! The ring the sound device's callback plays from: the sound thread
//! writes converted sound in, the callback reads it out, and neither waits
//! on the other. Sound already in the ring is thrown away (a seek, a pause,
//! a new speed) by marking how much of it to skip, which the callback does
//! the next time it runs.
//!
//! Every frame written and read is counted, and each callback notes when
//! the first frame it hands the device is heard, so the sound thread's
//! owner can tell which frame is being heard at any moment.

use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// A ring of `capacity` frames of `channels` channels, played at `gain`
/// until told otherwise.
pub(crate) fn ring(channels: u16, capacity: usize, gain: f32) -> (Writer, Reader) {
    let channels = usize::from(channels.max(1));
    let (producer, consumer) = RingBuffer::new(capacity * channels);
    let gauge = Arc::new(Gauge {
        written: AtomicU64::new(0),
        read: AtomicU64::new(0),
        skip_to: AtomicU64::new(0),
        flushes: AtomicU64::new(0),
        largest: AtomicU64::new(0),
        gain: AtomicU32::new(gain.to_bits()),
        stamp: Mutex::new(None),
    });
    (
        Writer {
            producer,
            channels,
            gauge: gauge.clone(),
        },
        Reader {
            consumer,
            channels,
            gauge,
            gain,
            flushes: 0,
        },
    )
}

/// What the writer, the callback and the player share. Counts are in
/// frames, from the ring's start.
pub(crate) struct Gauge {
    written: AtomicU64,
    /// Read by the callback, played or skipped.
    read: AtomicU64,
    /// The callback skips what was written before this.
    skip_to: AtomicU64,
    /// Counts flushes (one with nothing to skip still fades in what
    /// follows).
    flushes: AtomicU64,
    /// The most frames a callback has asked for.
    largest: AtomicU64,
    /// The gain asked for, as `f32` bits.
    gain: AtomicU32,
    /// The last callback's, for [`heard`](Self::heard). The callback only
    /// tries the lock, and leaves it as it was if it can't have it.
    stamp: Mutex<Option<Stamp>>,
}

/// When the device hears what a callback handed it.
#[derive(Clone, Copy, Debug)]
struct Stamp {
    /// Frames read before this callback's: the first it played.
    first: u64,
    /// Frames read once it had: the frames it played end here.
    handed: u64,
    /// When the device hears the first.
    at: Instant,
}

/// The frame being heard at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Heard {
    /// Frames heard since the ring's start: where the device has got to,
    /// in part frames.
    pub frames: f64,
    /// Frames the device has been handed: it can't have got further.
    pub handed: u64,
}

impl Gauge {
    /// Frames written so far.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Frames read by the callback so far, played or skipped.
    pub fn read(&self) -> u64 {
        self.read.load(Ordering::Acquire)
    }

    /// The most frames a callback has asked for at once.
    pub fn largest_callback(&self) -> u64 {
        self.largest.load(Ordering::Relaxed)
    }

    /// Play at `gain` (1.0 as written, 0.0 silent) from the next callback.
    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// The frame heard at `now`, playing `rate` frames a second: from the
    /// last callback on at the rate, but no further than the device has been
    /// handed. `None` before the first callback.
    pub fn heard(&self, now: Instant, rate: u32) -> Option<Heard> {
        let stamp = (*self.stamp.lock().unwrap_or_else(PoisonError::into_inner))?;
        let since = match now.checked_duration_since(stamp.at) {
            Some(after) => after.as_secs_f64(),
            None => -stamp.at.duration_since(now).as_secs_f64(),
        };
        let frames = (stamp.first as f64 + since * f64::from(rate)).min(stamp.handed as f64);
        Some(Heard {
            frames,
            handed: stamp.handed,
        })
    }
}

/// The sound thread's end.
pub(crate) struct Writer {
    producer: Producer<f32>,
    channels: usize,
    gauge: Arc<Gauge>,
}

impl Writer {
    /// Write what fits of `samples` (whole frames); returns how many samples
    /// were taken.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let fits = samples.len().min(self.producer.slots());
        let whole = fits - fits % self.channels;
        let (taken, _) = self.producer.push_partial_slice(&samples[..whole]);
        let frames = (taken.len() / self.channels) as u64;
        self.gauge.written.fetch_add(frames, Ordering::Release);
        taken.len()
    }

    /// Frames there is room for.
    #[cfg(test)]
    pub fn room(&self) -> usize {
        self.producer.slots() / self.channels
    }

    /// Frames written so far.
    pub fn written(&self) -> u64 {
        self.gauge.written()
    }

    /// Frames written to be played and not read yet: what a flush threw
    /// away doesn't count, though it takes room until the callback skips it.
    pub fn queued(&self) -> u64 {
        let from = self
            .gauge
            .read()
            .max(self.gauge.skip_to.load(Ordering::Acquire));
        self.written().saturating_sub(from)
    }

    /// Throw away everything written so far: the callback skips it. Returns
    /// the count the next frame written is at.
    pub fn flush(&mut self) -> u64 {
        let written = self.written();
        self.gauge.skip_to.store(written, Ordering::Release);
        self.gauge.flushes.fetch_add(1, Ordering::Release);
        written
    }

    pub fn gauge(&self) -> &Arc<Gauge> {
        &self.gauge
    }
}

/// The device callback's end.
pub(crate) struct Reader {
    consumer: Consumer<f32>,
    channels: usize,
    gauge: Arc<Gauge>,
    /// The gain the last callback ended at.
    gain: f32,
    /// The flushes seen.
    flushes: u64,
}

impl Reader {
    /// Fill `out` (whole frames) for the device, which hears its first frame
    /// at `heard_at`: skip what was flushed, play what is written at the
    /// gain, and silence where there's nothing. What follows a flush fades
    /// in over the callback that first plays it, so it doesn't click in.
    /// Never blocks or allocates.
    pub fn fill(&mut self, out: &mut [f32], heard_at: Instant) {
        self.gauge
            .largest
            .fetch_max((out.len() / self.channels) as u64, Ordering::Relaxed);
        let mut read = self.gauge.read.load(Ordering::Relaxed);
        let skip_to = self.gauge.skip_to.load(Ordering::Acquire);
        let flushes = self.gauge.flushes.load(Ordering::Acquire);
        if flushes != self.flushes {
            self.flushes = flushes;
            self.gain = 0.0;
        }
        if skip_to > read {
            let skip = (skip_to - read) as usize * self.channels;
            let skip = skip.min(self.consumer.slots());
            if let Ok(chunk) = self.consumer.read_chunk(skip) {
                chunk.commit_all();
            }
            read += (skip / self.channels) as u64;
        }
        let first = read;
        let (played, silent) = self.consumer.pop_partial_slice(out);
        let played = played.len();
        silent.fill(0.0);
        read += (played / self.channels) as u64;
        self.gauge.read.store(read, Ordering::Release);

        let target = f32::from_bits(self.gauge.gain.load(Ordering::Relaxed));
        let from = self.gain;
        if from != target {
            let frames = (out.len() / self.channels).max(1);
            for (index, frame) in out.chunks_mut(self.channels).enumerate() {
                let gain = from + (target - from) * (index + 1) as f32 / frames as f32;
                frame.iter_mut().for_each(|sample| *sample *= gain);
            }
        } else if target != 1.0 {
            out.iter_mut().for_each(|sample| *sample *= target);
        }
        // Silence needs no ramp: one still to come starts when the sound does.
        if played > 0 {
            self.gain = target;
        }

        // A callback of nothing but silence hands the device none of the
        // sound: when the last of it is heard is as the last stamp says.
        if played > 0
            && let Ok(mut stamp) = self.gauge.stamp.try_lock()
        {
            *stamp = Some(Stamp {
                first,
                handed: read,
                at: heard_at,
            });
        }
    }
}

#[cfg(test)]
#[path = "ring_tests.rs"]
mod tests;
