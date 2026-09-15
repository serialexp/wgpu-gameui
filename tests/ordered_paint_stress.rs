//! Headless regression for ordered color runs corrupting soup geometry.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, UiRenderer};

const W: u32 = 256;
const H: u32 = 256;

#[test]
#[ignore = "requires a GPU adapter"]
fn many_alternating_runs_do_not_produce_screen_sized_triangle() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("request device");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let fonts = wgpu_gameui::shared_font_system();
    let mut renderer = UiRenderer::new(&device, &queue, format, fonts.clone());
    let mut list = DrawList::with_font_system(fonts);

    for row in 0..160 {
        let y = 4.0 + (row % 30) as f32 * 8.0;
        list.chrome_rect(
            Rect::new(8.0, y, 24.0, 5.0),
            1.0,
            1.0,
            [0.2, 0.3, 0.5, 1.0],
            [0.6, 0.7, 0.9, 1.0],
        );
        list.line([38.0, y], [46.0, y + 4.0], 1.0, [0.0, 1.0, 0.0, 1.0]);
        list.text(wgpu_gameui::TextBlock::new("run", 52.0, y).with_size(6.0));
    }

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ordered paint stress target"),
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
    let view = target.create_view(&Default::default());
    let bytes_per_row = W * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ordered paint stress readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
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
    // One submission = one frame (see `UiRenderer::begin_frame`).
    renderer.begin_frame();
    renderer.render(&device, &queue, &mut encoder, &view, (W, H), 1.0, &list);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
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
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let pixels = slice.get_mapped_range();

    let white = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 245 && p[1] > 245 && p[2] > 245)
        .count();
    assert!(
        white < (W * H / 20) as usize,
        "unexpected screen-sized white geometry: {white} white pixels"
    );
}
