//! What keeping a text's cosmic-text `Buffer` costs in memory: the bytes and
//! allocations 10,000 kept buffers hold, repeated and unique labels, against
//! what the shared cache keeps for their finished layouts. Settles the
//! retained tree's shaping budget (`docs/design/prepared-text.md`, Phase 0).
//!
//! The measurements print and assert nothing about the sizes; they are
//! ignored. Two tests are not: what `PreparedText::heap_bytes` estimates is
//! checked against what the allocator handed out for it, and a changed
//! label's allocations against a new one's.
//!
//! The allocator counts per thread, so tests running side by side don't
//! count each other's.
//!
//! ```text
//! cargo test -p wgpu-gameui --release --test kept_shaping_bytes -- --ignored --nocapture
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};
use wgpu_gameui::{TextBlock, TextMeasurer, WrapMode, shared_font_system};

/// The system allocator, counting on each thread what it holds live and
/// how many blocks, and how many allocations it made.
struct Counting;

thread_local! {
    // Const-initialised, with nothing to drop: reading them allocates
    // nothing, and they can be read while the thread is torn down.
    static LIVE_BYTES: Cell<isize> = const { Cell::new(0) };
    static LIVE_BLOCKS: Cell<isize> = const { Cell::new(0) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count(bytes: isize, blocks: isize, allocations: usize) {
    let _ = LIVE_BYTES.try_with(|c| c.set(c.get() + bytes));
    let _ = LIVE_BLOCKS.try_with(|c| c.set(c.get() + blocks));
    let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + allocations));
}

// SAFETY: every call is forwarded to `System` unchanged; the counters only
// observe.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as isize, 1, 1);
        // SAFETY: the caller's contract is `System`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(-(layout.size() as isize), -1, 0);
        // SAFETY: the caller's contract is `System`'s.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size as isize - layout.size() as isize, 0, 1);
        // SAFETY: the caller's contract is `System`'s.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const LABELS: usize = 10_000;

/// This thread's live bytes, live blocks and allocations made.
fn snapshot() -> (isize, isize, usize) {
    (
        LIVE_BYTES.with(Cell::get),
        LIVE_BLOCKS.with(Cell::get),
        ALLOCATIONS.with(Cell::get),
    )
}

fn label(unique: bool, i: usize) -> String {
    if unique {
        format!("Row {i} · {:.1}%", (i * 37 % 1000) as f32 / 10.0)
    } else {
        "Row 1234 · 56.7%".to_owned()
    }
}

/// A text of `i`'s kind: labels, sentences that wrap, and paragraphs.
fn mixed(i: usize) -> String {
    match i % 3 {
        0 => label(true, i),
        1 => format!("Sentence {i} wraps once it is set in a narrow column of text."),
        _ => format!("Paragraph {i}. ").repeat(30) + "\nAnd a second line.",
    }
}

/// `PreparedText::heap_bytes`, summed over many prepared texts each measured
/// and laid out at a second width, is within a third of the bytes the
/// allocator handed out for them: what the retained tree budgets is what it
/// holds.
#[test]
fn a_prepared_texts_counted_bytes_are_what_it_holds() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let block = |text: &str, width: f32| {
        TextBlock::new(text, 0.0, 0.0)
            .with_size(16.0)
            .with_line_height(20.0)
            .with_max_width(width)
            .with_wrap(WrapMode::WordOrGlyph)
    };
    let texts: Vec<String> = (0..600).map(mixed).collect();
    let prepare = |measurer: &mut TextMeasurer| {
        texts
            .iter()
            .map(|t| {
                let mut prepared = measurer.prepare(&block(t, 300.0));
                let _ = prepared.layout(measurer, &block(t, 180.0));
                prepared
            })
            .collect::<Vec<_>>()
    };
    // Warm the font system's own caches, which are not the texts'.
    drop(prepare(&mut measurer));
    let start = snapshot();
    let prepared = prepare(&mut measurer);
    let held = snapshot();
    let counted: usize = prepared.iter().map(|p| p.heap_bytes()).sum();
    drop(prepared);
    let after = snapshot();
    let bytes = (held.0 - after.0) as f64;
    let ratio = counted as f64 / bytes;
    println!(
        "{} texts: counted {counted} B, held {bytes} B, ratio {ratio:.2} ({} B left behind)",
        texts.len(),
        after.0 - start.0
    );
    assert!(
        (0.75..=1.33).contains(&ratio),
        "counted {counted} B for {bytes} B held (ratio {ratio:.2})"
    );
}

/// A changed label, as the retained tree changes one (a prepared text given
/// new content, measured at the width it is offered and laid out at the
/// width it is drawn at), allocates at most two thirds of what preparing the
/// same label from nothing does: its buffer is set and shaped again, not made
/// anew, and measuring it makes no layout. (About 17 against 36 when this was
/// written; a new buffer for each change made it about 34.)
#[test]
fn a_changed_label_allocates_less_than_a_new_one() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let block = |text: &str, width: f32| {
        TextBlock::new(text, 0.0, 0.0)
            .with_size(13.0)
            .with_line_height(20.0)
            .with_max_width(width)
            .with_wrap(WrapMode::WordOrGlyph)
    };
    let mut prepared = measurer.prepare(&block(&label(true, 0), 420.0));
    const CHANGES: usize = 2_000;
    let texts: Vec<String> = (0..CHANGES).map(|i| label(true, i + 1)).collect();
    // Warm the font system's own caches, which are not the text's.
    for text in &texts[..100] {
        let (w, _) = prepared.measure(&mut measurer, &block(text, 420.0));
        let _ = prepared.layout(&mut measurer, &block(text, w.ceil()));
    }
    let start = snapshot();
    for text in &texts {
        let mut fresh = measurer.prepare(&block(text, 420.0));
        let (w, _) = fresh.measure(&mut measurer, &block(text, 420.0));
        let _ = fresh.layout(&mut measurer, &block(text, w.ceil()));
    }
    let new = snapshot().2 - start.2;
    let before = fonts.lock().expect("font system").layout_stats();
    let (mut measuring, mut laying_out) = (0, 0);
    for text in &texts {
        let start = snapshot();
        let (w, _) = prepared.measure(&mut measurer, &block(text, 420.0));
        let measured = snapshot();
        let _ = prepared.layout(&mut measurer, &block(text, w.ceil()));
        let laid = snapshot();
        measuring += measured.2 - start.2;
        laying_out += laid.2 - measured.2;
    }
    let stats = fonts.lock().expect("font system").layout_stats();
    let per = |n: usize| n as f64 / CHANGES as f64;
    let changed = measuring + laying_out;
    println!(
        "{CHANGES} changes: {:.1} allocations a change ({:.1} measuring, {:.1} laying out), \
         {} shaped, {} laid out; {:.1} a new label",
        per(changed),
        per(measuring),
        per(laying_out),
        stats.shaped - before.shaped,
        stats.laid_out - before.laid_out,
        per(new),
    );
    assert_eq!(
        stats.shaped - before.shaped,
        CHANGES as u64,
        "a shape a change"
    );
    assert!(
        changed * 3 <= new * 2,
        "{:.1} allocations a change against {:.1} a new label",
        per(changed),
        per(new)
    );
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
