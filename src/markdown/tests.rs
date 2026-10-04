use std::sync::Arc;

use super::layout::column_widths;
use super::outline::{Face, Outline, Shape, count_words, lines};
use super::parse::{Block, Inline, flags, parse};
use super::{Markdown, MarkdownStyle};
use crate::layout::Rect;
use crate::widgets::DrawList;
use crate::{StyleResolver, TextAlign, TextKey, Theme, Weight};

/// The table from Bart's message: an empty first header, aligned columns,
/// and cells of numbers, dashes and units.
const TABLE: &str = "\
| | Instructions per call | Cycles per call | Store→load stalls per call |
|---|---|---|---|
| LuaJIT C API | 304 | 54 | 0 |
| Lua 5.4 C API | 449 | 75 | 3.2 |
| mahina C API | 567 | 119 | 7.3 |
| mlua over LuaJIT / Lua 5.4 | — | 24 / 26 ns | — |
| mahina Rust crate | 2,894 | 172 ns | — |
| mahina Rust crate, linking fixed (experiment) | 1,516 | 76 ns | 26.8 |
";

fn list() -> DrawList {
    DrawList::with_font_system(crate::shared_font_system())
}

fn style() -> MarkdownStyle {
    let theme = Theme::default();
    MarkdownStyle::forge(&StyleResolver::new(&theme))
}

fn paragraph(source: &str) -> Inline {
    match parse(source).into_iter().next() {
        Some(Block::Paragraph(inline)) => inline,
        other => panic!("not a paragraph: {other:?}"),
    }
}

/// The text of the bytes a run or link covers.
fn covered<'a>(inline: &'a Inline, range: &std::ops::Range<usize>) -> &'a str {
    &inline.text[range.clone()]
}

#[test]
fn a_table_parses_into_its_header_rows_and_alignment() {
    let blocks = parse(TABLE);
    let [Block::Table(table)] = blocks.as_slice() else {
        panic!("one table: {blocks:?}");
    };
    assert_eq!(table.head.len(), 4);
    assert_eq!(table.head[0].text, "");
    assert_eq!(table.head[3].text, "Store→load stalls per call");
    assert_eq!(table.rows.len(), 6);
    assert_eq!(table.rows[3][2].text, "24 / 26 ns");
    assert_eq!(table.align, vec![TextAlign::Left; 4]);

    let aligned = parse("| a | b | c |\n|:-:|--:|:--|\n| 1 | 2 | 3 |");
    let [Block::Table(table)] = aligned.as_slice() else {
        panic!("one table");
    };
    assert_eq!(
        table.align,
        vec![TextAlign::Center, TextAlign::Right, TextAlign::Left]
    );
}

#[test]
fn inline_styles_become_runs_over_their_bytes() {
    let inline = paragraph("**bold** and *it* `code` ~~gone~~ [link](https://x.example) end");
    assert_eq!(inline.text, "bold and it code gone link end");
    let runs: Vec<(&str, u8)> = inline
        .runs
        .iter()
        .map(|(range, bits)| (covered(&inline, range), *bits))
        .collect();
    assert_eq!(
        runs,
        vec![
            ("bold", flags::STRONG),
            ("it", flags::EMPHASIS),
            ("code", flags::CODE),
            ("gone", flags::STRIKE),
            ("link", flags::LINK),
        ]
    );
    let [(range, url)] = inline.links.as_slice() else {
        panic!("one link");
    };
    assert_eq!(
        (covered(&inline, range), url.as_str()),
        ("link", "https://x.example")
    );
}

#[test]
fn nested_styles_join_into_one_run_per_combination() {
    let inline = paragraph("***both*** and **bold `code`**");
    let runs: Vec<(&str, u8)> = inline
        .runs
        .iter()
        .map(|(range, bits)| (covered(&inline, range), *bits))
        .collect();
    assert_eq!(
        runs,
        vec![
            ("both", flags::STRONG | flags::EMPHASIS),
            ("bold ", flags::STRONG),
            ("code", flags::STRONG | flags::CODE),
        ]
    );
}

