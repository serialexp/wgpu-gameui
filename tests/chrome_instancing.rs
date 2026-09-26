//! Headless GPU parity test for the instanced SDF chrome path.
//!
//! `DrawList::chrome_rect` rasterizes a button's rounded background + border
//! from a signed distance field in one instanced draw, instead of tessellating
//! ~80 vertices into the colored soup. One test renders the same grid of
//! buttons two ways — once via `chrome_rect` (one combined instance), once as a
//! separate `rounded_rect` fill + `rounded_rect_outline` — and asserts the
//! images match within a small tolerance (the border's anti-aliased edge
//! blends differently when drawn as its own layer).
//!
//! Rotated and scaled chrome also stays a single SDF instance: the shader
//! applies the affine. `rotated_chrome_uses_the_sdf_path_and_lands_where_the_
//! transform_says` checks every pixel of a rotated shape against the analytic
//! one, including the anti-aliased band just outside the edge.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test chrome_instancing -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Background, CornerRadii, DrawList, EdgeWidths, FontSystemHandle, GradientAxis, QuadStyle,
    UiRenderer,
};

const W: u32 = 480;
const H: u32 = 320;

/// Render a single `DrawList` to an RGBA image (tightly packed, W*4 per row).
fn render_list(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    list: &DrawList,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("parity target"),
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
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parity readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
    // One submission = one frame (see `UiRenderer::begin_frame`).
    ui.begin_frame();
    ui.render(device, queue, &mut encoder, &view, (W, H), 1.0, list);
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
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    let row_stride = (W * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * H as usize);
    for row in 0..H as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }
    pixels
}

/// Lay out a 4×3 grid of 90×50 buttons.
fn button_rects() -> Vec<Rect> {
    let mut rects = Vec::new();
    for row in 0..3 {
        for col in 0..4 {
            rects.push(Rect::new(
                20.0 + col as f32 * 110.0,
                20.0 + row as f32 * 90.0,
                90.0,
                50.0,
            ));
        }
    }
    rects
}

const RADIUS: f32 = 8.0;
const THICKNESS: f32 = 2.0;
const BG: [f32; 4] = [0.20, 0.45, 0.75, 1.0];
const BORDER: [f32; 4] = [0.85, 0.85, 0.90, 1.0];

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_chrome_matches_immediate() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("parity device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system: FontSystemHandle = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    let rects = button_rects();

    // Instanced path.
    let mut instanced = DrawList::with_font_system(font_system.clone());
    for r in &rects {
        instanced.chrome_rect(*r, RADIUS, THICKNESS, BG, BORDER);
    }

    // Reference: the same chrome as a separate fill + outline (two SDF
    // instances per button instead of one combined instance).
    let mut immediate = DrawList::with_font_system(font_system.clone());
    for r in &rects {
        immediate.rounded_rect(*r, RADIUS, BG);
        immediate.rounded_rect_outline(*r, RADIUS, THICKNESS, BORDER);
    }

    let img_inst = render_list(&device, &queue, &mut ui, &instanced);
    let img_imm = render_list(&device, &queue, &mut ui, &immediate);

    // Persist for eyeballing.
    std::fs::create_dir_all("test_output").ok();
    image::RgbaImage::from_raw(W, H, img_inst.clone())
        .and_then(|i| i.save("test_output/chrome_instanced.png").ok().map(|_| i));
    image::RgbaImage::from_raw(W, H, img_imm.clone())
        .and_then(|i| i.save("test_output/chrome_immediate.png").ok().map(|_| i));

    assert_eq!(img_inst.len(), img_imm.len());

    // Count pixels whose RGB differ beyond a tolerance. Edge AA + the 1px SDF
    // border are where the two paths legitimately diverge, so we allow a small
    // fraction. A non-trivial number of pixels must be drawn (sanity).
    let mut differing = 0usize;
    let mut drawn = 0usize;
    for (a, b) in img_inst
        .as_chunks::<4>()
        .0
        .iter()
        .zip(img_imm.as_chunks::<4>().0.iter())
    {
        let da = (a[0] as i32 - b[0] as i32).abs()
            + (a[1] as i32 - b[1] as i32).abs()
            + (a[2] as i32 - b[2] as i32).abs();
        if da > 48 {
            differing += 1;
        }
        // "Drawn" = not the black clear color in the immediate reference.
        if a[0] as i32 + a[1] as i32 + a[2] as i32 > 30 {
            drawn += 1;
        }
    }
    let total = (W * H) as usize;
    assert!(drawn > total / 20, "too few pixels drawn ({drawn}/{total})");

    // Differences should be confined to AA edges/borders: well under 6% of all
    // pixels. (Each 90×50 button is ~4500px × 12 = 54k drawn; their ~1px rounded
    // outlines/corners are a few thousand px total.)
    let frac = differing as f64 / total as f64;
    assert!(
        frac < 0.06,
        "instanced vs immediate differ in {:.2}% of pixels (>{:.0}%) — not just AA edges",
        frac * 100.0,
        6.0
    );
}

