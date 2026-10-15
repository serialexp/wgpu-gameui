use super::*;
use crate::container::Container;
use crate::test_fixtures::reader;

/// The fixture's sound from zero on, one channel at a time.
fn decode_channels(name: &str) -> (Vec<f32>, Vec<f32>, u32) {
    let mut container = Container::open(reader(name)).expect("opens");
    let track = container.audio.clone().expect("sound");
    let mut decoder = AudioDecoder::new(&track).expect("a decoder");
    let mut bytes = Vec::new();
    let mut block = AudioBlock {
        starts_at: 0,
        sample_rate: 0,
        channels: 0,
        samples: Vec::new(),
    };
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for sample in &track.samples {
        container.read_sample(sample, &mut bytes).expect("reads");
        decoder.decode(sample, &bytes, &mut block).expect("decodes");
        block.trim_before(0);
        for frame in block.samples.chunks(2) {
            left.push(frame[0]);
            right.push(frame[1]);
        }
    }
    (left, right, track.sample_rate)
}

/// Zero crossings a second: twice the pitch of a pure tone.
fn crossings_a_second(samples: &[f32], rate: u32) -> f64 {
    let crossings = samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    crossings as f64 * f64::from(rate) / samples.len() as f64
}

fn rms(samples: &[f32]) -> f64 {
    (samples
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

/// The fixture is 440 Hz on the left and 660 Hz on the right, at half
/// scale, for two seconds.
#[test]
fn the_tones_come_out_on_their_channels_at_their_pitch_and_level() {
    let (left, right, rate) = decode_channels("h264_aac.mp4");
    // Two seconds exactly: the last block's padding is cut off.
    assert_eq!(left.len(), 96000);
    // The middle second, clear of the encoder's start and end.
    let middle = 24000..72000;
    let left_pitch = crossings_a_second(&left[middle.clone()], rate) / 2.0;
    let right_pitch = crossings_a_second(&right[middle.clone()], rate) / 2.0;
    assert!((left_pitch - 440.0).abs() < 2.0, "left {left_pitch} Hz");
    assert!((right_pitch - 660.0).abs() < 2.0, "right {right_pitch} Hz");
    let level = 0.5 / 2f64.sqrt();
    for (side, samples) in [("left", &left), ("right", &right)] {
        let rms = rms(&samples[middle.clone()]);
        assert!((rms - level).abs() < 0.02, "{side} RMS {rms}");
    }
}

/// Without the edit list the sound would start a block late; with it, the
/// tone is already playing in the first samples heard.
#[test]
fn the_priming_block_is_not_heard() {
    let (left, _, _) = decode_channels("h264_aac.mp4");
    assert!(rms(&left[..480]) > 0.2, "{}", rms(&left[..480]));
}

fn block(starts_at: i64, frames: usize) -> AudioBlock {
    AudioBlock {
        starts_at,
        sample_rate: 1000,
        channels: 2,
        samples: (0..frames * 2).map(|i| i as f32).collect(),
    }
}

#[test]
fn trimming_drops_whole_frames_from_the_front() {
    // 1000 frames a second: a frame a millisecond.
    let mut trimmed = block(0, 10);
    trimmed.trim_before(3_500_000);
    // Frames 0 to 3 start before 3.5 ms; frame 4 is the first kept.
    assert_eq!(trimmed.frames(), 6);
    assert_eq!(trimmed.samples[0], 8.0);
    assert_eq!(trimmed.starts_at, 4_000_000);
    assert_eq!(trimmed.ends_at(), 10_000_000);

    let mut gone = block(0, 10);
    gone.trim_before(20_000_000);
    assert_eq!(gone.frames(), 0);

    let mut later = block(5_000_000, 10);
    later.trim_before(1_000_000);
    assert_eq!(later.frames(), 10, "nothing before the time to drop");
}

#[test]
fn trimming_the_end_keeps_the_frames_heard_before_it() {
    let mut trimmed = block(0, 10);
    trimmed.trim_after(3_500_000);
    // Frames 0 to 3 start before 3.5 ms.
    assert_eq!(trimmed.frames(), 4);
    assert_eq!(trimmed.samples.last(), Some(&7.0));

    let mut whole = block(0, 10);
    whole.trim_after(20_000_000);
    assert_eq!(whole.frames(), 10, "nothing after the time to drop");

    let mut gone = block(5_000_000, 10);
    gone.trim_after(1_000_000);
    assert_eq!(gone.frames(), 0);
}