#[test]
fn a_soft_break_is_a_space_and_a_hard_break_a_new_line() {
    assert_eq!(paragraph("one\ntwo").text, "one two");
    assert_eq!(paragraph("one  \ntwo").text, "one\ntwo");
}

#[test]
fn lists_nest_and_carry_their_task_boxes() {
    let blocks = parse("- one\n  - inner\n- [x] done\n- [ ] todo\n\n3. three\n4. four");
    let [
        Block::List { start: None, items },
        Block::List {
            start: Some(3),
            items: numbered,
        },
    ] = blocks.as_slice()
    else {
        panic!("two lists: {blocks:?}");
    };
    assert_eq!(items.len(), 3);
    let [Block::Paragraph(one), Block::List { items: inner, .. }] = items[0].blocks.as_slice()
    else {
        panic!("a tight item's text, then its list: {:?}", items[0].blocks);
    };
    assert_eq!(one.text, "one");
    assert_eq!(inner.len(), 1);
    assert_eq!(items[1].task, Some(true));
    assert_eq!(items[2].task, Some(false));
    assert_eq!(items[0].task, None);
    assert_eq!(numbered.len(), 2);
}

#[test]
fn code_quotes_headings_and_rules_parse_as_blocks() {
    let blocks = parse(
        "# Title\n\n> quoted\n> > deeper\n\n```rust\nfn x() {}\n\n```\n\n---\n\n<div>\nraw\n</div>",
    );
    let [
        Block::Heading { level: 1, text },
        Block::Quote(quoted),
        Block::Code(code),
        Block::Rule,
        Block::Code(html),
    ] = blocks.as_slice()
    else {
        panic!("five blocks: {blocks:?}");
    };
    assert_eq!(text.text, "Title");
    assert!(matches!(
        quoted.as_slice(),
        [Block::Paragraph(_), Block::Quote(_)]
    ));
    assert_eq!(code, "fn x() {}");
    assert_eq!(html, "<div>\nraw\n</div>");
    // Inline HTML is part of its paragraph's text.
    assert_eq!(paragraph("a <b>bold</b> tag").text, "a <b>bold</b> tag");
}

#[test]
fn columns_take_their_widest_when_they_fit_and_share_the_rest_when_not() {
    assert_eq!(
        column_widths(&[10.0, 20.0], &[50.0, 30.0], 100.0),
        vec![50.0, 30.0]
    );
    // 30 spare over 50 wanted: each gets 60% of what more it wants.
    let shared = column_widths(&[10.0, 20.0], &[50.0, 30.0], 60.0);
    assert!(
        (shared[0] - 34.0).abs() < 1e-3 && (shared[1] - 26.0).abs() < 1e-3,
        "{shared:?}"
    );
    assert_eq!(
        column_widths(&[40.0, 40.0], &[90.0, 90.0], 60.0),
        vec![40.0, 40.0]
    );
}

#[test]
fn a_table_that_fits_takes_its_natural_width_and_does_not_scroll() {
    let doc = Markdown::parse(TABLE);
    let mut list = list();
    let layout = doc.layout(&mut list, &style(), 1200.0);
    assert!(layout.tables().is_empty());
    assert!(layout.height() > 0.0);
    let narrower = doc.layout(&mut list, &style(), 400.0);
    assert!(narrower.tables().is_empty(), "the words still fit");
    assert!(
        narrower.height() > layout.height(),
        "its cells wrap: {} vs {}",
        narrower.height(),
        layout.height()
    );
}

#[test]
fn a_table_too_wide_for_its_words_scrolls_sideways() {
    let doc = Markdown::parse(TABLE);
    let mut list = list();
    let layout = doc.layout(&mut list, &style(), 150.0);
    let [table] = layout.tables() else {
        panic!("one scrolling table");
    };
    assert_eq!(table.rect.width, 150.0);
    assert!(table.content_width > 150.0);
    assert!(table.max_scroll() > 0.0);
    assert!(
        layout.height() > table.rect.height,
        "room for the scrollbar under it"
    );
    assert_eq!(layout.table_at(10.0, table.rect.y + 5.0), Some(0));
}

