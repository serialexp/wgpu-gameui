//! Headless GPU correctness test for the instanced icon/image path.
//!
//! Icons, sprites, and cropped images all render through one instanced draw:
//! a unit-quad base mesh + one `IconInstance` per icon, with the four
//! world-space corners baked in and bilinearly interpolated in `vs_icon` (so
//! rotation/scale/shear come for free). This replaces re-tessellating 6
//! verts/icon into a textured soup and re-uploading it every frame.
//!
//! There's no independent immediate path left to diff against, so this is a
//! golden-pixel test: render a 4-quadrant sprite and assert the sampled colors
//! land where the UV mapping says they should — for a plain draw, a cropped
//! draw, and a rotated draw, from a texture of the sprite's own and from a
//! shared atlas, and for draws that switch between the two.
//!
//! GPU-only, like `widget_gallery` — run with:
//! ```
//! DISPLAY=:0 cargo test --test icon_instancing -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, FontSystemHandle, Placement, SpriteId, UiRenderer};

const W: u32 = 256;
const H: u32 = 256;
const SPRITE: u32 = 32;

const RED: [u8; 4] = [220, 40, 40, 255];
const GREEN: [u8; 4] = [40, 220, 40, 255];
const BLUE: [u8; 4] = [40, 40, 220, 255];
const YELLOW: [u8; 4] = [220, 220, 40, 255];

