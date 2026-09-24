//! GPU readback tests for the colour pipeline: UI colours are sRGB-encoded and
//! blend in sRGB space like a browser, on both of the renderer's paths.
//!
//! * **Direct** (`Rgba8Unorm` target): the UI draws straight into the target.
//! * **Offscreen** (`Rgba8UnormSrgb` target): the UI draws into an internal
//!   layer and composites.
//!
//! Run with `DISPLAY=:0 cargo test --test color_pipeline -- --ignored`.

use wgpu_gameui::color::{hex, rgba8};
use wgpu_gameui::{DrawList, UiRenderer, shared_font_system};

const W: u32 = 64;
const H: u32 = 16;

fn device_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("color pipeline test device"),
            ..Default::default()
        },
        None,
    ))
    .ok()
}

/// Render `list` into a fresh `format` target cleared (via
/// [`UiRenderer::clear_color`]) to the sRGB-encoded `clear`, and read the
/// pixels back as bytes. For an `*Srgb` target the bytes are what the GPU
/// stored, i.e. sRGB-encoded — directly comparable with the direct path.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    clear: [f32; 4],
    list: &DrawList,
) -> (Vec<u8>, bool) {
    let mut ui = UiRenderer::new(device, queue, format, shared_font_system());
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("color pipeline target"),
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
        label: Some("color pipeline readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("color pipeline encoder"),
    });
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(ui.clear_color(clear)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
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
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&data[start..start + (W * 4) as usize]);
    }
    (pixels, ui.uses_offscreen_layer())
}

fn rgb_at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// CSS "source-over" of a straight sRGB `top` (alpha `a`) over opaque `under`,
/// computed on sRGB-encoded bytes — what Chromium does.
fn browser_over(top: [f32; 4], under: [f32; 4]) -> [u8; 3] {
    let a = top[3];
    std::array::from_fn(|c| ((top[c] * a + under[c] * (1.0 - a)) * 255.0).round() as u8)
}

fn assert_close(what: &str, got: [u8; 3], want: [u8; 3]) {
    let off = got
        .iter()
        .zip(want)
        .map(|(g, w)| g.abs_diff(w))
        .max()
        .unwrap();
    assert!(off <= 1, "{what}: got {got:?}, want {want:?}");
}

const BACKGROUND: u32 = 0x0a0d0f;
const PLATE: u32 = 0x3ebfc6;
const WHITE_30: [f32; 4] = rgba8([0xff, 0xff, 0xff], 0.3);
const BLACK_25: [f32; 4] = rgba8([0, 0, 0], 0.25);

/// The test scene, left to right in 16px columns:
/// 0. untouched clear
/// 1. opaque plate
/// 2. white 30% over the plate (UI over UI)
/// 3. black 25% over the plate, then white 30% over that (a stack)
///
/// plus a white 30% strip straight over the clear on the bottom rows.
fn scene() -> DrawList {
    let mut list = DrawList::new();
    list.quad(16.0, 0.0, 48.0, 12.0, hex(PLATE));
    list.quad(32.0, 0.0, 16.0, 12.0, WHITE_30);
    list.quad(48.0, 0.0, 16.0, 12.0, BLACK_25);
    list.quad(48.0, 0.0, 16.0, 12.0, WHITE_30);
    list.quad(0.0, 12.0, 16.0, 4.0, WHITE_30);
    list
}

#[test]
#[ignore = "needs a GPU adapter"]
fn translucent_ui_blends_like_the_browser_on_both_paths() {
    let Some((device, queue)) = device_queue() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let bg = hex(BACKGROUND);
    let plate = hex(PLATE);
    let over_plate = browser_over(WHITE_30, plate);
    let dark = browser_over(BLACK_25, plate).map(|c| c as f32 / 255.0);
    let stacked = browser_over(WHITE_30, [dark[0], dark[1], dark[2], 1.0]);
    let list = scene();

    for (format, offscreen) in [
        (wgpu::TextureFormat::Rgba8Unorm, false),
        (wgpu::TextureFormat::Rgba8UnormSrgb, true),
    ] {
        let (px, used_offscreen) = render(&device, &queue, format, bg, &list);
        assert_eq!(
            used_offscreen, offscreen,
            "{format:?} picked the wrong path"
        );
        let tag = |what: &str| format!("{format:?} {what}");
        assert_close(&tag("clear"), rgb_at(&px, 8, 4), [0x0a, 0x0d, 0x0f]);
        assert_close(&tag("opaque plate"), rgb_at(&px, 24, 4), [0x3e, 0xbf, 0xc6]);
        assert_close(&tag("white 30% over plate"), rgb_at(&px, 40, 4), over_plate);
        assert_close(&tag("stacked translucency"), rgb_at(&px, 56, 4), stacked);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn translucent_ui_over_the_host_target_follows_the_target_space() {
    let Some((device, queue)) = device_queue() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let bg = hex(BACKGROUND);
    let list = scene();

    // Direct: the host's own pixels blend in sRGB too — exactly the browser.
    let (px, _) = render(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, bg, &list);
    assert_close(
        "direct: white 30% over the clear",
        rgb_at(&px, 8, 14),
        browser_over(WHITE_30, bg),
    );

    // Offscreen: the composite blends the UI layer over the host in linear
    // light (documented on `UiRenderer`), so it is lighter than the browser.
    let (px, _) = render(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        bg,
        &list,
    );
    let lin = |c: f32| wgpu_gameui::color::srgb_channel_to_linear(c);
    let enc = |c: f32| wgpu_gameui::color::linear_channel_to_srgb(c);
    let linear_mix: [u8; 3] = std::array::from_fn(|c| {
        let mixed = lin(WHITE_30[c]) * 0.3 + lin(bg[c]) * 0.7;
        (enc(mixed) * 255.0).round() as u8
    });
    assert_close(
        "offscreen: white 30% over the clear",
        rgb_at(&px, 8, 14),
        linear_mix,
    );
}