#[test]
fn a_layout_is_kept_for_its_width_and_style() {
    let doc = Markdown::parse(TABLE);
    let mut list = list();
    let first = doc.layout(&mut list, &style(), 500.0);
    assert!(Arc::ptr_eq(&first, &doc.layout(&mut list, &style(), 500.0)));
    assert!(!Arc::ptr_eq(
        &first,
        &doc.layout(&mut list, &style(), 501.0)
    ));
    let mut bigger = style();
    bigger.size += 2.0;
    let other = doc.layout(&mut list, &bigger, 501.0);
    assert!(other.height() > first.height());
}

#[test]
fn a_paragraph_is_one_block_with_its_faces_and_colours() {
    let doc = Markdown::parse("plain **bold** `code` [link](https://x.example)");
    let mut list = list();
    let look = style();
    let layout = doc.layout(&mut list, &look, 600.0);
    let taken = layout.paint(&mut list, 0.0, 0.0, &[], Some(TextKey::new(7, 3)));
    assert_eq!(taken, 1);
    let [block] = list.texts.as_slice() else {
        panic!("one block: {}", list.texts.len());
    };
    assert_eq!(block.content, "plain bold code link");
    assert_eq!(block.selectable, Some(TextKey::new(7, 3)));
    let faces: Vec<_> = block
        .face_ranges
        .iter()
        .map(|face| {
            (
                &block.content[face.range.clone()],
                face.weight,
                face.font.clone(),
            )
        })
        .collect();
    assert_eq!(
        faces,
        vec![
            ("bold", Weight::BOLD, look.font.clone()),
            ("code", Weight::NORMAL, look.mono.clone()),
        ]
    );
    let colours: Vec<_> = block
        .style_ranges
        .iter()
        .map(|range| (&block.content[range.range.clone()], range.color))
        .collect();
    assert_eq!(
        colours,
        vec![("code", Some(look.code_ink)), ("link", Some(look.link_ink))]
    );
}

#[test]
fn a_link_is_found_under_its_text_and_nowhere_else() {
    let doc = Markdown::parse("see [the docs](https://docs.example) here");
    let mut list = list();
    let layout = doc.layout(&mut list, &style(), 600.0);
    let line = style().size * style().line_height / 2.0;
    let hits: Vec<Option<&str>> = (0..60)
        .map(|step| layout.link_at(step as f32 * 5.0, line, &[]))
        .collect();
    assert!(hits.contains(&Some("https://docs.example")));
    assert_eq!(hits[0], None, "not over `see`");
    assert_eq!(layout.link_at(5.0, 200.0, &[]), None, "not below the text");
}

#[test]
fn a_link_in_a_scrolled_table_moves_with_it() {
    let source = "| name | identifier | link |\n|---|---|---|\n| one | an_identifier_too_long_to_break_anywhere | [far](https://far.example) |";
    let doc = Markdown::parse(source);
    let mut list = list();
    let layout = doc.layout(&mut list, &style(), 160.0);
    let [table] = layout.tables() else {
        panic!("one scrolling table");
    };
    let table = *table;
    let max = table.max_scroll();
    let y = table.rect.y + table.rect.height - style().cell_pad[1] - 4.0;
    let found = |scroll: f32| {
        (0..32).any(|step| {
            let x = table.rect.x + step as f32 * 5.0;
            layout.link_at(x, y, &[scroll]) == Some("https://far.example")
        })
    };
    assert!(!found(0.0), "out of view to the right");
    assert!(found(max), "scrolled into view");
    assert_eq!(
        layout.link_at(table.rect.x + table.rect.width + 20.0, y, &[max]),
        None,
        "nothing past the table's edge"
    );
}

