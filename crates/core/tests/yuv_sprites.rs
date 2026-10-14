//! Ignored headless tests for pictures that change: sprites updated in place,
//! and YUV sprites converted to colour in the shader.
//!
//! GPU-only, like `icon_instancing` — run with:
//! ```
//! DISPLAY=:0 cargo test --test yuv_sprites -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::render::chroma_size;
use wgpu_gameui::{HeadlessGpu, Placement, YuvColour, YuvMatrix, YuvPlanes, YuvRange};

const SIZE: (u32, u32) = (32, 32);
const WHITE: [f32; 4] = [1.0; 4];

/// The captured pixel at `(x, y)`, without alpha.
fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// Whether two colours are within `tolerance` codes on every channel.
fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= tolerance)
}

/// An 8×8 picture whose left half has the codes `left` and right half `right`
/// (each `[y, u, v]`), as a decoder would give it.
fn halves(left: [u8; 3], right: [u8; 3]) -> YuvPlanes<'static> {
    let plane = |width: u32, height: u32, channel: usize| -> Vec<u8> {
        (0..height)
            .flat_map(|_| 0..width)
            .map(|x| {
                if x < width / 2 {
                    left[channel]
                } else {
                    right[channel]
                }
            })
            .collect()
    };
    let (cw, ch) = chroma_size(8, 8);
    YuvPlanes::packed(8, plane(8, 8, 0), plane(cw, ch, 1), plane(cw, ch, 2))
}

