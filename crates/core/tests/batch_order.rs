//! Ignored headless tests: a list drawn in batches looks exactly as it does
//! drawn in paint order (`docs/design/draw-call-batching.md`, Phase 6).
//!
//! Batching only regroups draws that can't change the same pixel, so the two
//! must match to the bit. Each scene is drawn at scales 1, 1.25 and 1.5, with
//! the view origin at zero, a little off zero (so content crosses the target's
//! edges), and around 1e5 with the content moved by as much (so it stays on
//! the target). Every batched plan is also checked against the brute-force
//! oracle as it is made.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::render::chroma_size;
use wgpu_gameui::{
    BoxShadow, Button, CornerRadii, DrawContext, DrawList, FocusState, HeadlessGpu, InputState,
    Placement, ScrollState, ScrollView, SpriteId, Stroke, StyleResolver, TextBlock, Theme,
    YuvColour, YuvPlanes,
};

const SIZE: (u32, u32) = (240, 160);
const SCALES: [f32; 3] = [1.0, 1.25, 1.5];
/// View origins, each with how far the content moves with it.
const ORIGINS: [([f32; 2], [f32; 2]); 3] = [
    ([0.0, 0.0], [0.0, 0.0]),
    ([7.0, 5.0], [0.0, 0.0]),
    ([1.0e5, 1.0e5], [1.0e5, 1.0e5]),
];

/// The sprites and nine-slice the scenes draw.
struct Art {
    red: SpriteId,
    blue: SpriteId,
    video: SpriteId,
}

fn solid(rgba: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    (0..w * h).flat_map(|_| rgba).collect()
}

fn load_art(gpu: &mut HeadlessGpu) -> Art {
    let ui = gpu.renderer();
    let atlas = ui.create_atlas(256);
    let red = ui
        .load_sprite_rgba8("red", 8, 8, solid([220, 40, 40, 255], 8, 8), Placement::Own)
        .unwrap();
    let blue = ui
        .load_sprite_rgba8(
            "blue",
            8,
            8,
            solid([40, 60, 220, 200], 8, 8),
            Placement::Atlas(atlas),
        )
        .unwrap();
    ui.load_sprite_rgba8(
        "green",
        8,
        8,
        solid([40, 200, 60, 255], 8, 8),
        Placement::Atlas(atlas),
    )
    .unwrap();
    let panel = ui
        .load_sprite_rgba8(
            "panel",
            12,
            12,
            solid([200, 200, 90, 230], 12, 12),
            Placement::Own,
        )
        .unwrap();
    ui.register_nine_slice("panel", panel, [4, 4, 4, 4]);
    let (cw, ch) = chroma_size(8, 8);
    let planes = YuvPlanes::packed(
        8,
        vec![150; 64],
        vec![90; (cw * ch) as usize],
        vec![170; (cw * ch) as usize],
    );
    let video = ui
        .create_yuv_sprite("video", 8, 8, planes, YuvColour::assumed_for(8, 8))
        .unwrap();
    Art { red, blue, video }
}

type Scene = fn(&mut DrawList, &Art);

fn button_grid(list: &mut DrawList, _: &Art) {
    let theme = Theme::default();
    let mut focus = FocusState::default();
    let input = InputState::default();
    let mut ctx = DrawContext::new(list, &mut focus, &theme, &input, 240.0, 160.0);
    for i in 0..12 {
        let (col, row) = ((i % 3) as f32, (i / 3) as f32);
        let rect = Rect::new(6.0 + 76.0 * col, 6.0 + 38.0 * row, 70.0, 30.0);
        Button::new("OK").draw(rect, &mut ctx);
    }
}

fn chrome_under_text(list: &mut DrawList, _: &Art) {
    for i in 0..4 {
        let x = 10.0 + 22.0 * i as f32;
        list.chrome_rect(
            Rect::new(x, 20.0 + 6.0 * i as f32, 60.0, 50.0),
            8.0,
            2.0,
            [0.1 + 0.2 * i as f32, 0.3, 0.6, 0.8],
            [1.0, 1.0, 1.0, 0.9],
        );
    }
    list.text(TextBlock::new("Over the boxes", 20.0, 40.0).with_size(16.0));
    list.chrome_rect(
        Rect::new(60.0, 44.0, 40.0, 30.0),
        4.0,
        0.0,
        [0.9, 0.5, 0.1, 0.7],
        [0.0; 4],
    );
}

