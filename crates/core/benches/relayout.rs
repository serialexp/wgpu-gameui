//! What laying a shaped text out again costs against shaping it: the
//! question prepared text rests on (`docs/design/prepared-text.md`, Phase 0).
//!
//! ```text
//! cargo bench -p wgpu-gameui --bench relayout
//! ```
//!
//! CPU only. cosmic-text keeps each paragraph's shaping in its `Buffer` and
//! lays it out again when the width changes; gameui's layouts drop the
//! buffer, so a second width is a second shape. Each text is set as gameui
//! sets it (the default sans, advanced shaping) at two widths:
//!
//! - `shape/<text>` — a new buffer set, shaped and laid out at a width: what
//!   a layout the cache does not have costs today.
//! - `relayout/<text>` — one kept buffer laid out at the other width, then
//!   back, nothing shaped: what a prepared text pays for a second width.
//!
//! The texts: a label (a row's, unwrapped, measured at an unlimited width
//! then drawn at its own, as the retained tree does), a sentence (unwrapped)
//! and a paragraph of about 600 characters wrapping at 300 and 320 px.
//!
//! Expectation: a relayout well under the shape, or prepared text buys
//! little; one near it goes to Bart before Phase 1.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use wgpu_gameui::shared_font_system;

struct Case {
    name: &'static str,
    text: String,
    wrap: Wrap,
    widths: [f32; 2],
}

fn cases() -> Vec<Case> {
    let paragraph = "The quick brown fox jumps over the lazy dog, and then it \
                     turns around to look at what it has done. "
        .repeat(6);
    vec![
        Case {
            name: "label",
            text: "Row 1234 · 56.7%".to_owned(),
            wrap: Wrap::None,
            widths: [1.0e6, 112.0],
        },
        Case {
            name: "sentence",
            text: "A sentence of about eighty characters, set on one line in a list row."
                .to_owned(),
            wrap: Wrap::None,
            widths: [1.0e6, 480.0],
        },
        Case {
            name: "paragraph",
            text: paragraph,
            wrap: Wrap::WordOrGlyph,
            widths: [300.0, 320.0],
        },
    ]
}

fn set(font_system: &mut FontSystem, case: &Case, width: f32) -> Buffer {
    let mut buffer = Buffer::new_empty(Metrics::new(16.0, 20.0));
    buffer.set_wrap(case.wrap);
    buffer.set_size(Some(width), None);
    let attrs = Attrs::new().family(Family::SansSerif);
    buffer.set_text(&case.text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    buffer
}

fn relayout(c: &mut Criterion) {
    let fonts = shared_font_system();
    let mut shared = fonts.lock().expect("font system");
    let font_system = shared.font_system();
    let mut group = c.benchmark_group("relayout");
    for case in cases() {
        group.bench_function(BenchmarkId::new("shape", case.name), |b| {
            b.iter(|| set(font_system, &case, case.widths[1]));
        });
        let mut buffer = set(font_system, &case, case.widths[0]);
        let mut at = 0;
        group.bench_function(BenchmarkId::new("relayout", case.name), |b| {
            b.iter(|| {
                at = 1 - at;
                buffer.set_size(Some(case.widths[at]), None);
                buffer.shape_until_scroll(font_system, false);
                buffer.layout_runs().count()
            });
        });
        assert!(
            buffer.lines.iter().all(|line| line.shape_opt().is_some()),
            "{}: laying out again kept the shaping",
            case.name
        );
    }
    group.finish();
}

criterion_group!(benches, relayout);
criterion_main!(benches);
