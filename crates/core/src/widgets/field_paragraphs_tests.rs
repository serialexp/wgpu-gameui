//! Paragraphs laid out one at a time answer every caret question as the
//! whole text laid out at once does, and only the paragraphs asked about are
//! laid out.

use super::*;
use crate::text::byte_on_adjacent_line;

const FIELD: FieldText = FieldText {
    font_size: 16.0,
    wrap: WrapMode::WordOrGlyph,
    max_width: 160.0,
    direction: crate::TextDirection::Auto,
};

/// How far apart two positions may be, in px.
const NEAR: f32 = 0.01;

/// Paragraphs that wrap, an empty one, a CRLF break, right-to-left text, a
/// word longer than the width, and a final break.
const MIXED: &str = "first line, long enough to wrap twice in a narrow field\n\
                     \n\
                     short\r\n\
                     שלום עולם, ועוד מילים כדי לגלוש\n\
                     abcdefghijklmnopqrstuvwxyzabcdefghijklmnop\n";

fn synced(text: &str) -> FieldParagraphs {
    let mut paragraphs = FieldParagraphs::default();
    paragraphs.sync(text, FIELD);
    paragraphs
}

fn whole(list: &mut DrawList, text: &str) -> Arc<[CaretPos]> {
    list.caret_layout(&FIELD.block(text, 0.0, 0.0))
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= NEAR
}

#[test]
fn a_text_splits_where_cosmic_text_breaks_its_lines() {
    assert_eq!(segments("a\nb\r\nc\n"), [0..1, 2..3, 5..6, 7..7]);
    let one = |range: Range<usize>| vec![range];
    assert_eq!(segments(""), one(0..0));
    assert_eq!(segments("one"), one(0..3));
}

#[test]
fn a_long_paragraph_is_cut_into_pieces_at_word_breaks() {
    let text = "word ".repeat(10_000);
    let pieces = segments(&text);
    assert!(pieces.len() >= 3, "{} pieces", pieces.len());
    for pair in pieces.windows(2) {
        assert_eq!(pair[0].end, pair[1].start, "pieces follow on");
        assert!(pair[0].len() <= PIECE_BYTES);
        assert!(
            text[..pair[0].end].ends_with(' ') || text[pair[0].end..].starts_with(' '),
            "cut at a word break: {:?}",
            pair[0]
        );
    }
    assert_eq!(pieces.last().unwrap().end, text.len());
}

#[test]
fn a_long_paragraph_with_no_word_break_is_cut_between_characters() {
    let text = "é".repeat(PIECE_BYTES);
    let pieces = segments(&text);
    assert!(pieces.len() >= 2);
    for piece in &pieces {
        assert!(piece.len() <= PIECE_BYTES);
        assert!(text.is_char_boundary(piece.start) && text.is_char_boundary(piece.end));
    }
}

#[test]
fn every_caret_is_where_the_whole_text_puts_it() {
    let mut list = DrawList::new();
    let all = whole(&mut list, MIXED);
    let mut paragraphs = synced(MIXED);
    for want in all.iter().map(|p| caret_for_byte(&all, p.byte)) {
        let got = paragraphs.caret(&mut list, FIELD, want.byte);
        assert_eq!(got.byte, want.byte);
        assert!(
            near(got.x, want.x)
                && near(got.top, want.line_top)
                && near(got.height, want.line_height),
            "byte {}: {got:?} against {want:?}",
            want.byte
        );
    }
    let last = all.last().unwrap();
    let total = paragraphs.top(paragraphs.len());
    assert!(
        near(total, last.line_top + last.line_height),
        "total height {total}"
    );
}

#[test]
fn a_point_picks_the_byte_the_whole_text_picks() {
    let mut list = DrawList::new();
    let all = whole(&mut list, MIXED);
    let mut paragraphs = synced(MIXED);
    let bottom = all.last().map_or(0.0, |p| p.line_top + p.line_height);
    let mut y = -10.0;
    while y < bottom + 10.0 {
        for x in [-5.0, 0.0, 7.5, 40.0, 90.0, 159.0, 200.0] {
            assert_eq!(
                paragraphs.byte_at(&mut list, FIELD, x, y),
                byte_at_point(&all, x, y),
                "at ({x}, {y})"
            );
            assert_eq!(
                paragraphs.byte_under(&mut list, FIELD, x, y),
                byte_under_point(&all, x, y),
                "under ({x}, {y})"
            );
        }
        y += 4.0;
    }
}

#[test]
fn up_and_down_land_where_they_do_in_the_whole_text() {
    let mut list = DrawList::new();
    let all = whole(&mut list, MIXED);
    let mut paragraphs = synced(MIXED);
    for from in all.iter() {
        for dir in [-1, 1] {
            assert_eq!(
                paragraphs.adjacent(&mut list, FIELD, from.byte, dir, from.x),
                byte_on_adjacent_line(&all, from.byte, dir, from.x),
                "from byte {} going {dir}",
                from.byte
            );
        }
    }
}

