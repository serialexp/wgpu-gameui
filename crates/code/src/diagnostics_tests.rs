use super::*;
use crate::document::{Change, EditKind, Selection};

fn document(text: &str) -> Document {
    Document::new(text)
}

fn error(range: Range<usize>, message: &str) -> Diagnostic {
    Diagnostic::new(range, Severity::Error, message)
}

/// Replace `range` with `text` and hand the edit to `diagnostics`.
fn edit(document: &mut Document, diagnostics: &mut Diagnostics, range: Range<usize>, text: &str) {
    document.take_edits();
    let at = range.start + text.len();
    document.edit(
        vec![Change {
            range,
            text: text.into(),
        }],
        Selection::caret(at),
        EditKind::Other,
    );
    diagnostics.follow(&document.take_edits());
}

fn ranges(diagnostics: &Diagnostics) -> Vec<Range<usize>> {
    diagnostics.all().iter().map(|d| d.range.clone()).collect()
}

#[test]
fn an_empty_range_is_widened_to_the_word_at_it_or_one_character() {
    let text = "let plyer = 1; (x)";
    let doc = document(text);
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(
        &doc,
        vec![
            error(4..4, "unknown"),
            error(15..15, "paren"),
            error(18..18, "end"),
        ],
    );
    assert_eq!(ranges(&diagnostics), vec![4..9, 15..16, 18..18]);
}

#[test]
fn ranges_are_clamped_to_the_text_and_its_characters() {
    // "é" is two bytes: 1..3.
    let doc = document("aé b");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(
        &doc,
        vec![
            error(2..40, "past the end"),
            error(Range { start: 9, end: 3 }, "backwards"),
        ],
    );
    // 2 is inside "é": back to its start. 40 is past the end: the end.
    assert_eq!(ranges(&diagnostics), vec![1..5, 5..5]);
}

#[test]
fn they_are_kept_in_order_and_counted_by_severity() {
    let doc = document("one two three");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(
        &doc,
        vec![
            Diagnostic::new(8..13, Severity::Warning, "w"),
            error(0..3, "e"),
            Diagnostic::new(4..7, Severity::Info, "i"),
        ],
    );
    assert_eq!(ranges(&diagnostics), vec![0..3, 4..7, 8..13]);
    assert_eq!(diagnostics.count(Severity::Error), 1);
    assert_eq!(diagnostics.count(Severity::Warning), 1);
    assert_eq!(diagnostics.count(Severity::Info), 1);
}