fn shadowed_text_around_a_box(list: &mut DrawList, _: &Art) {
    let shadowed = |text: &str, x: f32, y: f32| {
        TextBlock::new(text, x, y)
            .with_size(18.0)
            .with_shadow(0, 0, 0, 200, 3.0, 3.0, 2.0)
    };
    list.text(shadowed("First", 20.0, 30.0));
    // Somewhere else, between the two blocks in paint order.
    list.chrome_rect(
        Rect::new(170.0, 100.0, 50.0, 40.0),
        6.0,
        0.0,
        [0.2, 0.7, 0.3, 1.0],
        [0.0; 4],
    );
    list.text(shadowed("Second", 30.0, 36.0));
}

fn merged_shadow_and_glow(list: &mut DrawList, _: &Art) {
    // Back to back, so one command: B's shadow lands on A's fill.
    list.text(TextBlock::new("Alpha", 20.0, 30.0).with_size(22.0));
    list.text(
        TextBlock::new("Beta", 30.0, 38.0)
            .with_size(22.0)
            .with_shadow(200, 0, 0, 255, 2.0, 2.0, 1.0),
    );
    list.text(
        TextBlock::new("Gamma", 40.0, 70.0)
            .with_size(22.0)
            .with_glow(0, 200, 255, 255, 4.0),
    );
    list.chrome_rect(
        Rect::new(10.0, 60.0, 80.0, 20.0),
        2.0,
        0.0,
        [0.3, 0.3, 0.3, 0.6],
        [0.0; 4],
    );
}

fn clipped_text_in_a_scroll_view(list: &mut DrawList, _: &Art) {
    let theme = Theme::default();
    let style = StyleResolver::new(&theme);
    let mut input = InputState::default();
    let mut state = ScrollState::default();
    state.content_size = [200.0, 40.0 * 22.0];
    state.offset = [0.0, 31.0];
    let view = ScrollView::new(Rect::new(10.0, 10.0, 200.0, 130.0)).vertical_only();
    view.draw(&mut state, list, &style, &mut input, |list, inner| {
        for row in 0..40 {
            let y = inner.y + 22.0 * row as f32;
            list.chrome_rect(
                Rect::new(inner.x, y, 180.0, 20.0),
                3.0,
                0.0,
                [0.2, 0.2, 0.25, 1.0],
                [0.0; 4],
            );
            list.text(TextBlock::new(format!("Row {row}"), inner.x + 6.0, y + 3.0).with_size(13.0));
        }
    });
}

fn shadows_under_neighbours(list: &mut DrawList, _: &Art) {
    for i in 0..5 {
        let rect = Rect::new(12.0 + 40.0 * i as f32, 40.0, 36.0, 50.0);
        list.box_shadow_outset(
            rect,
            CornerRadii::default(),
            BoxShadow {
                offset: [4.0, 6.0],
                blur: 10.0,
                spread: 2.0,
                color: [0.0, 0.0, 0.0, 0.6],
                inset: false,
            },
        );
        list.chrome_rect(rect, 5.0, 1.0, [0.8, 0.8, 0.85, 1.0], [0.2, 0.2, 0.2, 1.0]);
    }
}

fn rotated_chrome(list: &mut DrawList, _: &Art) {
    list.chrome_rect(
        Rect::new(10.0, 10.0, 60.0, 40.0),
        4.0,
        0.0,
        [0.2, 0.5, 0.8, 1.0],
        [0.0; 4],
    );
    list.push_transform();
    list.translate(80.0, 60.0);
    list.rotate(0.6);
    list.chrome_rect(
        Rect::new(-30.0, -20.0, 60.0, 40.0),
        6.0,
        2.0,
        [0.9, 0.3, 0.3, 0.8],
        [1.0, 1.0, 1.0, 1.0],
    );
    list.pop_transform();
    list.text(TextBlock::new("next to it", 120.0, 60.0).with_size(14.0));
    list.chrome_rect(
        Rect::new(100.0, 20.0, 30.0, 30.0),
        2.0,
        0.0,
        [0.3, 0.8, 0.3, 1.0],
        [0.0; 4],
    );
}

