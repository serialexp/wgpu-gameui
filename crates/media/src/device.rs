//! The sound device: the system's default output through `cpal`, or, in
//! tests, a fake one that plays only when told to. Opened on the sound
//! thread, which then owns the stream (a `cpal` stream can't be handed
//! between threads on every platform).

use crate::MediaError;
use crate::convert::Format;
use crate::ring::Reader;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::time::Instant;

/// A device to play on, not yet playing.
pub(crate) trait Device {
    /// The format it plays.
    fn format(&self) -> Format;

    /// A stream playing from `reader`, paused until [`Stream::play`].
    /// `failed` is called (on the device's thread) if the stream fails once
    /// it is playing: the device was unplugged, say.
    fn open(self: Box<Self>, reader: Reader, failed: Failed)
    -> Result<Box<dyn Stream>, MediaError>;
}

/// Told when a playing stream fails.
pub(crate) type Failed = Box<dyn FnMut(MediaError) + Send>;

/// A stream the sound thread plays and pauses.
pub(crate) trait Stream {
    fn play(&mut self) -> Result<(), MediaError>;
    fn pause(&mut self) -> Result<(), MediaError>;
}

/// Finds the device, on the sound thread.
pub(crate) type OpenDevice = Box<dyn FnOnce() -> Result<Box<dyn Device>, MediaError> + Send>;

/// The system's default output.
pub(crate) fn default_device() -> Result<Box<dyn Device>, MediaError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| MediaError::Unsupported("no sound device".into()))?;
    let config = device
        .default_output_config()
        .map_err(|error| MediaError::Unsupported(format!("the sound device: {error}")))?;
    Ok(Box::new(Cpal {
        format: Format {
            rate: config.sample_rate(),
            channels: config.channels(),
        },
        sample_format: config.sample_format(),
        buffer_size: *config.buffer_size(),
        config: config.config(),
        device,
    }))
}

struct Cpal {
    device: cpal::Device,
    config: cpal::StreamConfig,
    sample_format: SampleFormat,
    buffer_size: cpal::SupportedBufferSize,
    format: Format,
}

impl Device for Cpal {
    fn format(&self) -> Format {
        self.format
    }

    fn open(
        self: Box<Self>,
        reader: Reader,
        failed: Failed,
    ) -> Result<Box<dyn Stream>, MediaError> {
        let stream = match self.sample_format {
            SampleFormat::F32 => self.build::<f32>(reader, failed),
            SampleFormat::F64 => self.build::<f64>(reader, failed),
            SampleFormat::I8 => self.build::<i8>(reader, failed),
            SampleFormat::I16 => self.build::<i16>(reader, failed),
            SampleFormat::I32 => self.build::<i32>(reader, failed),
            SampleFormat::I64 => self.build::<i64>(reader, failed),
            SampleFormat::U8 => self.build::<u8>(reader, failed),
            SampleFormat::U16 => self.build::<u16>(reader, failed),
            SampleFormat::U32 => self.build::<u32>(reader, failed),
            SampleFormat::U64 => self.build::<u64>(reader, failed),
            other => Err(MediaError::Unsupported(format!(
                "a sound device playing {other}"
            ))),
        }?;
        stream
            .pause()
            .map_err(|error| MediaError::Unsupported(format!("the sound device: {error}")))?;
        Ok(Box::new(CpalStream(stream)))
    }
}

impl Cpal {
    fn build<T: SizedSample + FromSample<f32>>(
        &self,
        mut reader: Reader,
        mut failed: Failed,
    ) -> Result<cpal::Stream, MediaError> {
        // The device's own samples are filled from these. Room for the
        // largest callback the device says it makes is made now, so the
        // callback doesn't allocate as it plays (it would only grow it for a
        // device that asks for more than it said).
        let frames = match self.buffer_size {
            cpal::SupportedBufferSize::Range { max, .. } => max.min(1 << 16),
            cpal::SupportedBufferSize::Unknown => 1 << 13,
        };
        let mut scratch: Vec<f32> =
            Vec::with_capacity(frames as usize * usize::from(self.format.channels));
        self.device
            .build_output_stream::<T, _, _>(
                self.config,
                move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                    let timestamp = info.timestamp();
                    let latency = timestamp.playback.duration_since(timestamp.callback);
                    let heard_at = Instant::now() + latency;
                    scratch.resize(out.len(), 0.0);
                    reader.fill(&mut scratch, heard_at);
                    for (sample, &value) in out.iter_mut().zip(&scratch) {
                        *sample = T::from_sample(value);
                    }
                },
                move |error| {
                    failed(MediaError::Unsupported(format!(
                        "the sound device: {error}"
                    )))
                },
                None,
            )
            .map_err(|error| MediaError::Unsupported(format!("the sound device: {error}")))
    }
}