#[test]
fn home_and_end_bounds_are_the_whole_texts_visual_line() {
    let mut list = DrawList::new();
    let all = whole(&mut list, MIXED);
    let mut paragraphs = synced(MIXED);
    for at in all.iter() {
        let line = on_line(&all, caret_for_byte(&all, at.byte).line, |p| p.line);
        let want = (
            line.iter().map(|p| p.byte).min().unwrap(),
            line.iter().map(|p| p.byte).max().unwrap(),
        );
        assert_eq!(
            paragraphs.line_bounds(&mut list, FIELD, at.byte),
            want,
            "byte {}",
            at.byte
        );
    }
}

#[test]
fn only_the_paragraphs_in_the_band_are_laid_out() {
    let text = (0..8_000)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut list = DrawList::new();
    let mut paragraphs = synced(&text);
    let shown = paragraphs.shown(&mut list, FIELD, 0.0, 100.0);
    let line = FIELD.line_height();
    assert!(
        (1..=(100.0 / line).ceil() as usize + 1).contains(&shown.len()),
        "{} paragraphs shown",
        shown.len()
    );
    let measured = (0..paragraphs.len())
        .filter(|&i| paragraphs.is_measured(i))
        .count();
    assert_eq!(measured, shown.len(), "laid out only what was shown");
    let stats = list
        .text_measurer
        .font_system_handle()
        .lock()
        .unwrap()
        .layout_stats();
    assert!(
        stats.layouts <= shown.len() + 1,
        "{} layouts kept",
        stats.layouts
    );
}

#[test]
fn an_edit_keeps_the_heights_of_the_paragraphs_it_left_alone() {
    let text = "one\ntwo\nthree\nfour";
    let mut list = DrawList::new();
    let mut paragraphs = synced(text);
    for segment in 0..paragraphs.len() {
        paragraphs.carets(&mut list, FIELD, segment);
    }
    let splice = paragraphs
        .sync("one\ntwo, longer now\nthree\nfour", FIELD)
        .expect("the text changed");
    let measured: Vec<bool> = (0..4).map(|i| paragraphs.is_measured(i)).collect();
    assert_eq!(measured, [true, false, true, true]);
    assert_eq!(splice.index(0), (0, true));
    assert_eq!(splice.index(1), (1, false));
    assert_eq!(splice.index(3), (3, true));
    assert_eq!(
        paragraphs.sync("one\ntwo, longer now\nthree\nfour", FIELD),
        None
    );
}

#[test]
fn a_new_line_splits_a_paragraph_and_shifts_the_ones_after_it() {
    let mut list = DrawList::new();
    let mut paragraphs = synced("one\ntwo\nthree");
    for segment in 0..paragraphs.len() {
        paragraphs.carets(&mut list, FIELD, segment);
    }
    let splice = paragraphs.sync("one\ntw\no\nthree", FIELD).unwrap();
    assert_eq!(paragraphs.len(), 4);
    assert_eq!(splice.index(2), (3, true), "three moved down one");
    assert!(paragraphs.is_measured(0) && paragraphs.is_measured(3));
    assert!(!paragraphs.is_measured(1) && !paragraphs.is_measured(2));
    assert_eq!(paragraphs.text(3), "three");
}

#[test]
fn the_first_split_maps_everything_to_the_first_paragraph() {
    let mut paragraphs = FieldParagraphs::default();
    let splice = paragraphs.sync("one\ntwo\nthree", FIELD).unwrap();
    assert_eq!(splice.index(0), (0, false));
}

#[test]
fn a_new_width_turns_every_height_back_into_an_estimate() {
    let mut list = DrawList::new();
    let mut paragraphs = synced("one\ntwo");
    paragraphs.carets(&mut list, FIELD, 0);
    assert!(paragraphs.is_measured(0));
    let wider = FieldText {
        max_width: 300.0,
        ..FIELD
    };
    assert_eq!(paragraphs.sync("one\ntwo", wider), None);
    assert!(!paragraphs.is_measured(0));
}

#[test]
fn the_caret_at_a_cut_ends_the_first_piece() {
    let text = "x".repeat(PIECE_BYTES + 10);
    let mut list = DrawList::new();
    let mut paragraphs = synced(&text);
    assert_eq!(paragraphs.len(), 2);
    let cut = paragraphs.range(1).start;
    let caret = paragraphs.caret(&mut list, FIELD, cut);
    assert_eq!(caret.segment, 0, "on the first piece");
    let after = paragraphs.caret(&mut list, FIELD, cut + 1);
    assert_eq!(after.segment, 1);
    assert!(after.top > caret.top, "the second piece starts below");
}
