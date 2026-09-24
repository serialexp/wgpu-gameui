//! Ignored headless smoke/geometry test for analytic box shadows.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{BoxShadow, CornerRadii, DrawList, HeadlessGpu};

fn alpha_at(pixels: &[u8], size: (u32, u32), x: u32, y: u32) -> u8 {
    pixels[((y * size.0 + x) * 4 + 3) as usize]
}

fn rgba_at(pixels: &[u8], size: (u32, u32), x: u32, y: u32) -> [u8; 4] {
    let start = ((y * size.0 + x) * 4) as usize;
    pixels[start..start + 4].try_into().unwrap()
}

fn assert_horizontal_mirror(left: &[u8], right: &[u8], size: (u32, u32), tolerance: u8) {
    for y in 0..size.1 {
        for x in 0..size.0 {
            let a = alpha_at(left, size, x, y);
            let b = alpha_at(right, size, size.0 - 1 - x, y);
            assert!(
                a.abs_diff(b) <= tolerance,
                "alpha mismatch at ({x}, {y}): {a} vs mirrored {b}"
            );
        }
    }
}

fn assert_all_alpha_zero(pixels: &[u8]) {
    assert_eq!(
        pixels.chunks_exact(4).filter(|pixel| pixel[3] != 0).count(),
        0,
        "an identical crisp outset source and element must cancel exactly"
    );
}