/// Signed distance from `p` to a rounded rect at the origin with `size` and a
/// uniform corner `radius` (negative inside). Same formula as the shader.
fn rounded_rect_distance(p: [f32; 2], size: [f32; 2], radius: f32) -> f32 {
    let q = [
        (p[0] - size[0] * 0.5).abs() - size[0] * 0.5 + radius,
        (p[1] - size[1] * 0.5).abs() - size[1] * 0.5 + radius,
    ];
    let outside = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt();
    outside + q[0].max(q[1]).min(0.0) - radius
}

/// One way of drawing a white rounded rect at `rect` (local space).
type DrawRotated = fn(&mut DrawList, Rect);

/// Render one white rounded rect under `translate(center) · rotate(angle) ·
/// scale(s, s)` and check every pixel against the analytic shape: pixels well
/// inside are full white, pixels well outside are untouched black, and the
/// thin band just *outside* the edge is partly lit — the anti-aliasing ramp
/// reaches past the edge, so the instance quad must be padded to draw it.
/// A transposed matrix (the shape turning the wrong way) fails the inside and
/// outside checks.
fn assert_rotated_shape(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    name: &str,
    draw: DrawRotated,
    angle: f32,
    s: f32,
) {
    let size = [140.0f32, 70.0];
    let center = [W as f32 * 0.5, H as f32 * 0.5];
    let mut list = DrawList::with_font_system(wgpu_gameui::shared_font_system());
    list.translate(center[0], center[1]);
    list.rotate(angle);
    list.scale(s, s);
    draw(
        &mut list,
        Rect::new(-size[0] * 0.5, -size[1] * 0.5, size[0], size[1]),
    );
    assert_eq!(
        list.chrome_instance_count(),
        1,
        "{name}: a rotated shape should be one SDF instance, not tessellated"
    );
    let image = render_list(device, queue, ui, &list);
    image::RgbaImage::from_raw(W, H, image.clone()).and_then(|i| {
        i.save(format!("test_output/chrome_rotated_{name}.png"))
            .ok()
    });

    let (sin, cos) = angle.sin_cos();
    let (mut inside, mut outside, mut fringe) = (0, 0, 0);
    for y in 0..H {
        for x in 0..W {
            // Pixel centre back into the rect's local space (inverse of
            // translate · rotate · scale), measured from the rect's top-left.
            let dx = x as f32 + 0.5 - center[0];
            let dy = y as f32 + 0.5 - center[1];
            let lx = (cos * dx + sin * dy) / s + size[0] * 0.5;
            let ly = (-sin * dx + cos * dy) / s + size[1] * 0.5;
            // Uniform scale: screen distance is local distance times `s`.
            let d = rounded_rect_distance([lx, ly], size, 10.0) * s;
            let v = image[((y * W + x) * 4) as usize];
            if d < -1.5 {
                inside += 1;
                assert!(
                    v >= 250,
                    "{name}: ({x},{y}) is {d:.2}px inside but only {v}"
                );
            } else if d > 1.5 {
                outside += 1;
                assert_eq!(v, 0, "{name}: ({x},{y}) is {d:.2}px outside but lit {v}");
            } else if d > 0.1 && d < 0.6 {
                fringe += 1;
                assert!(
                    v > 0,
                    "{name}: ({x},{y}) is {d:.2}px outside the edge, unlit (AA cut off)"
                );
            }
        }
    }
    assert!(
        inside > 5_000 && outside > 50_000 && fringe > 100,
        "{name}: {inside}/{outside}/{fringe}"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn rotated_chrome_uses_the_sdf_path_and_lands_where_the_transform_says() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("rotated chrome device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu_gameui::shared_font_system(),
    );
    std::fs::create_dir_all("test_output").ok();

    let cases: [(&str, DrawRotated); 5] = [
        ("paint_quad", |list, r| {
            list.paint_quad(
                r,
                QuadStyle {
                    background: Background::Solid([1.0; 4]),
                    border_widths: EdgeWidths::default(),
                    border_color: [1.0; 4],
                    corner_radii: CornerRadii::uniform(10.0),
                },
            )
        }),
        ("rounded_rect", |list, r| {
            list.rounded_rect(r, 10.0, [1.0; 4])
        }),
        ("chrome_rect", |list, r| {
            list.chrome_rect(r, 10.0, 2.0, [1.0; 4], [1.0; 4])
        }),
        ("chrome_rect_gradient", |list, r| {
            list.chrome_rect_gradient(r, 10.0, 2.0, [1.0; 4], [1.0; 4], [1.0; 4])
        }),
        // A 40px border on a 70px-tall rect leaves no inner hole, so the
        // outline covers the whole shape and the same pixel checks apply.
        ("rounded_rect_outline", |list, r| {
            list.rounded_rect_outline(r, 10.0, 40.0, [1.0; 4])
        }),
    ];
    for (name, draw) in cases {
        assert_rotated_shape(&device, &queue, &mut ui, name, draw, 0.35, 1.0);
        let scaled = format!("{name}_scaled");
        assert_rotated_shape(&device, &queue, &mut ui, &scaled, draw, -0.6, 1.5);
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn composable_quad_renders_affine_unequal_rounded_border_gradient_and_clip() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("composable quad device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");
    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        font_system.clone(),
    );
    let mut list = DrawList::with_font_system(font_system);
    list.push_clip(Rect::new(80.0, 55.0, 95.0, 85.0));
    list.translate(120.0, 95.0);
    list.rotate(0.35);
    list.set_tint([0.8, 1.0, 0.7, 1.0]);
    list.paint_quad(
        Rect::new(-55.0, -35.0, 130.0, 80.0),
        QuadStyle {
            background: Background::LinearGradient {
                start: [0.9, 0.15, 0.1, 1.0],
                end: [0.1, 0.2, 0.9, 1.0],
                axis: GradientAxis::Horizontal,
            },
            border_widths: EdgeWidths::new(3.0, 12.0, 7.0, 18.0),
            border_color: [0.9, 0.9, 0.2, 1.0],
            corner_radii: CornerRadii::new(24.0, 6.0, 18.0, 10.0),
        },
    );
    let image = render_list(&device, &queue, &mut ui, &list);
    let pixel = |x: u32, y: u32| -> &[u8] {
        let start = ((y * W + x) * 4) as usize;
        &image[start..start + 4]
    };
    assert!(pixel(110, 90)[3] > 100, "quad interior was not painted");
    assert_eq!(pixel(180, 90), &[0, 0, 0, 255], "clip leaked to the right");
    assert_eq!(pixel(100, 45), &[0, 0, 0, 255], "clip leaked above");
    let colored = image
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|rgba| rgba[0] > 20 || rgba[1] > 20 || rgba[2] > 20)
        .count();
    assert!(colored > 1_000, "too few composable quad pixels: {colored}");
}

