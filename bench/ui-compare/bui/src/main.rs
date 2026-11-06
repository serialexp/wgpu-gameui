//! BUI (`blenderui`): the CPU frame cost of the shared scene, rasterized on
//! the CPU.
//!
//! This is BUI's headless mode, the one its own `raster_probe` example uses:
//! `Application::headless()` windows have no GPU renderer, so a frame is the
//! whole job done on the CPU. Per timed frame: `render` (a column of rows of
//! fixed-size `div`s with a text child), layout, paint into a software
//! raster, and a copy of the 1920x1080 pixels out (`App::draw` returns them,
//! about 8 MB). The other libraries do their rasterizing on the GPU, so this
//! number is not like for like with theirs; BUI's GPU path (OpenGL, needs a
//! real window) is for the GPU-timing phase.

use blenderui::prelude::*;
use ui_compare_common::*;

struct Cells {
    grid: Grid,
    labels: Vec<String>,
    frame: u64,
    animated: bool,
}

impl Render for Cells {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // A forced redraw every frame, as in every other harness.
        window.refresh();
        if self.animated {
            self.frame += 1;
        }
        let grid = self.grid;
        let (w, h) = grid.size();
        let frame = self.frame;
        let animated = self.animated;
        let rows = grid.n.div_ceil(grid.cols);
        let labels = &self.labels;
        // `body` has a default 8 px margin, which would shift the scene.
        body().styled().m_0().bg(rgb(0x000000)).child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(GAP))
                .children((0..rows).map(|r| {
                    div().flex().flex_row().gap(px(GAP)).children(
                        (r * grid.cols..grid.n.min((r + 1) * grid.cols)).map(|i| {
                            let [red, green, blue] = fill(i, frame, animated);
                            let color = (red as u32) << 16 | (green as u32) << 8 | blue as u32;
                            div()
                                .w(px(w))
                                .h(px(h))
                                .flex_none()
                                .bg(rgb(color))
                                .rounded(px(grid.radius))
                                .font_family(FONT_FAMILY)
                                .text_size(px(grid.font_px))
                                .text_color(white())
                                .child(labels[i].clone())
                        }),
                    )
                })),
        )
    }
}

fn main() {
    assert!(
        blenderui::add_font(FONT_FAMILY, false, false, FONT_BYTES),
        "the shared font did not load"
    );
    Application::headless().run(|cx| {
        for n in counts() {
            for (scenario, animated) in [("static", false), ("animated", true)] {
                let opts = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size: size(px(WIDTH), px(HEIGHT)),
                    })),
                    ..Default::default()
                };
                let grid = Grid::new(n);
                let labels: Vec<String> = (0..n).map(ui_compare_common::label).collect();
                let win = cx
                    .open_window(opts, move |_, cx| {
                        cx.new(|_| Cells {
                            grid,
                            labels,
                            frame: 0,
                            animated,
                        })
                    })
                    .expect("open the window");

                // The frame must contain the scene: look at what was drawn.
                if animated {
                    let (w, h, pixels) = cx.draw(win.id);
                    assert_eq!((w, h), (WIDTH as usize, HEIGHT as usize));
                    let rgba: Vec<u8> = pixels
                        .iter()
                        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255])
                        .collect();
                    check_frame("bui", n, &rgba);
                }

                let stats = measure(|_| {
                    std::hint::black_box(cx.draw(win.id));
                });
                report("bui(cpu)", scenario, n, &stats);
            }
        }
    });
}
