//! Decoding the AAC track: one sample in, a block of sound out.

use crate::MediaError;
use crate::container::{AudioTrack, Sample, panic_message};
use std::panic::{AssertUnwindSafe, catch_unwind};
use symphonia_codec_aac::AacDecoder;
use symphonia_core::audio::{Channels, layouts};
use symphonia_core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder as _, AudioDecoderOptions};
use symphonia_core::packet::PacketRef;
use symphonia_core::units::{Duration as Ticks, Timestamp};

/// A decoded block of sound.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBlock {
    /// When its first sample is heard, in nanoseconds from the start.
    pub(crate) starts_at: i64,
    /// Samples a second.
    pub sample_rate: u32,
    /// How many channels each frame has.
    pub channels: u16,
    /// The samples, each frame's channels together (left, right, left, …),
    /// from -1.0 to 1.0.
    pub samples: Vec<f32>,
}

impl AudioBlock {
    /// How many frames (one sample per channel) it holds.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// When its first sample is heard, from the start.
    pub fn starts_at(&self) -> std::time::Duration {
        crate::container::duration(self.starts_at)
    }

    /// When it stops, in nanoseconds from the start.
    pub(crate) fn ends_at(&self) -> i64 {
        self.starts_at + frames_to_nanos(self.frames() as i64, self.sample_rate)
    }

    /// Drop the frames heard before `at` (nanoseconds from the start), so
    /// the block starts there. A block that ends by then is left empty.
    pub(crate) fn trim_before(&mut self, at: i64) {
        if at <= self.starts_at {
            return;
        }
        let frames =
            nanos_to_frames(at - self.starts_at, self.sample_rate).min(self.frames() as i64);
        self.samples
            .drain(..frames as usize * usize::from(self.channels));
        self.starts_at += frames_to_nanos(frames, self.sample_rate);
    }

    /// Drop the frames heard from `at` on, so the block ends there.
    pub(crate) fn trim_after(&mut self, at: i64) {
        let frames =
            nanos_to_frames(at - self.starts_at, self.sample_rate).clamp(0, self.frames() as i64);
        self.samples
            .truncate(frames as usize * usize::from(self.channels));
    }
}

fn frames_to_nanos(frames: i64, rate: u32) -> i64 {
    (i128::from(frames) * 1_000_000_000 / i128::from(rate.max(1))) as i64
}

/// Frames in `nanos`, rounded up so trimming leaves nothing before the time.
fn nanos_to_frames(nanos: i64, rate: u32) -> i64 {
    ((i128::from(nanos) * i128::from(rate) + 999_999_999) / 1_000_000_000) as i64
}

/// Symphonia's AAC-LC decoder, fed one sample at a time.
pub(crate) struct AudioDecoder {
    decoder: AacDecoder,
    sample_rate: u32,
    channels: u16,
}

impl AudioDecoder {
    pub fn new(track: &AudioTrack) -> Result<Self, MediaError> {
        let channels: Channels = match track.channels {
            1 => layouts::CHANNEL_LAYOUT_MONO,
            _ => layouts::CHANNEL_LAYOUT_STEREO,
        };
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_AAC)
            .with_sample_rate(track.sample_rate)
            .with_channels(channels);
        let decoder = AacDecoder::try_new(&params, &AudioDecoderOptions::default())
            .map_err(|error| MediaError::Unsupported(format!("this AAC: {error}")))?;
        Ok(Self {
            decoder,
            sample_rate: track.sample_rate,
            channels: track.channels,
        })
    }

    /// Decode `sample` from its `bytes` into `block`, reusing its samples'
    /// storage.
    pub fn decode(
        &mut self,
        sample: &Sample,
        bytes: &[u8],
        block: &mut AudioBlock,
    ) -> Result<(), MediaError> {
        let packet = PacketRef::new(0, Timestamp::new(0), Ticks::new(0), bytes);
        let decoded = catch_unwind(AssertUnwindSafe(|| {
            self.decoder
                .decode_ref(&packet)
                .map(|buffer| buffer.copy_to_vec_interleaved(&mut block.samples))
        }));
        match decoded {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(MediaError::Damaged(format!("the sound: {error}"))),
            Err(panic) => return Err(MediaError::Panicked(panic_message(&*panic))),
        }
        block.starts_at = sample.shown_at;
        block.sample_rate = self.sample_rate;
        block.channels = self.channels;
        // The last block is decoded whole, but the track ends before its
        // end: what's past the sample's duration is the encoder's padding.
        block.trim_after(sample.ends_at());
        Ok(())
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
