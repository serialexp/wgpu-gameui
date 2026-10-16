use super::*;

const MONO_44K: Format = Format {
    rate: 44_100,
    channels: 1,
};

/// `seconds` of a `pitch` Hz tone at `rate`, one channel, after `silence`
/// seconds of nothing.
fn tone(rate: u32, pitch: f64, silence: f64, seconds: f64) -> Vec<f32> {
    let quiet = (silence * f64::from(rate)) as usize;
    let loud = (seconds * f64::from(rate)) as usize;
    let mut samples = vec![0.0; quiet];
    samples.extend((0..loud).map(|index| {
        let t = index as f64 / f64::from(rate);
        (0.5 * (2.0 * std::f64::consts::PI * pitch * t).sin()) as f32
    }));
    samples
}

/// Zero crossings a second in `samples` (one channel): twice a pure tone's
/// pitch.
fn crossings_a_second(samples: &[f32], rate: u32) -> f64 {
    let crossings = samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    crossings as f64 * f64::from(rate) / samples.len() as f64
}

/// Everything `input` converts to, pushed in uneven pieces as decoded
/// blocks arrive, then finished.
fn convert(input: &[f32], from: Format, to: Format, speed: f64) -> Vec<f32> {
    let mut converter = Converter::new(from, to, speed).expect("a converter");
    let mut out = Vec::new();
    let piece = 1021 * usize::from(from.channels);
    for chunk in input.chunks(piece) {
        converter.push(chunk, &mut out);
    }
    converter.finish(&mut out);
    out
}

#[test]
fn at_normal_speed_and_the_same_format_the_sound_passes_through() {
    let input = tone(44_100, 440.0, 0.0, 0.25);
    assert_eq!(convert(&input, MONO_44K, MONO_44K, 1.0), input);
}

#[test]
fn channels_are_fitted_to_the_device() {
    let input = [0.25, -0.5];
    let stereo = Format {
        channels: 2,
        ..MONO_44K
    };
    assert_eq!(
        convert(&input, MONO_44K, stereo, 1.0),
        [0.25, 0.25, -0.5, -0.5]
    );
}

#[test]
fn another_rate_keeps_the_length_and_the_pitch() {
    let to = Format {
        rate: 48_000,
        channels: 1,
    };
    let out = convert(&tone(44_100, 440.0, 0.0, 1.0), MONO_44K, to, 1.0);
    assert!(out.len().abs_diff(48_000) <= 1, "{} frames", out.len());
    let pitch = crossings_a_second(&out, 48_000) / 2.0;
    assert!((pitch - 440.0).abs() < 2.0, "{pitch} Hz");
}

#[test]
fn resampling_adds_no_delay() {
    let to = Format {
        rate: 48_000,
        channels: 1,
    };
    // A tenth of a second of silence, then the tone: it starts as far in.
    let out = convert(&tone(44_100, 440.0, 0.1, 0.4), MONO_44K, to, 1.0);
    let starts = out
        .iter()
        .position(|sample| sample.abs() > 0.05)
        .expect("the tone");
    let expected = 4_800;
    // Within a millisecond: the resampler's filter rings a little either
    // side of a hard start.
    assert!(starts.abs_diff(expected) < 48, "starts at {starts}");
}

#[test]
fn twice_the_speed_halves_the_length_and_keeps_the_pitch() {
    let out = convert(&tone(44_100, 440.0, 0.0, 2.0), MONO_44K, MONO_44K, 2.0);
    let expected = 44_100.0;
    assert!(
        (out.len() as f64 - expected).abs() < expected * 0.03,
        "{} frames",
        out.len()
    );
    let pitch = crossings_a_second(&out, 44_100) / 2.0;
    assert!((pitch - 440.0).abs() < 440.0 * 0.02, "{pitch} Hz");
}

#[test]
fn half_the_speed_doubles_the_length_and_keeps_the_pitch() {
    let out = convert(&tone(44_100, 440.0, 0.0, 1.0), MONO_44K, MONO_44K, 0.5);
    let expected = 88_200.0;
    assert!(
        (out.len() as f64 - expected).abs() < expected * 0.03,
        "{} frames",
        out.len()
    );
    let pitch = crossings_a_second(&out, 44_100) / 2.0;
    assert!((pitch - 440.0).abs() < 440.0 * 0.02, "{pitch} Hz");
}

#[test]
fn speed_and_rate_together() {
    let to = Format {
        rate: 48_000,
        channels: 2,
    };
    let out = convert(&tone(44_100, 440.0, 0.0, 2.0), MONO_44K, to, 1.5);
    let frames = out.len() / 2;
    let expected = 2.0 / 1.5 * 48_000.0;
    assert!(
        (frames as f64 - expected).abs() < expected * 0.03,
        "{frames} frames"
    );
    let left: Vec<f32> = out.chunks(2).map(|frame| frame[0]).collect();
    let pitch = crossings_a_second(&left, 48_000) / 2.0;
    assert!((pitch - 440.0).abs() < 440.0 * 0.02, "{pitch} Hz");
}

#[test]
fn a_bad_format_is_refused() {
    let none = Format {
        rate: 0,
        channels: 1,
    };
    assert!(Converter::new(none, MONO_44K, 1.0).is_err());
    assert!(Converter::new(MONO_44K, none, 1.0).is_err());
}

/// The clock counts what the device has played at the speed, so a run
/// mustn't drift from it: what the stretch holds back comes out at the end,
/// and the length is right to within a frame of video at 50 a second.
/// (Twenty seconds was as close as three: it doesn't drift.)
#[test]
fn a_run_stays_in_step_with_the_speed() {
    let input = tone(44_100, 440.0, 0.0, 3.0);
    for speed in [0.5, 1.5, 2.0] {
        let out = convert(&input, MONO_44K, MONO_44K, speed);
        let expected = input.len() as f64 / speed;
        let off = (out.len() as f64 - expected).abs() / 44_100.0;
        assert!(off < 0.02, "{speed}×: {off} s off");
    }
}

/// What `h264_aac.mp4` plays on a 44.1 kHz device: each channel keeps its
/// own tone, its length, and its place.
#[test]
fn stereo_resampled_keeps_each_channel_to_itself() {
    let from = Format {
        rate: 48_000,
        channels: 2,
    };
    let to = Format {
        rate: 44_100,
        channels: 2,
    };
    let left = tone(48_000, 440.0, 0.0, 1.0);
    let right = tone(48_000, 660.0, 0.0, 1.0);
    let input: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l, r])
        .collect();
    let out = convert(&input, from, to, 1.0);
    assert!(
        (out.len() / 2).abs_diff(44_100) <= 1,
        "{} frames",
        out.len() / 2
    );
    for (channel, pitch) in [(0, 440.0), (1, 660.0)] {
        let samples: Vec<f32> = out.chunks(2).map(|frame| frame[channel]).collect();
        let heard = crossings_a_second(&samples, 44_100) / 2.0;
        assert!((heard - pitch).abs() < 2.0, "channel {channel}: {heard} Hz");
        // No clicks: a 660 Hz sine at 0.5 moves at most 0.047 a sample.
        let jump = samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max);
        assert!(jump < 0.06, "channel {channel}: a jump of {jump}");
    }
}
