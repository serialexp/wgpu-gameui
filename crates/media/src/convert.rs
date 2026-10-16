//! Turning decoded sound into what the device plays: faster or slower with
//! the pitch kept ([`wsola`]), at the device's rate ([`rubato`]), on the
//! device's channels ([`mix`]). Runs on the sound thread, never in the
//! device's callback; each stage is skipped when it has nothing to do, so
//! sound at normal speed and the device's own rate passes through untouched.

use crate::MediaError;
use crate::mix::mix;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};
use std::num::{NonZeroU16, NonZeroU32};
use wsola::TimeStretch;

/// Samples a second and channels a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    pub rate: u32,
    pub channels: u16,
}

/// Converts one run of sound from the file's format to the device's, at one
/// speed. A new speed, or sound from another place, is a new converter: the
/// stages hold sound from before.
pub(crate) struct Converter {
    from: Format,
    to: Format,
    stretch: Option<TimeStretch>,
    resample: Option<Resample>,
    /// Between the stages, kept to spare allocating each push.
    stretched: Vec<f32>,
    resampled: Vec<f32>,
}

impl Converter {
    /// A converter from `from` to `to`, playing at `speed` times normal.
    pub fn new(from: Format, to: Format, speed: f64) -> Result<Self, MediaError> {
        let (Some(rate), Some(channels)) =
            (NonZeroU32::new(from.rate), NonZeroU16::new(from.channels))
        else {
            return Err(MediaError::Unsupported(format!("sound as {from:?}")));
        };
        if to.rate == 0 || to.channels == 0 {
            return Err(MediaError::Unsupported(format!("a sound device as {to:?}")));
        }
        let stretch = (speed != 1.0)
            .then(|| TimeStretch::new(rate, channels))
            .transpose()
            .map_err(|error| MediaError::Unsupported(format!("this speed: {error}")))?
            .map(|mut stretch| {
                stretch.set_tempo(speed as f32);
                stretch
            });
        let resample = (from.rate != to.rate)
            .then(|| Resample::new(from.rate, to.rate, from.channels))
            .transpose()?;
        Ok(Self {
            from,
            to,
            stretch,
            resample,
            stretched: Vec::new(),
            resampled: Vec::new(),
        })
    }

    /// Take in `input` (frames of the file's format) and append what that
    /// makes for the device to `out`. Each stage holds some sound back until
    /// it has enough to work on; [`finish`](Self::finish) lets it go.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.run(input, false, out);
    }

    /// The sound has ended: append everything the stages still hold.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        self.run(&[], true, out);
    }

    fn run(&mut self, input: &[f32], finish: bool, out: &mut Vec<f32>) {
        let stretched = match &mut self.stretch {
            Some(stretch) => {
                stretch.push(input);
                self.stretched.clear();
                if finish {
                    self.stretched.extend(stretch.flush());
                } else {
                    self.stretched.extend(stretch.pull(usize::MAX));
                }
                &self.stretched[..]
            }
            None => input,
        };
        let resampled = match &mut self.resample {
            Some(resample) => {
                self.resampled.clear();
                resample.push(stretched, &mut self.resampled);
                if finish {
                    resample.finish(&mut self.resampled);
                }
                &self.resampled[..]
            }
            None => stretched,
        };
        mix(resampled, self.from.channels, self.to.channels, out);
    }
}

/// Resampling from one fixed rate to another, with the resampler's delay
/// taken off the front, so the sound starts when it did.
struct Resample {
    rubato: Fft<f32>,
    channels: usize,
    ratio: f64,
    /// Input not resampled yet: less than a chunk.
    pending: Vec<f32>,
    /// One chunk's output.
    chunk: Vec<f32>,
    /// Output frames still to drop: the resampler's delay.
    skip: usize,
    /// Input frames taken in, and output frames given out.
    taken: u64,
    given: u64,
}

impl Resample {
    /// Frames the resampler works on at a time: about 20 ms, so a block
    /// isn't held long before it reaches the ring.
    const CHUNK: usize = 1024;

    fn new(from: u32, to: u32, channels: u16) -> Result<Self, MediaError> {
        let channels = usize::from(channels);
        let rubato = Fft::new(
            from as usize,
            to as usize,
            Self::CHUNK,
            channels,
            FixedSync::Input,
        )
        .map_err(|error| MediaError::Unsupported(format!("resampling to {to} Hz: {error}")))?;
        let chunk = vec![0.0; rubato.output_frames_max() * channels];
        Ok(Self {
            skip: rubato.output_delay(),
            rubato,
            channels,
            ratio: f64::from(to) / f64::from(from),
            pending: Vec::new(),
            chunk,
            taken: 0,
            given: 0,
        })
    }

    fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.taken += (input.len() / self.channels) as u64;
        self.pending.extend_from_slice(input);
        while self.pending.len() / self.channels >= self.rubato.input_frames_next() {
            self.process(None, out);
        }
    }

    /// Resample what is pending, then silence, until the output is as long
    /// as the input was, at the new rate.
    fn finish(&mut self, out: &mut Vec<f32>) {
        let expected = (self.taken as f64 * self.ratio).round() as u64;
        while self.given < expected {
            let partial = self.pending.len() / self.channels;
            let before = out.len();
            self.process(Some(partial), out);
            if out.len() == before && partial == 0 && self.skip == 0 {
                break;
            }
        }
        let over = (self.given.saturating_sub(expected)) as usize * self.channels;
        out.truncate(out.len() - over.min(out.len()));
        self.given = self.given.min(expected);
    }

    /// One chunk: a whole one from `pending`, or `partial` frames of it and
    /// silence after.
    fn process(&mut self, partial: Option<usize>, out: &mut Vec<f32>) {
        let frames = partial.unwrap_or_else(|| self.rubato.input_frames_next());
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: partial,
            active_channels_mask: None,
        };
        let input = InterleavedSlice::new(&self.pending[..], self.channels, frames)
            .expect("pending holds the frames asked for");
        let capacity = self.chunk.len() / self.channels;
        let mut output = InterleavedSlice::new_mut(&mut self.chunk[..], self.channels, capacity)
            .expect("sized for the resampler's largest output");
        let (used, made) = self
            .rubato
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .expect("buffers sized for the resampler");
        self.pending.drain(..used.min(frames) * self.channels);
        let skipped = made.min(self.skip);
        self.skip -= skipped;
        out.extend_from_slice(&self.chunk[skipped * self.channels..made * self.channels]);
        self.given += (made - skipped) as u64;
    }
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