#[test]
fn what_the_clip_hides_is_not_drawn_but_keeps_its_key() {
    let doc = Markdown::parse("first\n\nsecond\n\nthird");
    let mut list = list();
    let layout = doc.layout(&mut list, &style(), 300.0);
    // Only the first paragraph's band is on show.
    list.push_clip(Rect::new(0.0, 0.0, 300.0, 5.0));
    let taken = layout.paint(&mut list, 0.0, 0.0, &[], Some(TextKey::new(1, 0)));
    list.pop_clip();
    assert_eq!(taken, 3);
    let shown: Vec<_> = list
        .texts
        .iter()
        .map(|b| (b.content.as_str(), b.selectable))
        .collect();
    assert_eq!(shown, vec![("first", Some(TextKey::new(1, 0)))]);
}

#[test]
fn the_estimate_wraps_whole_words_and_breaks_long_ones() {
    // Five characters to a line.
    let lines = |text: &str| {
        let mut words = Vec::new();
        count_words(text, &mut words);
        lines(&words, 5)
    };
    assert_eq!(lines("aa bb cc"), 2);
    assert_eq!(lines("aaa bbb"), 2);
    assert_eq!(lines("abcdefghijkl"), 3);
    assert_eq!(lines("one\n\ntwo"), 3);
    assert_eq!(lines(""), 1);
    // Characters, not bytes.
    assert_eq!(lines("→→→→→ →"), 2);
}

/// The outline lists every block once, in order, each container before what
/// it holds and a table's columns before its cells.
#[test]
fn an_outline_lays_the_blocks_out_flat_in_order() {
    let blocks = parse(
        "# Head\n\n> quoted\n>\n> - in\n\n- one\n- two\n\n  more\n\n```\ncode\n```\n\n---\n\n\
         | a | b |\n|---|---|\n| c |",
    );
    let outline = Outline::of(&blocks);
    let kinds: Vec<String> = outline
        .shapes()
        .iter()
        .map(|shape| match shape {
            Shape::Text { face, .. } => match face {
                Face::Body => "text".into(),
                Face::Heading(level) => format!("h{level}"),
                Face::Code => "code".into(),
            },
            Shape::Quote { len } => format!("quote {len}"),
            Shape::List { len } => format!("list {len}"),
            Shape::Item { len } => format!("item {len}"),
            Shape::Rule => "rule".into(),
            Shape::Table { columns, rows } => format!("table {columns}x{rows}"),
            Shape::Column(_) => "column".into(),
            Shape::Cell(_) => "cell".into(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "h1", "quote 4", "text", "list 2", "item 1", "text", "list 5", "item 1", "text",
            "item 2", "text", "text", "code", "rule", "table 2x1", "column", "column", "cell",
            "cell", "cell", "cell",
        ]
    );
}

/// The estimate may overshoot, but hardly undershoot: a transcript places
/// rows by it until they are measured, and a row that grows on measuring
/// moves everything under it.
#[test]
fn an_estimate_is_near_the_measured_height_and_rarely_under_it() {
    let prose = "Some words in a paragraph that runs on for a while, long enough \
                 that it wraps onto a second line at the narrower widths.";
    let sources = [
        TABLE.to_owned(),
        format!("{TABLE}\n\n{prose}\n\n- a\n- b\n\n```\ncode\nlines\n```"),
        format!("## Heading\n\n{prose}\n\n1. **one** `code`\n2. two\n   - inner {prose}"),
        format!("> {prose}\n\n---\n\n```\n{}\n```", "x".repeat(200)),
    ];
    let style = style();
    let mut list = list();
    for source in &sources {
        let doc = Markdown::parse(source);
        for width in [300.0, 400.0, 700.0] {
            let measured = doc.layout(&mut list, &style, width).height();
            let estimate = doc.estimate_height(&style.metrics(), width);
            assert!(
                estimate >= measured * 0.95 && estimate <= measured * 1.6,
                "at {width}: estimate {estimate} vs measured {measured} for {source:?}"
            );
        }
    }
}
