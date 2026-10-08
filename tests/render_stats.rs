//! `RenderStats` counts every byte a frame uploads, text included.

use wgpu_gameui::{HeadlessGpu, TextBlock};

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn text_vertex_uploads_are_counted() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let mut empty = gpu.draw_list();
    empty.quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]);
    gpu.capture(&empty, (64, 64));
    let without_text = gpu.renderer().frame_stats().buffer_bytes_uploaded;

    let mut list = gpu.draw_list();
    list.quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]);
    list.text(TextBlock::new("Counted", 0.0, 0.0).with_size(14.0));
    gpu.capture(&list, (64, 64));
    let stats = gpu.renderer().frame_stats();
    // Six vertices a glyph quad, seven glyphs; any vertex is several bytes.
    assert!(
        stats.buffer_bytes_uploaded >= without_text + 7 * 6 * 8,
        "the text's vertices are in the count: {stats:?} vs {without_text} without text"
    );
}
