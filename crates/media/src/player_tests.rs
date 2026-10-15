//! The player with its real worker thread, driven by made-up moments. A
//! test waits for the worker through the waker (or by looking at its
//! queues), with a generous limit that only says it hung; nothing here
//! measures how long anything took.

use super::*;
use crate::container::Sample;
use crate::test_fixtures::{Gate, bytes, ffmpeg_frames, gated, planes_md5, reader};
use std::io::Cursor;
use std::sync::mpsc;

/// Long enough that only a hang reaches it.
const HUNG: Duration = Duration::from_secs(30);

/// When picture `index` of `h264_aac.mp4` is shown: 512 ticks of 15360 a
/// picture.
fn picture(index: i64) -> i64 {
    index * 512 * 1_000_000_000 / 15360
}

struct Rig {
    /// Dropped before the player, so a worker held at it is let go.
    gate: Option<Gate>,
    player: Player,
    woken: mpsc::Receiver<()>,
    start: Instant,
}

impl Rig {
    fn new(name: &str) -> Self {
        Self::with(Player::from_reader(reader(name)).expect("opens"))
    }

    fn with(mut player: Player) -> Self {
        let (wake, woken) = mpsc::channel();
        player.set_waker(move || {
            let _ = wake.send(());
        });
        Self {
            gate: None,
            player,
            woken,
            start: Instant::now(),
        }
    }

    /// Fixture `name` read through a [`Gate`].
    fn gated(name: &str, ahead: Ahead) -> Self {
        let (reader, gate) = gated(name);
        let mut rig = Self::with(Player::with_ahead(reader, ahead).expect("opens"));
        rig.gate = Some(gate);
        rig
    }

    fn gate(&self) -> &Gate {
        self.gate.as_ref().expect("a gated rig")
    }

    fn at(&self, millis: u64) -> Instant {
        self.start + Duration::from_millis(millis)
    }

    /// Wait for the worker to say it has something.
    fn wait(&self) {
        self.woken.recv_timeout(HUNG).expect("the worker hung");
    }

    /// Update at `now` until `done` says so, waiting for the worker between
    /// updates that change nothing.
    fn update_until(
        &mut self,
        now: Instant,
        mut done: impl FnMut(&Player, Update) -> bool,
    ) -> Update {
        loop {
            let update = self.player.update(now);
            if done(&self.player, update) {
                return update;
            }
            self.wait();
        }
    }

    /// Wait (by looking) until the worker has `count` pictures queued.
    fn wait_for_queued(&self, count: usize) {
        let limit = Instant::now() + HUNG;
        while self.player.shared.lock().frames.len() < count {
            assert!(Instant::now() < limit, "the worker hung");
            std::thread::yield_now();
        }
    }
}

fn ms(time: Duration) -> u128 {
    time.as_millis()
}

/// Where fixture `name`'s pictures are in the file, in decode order.
fn video_samples(name: &str) -> Vec<Sample> {
    let container = Container::open(reader(name)).expect("opens");
    container.video.expect("video").samples
}

/// Wait (by looking) until `ready` says the queues are as the test needs.
fn wait_for(player: &Player, mut ready: impl FnMut(&worker::Queues) -> bool) {
    let limit = Instant::now() + HUNG;
    while !ready(&player.shared.lock()) {
        assert!(Instant::now() < limit, "the worker hung");
        std::thread::yield_now();
    }
}

#[test]
fn the_file_says_what_it_holds() {
    let rig = Rig::new("h264_aac.mp4");
    assert_eq!(
        rig.player.info(),
        MediaInfo {
            duration: Duration::from_secs(2),
            video_size: Some((320, 180)),
            sound: Some((48000, 2)),
        }
    );
    assert_eq!(rig.player.state(), PlaybackState::Paused);
}

#[test]
fn what_it_cannot_play_is_refused_when_opened() {
    for name in ["hevc.mp4", "high10.mp4", "interlaced.mp4", "mp3_audio.mp4"] {
        match Player::from_reader(reader(name)) {
            Err(MediaError::Unsupported(_)) => {}
            Err(other) => panic!("{name}: {other}"),
            Ok(_) => panic!("{name} opened"),
        }
    }
}

