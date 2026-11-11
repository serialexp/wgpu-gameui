//! What both gameui harnesses share: an offscreen 1920x1080 target on the
//! high-performance adapter, a frame drawn into it, and reading it back.

use ui_compare_common::{HEIGHT, WIDTH};
use wgpu_gameui::{DrawList, FontSystemHandle, RenderStats, UiRenderer};

/// The device and the target every frame is drawn into.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub target: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
}

impl Gpu {
    /// Open the high-performance adapter (naming it on stderr, as
    /// `library`) and make the target.
    pub fn new(library: &str) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("no GPU adapter");
        let info = adapter.get_info();
        eprintln!(
            "{library} adapter: {} ({:?}, {:?})",
            info.name, info.device_type, info.backend
        );
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("device");
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
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
        Self {
            device,
            queue,
            target,
            view,
            size,
            format,
        }
    }

    /// A renderer for the target, drawing text with `fonts`, comparing
    /// frames when [`frame_reuse`] says so.
    pub fn renderer(&self, fonts: FontSystemHandle) -> UiRenderer {
        let mut ui = UiRenderer::new(&self.device, &self.queue, self.format, fonts);
        ui.set_frame_comparison(frame_reuse());
        ui
    }

    /// One frame's CPU work: `render` (shaping, instance encoding, upload,
    /// command encode) and `submit`. The GPU is polled without waiting, so
    /// its execution is not in what a caller times.
    pub fn draw(&self, ui: &mut UiRenderer, list: &DrawList) -> RenderStats {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        ui.begin_frame();
        let stats = ui.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.view,
            self.size,
            1.0,
            list,
        );
        self.queue.submit(Some(encoder.finish()));
        self.device.poll(wgpu::Maintain::Poll);
        stats
    }

    /// Wait for the GPU to finish what was submitted.
    pub fn wait(&self) {
        self.device.poll(wgpu::Maintain::Wait);
    }

    /// Read the target back as RGBA bytes (blocking), to check what was
    /// drawn.
    pub fn read(&self) -> Vec<u8> {
        // 1920 * 4 is a multiple of 256, so rows need no padding.
        let bytes_per_row = self.size.0 * 4;
        assert_eq!(bytes_per_row % 256, 0);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row * self.size.1) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.size.1),
                },
            },
            wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.device.poll(wgpu::Maintain::Wait);
        let data = buffer.slice(..).get_mapped_range().to_vec();
        buffer.unmap();
        data
    }
}

/// Whether the renderer compares each frame with the last and replays the
/// plan of a list in which nothing moved (`UiRenderer::set_frame_comparison`,
/// off by default): `UI_COMPARE_REUSE=1`. Results are then reported under
/// the library's name with `+r`.
pub fn frame_reuse() -> bool {
    std::env::var_os("UI_COMPARE_REUSE").is_some_and(|v| v == "1")
}

/// `name`, or `name+r` with [`frame_reuse`] on.
pub fn library(name: &str) -> String {
    if frame_reuse() {
        format!("{name}+r")
    } else {
        name.to_string()
    }
}

/// What the renderer did in a scene's last frame.
pub fn render_counts(r: &RenderStats) -> [(&'static str, u64); 6] {
    [
        ("lists_identical", r.lists_identical as u64),
        ("plans_replayed", r.plans_replayed as u64),
        ("boxes_computed", r.boxes_computed),
        ("boxes_reused", r.boxes_reused),
        ("units_ordered", r.units_ordered as u64),
        ("glyphs_placed", r.glyphs_placed as u64),
    ]
}

/// A colour as 8-bit RGB, as straight RGBA in `0..=1`.
pub fn rgba([r, g, b]: [u8; 3]) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}
