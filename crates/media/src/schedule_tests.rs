use super::*;
use wgpu_gameui::{YuvColour, YuvPlanes};

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

fn queue(times: &[i64]) -> VecDeque<VideoFrame> {
    times.iter().map(|&at| frame(at)).collect()
}

fn times(queue: &VecDeque<VideoFrame>) -> Vec<i64> {
    queue.iter().map(|frame| frame.shown_at).collect()
}

#[test]
fn nothing_due_keeps_the_picture() {
    let mut queued = queue(&[10, 20]);
    let mut shown = Some(frame(0));
    assert_eq!(advance(&mut queued, &mut shown, 9), (0, false));
    assert_eq!(shown.map(|f| f.shown_at), Some(0));
    assert_eq!(times(&queued), [10, 20]);
}

#[test]
fn the_picture_due_is_shown_from_its_time() {
    let mut queued = queue(&[10, 20]);
    let mut shown = None;
    assert_eq!(advance(&mut queued, &mut shown, 10), (0, true));
    assert_eq!(shown.map(|f| f.shown_at), Some(10));
    assert_eq!(times(&queued), [20]);
}

#[test]
fn pictures_passed_over_are_dropped_and_counted() {
    let mut queued = queue(&[10, 20, 30, 40]);
    let mut shown = Some(frame(0));
    assert_eq!(advance(&mut queued, &mut shown, 35), (2, true));
    assert_eq!(shown.map(|f| f.shown_at), Some(30));
    assert_eq!(times(&queued), [40]);
}

fn sample(shown_at: i64, keyframe: bool) -> Sample {
    Sample {
        offset: 0,
        size: 0,
        decode_at: 0,
        shown_at,
        duration: 10,
        keyframe,
    }
}

/// Keyframes every third picture, at 0, 30 and 60, with the B picture
/// order of a real stream (I P B …) so decode order isn't display order.
fn video() -> Vec<Sample> {
    [
        (0, true),
        (20, false),
        (10, false),
        (30, true),
        (50, false),
        (40, false),
        (60, true),
        (80, false),
        (70, false),
    ]
    .iter()
    .map(|&(at, key)| sample(at, key))
    .collect()
}

fn sound() -> Vec<Sample> {
    (0..10).map(|i| sample(i * 10 - 10, true)).collect()
}

fn seek(target: i64, mode: SeekMode) -> SeekPlan {
    let video = video();
    plan_seek(&video, &keyframes(&video), &sound(), target, mode)
}

#[test]
fn keyframes_are_listed_in_decode_order() {
    assert_eq!(keyframes(&video()), [0, 3, 6]);
}

#[test]
fn an_exact_seek_decodes_from_the_keyframe_before_and_lands_where_asked() {
    let plan = seek(55, SeekMode::Exact);
    assert_eq!(plan.video_from, 3, "the keyframe at 30");
    assert_eq!(plan.target, 55);
    // The block at 50 is heard first; the one at 40 before it rebuilds it.
    assert_eq!(sound()[plan.sound_from].shown_at, 40);
}

#[test]
fn an_exact_seek_onto_a_keyframe_starts_there() {
    let plan = seek(60, SeekMode::Exact);
    assert_eq!(plan.video_from, 6);
}

#[test]
fn a_seek_before_the_first_picture_starts_at_the_first_keyframe() {
    let plan = seek(-5, SeekMode::Exact);
    assert_eq!(plan.video_from, 0);
    assert_eq!(plan.sound_from, 0);
}

#[test]
fn a_keyframe_seek_lands_on_the_nearest_keyframe_either_side() {
    let plan = seek(52, SeekMode::NearestKeyframe);
    assert_eq!(
        (plan.video_from, plan.target),
        (6, 60),
        "60 is nearer than 30"
    );
    let plan = seek(41, SeekMode::NearestKeyframe);
    assert_eq!((plan.video_from, plan.target), (3, 30));
    let plan = seek(-5, SeekMode::NearestKeyframe);
    assert_eq!((plan.video_from, plan.target), (0, 0));
    let plan = seek(500, SeekMode::NearestKeyframe);
    assert_eq!((plan.video_from, plan.target), (6, 60));
    let plan = seek(41, SeekMode::NearestKeyframe);
    assert_eq!((plan.video_from, plan.target), (3, 30));
    assert_eq!(sound()[plan.sound_from].shown_at, 20);
}

#[test]
fn sound_alone_seeks_where_asked() {
    let plan = plan_seek(&[], &[], &sound(), 35, SeekMode::NearestKeyframe);
    assert_eq!(plan.target, 35);
    assert_eq!(sound()[plan.sound_from].shown_at, 20);
}
