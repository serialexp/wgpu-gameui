//! How fast decoding runs on this machine, against the design's budget of
//! 1080p at 30 pictures a second on one core. These only print: run them
//! deliberately, in release, on a quiet machine, and compare with earlier
//! runs on the same one.
//!
//! ```text
//! crates/media/tests/fixtures/make.sh bench
//! cargo test -p wgpu-gameui-media --release -- --ignored --nocapture measure
//! ```

use crate::audio::{AudioBlock, AudioDecoder};
use crate::container::Container;
use crate::video::{Reorder, VideoDecoder};
use crate::{PlaybackState, Player};
use std::time::Instant;

const CLIP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test_output/bench_1080p.mp4");

fn open() -> Container {
    let file = std::fs::File::open(CLIP).unwrap_or_else(|error| {
        panic!("{CLIP}: {error}; make it with crates/media/tests/fixtures/make.sh bench")
    });
    Container::open(Box::new(file)).expect("opens")
}

#[test]
#[ignore = "a measurement: run in release with --nocapture"]
fn measure_video_decoding() {
    let mut container = open();
    let track = container.video.clone().expect("video");
    let mut decoder = VideoDecoder::new(&track).expect("a decoder");
    let mut reorder = Reorder::new(&track.samples);
    let mut bytes = Vec::new();
    let mut ready = Vec::new();
    let mut pictures = 0;
    let start = Instant::now();
    for (index, sample) in track.samples.iter().enumerate() {
        container.read_sample(sample, &mut bytes).expect("reads");
        let frame = decoder.decode(sample, &bytes).expect("decodes");
        reorder.decoded(index, Some(frame), &mut ready);
        pictures += ready.len();
        ready.clear();
    }
    let elapsed = start.elapsed();
    let rate = pictures as f64 / elapsed.as_secs_f64();
    println!(
        "video: {pictures} pictures of {}×{} in {elapsed:.2?}: {rate:.1} a second, \
         {:.1} ms each ({:.0}% of one core at 30 a second)",
        track.width,
        track.height,
        elapsed.as_secs_f64() * 1000.0 / pictures as f64,
        30.0 / rate * 100.0
    );
}

#[test]
#[ignore = "a measurement: run in release with --nocapture"]
fn measure_sound_decoding() {
    let mut container = open();
    let track = container.audio.clone().expect("sound");
    let mut decoder = AudioDecoder::new(&track).expect("a decoder");
    let mut bytes = Vec::new();
    let mut block = AudioBlock {
        starts_at: 0,
        sample_rate: 0,
        channels: 0,
        samples: Vec::new(),
    };
    let mut frames = 0;
    let start = Instant::now();
    for sample in &track.samples {
        container.read_sample(sample, &mut bytes).expect("reads");
        decoder.decode(sample, &bytes, &mut block).expect("decodes");
        frames += block.frames();
    }
    let elapsed = start.elapsed();
    let seconds = frames as f64 / f64::from(track.sample_rate);
    println!(
        "sound: {seconds:.1} s in {elapsed:.2?}: {:.0}× as fast as it plays",
        seconds / elapsed.as_secs_f64()
    );
}

/// The whole player, the worker decoding both tracks, played as fast as it
/// can decode: each update at the moment the last asked for, waiting on the
/// worker when it has nothing yet.
#[test]
#[ignore = "a measurement: run in release with --nocapture"]
fn measure_playing_as_fast_as_it_decodes() {
    let mut player = Player::open(CLIP).expect("opens");
    let (wake, woken) = std::sync::mpsc::channel();
    player.set_waker(move || {
        let _ = wake.send(());
    });
    let mut now = Instant::now();
    let start = Instant::now();
    player.play(now);
    let mut pictures = 0;
    loop {
        let update = player.update(now);
        pictures += usize::from(update.new_picture);
        // Ended asks for no update and wakes nothing: check before waiting.
        if player.state() == PlaybackState::Ended {
            break;
        }
        match update.next_at {
            Some(at) => now = at,
            None => {
                let _ = woken.recv();
            }
        }
    }
    let elapsed = start.elapsed();
    let length = player.info().duration.as_secs_f64();
    println!(
        "player: {length:.1} s ({pictures} pictures shown, {} dropped) in {elapsed:.2?}: \
         {:.1}× as fast as it plays",
        player.dropped_frames(),
        length / elapsed.as_secs_f64()
    );
}
