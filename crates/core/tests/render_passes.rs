//! Ignored headless test: a draw list's whole paint stream is drawn in one
//! render pass, however finely its primitive kinds alternate.
//!
//! A pass per run used to cost a full load/store of the target per text run,
//! and on Metal each pass holds a command buffer until the submission
//! completes: a frame of ~1,100 alternating runs exhausted the queue's 2048
//! and hung forever inside `Queue::submit`.
//!
//! Drawn in paint order and in batches (`docs/design/draw-call-batching.md`).
//! Batched, the off-target labels are culled, so the colour runs merge; only
//! the paint-order path still alternates.

use wgpu_gameui::{DrawList, HeadlessGpu, TextBlock, Vertex};

/// Rows of a quad followed by a label, so every row adds a color run and a
/// text run.
const ROWS: usize = 1_500;

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn alternating_runs_share_one_pass_and_keep_their_order() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    for paint_order in [true, false] {
        gpu.renderer().set_paint_order(paint_order);
        check_alternating_runs(&mut gpu, paint_order);
    }
}

fn check_alternating_runs(gpu: &mut HeadlessGpu, paint_order: bool) {
    let size = (64, 64);
    let mut list = gpu.draw_list();
    for i in 0..ROWS {
        // Each row paints over the previous ones, so the last row's colour
        // must win: order is preserved across the shared pass.
        let shade = (i % 2) as f32;
        list.quad(0.0, 0.0, 64.0, 64.0, [shade, 0.0, 1.0 - shade, 1.0]);
        list.text(TextBlock::new("x", 200.0, 0.0).with_size(10.0));
    }
    list.quad(8.0, 8.0, 16.0, 16.0, [0.0, 1.0, 0.0, 1.0]);

    let pixels = gpu.capture(&list, size);
    let stats = gpu.renderer().frame_stats();
    assert!(
        stats.paint_runs >= ROWS * 2,
        "the stream alternates: {stats:?}"
    );
    if paint_order {
        assert!(stats.draw_calls >= ROWS * 2, "drawn as recorded: {stats:?}");
    }
    assert_eq!(stats.render_passes, 1, "{stats:?}");

    let at = |x: u32, y: u32| {
        let i = ((y * size.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    // ROWS is even, so the last full-target quad is the odd (red) one.
    assert_eq!(at(40, 40), [255, 0, 0, 255], "paint order {paint_order}");
    assert_eq!(at(12, 12), [0, 255, 0, 255], "paint order {paint_order}");
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn soup_outside_the_paint_stream_draws_once() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    for paint_order in [true, false] {
        gpu.renderer().set_paint_order(paint_order);
        // Filled through the public arrays alone: no paint stream at all.
        let mut list = gpu.draw_list();
        push_translucent_cover(&mut list);
        check_drawn_once(&mut gpu, &list, 1, paint_order);
        // After a paint stream, as trailing soup it paints over.
        let mut list = gpu.draw_list();
        list.quad(0.0, 0.0, 8.0, 8.0, [0.0, 0.0, 1.0, 1.0]);
        push_translucent_cover(&mut list);
        check_drawn_once(&mut gpu, &list, 2, paint_order);
    }
}

/// Half-transparent white over the whole 64 px target, as raw soup.
fn push_translucent_cover(list: &mut DrawList) {
    let base = list.vertices.len() as u32;
    let color = [1.0, 1.0, 1.0, 0.5];
    for (x, y) in [(0.0, 0.0), (64.0, 0.0), (64.0, 64.0), (0.0, 64.0)] {
        list.vertices.push(Vertex::new(x, y, color));
    }
    list.indices
        .extend([0, 1, 2, 0, 2, 3].iter().map(|i| base + i));
}

fn check_drawn_once(gpu: &mut HeadlessGpu, list: &DrawList, draw_calls: usize, paint_order: bool) {
    let clear = gpu.clear_color([0.0, 0.0, 0.0, 1.0]);
    let pixels = gpu.capture_on(list, (64, 64), clear);
    let stats = gpu.renderer().frame_stats();
    assert_eq!(
        stats.draw_calls, draw_calls,
        "paint order {paint_order}: {stats:?}"
    );
    // Half white over black is mid grey; drawn twice it would be three
    // quarters.
    let i = ((40 * 64 + 40) * 4) as usize;
    let grey = pixels[i];
    assert!(
        (120..=136).contains(&grey),
        "paint order {paint_order}: {:?}",
        &pixels[i..i + 4]
    );
}