#[test]
fn paused_it_shows_the_first_picture_once_decoded() {
    let mut rig = Rig::new("h264_aac.mp4");
    let now = rig.at(0);
    let update = rig.update_until(now, |player, _| player.frame().is_some());
    assert!(update.new_picture);
    assert_eq!(update.next_at, None, "paused: nothing changes by itself");
    assert_eq!(rig.player.frame().expect("a picture").shown_at, 0);
    assert_eq!(rig.player.state(), PlaybackState::Paused);
}

/// Each update at the moment the last one asked for shows the next
/// picture: every picture, in order, at its own time, none dropped, and
/// then the end.
#[test]
fn playing_shows_every_picture_at_its_time_then_ends() {
    let mut rig = Rig::new("baseline_full_range.mp4");
    let mut now = rig.at(0);
    rig.player.play(now);
    let mut shown = Vec::new();
    loop {
        let update = rig.player.update(now);
        if update.new_picture {
            let frame = rig.player.frame().expect("a picture");
            assert_eq!(rig.player.time(now), frame.shown_at(), "shown on time");
            shown.push(frame.shown_at);
        }
        if rig.player.state() == PlaybackState::Ended {
            break;
        }
        match update.next_at {
            Some(at) => now = at,
            None => rig.wait(),
        }
    }
    let expected: Vec<i64> = (0..25).map(|i| i * 40_000_000).collect();
    assert_eq!(shown, expected);
    assert_eq!(rig.player.dropped_frames(), 0);
    assert_eq!(rig.player.time(now), Duration::from_secs(1));
}

#[test]
fn a_late_update_skips_to_the_picture_due_and_counts_the_rest() {
    let mut rig = Rig::new("h264_aac.mp4");
    let now = rig.at(0);
    rig.update_until(now, |player, _| player.frame().is_some());
    rig.wait_for_queued(8);
    rig.player.play(now);
    // 200 ms on, pictures 1 to 6 are due: 6 is shown, 1 to 5 dropped.
    let update = rig.player.update(rig.at(200));
    assert!(update.new_picture);
    assert_eq!(rig.player.frame().expect("a picture").shown_at, picture(6));
    assert_eq!(rig.player.dropped_frames(), 5);
}

/// Without pictures to show, the clock waits at the end of the one shown
/// (the state says loading) and goes on from there when they come.
#[test]
fn out_of_pictures_the_clock_waits_for_them() {
    let ahead = Ahead {
        frames: 2,
        ..Ahead::default()
    };
    let mut rig = Rig::with(Player::with_ahead(reader("h264_aac.mp4"), ahead).expect("opens"));
    let now = rig.at(0);
    rig.update_until(now, |player, _| player.frame().is_some());
    rig.wait_for_queued(2);
    rig.player.play(now);
    rig.player.update(now);
    // A second on, only pictures 1 and 2 were decoded: 2 is shown, and the
    // clock is held at its end.
    let late = rig.at(1000);
    rig.player.update(late);
    assert_eq!(rig.player.frame().expect("a picture").shown_at, picture(2));
    assert_eq!(rig.player.state(), PlaybackState::Loading);
    assert_eq!(rig.player.time(late).as_nanos(), picture(3) as u128);
    // The worker goes on; the clock runs again from where it waited.
    let update = rig.update_until(late, |player, _| player.state() == PlaybackState::Playing);
    assert_eq!(rig.player.time(late).as_nanos(), picture(3) as u128);
    assert!(update.next_at.is_some());
}

