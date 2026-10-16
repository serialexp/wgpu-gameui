use super::*;
use wgpu_gameui::{YuvColour, YuvPlanes};

const SECOND: i64 = 1_000_000_000;

/// Queues after a seek to 0, with the sound decoded to its end.
fn queues() -> Queues {
    let mut queues = Shared::new(0).queues.into_inner().expect("not poisoned");
    queues.seek = None;
    queues.sound_known_to = i64::MAX;
    queues
}

fn frame(shown_at: i64) -> VideoFrame {
    VideoFrame {
        shown_at,
        duration: SECOND / 30,
        width: 2,
        height: 2,
        planes: YuvPlanes::packed(2, vec![0; 4], vec![0; 1], vec![0; 1]),
        colour: YuvColour::assumed_for(2, 2),
    }
}

/// Blocks of sound covering `from` to `to` seconds, a tenth of a second
/// each.
fn sound(queues: &mut Queues, from: i64, to: i64) {
    for tenth in from * 10..to * 10 {
        queues.sound.push_back(AudioBlock {
            starts_at: tenth * SECOND / 10,
            sample_rate: 10,
            channels: 1,
            samples: vec![0.0],
        });
    }
}

#[test]
fn room_for_pictures_and_sound_means_decode_on() {
    let mut queues = queues();
    queues.frames.push_back(frame(0));
    assert!(!queues.full(&Ahead::default(), true));
}

#[test]
fn a_full_picture_queue_waits() {
    let mut queues = queues();
    for index in 0..8 {
        queues.frames.push_back(frame(index));
    }
    assert!(queues.full(&Ahead::default(), true));
}

/// The clock may be waiting for that sound, and the worker reaches it
/// through the pictures in decode order.
#[test]
fn pictures_decode_past_their_bound_while_the_sound_is_short() {
    let mut queues = queues();
    for index in 0..8 {
        queues.frames.push_back(frame(index));
    }
    queues.played_to = SECOND;
    queues.sound_known_to = SECOND + SECOND / 10;
    assert!(!queues.full(&Ahead::default(), true));
    queues.sound_known_to = 2 * SECOND;
    assert!(queues.full(&Ahead::default(), true));
}

#[test]
fn with_no_picture_queued_and_more_to_come_sound_does_not_hold_the_worker() {
    let mut queues = queues();
    sound(&mut queues, 0, 2);
    assert!(!queues.full(&Ahead::default(), true));
}

/// Once every picture is queued (and shown), nothing more is coming that
/// the player could be waiting for, so the sound bound holds: a long sound
/// track after a short video isn't decoded into memory whole.
#[test]
fn after_the_last_picture_the_sound_bound_holds() {
    let mut queues = queues();
    sound(&mut queues, 0, 2);
    queues.video_done = true;
    assert!(queues.full(&Ahead::default(), true));
}

#[test]
fn sound_alone_waits_at_its_bound() {
    let mut queues = queues();
    sound(&mut queues, 0, 1);
    assert!(queues.full(&Ahead::default(), false));
    queues.sound.pop_front();
    assert!(!queues.full(&Ahead::default(), false));
}

#[test]
fn a_failure_waits_for_a_seek_even_once_its_error_is_taken() {
    let mut queues = queues();
    queues.fail(MediaError::Damaged("test".into()));
    assert!(queues.full(&Ahead::default(), true));
    queues.error.take();
    assert!(queues.full(&Ahead::default(), true));
}
