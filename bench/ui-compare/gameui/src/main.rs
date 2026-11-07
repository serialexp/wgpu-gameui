//! wgpu-gameui: the CPU cost of one frame of the shared scene.
//!
//! Timed per frame: build a fresh `DrawList` (a rounded rectangle and a
//! `TextBlock` per cell), then `UiRenderer::render` (text shaping, instance
//! encoding, buffer upload, command encode) and `Queue::submit`. The GPU is
//! polled without waiting, so GPU execution is not in the sample.

use ui_compare_common::*;
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, TextBlock, TextContent, UiRenderer};

fn main() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter");
    let info = adapter.get_info();
    eprintln!(
        "gameui adapter: {} ({:?}, {:?})",
        info.name, info.device_type, info.backend
    );
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let fonts = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, fonts.clone());
    let size = (WIDTH as u32, HEIGHT as u32);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
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

    for n in counts() {
        let grid = Grid::new(n);
        let labels: Vec<TextContent> = (0..n).map(|i| label(i).into()).collect();
        // The frame must contain the scene: the renderer's own stats are the
        // check, since a harness that silently drew nothing would be fast.
        {
            let mut list = DrawList::with_font_system(fonts.clone());
            build(&mut list, &grid, &labels, 0, true);
            let mut encoder = device.create_command_encoder(&Default::default());
            ui.begin_frame();
            let stats = ui.render(&device, &queue, &mut encoder, &view, size, 1.0, &list);
            queue.submit(Some(encoder.finish()));
            eprintln!(
                "gameui {n} cells: paint_runs={} draw_calls={} primitives={}",
                stats.paint_runs, stats.draw_calls, stats.primitives
            );
            assert!(
                stats.primitives >= n,
                "gameui drew {} primitives",
                stats.primitives
            );
            check_frame("gameui", n, &read_target(&device, &queue, &target, size));
        }
        for (scenario, animated) in [("static", false), ("animated", true)] {
            let stats = measure(|frame| {
                let mut list = DrawList::with_font_system(fonts.clone());
                build(&mut list, &grid, &labels, frame, animated);
                let mut encoder = device.create_command_encoder(&Default::default());
                ui.begin_frame();
                ui.render(&device, &queue, &mut encoder, &view, size, 1.0, &list);
                queue.submit(Some(encoder.finish()));
                device.poll(wgpu::Maintain::Poll);
            });
            device.poll(wgpu::Maintain::Wait);
            report("gameui", scenario, n, &stats);
        }
    }
}

/// The scene as a `DrawList`: a rounded rectangle and a label per cell.
fn build(list: &mut DrawList, grid: &Grid, labels: &[TextContent], frame: u64, animated: bool) {
    let (w, h) = grid.size();
    for i in 0..grid.n {
        let (x, y) = grid.origin(i);
        let [r, g, b] = fill(i, frame, animated);
        list.rounded_rect(
            Rect::new(x, y, w, h),
            grid.radius,
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
        );
        let mut block = TextBlock::new(labels[i].clone(), x + 1.0, y + 1.0)
            .with_line_height(grid.font_px * 1.25)
            .with_max_width(w);
        block.font_size = grid.font_px;
        list.text(block);
    }
}

/// Read the target back as RGBA bytes (blocking), to check what was drawn.
fn read_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    size: (u32, u32),
) -> Vec<u8> {
    // 1920 * 4 is a multiple of 256, so rows need no padding.
    let bytes_per_row = size.0 * 4;
    assert_eq!(bytes_per_row % 256, 0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * size.1) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size.1),
            },
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    data
}
