//! egui: the CPU cost of one frame of the shared scene.
//!
//! Two lines per case:
//!
//! - `egui`: `Context::run_pass` (the cell shapes go through the painter, text
//!   is laid out into galleys) and `Context::tessellate` (shapes to triangle
//!   meshes). Everything egui does before a backend sees the meshes.
//! - `egui+wgpu`: the same, then what `egui-wgpu` does with them: apply the
//!   texture delta, `update_buffers`, record the render pass and `submit`. The
//!   GPU is polled without waiting. This is the line to compare with gameui,
//!   whose number includes its own upload and submit.

use egui::{Color32, CornerRadius, FontId, Pos2, RawInput, Rect, Vec2};
use egui_wgpu::wgpu;
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use ui_compare_common::*;

fn main() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("no GPU adapter");
    let info = adapter.get_info();
    eprintln!(
        "egui adapter: {} ({:?}, {:?})",
        info.name, info.device_type, info.backend
    );
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let size = [WIDTH as u32, HEIGHT as u32];
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
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
    let mut renderer = Renderer::new(&device, format, RendererOptions::default());
    let screen_desc = ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: 1.0,
    };

    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH, HEIGHT));

    // One frame of the scene: returns the shapes' output.
    let run =
        |ctx: &egui::Context, n: usize, grid: &Grid, labels: &[String], frame: u64, mode: Mode| {
            let input = RawInput {
                screen_rect: Some(screen),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            ctx.run_pass(input, |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                let (w, h) = grid.size();
                let font = FontId::proportional(grid.font_px);
                for i in 0..n {
                    let (x, y) = grid.origin(i);
                    let [r, g, b] = fill_in(mode, i, frame);
                    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
                    painter.rect_filled(
                        rect,
                        CornerRadius::same(grid.radius as u8),
                        Color32::from_rgb(r, g, b),
                    );
                    painter.text(
                        Pos2::new(x + 1.0, y + 1.0),
                        egui::Align2::LEFT_TOP,
                        &labels[i],
                        font.clone(),
                        Color32::WHITE,
                    );
                }
            })
        };

    // Two contexts: egui hands out each texture upload (the font atlas) once,
    // so the CPU-only runs, which drop it, must not share a context with the
    // runs that upload it to the GPU.
    let ctx_gpu = egui::Context::default();
    // egui's `default_fonts` feature is not published for 0.37, so load the
    // shared typeface by hand into both contexts.
    for c in [&ctx, &ctx_gpu] {
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert(
            "plex".into(),
            std::sync::Arc::new(egui::FontData::from_static(FONT_BYTES)),
        );
        fonts
            .families
            .insert(egui::FontFamily::Proportional, vec!["plex".into()]);
        c.set_fonts(fonts);
    }

    // One full GPU frame: run, tessellate, upload, record and submit.
    let mut gpu_frame = |make: &dyn Fn(&egui::Context) -> egui::FullOutput| {
        let out = make(&ctx_gpu);
        let meshes = ctx_gpu.tessellate(out.shapes, out.pixels_per_point);
        for (id, deltas) in &out.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&device, &queue, *id, delta);
            }
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        let extra = renderer.update_buffers(&device, &queue, &mut encoder, &meshes, &screen_desc);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, &meshes, &screen_desc);
        }
        queue.submit(extra.into_iter().chain(Some(encoder.finish())));
        let _ = device.poll(wgpu::PollType::Poll);
        for id in &out.textures_delta.free {
            renderer.free_texture(id);
        }
    };

    for n in counts() {
        let grid = Grid::new(n);
        let labels: Vec<String> = (0..n).map(label).collect();

        // Sanity: the frame must really contain the scene, so look at it.
        // (egui's first passes run before its fonts are ready, so warm up first.)
        for frame in 0..4 {
            gpu_frame(&|c| run(c, n, &grid, &labels, frame, Mode::Animated));
        }
        check_frame(
            "egui",
            n,
            &read_target(&device, &queue, &target, (size[0], size[1])),
        );

        for (scenario, mode) in MODES {
            let stats = measure(|frame| {
                let out = run(&ctx, n, &grid, &labels, frame, mode);
                let meshes = ctx.tessellate(out.shapes, out.pixels_per_point);
                std::hint::black_box(&meshes);
            });
            report("egui", scenario, n, &stats);
        }

        for (scenario, mode) in MODES {
            let stats = measure(|frame| gpu_frame(&|c| run(c, n, &grid, &labels, frame, mode)));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            report("egui+wgpu", scenario, n, &stats);
        }
    }

    if text_scenes_wanted() {
        let doc: Vec<String> = (0..DOC_LINES).map(doc_line).collect();
        let para = para_text();
        for name in ["scroll", "churn", "para"] {
            let scene = |c: &egui::Context, frame: u64| run_text(c, name, frame, &doc, &para);
            for frame in 0..4 {
                gpu_frame(&|c| scene(c, 40 + frame));
            }
            check_text_frame(
                "egui",
                name,
                &read_target(&device, &queue, &target, (size[0], size[1])),
            );
            let stats = measure(|frame| {
                let out = scene(&ctx, frame);
                let meshes = ctx.tessellate(out.shapes, out.pixels_per_point);
                std::hint::black_box(&meshes);
            });
            report_scene("egui", name, &stats);
            let stats = measure(|frame| gpu_frame(&|c| scene(c, frame)));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            report_scene("egui+wgpu", name, &stats);
        }
    }
}

/// One frame of a text scene (see the common crate): `scroll`, `churn` or `para`.
fn run_text(
    ctx: &egui::Context,
    scene: &str,
    frame: u64,
    doc: &[String],
    para: &str,
) -> egui::FullOutput {
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH, HEIGHT));
    let input = RawInput {
        screen_rect: Some(screen),
        time: Some(frame as f64 / 60.0),
        ..Default::default()
    };
    ctx.run_pass(input, |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let [r, g, b] = TEXT_BG;
        painter.rect_filled(screen, 0.0, Color32::from_rgb(r, g, b));
        if scene == "para" {
            let galley = painter.layout(
                para.to_owned(),
                FontId::proportional(PARA_PX),
                Color32::WHITE,
                PARA_W,
            );
            painter.galley(Pos2::new(PARA_X, para_y(frame)), galley, Color32::WHITE);
            return;
        }
        let font = FontId::proportional(LINE_PX);
        for c in 0..LINE_COLS {
            for row in 0..LINE_ROWS {
                let (i, y) = if scene == "scroll" {
                    scrolled_line(c, row, frame)
                } else {
                    (scrolled_line(c, row, 0).0, scrolled_line(c, row, 0).1)
                };
                let pos = Pos2::new(c as f32 * LINE_COL_W, y);
                if scene == "scroll" {
                    painter.text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        &doc[i],
                        font.clone(),
                        Color32::WHITE,
                    );
                } else {
                    painter.text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        churn_line(i, frame),
                        font.clone(),
                        Color32::WHITE,
                    );
                }
            }
        }
    })
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
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    buffer.unmap();
    data
}
