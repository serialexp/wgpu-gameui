//! Rune: the CPU cost of one frame of the shared scene, headless.
//!
//! Rune's own benchmarks (`crates/rune/examples/pinch_bench.rs`) run a
//! `Runtime` against an offscreen wgpu target with no window; this does the
//! same. Per timed frame: the state changes (animated) or is only marked
//! changed (static, so the view is rebuilt either way, as in every other
//! harness), `Runtime::frame` (view, layout, instances), then the renderer's
//! `upload_images`, `prepare` (atlas and instance upload), the pass, and
//! `submit`. The GPU is not waited on. Scale factor 1, so the 1920x1080 target
//! holds 1920x1080 points.

use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::text::TextSystem;
use rune::{
    App, AppSpec, Color, Cx, El, Env, Family, FrameOutcome, LoopSpec, Runtime, TextStyle, Tx, col,
    el, row, text,
};
use std::cell::RefCell;
use std::time::{Duration, Instant};
use ui_compare_common::*;

struct State {
    grid: Grid,
    labels: Vec<String>,
    frame: u64,
    animated: bool,
    /// Windowed runs add a 2x2 looping-opacity element, which is what keeps
    /// Rune drawing every frame on X11 (see `windowed`).
    pulse: bool,
}

fn srgb(c: [u8; 3]) -> Color {
    rune::color::hex(&format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).expect("hex")
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let grid = s.grid;
    let (w, h) = grid.size();
    let rows = grid.n.div_ceil(grid.cols);
    let font = TextStyle::new(grid.font_px, 400.0).family(Family::Named(FONT_FAMILY));
    let body = (0..rows).fold(col().gap(GAP), |body, r| {
        let cells =
            (r * grid.cols..grid.n.min((r + 1) * grid.cols)).fold(row().gap(GAP), |line, i| {
                line.child(
                    el().size(w, h)
                        .radius(grid.radius)
                        .fill(srgb(fill(i, s.frame, s.animated)))
                        .padding(1.0)
                        .child(text(s.labels[i].clone()).font(font).color(Color::WHITE)),
                )
            });
        body.child(cells)
    });
    let root = el().size(WIDTH, HEIGHT).fill(Color::BLACK).child(body);
    if s.pulse {
        root.child(
            el().size(2.0, 2.0)
                .absolute(WIDTH - 2.0, HEIGHT - 2.0)
                .fill(Color::BLACK)
                .looping(LoopSpec::opacity(1.0, 0.0, 0.0, 1.0)),
        )
    } else {
        root
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

struct Draw {
    gpu: Gpu,
    renderer: Renderer,
    out: Offscreen,
}

impl Draw {
    fn draw(&mut self, rt: &mut Runtime<State>, copy: bool) {
        self.renderer
            .upload_images(&self.gpu.device, &self.gpu.queue, &mut rt.images);
        self.renderer.prepare(
            &self.gpu.device,
            &self.gpu.queue,
            &mut rt.text.atlas,
            &rt.instances,
            (self.out.width, self.out.height),
        );
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer
            .draw(&mut encoder, &self.out.view, rt.clear_color());
        if copy {
            self.out.copy(&mut encoder);
        }
        self.renderer.submit(&self.gpu.queue, encoder);
    }
}

/// What the window's frame hook and `poll` share: Rune calls both from the
/// UI thread, and `poll` is the only place the state can change.
struct Windowed {
    cases: Vec<(usize, bool)>,
    case: usize,
    seen: usize,
    last: Option<(Instant, f64)>,
    wall: Vec<f64>,
    cpu: Vec<f64>,
    /// `update_ms + encode_ms` of each drawn frame, Rune's own account of the
    /// CPU wall time it spent building and encoding it.
    own: Vec<f64>,
    reset: bool,
}

thread_local! {
    static WINDOWED: RefCell<Windowed> = RefCell::new(Windowed {
        cases: counts().into_iter().flat_map(|n| [(n, false), (n, true)]).collect(),
        case: 0,
        seen: 0,
        last: None,
        wall: Vec::new(),
        cpu: Vec::new(),
        own: Vec::new(),
        reset: false,
    });
}

fn case_state(n: usize, animated: bool) -> State {
    State {
        grid: Grid::new(n),
        labels: (0..n).map(label).collect(),
        frame: 0,
        animated,
        pulse: true,
    }
}

fn windowed_init() -> State {
    let (n, animated) = WINDOWED.with(|w| w.borrow().cases[0]);
    case_state(n, animated)
}

/// Every poll asks for the next frame, and changes the state so the view is
/// rebuilt: a forced redraw each frame, as in every other harness.
fn windowed_poll(s: &mut State, _: f64, tx: &mut Tx) -> bool {
    WINDOWED.with(|w| {
        let mut w = w.borrow_mut();
        if std::mem::take(&mut w.reset) {
            let (n, animated) = w.cases[w.case];
            *s = case_state(n, animated);
        }
    });
    if s.animated {
        s.frame += 1;
    }
    tx.wake_after(Duration::ZERO);
    true
}

fn windowed_frame(f: &rune::FrameEncoded) {
    // A frame that changed nothing on screen is `Unchanged`: Rune builds it
    // but takes no drawable and encodes nothing. In the static scenario every
    // frame is one, so those count too; only a covered window's `Skipped`
    // frames are left out.
    if f.outcome() == FrameOutcome::Skipped {
        return;
    }
    WINDOWED.with(|w| {
        let mut w = w.borrow_mut();
        let (now, cpu) = (Instant::now(), thread_cpu_us());
        if let Some((prev, prev_cpu)) = w.last.replace((now, cpu)) {
            w.seen += 1;
            if w.seen > WARMUP_FRAMES {
                w.wall.push((now - prev).as_secs_f64() * 1e6);
                w.cpu.push(cpu - prev_cpu);
                if let (Some(u), Some(e)) = (f.update_ms(), f.encode_ms()) {
                    w.own.push((u + e) * 1e3);
                }
            }
        }
        if w.wall.len() >= TIMED_FRAMES {
            let (n, animated) = w.cases[w.case];
            let scenario = if animated { "animated" } else { "static" };
            let (wall, cpu, own) = (
                std::mem::take(&mut w.wall),
                std::mem::take(&mut w.cpu),
                std::mem::take(&mut w.own),
            );
            report("rune(win,wall)", scenario, n, &stats_from(wall));
            report("rune(win,cpu)", scenario, n, &stats_from(cpu));
            if !own.is_empty() {
                report("rune(win,own)", scenario, n, &stats_from(own));
            }
            w.case += 1;
            if w.case == w.cases.len() {
                std::process::exit(0);
            }
            w.seen = 0;
            w.last = None;
            w.reset = true;
        }
    });
}

/// `UI_COMPARE_WINDOW=1`: a real, visible window drawn by Rune's own window loop.
///
/// Rune presents with `AutoVsync`, so the wall interval is pinned near the
/// display's refresh; the thread CPU time between frames, and Rune's own
/// `update_ms + encode_ms` from `on_frame_encoded`, leave the waiting out.
///
/// Rune does not draw a frame in which nothing changed, and on X11 (no display
/// link) a window that has drawn goes quiet until something asks again:
/// `wake_after(Duration::ZERO)` and a thread waking the loop both left it at
/// one frame per second. A running looping animation asks for a frame every
/// refresh, as a spinner does in a real app, so the scene carries a 2x2
/// looping-opacity element in its corner (`State::pulse`). Its cost is part of
/// these numbers, and it makes the static scenario draw every frame, so the
/// static and animated rows differ only in the cells' colours.
fn windowed() {
    let mut spec = AppSpec::new("ui-compare-rune", (WIDTH, HEIGHT), windowed_init, view);
    spec.fonts = &[FONT_BYTES];
    spec.families = (FONT_FAMILY, FONT_FAMILY);
    spec.poll = Some(windowed_poll);
    App::new(spec).on_frame_encoded(windowed_frame).run();
}

fn main() {
    // An environment variable, not a flag: Rune parses the process's own
    // arguments and has a `--window KEY` of its own.
    if std::env::var_os("UI_COMPARE_WINDOW").is_some() {
        windowed();
        return;
    }
    let gpu = Gpu::headless();
    let renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let out = Offscreen::new(&gpu.device, WIDTH as u32, HEIGHT as u32);
    let mut d = Draw { gpu, renderer, out };

    for n in counts() {
        let grid = Grid::new(n);
        let labels: Vec<String> = (0..n).map(label).collect();
        for (scenario, animated) in [("static", false), ("animated", true)] {
            let state = State {
                grid,
                labels: labels.clone(),
                frame: 0,
                animated,
                pulse: false,
            };
            let text_system = TextSystem::new(&[FONT_BYTES], (FONT_FAMILY, FONT_FAMILY));
            let mut rt = Runtime::with_text(
                state,
                view,
                (WIDTH, HEIGHT),
                Env::new(true, 1.0),
                text_system,
            );

            // The frame must contain the scene: look at what was drawn.
            if animated {
                for _ in 0..4 {
                    rt.frame(1.0 / 60.0);
                }
                d.draw(&mut rt, true);
                let _ = d.gpu.device.poll(wgpu::PollType::wait_indefinitely());
                check_frame("rune", n, &d.out.read(&d.gpu.device));
            }

            // Wall time per frame (what `measure` times) and, beside it, the
            // thread's CPU time over the same frame: a gap between them is
            // time spent blocked, not working.
            let mut cpu_times = Vec::new();
            let stats = measure(|frame| {
                let cpu0 = thread_cpu_us();
                if animated {
                    rt.state.frame = frame;
                }
                rt.invalidate();
                rt.frame(1.0 / 60.0);
                d.draw(&mut rt, false);
                let _ = d.gpu.device.poll(wgpu::PollType::Poll);
                cpu_times.push(thread_cpu_us() - cpu0);
            });
            let _ = d.gpu.device.poll(wgpu::PollType::wait_indefinitely());
            report("rune", scenario, n, &stats);
            cpu_times.drain(..WARMUP_FRAMES);
            report("rune(cpu)", scenario, n, &stats_from(cpu_times));
        }
    }
}
