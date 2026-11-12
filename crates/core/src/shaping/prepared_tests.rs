//! A prepared text lays out exactly what the shared cache shapes afresh, at
//! every layout setting and after any sequence of them, and shapes only
//! when what shaping depends on changes.

use super::*;
use crate::text::{
    FaceRange, TextAlign, TextDirection, TextMeasurer, TextSpan, WrapMode, shared_font_system,
};
use crate::{FontSystemHandle, Weight};

/// Every field of `got` is bit for bit `want`'s.
fn assert_exact(got: &ShapedLayout, want: &ShapedLayout, what: &str) {
    let bits = |v: f32| v.to_bits();
    assert_eq!(got.glyphs.len(), want.glyphs.len(), "{what}: glyphs");
    for (g, w) in got.glyphs.iter().zip(&want.glyphs) {
        let same = g.font_id == w.font_id
            && g.font_weight == w.font_weight
            && g.glyph_id == w.glyph_id
            && bits(g.rel_x) == bits(w.rel_x)
            && bits(g.cell_x) == bits(w.cell_x)
            && bits(g.rel_y) == bits(w.rel_y)
            && bits(g.font_size) == bits(w.font_size)
            && bits(g.advance) == bits(w.advance)
            && g.byte_start == w.byte_start
            && g.byte_end == w.byte_end
            && g.rtl == w.rtl;
        assert!(same, "{what}: {g:?} against {w:?}");
    }
    assert_eq!(got.lines.len(), want.lines.len(), "{what}: lines");
    for (l, w) in got.lines.iter().zip(&want.lines) {
        let same = bits(l.top) == bits(w.top)
            && bits(l.height) == bits(w.height)
            && bits(l.baseline) == bits(w.baseline)
            && l.glyphs == w.glyphs
            && l.byte_start == w.byte_start
            && bits(l.left) == bits(w.left)
            && bits(l.right) == bits(w.right)
            && bits(l.width) == bits(w.width);
        assert!(same, "{what}: {l:?} against {w:?}");
    }
    assert_eq!(got.shown, want.shown, "{what}: shown");
    assert_eq!(
        (bits(got.size.0), bits(got.size.1)),
        (bits(want.size.0), bits(want.size.1)),
        "{what}: size {:?} against {:?}",
        got.size,
        want.size
    );
}

/// The block's layout shaped through the shared cache, as drawing it
/// without a prepared layout would look it up.
fn cached(fonts: &FontSystemHandle, block: &TextBlock) -> ShapedLayout {
    let mut shared = fonts.lock().unwrap();
    let content = block.drawn_content();
    let layout = shared.layout(&LayoutSpec::of_block(block), &content);
    ShapedLayout {
        glyphs: layout.glyphs.clone(),
        lines: layout.lines.clone(),
        shown: layout.shown,
        size: layout.size,
        ink: OnceLock::new(),
    }
}

fn stats(fonts: &FontSystemHandle) -> crate::LayoutStats {
    fonts.lock().unwrap().layout_stats()
}

/// Shape the '…' `block` is cut with, which the cache shapes once and
/// keeps, so a count of what laying the block out shapes leaves it out.
fn warm_ellipsis(fonts: &FontSystemHandle, block: &TextBlock) {
    let spec = LayoutSpec::of_block(block);
    let mark = LayoutSpec {
        faces: &[],
        ..spec.natural()
    };
    fonts.lock().unwrap().layout(&mark, "…");
}

const CONTENTS: &[&str] = &[
    "Hello there",
    "abc אבג mixed text with more words",
    "שלום עולם, מה שלומך היום",
    "first line\nsecond line, long enough to wrap around a narrow width\n\nfourth",
    "a\r\n\r\nb",
    "",
    "   ",
    "Averyveryverylongwordwiderthananywidth here",
    "office ﬁ ffl ligatures",
    "日本語のテキストを折り返す",
];

