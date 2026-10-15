//! Decoding the H.264 track: one sample in, its picture out, and the
//! pictures put back into the order they're shown in.

use crate::MediaError;
use crate::container::{Sample, VideoTrack, panic_message};
use crate::sps::{SpsInfo, read_sps};
use rust_h264::decoder::Decoder;
use rust_h264::nal::{NalUnitType, parse_avcc, parse_avcc_config};
use std::panic::{AssertUnwindSafe, catch_unwind};
use wgpu_gameui::{YuvColour, YuvPlanes};

/// A decoded picture and when to show it.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    /// When it is shown, in nanoseconds from the start.
    pub(crate) shown_at: i64,
    /// How long it is shown, in nanoseconds.
    pub(crate) duration: i64,
    /// Its size in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// Its three planes, as a YUV sprite takes them.
    pub planes: YuvPlanes<'static>,
    /// How its codes turn into colour.
    pub colour: YuvColour,
}

impl VideoFrame {
    /// When it is shown, from the start.
    pub fn shown_at(&self) -> std::time::Duration {
        crate::container::duration(self.shown_at)
    }

    /// When the next picture takes over, in nanoseconds from the start.
    pub(crate) fn ends_at(&self) -> i64 {
        self.shown_at + self.duration
    }
}

/// `rust_h264`'s decoder, fed one sample at a time.
pub(crate) struct VideoDecoder {
    decoder: Decoder,
    length_size: usize,
    sps: SpsInfo,
}

impl VideoDecoder {
    /// A decoder primed with the track's parameter sets. A stream the
    /// decoder can't play (interlaced, 10-bit, not 4:2:0) is
    /// [`MediaError::Unsupported`] here, before anything is decoded.
    pub fn new(track: &VideoTrack) -> Result<Self, MediaError> {
        let config = parse_avcc_config(&track.avcc)
            .map_err(|what| MediaError::Damaged(format!("the video's settings: {what}")))?;
        let Some(first_sps) = config.sps_nals.first() else {
            return Err(MediaError::Damaged(
                "the video has no settings (SPS)".into(),
            ));
        };
        let sps = read_sps(&first_sps.rbsp)?;
        let mut decoder = Self {
            decoder: Decoder::new(),
            length_size: config.length_size,
            sps,
        };
        for nal in config.sps_nals.iter().chain(&config.pps_nals) {
            decoder.guarded(|decoder| decoder.decode_nal(nal).map(drop))?;
        }
        Ok(decoder)
    }

    /// Decode `sample` from its `bytes`. Every sample of a progressive H.264
    /// track is one picture, and this returns it: the decoder holds a
    /// picture until the next one starts, so it is flushed after each
    /// sample.
    pub fn decode(&mut self, sample: &Sample, bytes: &[u8]) -> Result<VideoFrame, MediaError> {
        let nals = parse_avcc(bytes, self.length_size);
        for nal in &nals {
            if nal.nal_unit_type == NalUnitType::Sps {
                // New settings mid-stream: check them as the first were.
                self.sps = read_sps(&nal.rbsp)?;
            }
            self.guarded(|decoder| decoder.decode_nal(nal).map(drop))?;
        }
        let frame = self
            .guarded(|decoder| Ok(decoder.flush()))?
            .ok_or_else(|| MediaError::Damaged("a video sample has no picture".into()))?;
        let colour = self.sps.colour(frame.width, frame.height);
        Ok(VideoFrame {
            shown_at: sample.shown_at,
            duration: sample.duration,
            width: frame.width,
            height: frame.height,
            planes: YuvPlanes::packed(frame.width, frame.y, frame.u, frame.v),
            colour,
        })
    }

    /// Run `step` on the decoder, with its error or panic as a
    /// [`MediaError`]. After a panic the decoder's state is unknown, so the
    /// caller stops using it.
    fn guarded<T>(
        &mut self,
        step: impl FnOnce(&mut Decoder) -> Result<T, rust_h264::error::DecodeError>,
    ) -> Result<T, MediaError> {
        match catch_unwind(AssertUnwindSafe(|| step(&mut self.decoder))) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(rust_h264::error::DecodeError::Unsupported(what))) => {
                Err(MediaError::Unsupported(format!("H.264 with {what}")))
            }
            Ok(Err(error)) => Err(MediaError::Damaged(format!("the video: {error}"))),
            Err(panic) => Err(MediaError::Panicked(panic_message(&*panic))),
        }
    }
}

/// Puts decoded pictures back into the order they're shown in.
///
/// H.264 with B-frames decodes a picture before the earlier ones that refer
/// to it, so pictures come out of the decoder out of order. The sample
/// table says exactly when each is shown, so a picture is let out once
/// every sample still to be decoded is shown after it.
pub(crate) struct Reorder {
    /// For each sample in decode order, the earliest time any sample after it
    /// is shown (`i64::MAX` after the last).
    shown_after: Vec<i64>,
    /// Decoded pictures waiting for an earlier one, earliest first.
    waiting: Vec<VideoFrame>,
}

impl Reorder {
    pub fn new(samples: &[Sample]) -> Self {
        let mut shown_after = vec![i64::MAX; samples.len()];
        for index in (0..samples.len().saturating_sub(1)).rev() {
            shown_after[index] = shown_after[index + 1].min(samples[index + 1].shown_at);
        }
        Self {
            shown_after,
            waiting: Vec::new(),
        }
    }

    /// Sample `index` has been decoded into `frame` (or skipped, with
    /// `None`): push the pictures now due onto `ready`, in order.
    pub fn decoded(
        &mut self,
        index: usize,
        frame: Option<VideoFrame>,
        ready: &mut impl Extend<VideoFrame>,
    ) {
        if let Some(frame) = frame {
            let at = self
                .waiting
                .partition_point(|waiting| waiting.shown_at <= frame.shown_at);
            self.waiting.insert(at, frame);
        }
        let before = self.shown_after.get(index).copied().unwrap_or(i64::MAX);
        let due = self
            .waiting
            .partition_point(|waiting| waiting.shown_at < before);
        ready.extend(self.waiting.drain(..due));
    }

    /// Forget the pictures waiting, for a seek.
    pub fn clear(&mut self) {
        self.waiting.clear();
    }
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;
