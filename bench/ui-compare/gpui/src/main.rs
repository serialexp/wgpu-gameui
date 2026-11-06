//! gpui: the CPU frame cost of the shared scene, GPU hand-off included.
//!
//! gpui's own benchmark machinery, run headless: `BenchAppContext` with a real
//! text system and `WgpuHeadlessRenderer`, so a frame is what gpui does in a
//! window, minus the compositor. Per timed frame: the view's `render` (a column
//! of rows of fixed-size `div`s with a text child), taffy layout, prepaint,
//! paint into a `Scene`, then the scene is handed to the wgpu renderer
//! (atlas upload, instance upload, render pass, submit).
//!
//! Timing is the gap between successive calls of the per-iteration update
//! callback, which is one whole iteration of gpui's loop (pump tasks, update,
//! draw, present). Criterion only drives the iterations; its statistics are
//! not used. gpui's own report is printed to stderr under each case.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use criterion::Criterion;
use gpui::{
    BenchAppContext, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, px, rgb, size,
};
use ui_compare_common::*;

/// gpui's test window has a fixed scale factor of 2 and no way to change it
/// from outside, so the scene is built at half its logical size: the frame is
/// then the same 1920x1080 device pixels, with the same cells and 12 px (device)
/// text, as every other library draws at scale 1.
const LOGICAL: f32 = 0.5;

struct Cells {
    grid: Grid,
    labels: Vec<SharedString>,
    frame: u64,
    animated: bool,
}

impl Render for Cells {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = self.grid.size();
        let grid = self.grid;
        let rows = grid.n.div_ceil(grid.cols);
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP * LOGICAL))
            .bg(rgb(0x000000))
            .children((0..rows).map(|r| {
                div().flex().flex_row().gap(px(GAP * LOGICAL)).children(
                    (r * grid.cols..grid.n.min((r + 1) * grid.cols)).map(|i| {
                        let [red, green, blue] = fill(i, self.frame, self.animated);
                        let color = (red as u32) << 16 | (green as u32) << 8 | blue as u32;
                        div()
                            .w(px(w * LOGICAL))
                            .h(px(h * LOGICAL))
                            .flex_none()
                            .rounded(px(grid.radius * LOGICAL))
                            .font_family(FONT_FAMILY)
                            .bg(rgb(color))
                            .text_size(px(grid.font_px * LOGICAL))
                            .text_color(rgb(0xffffff))
                            .child(self.labels[i].clone())
                    }),
                )
            }))
    }
}

/// Frame gaps recorded for one case, in microseconds.
type Gaps = Rc<RefCell<Vec<f64>>>;

fn run_case(
    criterion: &mut Criterion<gpui::BenchMeasurement>,
    n: usize,
    animated: bool,
    gaps: &Gaps,
) {
    let name = format!("{n}-{}", if animated { "animated" } else { "static" });
    let gaps = gaps.clone();
    criterion.bench_function(&name, |bencher| {
        let mut cx = BenchAppContext::new(
            gpui::bench_platform(
                Some(Box::new(gpui_platform::current_headless_renderer)),
                gpui_platform::bench_text_system(),
            ),
            Some("ui_compare"),
            bencher,
        );
        let mut window = cx.add_empty_window();
        window.simulate_resize(size(px(WIDTH * LOGICAL), px(HEIGHT * LOGICAL)));
        let grid = Grid::new(n);
        let labels: Vec<SharedString> = (0..n).map(|i| label(i).into()).collect();
        let root = window.update(|window, cx| {
            window.replace_root(cx, |_, _| Cells {
                grid,
                labels,
                frame: 0,
                animated,
            })
        });
        window.update(|window, _| window.refresh());

        // One iteration is one gap between calls of this callback; the first
        // call of a run has nothing before it.
        let mut previous: Option<Instant> = None;
        let gaps = gaps.clone();
        cx.bench_renderer(root, move |view, _, cx| {
            let now = Instant::now();
            if let Some(before) = previous {
                gaps.borrow_mut()
                    .push(now.duration_since(before).as_secs_f64() * 1e6);
            }
            previous = Some(now);
            if animated {
                view.frame += 1;
            }
            cx.notify();
        });

        // The frame must contain the scene: a quad per cell and a glyph or
        // two per label in the scene, and the rendered image mostly lit.
        window.update(|window, _| {
            let scene = window.rendered_scene();
            assert!(
                scene.quads.len() >= n,
                "gpui painted {} quads for {n} cells",
                scene.quads.len()
            );
            // Text is drawn as monochrome or, on Linux with subpixel text on,
            // subpixel glyph sprites.
            let glyphs = scene.monochrome_sprites.len() + scene.subpixel_sprites.len();
            assert!(glyphs >= n, "gpui painted {glyphs} glyphs for {n} labels");
            if animated {
                let image = window.render_to_image().expect("render the frame");
                assert_eq!(
                    (image.width(), image.height()),
                    (WIDTH as u32, HEIGHT as u32)
                );
                check_frame("gpui", n, image.as_raw());
            }
        });
        cx.teardown();
    });
}

fn main() {
    env_logger::init();
    // The shared typeface, into the text system every case uses.
    gpui_platform::bench_text_system()
        .add_fonts(vec![std::borrow::Cow::Borrowed(FONT_BYTES)])
        .expect("load the font");
    // (Without a windowing feature on gpui_platform the text system is a no-op
    // and nothing is shaped.)
    let names = gpui_platform::bench_text_system().all_font_names();
    assert!(
        names.iter().any(|n| n == FONT_FAMILY),
        "the shared font did not load"
    );
    let mut criterion = Criterion::default()
        .with_measurement(gpui::BenchMeasurement::from_env_or_exit())
        .sample_size(10)
        .warm_up_time(Duration::from_millis(1))
        .measurement_time(Duration::from_secs(2))
        .without_plots();
    for n in counts() {
        for (scenario, animated) in [("static", false), ("animated", true)] {
            let gaps: Gaps = Rc::default();
            run_case(&mut criterion, n, animated, &gaps);
            report("gpui", scenario, n, &stats_of(&gaps.borrow()));
        }
    }
}

/// Mean (without the slowest 2%, which hold the gaps between criterion's
/// samples), median and 99th percentile of the recorded gaps.
fn stats_of(gaps: &[f64]) -> Stats {
    let mut sorted = gaps.to_vec();
    assert!(!sorted.is_empty(), "no frames were recorded");
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let kept = &sorted[..sorted.len() - sorted.len() / 50];
    Stats {
        frames: sorted.len(),
        mean_us: kept.iter().sum::<f64>() / kept.len() as f64,
        p50_us: at(0.5),
        p99_us: at(0.99),
        max_us: *sorted.last().unwrap(),
    }
}
