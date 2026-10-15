//! Reading an MP4 file's index: which tracks it has, their codec settings,
//! and for each sample (one compressed picture, or one block of sound) where
//! its bytes are and when it is decoded and shown. The sample bytes stay in
//! the file and are read one sample at a time as they're needed
//! ([`Container::read_sample`]), so a long video isn't loaded whole.

use crate::MediaError;
use re_mp4::{Mp4, StsdBoxContent, TrackKind};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

/// A file the player reads from: anything that can be read and sought, and
/// handed to the worker thread.
pub trait MediaReader: Read + Seek + Send {}
impl<T: Read + Seek + Send> MediaReader for T {}

/// One sample of a track. Times are in nanoseconds from the start of the
/// presentation, and can be negative: a picture decoded before the first one
/// is shown, or the block of silence an AAC encoder puts first, which the
/// file's edit list says to skip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sample {
    /// Where its bytes start in the file.
    pub offset: u64,
    /// How many bytes it has.
    pub size: u32,
    /// When it is decoded. Orders the two tracks' samples into one stream.
    pub decode_at: i64,
    /// When it is shown (or heard).
    pub shown_at: i64,
    /// How long it is shown (or lasts).
    pub duration: i64,
    /// Whether decoding can start here (a picture needing no earlier one).
    pub keyframe: bool,
}

impl Sample {
    /// When it stops being shown.
    pub fn ends_at(&self) -> i64 {
        self.shown_at + self.duration
    }
}

/// Nanoseconds from the start as a [`Duration`], before the start as zero.
pub(crate) fn duration(nanos: i64) -> Duration {
    Duration::from_nanos(nanos.max(0) as u64)
}

/// A [`Duration`] as nanoseconds from the start.
pub(crate) fn nanos(duration: Duration) -> i64 {
    i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
}

/// The H.264 track.
#[derive(Clone, Debug)]
pub(crate) struct VideoTrack {
    pub width: u32,
    pub height: u32,
    /// The `avcC` record: the parameter sets and how sample NAL units are
    /// length-prefixed.
    pub avcc: Vec<u8>,
    /// In decode order.
    pub samples: Vec<Sample>,
}

/// The AAC-LC track.
#[derive(Clone, Debug)]
pub(crate) struct AudioTrack {
    pub sample_rate: u32,
    pub channels: u16,
    /// In decode order, which is also the order they're heard in.
    pub samples: Vec<Sample>,
}

/// An opened MP4: its index, and the reader its sample bytes come from.
pub(crate) struct Container {
    reader: Box<dyn MediaReader>,
    /// The file's length, which no sample may run past.
    size: u64,
    pub video: Option<VideoTrack>,
    pub audio: Option<AudioTrack>,
    /// How long it plays: to the end of whichever track ends last.
    pub duration: Duration,
}

impl Container {
    /// Read the index of the MP4 in `reader`, keeping the reader for the
    /// samples. A file without a playable track, or with a codec this player
    /// doesn't decode, is an error.
    pub fn open(mut reader: Box<dyn MediaReader>) -> Result<Self, MediaError> {
        let size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        // `re_mp4` indexes into what it reads, and so do its track lookups;
        // a damaged file must not take the app down with it.
        let (video, audio) = catch_unwind(AssertUnwindSafe(|| {
            Mp4::read(&mut reader, size)
                .map_err(MediaError::from)
                .and_then(|mp4| tracks(&mp4))
        }))
        .map_err(|panic| MediaError::Damaged(panic_message(&panic)))??;
        if video.is_none() && audio.is_none() {
            return Err(MediaError::Unsupported(
                "the file has no video or sound track".into(),
            ));
        }
        let video_samples = video.as_ref().map(|track| track.samples.as_slice());
        let audio_samples = audio.as_ref().map(|track| track.samples.as_slice());
        let end = [video_samples, audio_samples]
            .into_iter()
            .flatten()
            .filter_map(|samples| samples.iter().map(Sample::ends_at).max())
            .max();
        let duration = duration(end.unwrap_or_default());
        Ok(Self {
            reader,
            size,
            video,
            audio,
            duration,
        })
    }

