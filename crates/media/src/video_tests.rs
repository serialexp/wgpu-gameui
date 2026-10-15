use super::*;
use crate::container::Container;
use crate::test_fixtures::{ffmpeg_frames, planes_md5, reader};
use wgpu_gameui::{YuvMatrix, YuvRange};

/// Every picture of fixture `name`, decoded from the start and put in the
/// order they're shown in.
fn decode_all(name: &str) -> Vec<VideoFrame> {
    let mut container = Container::open(reader(name)).expect("opens");
    let track = container.video.clone().expect("a video track");
    let mut decoder = VideoDecoder::new(&track).expect("a decoder");
    let mut reorder = Reorder::new(&track.samples);
    let mut bytes = Vec::new();
    let mut frames = Vec::new();
    for (index, sample) in track.samples.iter().enumerate() {
        container.read_sample(sample, &mut bytes).expect("reads");
        let frame = decoder.decode(sample, &bytes).expect("decodes");
        reorder.decoded(index, Some(frame), &mut frames);
    }
    frames
}

/// Each picture must be ffmpeg's to the bit (H.264 decoding is exact), shown
/// at the time ffmpeg shows it, in the same order.
fn assert_matches_ffmpeg(name: &str) {
    let frames = decode_all(&format!("{name}.mp4"));
    let (expected, (numerator, denominator)) = ffmpeg_frames(name);
    assert_eq!(frames.len(), expected.len(), "pictures");
    for (index, (frame, expected)) in frames.iter().zip(&expected).enumerate() {
        let shown_at = expected.pts * numerator * 1_000_000_000 / denominator;
        assert_eq!(frame.shown_at, shown_at, "picture {index}'s time");
        assert_eq!(planes_md5(frame), expected.md5, "picture {index}'s pixels");
    }
}

#[test]
fn high_profile_with_b_frames_decodes_as_ffmpeg_does_in_display_order() {
    assert_matches_ffmpeg("h264_aac");
}

#[test]
fn baseline_decodes_as_ffmpeg_does() {
    assert_matches_ffmpeg("baseline_full_range");
}

#[test]
fn a_picture_carries_its_size_and_the_streams_colour_settings() {
    let frames = decode_all("h264_aac.mp4");
    let first = &frames[0];
    // 180 isn't a multiple of 16: the stream codes 192 lines and crops.
    assert_eq!((first.width, first.height), (320, 180));
    assert_eq!(first.planes.y.len(), 320 * 180);
    assert_eq!(first.planes.u.len(), 160 * 90);
    // The stream says nothing: what players assume for its size.
    assert_eq!(first.colour, YuvColour::assumed_for(320, 180));

    let frames = decode_all("baseline_full_range.mp4");
    assert_eq!(
        frames[0].colour,
        YuvColour {
            matrix: YuvMatrix::Bt709,
            range: YuvRange::Full
        }
    );
}

#[test]
fn interlaced_and_ten_bit_video_are_refused_before_decoding() {
    for (name, says) in [("interlaced.mp4", "interlaced"), ("high10.mp4", "8-bit")] {
        let container = Container::open(reader(name)).expect("opens");
        let track = container.video.clone().expect("a video track");
        match VideoDecoder::new(&track) {
            Err(MediaError::Unsupported(what)) => assert!(what.contains(says), "{name}: {what}"),
            Err(other) => panic!("{name}: {other}"),
            Ok(_) => panic!("{name} accepted"),
        }
    }
}

fn frame(shown_at: i64) -> VideoFrame {
    VideoFrame {
        shown_at,
        duration: 10,
        width: 2,
        height: 2,
        planes: YuvPlanes::packed(2, vec![0; 4], vec![0; 1], vec![0; 1]),
        colour: YuvColour::assumed_for(2, 2),
    }
}

fn sample(shown_at: i64) -> Sample {
    Sample {
        offset: 0,
        size: 0,
        decode_at: 0,
        shown_at,
        duration: 10,
        keyframe: false,
    }
}

/// Shown order I B B P, decoded as I P B B: each picture is let out as
/// soon as nothing still to decode is shown before it.
#[test]
fn reorder_lets_a_picture_out_once_nothing_earlier_is_still_to_decode() {
    let shown = [0, 30, 10, 20];
    let samples: Vec<Sample> = shown.iter().map(|&at| sample(at)).collect();
    let mut reorder = Reorder::new(&samples);
    let mut out: Vec<VideoFrame> = Vec::new();
    let times = |out: &Vec<VideoFrame>| out.iter().map(|frame| frame.shown_at).collect::<Vec<_>>();

    reorder.decoded(0, Some(frame(0)), &mut out);
    assert_eq!(times(&out), [0]);
    reorder.decoded(1, Some(frame(30)), &mut out);
    assert_eq!(times(&out), [0], "30 waits for 10 and 20");
    reorder.decoded(2, Some(frame(10)), &mut out);
    assert_eq!(times(&out), [0, 10]);
    reorder.decoded(3, Some(frame(20)), &mut out);
    assert_eq!(times(&out), [0, 10, 20, 30]);
}

#[test]
fn a_skipped_sample_does_not_hold_the_others_back() {
    let samples: Vec<Sample> = [0, 20, 10].iter().map(|&at| sample(at)).collect();
    let mut reorder = Reorder::new(&samples);
    let mut out: Vec<VideoFrame> = Vec::new();
    reorder.decoded(0, Some(frame(0)), &mut out);
    reorder.decoded(1, Some(frame(20)), &mut out);
    reorder.decoded(2, None, &mut out);
    let times: Vec<i64> = out.iter().map(|frame| frame.shown_at).collect();
    assert_eq!(times, [0, 20]);
}
