//! Headless GPU checks for analytic strokes (`DrawList::line`,
//! `stroke_line`, `stroke_polyline`, `stroke_closed`).
//!
//! The recorded segments are covered by unit tests (`widgets/stroke.rs`);
//! these check what the shader paints, from the alpha channel of a capture on
//! a transparent target (alpha is not sRGB-encoded, so it reads as coverage
//! times the colour's alpha):
//! - whole-px axis-aligned lines are crisp, on the pixels a quad covered;
//! - a translucent stroke paints each pixel once at its corners, whichever the
//!   join, at DPR 1 and 1.5, and leaves no holes;
//! - a diagonal, turned or scaled line covers its area, with smooth edges;
//! - caps reach past the ends as SVG's do;
//! - dashes are solid with empty gaps, and run on round corners;
//! - the clip cuts.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test strokes -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Cap, DrawList, HeadlessGpu, Join, Stroke};

const W: u32 = 160;
const H: u32 = 120;
const WHITE: [f32; 4] = [1.0; 4];
const HALF: [f32; 4] = [1.0, 1.0, 1.0, 0.5];

struct Alpha {
    width: u32,
    pixels: Vec<u8>,
}

impl Alpha {
    fn at(&self, x: u32, y: u32) -> u8 {
        self.pixels[((y * self.width + x) * 4 + 3) as usize]
    }

    fn values(&self) -> impl Iterator<Item = u8> + '_ {
        self.pixels.as_chunks::<4>().0.iter().map(|p| p[3])
    }

    /// Total alpha, in pixels of full coverage.
    fn area(&self) -> f32 {
        self.values().map(|a| f32::from(a) / 255.0).sum()
    }

    /// The painted pixels (any alpha), as `(x, y)`.
    fn painted(&self) -> Vec<(u32, u32)> {
        self.values()
            .enumerate()
            .filter(|&(_, a)| a > 0)
            .map(|(i, _)| (i as u32 % self.width, i as u32 / self.width))
            .collect()
    }
}

fn gpu() -> HeadlessGpu {
    HeadlessGpu::new().expect("no GPU adapter (run under DISPLAY=:0)")
}

fn capture(gpu: &mut HeadlessGpu, list: &DrawList) -> Alpha {
    capture_at(gpu, list, 1.0)
}