#[test]
fn an_exact_seek_shows_the_picture_at_the_target_as_ffmpeg_decodes_it() {
    let mut rig = Rig::new("h264_aac.mp4");
    let now = rig.at(0);
    rig.update_until(now, |player, _| player.frame().is_some());
    rig.player
        .seek(Duration::from_millis(1250), SeekMode::Exact, now);
    rig.update_until(now, |player, update| {
        update.new_picture && player.frame().is_some()
    });
    // 1.25 s is in picture 37 (1.2333 s to 1.2667 s), decoded from the
    // keyframe at 1 s.
    let frame = rig.player.frame().expect("a picture");
    assert_eq!(frame.shown_at, picture(37));
    let (expected, _) = ffmpeg_frames("h264_aac");
    assert_eq!(planes_md5(frame), expected[37].md5);
    assert_eq!(ms(rig.player.time(now)), 1250);
    assert_eq!(rig.player.state(), PlaybackState::Paused);
}

#[test]
fn a_keyframe_seek_lands_on_the_nearest_keyframe() {
    let mut rig = Rig::new("h264_aac.mp4");
    let now = rig.at(0);
    rig.player
        .seek(Duration::from_millis(1400), SeekMode::NearestKeyframe, now);
    rig.update_until(now, |player, _| {
        player.frame().is_some_and(|frame| frame.shown_at > 0)
    });
    // Keyframes at 1 s and 1.5 s: 1.5 s is nearer.
    assert_eq!(
        rig.player.frame().expect("a picture").shown_at,
        1_500_000_000
    );
    assert_eq!(ms(rig.player.time(now)), 1500);
}

#[test]
fn playing_after_a_seek_waits_at_the_target_for_its_picture() {
    let mut rig = Rig::new("h264_aac.mp4");
    let now = rig.at(0);
    rig.player.play(now);
    rig.player
        .seek(Duration::from_millis(1000), SeekMode::Exact, now);
    assert_eq!(rig.player.state(), PlaybackState::Loading);
    // However late the picture comes, playing starts from the target.
    let later = rig.at(5000);
    rig.update_until(later, |player, _| player.state() == PlaybackState::Playing);
    assert_eq!(ms(rig.player.time(later)), 1000);
    assert_eq!(
        rig.player.frame().expect("a picture").shown_at,
        1_000_000_000
    );
}

#[test]
fn the_sound_queued_after_a_seek_starts_at_the_target() {
    let mut rig = Rig::new("h264_aac.mp4");
    let now = rig.at(0);
    rig.player
        .seek(Duration::from_millis(700), SeekMode::Exact, now);
    rig.update_until(now, |player, _| player.frame().is_some());
    let limit = Instant::now() + HUNG;
    loop {
        if let Some(block) = rig.player.shared.lock().sound.front() {
            assert_eq!(block.starts_at, 700_000_000);
            break;
        }
        assert!(Instant::now() < limit, "the worker hung");
        std::thread::yield_now();
    }
}

#[test]
fn after_the_end_playing_starts_over() {
    let mut rig = Rig::new("baseline_full_range.mp4");
    let now = rig.at(0);
    rig.player
        .seek(Duration::from_millis(900), SeekMode::Exact, now);
    rig.player.play(now);
    let mut now = now;
    rig.update_until(now, |player, _| player.state() == PlaybackState::Playing);
    let end = loop {
        let update = rig.player.update(now);
        if rig.player.state() == PlaybackState::Ended {
            break now;
        }
        match update.next_at {
            Some(at) => now = at,
            None => rig.wait(),
        }
    };
    assert_eq!(rig.player.time(end), Duration::from_secs(1));
    assert_eq!(
        rig.player.time(rig.at(60_000)),
        Duration::from_secs(1),
        "stays at the end"
    );
    rig.player.play(end);
    assert_eq!(rig.player.time(end), Duration::ZERO);
    rig.update_until(end, |player, _| player.state() == PlaybackState::Playing);
    assert_eq!(rig.player.frame().expect("a picture").shown_at, 0);
}

#[test]
fn speed_makes_the_next_picture_due_sooner() {
    let mut rig = Rig::new("baseline_full_range.mp4");
    let now = rig.at(0);
    rig.update_until(now, |player, _| player.frame().is_some());
    rig.wait_for_queued(1);
    rig.player.set_speed(2.0, now);
    rig.player.play(now);
    let update = rig.player.update(now);
    // The next picture is 40 ms on: 20 ms of wall time at double speed.
    assert_eq!(update.next_at, Some(rig.at(20)));
    rig.player.toggle(now);
    assert_eq!(rig.player.state(), PlaybackState::Paused);
}

