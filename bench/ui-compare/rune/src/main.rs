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
use rune::{Color, Cx, El, Env, Family, Runtime, TextStyle, col, el, row, text};
use ui_compare_common::*;

struct State {
    grid: Grid,
    labels: Vec<String>,
    frame: u64,
    animated: bool,
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
    el().size(WIDTH, HEIGHT).fill(Color::BLACK).child(body)
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

fn main() {
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