/// A quad rounded on one side only (a sheet footer: square top, rounded
/// bottom) must fill evenly. Each half of the shape picks its own corner
/// radii, and the row where the halves meet must not show a seam.
#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn one_side_rounded_quad_has_no_seam_where_its_halves_meet() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("seam device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");
    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        font_system.clone(),
    );
    let fill = [0.4, 0.6, 0.8, 1.0];
    // Odd and even heights, and a half-pixel offset: the halves meet on a
    // pixel centre, a pixel edge, or in between.
    let quads = [
        (
            Rect::new(20.0, 20.0, 200.0, 43.0),
            CornerRadii::new(0.0, 0.0, 6.0, 6.0),
        ),
        (
            Rect::new(20.0, 80.0, 200.0, 44.0),
            CornerRadii::new(0.0, 0.0, 6.0, 6.0),
        ),
        (
            Rect::new(20.0, 140.5, 200.0, 43.0),
            CornerRadii::new(6.0, 6.0, 0.0, 0.0),
        ),
        (
            Rect::new(240.0, 20.0, 43.0, 200.0),
            CornerRadii::new(0.0, 6.0, 6.0, 0.0),
        ),
    ];
    let mut list = DrawList::with_font_system(font_system);
    for (rect, radii) in quads {
        list.paint_quad_background(rect, Background::Solid(fill), radii);
    }
    let image = render_list(&device, &queue, &mut ui, &list);
    let pixel = |x: u32, y: u32| -> [u8; 4] {
        let start = ((y * W + x) * 4) as usize;
        image[start..start + 4].try_into().unwrap()
    };
    for (rect, _) in quads {
        let (cx, cy) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        let expected = pixel(cx as u32 - 12, cy as u32 - 12);
        // Every pixel fully inside the quad, along both centre lines.
        for y in (rect.y.ceil() as u32 + 1)..(rect.bottom().floor() as u32 - 1) {
            let got = pixel(cx as u32, y);
            assert_eq!(got, expected, "{rect:?}: seam at row {y}");
        }
        for x in (rect.x.ceil() as u32 + 1)..(rect.right().floor() as u32 - 1) {
            let got = pixel(x, cy as u32);
            assert_eq!(got, expected, "{rect:?}: seam at column {x}");
        }
    }
}
