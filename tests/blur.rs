//! GPU readback tests for the backdrop blur (`UiRenderer::blur_backdrop`).
//!
//! These render through a real wgpu device (run under `DISPLAY=:0` / with a GPU
//! adapter available). They build a scene texture with a sharp vertical
//! black→white edge, blur it into an offscreen target, read the pixels back, and
//! assert the edge got smeared (and that a larger radius smears it wider).

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Backdrop, BlurParams, ColorEncoding, UiRenderer, shared_font_system};

const SIZE: u32 = 64;

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
            label: Some("blur test device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device")
}

/// How a blur test's scene is stored and sampled, and what it's drawn into.
#[derive(Clone, Copy, Debug)]
struct Setup {
    scene: wgpu::TextureFormat,
    encoding: ColorEncoding,
    target: wgpu::TextureFormat,
    /// Byte value of the scene's left half (the right half is 255).
    left: u8,
}

/// The classic swapchain setup: sRGB scene sampled as linear, sRGB target.
const SRGB_HOST: Setup = Setup {
    scene: wgpu::TextureFormat::Rgba8UnormSrgb,
    encoding: ColorEncoding::Linear,
    target: wgpu::TextureFormat::Rgba8UnormSrgb,
    left: 0,
};

/// A `SIZE`×`SIZE` scene: left half black, right half white (sharp edge at the
/// vertical midline). Returns the sampleable texture + its view.
fn edge_scene(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
    scene_with(device, queue, SRGB_HOST.scene, SRGB_HOST.left)
}

/// A `SIZE`×`SIZE` scene stored as `format`: left half `left`, right half
/// white.
fn scene_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    left: u8,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blur test scene"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let v = if x < SIZE / 2 { left } else { 255u8 };
            pixels[i] = v;
            pixels[i + 1] = v;
            pixels[i + 2] = v;
            pixels[i + 3] = 255;
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * SIZE),
            rows_per_image: Some(SIZE),
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Blur the edge scene over the full target with the given radius/downsample and
/// return the de-padded RGBA bytes of the result.
fn blur_to_pixels(radius: f32, downsample: u32) -> Vec<u8> {
    blur_with(SRGB_HOST, radius, downsample, [1.0, 1.0, 1.0, 1.0])
}

/// [`blur_to_pixels`] for an explicit [`Setup`] and tint.
fn blur_with(setup: Setup, radius: f32, downsample: u32, tint: [f32; 4]) -> Vec<u8> {
    let (device, queue) = device_queue();
    let format = setup.target;
    let mut ui = UiRenderer::new(&device, &queue, format, shared_font_system());

    let (_scene, scene_view) = scene_with(&device, &queue, setup.scene, setup.left);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blur test target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let row_stride = SIZE * 4;
    let bytes_per_row = (row_stride + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("blur test encoder"),
    });
    // Clear the target to a sentinel so a no-op blur would be obvious.
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 1.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    ui.blur_backdrop(
        &device,
        &queue,
        &mut encoder,
        &target_view,
        &Backdrop {
            view: &scene_view,
            size: (SIZE, SIZE),
            encoding: setup.encoding,
        },
        Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
        (SIZE, SIZE),
        1.0,
        &BlurParams {
            radius,
            downsample,
            tint,
        },
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
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    let mut pixels = Vec::with_capacity((row_stride * SIZE) as usize);
    for row in 0..SIZE as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&data[start..start + row_stride as usize]);
    }
    pixels
}

/// Red channel at (x, y) from a de-padded RGBA buffer.
fn red_at(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4) as usize]
}

#[test]
fn blur_smears_a_sharp_edge() {
    let px = blur_to_pixels(6.0, 1);
    let mid = SIZE / 2;

    // Far from the edge stays saturated black / white.
    assert!(red_at(&px, 2, mid) < 30, "far-left should stay dark");
    assert!(
        red_at(&px, SIZE - 3, mid) > 225,
        "far-right should stay bright"
    );

    // At the edge the value is an intermediate gray (proves the smear). It is
    // NOT the green clear sentinel — green's red channel would be ~0, but we
    // also require the *blue* channel to be low (gray, not the green clear).
    let edge = red_at(&px, mid, mid);
    assert!(
        edge > 30 && edge < 225,
        "edge column should be mid-gray, got {edge}"
    );
    let edge_blue = px[((mid * SIZE + mid) * 4 + 2) as usize];
    assert!(
        edge_blue > 30,
        "edge should be gray (blue present), not green clear"
    );
}

