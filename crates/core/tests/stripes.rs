//! Headless GPU checks for the analytic stripe fill (`DrawList::stripes`,
//! behind `hatch`, `dashed_hline` and `dashed_rect_outline`).
//!
//! The recorded data is covered by unit tests; these check what the shader
//! paints: dashes on whole pixels are solid and their gaps empty, crisp
//! hatching paints the same pixels the old unsmoothed 1 px lines did (to the
//! area's edges), a turned dashed line matches an upright one, the clip cuts,
//! a turned stripe fill lands where its transform says with a smoothed edge,
//! and bands too dense to tell apart fade to their average.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test stripes -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, Stripes, UiRenderer};

const W: u32 = 320;
const H: u32 = 240;
const WHITE: [f32; 4] = [1.0; 4];

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    ui: UiRenderer,
}

impl Gpu {
    fn new() -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("no GPU adapter (run under DISPLAY=:0)");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("stripes device"),
                ..Default::default()
            },
            None,
        ))
        .expect("request device");
        let ui = UiRenderer::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu_gameui::shared_font_system(),
        );
        Self { device, queue, ui }
    }

    /// Render `list` over black; the red channel of each pixel, row-major.
    fn render(&mut self, list: &DrawList) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stripes target"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bytes_per_row = (W * 4 + 255) & !255;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stripes readback"),
            size: (bytes_per_row * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.ui.begin_frame();
        self.ui.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            (W, H),
            1.0,
            list,
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let mut red = Vec::with_capacity((W * H) as usize);
        for row in 0..H as usize {
            let start = row * bytes_per_row as usize;
            red.extend((0..W as usize).map(|x| data[start + x * 4]));
        }
        red
    }
}

fn at(image: &[u8], x: u32, y: u32) -> u8 {
    image[(y * W + x) as usize]
}

