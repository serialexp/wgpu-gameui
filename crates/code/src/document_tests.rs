use super::*;

fn change(range: Range<usize>, text: &str) -> Change {
    Change {
        range,
        text: text.to_owned(),
    }
}

#[test]
fn lines_end_at_newlines_and_leave_their_endings_out() {
    let document = Document::new("fn a\r\n  b\nc");
    assert_eq!(document.line_count(), 3);
    assert_eq!(document.line(0), "fn a");
    assert_eq!(document.line(1), "  b");
    assert_eq!(document.line(2), "c");
    assert_eq!(document.line_start(1), 6);
    assert_eq!(document.line_end(0), 4, "before the \\r");
    assert_eq!(document.line_end(2), 11);
    assert_eq!(document.line_of(5), 0, "the \\n is the first line's");
    assert_eq!(document.line_of(6), 1);
    assert_eq!(document.line_of(11), 2, "the end is on the last line");
}

#[test]
fn a_file_ending_in_a_newline_has_an_empty_last_line() {
    let document = Document::new("a\n");
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.line(1), "");
    let empty = Document::new("");
    assert_eq!(empty.line_count(), 1);
    assert_eq!(empty.line(0), "");
}

#[test]
fn other_unicode_line_breaks_stay_inside_their_line() {
    // As for Tree-sitter, whose rows the document's lines must match.
    let document = Document::new("a\u{2028}b\rc\nd");
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.line(0), "a\u{2028}b\rc");
}

#[test]
fn the_line_ending_is_the_files_own() {
    assert_eq!(Document::new("a\r\nb\nc").line_ending(), "\r\n");
    assert_eq!(Document::new("a\nb\r\nc").line_ending(), "\n");
    assert_eq!(Document::new("one line").line_ending(), "\n");
}

#[test]
fn a_point_is_a_row_and_a_byte_column() {
    let document = Document::new("ab\nçd\n");
    assert_eq!(document.point(0), Point { row: 0, column: 0 });
    assert_eq!(document.point(2), Point { row: 0, column: 2 });
    assert_eq!(document.point(3), Point { row: 1, column: 0 });
    assert_eq!(
        document.point(5),
        Point { row: 1, column: 2 },
        "ç is two bytes"
    );
    assert_eq!(document.point(7), Point { row: 2, column: 0 });
}

#[test]
fn an_edit_changes_the_text_the_selection_and_the_version() {
    let mut document = Document::new("let x = 1;");
    let version = document.version();
    document.edit(
        vec![change(8..9, "42")],
        Selection::caret(10),
        EditKind::Other,
    );
    assert_eq!(document.text(), "let x = 42;");
    assert_eq!(document.selection(), Selection::caret(10));
    assert!(document.version() > version);
}

#[test]
fn an_edit_is_told_to_the_parser_with_its_points() {
    let mut document = Document::new("a\nbc\nd");
    document.take_edits();
    // "bc\nd" → "X": two rows become one.
    document.edit(
        vec![change(2..6, "X")],
        Selection::caret(3),
        EditKind::Other,
    );
    assert_eq!(
        document.take_edits(),
        [TextEdit {
            start: 2,
            old_end: 6,
            new_end: 3,
            start_point: Point { row: 1, column: 0 },
            old_end_point: Point { row: 2, column: 1 },
            new_end_point: Point { row: 1, column: 1 },
        }]
    );
    assert!(document.take_edits().is_empty(), "taken once");
}

#[test]
fn changes_in_one_edit_are_all_against_the_text_before_it() {
    // Indenting three lines: given first to last, applied as one.
    let mut document = Document::new("a\nb\nc");
    document.take_edits();
    document.edit(
        vec![change(0..0, "  "), change(2..2, "  "), change(4..4, "  ")],
        Selection::caret(0),
        EditKind::Other,
    );
    assert_eq!(document.text(), "  a\n  b\n  c");
    // Applied last to first, so each edit's positions hold when it comes.
    let starts: Vec<usize> = document
        .take_edits()
        .iter()
        .map(|edit| edit.start)
        .collect();
    assert_eq!(starts, [4, 2, 0]);
}

#[test]
fn undo_and_redo_bring_back_the_text_and_the_selection() {
    let mut document = Document::new("hello");
    document.set_selection(Selection { anchor: 0, head: 5 });
    document.edit(
        vec![change(0..5, "bye")],
        Selection::caret(3),
        EditKind::Other,
    );
    assert!(document.undo());
    assert_eq!(document.text(), "hello");
    assert_eq!(document.selection(), Selection { anchor: 0, head: 5 });
    assert!(document.redo());
    assert_eq!(document.text(), "bye");
    assert_eq!(document.selection(), Selection::caret(3));
    assert!(!document.redo(), "nothing more to redo");
}