/// Every layout setting a prepared text can be laid out under, in an order
/// that changes one or more of them each step.
fn layouts() -> Vec<(f32, TextAlign, WrapMode, bool, f32)> {
    let mut out = Vec::new();
    let aligns = [
        TextAlign::Start,
        TextAlign::Center,
        TextAlign::End,
        TextAlign::Left,
        TextAlign::Right,
    ];
    let wraps = [
        WrapMode::WordOrGlyph,
        WrapMode::None,
        WrapMode::Word,
        WrapMode::Glyph,
    ];
    for (i, width) in [123.25, 37.5, 1.0e6, 80.0, 0.0].into_iter().enumerate() {
        for (j, align) in aligns.into_iter().enumerate() {
            let wrap = wraps[(i + j) % wraps.len()];
            let ellipsize = (i + j) % 3 == 0;
            let line_height = if j % 2 == 0 { 20.0 } else { 23.5 };
            out.push((width, align, wrap, ellipsize, line_height));
        }
    }
    out
}

fn with_layout(
    block: &TextBlock,
    (width, align, wrap, ellipsize, line_height): (f32, TextAlign, WrapMode, bool, f32),
) -> TextBlock {
    let mut block = block
        .clone()
        .with_max_width(width)
        .with_align(align)
        .with_wrap(wrap)
        .with_line_height(line_height);
    block.ellipsize = ellipsize;
    block
}

/// Walk one prepared text through every layout and back, comparing each
/// with the cache's fresh shape, and check it shaped only when prepared.
fn walk(fonts: &FontSystemHandle, base: &TextBlock, what: &str) {
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let mut prepared = measurer.prepare(base);
    let mut steps = layouts();
    let back: Vec<_> = steps.iter().rev().copied().collect();
    steps.extend(back);
    for (step, layout) in steps.into_iter().enumerate() {
        let block = with_layout(base, layout);
        let what = format!("{what}, step {step} {layout:?}");
        warm_ellipsis(fonts, &block);
        let start = stats(fonts);
        let drawn = prepared.layout(&mut measurer, &block);
        let measured = prepared.measure(&mut measurer, &block);
        let after = stats(fonts);
        let cut = block.ellipsize
            && !block.vertical
            && drawn.layout().shown < block.drawn_content().len() as u32;
        // A cut line is shaped afresh, and once kept is not shaped again;
        // nothing else shapes.
        assert!(
            after.shaped - start.shaped <= u64::from(cut),
            "{what}: shaped {}",
            after.shaped - start.shaped
        );
        assert_exact(drawn.layout(), &cached(fonts, &block), &what);
        let want = TextMeasurer::with_font_system(fonts.clone()).measure_block(&block);
        assert_eq!(
            (measured.0.to_bits(), measured.1.to_bits()),
            (want.0.to_bits(), want.1.to_bits()),
            "{what}: measured {measured:?} against {want:?}"
        );
    }
}

#[test]
fn every_layout_of_a_prepared_text_is_the_one_a_fresh_shape_gives() {
    let fonts = shared_font_system();
    for content in CONTENTS {
        for direction in [TextDirection::Auto, TextDirection::Ltr, TextDirection::Rtl] {
            let base = TextBlock::new(*content, 0.0, 0.0)
                .with_size(16.0)
                .with_direction(direction);
            walk(&fonts, &base, &format!("{content:?} {direction:?}"));
        }
    }
}

#[test]
fn letter_spacing_faces_spans_and_vertical_text_lay_out_as_a_fresh_shape() {
    let fonts = shared_font_system();
    let text = "Bold here, plain there";
    let faced = TextBlock::new(text, 0.0, 0.0).with_face_ranges(vec![FaceRange {
        range: 0..9,
        font: None,
        weight: Weight::BOLD,
        style: crate::Style::Normal,
    }]);
    walk(&fonts, &faced, "faces");
    for spacing in [-1.5, 2.0] {
        let spaced = TextBlock::new(text, 0.0, 0.0).with_letter_spacing(spacing);
        walk(&fonts, &spaced, &format!("letter spacing {spacing}"));
    }
    let spans = TextBlock::new("", 0.0, 0.0).with_spans(vec![
        TextSpan {
            text: "two ".into(),
            color: Some([1.0, 0.0, 0.0, 1.0]),
            ..TextSpan::default()
        },
        TextSpan {
            text: "spans".into(),
            ..TextSpan::default()
        },
    ]);
    walk(&fonts, &spans, "spans");
    let vertical = TextBlock::new("縦書き", 0.0, 0.0).with_vertical();
    walk(&fonts, &vertical, "vertical");
}

