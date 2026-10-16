use super::*;
use std::time::Duration;

const STEREO: u16 = 2;
const RATE: u32 = 1_000;

/// What a callback asking for `frames` stereo frames heard at `at` gets.
fn fill(reader: &mut Reader, frames: usize, at: Instant) -> Vec<f32> {
    let mut out = vec![f32::NAN; frames * usize::from(STEREO)];
    reader.fill(&mut out, at);
    out
}

/// `frames` stereo frames counting up from `from`: 1, 1, 2, 2, …
fn counting(from: usize, frames: usize) -> Vec<f32> {
    (from..from + frames)
        .flat_map(|frame| [frame as f32; 2])
        .collect()
}

#[test]
fn what_is_written_is_played_in_order() {
    let (mut writer, mut reader) = ring(STEREO, 8, 1.0);
    assert_eq!(writer.write(&counting(1, 3)), 6);
    let now = Instant::now();
    assert_eq!(fill(&mut reader, 2, now), counting(1, 2));
    assert_eq!(fill(&mut reader, 1, now), counting(3, 1));
    assert_eq!(writer.gauge().read(), 3);
}

#[test]
fn an_empty_ring_plays_silence_and_reads_nothing() {
    let (mut writer, mut reader) = ring(STEREO, 8, 1.0);
    writer.write(&counting(1, 1));
    let out = fill(&mut reader, 3, Instant::now());
    assert_eq!(out, [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(writer.gauge().read(), 1);
}

#[test]
fn a_full_ring_takes_whole_frames_only() {
    let (mut writer, _reader) = ring(STEREO, 4, 1.0);
    assert_eq!(writer.write(&counting(1, 3)), 6);
    // One frame of room: of three offered, one is taken.
    assert_eq!(writer.room(), 1);
    assert_eq!(writer.write(&counting(4, 3)), 2);
    assert_eq!(writer.write(&counting(5, 1)), 0);
    assert_eq!(writer.written(), 4);
}

#[test]
fn a_part_frame_is_not_taken() {
    let (mut writer, _reader) = ring(STEREO, 4, 1.0);
    assert_eq!(writer.write(&[0.5, 0.5, 0.25]), 2);
    assert_eq!(writer.written(), 1);
}

/// Once the sound runs out, the device goes on asking: its silence doesn't
/// move when the last of the sound is heard.
#[test]
fn silence_after_the_sound_leaves_when_it_is_heard_as_it_was() {
    let (mut writer, mut reader) = ring(STEREO, 64, 1.0);
    writer.write(&counting(0, 10));
    let at = Instant::now();
    fill(&mut reader, 10, at);
    fill(&mut reader, 10, at + Duration::from_millis(10));
    let heard = writer.gauge().heard(at + Duration::from_millis(12), RATE);
    assert_eq!(heard.map(|heard| heard.frames), Some(10.0));
}

#[test]
fn a_flush_throws_away_what_was_written_before_it() {
    let (mut writer, mut reader) = ring(STEREO, 8, 1.0);
    writer.write(&counting(1, 4));
    let now = Instant::now();
    assert_eq!(fill(&mut reader, 1, now), counting(1, 1));
    // Three frames still queued are thrown away; what comes after plays.
    assert_eq!(writer.flush(), 4);
    assert_eq!(writer.queued(), 0);
    writer.write(&counting(10, 2));
    assert_eq!(writer.queued(), 2);
    // Faded in, over the callback.
    assert_eq!(fill(&mut reader, 2, now), [5.0, 5.0, 11.0, 11.0]);
    assert_eq!(writer.gauge().read(), 6);
}

#[test]
fn a_flush_frees_its_room_once_the_device_reads() {
    let (mut writer, mut reader) = ring(STEREO, 4, 1.0);
    writer.write(&counting(1, 4));
    writer.flush();
    assert_eq!(writer.room(), 0);
    // Nothing is played from before the flush, even with nothing after it.
    assert_eq!(fill(&mut reader, 1, Instant::now()), [0.0, 0.0]);
    assert_eq!(writer.room(), 4);
}

#[test]
fn a_new_gain_is_ramped_over_one_callback() {
    let (mut writer, mut reader) = ring(STEREO, 16, 1.0);
    writer.write(&[1.0; 16]);
    writer.gauge().set_gain(0.0);
    let now = Instant::now();
    // From the old gain (1) to the new over the four frames, frame by frame.
    assert_eq!(
        fill(&mut reader, 4, now),
        [0.75, 0.75, 0.5, 0.5, 0.25, 0.25, 0.0, 0.0]
    );
    assert_eq!(fill(&mut reader, 2, now), [0.0; 4]);
    writer.gauge().set_gain(0.5);
    assert_eq!(fill(&mut reader, 2, now), [0.25, 0.25, 0.5, 0.5]);
}

#[test]
fn what_follows_a_flush_fades_in_once_it_plays() {
    let (mut writer, mut reader) = ring(STEREO, 16, 1.0);
    writer.flush();
    let now = Instant::now();
    // Silence for want of sound doesn't use the fade up…
    assert_eq!(fill(&mut reader, 2, now), [0.0; 4]);
    writer.write(&[1.0; 8]);
    // …the sound, when it comes, does.
    assert_eq!(
        fill(&mut reader, 4, now),
        [0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0]
    );
}

#[test]
fn nothing_is_heard_before_the_device_has_read() {
    let (writer, _reader) = ring(STEREO, 8, 1.0);
    assert_eq!(writer.gauge().heard(Instant::now(), RATE), None);
}

#[test]
fn what_is_heard_moves_on_from_the_last_callback_at_the_rate() {
    let (mut writer, mut reader) = ring(STEREO, 64, 1.0);
    writer.write(&counting(0, 40));
    let at = Instant::now();
    fill(&mut reader, 10, at);
    let gauge = writer.gauge();
    let heard = |millis: u64| gauge.heard(at + Duration::from_millis(millis), RATE);
    // A frame a millisecond at 1 kHz, from where this callback's first
    // frame is heard.
    assert_eq!(
        heard(0),
        Some(Heard {
            frames: 0.0,
            handed: 10
        })
    );
    assert_eq!(heard(4).map(|heard| heard.frames), Some(4.0));
    // No further than the device has been handed.
    assert_eq!(heard(25).map(|heard| heard.frames), Some(10.0));
    fill(&mut reader, 10, at + Duration::from_millis(10));
    assert_eq!(heard(15).map(|heard| heard.frames), Some(15.0));
}

#[test]
fn silence_played_for_want_of_sound_is_not_heard() {
    let (mut writer, mut reader) = ring(STEREO, 64, 1.0);
    writer.write(&counting(0, 3));
    let at = Instant::now();
    fill(&mut reader, 10, at);
    let heard = writer.gauge().heard(at + Duration::from_millis(8), RATE);
    assert_eq!(
        heard,
        Some(Heard {
            frames: 3.0,
            handed: 3
        })
    );
}

#[test]
fn what_is_heard_counts_from_after_a_flush() {
    let (mut writer, mut reader) = ring(STEREO, 64, 1.0);
    writer.write(&counting(0, 20));
    let at = Instant::now();
    fill(&mut reader, 5, at);
    assert_eq!(writer.flush(), 20);
    writer.write(&counting(0, 10));
    fill(&mut reader, 5, at + Duration::from_millis(5));
    let heard = writer
        .gauge()
        .heard(at + Duration::from_millis(7), RATE)
        .expect("heard");
    assert_eq!(
        heard,
        Heard {
            frames: 22.0,
            handed: 25
        }
    );
}

/// The writer and the callback on their own threads, the writer flushing
/// now and then: the callback never plays a torn frame, never reads more
/// than was written, and reads everything once the writer stops.
#[test]
fn a_writer_and_a_callback_on_their_own_threads_agree() {
    let (mut writer, mut reader) = ring(STEREO, 64, 1.0);
    let gauge = writer.gauge().clone();
    let writing = std::thread::spawn(move || {
        let mut next = 1;
        for round in 0..20_000usize {
            let frames = 1 + round % 7;
            let samples = counting(next, frames);
            let taken = writer.write(&samples) / usize::from(STEREO);
            next += taken;
            if round.is_multiple_of(997) {
                writer.flush();
            }
            if taken == 0 {
                std::thread::yield_now();
            }
        }
        writer.written()
    });
    let reading = std::thread::spawn(move || {
        let now = Instant::now();
        let mut frames = 0u64;
        loop {
            let mut out = vec![0.0; 2 * (1 + frames as usize % 5)];
            reader.fill(&mut out, now);
            for frame in out.chunks(2) {
                assert_eq!(frame[0], frame[1], "a torn frame");
            }
            frames += 1;
            if frames.is_multiple_of(64) {
                std::thread::yield_now();
            }
            if frames > 200_000 {
                return reader;
            }
        }
    });
    let written = writing.join().expect("the writer");
    let mut reader = reading.join().expect("the reader");
    assert!(gauge.read() <= written);
    // What's left plays out, and the counts meet.
    let mut out = vec![0.0; 2 * 64];
    reader.fill(&mut out, Instant::now());
    assert_eq!(gauge.read(), written);
}