/// A video sample with its bytes zeroed has no picture in it: the player
/// says why it stopped, instead of the app finding out some other way.
#[test]
fn a_damaged_picture_stops_playback_with_an_error() {
    let mut file = bytes("baseline_full_range.mp4");
    let container = Container::open(reader("baseline_full_range.mp4")).expect("opens");
    let sample = container.video.as_ref().expect("video").samples[5];
    let start = sample.offset as usize;
    file[start..start + sample.size as usize].fill(0);
    let mut rig = Rig::with(Player::from_reader(Box::new(Cursor::new(file))).expect("opens"));
    let mut now = rig.at(0);
    rig.player.play(now);
    loop {
        let update = rig.player.update(now);
        if rig.player.state() == PlaybackState::Error {
            break;
        }
        match update.next_at {
            Some(at) => now = at,
            None => rig.wait(),
        }
    }
    assert!(matches!(rig.player.error(), Some(MediaError::Damaged(_))));
    // The pictures before it played.
    assert_eq!(
        rig.player.frame().expect("a picture").shown_at,
        4 * 40_000_000
    );
    rig.player.play(now);
    assert_eq!(rig.player.state(), PlaybackState::Error, "it stays stopped");
}

/// Decoding too slowly to keep up: once the queued pictures run out, the
/// update asked for at the end of the last one holds the clock there
/// (loading), without the app having to update at that moment by chance,
/// and playing goes on from there once the worker catches up.
#[test]
fn running_out_of_pictures_holds_the_clock_when_the_last_one_ends() {
    let ahead = Ahead {
        frames: 2,
        ..Ahead::default()
    };
    let mut rig = Rig::gated("h264_aac.mp4", ahead);
    let mut now = rig.at(0);
    rig.update_until(now, |player, _| player.frame().is_some());
    rig.wait_for_queued(2);
    // The worker decodes no picture more until the gate lets it.
    let samples = video_samples("h264_aac.mp4");
    rig.gate().hold(samples.iter().map(|sample| sample.offset));
    rig.player.play(now);
    loop {
        let update = rig.player.update(now);
        if rig.player.state() == PlaybackState::Loading {
            break;
        }
        assert_eq!(rig.player.state(), PlaybackState::Playing);
        now = update
            .next_at
            .expect("a picture is up: the update at its end is asked for");
    }
    let held = rig.player.frame().expect("a picture").ends_at();
    assert_eq!(nanos(rig.player.time(now)), held);
    assert_eq!(
        rig.player.update(now).next_at,
        None,
        "waiting for the worker"
    );
    rig.gate().release();
    let later = now + Duration::from_secs(1);
    rig.update_until(later, |player, _| player.state() == PlaybackState::Playing);
    assert_eq!(nanos(rig.player.time(later)), held);
}

/// A seek while the worker decodes for the one before: what it was decoding
/// is thrown away, and the picture shown is the new target's.
#[test]
fn a_seek_while_decoding_for_the_last_one_shows_the_new_target() {
    let mut rig = Rig::gated("h264_aac.mp4", Ahead::default());
    let now = rig.at(0);
    rig.update_until(now, |player, _| player.frame().is_some());
    rig.wait_for_queued(8);
    let samples = video_samples("h264_aac.mp4");
    rig.gate().hold(samples.iter().map(|sample| sample.offset));
    rig.player
        .seek(Duration::from_millis(1000), SeekMode::Exact, now);
    // Held reading the keyframe at 1 s.
    rig.gate().wait_until_holding();
    rig.player
        .seek(Duration::from_millis(500), SeekMode::Exact, now);
    rig.gate().release();
    rig.update_until(now, |player, update| {
        update.new_picture && player.frame().is_some_and(|frame| frame.shown_at != 0)
    });
    assert_eq!(rig.player.frame().expect("a picture").shown_at, picture(15));
    assert_eq!(ms(rig.player.time(now)), 500);
}

