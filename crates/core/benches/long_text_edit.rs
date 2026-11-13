//! What an edit to a long text costs (`docs/design/prepared-text.md`,
//! "a prepared text editing its paragraphs in place").
//!
//! ```text
//! cargo bench -p wgpu-gameui --bench long_text_edit
//! ```
//!
//! CPU only. Each iteration types into the middle paragraph of a 100 KB
//! text, a new content every time, and lays it out at 800 px:
//!
//! - `prepared/<text>` — through one [`PreparedText`], as a retained node
//!   does when its text changes: only the changed paragraphs are shaped,
//!   and the whole text is extracted again.
//! - `cache/<text>` — through the shared cache (`measure_block`), as
//!   immediate mode draws it: a long text is laid out from its last
//!   version's paragraphs.
//!
//! The texts: `paragraphs`, 2,000 paragraphs of about 50 characters (the
//! representative size), and `one-paragraph`, 100 KB without a line break.
//! cosmic-text shapes a paragraph at a time; the patched fork (root
//! `Cargo.toml`) shapes again only the words an edit touched and lays out
//! again only the lines it moved, so the prepared path does that, while the
//! cache path, laying out a fresh buffer, shapes the paragraph whole.
//!
//! Budget: `prepared/paragraphs` and `prepared/one-paragraph` each within
//! 3 ms on Bart's machine, with no load; the copy of every glyph into the
//! layout is about half of the first and 40% of the second.
//!
//! [`PreparedText`]: wgpu_gameui::PreparedText

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use wgpu_gameui::{TextBlock, TextMeasurer, shared_font_system};

fn paragraphs() -> String {
    (0..2000)
        .map(|i| format!("paragraph {i}: a few words to shape, and a few more"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn one_paragraph() -> String {
    "lorem ipsum dolor sit amet consectetur ".repeat(2600)
}

/// `text` with `n` typed into its middle, at a word's start.
fn typed(text: &str, n: u64) -> String {
    let middle = text.len() / 2;
    let at = text[..middle].rfind(' ').map_or(middle, |space| space + 1);
    format!("{}{n} {}", &text[..at], &text[at..])
}

fn long_text_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("long_text_edit");
    group.sample_size(30);
    for (name, text) in [
        ("paragraphs", paragraphs()),
        ("one-paragraph", one_paragraph()),
    ] {
        let block = |content: String| TextBlock::new(content, 0.0, 0.0).with_max_width(800.0);
        let mut measurer = TextMeasurer::with_font_system(shared_font_system());
        let mut prepared = measurer.prepare(&block(text.clone()));
        prepared.layout(&mut measurer, &block(text.clone()));
        let mut n = 0;
        group.bench_function(BenchmarkId::new("prepared", name), |b| {
            b.iter_batched(
                || {
                    n += 1;
                    block(typed(&text, n))
                },
                |edited| prepared.layout(&mut measurer, &edited),
                criterion::BatchSize::LargeInput,
            );
        });
        let mut measurer = TextMeasurer::with_font_system(shared_font_system());
        measurer.measure_block(&block(text.clone()));
        group.bench_function(BenchmarkId::new("cache", name), |b| {
            b.iter_batched(
                || {
                    n += 1;
                    block(typed(&text, n))
                },
                |edited| measurer.measure_block(&edited),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, long_text_edit);
criterion_main!(benches);