    /// Read `sample`'s bytes into `bytes`, replacing what it held. A sample
    /// that runs past the end of the file is an error, found before any room
    /// is made for it: a damaged size can say gigabytes.
    pub fn read_sample(&mut self, sample: &Sample, bytes: &mut Vec<u8>) -> Result<(), MediaError> {
        let past_end = sample
            .offset
            .checked_add(u64::from(sample.size))
            .is_none_or(|end| end > self.size);
        if past_end {
            return Err(MediaError::Damaged(
                "a sample runs past the end of the file".into(),
            ));
        }
        bytes.resize(sample.size as usize, 0);
        self.reader.seek(SeekFrom::Start(sample.offset))?;
        self.reader.read_exact(bytes).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                MediaError::Damaged("a sample runs past the end of the file".into())
            } else {
                MediaError::Io(error)
            }
        })
    }
}

/// The first video track and the first sound track, each checked for a codec
/// this player decodes.
fn tracks(mp4: &Mp4) -> Result<(Option<VideoTrack>, Option<AudioTrack>), MediaError> {
    let mut video = None;
    let mut audio = None;
    for track in mp4.tracks().values() {
        let contents = &track.trak(mp4).mdia.minf.stbl.stsd.contents;
        match track.kind {
            Some(TrackKind::Video) if video.is_none() => {
                let StsdBoxContent::Avc1(avc1) = contents else {
                    return Err(MediaError::Unsupported(format!(
                        "{} video; this player plays H.264",
                        video_codec_name(contents)
                    )));
                };
                video = Some(VideoTrack {
                    width: u32::from(avc1.width),
                    height: u32::from(avc1.height),
                    avcc: avc1.avcc.raw.clone(),
                    samples: samples(mp4, track),
                });
            }
            Some(TrackKind::Audio) if audio.is_none() => {
                audio = Some(audio_track(mp4, contents, track)?);
            }
            _ => {}
        }
    }
    Ok((video, audio))
}

fn video_codec_name(contents: &StsdBoxContent) -> String {
    match contents {
        StsdBoxContent::Av01(_) => "AV1".into(),
        StsdBoxContent::Hvc1(_) | StsdBoxContent::Hev1(_) => "H.265 (HEVC)".into(),
        StsdBoxContent::Vp08(_) => "VP8".into(),
        StsdBoxContent::Vp09(_) => "VP9".into(),
        StsdBoxContent::Unknown(fourcc) => format!("“{fourcc}”"),
        _ => "this kind of".into(),
    }
}

/// MPEG-4 audio in the `esds` descriptor's object type.
const MPEG4_AUDIO: u8 = 0x40;
/// AAC-LC, the audio object type the decoder plays.
const AAC_LC: u8 = 2;

fn audio_track(
    mp4: &Mp4,
    contents: &StsdBoxContent,
    track: &re_mp4::Track,
) -> Result<AudioTrack, MediaError> {
    let StsdBoxContent::Mp4a(mp4a) = contents else {
        return Err(MediaError::Unsupported(
            "this sound codec; this player plays AAC".into(),
        ));
    };
    let Some(esds) = &mp4a.esds else {
        return Err(MediaError::Damaged(
            "the sound track has no codec settings".into(),
        ));
    };
    let config = &esds.es_desc.dec_config;
    if config.object_type_indication != MPEG4_AUDIO {
        return Err(MediaError::Unsupported(format!(
            "sound of MPEG object type {:#04x}; this player plays AAC",
            config.object_type_indication
        )));
    }
    let specific = &config.dec_specific;
    if specific.profile != AAC_LC {
        return Err(MediaError::Unsupported(format!(
            "{}; this player plays AAC-LC",
            aac_name(specific.profile)
        )));
    }
    let channels = u16::from(specific.chan_conf);
    if !(1..=2).contains(&channels) {
        return Err(MediaError::Unsupported(if channels == 0 {
            "AAC with its channels described in the stream; this player plays mono and stereo"
                .into()
        } else {
            format!("AAC with {channels} channels; this player plays mono and stereo")
        }));
    }
    // An index past the table means the rate is written out; `re_mp4` keeps
    // that only on the sample entry.
    let sample_rate = AAC_SAMPLE_RATES
        .get(usize::from(specific.freq_index))
        .copied()
        .unwrap_or_else(|| u32::from(mp4a.samplerate.value()));
    Ok(AudioTrack {
        sample_rate,
        channels,
        samples: samples(mp4, track),
    })
}

