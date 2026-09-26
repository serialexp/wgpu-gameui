//! Ignored headless test: `UiRenderer::set_view_origin` renders a window of a
//! canvas far taller than any texture, with clips still in canvas coordinates.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{HeadlessGpu, TextBlock};

/// Well past every adapter's texture limit, so this window could never be
/// reached by rendering the canvas from the top.
const TOP: f32 = 40_000.0;

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn view_origin_draws_a_window_of_a_huge_canvas() {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
    let size = (64, 64);
    let mut list = gpu.draw_list();
    // Canvas-top content that must NOT show up in the window.
    list.quad(0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]);
    // A green square at window (8, 8)..(24, 24).
    list.quad(8.0, TOP + 8.0, 16.0, 16.0, [0.0, 1.0, 0.0, 1.0]);
    // A blue quad clipped (in canvas coordinates) to window x ≥ 40.
    list.push_clip(Rect::new(40.0, TOP, 24.0, 64.0));
    list.quad(32.0, TOP + 8.0, 32.0, 16.0, [0.0, 0.0, 1.0, 1.0]);
    list.pop_clip();
    // White text in the window's lower half.
    list.text(
        TextBlock::new("WWW", 4.0, TOP + 34.0)
            .with_size(20.0)
            .with_color(255, 255, 255),
    );

    gpu.renderer().set_view_origin(0.0, TOP);
    assert_eq!(gpu.renderer().view_origin(), (0.0, TOP));
    let pixels = gpu.capture(&list, size);
    gpu.renderer().set_view_origin(0.0, 0.0);

    let at = |x: u32, y: u32| {
        let i = ((y * size.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    assert_eq!(
        at(16, 16),
        [0, 255, 0],
        "the shifted square lands at (8, 8)"
    );
    assert_eq!(at(50, 16), [0, 0, 255], "inside the clip");
    assert_eq!(at(35, 16), [0, 0, 0], "clipped away, and no canvas-top red");
    assert_eq!(
        at(2, 2),
        [0, 0, 0],
        "canvas-top content stays off the window"
    );
    let text_lit = (34..60)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .any(|(x, y)| at(x, y).iter().all(|&c| c > 200));
    assert!(text_lit, "text follows the view origin too");
}