#[test]
fn undo_is_told_to_the_parser_too() {
    let mut document = Document::new("ab");
    document.edit(
        vec![change(1..1, "XY")],
        Selection::caret(3),
        EditKind::Other,
    );
    document.take_edits();
    document.undo();
    assert_eq!(
        document.take_edits(),
        [TextEdit {
            start: 1,
            old_end: 3,
            new_end: 1,
            start_point: Point { row: 0, column: 1 },
            old_end_point: Point { row: 0, column: 3 },
            new_end_point: Point { row: 0, column: 1 },
        }]
    );
}

#[test]
fn a_new_edit_drops_what_could_be_redone() {
    let mut document = Document::new("");
    document.edit(
        vec![change(0..0, "a")],
        Selection::caret(1),
        EditKind::Other,
    );
    document.undo();
    document.edit(
        vec![change(0..0, "b")],
        Selection::caret(1),
        EditKind::Other,
    );
    assert!(!document.redo());
    assert_eq!(document.text(), "b");
}

/// Types `text` at the caret, one character at a time.
fn type_text(document: &mut Document, text: &str) {
    for character in text.chars() {
        let at = document.selection().head;
        document.edit(
            vec![change(at..at, &character.to_string())],
            Selection::caret(at + character.len_utf8()),
            EditKind::Typing,
        );
    }
}

#[test]
fn a_run_of_typing_is_undone_at_once() {
    let mut document = Document::new("");
    type_text(&mut document, "word");
    assert!(document.undo());
    assert_eq!(document.text(), "");
    assert!(!document.undo());
}

#[test]
fn moving_the_caret_ends_a_run_of_typing() {
    let mut document = Document::new("");
    type_text(&mut document, "ab");
    document.set_selection(Selection::caret(1));
    type_text(&mut document, "X");
    document.undo();
    assert_eq!(document.text(), "ab");
    document.undo();
    assert_eq!(document.text(), "");
}

#[test]
fn putting_the_caret_where_it_already_is_doesnt_end_a_run() {
    let mut document = Document::new("");
    type_text(&mut document, "ab");
    document.set_selection(Selection::caret(2));
    type_text(&mut document, "c");
    document.undo();
    assert_eq!(document.text(), "");
}

#[test]
fn typing_and_deleting_are_separate_steps() {
    let mut document = Document::new("");
    type_text(&mut document, "abc");
    for _ in 0..2 {
        let at = document.selection().head;
        document.edit(
            vec![change(at - 1..at, "")],
            Selection::caret(at - 1),
            EditKind::Deleting,
        );
    }
    assert_eq!(document.text(), "a");
    document.undo();
    assert_eq!(document.text(), "abc", "both deletions at once");
    document.undo();
    assert_eq!(document.text(), "");
}

#[test]
fn other_edits_are_a_step_each() {
    let mut document = Document::new("");
    for _ in 0..2 {
        let at = document.selection().head;
        document.edit(
            vec![change(at..at, "\n")],
            Selection::caret(at + 1),
            EditKind::Other,
        );
    }
    document.undo();
    assert_eq!(document.text(), "\n");
}

#[test]
fn undo_ends_a_run_of_typing() {
    let mut document = Document::new("");
    type_text(&mut document, "ab");
    document.undo();
    type_text(&mut document, "c");
    document.redo();
    assert_eq!(document.text(), "c", "the new typing dropped the redo");
    document.undo();
    assert_eq!(document.text(), "");
}

#[test]
fn a_selection_runs_either_way() {
    let backwards = Selection { anchor: 7, head: 2 };
    assert_eq!(backwards.range(), 2..7);
    assert!(!backwards.is_empty());
    assert!(Selection::caret(3).is_empty());
}

#[test]
fn a_long_file_is_read_by_the_line_without_copying_it_whole() {
    let text: String = (0..100_000).map(|n| format!("line {n}\n")).collect();
    let document = Document::new(&text);
    assert_eq!(document.line_count(), 100_001);
    assert_eq!(document.line(54_321), "line 54321");
    assert_eq!(document.line_of(document.line_start(99_999)), 99_999);
}

#[test]
fn the_text_comes_out_in_chunks_for_the_parser() {
    let text: String = (0..10_000).map(|n| format!("{n},")).collect();
    let document = Document::new(&text);
    let mut read = String::new();
    while read.len() < document.len() {
        read.push_str(document.chunk_at(read.len()));
    }
    assert_eq!(read, text);
    assert_eq!(document.chunk_at(document.len()), "");
}

#[test]
fn characters_are_counted_in_a_range() {
    let document = Document::new("añb\n€c");
    assert_eq!(document.chars_in(0..document.len()), 6);
    assert_eq!(document.chars_in(1..3), 1);
    assert_eq!(document.chars_in(2..2), 0);
}
