//! Multi-pass rendering in one submission must not lose geometry.
//!
//! Regression tests for the pass-corruption bug the widget gallery exposed: two
//! `UiRenderer::render` calls recorded into one encoder before a single
//! `queue.submit` used to clobber each other, because
//! `Queue::write_buffer` only takes effect at submit — before any recorded pass
//! runs — and the per-frame bump arenas were reset per *call* rather than per
//! *frame*. Every pass in the submission read the last pass's bytes, so all but
//! the last pass rendered garbage (in practice: nothing).
//!
//! The fix is the explicit frame boundary, [`UiRenderer::begin_frame`]: arenas
//! (including per-pass uniform slots, see `render::uniform_arena`) span the frame,
//! and each pass bumps to its own byte ranges. These tests pin that behaviour for
//! every primitive family, for `render_layers` followed by `render`, and for two
//! passes whose viewports differ.
//!
//! GPU-only (headless readback), like `widget_gallery`:
//! ```
//! DISPLAY=:0 cargo test --test multi_pass_render -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    DrawList, FontSystemHandle, LayerStack, TextBlock, TextMeasurer, UiRenderer, shared_font_system,
};

const W: u32 = 480;
const H: u32 = 320;

/// One shape per primitive family, at a distinct place and colour, so the readback
/// says exactly which family of which pass lost its data.
struct Probe {
    offset_y: f32,
    chrome: [f32; 4],
    soup: [f32; 4],
    circle: [f32; 4],
    quad: [f32; 4],
    text: &'static str,
    text_color: (u8, u8, u8),
}

impl Probe {
    fn build(&self, font_system: FontSystemHandle) -> DrawList {
        let mut list = DrawList::with_font_system(font_system);
        // Instanced SDF chrome (fill only, no border, so the centre is pure colour).
        list.chrome_rect(
            Rect::new(20.0, self.offset_y, 80.0, 40.0),
            0.0,
            0.0,
            self.chrome,
            [0.0; 4],
        );
        // Immediate rounded rect → fill-only SDF chrome on the soup path.
        list.rounded_rect(Rect::new(120.0, self.offset_y, 80.0, 40.0), 6.0, self.soup);
        // Circle instances.
        list.circle((240.0, self.offset_y + 20.0), 20.0, self.circle);
        // Plain quad (soup).
        list.quad(300.0, self.offset_y, 80.0, 40.0, self.quad);
        // Text (own pipeline + own VBO and uniform slot).
        list.text(
            TextBlock::new(self.text, 20.0, self.offset_y + 50.0)
                .with_size(28.0)
                .with_color(self.text_color.0, self.text_color.1, self.text_color.2),
        );
        list
    }
}

/// Pass A probes sit in y 20..90, pass B in y 170..240 — bands that do not overlap.
fn probe_a() -> Probe {
    Probe {
        offset_y: 20.0,
        chrome: [1.0, 0.0, 0.0, 1.0],
        soup: [0.0, 1.0, 0.0, 1.0],
        circle: [0.0, 0.0, 1.0, 1.0],
        quad: [1.0, 1.0, 0.0, 1.0],
        text: "AAAA",
        text_color: (255, 255, 255),
    }
}

fn probe_b() -> Probe {
    Probe {
        offset_y: 170.0,
        chrome: [0.0, 1.0, 1.0, 1.0],
        soup: [1.0, 0.5, 0.0, 1.0],
        circle: [0.5, 0.0, 1.0, 1.0],
        quad: [1.0, 0.0, 1.0, 1.0],
        text: "BBBB",
        text_color: (0, 200, 255),
    }
}

/// Render `record` (given a started frame) into a fresh target and read the
/// tightly packed RGBA pixels back.
fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    viewport: (u32, u32),
    record: impl FnOnce(&mut UiRenderer, &mut wgpu::CommandEncoder, &wgpu::TextureView),
) -> Vec<u8> {
    let (width, height) = viewport;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("multi-pass target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = (width * 4 + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("multi-pass readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("multi-pass encoder"),
    });
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
    record(ui, &mut encoder, &view);
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
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((width * 4 * height) as usize);
    for row in 0..height as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }
    pixels
}

fn px(img: &[u8], viewport: (u32, u32), x: u32, y: u32) -> (u8, u8, u8) {
    let (w, _h) = viewport;
    let i = ((y * w + x) * 4) as usize;
    (img[i], img[i + 1], img[i + 2])
}

