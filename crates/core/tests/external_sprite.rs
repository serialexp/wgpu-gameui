//! GPU-native preview lifecycle, sampling parity and absence of CPU uploads.
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{HeadlessGpu, Image, ImageFit, SpriteError};

fn texture(device: &wgpu::Device, queue: &wgpu::Queue, color: [u8; 4], edge: u32) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("external sprite test"),
        size: wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels: Vec<_> = (0..edge * edge).flat_map(|_| color).collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(edge * 4),
            rows_per_image: Some(edge),
        },
        wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: 1,
        },
    );
    texture
}

#[test]
fn external_view_replaces_in_place_samples_and_unloads_without_uploads() {
    let Some(mut gpu) = HeadlessGpu::new() else {
        return;
    };
    let device = gpu.device().clone();
    let queue = gpu.queue().clone();
    let red = texture(&device, &queue, [255, 0, 0, 255], 2);
    let view = red.create_view(&Default::default());
    let id = gpu
        .renderer()
        .register_texture_view(&device, "preview", &view, 2, 2)
        .unwrap();
    let mut list = gpu.draw_list();
    Image::sprite(id)
        .fit(ImageFit::Stretch)
        .draw(Rect::new(0.0, 0.0, 32.0, 32.0), &mut list);
    let pixels = gpu.capture(&list, (32, 32));
    assert_eq!(
        &pixels[(16 * 32 + 16) * 4..(16 * 32 + 16) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(gpu.renderer().frame_stats().texture_uploads, 0);
    let green = texture(&device, &queue, [0, 255, 0, 255], 4);
    let view = green.create_view(&Default::default());
    let replaced = gpu
        .renderer()
        .register_texture_view(&device, "preview", &view, 4, 4)
        .unwrap();
    assert_eq!(id, replaced);
    // Dropping the producer's handles must not invalidate the sampled view.
    drop(view);
    drop(green);
    drop(red);
    let pixels = gpu.capture(&list, (32, 32));
    assert_eq!(
        &pixels[(16 * 32 + 16) * 4..(16 * 32 + 16) * 4 + 4],
        &[0, 255, 0, 255]
    );
    assert_eq!(gpu.renderer().frame_stats().texture_uploads, 0);
    let placeholder = texture(&device, &queue, [0, 0, 0, 255], 1);
    let view = placeholder.create_view(&Default::default());
    assert_eq!(
        gpu.renderer()
            .register_texture_view(&device, "preview", &view, 0, 1),
        Err(SpriteError::Empty)
    );
    assert_eq!(gpu.renderer().sprite_id("preview"), Some(id));
    assert!(gpu.renderer().unload_sprite(id));
    assert!(!gpu.renderer().unload_sprite(id));
    assert_eq!(gpu.renderer().sprite_id("preview"), None);
    let pixels = gpu.capture(&list, (32, 32));
    assert!(pixels.iter().all(|&byte| byte == 0));
}
