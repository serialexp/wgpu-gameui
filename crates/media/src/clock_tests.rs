use super::*;

const MS: i64 = 1_000_000;

fn ms(count: u64) -> Duration {
    Duration::from_millis(count)
}

#[test]
fn a_stopped_clock_stays_where_it_is() {
    let start = Instant::now();
    let clock = Clock::stopped_at(5 * MS);
    assert!(!clock.is_running());
    assert_eq!(clock.time(start + ms(1000)), 5 * MS);
    assert_eq!(clock.when(10 * MS, start), None);
}

#[test]
fn a_started_clock_runs_with_the_wall_from_where_it_was() {
    let start = Instant::now();
    let mut clock = Clock::stopped_at(5 * MS);
    clock.start(start);
    assert_eq!(clock.time(start + ms(20)), 25 * MS);
    clock.stop(start + ms(20));
    assert_eq!(clock.time(start + ms(500)), 25 * MS);
    clock.start(start + ms(500));
    assert_eq!(clock.time(start + ms(510)), 35 * MS);
}

#[test]
fn starting_a_running_clock_again_changes_nothing() {
    let start = Instant::now();
    let mut clock = Clock::stopped_at(0);
    clock.start(start);
    clock.start(start + ms(30));
    assert_eq!(clock.time(start + ms(40)), 40 * MS);
}

#[test]
fn setting_jumps_and_a_running_clock_runs_on_from_there() {
    let start = Instant::now();
    let mut clock = Clock::stopped_at(0);
    clock.start(start);
    clock.set(1000 * MS, start + ms(50));
    assert_eq!(clock.time(start + ms(60)), 1010 * MS);
    clock.stop(start + ms(60));
    clock.set(3 * MS, start + ms(70));
    assert_eq!(clock.time(start + ms(900)), 3 * MS);
}

#[test]
fn speed_scales_what_runs_from_the_change_on() {
    let start = Instant::now();
    let mut clock = Clock::stopped_at(0);
    clock.start(start);
    clock.set_speed(2.0, start + ms(10));
    assert_eq!(clock.time(start + ms(20)), 30 * MS);
    clock.set_speed(0.5, start + ms(20));
    assert_eq!(clock.time(start + ms(40)), 40 * MS);
    assert_eq!(clock.speed(), 0.5);
}

#[test]
fn when_says_the_moment_a_time_comes_at_the_speed() {
    let start = Instant::now();
    let mut clock = Clock::stopped_at(0);
    clock.start(start);
    assert_eq!(clock.when(40 * MS, start), Some(start + ms(40)));
    clock.set_speed(2.0, start);
    assert_eq!(clock.when(40 * MS, start), Some(start + ms(20)));
    assert_eq!(clock.when(0, start + ms(5)), None, "already past");
    // The moment given has the time reached, never a nanosecond short.
    let due = clock.when(MS + 1, start).expect("ahead");
    assert!(clock.time(due) > MS);
}