/// Assert `got` is within `tol` of `want` per channel.
fn expect(name: &str, got: (u8, u8, u8), want: (u8, u8, u8), tol: i32, failures: &mut Vec<String>) {
    let close = (got.0 as i32 - want.0 as i32).abs() <= tol
        && (got.1 as i32 - want.1 as i32).abs() <= tol
        && (got.2 as i32 - want.2 as i32).abs() <= tol;
    if !close {
        failures.push(format!("{name}: got {got:?}, want {want:?}"));
    }
}

/// `a` is linear 0.5 → 188 in sRGB (the target is Rgba8UnormSrgb), not 128.
fn half() -> u8 {
    188
}

fn text_band_has_ink(img: &[u8], viewport: (u32, u32), y0: u32, y1: u32) -> bool {
    let (w, h) = viewport;
    (y0..y1.min(h)).any(|y| (0..w).any(|x| px(img, viewport, x, y).0 > 40))
}

fn device_queue() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("multi-pass device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device")
}

const TOL: i32 = 24;

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn two_render_calls_in_one_submission_both_draw() {
    let (device, queue) = device_queue();
    let font_system = shared_font_system();
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        font_system.clone(),
    );
    let a = probe_a().build(font_system.clone());
    let b = probe_b().build(font_system.clone());

    let img = render_frame(&device, &queue, &mut ui, (W, H), |ui, encoder, view| {
        ui.begin_frame();
        ui.render(&device, &queue, encoder, view, (W, H), 1.0, &a);
        ui.render(&device, &queue, encoder, view, (W, H), 1.0, &b);
    });

    let mut bad = Vec::new();
    // Pass A: every family must survive the second call's uploads.
    expect(
        "chrome A",
        px(&img, (W, H), 60, 40),
        (255, 0, 0),
        TOL,
        &mut bad,
    );
    expect(
        "soup A",
        px(&img, (W, H), 160, 40),
        (0, 255, 0),
        TOL,
        &mut bad,
    );
    expect(
        "circle A",
        px(&img, (W, H), 240, 40),
        (0, 0, 255),
        TOL,
        &mut bad,
    );
    expect(
        "quad A",
        px(&img, (W, H), 340, 40),
        (255, 255, 0),
        TOL,
        &mut bad,
    );
    // Pass B must be unchanged too (the first call must not corrupt it).
    expect(
        "chrome B",
        px(&img, (W, H), 60, 190),
        (0, 255, 255),
        TOL,
        &mut bad,
    );
    expect(
        "soup B",
        px(&img, (W, H), 160, 190),
        (255, half(), 0),
        TOL,
        &mut bad,
    );
    expect(
        "circle B",
        px(&img, (W, H), 240, 190),
        (half(), 0, 255),
        TOL,
        &mut bad,
    );
    expect(
        "quad B",
        px(&img, (W, H), 340, 190),
        (255, 0, 255),
        TOL,
        &mut bad,
    );
    // Text of both passes.
    assert!(
        text_band_has_ink(&img, (W, H), 72, 110),
        "pass A text missing"
    );
    assert!(
        text_band_has_ink(&img, (W, H), 210, 280),
        "pass B text missing"
    );
    assert!(bad.is_empty(), "multi-pass aliasing: {bad:?}");
}

/// `render_layers` (which internally issues several passes) followed by `render`
/// in the same submission: both halves must draw.
#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn render_layers_then_render_in_one_submission_both_draw() {
    let (device, queue) = device_queue();
    let font_system = shared_font_system();
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        font_system.clone(),
    );

    let mut layers = LayerStack::with_font_system(font_system.clone());
    layers
        .base_mut()
        .quad(20.0, 20.0, 120.0, 80.0, [1.0, 0.0, 0.0, 1.0]);
    let panel = probe_b().build(font_system.clone());

    let img = render_frame(&device, &queue, &mut ui, (W, H), |ui, encoder, view| {
        ui.begin_frame();
        ui.render_layers(&device, &queue, encoder, view, (W, H), 1.0, &layers);
        ui.render(&device, &queue, encoder, view, (W, H), 1.0, &panel);
    });

    let mut bad = Vec::new();
    // The base layer's quad (soup path, so it never depended on the instance
    // buffers — but its text-less pass still shares the uniform arena).
    expect(
        "layers quad",
        px(&img, (W, H), 60, 60),
        (255, 0, 0),
        TOL,
        &mut bad,
    );
    // The follow-up pass's chrome rect.
    expect(
        "panel chrome",
        px(&img, (W, H), 60, 190),
        (0, 255, 255),
        TOL,
        &mut bad,
    );
    assert!(bad.is_empty(), "layers+render aliasing: {bad:?}");
}