fn sprites_in_two_textures(list: &mut DrawList, art: &Art) {
    for i in 0..6 {
        let x = 10.0 + 30.0 * i as f32;
        let sprite = if i % 2 == 0 { art.red } else { art.blue };
        list.icon_sprite(sprite, x, 20.0, 40.0, 40.0, [1.0; 4]);
        list.icon("green", x + 10.0, 50.0, 20.0, 20.0);
    }
    list.image(art.video, Rect::new(40.0, 80.0, 80.0, 60.0), [1.0; 4]);
    list.icon_sprite(art.red, 100.0, 100.0, 40.0, 40.0, [1.0, 1.0, 1.0, 0.7]);
    list.icon("missing", 150.0, 100.0, 20.0, 20.0);
}

fn strokes_crossing(list: &mut DrawList, _: &Art) {
    list.chrome_rect(
        Rect::new(20.0, 20.0, 100.0, 60.0),
        8.0,
        0.0,
        [0.2, 0.3, 0.5, 1.0],
        [0.0; 4],
    );
    list.text(TextBlock::new("crossed", 30.0, 40.0).with_size(20.0));
    list.line([10.0, 10.0], [200.0, 120.0], 3.0, [1.0, 0.8, 0.1, 0.9]);
    list.stroke_polyline(
        &[[15.0, 100.0], [60.0, 30.0], [110.0, 110.0], [180.0, 20.0]],
        &Stroke::new(4.0),
        [0.1, 1.0, 0.6, 0.8],
    );
}

fn circles_nine_slices_and_soup(list: &mut DrawList, _: &Art) {
    list.radial_gradient(
        Rect::new(10.0, 10.0, 90.0, 90.0),
        [1.0, 1.0, 1.0, 1.0],
        [0.1, 0.1, 0.4, 0.0],
        32,
    );
    list.circle((60.0, 60.0), 20.0, [0.9, 0.2, 0.5, 0.8]);
    list.nine_slice(50.0, 50.0, 80.0, 50.0, "panel");
    list.circle((130.0, 40.0), 15.0, [0.2, 0.9, 0.5, 1.0]);
    list.chrome_rect(
        Rect::new(120.0, 80.0, 60.0, 40.0),
        4.0,
        0.0,
        [0.5, 0.5, 0.9, 1.0],
        [0.0; 4],
    );
    list.radial_gradient(
        Rect::new(140.0, 90.0, 60.0, 60.0),
        [1.0, 0.9, 0.2, 0.9],
        [1.0, 0.2, 0.2, 0.0],
        24,
    );
}

#[cfg(feature = "phosphor-icons")]
fn icons_over_text(list: &mut DrawList, _: &Art) {
    use wgpu_gameui::PhosphorIcon;
    list.text(TextBlock::new("icons over this", 10.0, 30.0).with_size(18.0));
    list.phosphor_icon(
        Rect::new(20.0, 24.0, 24.0, 24.0),
        PhosphorIcon::Folder,
        [1.0, 0.6, 0.1, 1.0],
    );
    list.text(TextBlock::new("and under this", 30.0, 34.0).with_size(18.0));
    list.phosphor_icon(
        Rect::new(150.0, 80.0, 32.0, 32.0),
        PhosphorIcon::Plus,
        [0.3, 0.9, 1.0, 1.0],
    );
}

fn wholly_culled(list: &mut DrawList, _: &Art) {
    list.chrome_rect(
        Rect::new(-500.0, -500.0, 40.0, 40.0),
        4.0,
        0.0,
        [1.0; 4],
        [0.0; 4],
    );
    list.text(TextBlock::new("far away", 2000.0, 2000.0).with_size(14.0));
    list.push_clip(Rect::new(10.0, 10.0, 50.0, 50.0));
    list.chrome_rect(
        Rect::new(100.0, 100.0, 40.0, 40.0),
        4.0,
        0.0,
        [1.0; 4],
        [0.0; 4],
    );
    list.pop_clip();
}

