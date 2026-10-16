//! Text projects with the same logical size as the geometry pipelines. A
//! target whose size doesn't divide by the scale factor has a fractional
//! logical size; text used to round it down, which drew it up to about a
//! physical pixel further right and down than chrome near the far edges.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test text_projection -- --ignored
//! ```

use wgpu_gameui::color::Srgb;
use wgpu_gameui::{DrawList, HeadlessGpu, TextBlock};

const SCALE: f32 = 1.25;
/// 1000 / 1.25 = 800 logical px exactly; 1001 / 1.25 = 800.8.
const EVEN: (u32, u32) = (1000, 600);
const ODD: (u32, u32) = (1001, 601);

fn scene(list: &mut DrawList) {
    // Near the far corner, where rounding the logical size moved text most.
    list.text(
        TextBlock::new("Il|Il", 700.0, 420.0)
            .with_size(24.0)
            .with_color(255, 255, 255),
    );
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn text_lands_on_the_same_pixels_whatever_the_targets_odd_size() {
    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut list = gpu.draw_list();
    scene(&mut list);
    let even = gpu.capture_scaled_on(&list, EVEN, SCALE, Srgb::new([0.0, 0.0, 0.0, 1.0]));
    let odd = gpu.capture_scaled_on(&list, ODD, SCALE, Srgb::new([0.0, 0.0, 0.0, 1.0]));

    let mut inked = 0;
    let mut differing = Vec::new();
    for y in 0..EVEN.1 {
        for x in 0..EVEN.0 {
            let (a, b) = (pixel(&even, EVEN.0, x, y), pixel(&odd, ODD.0, x, y));
            if a[0] > 0 {
                inked += 1;
            }
            if a != b {
                differing.push((x, y, a, b));
            }
        }
    }
    assert!(
        inked > 100,
        "the text drew too little to compare: {inked} px"
    );
    assert!(
        differing.is_empty(),
        "{} pixels differ between the {EVEN:?} and {ODD:?} targets, first {:?}",
        differing.len(),
        &differing[..differing.len().min(5)]
    );
}