/// Every pixel outside `area` is black.
fn assert_nothing_outside(image: &[u8], area: Rect, what: &str) {
    for y in 0..H {
        for x in 0..W {
            let inside = (x as f32) >= area.x
                && (x as f32) < area.right()
                && (y as f32) >= area.y
                && (y as f32) < area.bottom();
            if !inside {
                assert_eq!(at(image, x, y), 0, "{what}: ({x}, {y}) painted outside");
            }
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn dashes_on_whole_pixels_are_solid_and_their_gaps_empty() {
    let mut gpu = Gpu::new();
    let mut list = DrawList::new();
    list.dashed_hline(10.0, 20.0, 52.0, 3.0, 2.0, WHITE);
    let image = gpu.render(&list);
    for dx in 0..52 {
        let expected = if dx % 5 < 3 { 255 } else { 0 };
        assert_eq!(at(&image, 10 + dx, 20), expected, "x {}", 10 + dx);
    }
    assert_nothing_outside(&image, Rect::new(10.0, 20.0, 52.0, 1.0), "dashes");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn crisp_hatching_paints_whole_pixels_to_the_edges() {
    let mut gpu = Gpu::new();
    // `DrawList::hatch`: lines a whole step from the corner of the box half a
    // pixel in, so through pixel (dx, dy) with dx + dy a multiple of 4.
    let area = Rect::new(100.0, 100.0, 40.0, 30.0);
    let mut list = DrawList::new();
    list.hatch(area, 4.0, WHITE);
    let image = gpu.render(&list);
    for dy in 0..30 {
        for dx in 0..40 {
            let expected = if (dx + dy) % 4 == 0 { 255 } else { 0 };
            assert_eq!(at(&image, 100 + dx, 100 + dy), expected, "({dx}, {dy})");
        }
    }
    assert_nothing_outside(&image, area, "hatch");

    // `Stripes::hatch` (the waffle's): lines from the rect's own corner, so
    // through pixel centres with dx + dy + 1 a multiple of 4.
    let area = Rect::new(10.0, 10.0, 12.0, 12.0);
    let mut list = DrawList::new();
    list.stripes(area, Stripes::hatch(4.0, WHITE));
    let image = gpu.render(&list);
    for dy in 0..12 {
        for dx in 0..12 {
            let expected = if (dx + dy + 1) % 4 == 0 { 255 } else { 0 };
            assert_eq!(at(&image, 10 + dx, 10 + dy), expected, "({dx}, {dy})");
        }
    }
    assert_nothing_outside(&image, area, "cell hatch");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_dashed_line_turned_a_quarter_matches_an_upright_one() {
    let mut gpu = Gpu::new();
    let mut turned = DrawList::new();
    turned.push_transform();
    turned.translate(200.0, 50.0);
    turned.rotate(std::f32::consts::FRAC_PI_2);
    turned.dashed_hline(0.0, 0.0, 50.0, 3.0, 2.0, WHITE);
    turned.pop_transform();
    let mut upright = DrawList::new();
    upright.stripes(
        Rect::new(199.0, 50.0, 1.0, 50.0),
        Stripes {
            normal: [0.0, 1.0],
            period: 5.0,
            width: 3.0,
            offset: 0.0,
            color: WHITE,
            crisp: false,
        },
    );
    let (a, b) = (gpu.render(&turned), gpu.render(&upright));
    // Along the line they agree. Turned, the line's own edge is smoothed, so
    // its ends may glow faintly just past it; nothing reaches further.
    for y in 50..100 {
        let (p, q) = (at(&a, 199, y), at(&b, 199, y));
        assert!(p.abs_diff(q) <= 2, "(199, {y}): turned {p}, upright {q}");
    }
    assert_eq!(at(&b, 199, 50), 255);
    assert_eq!(at(&b, 199, 53), 0);
    assert_nothing_outside(&a, Rect::new(198.0, 49.0, 3.0, 52.0), "turned");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn the_clip_cuts_stripes() {
    let mut gpu = Gpu::new();
    let mut list = DrawList::new();
    list.push_clip(Rect::new(0.0, 0.0, 30.0, H as f32));
    list.dashed_hline(10.0, 20.0, 100.0, 3.0, 2.0, WHITE);
    list.hatch(Rect::new(0.0, 40.0, 100.0, 40.0), 4.0, WHITE);
    list.pop_clip();
    let image = gpu.render(&list);
    assert_eq!(at(&image, 10, 20), 255);
    assert_eq!(at(&image, 0, 40), 255, "the hatch's corner pixel");
    for y in 0..H {
        for x in 30..W {
            assert_eq!(at(&image, x, y), 0, "({x}, {y}) past the clip");
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_turned_stripe_fill_lands_where_the_transform_says_with_a_smooth_edge() {
    let mut gpu = Gpu::new();
    // Bands as wide as their period fill the rect, leaving only its edge.
    let size = [100.0f32, 60.0];
    let (angle, center) = (0.35f32, [160.0f32, 120.0]);
    for scale in [1.0f32, 1.5] {
        let mut list = DrawList::new();
        list.translate(center[0], center[1]);
        list.rotate(angle);
        list.scale(scale, scale);
        list.stripes(
            Rect::new(-size[0] * 0.5, -size[1] * 0.5, size[0], size[1]),
            Stripes {
                normal: [1.0, 0.0],
                period: 4.0,
                width: 4.0,
                offset: 0.0,
                color: WHITE,
                crisp: false,
            },
        );
        let image = gpu.render(&list);
        let (sin, cos) = angle.sin_cos();
        let (mut inside, mut outside, mut fringe) = (0, 0, 0);
        for y in 0..H {
            for x in 0..W {
                // The pixel centre back in the rect's space, from its centre.
                let dx = x as f32 + 0.5 - center[0];
                let dy = y as f32 + 0.5 - center[1];
                let lx = (cos * dx + sin * dy) / scale;
                let ly = (-sin * dx + cos * dy) / scale;
                let q = [lx.abs() - size[0] * 0.5, ly.abs() - size[1] * 0.5];
                let d = ((q[0].max(0.0)).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0)) * scale;
                let v = at(&image, x, y);
                if d < -1.5 {
                    inside += 1;
                    assert!(v >= 250, "scale {scale} ({x}, {y}): {d:.2} inside, {v}");
                } else if d > 1.5 {
                    outside += 1;
                    assert_eq!(v, 0, "scale {scale} ({x}, {y}): {d:.2} outside, {v}");
                } else if d > 0.1 && d < 0.3 {
                    fringe += 1;
                    assert!(v > 0, "scale {scale} ({x}, {y}): {d:.2} outside, unlit");
                }
            }
        }
        assert!(
            inside > 4_000 && outside > 50_000 && fringe > 50,
            "scale {scale}: {inside}/{outside}/{fringe}"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn bands_too_dense_to_tell_apart_fade_to_their_average() {
    let mut gpu = Gpu::new();
    let mut half = DrawList::new();
    half.stripes(
        Rect::new(20.0, 20.0, 100.0, 100.0),
        Stripes {
            normal: [0.6, 0.8],
            period: 0.7,
            width: 0.35,
            offset: 0.0,
            color: WHITE,
            crisp: false,
        },
    );
    // The same half coverage as one flat fill at half alpha.
    let mut flat = DrawList::new();
    flat.quad(20.0, 20.0, 100.0, 100.0, [1.0, 1.0, 1.0, 0.5]);
    let (a, b) = (gpu.render(&half), gpu.render(&flat));
    let expected = at(&b, 50, 50);
    for y in 20..120 {
        for x in 20..120 {
            let p = at(&a, x, y);
            assert!(
                p.abs_diff(expected) <= 3,
                "({x}, {y}): {p}, flat {expected}"
            );
        }
    }
}
