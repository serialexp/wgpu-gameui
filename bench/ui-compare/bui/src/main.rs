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
        self.scene(window)
    }
}

impl Cells {
    fn scene(&mut self, window: &mut Window) -> impl IntoElement {
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

/// `--window`: BUI on the GPU (OpenGL) in a real X11 window.
///
/// A native window renders from its own event loop, so there is no `draw` to
/// time. The view times itself instead: the interval between two `render`
/// calls with vsync off and a new frame requested each time. That interval is
/// the whole frame as the window sees it, rebuild, layout, paint, the GL
/// submission and the buffer swap, so unlike the other rows it can include
/// waiting on the GPU or the compositor (it is pinned near the display's
/// refresh interval at small sizes). The thread's CPU time over the same
/// interval leaves the waiting out, and is the row to compare with the other
/// libraries' CPU frame cost; it includes the driver's work on this thread.
struct Windowed {
    cases: Vec<(usize, bool)>,
    case: usize,
    cells: Cells,
    last: Option<(std::time::Instant, f64)>,
    times: Vec<f64>,
    cpu_times: Vec<f64>,
    seen: usize,
    checked_gpu: bool,
}

impl Render for Windowed {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if !self.checked_gpu {
            self.checked_gpu = true;
            window.set_vsync(false);
        }
        let now = std::time::Instant::now();
        let cpu = thread_cpu_us();
        if let Some((prev, prev_cpu)) = self.last.replace((now, cpu)) {
            self.seen += 1;
            if self.seen > WARMUP_FRAMES {
                self.times.push((now - prev).as_secs_f64() * 1e6);
                self.cpu_times.push(cpu - prev_cpu);
            }
        }
        if self.times.len() >= TIMED_FRAMES {
            let (n, animated) = self.cases[self.case];
            let scenario = if animated { "animated" } else { "static" };
            // The GPU path switches on a few frames after the window opens;
            // label each row with what actually drew it.
            let (wall, cpu) = if window.is_gpu() {
                ("bui(gpu,wall)", "bui(gpu,cpu)")
            } else {
                ("bui(win,wall)", "bui(win,cpu)")
            };
            report(
                wall,
                scenario,
                n,
                &stats_from(std::mem::take(&mut self.times)),
            );
            report(
                cpu,
                scenario,
                n,
                &stats_from(std::mem::take(&mut self.cpu_times)),
            );
            self.case += 1;
            if self.case == self.cases.len() {
                std::process::exit(0);
            }
            let (n, animated) = self.cases[self.case];
            self.cells = Cells {
                grid: Grid::new(n),
                labels: (0..n).map(ui_compare_common::label).collect(),
                frame: 0,
                animated,
            };
            self.seen = 0;
            self.last = None;
        }
        window.request_animation_frame();
        self.cells.scene(window)
    }
}

/// CPU time this thread has used, in microseconds.
fn thread_cpu_us() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a plain clock query writing into `ts`.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    assert_eq!(rc, 0);
    ts.tv_sec as f64 * 1e6 + ts.tv_nsec as f64 / 1e3
}

fn windowed() {
    let cases: Vec<(usize, bool)> = counts()
        .into_iter()
        .flat_map(|n| [(n, false), (n, true)])
        .collect();
    let (n, animated) = cases[0];
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(WIDTH), px(HEIGHT)),
        })),
        is_resizable: Some(false),
        gpu: Some(true),
        ..Default::default()
    };
    Application::new().gpu(true).run(move |cx| {
        cx.open_window(opts, move |_, cx| {
            cx.new(|_| Windowed {
                cells: Cells {
                    grid: Grid::new(n),
                    labels: (0..n).map(ui_compare_common::label).collect(),
                    frame: 0,
                    animated,
                },
                cases,
                case: 0,
                last: None,
                times: Vec::new(),
                cpu_times: Vec::new(),
                seen: 0,
                checked_gpu: false,
            })
        })
        .expect("open the window");
    });
}

fn main() {
    assert!(
        blenderui::add_font(FONT_FAMILY, false, false, FONT_BYTES),
        "the shared font did not load"
    );
    if std::env::args().any(|a| a == "--window") {
        windowed();
        return;
    }
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
