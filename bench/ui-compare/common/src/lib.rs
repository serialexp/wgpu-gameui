//! The scene every library draws, and the timing loop that measures it.
//!
//! The scene is a grid of `n` cells in a 1920x1080 logical window. Each cell
//! is a filled rounded rectangle with a short text label. Two scenarios:
//!
//! - `static`: nothing changes between frames. Measures what a library does
//!   when asked for a frame it could in principle reuse.
//! - `animated`: every cell's colour changes every frame. Measures the full
//!   cost of rebuilding and redrawing the whole scene.
//!
//! What is timed is **CPU frame cost**: everything the library does on the
//! calling thread to produce a frame's draw commands and hand them to the GPU
//! API, with no waiting for the GPU. Each harness says exactly what its
//! closure covers, because the libraries split the work differently.

use std::time::{Duration, Instant};

/// The one typeface every library draws labels in: the font gameui bundles.
/// Each harness loads these bytes itself, so no library gets a different
/// font, or none.
pub const FONT_BYTES: &[u8] =
    include_bytes!("../../../../crates/core/assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf");
pub const FONT_FAMILY: &str = "IBM Plex Sans";

pub const WIDTH: f32 = 1920.0;
pub const HEIGHT: f32 = 1080.0;
/// Cell counts every harness runs.
pub const COUNTS: &[usize] = &[100, 1_000, 3_000, 10_000];

/// The grid layout for `n` cells: they tile the whole window, so **every cell
/// is on screen** at every count. (A library that culls what is off screen,
/// as egui does, would otherwise draw fewer cells than one that does not, and
/// the numbers would not compare.) The cells shrink as `n` grows.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub n: usize,
    pub cols: usize,
    /// Distance between cell origins, in logical pixels.
    pub pitch_w: f32,
    pub pitch_h: f32,
    /// Label size: 12 px, or smaller when the cell is too short for it.
    pub font_px: f32,
    /// Corner radius: 4 px, or a quarter of the smaller side of a small cell.
    pub radius: f32,
}

/// Space between neighbouring cells.
pub const GAP: f32 = 1.0;

impl Grid {
    pub fn new(n: usize) -> Self {
        // Pick columns so the cells come out roughly as wide as they are tall
        // by the window's aspect ratio, and every row still fits.
        let cols = ((n as f64 * WIDTH as f64 / HEIGHT as f64).sqrt().ceil() as usize).max(1);
        let rows = n.div_ceil(cols);
        let pitch_w = WIDTH / cols as f32;
        let pitch_h = HEIGHT / rows as f32;
        Self {
            n,
            cols,
            pitch_w,
            pitch_h,
            font_px: (pitch_h * 0.55).min(12.0),
            radius: (pitch_w.min(pitch_h) * 0.25).floor().clamp(1.0, 4.0),
        }
    }

    /// Top-left corner of cell `i`, in logical pixels.
    pub fn origin(&self, i: usize) -> (f32, f32) {
        (
            (i % self.cols) as f32 * self.pitch_w,
            (i / self.cols) as f32 * self.pitch_h,
        )
    }

    /// The visible size of a cell (cell pitch minus the gap).
    pub fn size(&self) -> (f32, f32) {
        (self.pitch_w - GAP, self.pitch_h - GAP)
    }
}

/// The label of cell `i`: the index modulo 100, at most two characters, so it
/// fits the smallest cell without being clipped or overflowing differently in
/// each library.
pub fn label(i: usize) -> String {
    (i % 100).to_string()
}

/// The fill of cell `i` on `frame`, as 8-bit RGB. `animated == false` ignores
/// `frame`, so a static scene is identical from one frame to the next.
pub fn fill(i: usize, frame: u64, animated: bool) -> [u8; 3] {
    let t = if animated { frame } else { 0 };
    let h = ((i as u64).wrapping_mul(37).wrapping_add(t * 3) % 360) as f32;
    hsv(h, 0.55, 0.75)
}

fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    ]
}

/// Frames timed per case after the warm-up.
const SAMPLES: usize = 200;
/// Frames run before timing, so caches, atlases and buffers are warm.
const WARMUP: usize = 30;
/// A case stops early once it has used this much time, so 10k cells in a slow
/// library does not run for minutes.
const BUDGET: Duration = Duration::from_secs(20);

/// Per-frame timings of one case, in microseconds.
#[derive(Debug)]
pub struct Stats {
    pub frames: usize,
    pub mean_us: f64,
    pub p50_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
}

/// Run `frame(index)` for a warm-up and then timed, and report the spread.
/// `frame` must do the whole CPU frame, including handing it to the GPU API.
pub fn measure(mut frame: impl FnMut(u64)) -> Stats {
    for i in 0..WARMUP {
        frame(i as u64);
    }
    let started = Instant::now();
    let mut times = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let t = Instant::now();
        frame((WARMUP + i) as u64);
        times.push(t.elapsed().as_secs_f64() * 1e6);
        if started.elapsed() > BUDGET {
            break;
        }
    }
    stats_from(times)
}

/// Statistics of per-frame timings in microseconds, for a harness that gets
/// its frame times from somewhere other than [`measure`] (a window's own loop).
pub fn stats_from(mut times: Vec<f64>) -> Stats {
    times.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    Stats {
        frames: times.len(),
        mean_us: times.iter().sum::<f64>() / times.len() as f64,
        p50_us: at(0.5),
        p99_us: at(0.99),
        max_us: *times.last().unwrap(),
    }
}

/// Frames timed per case after the warm-up (public for harnesses that count
/// their own frames).
pub const TIMED_FRAMES: usize = SAMPLES;
/// Frames to run before timing starts.
pub const WARMUP_FRAMES: usize = WARMUP;

/// Print one result line: `library scenario n frames mean p50 p99 max`
/// (times in ms). One format for every harness so the lines can be joined.
pub fn report(library: &str, scenario: &str, n: usize, s: &Stats) {
    println!(
        "{library:<10} {scenario:<9} {n:>6} cells  {:>4} frames  mean {:>8.3} ms  p50 {:>8.3} ms  p99 {:>8.3} ms  max {:>8.3} ms",
        s.frames,
        s.mean_us / 1e3,
        s.p50_us / 1e3,
        s.p99_us / 1e3,
        s.max_us / 1e3,
    );
}

/// Counts to run: all of them, or the one given as the first argument.
pub fn counts() -> Vec<usize> {
    match std::env::args().nth(1).and_then(|a| a.parse().ok()) {
        Some(n) => vec![n],
        None => COUNTS.to_vec(),
    }
}

/// The fraction of pixels in an RGBA buffer that are not black. The scene
/// tiles the window with coloured cells, so a frame that drew it is mostly
/// non-black; a harness that drew nothing is not.
pub fn lit_fraction(rgba: &[u8]) -> f64 {
    let lit = rgba
        .chunks_exact(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    lit as f64 / (rgba.len() / 4) as f64
}

/// Save an RGBA frame under `shots/<library>-<n>.png` (next to the harness),
/// to look at, and check that the scene is really in it.
pub fn check_frame(library: &str, n: usize, rgba: &[u8]) {
    let lit = lit_fraction(rgba);
    assert!(
        lit > 0.5,
        "{library}: only {:.0}% of the frame is lit for {n} cells",
        lit * 100.0
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../shots");
    std::fs::create_dir_all(&dir).expect("create shots");
    let file = std::fs::File::create(dir.join(format!("{library}-{n}.png"))).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH as u32, HEIGHT as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(rgba)
        .expect("png data");
    eprintln!("{library} {n} cells: {:.0}% of the frame lit", lit * 100.0);
}
