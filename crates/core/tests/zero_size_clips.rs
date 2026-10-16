//! A clip of zero size holds nothing. Nested clips that don't overlap leave a
//! zero-size clip at the inner rect's corner (`DrawList::push_clip`); the
//! shaders keep a pixel exactly on a clip's edge, so where that corner landed
//! on a pixel centre one pixel still drew, outside the outer clip.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test zero_size_clips -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, HeadlessGpu};

const SIZE: (u32, u32) = (64, 64);
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// The kinds of draw that each clip in a shader of its own, as a small shape
/// around `corner`. Small, so the clip position the shader interpolates at the
/// pixel centre comes out exactly on the corner (across a whole-target quad it
/// lands a hair off, and the pixel is discarded anyway).
const KINDS: [&str; 3] = ["soup", "chrome", "circle"];

fn draw(list: &mut DrawList, kind: &str, corner: (f32, f32)) {
    let (x, y) = corner;
    match kind {
        // `quad` would record chrome here: soup is what a triangle records.
        "soup" => list.triangle(
            (x - 1.0, y - 1.0),
            (x + 2.0, y - 1.0),
            (x - 1.0, y + 2.0),
            WHITE,
        ),
        "chrome" => list.chrome_rect(
            Rect::new(x - 1.0, y - 1.0, 2.0, 2.0),
            0.0,
            0.0,
            WHITE,
            WHITE,
        ),
        _ => list.circle((x, y), 2.0, WHITE),
    }
}

fn drawn(pixels: &[u8]) -> Vec<usize> {
    pixels
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| p[3] != 0)
        .map(|(i, _)| i)
        .collect()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn nothing_draws_inside_a_clip_that_misses_its_parent() {
    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    // Each corner lands on a pixel centre: x.5 at scale 1, and 2.0 at 1.25,
    // whose pixel centre 2.5 is logical 2.5 / 1.25 = 2.0.
    let cases = [(1.0, (2.5, 2.5)), (1.0, (32.5, 16.5)), (1.25, (2.0, 2.0))];
    let mut leaks = Vec::new();
    for (scale, corner) in cases {
        for kind in KINDS {
            let mut list = gpu.draw_list();
            list.push_clip(Rect::new(50.0, 50.0, 4.0, 4.0));
            list.push_clip(Rect::new(corner.0, corner.1, 10.0, 10.0));
            draw(&mut list, kind, corner);
            list.pop_clip();
            list.pop_clip();
            let drawn = drawn(&gpu.capture_scaled(&list, SIZE, scale));
            if !drawn.is_empty() {
                leaks.push(format!(
                    "{kind} at {corner:?}, scale {scale}: {} pixels",
                    drawn.len()
                ));
            }
        }
    }
    assert!(leaks.is_empty(), "drew inside the empty clip: {leaks:#?}");
}

/// A clip that holds a pixel still draws it: the control for the test above.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_clip_of_one_pixel_still_draws_it() {
    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    for kind in KINDS {
        let mut list = gpu.draw_list();
        list.push_clip(Rect::new(2.0, 2.0, 1.0, 1.0));
        draw(&mut list, kind, (2.5, 2.5));
        list.pop_clip();
        let drawn = drawn(&gpu.capture(&list, SIZE));
        assert_eq!(drawn, vec![2 * SIZE.0 as usize + 2], "{kind}");
    }
}
