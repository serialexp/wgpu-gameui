use super::*;

fn mixed(input: &[f32], from: u16, to: u16) -> Vec<f32> {
    let mut out = Vec::new();
    mix(input, from, to, &mut out);
    out
}

#[test]
fn the_same_channels_pass_through() {
    let stereo = [0.1, -0.2, 0.3, -0.4];
    assert_eq!(mixed(&stereo, 2, 2), stereo);
}

#[test]
fn mono_goes_to_every_channel() {
    assert_eq!(mixed(&[0.5, -0.25], 1, 2), [0.5, 0.5, -0.25, -0.25]);
    assert_eq!(mixed(&[0.5], 1, 4), [0.5; 4]);
}

#[test]
fn stereo_to_mono_averages_the_two() {
    assert_eq!(mixed(&[0.5, 0.25, -1.0, 1.0], 2, 1), [0.375, 0.0]);
}

#[test]
fn stereo_on_more_speakers_plays_on_the_front_two_and_leaves_the_rest_silent() {
    assert_eq!(mixed(&[0.5, -0.5], 2, 6), [0.5, -0.5, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn appends_to_what_the_buffer_holds() {
    let mut out = vec![9.0];
    mix(&[0.5], 1, 2, &mut out);
    assert_eq!(out, [9.0, 0.5, 0.5]);
}

#[test]
fn a_part_frame_at_the_end_is_left_out() {
    assert_eq!(mixed(&[0.1, 0.2, 0.3], 2, 2), [0.1, 0.2]);
    assert_eq!(mixed(&[0.5, 0.25, 0.75], 2, 1), [0.375]);
}