/// What the colour conversion says `codes` look like, in 8-bit sRGB.
fn expected(colour: YuvColour, codes: [u8; 3]) -> [u8; 3] {
    colour
        .to_rgb()
        .apply(codes.map(|c| f32::from(c) / 255.0))
        .map(|c| (c * 255.0).round() as u8)
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn yuv_sprites_draw_in_the_colours_their_settings_give() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    // BT.709 limited-range red and green.
    let (red, green) = ([63, 102, 240], [173, 42, 26]);
    for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709] {
        for range in [YuvRange::Limited, YuvRange::Full] {
            let colour = YuvColour { matrix, range };
            let sprite = gpu
                .renderer()
                .create_yuv_sprite("video", 8, 8, halves(red, green), colour)
                .unwrap();
            let mut list = gpu.draw_list();
            list.image(sprite, Rect::new(0.0, 0.0, 32.0, 32.0), WHITE);
            let pixels = gpu.capture(&list, SIZE);
            // Away from the middle, where the halves blend.
            for (x, codes) in [(4, red), (28, green)] {
                let want = expected(colour, codes);
                let got = at(&pixels, x, 16);
                assert!(
                    near(got, want, 2),
                    "{matrix:?} {range:?} at x={x}: got {got:?}, want {want:?}"
                );
            }
            if colour
                == (YuvColour {
                    matrix: YuvMatrix::Bt709,
                    range: YuvRange::Limited,
                })
            {
                assert!(near(at(&pixels, 4, 16), [255, 0, 0], 3));
                assert!(near(at(&pixels, 28, 16), [0, 255, 0], 3));
            }
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_yuv_sprite_is_tinted_and_clipped_like_any_image() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let grey = [126, 128, 128];
    let colour = YuvColour::assumed_for(8, 8);
    let sprite = gpu
        .renderer()
        .create_yuv_sprite("video", 8, 8, halves(grey, grey), colour)
        .unwrap();
    let mut list = gpu.draw_list();
    list.push_clip(Rect::new(0.0, 0.0, 16.0, 32.0));
    list.image(
        sprite,
        Rect::new(0.0, 0.0, 32.0, 32.0),
        [1.0, 0.0, 0.0, 1.0],
    );
    list.pop_clip();
    let pixels = gpu.capture(&list, SIZE);
    let [r, ..] = expected(colour, grey);
    assert!(near(at(&pixels, 8, 16), [r, 0, 0], 2), "tinted red");
    assert_eq!(at(&pixels, 24, 16), [0, 0, 0], "clipped away");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn an_updated_sprite_shows_its_new_pixels_from_the_same_texture() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let solid = |rgba: [u8; 4]| rgba.repeat(8 * 8);
    let sprite = gpu
        .renderer()
        .load_sprite_rgba8("frame", 8, 8, solid([255, 0, 0, 255]), Placement::Own)
        .unwrap();
    let mut list = gpu.draw_list();
    list.image(sprite, Rect::new(0.0, 0.0, 32.0, 32.0), WHITE);

    let first = gpu.capture(&list, SIZE);
    assert_eq!(at(&first, 16, 16), [255, 0, 0]);
    assert_eq!(gpu.renderer().frame_stats().textures_created, 1);

    gpu.renderer()
        .update_sprite_rgba8(sprite, solid([0, 0, 255, 255]))
        .unwrap();
    let second = gpu.capture(&list, SIZE);
    assert_eq!(at(&second, 16, 16), [0, 0, 255]);
    let stats = gpu.renderer().frame_stats();
    assert_eq!(stats.texture_uploads, 1, "the new pixels went up");
    assert_eq!(stats.textures_created, 0, "into the texture it had");
    assert_eq!(gpu.renderer().sprite_id("frame"), Some(sprite));

    // Nothing changed: nothing is uploaded.
    gpu.capture(&list, SIZE);
    assert_eq!(gpu.renderer().frame_stats().texture_uploads, 0);
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn an_updated_yuv_sprite_shows_its_new_planes_from_the_same_textures() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let colour = YuvColour::assumed_for(8, 8);
    let (black, white) = ([16, 128, 128], [235, 128, 128]);
    let sprite = gpu
        .renderer()
        .create_yuv_sprite("video", 8, 8, halves(black, black), colour)
        .unwrap();
    let mut list = gpu.draw_list();
    list.image(sprite, Rect::new(0.0, 0.0, 32.0, 32.0), WHITE);

    let first = gpu.capture(&list, SIZE);
    assert!(near(at(&first, 16, 16), [0, 0, 0], 1));
    assert_eq!(gpu.renderer().frame_stats().textures_created, 1);

    gpu.renderer()
        .update_yuv_sprite(sprite, halves(white, white))
        .unwrap();
    let second = gpu.capture(&list, SIZE);
    assert!(near(at(&second, 16, 16), [255, 255, 255], 1));
    let stats = gpu.renderer().frame_stats();
    assert_eq!(stats.texture_uploads, 1);
    assert_eq!(stats.textures_created, 0);
    assert_eq!(stats.texture_bytes_uploaded, 64 + 16 + 16, "three planes");

    assert!(gpu.renderer().unload_sprite(sprite));
    assert_eq!(gpu.renderer().sprite_id("video"), None);
}

/// A plane `width`×`height` in rows `stride` apart, `value(x)` along each row
/// and junk in the padding.
fn padded_plane(width: u32, height: u32, stride: u32, value: impl Fn(u32) -> u8) -> Vec<u8> {
    (0..height)
        .flat_map(|_| (0..stride).map(|x| if x < width { value(x) } else { 0xEE }))
        .collect()
}

/// What a linear-filtering, clamp-to-edge sampler reads from a plane at `uv`.
fn sample(plane: &[u8], (width, height): (u32, u32), stride: u32, [u, v]: [f32; 2]) -> f32 {
    let at = |x: i64, y: i64| {
        let x = x.clamp(0, i64::from(width) - 1) as usize;
        let y = y.clamp(0, i64::from(height) - 1) as usize;
        f32::from(plane[y * stride as usize + x])
    };
    let (x, y) = (u * width as f32 - 0.5, v * height as f32 - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn an_odd_sized_picture_with_padded_rows_keeps_its_colour_in_place() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    // 7×5, so the colour planes are 4×3 and their last column covers only the
    // picture's last column. Grey, but blue in that last column.
    let (width, height) = (7, 5);
    let (cw, ch) = chroma_size(width, height);
    let (y_stride, chroma_stride) = (9, 6);
    let y = padded_plane(width, height, y_stride, |x| if x == 6 { 41 } else { 126 });
    let u = padded_plane(cw, ch, chroma_stride, |x| if x == 3 { 240 } else { 128 });
    let v = padded_plane(cw, ch, chroma_stride, |x| if x == 3 { 110 } else { 128 });
    let colour = YuvColour::assumed_for(width, height);
    let planes = YuvPlanes {
        y: y.clone().into(),
        u: u.clone().into(),
        v: v.clone().into(),
        y_stride,
        chroma_stride,
    };
    let sprite = gpu
        .renderer()
        .create_yuv_sprite("odd", width, height, planes, colour)
        .unwrap();
    let mut list = gpu.draw_list();
    // Four pixels to a picture pixel.
    list.image(sprite, Rect::new(0.0, 0.0, 28.0, 20.0), WHITE);
    let pixels = gpu.capture(&list, SIZE);

    // What the colour at a pixel should be, with the colour planes' UV
    // scaled to the picture (`true`) or taken as the picture's (`false`).
    let predict = |px: u32, py: u32, scaled: bool| -> [u8; 3] {
        let uv = [(px as f32 + 0.5) / 28.0, (py as f32 + 0.5) / 20.0];
        let chroma_uv = if scaled {
            [
                uv[0] * width as f32 / (2 * cw) as f32,
                uv[1] * height as f32 / (2 * ch) as f32,
            ]
        } else {
            uv
        };
        let codes = [
            sample(&y, (width, height), y_stride, uv),
            sample(&u, (cw, ch), chroma_stride, chroma_uv),
            sample(&v, (cw, ch), chroma_stride, chroma_uv),
        ];
        colour
            .to_rgb()
            .apply(codes.map(|c| c / 255.0))
            .map(|c| (c * 255.0).round() as u8)
    };
    let mut discriminating = 0;
    for (px, py) in [(2, 2), (21, 9), (22, 10), (26, 10), (27, 19)] {
        let want = predict(px, py, true);
        let got = at(&pixels, px, py);
        assert!(
            near(got, want, 3),
            "({px}, {py}): got {got:?}, want {want:?}"
        );
        discriminating += usize::from(!near(predict(px, py, false), want, 6));
    }
    assert!(
        discriminating > 0,
        "the pixels checked tell the scaled colour planes from unscaled ones"
    );
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn a_nine_slice_of_a_yuv_sprite_is_skipped_not_drawn_wrong() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let white = [235, 128, 128];
    let sprite = gpu
        .renderer()
        .create_yuv_sprite(
            "video",
            8,
            8,
            halves(white, white),
            YuvColour::assumed_for(8, 8),
        )
        .unwrap();
    let panel = gpu
        .renderer()
        .register_nine_slice("panel", sprite, [2, 2, 2, 2]);
    let mut list = gpu.draw_list();
    list.nine_slice_id(panel, 0.0, 0.0, 32.0, 32.0, WHITE);
    let pixels = gpu.capture(&list, SIZE);
    assert_eq!(at(&pixels, 16, 16), [0, 0, 0]);
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn an_id_reused_before_its_first_upload_gets_one_texture() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let solid = |rgba: [u8; 4]| rgba.repeat(8 * 8);
    let first = gpu
        .renderer()
        .load_sprite_rgba8("a", 8, 8, solid([255, 0, 0, 255]), Placement::Own)
        .unwrap();
    assert!(gpu.renderer().unload_sprite(first));
    let second = gpu
        .renderer()
        .load_sprite_rgba8("b", 8, 8, solid([0, 255, 0, 255]), Placement::Own)
        .unwrap();
    assert_eq!(second, first, "the id is reused");
    let mut list = gpu.draw_list();
    list.image(second, Rect::new(0.0, 0.0, 32.0, 32.0), WHITE);
    let pixels = gpu.capture(&list, SIZE);
    assert_eq!(at(&pixels, 16, 16), [0, 255, 0]);
    let stats = gpu.renderer().frame_stats();
    assert_eq!((stats.textures_created, stats.texture_uploads), (1, 1));
}
