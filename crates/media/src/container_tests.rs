use super::*;
use crate::test_fixtures::{bytes, reader};
use std::io::Cursor;

fn open(name: &str) -> Container {
    Container::open(reader(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn open_bytes(bytes: Vec<u8>) -> Result<Container, MediaError> {
    Container::open(Box::new(Cursor::new(bytes)))
}

#[test]
fn the_index_has_both_tracks_with_their_settings() {
    let container = open("h264_aac.mp4");
    let video = container.video.as_ref().expect("video");
    assert_eq!((video.width, video.height), (320, 180));
    assert_eq!(video.samples.len(), 60);
    let audio = container.audio.as_ref().expect("audio");
    assert_eq!((audio.sample_rate, audio.channels), (48000, 2));
    // ffmpeg's count, including the block that primes the decoder.
    assert_eq!(audio.samples.len(), 95);
    assert_eq!(container.duration, Duration::from_secs(2));
}

#[test]
fn keyframes_and_times_come_from_the_sample_table() {
    let container = open("h264_aac.mp4");
    let samples = &container.video.as_ref().expect("video").samples;
    let keyframes: Vec<usize> = (0..samples.len())
        .filter(|&i| samples[i].keyframe)
        .collect();
    assert_eq!(keyframes, [0, 15, 30, 45]);
    // 30 a second, the first picture shown at zero, and decoded a frame
    // ahead of it (B-frames reorder).
    let frame = 1_000_000_000 / 30;
    assert_eq!(samples[0].shown_at, 0);
    assert_eq!(
        samples[0].duration, frame,
        "512 ticks of 15360, rounded down"
    );
    assert!(samples[0].decode_at < 0, "{}", samples[0].decode_at);
    // Decode order isn't display order: the second sample is the P picture
    // shown four frames on.
    assert_eq!(samples[1].shown_at, 4 * 512 * 1_000_000_000 / 15360);
}

/// The AAC track's edit list skips the block that primes the decoder, so
/// that block is heard before zero and the next one starts at zero, as
/// ffmpeg plays it.
#[test]
fn the_edit_list_puts_the_aac_priming_block_before_the_start() {
    let container = open("h264_aac.mp4");
    let samples = &container.audio.as_ref().expect("audio").samples;
    let block = 1024 * 1_000_000_000 / 48000;
    assert_eq!(samples[0].shown_at, -block);
    assert_eq!(samples[1].shown_at, 0);
    assert_eq!(samples[1].decode_at, 0);
}

#[test]
fn a_video_without_sound_has_no_sound_track() {
    let container = open("baseline_full_range.mp4");
    assert!(container.audio.is_none());
    assert_eq!(container.video.as_ref().expect("video").samples.len(), 25);
    assert_eq!(container.duration, Duration::from_secs(1));
}

#[test]
fn sample_bytes_are_read_from_where_the_index_says() {
    let mut container = open("h264_aac.mp4");
    let sample = container.video.as_ref().expect("video").samples[0];
    let file = bytes("h264_aac.mp4");
    let mut read = vec![1, 2, 3];
    container.read_sample(&sample, &mut read).expect("reads");
    let start = sample.offset as usize;
    assert_eq!(read, file[start..start + sample.size as usize]);
}

#[test]
fn codecs_it_does_not_decode_are_named_in_the_error() {
    for (name, says) in [("hevc.mp4", "H.265"), ("mp3_audio.mp4", "AAC")] {
        match open_bytes(bytes(name)) {
            Err(MediaError::Unsupported(what)) => assert!(what.contains(says), "{name}: {what}"),
            Err(other) => panic!("{name}: {other}"),
            Ok(_) => panic!("{name} opened"),
        }
    }
}

#[test]
fn what_is_not_an_mp4_is_damaged() {
    for bytes in [
        b"just some text, not a video".to_vec(),
        vec![0xff; 4096],
        Vec::new(),
    ] {
        match open_bytes(bytes) {
            Err(MediaError::Damaged(_)) => {}
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("opened"),
        }
    }
}

/// A file cut short has its samples' box (`mdat`) claiming more than the
/// file holds, so it doesn't open, even with the index whole before it.
#[test]
fn a_cut_file_is_damaged() {
    let mut file = bytes("h264_aac.mp4");
    file.truncate(file.len() / 2);
    assert!(matches!(open_bytes(file), Err(MediaError::Damaged(_))));
}

#[test]
fn a_sample_past_the_end_of_the_file_is_damaged() {
    let mut container = open("h264_aac.mp4");
    let length = bytes("h264_aac.mp4").len() as u64;
    let mut sample = container.video.as_ref().expect("video").samples[0];
    sample.offset = length - 10;
    sample.size = 100;
    let mut read = Vec::new();
    match container.read_sample(&sample, &mut read) {
        Err(MediaError::Damaged(_)) => {}
        other => panic!("{other:?}"),
    }
}

/// A damaged size of 4 GiB is refused before any room is made for it.
#[test]
fn a_huge_sample_size_is_refused_without_allocating() {
    let mut container = open("h264_aac.mp4");
    let mut sample = container.video.as_ref().expect("video").samples[0];
    sample.size = u32::MAX;
    let mut read = Vec::new();
    assert!(matches!(
        container.read_sample(&sample, &mut read),
        Err(MediaError::Damaged(_))
    ));
    assert_eq!(read.capacity(), 0);
    sample.offset = u64::MAX - 1;
    assert!(matches!(
        container.read_sample(&sample, &mut read),
        Err(MediaError::Damaged(_))
    ));
}

/// Every prefix of a file opens or fails with an error: none panics.
#[test]
fn no_cut_of_a_file_takes_the_reader_down() {
    let file = bytes("baseline_full_range.mp4");
    for length in (0..file.len()).step_by(97) {
        let _ = open_bytes(file[..length].to_vec());
    }
}
