use super::*;
use crate::test_fixtures::reader;
use crate::{Options, Player};
use std::sync::mpsc;

/// Long enough that only a hang reaches it.
const HUNG: Duration = Duration::from_secs(30);

/// A sink that notes what it was asked, failing when told to.
#[derive(Default)]
struct Sink {
    told: Vec<String>,
    next: SpriteId,
    fail: bool,
}

impl PictureSink for Sink {
    fn create(&mut self, frame: &VideoFrame) -> Result<SpriteId, SpriteError> {
        if self.fail {
            return Err(SpriteError::Empty);
        }
        self.next += 1;
        self.told.push(format!(
            "create {} {}x{}",
            self.next, frame.width, frame.height
        ));
        Ok(self.next)
    }

    fn update(&mut self, sprite: SpriteId, _: &VideoFrame) -> Result<(), SpriteError> {
        self.told.push(format!("update {sprite}"));
        Ok(())
    }

    fn unload(&mut self, sprite: SpriteId) {
        self.told.push(format!("unload {sprite}"));
    }
}

struct Rig {
    source: PlayerSource,
    sink: Sink,
    woken: mpsc::Receiver<()>,
    start: Instant,
}

impl Rig {
    /// `h264_aac.mp4` (320 × 180, a picture every 1/30 s) without sound:
    /// its clock runs on the made-up moments.
    fn new() -> Self {
        let mut player = Player::with_options(
            reader("h264_aac.mp4"),
            Options {
                sound: false,
                ..Options::default()
            },
        )
        .expect("opens");
        let (wake, woken) = mpsc::channel();
        player.set_waker(move || {
            let _ = wake.send(());
        });
        let start = Instant::now();
        Self {
            source: PlayerSource::new(player, start),
            sink: Sink::default(),
            woken,
            start,
        }
    }

    fn at(&self, millis: u64) -> Instant {
        self.start + Duration::from_millis(millis)
    }

    /// Update at `millis` until the sink has been told `count` things,
    /// waiting for the worker between updates.
    fn update_until_told(&mut self, millis: u64, count: usize) {
        let now = self.at(millis);
        loop {
            self.source.update(now, &mut self.sink);
            if self.sink.told.len() >= count {
                return;
            }
            self.woken.recv_timeout(HUNG).expect("the worker hung");
        }
    }
}

#[test]
fn the_first_picture_makes_a_sprite_and_the_next_ones_reuse_it() {
    let mut rig = Rig::new();
    assert_eq!(rig.source.picture(), None);
    rig.update_until_told(0, 1);
    assert_eq!(rig.sink.told, ["create 1 320x180"]);
    assert_eq!(
        rig.source.picture(),
        Some(Picture {
            sprite: 1,
            width: 320,
            height: 180
        })
    );
    assert_eq!(rig.source.state(), PlayState::Paused);
    assert_eq!(
        rig.source.length(),
        Some(rig.source.player().info().duration)
    );

    rig.source.play();
    rig.update_until_told(0, 1);
    // A picture and a bit on.
    rig.update_until_told(40, 2);
    assert_eq!(rig.sink.told, ["create 1 320x180", "update 1"]);

    rig.source.release(&mut rig.sink);
    assert_eq!(rig.sink.told.last().map(String::as_str), Some("unload 1"));
    assert_eq!(rig.source.picture(), None);
}

#[test]
fn what_the_user_does_asks_for_an_update_at_once() {
    let mut rig = Rig::new();
    assert_eq!(
        rig.source.next_change(),
        Some(Duration::ZERO),
        "nothing shown yet"
    );
    rig.update_until_told(0, 1);
    assert_eq!(rig.source.next_change(), None, "paused, with its picture");
    let asks: [fn(&mut PlayerSource); 4] = [
        |s| s.play(),
        |s| s.pause(),
        |s| s.seek(Duration::from_millis(500)),
        |s| s.set_speed(1.5),
    ];
    for ask in asks {
        ask(&mut rig.source);
        assert_eq!(rig.source.next_change(), Some(Duration::ZERO));
        rig.source.update(rig.at(0), &mut rig.sink);
    }
}

#[test]
fn playing_asks_again_by_the_next_picture() {
    let mut rig = Rig::new();
    rig.update_until_told(0, 1);
    rig.source.play();
    rig.source.update(rig.at(0), &mut rig.sink);
    let next = rig.source.next_change().expect("playing changes");
    assert!(next <= Duration::from_secs(1) / 30, "{next:?}");
}

#[test]
fn a_picture_that_cant_be_shown_stops_playback_with_an_error() {
    let mut rig = Rig::new();
    rig.sink.fail = true;
    rig.source.play();
    let now = rig.at(0);
    while rig.source.state() != PlayState::Error {
        rig.source.update(now, &mut rig.sink);
        if rig.source.state() != PlayState::Error {
            rig.woken.recv_timeout(HUNG).expect("the worker hung");
        }
    }
    let error = rig.source.error().expect("says why");
    assert!(error.contains("pictures can't be shown"), "{error}");
    assert_eq!(rig.source.picture(), None);
    assert!(!rig.source.player().state().eq(&PlaybackState::Playing));
    assert_eq!(rig.source.next_change(), None);
}

#[test]
fn the_readout_changes_at_each_whole_second() {
    let cases = [
        (0, 1.0, 1000),
        (250, 1.0, 750),
        (250, 2.0, 375),
        (1999, 1.0, 1),
        (500, 0.5, 1000),
    ];
    for (at, speed, left) in cases {
        assert_eq!(
            until_next_second(Duration::from_millis(at), speed),
            Duration::from_millis(left),
            "{at} ms at {speed}×"
        );
    }
}
