//! What keeping a text's cosmic-text `Buffer` costs in memory: the bytes and
//! allocations 10,000 kept buffers hold, repeated and unique labels, against
//! what the shared cache keeps for their finished layouts. Settles the
//! retained tree's shaping budget (`docs/design/prepared-text.md`, Phase 0).
//!
//! A measurement, not a test: it prints and asserts nothing about the sizes.
//!
//! ```text
//! cargo test -p wgpu-gameui --release --test kept_shaping_bytes -- --ignored --nocapture
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};
use wgpu_gameui::{TextBlock, TextMeasurer, WrapMode, shared_font_system};

/// The system allocator, counting what is live and how many blocks.
struct Counting;

static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);
static LIVE_BLOCKS: AtomicIsize = AtomicIsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded to `System` unchanged; the counters only
// observe.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        LIVE_BLOCKS.fetch_add(1, Ordering::Relaxed);
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller's contract is `System`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        LIVE_BLOCKS.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: the caller's contract is `System`'s.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE_BYTES.fetch_add(
            new_size as isize - layout.size() as isize,
            Ordering::Relaxed,
        );
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller's contract is `System`'s.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const LABELS: usize = 10_000;

fn snapshot() -> (isize, isize, usize) {
    (
        LIVE_BYTES.load(Ordering::Relaxed),
        LIVE_BLOCKS.load(Ordering::Relaxed),
        ALLOCATIONS.load(Ordering::Relaxed),
    )
}

fn label(unique: bool, i: usize) -> String {
    if unique {
        format!("Row {i} · {:.1}%", (i * 37 % 1000) as f32 / 10.0)
    } else {
        "Row 1234 · 56.7%".to_owned()
    }
}

/// Where one label's bytes go: set, shaped, laid out.
#[test]
#[ignore = "a measurement: run it with --ignored --nocapture"]
fn bytes_of_one_buffer_by_stage() {
    let fonts = shared_font_system();
    let mut shared = fonts.lock().expect("font system");
    let font_system = shared.font_system();
    let attrs = Attrs::new().family(Family::SansSerif);
    for round in 0..2 {
        let start = snapshot();
        let mut buffer = Buffer::new_empty(Metrics::new(16.0, 20.0));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(Some(112.0), None);
        buffer.set_text("Row 1234 · 56.7%", &attrs, Shaping::Advanced, None);
        let set = snapshot();
        let line = &mut buffer.lines[0];
        line.shape(font_system, 8);
        let shaped = snapshot();
        buffer.shape_until_scroll(font_system, false);
        let laid = snapshot();
        println!(
            "round {round}: set {} B in {} blocks, shaped +{} B in {}, laid out +{} B in {}",
            set.0 - start.0,
            set.1 - start.1,
            shaped.0 - set.0,
            shaped.1 - set.1,
            laid.0 - shaped.0,
            laid.1 - shaped.1,
        );
        drop(buffer);
    }
}

#[test]
#[ignore = "a measurement: run it with --ignored --nocapture"]
fn bytes_held_by_kept_buffers_and_by_the_cache() {
    let fonts = shared_font_system();
    for unique in [false, true] {
        let texts: Vec<String> = (0..LABELS).map(|i| label(unique, i)).collect();
        let kind = if unique { "unique" } else { "repeated" };

        // Kept buffers, each shaped and laid out at the width it is drawn
        // at, as a prepared text would hold one per node.
        let mut shared = fonts.lock().expect("font system");
        let font_system = shared.font_system();
        let attrs = Attrs::new().family(Family::SansSerif);
        let shape = |text: &str, font_system: &mut cosmic_text::FontSystem| {
            let mut buffer = Buffer::new_empty(Metrics::new(16.0, 20.0));
            buffer.set_wrap(Wrap::None);
            buffer.set_size(Some(112.0), None);
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(font_system, false);
            buffer
        };
        // Warm the font system's own caches, which are not the buffers'.
        drop(shape(&texts[0], font_system));
        let start = snapshot();
        let buffers: Vec<Buffer> = texts.iter().map(|t| shape(t, font_system)).collect();
        let held = snapshot();
        drop(buffers);
        let after = snapshot();
        let bytes = held.0 - after.0;
        let blocks = held.1 - after.1;
        println!(
            "{kind:>8}: {LABELS} kept buffers hold {:.2} MB in {blocks} blocks \
             ({} B, {:.1} blocks a label); {} allocations made, {} B left behind",
            bytes as f64 / 1e6,
            bytes / LABELS as isize,
            blocks as f64 / LABELS as f64,
            held.2 - start.2,
            after.0 - start.0,
        );
        drop(shared);

        // The shared cache's finished layouts of the same labels, at the
        // same width, for comparison.
        let mut measurer = TextMeasurer::with_font_system(fonts.clone());
        let before = fonts.lock().expect("font system").layout_stats();
        let start = snapshot();
        for text in &texts {
            measurer.measure_block(
                &TextBlock::new(text, 0.0, 0.0)
                    .with_size(16.0)
                    .with_line_height(20.0)
                    .with_max_width(112.0)
                    .with_wrap(WrapMode::None),
            );
        }
        let held = snapshot();
        let stats = fonts.lock().expect("font system").layout_stats();
        println!(
            "{kind:>8}: the cache keeps {} layouts, counted {:.2} MB, heap {:.2} MB",
            stats.layouts - before.layouts,
            (stats.bytes - before.bytes) as f64 / 1e6,
            (held.0 - start.0) as f64 / 1e6,
        );
        fonts.lock().expect("font system").clear_caches();
    }
}
