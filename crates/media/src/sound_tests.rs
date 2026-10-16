//! The sound thread with a fake device and blocks queued by hand. A test
//! waits for the thread by looking at what it has written, with a generous
//! limit that only says it hung; nothing here measures how long anything
//! took.

use super::*;
use crate::device::Fake;
use std::sync::mpsc;

/// Long enough that only a hang reaches it.
const HUNG: Duration = Duration::from_secs(30);

/// A thousand frames a second: a frame a millisecond.
const MONO_1K: Format = Format {
    rate: 1_000,
    channels: 1,
};

const MILLI: i64 = 1_000_000;

/// A mono block at 1 kHz from `starts` milliseconds holding `samples`.
fn block(starts: i64, samples: Vec<f32>) -> AudioBlock {
    AudioBlock {
        starts_at: starts * MILLI,
        sample_rate: MONO_1K.rate,
        channels: 1,
        samples,
    }
}

/// `frames` samples counting up from `from`, a thousandth apart.
fn counting(from: usize, frames: usize) -> Vec<f32> {
    (from..from + frames)
        .map(|frame| frame as f32 / 1000.0)
        .collect()
}

/// Queues holding `blocks`, with the sound known to `known_to`
/// (`i64::MAX`: decoded to its end).
fn queued(blocks: Vec<AudioBlock>, known_to: i64) -> Arc<Shared> {
    let shared = Arc::new(Shared::new(0));
    {
        let mut queues = shared.lock();
        queues.seek = None;
        queues.sound = blocks.into();
        queues.sound_known_to = known_to;
    }
    shared
}

/// Queues holding `blocks`, the whole of the sound.
fn all_of(blocks: Vec<AudioBlock>) -> Arc<Shared> {
    queued(blocks, i64::MAX)
}