#[test]
fn a_text_given_one_content_after_another_lays_out_as_a_fresh_shape_of_each() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let mut prepared = measurer.prepare(&TextBlock::new("Before", 0.0, 0.0));
    let steps = layouts();
    // Twice through, so each content follows another kind of content: more
    // lines and fewer, empty and not, one direction and the other.
    let contents = CONTENTS.iter().chain(CONTENTS.iter().rev());
    for (step, content) in contents.enumerate() {
        let layout = steps[step * 7 % steps.len()];
        let block = with_layout(&TextBlock::new(*content, 0.0, 0.0).with_size(16.0), layout);
        let what = format!("step {step} {content:?} {layout:?}");
        warm_ellipsis(&fonts, &block);
        let start = stats(&fonts);
        let measured = prepared.measure(&mut measurer, &block);
        let drawn = prepared.layout(&mut measurer, &block);
        let cut = block.ellipsize && drawn.layout().shown < block.drawn_content().len() as u32;
        assert!(
            stats(&fonts).shaped - start.shaped <= 1 + u64::from(cut),
            "{what}: shaped {}",
            stats(&fonts).shaped - start.shaped
        );
        assert_exact(drawn.layout(), &cached(&fonts, &block), &what);
        let want = TextMeasurer::with_font_system(fonts.clone()).measure_block(&block);
        assert_eq!(
            (measured.0.to_bits(), measured.1.to_bits()),
            (want.0.to_bits(), want.1.to_bits()),
            "{what}: measured {measured:?} against {want:?}"
        );
    }
}

#[test]
fn a_label_given_new_content_is_shaped_once_and_measured_without_a_layout() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    // Centred, so where its glyphs stand depends on the width.
    let offered = |text: &str| {
        TextBlock::new(text, 0.0, 0.0)
            .with_max_width(420.0)
            .with_wrap(WrapMode::WordOrGlyph)
            .with_align(TextAlign::Center)
    };
    let mut prepared = measurer.prepare(&offered("Row 0 · 0.0%"));
    for i in 1..20 {
        let text = format!("Row {i} · {}.{}%", i * 3, i % 10);
        let start = stats(&fonts);
        let measured = prepared.measure(&mut measurer, &offered(&text));
        let w = measured.0;
        // Measured, the size is read off the buffer: no layout is made.
        assert_eq!(prepared.kept.iter().flatten().count(), 0, "{text}");
        let own = offered(&text).with_max_width(w.ceil());
        let drawn = prepared.layout(&mut measurer, &own);
        let after = stats(&fonts);
        assert_eq!(after.shaped - start.shaped, 1, "{text}: shaped once");
        assert_eq!(
            after.laid_out - start.laid_out,
            1,
            "{text}: laid out once more"
        );
        assert_eq!(prepared.kept.iter().flatten().count(), 1, "{text}");
        // Measured again, the size is kept.
        assert_eq!(prepared.measure(&mut measurer, &offered(&text)), measured);
        assert_eq!(stats(&fonts).laid_out, after.laid_out, "{text}: size kept");
        let want = TextMeasurer::with_font_system(fonts.clone()).measure_block(&offered(&text));
        assert_eq!(measured, want, "{text}: measured");
        assert_exact(drawn.layout(), &cached(&fonts, &own), &text);
        // Drawn at the width it was measured at, after the other: the buffer
        // is laid out under that width again, not read as it was left.
        let back = prepared.layout(&mut measurer, &offered(&text));
        assert_exact(back.layout(), &cached(&fonts, &offered(&text)), &text);
    }
}

