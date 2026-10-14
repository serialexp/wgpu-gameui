//! Headless regressions for ordered color runs corrupting soup geometry.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{BoxShadow, CornerRadii, DrawList, RenderStats, UiRenderer};

const W: u32 = 256;
const H: u32 = 256;

fn render(list: &DrawList) -> Option<(Vec<u8>, RenderStats)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .ok()?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let fonts = wgpu_gameui::shared_font_system();
    let mut renderer = UiRenderer::new(&device, &queue, format, fonts);
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
    renderer.begin_frame();
    let stats = renderer.render(&device, &queue, &mut encoder, &view, (W, H), 1.0, list);
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
    let pixels = slice.get_mapped_range().to_vec();
    Some((pixels, stats))
}

fn rgba_at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * W + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().unwrap()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn one_draw_preserves_many_overlapping_source_over_operations() {
    const PAIRS: usize = 128;
    const SHADOW: [f32; 4] = [0.9, 0.05, 0.1, 0.02];
    const CHROME: [f32; 4] = [0.05, 0.2, 0.95, 0.015];
    let rect = Rect::new(48.0, 48.0, 96.0, 96.0);
    let mut unified = DrawList::new();
    let mut ordered_draw_reference = DrawList::new();

    for _ in 0..PAIRS {
        // A collapsed inset hole covers the complete element, giving a stable
        // full-coverage center pixel after every source-over operation.
        unified.box_shadow_inset(
            rect,
            CornerRadii::default(),
            BoxShadow {
                spread: 100.0,
                color: SHADOW,
                inset: true,
                ..Default::default()
            },
        );
        unified.chrome_rect(rect, 0.0, 0.0, CHROME, CHROME);
    }
    for pair in 0..PAIRS {
        ordered_draw_reference.box_shadow_inset(
            rect,
            CornerRadii::default(),
            BoxShadow {
                spread: 100.0,
                color: SHADOW,
                inset: true,
                ..Default::default()
            },
        );
        // Off-sample soup is an ordering barrier, forcing the established
        // multi-draw path to serve as the same-GPU compositing oracle.
        let y = 2.0 + (pair % 200) as f32;
        ordered_draw_reference.line([220.0, y], [224.0, y], 1.0, [0.0, 0.0, 0.0, 0.0]);
        ordered_draw_reference.chrome_rect(rect, 0.0, 0.0, CHROME, CHROME);
        ordered_draw_reference.line([228.0, y], [232.0, y], 1.0, [0.0, 0.0, 0.0, 0.0]);
    }

    let Some((unified_pixels, stats)) = render(&unified) else {
        return;
    };
    let Some((reference_pixels, reference_stats)) = render(&ordered_draw_reference) else {
        return;
    };
    assert_eq!(stats.primitives, PAIRS * 2);
    assert_eq!(stats.paint_runs, 1);
    assert_eq!(stats.draw_calls, 1);
    assert_eq!(stats.color_runs, 1);
    assert_eq!(stats.render_passes, 1);
    assert!(reference_stats.draw_calls > PAIRS * 2);

    assert_eq!(
        rgba_at(&unified_pixels, 96, 96),
        rgba_at(&reference_pixels, 96, 96),
        "one heterogeneous draw must match explicit ordered source-over draws"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn many_alternating_runs_do_not_produce_screen_sized_triangle() {
    let fonts = wgpu_gameui::shared_font_system();
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

    let Some((pixels, _)) = render(&list) else {
        return;
    };
    let white = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 245 && p[1] > 245 && p[2] > 245)
        .count();
    assert!(
        white < (W * H / 20) as usize,
        "unexpected screen-sized white geometry: {white} white pixels"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn alternating_shadow_chrome_and_soup_preserves_pixels_and_stats() {
    const ROWS: usize = 10;
    let mut list = DrawList::new();
    for row in 0..ROWS {
        let y = 5.0 + row as f32 * 22.0;
        let shadow_color = if row % 2 == 0 {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 0.2, 1.0, 1.0]
        };
        let chrome_color = if row % 2 == 0 {
            [0.0, 0.8, 0.1, 1.0]
        } else {
            [0.8, 0.7, 0.0, 1.0]
        };
        list.box_shadow_outset(
            Rect::new(8.0, y, 24.0, 9.0),
            CornerRadii::default(),
            BoxShadow {
                offset: [0.0, 7.0],
                spread: 2.0,
                color: shadow_color,
                ..Default::default()
            },
        );
        list.chrome_rect(
            Rect::new(8.0, y, 24.0, 9.0),
            2.0,
            1.0,
            chrome_color,
            [1.0, 1.0, 1.0, 1.0],
        );
        list.line([42.0, y + 4.5], [60.0, y + 4.5], 8.0, [0.0, 1.0, 1.0, 1.0]);
    }
    // Flush the final soup run without adding another soup primitive.
    list.chrome_rect(Rect::new(70.0, 5.0, 5.0, 5.0), 0.0, 0.0, [1.0; 4], [1.0; 4]);

    let Some((pixels, stats)) = render(&list) else {
        return;
    };
    assert_eq!(stats.shadow_instances, ROWS);
    assert_eq!(stats.primitives, ROWS * 4 + 1);
    assert_eq!(stats.paint_runs, ROWS * 2 + 1);
    assert_eq!(stats.draw_calls, ROWS * 2 + 1);
    assert_eq!(stats.color_runs, ROWS * 2 + 1);
    assert_eq!(stats.render_passes, 1);

    for row in 0..ROWS {
        let y = 5 + row as u32 * 22;
        let shadow = rgba_at(&pixels, 20, y + 15);
        if row % 2 == 0 {
            assert!(
                shadow[0] > 240 && shadow[2] < 20,
                "red shadow row {row}: {shadow:?}"
            );
        } else {
            assert!(
                shadow[2] > 240 && shadow[0] < 20,
                "blue shadow row {row}: {shadow:?}"
            );
        }
        let chrome = rgba_at(&pixels, 20, y + 4);
        assert!(
            chrome[0] > 20 || chrome[1] > 100,
            "chrome row {row}: {chrome:?}"
        );
        let soup = rgba_at(&pixels, 50, y + 4);
        assert!(
            soup[1] > 240 && soup[2] > 240 && soup[0] < 20,
            "soup row {row}: {soup:?}"
        );
    }
    assert_eq!(rgba_at(&pixels, 200, 240), [0, 0, 0, 255]);
}