/// Wait until `done` says so.
fn wait(mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + HUNG;
    while !done() {
        assert!(Instant::now() < limit, "the sound thread hung");
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Rig {
    sound: Sound,
    device: Fake,
    start: Instant,
}

impl Rig {
    fn new(shared: Arc<Shared>) -> Self {
        Self::with_format(shared, MONO_1K)
    }

    fn with_format(shared: Arc<Shared>, format: Format) -> Self {
        let device = Fake::new(format);
        let sound = Sound::start(shared, format, device.opener()).expect("starts");
        Self {
            sound,
            device,
            start: Instant::now(),
        }
    }

    fn at(&self, millis: u64) -> Instant {
        self.start + Duration::from_millis(millis)
    }

    fn play_from(&mut self, millis: i64) {
        self.sound.set_segment(Some(Segment {
            at: millis * MILLI,
            speed: 1.0,
        }));
    }

    /// What the device plays in `frames` frames, the first in a callback
    /// of its own: a one-frame callback has no room to fade in, so what is
    /// played is what was written.
    fn listen(&self, frames: usize) -> Vec<f32> {
        let mut played = self.device.play(1, self.at(0)).expect("playing");
        played.extend(self.device.play(frames - 1, self.at(0)).expect("playing"));
        played
    }

    fn wait_until_written(&self, frames: u64) {
        wait(|| {
            self.sound
                .written()
                .is_some_and(|written| written >= frames)
        });
    }

    fn wait_until_ended(&self) {
        wait(|| self.sound.ended());
    }
}

#[test]
fn the_blocks_play_from_where_the_segment_starts() {
    let shared = all_of(vec![
        block(0, counting(0, 100)),
        block(100, counting(100, 100)),
    ]);
    let mut rig = Rig::new(shared);
    rig.play_from(50);
    rig.wait_until_ended();
    assert_eq!(rig.listen(150), counting(50, 150));
}

#[test]
fn what_is_heard_is_where_the_device_has_got_to() {
    let shared = all_of(vec![block(0, counting(0, 100))]);
    let mut rig = Rig::new(shared);
    rig.play_from(30);
    rig.wait_until_ended();
    assert_eq!(
        rig.sound.hearing(rig.at(0)),
        Hearing::Starting(30 * MILLI),
        "nothing handed yet"
    );
    rig.device.play(40, rig.at(0));
    assert_eq!(rig.sound.hearing(rig.at(0)), Hearing::At(30 * MILLI));
    assert_eq!(rig.sound.hearing(rig.at(25)), Hearing::At(55 * MILLI));
}

#[test]
fn a_gap_between_blocks_plays_as_silence() {
    let shared = all_of(vec![block(0, counting(0, 10)), block(30, counting(30, 10))]);
    let mut rig = Rig::new(shared);
    rig.play_from(0);
    rig.wait_until_ended();
    let mut expected = counting(0, 10);
    expected.extend([0.0; 20]);
    expected.extend(counting(30, 10));
    assert_eq!(rig.listen(40), expected);
}

/// AAC blocks are 1024 frames, which at 44.1 kHz isn't a whole number of
/// nanoseconds: each block's time is rounded down, so one can start a
/// nanosecond after the last ended. That's no gap.
#[test]
fn blocks_a_nanosecond_apart_play_with_nothing_between() {
    let rate = 44_100;
    let blocks: Vec<AudioBlock> = (0..20)
        .map(|index| AudioBlock {
            starts_at: (index * 1024 * 1_000_000_000 / i64::from(rate)),
            sample_rate: rate,
            channels: 1,
            samples: vec![0.5; 1024],
        })
        .collect();
    assert!(
        blocks
            .windows(2)
            .any(|pair| pair[1].starts_at > pair[0].ends_at()),
        "the times leave gaps of a nanosecond"
    );
    let format = Format { rate, channels: 1 };
    let mut rig = Rig::with_format(all_of(blocks), format);
    rig.play_from(0);
    let mut played = Vec::new();
    while played.len() < 20 * 1024 {
        let want = (played.len() as u64 + 4_000).min(20 * 1024);
        rig.wait_until_written(want);
        let frames = want as usize - played.len();
        played.extend(rig.listen(frames));
    }
    rig.wait_until_ended();
    assert_eq!(rig.sound.written(), Some(20 * 1024));
    assert_eq!(played, vec![0.5; 20 * 1024]);
}

#[test]
fn up_to_where_the_sound_is_known_it_plays_silence_without_waiting() {
    // The sound starts at 50 ms: until the worker decodes it, nothing else
    // can be there.
    let shared = queued(Vec::new(), 50 * MILLI);
    let mut rig = Rig::new(shared.clone());
    rig.play_from(0);
    rig.wait_until_written(50);
    assert_eq!(rig.sound.written(), Some(50));
    assert!(!rig.sound.ended());
    {
        let mut queues = shared.lock();
        queues.sound.push_back(block(50, counting(50, 10)));
        queues.sound_known_to = i64::MAX;
    }
    rig.wait_until_ended();
    let mut expected = vec![0.0; 50];
    expected.extend(counting(50, 10));
    assert_eq!(rig.listen(60), expected);
}

/// The sound track can end before the video: with all of it decoded, its
/// end is played to and marked, though the worker is still on pictures.
#[test]
fn sound_shorter_than_the_video_ends_where_it_does() {
    let shared = all_of(vec![block(0, counting(0, 20))]);
    assert!(!shared.lock().done, "the video is still decoding");
    let mut rig = Rig::new(shared);
    rig.play_from(0);
    rig.wait_until_ended();
    assert_eq!(rig.listen(20), counting(0, 20));
    assert_eq!(rig.sound.hearing(rig.at(20)), Hearing::Nothing);
}

#[test]
fn the_ring_is_kept_a_little_ahead_and_no_further() {
    let blocks = (0..20)
        .map(|index| block(index * 100, counting(1 + index as usize * 100, 100)))
        .collect();
    let mut rig = Rig::new(all_of(blocks));
    rig.play_from(0);
    rig.wait_until_written(200);
    // The sound thread stops at a fifth of a second's worth, give or take
    // the block it was converting.
    let written = rig.sound.written().expect("written");
    assert!(written <= 300, "{written}");
    assert!(!rig.sound.ended());
    // As the device plays it, the thread writes on to the end.
    let mut played = Vec::new();
    while played.len() < 2_000 {
        let want = (played.len() + 100).min(2_000) as u64;
        rig.wait_until_written(want);
        let frames = want as usize - played.len();
        played.extend(rig.listen(frames));
    }
    rig.wait_until_ended();
    assert_eq!(played, counting(1, 2_000));
}

#[test]
fn the_ring_is_kept_ahead_of_the_largest_callback() {
    let blocks = (0..20)
        .map(|index| block(index * 100, counting(index as usize * 100, 100)))
        .collect();
    let mut rig = Rig::new(all_of(blocks));
    rig.play_from(0);
    rig.wait_until_written(200);
    // A device that asks for 300 frames at a time (it read the 200 there
    // were): the thread keeps twice that queued.
    rig.device.play(300, rig.at(0));
    rig.wait_until_written(200 + 600);
}

#[test]
fn nothing_plays_while_the_clock_is_stopped() {
    let shared = all_of(vec![block(0, counting(0, 100))]);
    let mut rig = Rig::new(shared);
    wait(|| rig.device.open());
    assert!(!rig.device.playing());
    rig.play_from(0);
    wait(|| rig.device.playing());
    rig.sound.set_segment(None);
    wait(|| !rig.device.playing());
    assert_eq!(rig.sound.hearing(rig.at(0)), Hearing::Nothing);
}

#[test]
fn a_new_segment_throws_away_what_the_last_one_queued() {
    let shared = all_of(vec![block(0, counting(0, 300))]);
    let mut rig = Rig::new(shared);
    rig.play_from(0);
    rig.wait_until_written(200);
    rig.play_from(250);
    rig.wait_until_ended();
    let played = rig.device.play(50, rig.at(0)).expect("playing");
    // Faded in over the first callback.
    assert_eq!(played[49], counting(299, 1)[0]);
    assert!(played[0].abs() < counting(250, 1)[0] / 10.0);
    assert_eq!(rig.sound.hearing(rig.at(10)), Hearing::At(260 * MILLI));
}

#[test]
fn once_its_sound_has_all_been_heard_nothing_is() {
    let shared = all_of(vec![block(0, counting(0, 50))]);
    let mut rig = Rig::new(shared);
    rig.play_from(0);
    rig.wait_until_ended();
    rig.device.play(50, rig.at(0));
    assert_eq!(rig.sound.hearing(rig.at(49)), Hearing::At(49 * MILLI));
    assert_eq!(rig.sound.hearing(rig.at(50)), Hearing::Nothing);
}

#[test]
fn it_waits_for_sound_not_decoded_yet() {
    let shared = queued(vec![block(0, counting(0, 20))], 20 * MILLI);
    let mut rig = Rig::new(shared.clone());
    rig.play_from(0);
    rig.wait_until_written(20);
    assert!(!rig.sound.ended());
    {
        let mut queues = shared.lock();
        queues.sound.push_back(block(20, counting(20, 20)));
        queues.sound_known_to = i64::MAX;
    }
    rig.wait_until_ended();
    rig.device.play(10, rig.at(0));
    assert_eq!(rig.device.play(30, rig.at(10)), Some(counting(10, 30)));
}

#[test]
fn after_a_seek_it_plays_nothing_from_before_it() {
    let shared = queued(vec![block(0, counting(0, 20))], 20 * MILLI);
    let mut rig = Rig::new(shared.clone());
    rig.play_from(0);
    rig.wait_until_written(20);
    {
        // The worker's next blocks are another place's.
        let mut queues = shared.lock();
        queues.generation += 1;
        queues.sound = vec![block(500, counting(500, 20))].into();
        queues.sound_known_to = i64::MAX;
    }
    rig.play_from(500);
    rig.wait_until_ended();
    rig.device.play(10, rig.at(0));
    assert_eq!(rig.device.play(10, rig.at(10)), Some(counting(510, 10)));
    assert_eq!(rig.sound.written(), Some(20));
}

#[test]
fn the_gain_is_applied_even_if_set_before_the_device_opened() {
    let shared = all_of(vec![block(0, vec![0.5; 100])]);
    let mut rig = Rig::new(shared);
    rig.sound.set_gain(0.5);
    rig.play_from(0);
    rig.wait_until_ended();
    // What starts a segment fades in over the first callback…
    let first = rig.device.play(10, rig.at(0)).expect("playing");
    assert_eq!(first[0], 0.025);
    assert_eq!(first[9], 0.25);
    // …and plays at the gain from then on.
    assert_eq!(rig.device.play(10, rig.at(10)), Some(vec![0.25; 10]));
}

#[test]
fn without_a_device_there_is_no_sound_and_it_says_why() {
    let shared = all_of(vec![block(0, counting(0, 100))]);
    let open: OpenDevice = Box::new(|| Err(MediaError::Unsupported("no sound device".into())));
    let mut sound = Sound::start(shared, MONO_1K, open).expect("starts");
    sound.set_segment(Some(Segment { at: 0, speed: 1.0 }));
    wait(|| sound.error().is_some());
    assert_eq!(sound.hearing(Instant::now()), Hearing::Nothing);
}

#[test]
fn a_device_that_fails_ends_the_sound_and_says_why() {
    let shared = all_of(vec![block(0, counting(0, 100))]);
    let mut rig = Rig::new(shared);
    rig.play_from(0);
    wait(|| rig.device.playing());
    rig.device.play(10, rig.at(0));
    rig.device.fail();
    // The thread drops the stream, and the player hears nothing more.
    wait(|| !rig.device.open());
    assert!(rig.sound.error().is_some());
    assert_eq!(rig.sound.hearing(rig.at(5)), Hearing::Nothing);
}

#[test]
fn dropping_it_while_the_device_opens_does_not_wait_for_the_device() {
    let (release, opening) = mpsc::channel::<()>();
    let open: OpenDevice = Box::new(move || {
        let _ = opening.recv();
        Err(MediaError::Unsupported("released".into()))
    });
    let sound = Sound::start(all_of(Vec::new()), MONO_1K, open).expect("starts");
    let (dropped, was_dropped) = mpsc::channel();
    std::thread::spawn(move || {
        drop(sound);
        let _ = dropped.send(());
    });
    was_dropped
        .recv_timeout(HUNG)
        .expect("dropping waited for the device");
    let _ = release.send(());
}

#[test]
fn at_twice_the_speed_a_device_second_is_two_media_seconds() {
    let rate = 8_000;
    let format = Format { rate, channels: 1 };
    let tone: Vec<f32> = (0..rate as usize)
        .map(|frame| (frame as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5)
        .collect();
    let shared = all_of(vec![AudioBlock {
        starts_at: 0,
        sample_rate: rate,
        channels: 1,
        samples: tone,
    }]);
    let mut rig = Rig::with_format(shared, format);
    rig.sound.set_segment(Some(Segment {
        at: 100 * MILLI,
        speed: 2.0,
    }));
    rig.wait_until_written(800);
    rig.device.play(800, rig.at(0));
    // 10 ms of the device's time is 20 ms of the file's.
    assert_eq!(rig.sound.hearing(rig.at(10)), Hearing::At(120 * MILLI));
}