struct CpalStream(cpal::Stream);

impl Stream for CpalStream {
    fn play(&mut self) -> Result<(), MediaError> {
        self.0
            .play()
            .map_err(|error| MediaError::Unsupported(format!("the sound device: {error}")))
    }

    fn pause(&mut self) -> Result<(), MediaError> {
        self.0
            .pause()
            .map_err(|error| MediaError::Unsupported(format!("the sound device: {error}")))
    }
}

#[cfg(test)]
pub(crate) use fake::Fake;

#[cfg(test)]
mod fake {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A device that plays only when a test says it played some frames.
    #[derive(Clone)]
    pub(crate) struct Fake {
        format: Format,
        inner: Arc<Mutex<Inner>>,
    }

    #[derive(Default)]
    struct Inner {
        reader: Option<Reader>,
        failed: Option<Failed>,
        open: bool,
        playing: bool,
    }

    impl Fake {
        pub fn new(format: Format) -> Self {
            Self {
                format,
                inner: Arc::default(),
            }
        }

        /// Opens this device on the sound thread.
        pub fn opener(&self) -> OpenDevice {
            let fake = self.clone();
            Box::new(move || Ok(Box::new(fake) as Box<dyn Device>))
        }

        /// The stream fails, as an unplugged device's does.
        pub fn fail(&self) {
            let failed = self.inner.lock().expect("fake device").failed.take();
            if let Some(mut failed) = failed {
                failed(MediaError::Unsupported("the device was unplugged".into()));
            }
        }

        /// Whether the stream is open, playing or not: the sound thread
        /// drops it as it ends.
        pub fn open(&self) -> bool {
            self.inner.lock().expect("fake device").open
        }

        /// Whether the sound thread has the stream playing.
        pub fn playing(&self) -> bool {
            self.inner.lock().expect("fake device").playing
        }

        /// The device played `frames` frames, the first heard at
        /// `heard_at`: what it played, or `None` while paused (a paused
        /// stream's callback doesn't run).
        pub fn play(&self, frames: usize, heard_at: Instant) -> Option<Vec<f32>> {
            let mut inner = self.inner.lock().expect("fake device");
            if !inner.playing {
                return None;
            }
            let reader = inner.reader.as_mut()?;
            let mut out = vec![0.0; frames * usize::from(self.format.channels)];
            reader.fill(&mut out, heard_at);
            Some(out)
        }
    }

    impl Device for Fake {
        fn format(&self) -> Format {
            self.format
        }

        fn open(
            self: Box<Self>,
            reader: Reader,
            failed: Failed,
        ) -> Result<Box<dyn Stream>, MediaError> {
            let mut inner = self.inner.lock().expect("fake device");
            inner.reader = Some(reader);
            inner.failed = Some(failed);
            inner.open = true;
            drop(inner);
            Ok(Box::new(FakeStream(self.inner.clone())))
        }
    }

    struct FakeStream(Arc<Mutex<Inner>>);

    impl Drop for FakeStream {
        fn drop(&mut self) {
            let mut inner = self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.open = false;
            inner.playing = false;
        }
    }

    impl Stream for FakeStream {
        fn play(&mut self) -> Result<(), MediaError> {
            self.0.lock().expect("fake device").playing = true;
            Ok(())
        }

        fn pause(&mut self) -> Result<(), MediaError> {
            self.0.lock().expect("fake device").playing = false;
            Ok(())
        }
    }
}