/// Flat areas come through unchanged, and the tint multiplies in sRGB space,
/// whichever way the scene is stored and whatever the target stores: the
/// encode/decode flags must line up for all four pairings.
#[test]
fn blur_output_is_colour_space_consistent() {
    use wgpu::TextureFormat::{Rgba8Unorm, Rgba8UnormSrgb};
    let pairings = [
        (Rgba8UnormSrgb, ColorEncoding::Linear, Rgba8UnormSrgb),
        (Rgba8UnormSrgb, ColorEncoding::Linear, Rgba8Unorm),
        (Rgba8Unorm, ColorEncoding::Srgb, Rgba8UnormSrgb),
        (Rgba8Unorm, ColorEncoding::Srgb, Rgba8Unorm),
    ];
    for (scene, encoding, target) in pairings {
        let setup = Setup {
            scene,
            encoding,
            target,
            left: 0x80,
        };
        let plain = blur_with(setup, 2.0, 1, [1.0, 1.0, 1.0, 1.0]);
        let grey = red_at(&plain, 4, SIZE / 2);
        assert!(
            grey.abs_diff(0x80) <= 1,
            "{setup:?}: flat #808080 came out as {grey:#04x}"
        );
        // Half tint on white: sRGB 1.0 × 0.5 → 128 (a linear multiply would
        // give 188 on an sRGB target).
        let tinted = blur_with(setup, 2.0, 1, [0.5, 0.5, 0.5, 1.0]);
        let half = red_at(&tinted, SIZE - 4, SIZE / 2);
        assert!(
            half.abs_diff(128) <= 1,
            "{setup:?}: white × 0.5 tint came out as {half}"
        );
    }
}

/// Count columns in the middle row whose red channel is strictly between the
/// two extremes — i.e. the width of the blurred transition band.
fn transition_band_width(pixels: &[u8]) -> u32 {
    let mid = SIZE / 2;
    (0..SIZE)
        .filter(|&x| {
            let r = red_at(pixels, x, mid);
            r > 30 && r < 225
        })
        .count() as u32
}

#[test]
fn larger_radius_widens_the_transition() {
    let narrow = transition_band_width(&blur_to_pixels(2.0, 1));
    let wide = transition_band_width(&blur_to_pixels(12.0, 1));
    assert!(narrow > 0, "even a small blur must produce some transition");
    assert!(
        wide > narrow,
        "radius 12 band ({wide}) should be wider than radius 2 band ({narrow})"
    );
}

/// Two `blur_backdrop` calls recorded into one submission must not clobber each
/// other's uniforms: each call's region keeps its own radius and tint. This used
/// to fail exactly like the multi-pass render bug — both calls wrote the same
/// uniform bytes, and the second call's params reached the GPU for both.
#[test]
fn two_blur_calls_in_one_submission_keep_their_own_params() {
    let (device, queue) = device_queue();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut ui = UiRenderer::new(&device, &queue, format, shared_font_system());
    let (_scene, scene_view) = edge_scene(&device, &queue);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blur two-call target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = ((SIZE * 4) + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur two-call readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("blur two-call encoder"),
    });
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 1.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    let backdrop = Backdrop {
        view: &scene_view,
        size: (SIZE, SIZE),
        encoding: ColorEncoding::Linear,
    };
    ui.begin_frame();
    // Left half: tiny radius (the sharp edge stays put), neutral tint.
    ui.blur_backdrop(
        &device,
        &queue,
        &mut encoder,
        &target_view,
        &backdrop,
        Rect::new(0.0, 0.0, SIZE as f32 / 2.0, SIZE as f32),
        (SIZE, SIZE),
        1.0,
        &BlurParams {
            radius: 1.0,
            downsample: 2,
            tint: [1.0, 1.0, 1.0, 1.0],
        },
    );
    // Right half: big radius (heavily smeared toward mid grey), red tint.
    ui.blur_backdrop(
        &device,
        &queue,
        &mut encoder,
        &target_view,
        &backdrop,
        Rect::new(SIZE as f32 / 2.0, 0.0, SIZE as f32 / 2.0, SIZE as f32),
        (SIZE, SIZE),
        1.0,
        &BlurParams {
            radius: 16.0,
            downsample: 2,
            tint: [1.0, 0.0, 0.0, 1.0],
        },
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
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for row in 0..SIZE as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&data[start..start + (SIZE * 4) as usize]);
    }

    let sample = |x: u32, y: u32| {
        let i = ((y * SIZE + x) * 4) as usize;
        (pixels[i], pixels[i + 1], pixels[i + 2])
    };

    // The left half was NOT blurred away (radius 1 keeps the near-black and
    // near-white halves) and must be neutral: any red tint there means the
    // second call's uniforms leaked into the first call's passes.
    let left_dark = sample(8, SIZE / 2);
    assert!(
        left_dark.0 < 40 && left_dark.1 < 40 && left_dark.2 < 40,
        "left region lost its own params: {left_dark:?} (expected near-black, neutral)"
    );
    // The right half must be visibly red-tinted: the first call's neutral tint
    // leaking here would show as grey.
    let right = sample(40, SIZE / 2);
    assert!(
        right.0 > 60 && right.0 > right.1 + 20 && right.0 > right.2 + 20,
        "right region lost its red tint: {right:?}"
    );
}