fn asymmetric_shadow(list: &mut DrawList, radii: CornerRadii, blur: f32) {
    list.box_shadow_outset(
        Rect::new(45.5, 35.5, 69.0, 49.0),
        radii,
        BoxShadow {
            blur,
            color: [0.2, 0.6, 0.9, 0.8],
            ..Default::default()
        },
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn identical_zero_blur_outset_is_transparent_but_crisp_offset_and_spread_remain() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (96, 80);
    let element = Rect::new(24.0, 20.0, 40.0, 32.0);
    let radii = CornerRadii::uniform(7.0);

    let mut identical = gpu.draw_list();
    identical.box_shadow_outset(
        element,
        radii,
        BoxShadow {
            color: [0.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
    );
    assert_all_alpha_zero(&gpu.capture(&identical, size));

    let mut offset = gpu.draw_list();
    offset.box_shadow_outset(
        element,
        radii,
        BoxShadow {
            offset: [6.0, 0.0],
            color: [0.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
    );
    let offset_pixels = gpu.capture(&offset, size);
    assert_eq!(alpha_at(&offset_pixels, size, 68, 36), 255);
    assert_eq!(alpha_at(&offset_pixels, size, 27, 36), 0);

    let mut spread = gpu.draw_list();
    spread.box_shadow_outset(
        element,
        radii,
        BoxShadow {
            spread: 4.0,
            color: [0.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
    );
    let spread_pixels = gpu.capture(&spread, size);
    assert_eq!(alpha_at(&spread_pixels, size, 21, 36), 255);
    assert_eq!(alpha_at(&spread_pixels, size, 40, 36), 0);
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn analytic_shadow_zero_blur_inset_outset_and_affine() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let mut list = gpu.draw_list();
    let element = Rect::new(30.0, 30.0, 70.0, 50.0);
    list.box_shadow_outset(
        element,
        CornerRadii::new(12.0, 4.0, 16.0, 2.0),
        BoxShadow {
            offset: [8.0, 5.0],
            blur: 12.0,
            spread: 2.0,
            color: [0.0, 0.0, 0.0, 0.8],
            inset: false,
        },
    );
    list.rounded_rect(element, 8.0, [0.7, 0.7, 0.7, 1.0]);
    list.box_shadow_inset(
        element,
        CornerRadii::uniform(8.0),
        BoxShadow {
            offset: [3.0, 0.0],
            blur: 0.0,
            spread: 38.0,
            color: [0.0, 0.2, 0.8, 0.7],
            inset: true,
        },
    );
    list.push_transform();
    list.translate(155.0, 35.0);
    list.rotate(0.35);
    list.scale(-1.2, 0.8);
    list.box_shadow_outset(
        Rect::new(0.0, 0.0, 55.0, 32.0),
        CornerRadii::uniform(7.0),
        BoxShadow {
            blur: 8.0,
            color: [0.0, 0.8, 1.0, 0.8],
            ..Default::default()
        },
    );
    list.pop_transform();

    let pixels = gpu.capture(&list, (240, 130));
    let affected = pixels
        .chunks_exact(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    assert!(
        affected > 2_000,
        "analytic shadows painted too few pixels: {affected}"
    );
    wgpu_gameui::write_png("test_output/analytic_shadows.png", &pixels, (240, 130)).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn asymmetric_corner_quadrature_crosses_centerline_and_reflects() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (160, 120);
    let radii = CornerRadii::new(30.0, 3.0, 22.0, 9.0);

    let mut left = gpu.draw_list();
    asymmetric_shadow(&mut left, radii, 20.0);
    let left_pixels = gpu.capture(&left, size);

    let mut right = gpu.draw_list();
    right.push_transform();
    right.translate(size.0 as f32, 0.0);
    right.scale(-1.0, 1.0);
    asymmetric_shadow(&mut right, radii, 20.0);
    right.pop_transform();
    let right_pixels = gpu.capture(&right, size);

    // This compares every alpha value, including rows whose four y samples
    // straddle the source centerline. Reflection must only reverse geometry;
    // it must not change which radii each source slice uses.
    assert_horizontal_mirror(&left_pixels, &right_pixels, size, 6);
    let corner_alpha = alpha_at(&left_pixels, size, 40, 45);
    assert!(
        corner_alpha > 20,
        "each sampled source slice must use its own upper-left radius: {corner_alpha}"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn inset_clips_to_each_padding_edge() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (80, 70);
    let mut list = gpu.draw_list();
    list.box_shadow_inset(
        Rect::new(20.0, 18.0, 40.0, 32.0),
        CornerRadii::default(),
        BoxShadow {
            spread: 5.0,
            color: [0.0, 1.0, 1.0, 1.0],
            inset: true,
            ..Default::default()
        },
    );

    let pixels = gpu.capture(&list, size);
    for (x, y) in [(21, 34), (58, 34), (40, 19), (40, 48)] {
        assert!(
            alpha_at(&pixels, size, x, y) > 240,
            "missing inset at ({x}, {y})"
        );
    }
    assert_eq!(alpha_at(&pixels, size, 40, 34), 0, "inset filled its hole");
    for (x, y) in [(19, 34), (60, 34), (40, 17), (40, 50)] {
        assert_eq!(
            alpha_at(&pixels, size, x, y),
            0,
            "inset escaped at ({x}, {y})"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn active_shadow_clip_is_physical_at_dpr_1_1_5_and_2() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let mut list = gpu.draw_list();
    list.push_clip(Rect::new(18.0, 14.0, 20.0, 16.0));
    list.box_shadow_inset(
        Rect::new(8.0, 6.0, 42.0, 34.0),
        CornerRadii::default(),
        BoxShadow {
            spread: 30.0,
            color: [0.0, 1.0, 1.0, 1.0],
            inset: true,
            ..Default::default()
        },
    );

    for dpr in [1.0_f32, 1.5, 2.0] {
        let size = ((64.0 * dpr) as u32, (48.0 * dpr) as u32);
        let pixels = gpu.capture_scaled(&list, size, dpr);
        let inside = ((28.0 * dpr) as u32, (22.0 * dpr) as u32);
        let outside = ((16.0 * dpr) as u32, (22.0 * dpr) as u32);
        assert!(
            alpha_at(&pixels, size, inside.0, inside.1) > 240,
            "clip lost content at DPR {dpr}"
        );
        assert_eq!(
            alpha_at(&pixels, size, outside.0, outside.1),
            0,
            "clip leaked at DPR {dpr}"
        );
    }
}

#[test]
fn singular_shadow_transform_is_rejected_on_cpu() {
    let mut list = DrawList::new();
    list.scale(0.0, 1.0);
    list.box_shadow_outset(
        Rect::new(4.0, 4.0, 20.0, 12.0),
        CornerRadii::uniform(2.0),
        BoxShadow {
            color: [1.0; 4],
            ..Default::default()
        },
    );
    assert!(list.shadow_instance_count() == 0);
    assert_eq!(list.dropped_degenerate(), 1);
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn uniform_affine_scale_scales_shadow_geometry() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (100, 64);
    let mut list = gpu.draw_list();
    list.push_transform();
    list.translate(50.0, 8.0);
    list.scale(2.0, 2.0);
    list.box_shadow_inset(
        Rect::new(2.0, 2.0, 18.0, 12.0),
        CornerRadii::default(),
        BoxShadow {
            spread: 3.0,
            color: [0.0, 1.0, 1.0, 1.0],
            inset: true,
            ..Default::default()
        },
    );
    list.pop_transform();

    let pixels = gpu.capture(&list, size);
    assert!(
        alpha_at(&pixels, size, 55, 20) > 240,
        "scaled left inset edge missing"
    );
    assert!(
        alpha_at(&pixels, size, 87, 20) > 240,
        "scaled right inset edge missing"
    );
    assert_eq!(
        alpha_at(&pixels, size, 71, 20),
        0,
        "scaled inset band did not preserve its hole"
    );
    assert_eq!(
        alpha_at(&pixels, size, 53, 20),
        0,
        "scaled shadow escaped its element"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn transformed_clip_uses_documented_world_aabb() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (80, 80);
    let mut list = gpu.draw_list();
    list.push_transform();
    list.translate(40.0, 20.0);
    list.rotate(std::f32::consts::FRAC_PI_4);
    list.push_clip(Rect::new(0.0, 0.0, 20.0, 20.0));
    list.pop_transform();
    list.box_shadow_inset(
        Rect::new(20.0, 15.0, 45.0, 40.0),
        CornerRadii::default(),
        BoxShadow {
            spread: 50.0,
            color: [0.0, 1.0, 1.0, 1.0],
            inset: true,
            ..Default::default()
        },
    );

    let pixels = gpu.capture(&list, size);
    // A true rotated clip would reject this corner; push_clip deliberately stores
    // the transformed world-space AABB, whose approximate bounds include it.
    assert!(alpha_at(&pixels, size, 27, 21) > 240);
    assert_eq!(alpha_at(&pixels, size, 25, 18), 0);
    assert_eq!(alpha_at(&pixels, size, 55, 50), 0);
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn two_by_twenty_six_cyan_splitter_glow_is_symmetric() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (96, 96);
    let mut list = gpu.draw_list();
    list.box_shadow_outset(
        Rect::new(47.0, 35.0, 2.0, 26.0),
        CornerRadii::uniform(1.0),
        BoxShadow {
            blur: 44.0,
            color: [0.0, 0.8, 1.0, 0.9],
            ..Default::default()
        },
    );
    let pixels = gpu.capture(&list, size);
    assert_horizontal_mirror(&pixels, &pixels, size, 1);
    let glow = rgba_at(&pixels, size, 32, 48);
    assert!(
        glow[1] > glow[0] && glow[2] > glow[1] && glow[3] > 2,
        "cyan glow sentinel: {glow:?}"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn mixed_color_shadows_are_reverse_stacked_below_opaque_surface() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (96, 72);
    let mut list = gpu.draw_list();
    let element = Rect::new(32.0, 22.0, 32.0, 28.0);
    let shadows = [
        BoxShadow {
            offset: [-5.0, 0.0],
            blur: 16.0,
            color: [1.0, 0.0, 0.0, 0.8],
            ..Default::default()
        },
        BoxShadow {
            offset: [5.0, 0.0],
            blur: 16.0,
            color: [0.0, 0.2, 1.0, 0.8],
            ..Default::default()
        },
    ];
    list.box_shadows_outset(element, CornerRadii::uniform(4.0), &shadows);
    list.rounded_rect(element, 4.0, [0.2, 0.8, 0.2, 1.0]);

    let pixels = gpu.capture(&list, size);
    let overlap = rgba_at(&pixels, size, 30, 36);
    assert!(
        overlap[0] > overlap[2],
        "first declared red shadow was not topmost: {overlap:?}"
    );
    let surface = rgba_at(&pixels, size, 48, 36);
    // Colours are sRGB-encoded and captured without conversion: [0.2, 0.8, 0.2]
    // reads back as 0.2*255 / 0.8*255.
    assert_eq!(
        surface,
        [51, 204, 51, 255],
        "opaque green surface did not cover shadows"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn asymmetric_corner_quadrature_remains_stable_for_broad_blur() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (160, 120);
    let radii = CornerRadii::new(32.0, 1.0, 24.0, 6.0);

    let mut direct = gpu.draw_list();
    asymmetric_shadow(&mut direct, radii, 54.0);
    let direct_pixels = gpu.capture(&direct, size);

    let mut reflected = gpu.draw_list();
    reflected.push_transform();
    reflected.translate(size.0 as f32, 0.0);
    reflected.scale(-1.0, 1.0);
    asymmetric_shadow(&mut reflected, radii, 54.0);
    reflected.pop_transform();
    let reflected_pixels = gpu.capture(&reflected, size);

    assert_horizontal_mirror(&direct_pixels, &reflected_pixels, size, 2);
    let nonzero = direct_pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[3] > 0)
        .count();
    assert!(
        nonzero > 5_000,
        "broad blur painted too few pixels: {nonzero}"
    );
}