#[test]
fn typing_before_a_problem_moves_it_along() {
    let mut doc = document("x = plyer\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(&doc, vec![error(4..9, "unknown")]);
    edit(&mut doc, &mut diagnostics, 0..0, "let ");
    assert_eq!(ranges(&diagnostics), vec![8..13]);
    // Right at its start, too.
    edit(&mut doc, &mut diagnostics, 8..8, "&");
    assert_eq!(ranges(&diagnostics), vec![9..14]);
}

#[test]
fn typing_after_or_right_at_its_end_leaves_it() {
    let mut doc = document("plyer\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(&doc, vec![error(0..5, "unknown")]);
    edit(&mut doc, &mut diagnostics, 5..5, "s");
    assert_eq!(ranges(&diagnostics), vec![0..5]);
}

#[test]
fn typing_inside_stretches_it_and_deleting_inside_shrinks_it() {
    let mut doc = document("plyer\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(&doc, vec![error(0..5, "unknown")]);
    edit(&mut doc, &mut diagnostics, 2..2, "a");
    assert_eq!(ranges(&diagnostics), vec![0..6]);
    edit(&mut doc, &mut diagnostics, 1..4, "");
    assert_eq!(ranges(&diagnostics), vec![0..3]);
}

#[test]
fn deleting_across_an_edge_keeps_what_is_left() {
    let mut doc = document("ab plyer\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(&doc, vec![error(3..8, "unknown")]);
    // "b pl" goes: "yer" is left, from 1.
    edit(&mut doc, &mut diagnostics, 1..5, "");
    assert_eq!(ranges(&diagnostics), vec![1..4]);
    // And over its end: "ye" stays.
    edit(&mut doc, &mut diagnostics, 3..5, "");
    assert_eq!(ranges(&diagnostics), vec![1..3]);
}

#[test]
fn deleting_all_of_it_leaves_it_empty_where_it_was() {
    let mut doc = document("a plyer b\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(&doc, vec![error(2..7, "unknown")]);
    edit(&mut doc, &mut diagnostics, 1..8, "");
    assert_eq!(ranges(&diagnostics), vec![1..1]);
}

#[test]
fn replacing_it_whole_leaves_it_empty_before_the_new_text() {
    let mut doc = document("a plyer b\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(&doc, vec![error(2..7, "unknown")]);
    edit(&mut doc, &mut diagnostics, 2..7, "player");
    assert_eq!(ranges(&diagnostics), vec![2..2]);
}

#[test]
fn the_ones_on_a_line_are_found_and_the_worst_named() {
    let doc = document("one\ntwo three\nfour\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(
        &doc,
        vec![
            Diagnostic::new(4..7, Severity::Warning, "two"),
            error(8..13, "three"),
            // Spans the line break: on both lines.
            Diagnostic::new(2..5, Severity::Info, "across"),
            error(14..18, "four"),
        ],
    );
    let line = doc.line_start(1)..doc.line_end(1);
    let on: Vec<&str> = diagnostics
        .on(line.clone())
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(on, vec!["across", "two", "three"]);
    assert_eq!(diagnostics.worst_on(line), Some(Severity::Error));
    let first = doc.line_start(0)..doc.line_end(0);
    assert_eq!(diagnostics.worst_on(first), Some(Severity::Info));
}

#[test]
fn an_empty_one_is_on_the_line_it_sits_on() {
    let doc = document("ab\ncd\n");
    let mut diagnostics = Diagnostics::default();
    // At the end of line 0, before its "\n".
    diagnostics.set(&doc, vec![error(2..2, "end")]);
    assert_eq!(diagnostics.on(0..2).count(), 1);
    assert_eq!(diagnostics.on(3..5).count(), 0);
}

#[test]
fn the_next_of_a_kind_is_after_the_caret_and_wraps_round() {
    let doc = document("aa bb cc dd\n");
    let mut diagnostics = Diagnostics::default();
    diagnostics.set(
        &doc,
        vec![
            error(0..2, "a"),
            Diagnostic::new(3..5, Severity::Warning, "b"),
            error(6..8, "c"),
        ],
    );
    let next = |at| {
        diagnostics
            .next_after(at, Severity::Error)
            .map(|d| d.range.start)
    };
    assert_eq!(next(0), Some(6), "not the one the caret is on");
    assert_eq!(next(4), Some(6));
    assert_eq!(next(6), Some(0), "wraps round");
    assert_eq!(
        diagnostics
            .next_after(9, Severity::Warning)
            .map(|d| d.range.start),
        Some(3)
    );
    assert_eq!(
        diagnostics
            .next_after(0, Severity::Info)
            .map(|d| d.range.start),
        None
    );
}

#[test]
fn a_long_file_with_many_problems_is_followed_edit_by_edit() {
    // Representative scale: a problem on every line of a 20,000-line file,
    // typed into near the top. Each edit touches each problem once.
    let line = "let value = missing;\n";
    let mut doc = document(&line.repeat(20_000));
    let mut diagnostics = Diagnostics::default();
    let list = (0..20_000)
        .map(|n| error(n * line.len() + 12..n * line.len() + 19, "missing"))
        .collect();
    diagnostics.set(&doc, list);
    for at in 0..20 {
        edit(&mut doc, &mut diagnostics, at..at, "x");
    }
    let last = diagnostics.all().last().unwrap().range.clone();
    let start = 19_999 * line.len() + 12 + 20;
    assert_eq!(last, start..start + 7);
    assert_eq!(doc.slice(last), "missing");
}