/// Decoding failed further on than the player had got: a seek back plays
/// from there, and no error is reported for a place it never reached.
#[test]
fn a_seek_back_from_a_decode_error_ahead_plays_on() {
    let mut file = bytes("baseline_full_range.mp4");
    let sample = video_samples("baseline_full_range.mp4")[20];
    let start = sample.offset as usize;
    file[start..start + sample.size as usize].fill(0);
    let mut rig = Rig::with(Player::from_reader(Box::new(Cursor::new(file))).expect("opens"));
    let now = rig.at(0);
    // From the keyframe at 0.4 s on to the damaged one at 0.8 s.
    rig.player
        .seek(Duration::from_millis(700), SeekMode::Exact, now);
    wait_for(&rig.player, |queues| queues.failed);
    rig.player
        .seek(Duration::from_millis(100), SeekMode::Exact, now);
    rig.update_until(now, |_, update| update.new_picture);
    assert_eq!(
        rig.player.frame().expect("a picture").shown_at,
        2 * 40_000_000
    );
    assert_eq!(rig.player.state(), PlaybackState::Paused);
    assert!(rig.player.error().is_none());
}

/// A panic on the worker (here in the app's waker) stops playback with an
/// error, instead of leaving the player waiting forever.
#[test]
fn a_panic_on_the_worker_stops_playback_with_an_error() {
    let mut player = Player::from_reader(reader("baseline_full_range.mp4")).expect("opens");
    player.set_waker(|| panic!("the app's waker"));
    let now = Instant::now();
    player.play(now);
    let limit = Instant::now() + HUNG;
    while player.state() != PlaybackState::Error {
        assert!(Instant::now() < limit, "no error came");
        player.update(now);
        std::thread::yield_now();
    }
    match player.error() {
        Some(MediaError::Panicked(what)) => assert!(what.contains("the app's waker"), "{what}"),
        other => panic!("{other:?}"),
    }
    player.seek(Duration::ZERO, SeekMode::Exact, now);
    player.play(now);
    assert_eq!(player.state(), PlaybackState::Error, "it stays stopped");
}

/// The sound can be decoded before the picture it goes with (in
/// `late_video.mp4` the pictures start half a second after the sound). The
/// seek is over only once its picture is queued, not on the sound: the
/// clock doesn't run while the picture from before the seek is still up.
#[test]
fn a_seek_waits_for_its_picture_not_just_its_sound() {
    let mut rig = Rig::gated("late_video.mp4", Ahead::default());
    let now = rig.at(0);
    rig.player
        .seek(Duration::from_millis(900), SeekMode::Exact, now);
    rig.update_until(now, |player, _| player.frame().is_some());
    wait_for(&rig.player, |queues| queues.done);
    let samples = video_samples("late_video.mp4");
    assert_eq!(samples[0].shown_at, 500_000_000, "the empty edit first");
    rig.gate().hold(samples.iter().map(|sample| sample.offset));
    rig.player.play(now);
    rig.player
        .seek(Duration::from_millis(200), SeekMode::Exact, now);
    wait_for(&rig.player, |queues| !queues.sound.is_empty());
    rig.player.update(now);
    assert_eq!(rig.player.state(), PlaybackState::Loading);
    assert_eq!(ms(rig.player.time(rig.at(300))), 200, "the clock waits");
    rig.gate().release();
    let update = rig.update_until(now, |player, _| player.state() == PlaybackState::Playing);
    assert_eq!(ms(rig.player.time(now)), 200);
    // Nothing to show until the pictures start: the one from 0.9 s is gone.
    assert!(rig.player.frame().is_none());
    assert_eq!(
        update.next_at,
        Some(rig.at(300)),
        "the first picture's time"
    );
}

