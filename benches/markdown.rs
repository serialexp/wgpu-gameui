//! What a long markdown message costs: parsing it, laying it out, and painting
//! the part of it a window shows.
//!
//! ```text
//! cargo bench --features markdown --bench markdown
//! ```
//!
//! CPU only, no GPU. The message is an agent's long answer: `PARAGRAPHS`
//! paragraphs with inline styles and links, lists and code blocks between
//! them, and a table of `ROWS` rows.
//!
//! - `parse` — source to blocks.
//! - `layout_cold` — a new layout at a width not shaped before, so every text
//!   block is shaped: what a window resize costs per message.
//! - `layout_warm` — a new layout at the width just shaped, every block found
//!   in the shaping cache: what a second document of the same text costs.
//! - `paint_viewport` — a laid-out message painted under a 1080-pixel clip
//!   in its middle: what a frame costs. Blocks outside the clip are culled,
//!   so this stays near flat as the message grows.
//! - `estimate_history` — the height estimate of `HISTORY` short messages,
//!   each parsed apart as a transcript keeps them: what a frame of dragging
//!   a window's edge costs a transcript of that many agent messages.
//! - `estimate_one` — the same count of estimates of one message, always in
//!   cache: the arithmetic alone, apart from reaching each document.
//!
//! Budget, at 2,000 paragraphs and a 300-row table: parse under 5 ms, a cold
//! layout under 250 ms (the desktop measures off the UI thread), and a frame's
//! paint under 1 ms. The history's estimate, every frame of a resize, under
//! 8 ms.

use std::cell::Cell;
use std::fmt::Write as _;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, Markdown, MarkdownStyle, StyleResolver, Theme, shared_font_system};

/// Message sizes: paragraphs, table rows.
const SIZES: &[(usize, usize)] = &[(200, 30), (2_000, 300)];

fn message(paragraphs: usize, rows: usize) -> String {
    let mut source = String::from("## Results\n\n");
    for index in 0..paragraphs {
        match index % 10 {
            3 => {
                let _ = writeln!(
                    source,
                    "- item {index} with `code`\n- and **another** one\n  - nested {index}\n"
                );
            }
            7 => {
                let _ = writeln!(
                    source,
                    "```rust\nfn f{index}(x: u32) -> u32 {{\n    x * {index}\n}}\n```\n"
                );
            }
            _ => {
                let _ = writeln!(
                    source,
                    "Paragraph {index} measures **the call** across the boundary with \
                     `perf stat`, *warm*, and links [the method](https://example.com/{index}) \
                     before it runs on for a while longer so that it wraps onto a second \
                     line at the usual transcript width.\n"
                );
            }
        }
        if index == paragraphs / 2 {
            source.push_str("| | Instructions per call | Cycles per call | Stalls |\n");
            source.push_str("|---|--:|--:|--:|\n");
            for row in 0..rows {
                let _ = writeln!(
                    source,
                    "| mahina Rust crate {row} | {} | {} ns | {}.{} |",
                    row * 7 + 300,
                    row % 170,
                    row % 30,
                    row % 10
                );
            }
            source.push('\n');
        }
    }
    source
}

/// Messages in the history `estimate_history` estimates.
const HISTORY: usize = 50_000;

/// A short agent message: a paragraph, a list, a code block, and every tenth
/// a small table.
fn short(index: usize) -> String {
    let table = "| | Instructions | Cycles |\n|---|--:|--:|\n| LuaJIT C API | 304 | 54 |\n\
                 | mahina Rust crate, linking fixed (experiment) | 1,516 | 76 ns |\n";
    format!(
        "Row {index} measured **the call** with `perf stat`, see [it](https://x.example).\n\n\
         - one\n- two\n\n```\ncode {index}\n```\n\n{}",
        if index % 10 == 1 { table } else { "Done." }
    )
}

fn style() -> MarkdownStyle {
    let theme = Theme::default();
    MarkdownStyle::forge(&StyleResolver::new(&theme))
}

fn bench(c: &mut Criterion) {
    let style = style();
    let mut list = DrawList::with_font_system(shared_font_system());

    let mut group = c.benchmark_group("markdown");
    group.sample_size(20);
    for &(paragraphs, rows) in SIZES {
        let source = message(paragraphs, rows);
        let id = format!("{paragraphs}p_{rows}r");

        group.bench_with_input(BenchmarkId::new("parse", &id), &source, |b, source| {
            b.iter(|| Markdown::parse(source));
        });

        // Each iteration a width not used before, so nothing is in the
        // shaping cache.
        let width = Cell::new(400.0f32);
        group.bench_with_input(
            BenchmarkId::new("layout_cold", &id),
            &source,
            |b, source| {
                b.iter_batched(
                    || Markdown::parse(source),
                    |doc| {
                        width.set(width.get() + 1.0);
                        doc.layout(&mut list, &style, width.get()).height()
                    },
                    criterion::BatchSize::LargeInput,
                );
            },
        );

        Markdown::parse(&source).layout(&mut list, &style, 720.0);
        group.bench_with_input(
            BenchmarkId::new("layout_warm", &id),
            &source,
            |b, source| {
                b.iter_batched(
                    || Markdown::parse(source),
                    |doc| doc.layout(&mut list, &style, 720.0).height(),
                    criterion::BatchSize::LargeInput,
                );
            },
        );

        let doc = Markdown::parse(&source);
        let layout = doc.layout(&mut list, &style, 720.0);
        let top = (layout.height() / 2.0).floor();
        let scroll = vec![0.0; layout.tables().len()];
        group.bench_function(BenchmarkId::new("paint_viewport", &id), |b| {
            b.iter(|| {
                list.clear();
                list.push_clip(Rect::new(0.0, 0.0, 1920.0, 1080.0));
                let taken = layout.paint(&mut list, 0.0, -top, &scroll, None);
                list.pop_clip();
                taken
            });
        });
    }

    let metrics = style.metrics();
    let history: Vec<Markdown> = (0..HISTORY)
        .map(|index| Markdown::parse(&short(index)))
        .collect();
    let width = Cell::new(600.0f32);
    group.bench_function(BenchmarkId::new("estimate_history", HISTORY), |b| {
        b.iter(|| {
            width.set(600.0 + (width.get() + 1.0) % 400.0);
            history
                .iter()
                .map(|doc| doc.estimate_height(&metrics, width.get()))
                .sum::<f32>()
        });
    });
    let one = Markdown::parse(&short(1));
    group.bench_function(BenchmarkId::new("estimate_one", HISTORY), |b| {
        b.iter(|| {
            width.set(600.0 + (width.get() + 1.0) % 400.0);
            (0..HISTORY)
                .map(|_| std::hint::black_box(&one).estimate_height(&metrics, width.get()))
                .sum::<f32>()
        });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
