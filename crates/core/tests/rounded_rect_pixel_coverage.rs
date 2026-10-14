//! CPU-oracle and GPU checks for pixel-area rounded-rectangle shadow clipping.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Affine2, BoxShadow, CornerRadii, HeadlessGpu};

const SUBPIXEL: &[[f32; 2]] = &[[0.0, 0.0], [0.125, 0.375], [0.49, 0.21]];
const DPR: &[f32] = &[1.0, 1.5, 2.0];
const ORACLE_GRID: u32 = 256;

fn rounded_rect_distance(p: [f32; 2], rect: Rect, radius: f32) -> f32 {
    let half = [rect.width * 0.5, rect.height * 0.5];
    let center = [rect.x + half[0], rect.y + half[1]];
    let q = [
        (p[0] - center[0]).abs() - half[0] + radius,
        (p[1] - center[1]).abs() - half[1] + radius,
    ];
    q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - radius
}

fn rounded_rect_contains(p: [f32; 2], rect: Rect, radius: f32) -> bool {
    rounded_rect_distance(p, rect, radius) <= 0.0
}

fn oracle_pixel_coverage(px: [u32; 2], dpr: f32, inverse: Affine2, rect: Rect, radius: f32) -> f32 {
    let mut covered = 0_u32;
    for sy in 0..ORACLE_GRID {
        for sx in 0..ORACLE_GRID {
            let world = [
                (px[0] as f32 + (sx as f32 + 0.5) / ORACLE_GRID as f32) / dpr,
                (px[1] as f32 + (sy as f32 + 0.5) / ORACLE_GRID as f32) / dpr,
            ];
            if rounded_rect_contains(inverse.transform_point(world), rect, radius) {
                covered += 1;
            }
        }
    }
    covered as f32 / (ORACLE_GRID * ORACLE_GRID) as f32
}

fn alpha_at(pixels: &[u8], width: u32, x: u32, y: u32) -> f32 {
    pixels[((y * width + x) * 4 + 3) as usize] as f32 / 255.0
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn tiny_rounded_shadow_clips_match_high_resolution_area_oracle() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size_css = 24_u32;
    let radius = 1.35;

    for &dpr in DPR {
        let size = (
            (size_css as f32 * dpr) as u32,
            (size_css as f32 * dpr) as u32,
        );
        for &offset in SUBPIXEL {
            for transformed in [false, true] {
                let rect = Rect::new(9.0 + offset[0], 9.0 + offset[1], 3.2, 2.7);
                let transform = if transformed {
                    Affine2::translation(11.0, 11.0)
                        .compose(&Affine2::rotation(0.31))
                        .compose(&Affine2::scale(1.17, 0.83))
                        .compose(&Affine2::translation(-11.0, -11.0))
                } else {
                    Affine2::IDENTITY
                };
                let inverse = transform.inverse();
                let mut list = gpu.draw_list();
                if transformed {
                    list.push_transform();
                    list.translate(11.0, 11.0);
                    list.rotate(0.31);
                    list.scale(1.17, 0.83);
                    list.translate(-11.0, -11.0);
                }
                list.box_shadow_inset(
                    rect,
                    CornerRadii::uniform(radius),
                    BoxShadow {
                        blur: 0.0,
                        spread: 8.0,
                        color: [0.0, 0.0, 0.0, 1.0],
                        inset: true,
                        ..Default::default()
                    },
                );
                if transformed {
                    list.pop_transform();
                }
                let pixels = gpu.capture_scaled(&list, size, dpr);

                let transformed_center = transform
                    .transform_point([rect.x + rect.width * 0.5, rect.y + rect.height * 0.5]);
                let center_px = [
                    (transformed_center[0] * dpr).floor() as i32,
                    (transformed_center[1] * dpr).floor() as i32,
                ];
                let mut compared = 0;
                for y in (center_px[1] - 5).max(0)..=(center_px[1] + 5).min(size.1 as i32 - 1) {
                    for x in (center_px[0] - 5).max(0)..=(center_px[0] + 5).min(size.0 as i32 - 1) {
                        let world_center = [(x as f32 + 0.5) / dpr, (y as f32 + 0.5) / dpr];
                        let local_center = inverse.transform_point(world_center);
                        let local_pixel_radius = 0.75 / dpr;
                        if rounded_rect_distance(local_center, rect, radius).abs()
                            > local_pixel_radius
                        {
                            continue;
                        }
                        let expected =
                            oracle_pixel_coverage([x as u32, y as u32], dpr, inverse, rect, radius);
                        if expected > 0.001 && expected < 0.999 {
                            let actual = alpha_at(&pixels, size.0, x as u32, y as u32);
                            assert!(
                                (actual - expected).abs() <= 0.14,
                                "DPR {dpr}, offset {offset:?}, transformed={transformed}, ({x},{y}): GPU {actual:.4}, oracle {expected:.4}"
                            );
                            compared += 1;
                        }
                    }
                }
                assert!(compared >= 3, "oracle did not exercise enough edge pixels");
            }
        }
    }
}