const SCENES: &[(&str, Scene)] = &[
    ("button grid", button_grid),
    ("chrome under text", chrome_under_text),
    ("shadowed text around a box", shadowed_text_around_a_box),
    ("merged shadow and glow", merged_shadow_and_glow),
    (
        "clipped text in a scroll view",
        clipped_text_in_a_scroll_view,
    ),
    ("shadows under neighbours", shadows_under_neighbours),
    ("rotated chrome", rotated_chrome),
    ("sprites in two textures", sprites_in_two_textures),
    ("strokes crossing", strokes_crossing),
    (
        "circles, nine-slices and soup",
        circles_nine_slices_and_soup,
    ),
    #[cfg(feature = "phosphor-icons")]
    ("icons over text", icons_over_text),
    ("wholly culled", wholly_culled),
];

/// `scene` with its content moved by `shift`.
fn build(gpu: &HeadlessGpu, scene: Scene, art: &Art, shift: [f32; 2]) -> DrawList {
    let mut list = gpu.draw_list();
    list.push_transform();
    list.translate(shift[0], shift[1]);
    scene(&mut list, art);
    list.pop_transform();
    list
}

/// Where the two images differ: how many pixels, and the first.
fn difference(a: &[u8], b: &[u8]) -> Option<(usize, u32, u32)> {
    let mut first = None;
    let mut count = 0;
    for (i, (pa, pb)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
        if pa != pb {
            count += 1;
            first.get_or_insert(i as u32);
        }
    }
    first.map(|i| (count, i % SIZE.0, i / SIZE.0))
}

#[test]
#[ignore = "requires a GPU adapter"]
fn batches_draw_the_same_pixels_as_paint_order() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let art = load_art(&mut gpu);
    gpu.renderer().set_check_plan(true);
    let mut failures = Vec::new();
    for &(name, scene) in SCENES {
        for scale in SCALES {
            for (origin, shift) in ORIGINS {
                let list = build(&gpu, scene, &art, shift);
                gpu.renderer().set_view_origin(origin[0], origin[1]);
                gpu.renderer().set_paint_order(true);
                let painted = gpu.capture_scaled(&list, SIZE, scale);
                let paint_stats = gpu.renderer().frame_stats();
                gpu.renderer().set_paint_order(false);
                let batched = gpu.capture_scaled(&list, SIZE, scale);
                let batch_stats = gpu.renderer().frame_stats();
                if name != "wholly culled" && shift == [0.0, 0.0] {
                    assert!(
                        painted.chunks(4).any(|p| p[3] > 0),
                        "{name} at {scale}: nothing drew"
                    );
                }
                assert!(
                    batch_stats.draw_calls <= paint_stats.draw_calls,
                    "{name} at {scale}, origin {origin:?}: {} batched draws against {} in paint order",
                    batch_stats.draw_calls,
                    paint_stats.draw_calls
                );
                if let Some((count, x, y)) = difference(&painted, &batched) {
                    failures.push(format!(
                        "{name} at scale {scale}, origin {origin:?}: {count} pixels differ, the first at ({x}, {y})"
                    ));
                }
            }
        }
    }
    gpu.renderer().set_view_origin(0.0, 0.0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_wholly_culled_list_draws_nothing_batched() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let art = load_art(&mut gpu);
    let list = build(&gpu, wholly_culled, &art, [0.0, 0.0]);
    let pixels = gpu.capture(&list, SIZE);
    let stats = gpu.renderer().frame_stats();
    assert!(pixels.chunks(4).all(|p| p[3] == 0));
    assert_eq!(stats.draw_calls, 0, "{stats:?}");
    assert_eq!(stats.units_ordered, 0, "{stats:?}");
    assert_eq!(stats.render_passes, 0, "{stats:?}");
}
