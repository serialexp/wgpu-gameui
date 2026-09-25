//! Ignored headless test: a draw list's whole paint stream is drawn in one
//! render pass, however finely its primitive kinds alternate.
//!
//! A pass per run used to cost a full load/store of the target per text run,
//! and on Metal each pass holds a command buffer until the submission
//! completes: a frame of ~1,100 alternating runs exhausted the queue's 2048
//! and hung forever inside `Queue::submit`.

use wgpu_gameui::{HeadlessGpu, TextBlock};

/// Rows of a quad followed by a label, so every row adds a color run and a
/// text run.
const ROWS: usize = 1_500;

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn alternating_runs_share_one_pass_and_keep_their_order() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
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
    assert_eq!(stats.render_passes, 1, "{stats:?}");

    let at = |x: u32, y: u32| {
        let i = ((y * size.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    // ROWS is even, so the last full-target quad is the odd (red) one.
    assert_eq!(at(40, 40), [255, 0, 0, 255]);
    assert_eq!(at(12, 12), [0, 255, 0, 255]);
}