/// A `SPRITE×SPRITE` sprite split into four solid color quadrants:
/// TL=red, TR=green, BL=blue, BR=yellow.
fn quadrant_sprite() -> Vec<u8> {
    let s = SPRITE;
    let h = s / 2;
    let mut out = vec![0u8; (s * s * 4) as usize];
    for y in 0..s {
        for x in 0..s {
            let c = match (x < h, y < h) {
                (true, true) => RED,
                (false, true) => GREEN,
                (true, false) => BLUE,
                (false, false) => YELLOW,
            };
            let idx = ((y * s + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

/// The quadrant sprite in a texture of its own and in a shared atlas: drawing
/// must look the same from either.
fn quadrant_sprites(ui: &mut UiRenderer) -> [(&'static str, SpriteId); 2] {
    let atlas = ui.create_atlas(1024);
    [("own", Placement::Own), ("atlas", Placement::Atlas(atlas))].map(|(name, placement)| {
        let sprite = ui
            .load_sprite_rgba8(name, SPRITE, SPRITE, quadrant_sprite(), placement)
            .expect("load the quadrant sprite");
        (name, sprite)
    })
}

fn solid(color: [u8; 4]) -> Vec<u8> {
    color.repeat((SPRITE * SPRITE) as usize)
}

fn render_list(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ui: &mut UiRenderer,
    list: &DrawList,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("icon target"),
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
        label: Some("icon readback"),
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

fn sample(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// Nearest of the four quadrant colors to `c` (so sRGB round-trip ±a few LSB
/// doesn't matter; the four colors are far apart).
fn classify(c: [u8; 4]) -> &'static str {
    let cands = [("R", RED), ("G", GREEN), ("B", BLUE), ("Y", YELLOW)];
    let mut best = "?";
    let mut best_d = i32::MAX;
    for (name, q) in cands {
        let d = (c[0] as i32 - q[0] as i32).pow(2)
            + (c[1] as i32 - q[1] as i32).pow(2)
            + (c[2] as i32 - q[2] as i32).pow(2);
        if d < best_d {
            best_d = d;
            best = name;
        }
    }
    best
}

fn setup() -> (wgpu::Device, wgpu::Queue, UiRenderer, FontSystemHandle) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter (run under DISPLAY=:0)");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("icon device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system: FontSystemHandle = wgpu_gameui::shared_font_system();
    let ui = UiRenderer::new(&device, &queue, format, font_system.clone());
    (device, queue, ui, font_system)
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_icon_maps_quadrants() {
    let (device, queue, mut ui, font_system) = setup();
    for (placed, sprite) in quadrant_sprites(&mut ui) {
        // Plain full draw into a 200×200 rect at (28,28). Quadrant centers should
        // sample their source quadrant colors.
        let dest = Rect::new(28.0, 28.0, 200.0, 200.0);
        let mut list = DrawList::with_font_system(font_system.clone());
        list.image(sprite, dest, [1.0; 4]);
        let img = render_list(&device, &queue, &mut ui, &list);

        std::fs::create_dir_all("test_output").ok();
        image::RgbaImage::from_raw(W, H, img.clone()).and_then(|i| {
            i.save(format!("test_output/icon_full_{placed}.png"))
                .ok()
                .map(|_| i)
        });

        // Quadrant centers: quarter/three-quarter of the dest in each axis.
        let q = |fx: f32, fy: f32| {
            sample(
                &img,
                (dest.x + dest.width * fx) as u32,
                (dest.y + dest.height * fy) as u32,
            )
        };
        assert_eq!(
            classify(q(0.25, 0.25)),
            "R",
            "{placed}: top-left should be red"
        );
        assert_eq!(
            classify(q(0.75, 0.25)),
            "G",
            "{placed}: top-right should be green"
        );
        assert_eq!(
            classify(q(0.25, 0.75)),
            "B",
            "{placed}: bottom-left should be blue"
        );
        assert_eq!(
            classify(q(0.75, 0.75)),
            "Y",
            "{placed}: bottom-right should be yellow"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_icon_crop_samples_subrect() {
    let (device, queue, mut ui, font_system) = setup();
    for (placed, sprite) in quadrant_sprites(&mut ui) {
        // Crop the top-left quarter (all red) and stretch it across the dest:
        // every quadrant center should now read red.
        let dest = Rect::new(28.0, 28.0, 200.0, 200.0);
        let mut list = DrawList::with_font_system(font_system.clone());
        list.image_cropped(sprite, dest, [0.0, 0.0, 0.5, 0.5], [1.0; 4]);
        let img = render_list(&device, &queue, &mut ui, &list);

        let q = |fx: f32, fy: f32| {
            sample(
                &img,
                (dest.x + dest.width * fx) as u32,
                (dest.y + dest.height * fy) as u32,
            )
        };
        for (fx, fy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            assert_eq!(
                classify(q(fx, fy)),
                "R",
                "{placed}: cropped TL-quarter should be red everywhere"
            );
        }
    }
}

/// Draws that switch between a texture of a sprite's own and a shared atlas
/// keep their paint order: each switch starts a new draw call rather than the
/// sprites being regrouped by texture. Only the small sprite of its own counts
/// as a batch an atlas would have saved.
#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn switching_textures_keeps_paint_order() {
    let (device, queue, mut ui, font_system) = setup();
    let atlas = ui.create_atlas(256);
    let red = ui
        .load_sprite_rgba8("red", SPRITE, SPRITE, solid(RED), Placement::Atlas(atlas))
        .unwrap();
    let blue = ui
        .load_sprite_rgba8("blue", SPRITE, SPRITE, solid(BLUE), Placement::Atlas(atlas))
        .unwrap();
    let green = ui
        .load_sprite_rgba8("green", SPRITE, SPRITE, solid(GREEN), Placement::Own)
        .unwrap();

    // Red everywhere, green over the left half, blue over the top-left quarter.
    let dest = Rect::new(28.0, 28.0, 200.0, 200.0);
    let (half_w, half_h) = (dest.width / 2.0, dest.height / 2.0);
    let mut list = DrawList::with_font_system(font_system.clone());
    list.image(red, dest, [1.0; 4]);
    list.image(
        green,
        Rect::new(dest.x, dest.y, half_w, dest.height),
        [1.0; 4],
    );
    list.image(blue, Rect::new(dest.x, dest.y, half_w, half_h), [1.0; 4]);
    let img = render_list(&device, &queue, &mut ui, &list);

    let q = |fx: f32, fy: f32| {
        classify(sample(
            &img,
            (dest.x + dest.width * fx) as u32,
            (dest.y + dest.height * fy) as u32,
        ))
    };
    assert_eq!(q(0.25, 0.25), "B", "blue was drawn last, over green");
    assert_eq!(q(0.25, 0.75), "G", "green over red");
    assert_eq!(q(0.75, 0.25), "R");
    assert_eq!(q(0.75, 0.75), "R");
    let stats = ui.frame_stats();
    assert_eq!(stats.small_texture_batches, 1, "{stats:?}");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn instanced_icon_rotation_permutes_quadrants() {
    let (device, queue, mut ui, font_system) = setup();
    let sprite = ui
        .load_sprite_rgba8("quad", SPRITE, SPRITE, quadrant_sprite(), Placement::Own)
        .unwrap();

    let dest = Rect::new(28.0, 28.0, 200.0, 200.0);
    let cx = dest.x + dest.width / 2.0;
    let cy = dest.y + dest.height / 2.0;

    // Unrotated arrangement.
    let mut plain = DrawList::with_font_system(font_system.clone());
    plain.image(sprite, dest, [1.0; 4]);
    let img_plain = render_list(&device, &queue, &mut ui, &plain);

    // Rotated 90° about the dest center — the baked-in corners are transformed,
    // so the quadrants must rotate with no fallback / no lost content.
    let mut rot = DrawList::with_font_system(font_system.clone());
    rot.push_transform();
    rot.translate(cx, cy);
    rot.rotate(std::f32::consts::FRAC_PI_2);
    rot.image(
        sprite,
        Rect::new(
            -dest.width / 2.0,
            -dest.height / 2.0,
            dest.width,
            dest.height,
        ),
        [1.0; 4],
    );
    rot.pop_transform();
    let img_rot = render_list(&device, &queue, &mut ui, &rot);

    image::RgbaImage::from_raw(W, H, img_rot.clone())
        .and_then(|i| i.save("test_output/icon_rotated.png").ok().map(|_| i));

    let q = |img: &[u8], fx: f32, fy: f32| {
        classify(sample(
            img,
            (dest.x + dest.width * fx) as u32,
            (dest.y + dest.height * fy) as u32,
        ))
    };

    // All four colors still present after rotation (nothing clipped/lost).
    let mut rotated: Vec<&str> = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
        .iter()
        .map(|&(fx, fy)| q(&img_rot, fx, fy))
        .collect();
    rotated.sort_unstable();
    assert_eq!(rotated, vec!["B", "G", "R", "Y"], "all 4 quadrants present");

    // Arrangement must differ from unrotated (rotation actually applied). The
    // top-left quadrant in particular should no longer be red.
    let plain_tl = q(&img_plain, 0.25, 0.25);
    let rot_tl = q(&img_rot, 0.25, 0.25);
    assert_eq!(plain_tl, "R");
    assert_ne!(rot_tl, "R", "rotation should move red out of the top-left");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_sprite_loaded_under_an_image_key_retires_the_image() {
    let (_device, _queue, mut ui, _font_system) = setup();
    ui.load_image_rgba8("shared-key", 2, 2, &solid(RED)[..16], Placement::Own)
        .unwrap();
    assert!(ui.has_image("shared-key"));
    ui.load_sprite_rgba8("shared-key", 2, 2, &solid(GREEN)[..16], Placement::Own)
        .unwrap();
    // The image's sprite was freed for the name, so its cache entry is gone
    // too: a load under the key loads again rather than handing out a freed
    // (and maybe reused) id.
    assert!(!ui.has_image("shared-key"));
    let again = ui
        .load_image_rgba8("shared-key", 2, 2, &solid(BLUE)[..16], Placement::Own)
        .unwrap();
    assert_eq!(ui.sprite_id("shared-key"), Some(again));
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn uploads_are_counted_as_they_happen() {
    let (device, queue, mut ui, font_system) = setup();
    let atlas = ui.create_atlas(256);
    let shared = ui
        .load_image_rgba8(
            "shared",
            SPRITE,
            SPRITE,
            solid(RED),
            Placement::Atlas(atlas),
        )
        .unwrap();
    let own = ui
        .load_sprite_rgba8("own", SPRITE, SPRITE, solid(GREEN), Placement::Own)
        .unwrap();
    let draw = |ui: &mut UiRenderer, sprites: &[SpriteId]| {
        let mut list = DrawList::with_font_system(font_system.clone());
        for &sprite in sprites {
            list.image(sprite, Rect::new(0.0, 0.0, 32.0, 32.0), [1.0; 4]);
        }
        render_list(&device, &queue, ui, &list);
        ui.frame_stats()
    };
    let stats = draw(&mut ui, &[shared, own]);
    let sprite_bytes = u64::from(SPRITE * SPRITE * 4);
    assert_eq!(stats.atlas_uploads, 1, "{stats:?}");
    assert_eq!(stats.texture_uploads, 1, "{stats:?}");
    assert_eq!(stats.texture_bytes_uploaded, sprite_bytes, "{stats:?}");
    // Nothing changed: nothing goes up again.
    let stats = draw(&mut ui, &[shared, own]);
    assert_eq!(
        (stats.atlas_uploads, stats.texture_uploads),
        (0, 0),
        "{stats:?}"
    );
    // An atlas left empty is not uploaded again (its texture is let go), and
    // one that gets a sprite again is.
    ui.unload_image("shared");
    let stats = draw(&mut ui, &[own]);
    assert_eq!(stats.atlas_uploads, 0, "{stats:?}");
    let again = ui
        .load_image_rgba8(
            "shared",
            SPRITE,
            SPRITE,
            solid(BLUE),
            Placement::Atlas(atlas),
        )
        .unwrap();
    let stats = draw(&mut ui, &[again]);
    assert_eq!(stats.atlas_uploads, 1, "{stats:?}");
}
