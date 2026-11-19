use super::*;
use EditKind::{Deleting, Other, Typing};

fn caret(cursor: usize) -> Caret {
    Caret {
        cursor,
        selection: None,
    }
}

/// Replace `range` of `t` with `inserted` as a field does, the caret
/// `before` it, and note it in `h`.
fn edit(
    h: &mut TextHistory,
    t: &mut String,
    range: Range<usize>,
    inserted: &str,
    before: Caret,
    kind: EditKind,
) {
    let removed = t[range.clone()].to_owned();
    t.replace_range(range.clone(), inserted);
    h.record(Edit {
        start: range.start,
        removed: &removed,
        inserted,
        before,
        after: caret(range.start + inserted.len()),
        kind,
    });
}

/// Type `typed` a character at a time at the end of `t`.
fn type_out(h: &mut TextHistory, t: &mut String, typed: &str) {
    for character in typed.chars() {
        let (at, typed) = (t.len(), character.to_string());
        edit(h, t, at..at, &typed, caret(at), Typing);
    }
}

fn empty() -> (TextHistory, String) {
    (TextHistory::default(), String::new())
}

#[test]
fn a_run_of_typing_is_one_step() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "hello");
    assert_eq!(h.steps(), (1, 0));
    assert_eq!(h.undo(&mut t), Some(caret(0)));
    assert_eq!(t, "");
    assert_eq!(h.redo(&mut t), Some(caret(5)));
    assert_eq!(t, "hello");
}

#[test]
fn moving_the_caret_ends_a_run() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "ac");
    // The caret went back between the two before this.
    edit(&mut h, &mut t, 1..1, "b", caret(1), Typing);
    assert_eq!(t, "abc");
    assert_eq!(h.steps(), (2, 0));
    assert_eq!(h.undo(&mut t), Some(caret(1)));
    assert_eq!(t, "ac");
}

#[test]
fn a_run_of_backspacing_and_one_of_deleting_are_one_step_each() {
    let (mut h, mut t) = (TextHistory::default(), String::from("abcdef"));
    for at in [5, 4, 3] {
        edit(&mut h, &mut t, at..at + 1, "", caret(at + 1), Deleting);
    }
    assert_eq!(t, "abc");
    assert_eq!(h.steps(), (1, 0));
    assert_eq!(h.undo(&mut t), Some(caret(6)));
    assert_eq!(t, "abcdef");

    let (mut h, mut t) = (TextHistory::default(), String::from("abcdef"));
    for _ in 0..3 {
        edit(&mut h, &mut t, 1..2, "", caret(1), Deleting);
    }
    assert_eq!(t, "aef");
    assert_eq!(h.steps(), (1, 0));
    h.undo(&mut t);
    assert_eq!(t, "abcdef");
}

#[test]
fn typing_after_deleting_is_a_step_of_its_own() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "ab");
    edit(&mut h, &mut t, 1..2, "", caret(2), Deleting);
    type_out(&mut h, &mut t, "c");
    assert_eq!(t, "ac");
    assert_eq!(h.steps(), (3, 0));
    h.undo(&mut t);
    assert_eq!(t, "a");
    h.undo(&mut t);
    assert_eq!(t, "ab");
}

#[test]
fn another_kind_of_edit_never_joins_a_run() {
    let (mut h, mut t) = empty();
    edit(&mut h, &mut t, 0..0, "pasted", caret(0), Other);
    edit(&mut h, &mut t, 6..6, "\n", caret(6), Other);
    type_out(&mut h, &mut t, "x");
    assert_eq!(h.steps(), (3, 0));
}

#[test]
fn typing_over_a_selection_undoes_to_the_selection() {
    let (mut h, mut t) = (TextHistory::default(), String::from("one two"));
    let selected = Caret {
        cursor: 7,
        selection: Some(4),
    };
    edit(&mut h, &mut t, 4..7, "3", selected, Typing);
    type_out(&mut h, &mut t, "!");
    assert_eq!(t, "one 3!");
    assert_eq!(h.steps(), (1, 0), "the typing after it is the same run");
    assert_eq!(h.undo(&mut t), Some(selected));
    assert_eq!(t, "one two");
}

#[test]
fn a_new_edit_drops_what_could_be_redone() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "a");
    h.undo(&mut t);
    type_out(&mut h, &mut t, "b");
    assert_eq!(h.steps(), (1, 0));
    assert_eq!(h.redo(&mut t), None);
    assert_eq!(t, "b");
}

#[test]
fn typing_after_an_undo_starts_a_new_step() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "ab");
    edit(&mut h, &mut t, 2..2, "\n", caret(2), Other);
    type_out(&mut h, &mut t, "c");
    h.undo(&mut t);
    // The caret is back where the step before ended, but an undo closed it.
    type_out(&mut h, &mut t, "d");
    h.undo(&mut t);
    assert_eq!(t, "ab\n");
}

#[test]
fn a_history_that_no_longer_fits_the_text_is_forgotten() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "hello");
    t = "replaced from outside".into();
    assert_eq!(h.undo(&mut t), None);
    assert_eq!(t, "replaced from outside");
    assert_eq!(h.steps(), (0, 0));
}

#[test]
fn multibyte_text_undoes_on_character_boundaries() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "日本語");
    let end = t.len();
    let at = end - "語".len();
    edit(&mut h, &mut t, at..end, "", caret(end), Deleting);
    assert_eq!(t, "日本");
    h.undo(&mut t);
    assert_eq!(t, "日本語");
    h.undo(&mut t);
    assert_eq!(t, "");
}

#[test]
fn a_wiped_history_keeps_no_text() {
    let (mut h, mut t) = empty();
    type_out(&mut h, &mut t, "secret");
    h.wipe();
    assert_eq!((h.steps(), h.bytes), ((0, 0), 0));
}

#[test]
fn the_oldest_steps_are_forgotten_past_the_limit() {
    let (mut h, mut t) = empty();
    for _ in 0..MAX_STEPS + 10 {
        let at = t.len();
        edit(&mut h, &mut t, at..at, "x", caret(at), Other);
    }
    assert_eq!(h.steps(), (MAX_STEPS, 0));
    let big = "y".repeat(MAX_BYTES);
    let at = t.len();
    edit(&mut h, &mut t, at..at, &big, caret(at), Other);
    assert_eq!(h.steps(), (1, 0), "one step past the bytes is kept alone");
    assert_eq!(h.bytes, MAX_BYTES);
}