/// Once every picture is decoded, up to `Ahead`'s worth are still queued:
/// they are all shown even when the sound runs out first (here held back
/// at the gate), and only then does the player wait for sound.
#[test]
fn the_pictures_queued_at_the_end_are_shown_before_waiting_on_sound() {
    let ahead = Ahead {
        frames: 64,
        ..Ahead::default()
    };
    let mut rig = Rig::gated("h264_aac.mp4", ahead);
    let container = Container::open(reader("h264_aac.mp4")).expect("opens");
    let video = container.video.expect("video").samples;
    let last_decoded = video.iter().map(|sample| sample.decode_at).max();
    let after_the_video: Vec<u64> = container
        .audio
        .expect("sound")
        .samples
        .iter()
        .filter(|sample| Some(sample.decode_at) >= last_decoded)
        .map(|sample| sample.offset)
        .collect();
    assert!(!after_the_video.is_empty(), "sound decoded after the video");
    rig.gate().hold(after_the_video);
    let mut now = rig.at(0);
    rig.player.play(now);
    loop {
        let update = rig.player.update(now);
        if let Some(at) = update.next_at {
            now = at;
        } else if rig.player.frame().map(|frame| frame.shown_at) == Some(picture(59)) {
            break;
        } else {
            rig.wait();
        }
    }
    assert_eq!(
        rig.player.state(),
        PlaybackState::Loading,
        "waiting on sound"
    );
    assert_eq!(rig.player.time(now), Duration::from_secs(2));
    assert_eq!(rig.player.dropped_frames(), 0);
    rig.gate().release();
    rig.update_until(now, |player, _| player.state() == PlaybackState::Ended);
}

/// Plays `rig` from where it is until it ends, following the moments each
/// update asks for; returns the moment it ended and how many of those
/// moments it asked for (not counting waits for the worker).
fn play_to_the_end(rig: &mut Rig) -> (Instant, usize) {
    let mut now = rig.at(0);
    rig.player.play(now);
    let mut asked = 0;
    loop {
        let update = rig.player.update(now);
        if rig.player.state() == PlaybackState::Ended {
            return (now, asked);
        }
        match update.next_at {
            Some(at) => {
                now = at;
                asked += 1;
            }
            None => rig.wait(),
        }
    }
}

/// Sound alone plays to its end; without pictures, the player lets the
/// sound go block by block, so the worker decodes on.
#[test]
fn sound_alone_plays_to_the_end() {
    let mut rig = Rig::new("aac_only.m4a");
    assert_eq!(rig.player.info().video_size, None);
    let blocks = Container::open(reader("aac_only.m4a"))
        .expect("opens")
        .audio
        .expect("sound")
        .samples
        .len();
    let (end, asked) = play_to_the_end(&mut rig);
    assert_eq!(rig.player.time(end), rig.player.info().duration);
    assert_eq!(ms(rig.player.time(end)), 1000);
    // Each update asked for lets a block go, or ends: no spinning while the
    // worker catches up.
    assert!(asked <= blocks + 2, "{asked} updates for {blocks} blocks");
}

/// After the last picture the sound plays on to the end, and the last
/// picture stays up.
#[test]
fn sound_plays_on_after_the_last_picture() {
    let mut rig = Rig::new("late_video.mp4");
    let last = video_samples("late_video.mp4")
        .iter()
        .map(|sample| sample.shown_at)
        .max();
    let (end, _) = play_to_the_end(&mut rig);
    assert_eq!(ms(rig.player.time(end)), 1500);
    assert_eq!(rig.player.frame().map(|frame| frame.shown_at), last);
    assert_eq!(rig.player.dropped_frames(), 0);
}

/// Paused at the end (a seek there) is ended too, and playing starts over.
#[test]
fn a_seek_to_the_end_is_ended_and_playing_starts_over() {
    let mut rig = Rig::new("baseline_full_range.mp4");
    let now = rig.at(0);
    rig.player
        .seek(Duration::from_secs(1), SeekMode::Exact, now);
    rig.update_until(now, |player, _| player.state() == PlaybackState::Ended);
    rig.player.play(now);
    assert_eq!(rig.player.time(now), Duration::ZERO);
}

#[test]
fn dropping_a_player_stops_its_worker() {
    let rig = Rig::new("h264_aac.mp4");
    rig.wait_for_queued(8);
    // The worker is waiting for room; dropping must not hang on it.
    drop(rig);
}