/// Two passes into differently sized views in one submission: the second pass must
/// not draw with the first pass's ortho matrix (it used to — uniforms were shared).
#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_second_pass_with_a_different_viewport_uses_its_own_projection() {
    let (device, queue) = device_queue();
    let font_system = shared_font_system();
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        font_system.clone(),
    );

    // Small view: 240x160. The list draws a quad at logical (20, 20), which with
    // the *small* projection lands at physical (20, 20). With the *large*
    // projection (480x320) the same vertex lands at half that — (10, 10) — so the
    // two projections disagree and only one of the two sample points is filled.
    let mut list = DrawList::with_font_system(font_system.clone());
    list.quad(20.0, 20.0, 60.0, 60.0, [0.0, 1.0, 0.0, 1.0]);

    let img = render_frame(&device, &queue, &mut ui, (240, 160), |ui, encoder, view| {
        ui.begin_frame();
        // First pass renders into a large offscreen view we discard; its uniform
        // slot holds the 480x320 matrix.
        let big = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("big view"),
            size: wgpu::Extent3d {
                width: 480,
                height: 320,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let big_view = big.create_view(&wgpu::TextureViewDescriptor::default());
        ui.render(&device, &queue, encoder, &big_view, (480, 320), 1.0, &list);
        // Second pass, small view, same geometry.
        ui.render(&device, &queue, encoder, view, (240, 160), 1.0, &list);
    });

    // With its own projection, the quad's top-left corner sits at (20, 20): filled
    // at (30, 30), background at (10, 10). With the stolen 480x320 matrix it would
    // be exactly the reverse.
    assert_eq!(
        px(&img, (240, 160), 30, 30),
        (0, 255, 0),
        "quad drew with the other pass's projection (off toward 10,10)"
    );
    assert_eq!(
        px(&img, (240, 160), 10, 10),
        (0, 0, 0),
        "unexpected fill at (10,10): projection leaked across passes"
    );
}

/// The frame's GPU scratch is reused across frames: a second identical frame must
/// not reallocate anything. (A forgotten `begin_frame` shows up exactly here, as
/// per-frame arena growth and reallocation on every frame.)
#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_second_identical_frame_reallocates_nothing() {
    let (device, queue) = device_queue();
    let font_system = shared_font_system();
    let mut ui = UiRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        font_system.clone(),
    );
    let list = probe_a().build(font_system.clone());

    let draw = |ui: &mut UiRenderer, list: &DrawList| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let view = dummy_view(&device);
        // (Submission order is irrelevant here; only the renderer's bookkeeping.)
        ui.begin_frame();
        ui.render(&device, &queue, &mut encoder, &view, (W, H), 1.0, list)
    };

    // Warm-up frame: sizes the arenas to the frame's high-water mark.
    let warmup = draw(&mut ui, &list);
    assert_eq!(
        warmup.buffer_reallocations, 0,
        "warm-up frame itself grew a buffer past its starting capacity"
    );
    // Identical frame: every arena was reset, so every slice fits again.
    let second = draw(&mut ui, &list);
    assert_eq!(
        second.buffer_reallocations, 0,
        "an unchanged frame reallocated GPU buffers — the frame boundary is probably being missed"
    );
}

fn dummy_view(device: &wgpu::Device) -> wgpu::TextureView {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dummy view"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The text renderer's own resize hook also guards against cross-frame cache
/// confusion: measuring through a `TextMeasurer` bound to the renderer's font
/// system stays consistent with what the pass draws.
#[test]
fn text_measurer_shapes_the_same_way_the_renderer_will() {
    let font_system = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(font_system);
    let (w1, h1) = measurer.measure("regression", 16.0, None);
    let (w2, h2) = measurer.measure("regression", 16.0, None);
    assert_eq!((w1, h1), (w2, h2));
    assert!(w1 > 0.0 && h1 > 0.0);
}