#[test]
fn a_hugging_label_is_shaped_once_and_laid_out_once_more() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let offered = TextBlock::new("Row 12 · 3.4%", 0.0, 0.0)
        .with_max_width(1.0e6)
        .with_wrap(WrapMode::None);
    let start = stats(&fonts);
    let mut prepared = measurer.prepare(&offered);
    let (w, _) = prepared.measure(&mut measurer, &offered);
    let own = offered.clone().with_max_width(w.ceil());
    let drawn = prepared.layout(&mut measurer, &own);
    let after = stats(&fonts);
    assert_eq!(after.shaped - start.shaped, 1, "shaped once");
    assert_eq!(
        after.laid_out - start.laid_out,
        1,
        "laid out at its own width"
    );
    // Asking again for either is kept.
    prepared.measure(&mut measurer, &offered);
    let again = prepared.layout(&mut measurer, &own);
    assert!(Arc::ptr_eq(&drawn.0, &again.0));
    assert_eq!(stats(&fonts).laid_out, after.laid_out, "both kept");
    assert_eq!(stats(&fonts).shaped, after.shaped);
}

#[test]
fn what_shaping_depends_on_prepares_again_and_the_rest_only_lays_out() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let base = TextBlock::new("Some text", 0.0, 0.0);
    let mut prepared = measurer.prepare(&base);
    let shapes = |prepared: &mut PreparedText, measurer: &mut TextMeasurer, block: &TextBlock| {
        let start = stats(&fonts).shaped;
        prepared.layout(measurer, block);
        stats(&fonts).shaped - start
    };
    for (block, what) in [
        (base.clone().with_max_width(40.0), "width"),
        (base.clone().with_align(TextAlign::Center), "align"),
        (base.clone().with_wrap(WrapMode::Glyph), "wrap"),
        (base.clone().with_line_height(31.0), "line height"),
    ] {
        assert_eq!(shapes(&mut prepared, &mut measurer, &block), 0, "{what}");
    }
    for (block, what) in [
        (TextBlock::new("Other text", 0.0, 0.0), "content"),
        (base.clone().with_size(17.0), "size"),
        (base.clone().with_weight(Weight::BOLD), "weight"),
        (base.clone().with_letter_spacing(1.0), "letter spacing"),
        (base.clone().with_direction(TextDirection::Rtl), "direction"),
        (base.clone().with_vertical(), "vertical"),
    ] {
        assert_eq!(shapes(&mut prepared, &mut measurer, &block), 1, "{what}");
        assert_exact(
            prepared.layout(&mut measurer, &block).layout(),
            &cached(&fonts, &block),
            what,
        );
    }
}

#[test]
fn a_font_change_or_another_font_system_prepares_again() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let block = TextBlock::new("Some text", 0.0, 0.0);
    let mut prepared = measurer.prepare(&block);
    let old = prepared.layout(&mut measurer, &block);
    measurer.clear_cache();
    let start = stats(&fonts).shaped;
    let new = prepared.layout(&mut measurer, &block);
    assert_eq!(stats(&fonts).shaped - start, 1, "a new generation");
    assert!(!Arc::ptr_eq(&old.0, &new.0));
    {
        let shared = fonts.lock().unwrap();
        let spec = LayoutSpec::of_block(&block);
        assert!(!old.is_for(&spec, &block.content, &shared), "stale");
        assert!(new.is_for(&spec, &block.content, &shared));
    }
    let other = shared_font_system();
    let mut elsewhere = TextMeasurer::with_font_system(other.clone());
    let start = stats(&other).shaped;
    let there = prepared.layout(&mut elsewhere, &block);
    assert_eq!(stats(&other).shaped - start, 1, "another font system");
    let shared = fonts.lock().unwrap();
    assert!(!there.is_for(&LayoutSpec::of_block(&block), &block.content, &shared));
}