/// The sample rates an AAC config's frequency index stands for.
const AAC_SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

fn aac_name(object_type: u8) -> String {
    match object_type {
        1 => "AAC Main".into(),
        3 => "AAC SSR".into(),
        4 => "AAC LTP".into(),
        5 => "HE-AAC".into(),
        29 => "HE-AAC v2".into(),
        other => format!("AAC of object type {other}"),
    }
}

/// The track's samples with their times in nanoseconds of the presentation.
fn samples(mp4: &Mp4, track: &re_mp4::Track) -> Vec<Sample> {
    let to_nanos = |ticks: i64, timescale: u64| -> i64 {
        // In i128: a long track at a fine timescale overflows i64 here.
        (i128::from(ticks) * 1_000_000_000 / i128::from(timescale.max(1))) as i64
    };
    let (skip, delay) = edit(mp4, track);
    let shift =
        to_nanos(delay, u64::from(mp4.moov.mvhd.timescale)) - to_nanos(skip, track.timescale);
    track
        .samples
        .iter()
        .map(|sample| {
            let shown = sample.composition_timestamp;
            let shown_at = to_nanos(shown, track.timescale);
            // The end converted the same way, so a picture ends exactly
            // where the next begins, however each rounds.
            let ends_at = to_nanos(shown + sample.duration as i64, track.timescale);
            Sample {
                offset: sample.offset,
                size: u32::try_from(sample.size).unwrap_or(u32::MAX),
                decode_at: to_nanos(sample.decode_timestamp, track.timescale) + shift,
                shown_at: shown_at + shift,
                duration: ends_at - shown_at,
                keyframe: sample.is_sync,
            }
        })
        .collect()
}

/// Where the track's edit list starts it, as `(skip, delay)`: how much of
/// the start of its samples to skip, in the track's timescale, past what
/// `re_mp4` already did; and how long to wait before it starts, in the
/// movie's timescale.
///
/// `re_mp4` ignores the edit list and moves every track so its first sample
/// is shown at zero. That is right for a video whose edit starts at its first
/// picture, but an AAC track is usually edited to skip the encoder's first
/// block (1024 samples of silence that only primes the decoder), which would
/// otherwise put the sound a block late. Only the first edit that plays
/// something is honoured, with the empty ones before it as a delay; the
/// edits after it (rare outside editing software) are ignored.
fn edit(mp4: &Mp4, track: &re_mp4::Track) -> (i64, i64) {
    let trak = track.trak(mp4);
    let Some(elst) = trak.edts.as_ref().and_then(|edts| edts.elst.as_ref()) else {
        return (0, 0);
    };
    // An empty edit's media time is -1, read as an unsigned 32 or 64 bit
    // number.
    let is_empty = |time: u64| time == u64::from(u32::MAX) || time == u64::MAX;
    let mut delay = 0i64;
    for entry in &elst.entries {
        if is_empty(entry.media_time) {
            delay = delay.saturating_add(entry.segment_duration as i64);
            continue;
        }
        // Undo `re_mp4`'s move to find where the samples' own clock starts:
        // the first sample is decoded at zero on it, and `re_mp4` moved the
        // decode times back by the shift for negative composition offsets
        // and by the earliest composition time.
        let dts_shift = trak
            .mdia
            .minf
            .stbl
            .ctts
            .as_ref()
            .map(|ctts| {
                ctts.entries
                    .iter()
                    .map(|entry| -i64::from(entry.sample_offset))
                    .max()
                    .unwrap_or(0)
                    .max(0)
            })
            .unwrap_or(0);
        let first_decode = track
            .samples
            .first()
            .map_or(0, |sample| sample.decode_timestamp);
        let earliest_shown = -first_decode - dts_shift;
        return (entry.media_time as i64 - earliest_shown, delay);
    }
    (0, delay)
}

/// What a caught panic said, if it said anything.
pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "it panicked".into())
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
