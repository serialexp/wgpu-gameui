//! `UiRenderer::set_text_contrast`: light text on a dark background gains
//! weight at its edges, dark text on a light background stays about as it was,
//! and glyph interiors, shadows and the background are untouched.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test text_contrast -- --ignored --nocapture
//! ```
//! Also writes `test_output/text_contrast.png`: the same text with the
//! correction off (left) and on (right), to compare by eye.

use wgpu_gameui::color::Srgb;
use wgpu_gameui::{DrawList, HeadlessGpu, TextBlock, TextContrast};

const W: u32 = 360;
const H: u32 = 200;
/// The dark half is agent-ui's window background; the light half a paper white.
const DARK: [u8; 3] = [0x0a, 0x0d, 0x0f];
const LIGHT: [u8; 3] = [0xf4, 0xf5, 0xf6];
const SPLIT: u32 = H / 2;

fn rgba(c: [u8; 3]) -> [f32; 4] {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        1.0,
    ]
}

fn scene(list: &mut DrawList) {
    list.quad(0.0, SPLIT as f32, W as f32, (H - SPLIT) as f32, rgba(LIGHT));
    let lines = [
        (
            "Light text on a dark background, 13 px",
            13.0,
            (230, 232, 235),
        ),
        ("Secondary text, the quick brown fox", 12.0, (154, 163, 171)),
        ("Heading 18 px — Agent sessions", 18.0, (245, 246, 247)),
    ];
    for (i, (text, size, (r, g, b))) in lines.into_iter().enumerate() {
        list.text(
            TextBlock::new(text, 10.0, 10.0 + 26.0 * i as f32)
                .with_size(size)
                .with_color(r, g, b),
        );
    }
    let lines = [
        ("Dark text on a light background, 13 px", 13.0, (20, 22, 25)),
        ("Secondary text, the quick brown fox", 12.0, (90, 96, 104)),
        ("Heading 18 px — Agent sessions", 18.0, (10, 12, 14)),
    ];
    for (i, (text, size, (r, g, b))) in lines.into_iter().enumerate() {
        list.text(
            TextBlock::new(text, 10.0, SPLIT as f32 + 10.0 + 26.0 * i as f32)
                .with_size(size)
                .with_color(r, g, b),
        );
    }
}

fn capture(gpu: &mut HeadlessGpu, list: &DrawList, contrast: TextContrast) -> Vec<u8> {
    gpu.renderer().set_text_contrast(contrast);
    gpu.capture_on(list, (W, H), Srgb::new(rgba(DARK)))
}

/// How far each pixel of rows `rows` is from `background`, summed: the ink.
fn ink(pixels: &[u8], rows: std::ops::Range<u32>, background: [u8; 3]) -> u64 {
    let mut total = 0;
    for y in rows {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            for c in 0..3 {
                total += (pixels[i + c] as i32 - background[c] as i32).unsigned_abs() as u64;
            }
        }
    }
    total
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn light_text_on_dark_gains_weight_and_dark_text_on_light_does_not() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter available");
    let mut list = gpu.draw_list();
    scene(&mut list);

    assert_eq!(
        gpu.renderer().text_contrast(),
        TextContrast::GPUI,
        "the correction is on by default"
    );
    let off = capture(&mut gpu, &list, TextContrast::OFF);
    let on = capture(&mut gpu, &list, TextContrast::GPUI);

    // Side by side for the eye: off on the left, on on the right.
    let mut both = vec![0u8; (W * 2 * H * 4) as usize];
    for y in 0..H as usize {
        let row = W as usize * 4;
        both[y * row * 2..y * row * 2 + row].copy_from_slice(&off[y * row..(y + 1) * row]);
        both[y * row * 2 + row..(y + 1) * row * 2].copy_from_slice(&on[y * row..(y + 1) * row]);
    }
    std::fs::create_dir_all("test_output").unwrap();
    wgpu_gameui::write_png("test_output/text_contrast.png", &both, (W * 2, H)).unwrap();
    eprintln!("wrote test_output/text_contrast.png (left: off, right: on)");

    let dark_off = ink(&off, 0..SPLIT, DARK);
    let dark_on = ink(&on, 0..SPLIT, DARK);
    let light_off = ink(&off, SPLIT..H, LIGHT);
    let light_on = ink(&on, SPLIT..H, LIGHT);
    eprintln!("light on dark: {dark_off} → {dark_on}; dark on light: {light_off} → {light_on}");

    assert!(dark_off > 0 && light_off > 0, "the text drew");
    assert!(
        dark_on as f64 > dark_off as f64 * 1.08,
        "light-on-dark text gains visible weight: {dark_off} → {dark_on}"
    );
    let change = (light_on as f64 - light_off as f64).abs() / light_off as f64;
    assert!(
        change < 0.06,
        "dark-on-light text stays about as heavy: {light_off} → {light_on}"
    );

    // Only edges move: background stays background. A pixel the glyph barely
    // touches (under half a step of coverage) can round one step either way.
    for (i, (a, b)) in off.chunks(4).zip(on.chunks(4)).enumerate() {
        let y = i as u32 / W;
        let bg = if y < SPLIT { DARK } else { LIGHT };
        if a[..3] == bg {
            let step = (0..3)
                .map(|c| (a[c] as i32 - b[c] as i32).abs())
                .max()
                .unwrap();
            assert!(step <= 1, "background pixel {i} changed: {a:?} → {b:?}");
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn shadows_and_glows_keep_their_falloff() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter available");
    let mut list = gpu.draw_list();
    // A transparent glyph leaves only its soft glow and shadow on screen.
    list.text(
        TextBlock::new("Glow", 20.0, 20.0)
            .with_size(40.0)
            .with_rgba(255, 255, 255, 0)
            .with_glow(80, 180, 255, 255, 3.0),
    );
    list.text(
        TextBlock::new("Shadow", 20.0, 100.0)
            .with_size(40.0)
            .with_rgba(255, 255, 255, 0)
            .with_shadow(0, 0, 0, 220, 2.0, 2.0, 2.5),
    );
    let off = capture(&mut gpu, &list, TextContrast::OFF);
    let on = capture(&mut gpu, &list, TextContrast::GPUI);
    assert!(ink(&off, 0..H, DARK) > 0, "the glow drew");
    assert!(off == on, "soft edges are not corrected");
}