#[test]
fn a_fitting_ellipsis_is_laid_out_and_a_cut_one_shaped_as_the_cache_cuts_it() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let base = TextBlock::new("An ellipsized label\nwith a second line", 0.0, 0.0)
        .with_ellipsis()
        .with_align(TextAlign::Center);
    let mut prepared = measurer.prepare(&base);
    let (natural, _) = prepared.measure(&mut measurer, &base);
    let start = stats(&fonts);
    let fits = base.clone().with_max_width(natural + 10.0);
    let layout = prepared.layout(&mut measurer, &fits);
    assert_eq!(stats(&fonts).shaped, start.shaped, "it fits: laid out");
    assert_exact(layout.layout(), &cached(&fonts, &fits), "fits");
    let cut = base.clone().with_max_width(natural / 2.0);
    warm_ellipsis(&fonts, &cut);
    let start = stats(&fonts);
    let layout = prepared.layout(&mut measurer, &cut);
    assert_eq!(
        stats(&fonts).shaped - start.shaped,
        1,
        "cut: the cut shaped"
    );
    assert!((layout.layout().shown as usize) < base.content.len());
    assert_exact(layout.layout(), &cached(&fonts, &cut), "cut");
}

#[test]
fn layouts_and_the_blocks_carrying_them_can_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PreparedLayout>();
    send_sync::<TextBlock>();
    send_sync::<crate::DrawList>();
    fn send<T: Send>() {}
    send::<PreparedText>();
}
/// `count` paragraphs of about 50 bytes, the `i`th saying `i`.
fn paragraphs(count: usize) -> String {
    (0..count)
        .map(|i| format!("paragraph {i}: a few words to shape, and a few more"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Give `prepared` the content `content` in `base`'s place, as a retained
/// node does when its text changes: how many paragraphs that shaped, and
/// the layouts at `base`'s width and at a second, each checked against the
/// cache's fresh shape.
fn edit(
    fonts: &FontSystemHandle,
    measurer: &mut TextMeasurer,
    prepared: &mut PreparedText,
    base: &TextBlock,
    content: &str,
    what: &str,
) -> u64 {
    let block = TextBlock {
        content: content.to_string().into(),
        ..base.clone()
    };
    let before = crate::text::paragraphs_shaped();
    prepared.measure(measurer, &block);
    let layout = prepared.layout(measurer, &block);
    let shaped = crate::text::paragraphs_shaped() - before;
    assert_exact(layout.layout(), &cached(fonts, &block), what);
    let narrow = block.clone().with_max_width(97.5);
    let layout = prepared.layout(measurer, &narrow);
    assert_exact(
        layout.layout(),
        &cached(fonts, &narrow),
        &format!("{what}, narrower"),
    );
    shaped
}

#[test]
fn an_edit_shapes_only_the_paragraphs_it_changed() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let text = paragraphs(40);
    let base = TextBlock::new(text.clone(), 0.0, 0.0).with_max_width(300.0);
    let mut prepared = measurer.prepare(&base);
    prepared.layout(&mut measurer, &base);
    // Each edit is made to the one before.
    let typed = text.replace("paragraph 20:", "paragraph 20, typed into:");
    let first = typed.replace("paragraph 0:", "the first paragraph:");
    let last = first.replace("paragraph 39:", "the last paragraph:");
    let inserted = last.replace("paragraph 10:", "a new paragraph\nparagraph 10:");
    let deleted = inserted.replace("a new paragraph\n", "");
    let joined = deleted.replacen(
        "and a few more\nparagraph 11:",
        "and a few more, paragraph 11:",
        1,
    );
    let split = joined.replace(
        "a few words to shape, and a few more, paragraph 11:",
        "a few words\nto shape, and a few more, paragraph 11:",
    );
    let ended = format!("{split}\n");
    let crlf = ended.replacen('\n', "\r\n", 3);
    // A paragraph repeated and then one of the two taken out: the runs kept
    // at the start and at the end can't overlap in the paragraph both have.
    let fifth = "paragraph 5: a few words to shape, and a few more\n";
    let repeated = text.replace(fifth, &fifth.repeat(2));
    let edits: [(&str, &str, u64); 15] = [
        ("typed into the middle", &typed, 1),
        ("the first paragraph", &first, 1),
        ("the last paragraph", &last, 1),
        ("a paragraph inserted", &inserted, 1),
        ("a paragraph deleted", &deleted, 0),
        ("two paragraphs joined", &joined, 1),
        ("one paragraph split", &split, 2),
        // The last line gains a break, and an empty line follows it.
        ("a line break at the end", &ended, 2),
        ("line breaks of another kind", &crlf, 3),
        ("the same text again", &crlf.clone(), 0),
        ("the break at the end taken off", &split, 4),
        ("emptied", "", 1),
        ("filled again", &text, 40),
        ("a paragraph repeated", &repeated, 1),
        ("the repeat taken out", &text, 0),
    ];
    for (what, content, paragraphs) in edits {
        let shaped = edit(&fonts, &mut measurer, &mut prepared, &base, content, what);
        assert_eq!(shaped, paragraphs, "{what}: paragraphs shaped");
    }
}

#[test]
fn an_edit_under_a_forced_direction_keeps_the_first_paragraph_s_mark() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let text = "first אבג\r\nsecond\r\n\r\nשלום fourth";
    for direction in [TextDirection::Rtl, TextDirection::Ltr] {
        let base = TextBlock::new(text, 0.0, 0.0)
            .with_max_width(200.0)
            .with_direction(direction);
        let mut prepared = measurer.prepare(&base);
        prepared.layout(&mut measurer, &base);
        let what = format!("{direction:?}");
        let second = text.replace("second", "second, typed");
        let shaped = edit(&fonts, &mut measurer, &mut prepared, &base, &second, &what);
        assert_eq!(shaped, 1, "{what}: the second paragraph");
        // The first line holds the mark: it is kept only if it is the same.
        let first = second.replace("first", "1st");
        let shaped = edit(&fonts, &mut measurer, &mut prepared, &base, &first, &what);
        assert_eq!(shaped, 1, "{what}: the first paragraph");
    }
}

#[test]
fn an_edit_to_a_text_with_face_ranges_or_vertical_shapes_it_whole() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let text = paragraphs(5);
    let faced = TextBlock::new(text.clone(), 0.0, 0.0)
        .with_max_width(300.0)
        .with_face_ranges(vec![FaceRange {
            range: 0..9,
            font: None,
            weight: Weight::BOLD,
            style: crate::Style::Normal,
        }]);
    let mut prepared = measurer.prepare(&faced);
    prepared.layout(&mut measurer, &faced);
    let typed = text.replace("paragraph 3:", "paragraph 3, typed:");
    let shaped = edit(
        &fonts,
        &mut measurer,
        &mut prepared,
        &faced,
        &typed,
        "faces",
    );
    assert_eq!(shaped, 5, "faces: every paragraph");

    let vertical = TextBlock::new("縦書き", 0.0, 0.0).with_vertical();
    let mut prepared = measurer.prepare(&vertical);
    prepared.layout(&mut measurer, &vertical);
    let shaped = edit(
        &fonts,
        &mut measurer,
        &mut prepared,
        &vertical,
        "縦書きだ",
        "vertical",
    );
    assert_eq!(shaped, 4, "vertical: every cluster's line");
}

/// The representative size: a 100 KB text of 2,000 paragraphs, edited in
/// the middle, shapes the one paragraph typed into.
#[test]
fn an_edit_to_a_100_kb_text_shapes_one_paragraph() {
    let fonts = shared_font_system();
    let mut measurer = TextMeasurer::with_font_system(fonts.clone());
    let text = paragraphs(2000);
    assert!(text.len() > 100_000, "{}", text.len());
    let base = TextBlock::new(text.clone(), 0.0, 0.0).with_max_width(800.0);
    let mut prepared = measurer.prepare(&base);
    prepared.layout(&mut measurer, &base);
    let typed = TextBlock {
        content: text
            .replace("paragraph 1000:", "paragraph 1000, typed:")
            .into(),
        ..base.clone()
    };
    let before = crate::text::paragraphs_shaped();
    let layout = prepared.layout(&mut measurer, &typed);
    assert_eq!(crate::text::paragraphs_shaped() - before, 1);
    assert_eq!(
        layout.layout().lines.len(),
        prepared
            .buffer
            .lines
            .iter()
            .map(|line| line.layout_opt().map_or(0, Vec::len))
            .sum::<usize>()
    );
}
