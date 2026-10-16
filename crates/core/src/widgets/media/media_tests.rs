use super::*;

#[test]
fn times_read_as_the_players_show_them() {
    let cases = [
        (0, "0:00"),
        (9, "0:09"),
        (61, "1:01"),
        (599, "9:59"),
        (3599, "59:59"),
        (3600, "1:00:00"),
        (3661, "1:01:01"),
        (36_000, "10:00:00"),
    ];
    for (seconds, shown) in cases {
        assert_eq!(format_time(Duration::from_secs(seconds)), shown);
    }
    // Rounded down, as a clock reads.
    assert_eq!(format_time(Duration::from_millis(59_999)), "0:59");
}

#[test]
fn speeds_read_without_trailing_zeros() {
    assert_eq!(format_speed(0.5), "0.5×");
    assert_eq!(format_speed(1.0), "1×");
    assert_eq!(format_speed(1.5), "1.5×");
    assert_eq!(format_speed(2.0), "2×");
    assert_eq!(format_speed(1.2499999), "1.25×");
}

#[test]
fn the_speed_key_steps_round_and_starts_a_stray_speed_at_one() {
    assert_eq!(next_speed(0.5), 1.0);
    assert_eq!(next_speed(1.0), 1.5);
    assert_eq!(next_speed(1.5), 2.0);
    assert_eq!(next_speed(2.0), 0.5);
    assert_eq!(next_speed(3.0), 1.0);
}

/// A source that notes what it is told, and does it.
pub(super) struct Fake {
    pub state: PlayState,
    pub time: Duration,
    pub length: Option<Duration>,
    pub buffered: Option<Duration>,
    pub volume: f32,
    pub muted: bool,
    pub speed: f64,
    pub error: Option<String>,
    pub picture: Option<Picture>,
    pub next_change: Option<Duration>,
    pub told: Vec<String>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            state: PlayState::Paused,
            time: Duration::ZERO,
            length: Some(Duration::from_secs(60)),
            buffered: None,
            volume: 1.0,
            muted: false,
            speed: 1.0,
            error: None,
            picture: None,
            next_change: None,
            told: Vec::new(),
        }
    }
}

impl Fake {
    /// What it was told since last asked.
    #[cfg(feature = "phosphor-icons")]
    pub fn told(&mut self) -> Vec<String> {
        std::mem::take(&mut self.told)
    }
}

impl MediaSource for Fake {
    fn state(&self) -> PlayState {
        self.state
    }
    fn time(&self) -> Duration {
        self.time
    }
    fn length(&self) -> Option<Duration> {
        self.length
    }
    fn buffered(&self) -> Option<Duration> {
        self.buffered
    }
    fn volume(&self) -> f32 {
        self.volume
    }
    fn muted(&self) -> bool {
        self.muted
    }
    fn speed(&self) -> f64 {
        self.speed
    }
    fn error(&self) -> Option<String> {
        self.error.clone()
    }
    fn picture(&self) -> Option<Picture> {
        self.picture
    }
    fn next_change(&self) -> Option<Duration> {
        self.next_change
    }
    fn play(&mut self) {
        self.state = PlayState::Playing;
        self.told.push("play".into());
    }
    fn pause(&mut self) {
        self.state = PlayState::Paused;
        self.told.push("pause".into());
    }
    fn seek(&mut self, to: Duration) {
        self.time = to;
        self.told.push(format!("seek {}", to.as_secs_f64()));
    }
    fn scrub(&mut self, to: Duration) {
        self.time = to;
        self.told.push(format!("scrub {}", to.as_secs_f64()));
    }
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.told.push(format!("volume {volume}"));
    }
    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.told.push(format!("muted {muted}"));
    }
    fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
        self.told.push(format!("speed {speed}"));
    }
}

#[test]
fn toggling_plays_pauses_replays_and_leaves_an_error_be() {
    let cases = [
        (PlayState::Paused, vec!["play"]),
        (PlayState::Playing, vec!["pause"]),
        (PlayState::Loading, vec!["pause"]),
        (PlayState::Ended, vec!["seek 0", "play"]),
        (PlayState::Error, vec![]),
    ];
    for (state, told) in cases {
        let mut source = Fake {
            state,
            ..Fake::default()
        };
        toggle(&mut source);
        assert_eq!(source.told, told, "{state:?}");
    }
}