fn capture_at(gpu: &mut HeadlessGpu, list: &DrawList, scale: f32) -> Alpha {
    let width = (W as f32 * scale) as u32;
    let height = (H as f32 * scale) as u32;
    Alpha {
        width,
        pixels: gpu.capture_scaled(list, (width, height), scale),
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn whole_px_axis_aligned_lines_are_crisp_on_the_pixels_a_quad_covered() {
    let mut gpu = gpu();
    let mut list = gpu.draw_list();
    // A 1 px rule at y = 10 from x = 3 to 7 covered the row of pixel centres
    // 9.5, columns 3 to 6; a 2 px one at x = 20.4 the columns 19 and 20.
    list.line([3.0, 10.0], [7.0, 10.0], 1.0, WHITE);
    list.line([20.4, 30.0], [20.4, 40.0], 2.0, WHITE);
    let alpha = capture(&mut gpu, &list);
    let mut expected: Vec<(u32, u32)> = (3..7).map(|x| (x, 9)).collect();
    expected.extend((30..40).flat_map(|y| [(19, y), (20, y)]));
    expected.sort_by_key(|&(x, y)| (y, x));
    assert_eq!(alpha.painted(), expected);
    assert!(expected.iter().all(|&(x, y)| alpha.at(x, y) == 255));
}

/// A translucent stroke with corners of every kind: its pixels are at most one
/// coat, and the pixels well inside it exactly one.
fn assert_one_coat(alpha: &Alpha, label: &str) {
    let coat = 128;
    let max = alpha.values().max().unwrap_or(0);
    assert!(max <= coat, "{label}: a pixel was painted twice ({max})");
    let full = alpha.values().filter(|&a| a == coat).count();
    assert!(full > 200, "{label}: only {full} pixels at one coat");
}

fn corners(list: &mut DrawList, join: Join) {
    let stroke = Stroke::new(7.0).join(join).miter_limit(10.0);
    // A right angle, an acute turn, a shallow one and a turn back.
    let points = [
        [12.3, 14.1],
        [70.6, 14.1],
        [70.6, 60.2],
        [20.4, 100.7],
        [60.9, 90.3],
        [150.2, 105.5],
        [100.5, 70.5],
    ];
    list.stroke_polyline(&points, &stroke, HALF);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_translucent_polyline_paints_each_pixel_once_at_every_join() {
    let mut gpu = gpu();
    for join in [Join::Miter, Join::Round, Join::Bevel] {
        for scale in [1.0, 1.5, 2.0] {
            let mut list = gpu.draw_list();
            corners(&mut list, join);
            let alpha = capture_at(&mut gpu, &list, scale);
            assert_one_coat(&alpha, &format!("{join:?} at {scale}x"));
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_translucent_rectangle_outline_is_one_coat_to_its_corners() {
    let mut gpu = gpu();
    let mut list = gpu.draw_list();
    // Snapped onto the grid: its corners' halving lines run through pixel
    // centres, where both segments must still agree on every pixel.
    let square = [[10.0, 10.0], [90.0, 10.0], [90.0, 70.0], [10.0, 70.0]];
    list.stroke_closed(&square, &Stroke::new(3.0), HALF);
    let alpha = capture(&mut gpu, &list);
    let painted = alpha.painted();
    assert!(painted.iter().all(|&(x, y)| alpha.at(x, y) == 128));
    // The outline's ring of whole pixels: 3 px wide round the 80 × 60 box.
    let outer = 83 * 63;
    let inner = 77 * 57;
    assert_eq!(painted.len(), outer - inner);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_dense_zigzag_has_no_holes() {
    // A chart line squeezed to two points a pixel: steep segments shorter
    // across than the stroke is wide. Every pixel between its top and bottom
    // must be covered.
    let mut gpu = gpu();
    let mut list = gpu.draw_list();
    let points: Vec<[f32; 2]> = (0..120)
        .map(|i| [20.0 + i as f32, if i % 2 == 0 { 20.0 } else { 90.0 }])
        .collect();
    for join in [Join::Miter, Join::Round, Join::Bevel] {
        list.clear();
        list.stroke_polyline(&points, &Stroke::new(3.5).join(join), WHITE);
        let alpha = capture(&mut gpu, &list);
        for y in 22..88 {
            for x in 22..138 {
                assert_eq!(alpha.at(x, y), 255, "{join:?}: a hole at {x}, {y}");
            }
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn diagonal_turned_and_scaled_lines_cover_their_area_smoothly() {
    let mut gpu = gpu();
    let mut list = gpu.draw_list();
    // 60 px long, 3 px wide at 30°: 180 px².
    let (sin, cos) = 30f32.to_radians().sin_cos();
    list.line(
        [40.0, 40.0],
        [40.0 + 60.0 * cos, 40.0 + 60.0 * sin],
        3.0,
        WHITE,
    );
    let alpha = capture(&mut gpu, &list);
    assert!((alpha.area() - 180.0).abs() < 2.0, "area {}", alpha.area());
    let partial = alpha.values().filter(|&a| a > 0 && a < 255).count();
    assert!(
        partial > 60,
        "edges should be smooth: {partial} partial pixels"
    );

    // The same line drawn upright under a rotation, and at half size under a
    // scale of two, covers the same area.
    for scale in [1.0, 2.0] {
        list.clear();
        list.push_transform();
        list.translate(40.0, 40.0);
        list.scale(scale, scale);
        list.rotate(30f32.to_radians());
        list.line([0.0, 0.0], [60.0 / scale, 0.0], 3.0 / scale, WHITE);
        list.pop_transform();
        let turned = capture(&mut gpu, &list);
        assert!(
            (turned.area() - 180.0).abs() < 2.0,
            "area {}",
            turned.area()
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn caps_reach_past_the_ends() {
    let mut gpu = gpu();
    // A 40 px line 6 px wide, on whole pixels: butt 240 px², square 36 more,
    // round a 3 px disc (28.3 px²) more.
    for (cap, area) in [
        (Cap::Butt, 240.0),
        (Cap::Square, 276.0),
        (Cap::Round, 268.3),
    ] {
        let mut list = gpu.draw_list();
        list.stroke_line(
            [40.0, 50.0],
            [80.0, 50.0],
            &Stroke::new(6.0).cap(cap),
            WHITE,
        );
        let alpha = capture(&mut gpu, &list);
        assert!(
            (alpha.area() - area).abs() < 1.0,
            "{cap:?}: area {}",
            alpha.area()
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn dashes_are_solid_with_empty_gaps_and_run_on_round_corners() {
    let mut gpu = gpu();
    let mut list = gpu.draw_list();
    // 4 on, 3 off from x = 10, on a whole-pixel row.
    list.stroke_line(
        [10.0, 20.0],
        [80.0, 20.0],
        &Stroke::new(1.0).dashed(4.0, 3.0),
        WHITE,
    );
    let alpha = capture(&mut gpu, &list);
    for x in 10..80 {
        let on = (x - 10) % 7 < 4;
        assert_eq!(alpha.at(x, 19), if on { 255 } else { 0 }, "x = {x}");
    }
    assert_eq!(alpha.painted().len(), 40);

    // Round a right angle, the pattern carries on: half of 100 px on 5/5.
    // Started 7 px in, the corner (50 px along) falls in the middle of a gap,
    // so its square stays empty.
    list.clear();
    let corner = [[20.0, 40.0], [70.0, 40.0], [70.0, 90.0]];
    list.stroke_polyline(&corner, &Stroke::new(2.0).dashed_from(5.0, 5.0, 7.0), WHITE);
    let alpha = capture(&mut gpu, &list);
    assert!((alpha.area() - 100.0).abs() < 2.0, "area {}", alpha.area());
    for (x, y) in [(69, 39), (69, 40), (70, 39), (70, 40)] {
        assert_eq!(alpha.at(x, y), 0, "{x}, {y}");
    }
    // Shifted so a dash spans the corner, its join fills the corner square.
    list.clear();
    list.stroke_polyline(&corner, &Stroke::new(2.0).dashed_from(5.0, 5.0, 2.0), WHITE);
    let alpha = capture(&mut gpu, &list);
    for (x, y) in [(69, 39), (69, 40), (70, 39), (70, 40)] {
        assert_eq!(alpha.at(x, y), 255, "{x}, {y}");
    }

    // An offset moves the pattern along.
    list.clear();
    list.stroke_line(
        [10.0, 20.0],
        [80.0, 20.0],
        &Stroke::new(1.0).dashed_from(4.0, 3.0, 2.0),
        WHITE,
    );
    let alpha = capture(&mut gpu, &list);
    assert_eq!(
        (alpha.at(10, 19), alpha.at(11, 19), alpha.at(12, 19)),
        (255, 255, 0)
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn the_clip_cuts_a_stroke() {
    let mut gpu = gpu();
    let mut list = gpu.draw_list();
    list.push_clip(Rect::new(0.0, 0.0, 50.0, H as f32));
    list.line([10.0, 20.0], [100.0, 20.0], 1.0, WHITE);
    list.pop_clip();
    let alpha = capture(&mut gpu, &list);
    assert_eq!(alpha.painted().len(), 40);
}
