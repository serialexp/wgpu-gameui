use super::*;

/// A document from `marked`: `‸` is the caret; `«` and `»` are a
/// selection's anchor and head.
fn doc(marked: &str) -> Document {
    let mut text = String::new();
    let (mut anchor, mut head) = (None, None);
    for character in marked.chars() {
        match character {
            '‸' => {
                anchor = Some(text.len());
                head = Some(text.len());
            }
            '«' => anchor = Some(text.len()),
            '»' => head = Some(text.len()),
            _ => text.push(character),
        }
    }
    let mut document = Document::new(&text);
    let head = head.expect("a caret or a selection");
    document.set_selection(Selection {
        anchor: anchor.unwrap_or(head),
        head,
    });
    document
}

/// The document's text with its selection marked as [`doc`] reads it.
fn marked(document: &Document) -> String {
    let mut text = document.text();
    let Selection { anchor, head } = document.selection();
    if anchor == head {
        text.insert(head, '‸');
    } else {
        // The later one first, so the earlier position still holds.
        let (first, second) = if anchor < head {
            ((anchor, '«'), (head, '»'))
        } else {
            ((head, '»'), (anchor, '«'))
        };
        text.insert(second.0, second.1);
        text.insert(first.0, first.1);
    }
    text
}

fn rules() -> Rules {
    Rules::default()
}

fn lua() -> Rules {
    crate::Language::Lua.rules(4)
}

#[test]
fn typing_replaces_the_selection() {
    let mut document = doc("let «x» = 1;");
    type_text(&mut document, "y", &rules());
    assert_eq!(marked(&document), "let y‸ = 1;");
}

#[test]
fn a_bracket_closes_itself() {
    for (open, pair) in [("(", "()"), ("[", "[]"), ("{", "{}")] {
        let mut document = doc("f‸");
        type_text(&mut document, open, &rules());
        assert_eq!(marked(&document), format!("f{}‸{}", &pair[..1], &pair[1..]));
    }
}

#[test]
fn a_bracket_before_a_word_stays_on_its_own() {
    let mut document = doc("f(‸x)");
    type_text(&mut document, "[", &rules());
    assert_eq!(marked(&document), "f([‸x)");
}

#[test]
fn a_quote_closes_itself_but_not_inside_a_word() {
    let mut document = doc("x = ‸");
    type_text(&mut document, "\"", &rules());
    assert_eq!(marked(&document), "x = \"‸\"");
    let mut document = doc("don‸");
    type_text(&mut document, "'", &rules());
    assert_eq!(marked(&document), "don'‸");
    let mut document = doc("‸word");
    type_text(&mut document, "'", &rules());
    assert_eq!(marked(&document), "'‸word");
}

#[test]
fn a_language_can_leave_a_quote_alone() {
    // Rust's lifetimes: 'a.
    let rust = Rules {
        pairs: &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"')],
        ..Rules::default()
    };
    let mut document = doc("fn f<‸");
    type_text(&mut document, "'", &rust);
    assert_eq!(marked(&document), "fn f<'‸");
}

#[test]
fn typing_a_closer_steps_over_the_one_there() {
    let mut document = doc("f(x‸)");
    type_text(&mut document, ")", &rules());
    assert_eq!(marked(&document), "f(x)‸");
    let mut document = doc("\"x‸\"");
    type_text(&mut document, "\"", &rules());
    assert_eq!(marked(&document), "\"x\"‸");
}

#[test]
fn an_opener_wraps_the_selection() {
    let mut document = doc("call «x + 1»");
    type_text(&mut document, "(", &rules());
    assert_eq!(marked(&document), "call («x + 1»)");
    let mut document = doc("«name»");
    type_text(&mut document, "\"", &rules());
    assert_eq!(marked(&document), "\"«name»\"");
}

#[test]
fn closing_by_itself_can_be_turned_off() {
    let off = Rules {
        auto_close: false,
        ..Rules::default()
    };
    let mut document = doc("f‸)");
    type_text(&mut document, "(", &off);
    type_text(&mut document, ")", &off);
    assert_eq!(marked(&document), "f()‸)");
}

#[test]
fn enter_keeps_the_indent() {
    let mut document = doc("    let x = 1;‸");
    newline(&mut document, &rules());
    assert_eq!(marked(&document), "    let x = 1;\n    ‸");
}

#[test]
fn enter_after_an_opener_steps_in() {
    let mut document = doc("  if x {‸");
    newline(&mut document, &rules());
    assert_eq!(marked(&document), "  if x {\n      ‸");
}

#[test]
fn enter_between_a_pair_opens_it_onto_its_own_lines() {
    let mut document = doc("fn f() {‸}");
    newline(&mut document, &rules());
    assert_eq!(marked(&document), "fn f() {\n    ‸\n}");
}

#[test]
fn enter_uses_the_files_line_ending() {
    let mut document = doc("a\r\n  b‸");
    newline(&mut document, &rules());
    assert_eq!(marked(&document), "a\r\n  b\r\n  ‸");
}

#[test]
fn enter_replaces_the_selection() {
    let mut document = doc("  a«bc»d");
    newline(&mut document, &rules());
    assert_eq!(marked(&document), "  a\n  ‸d");
}

#[test]
fn enter_after_luas_block_words_steps_in() {
    for line in [
        "if x then",
        "while x do",
        "else",
        "repeat",
        "local function f(a)",
        "f(function()",
    ] {
        let mut document = doc(&format!("{line}‸"));
        newline(&mut document, &lua());
        assert_eq!(marked(&document), format!("{line}\n    ‸"), "{line}");
    }
    // A word ending in one isn't one.
    let mut document = doc("undo‸");
    newline(&mut document, &lua());
    assert_eq!(marked(&document), "undo\n‸");
}

#[test]
fn tab_reaches_the_next_stop() {
    let mut document = doc("ab‸");
    indent(&mut document, &rules());
    assert_eq!(marked(&document), "ab  ‸");
    let mut document = doc("‸x");
    indent(&mut document, &rules());
    assert_eq!(marked(&document), "    ‸x");
}

#[test]
fn tab_indents_every_line_of_a_selection() {
    let mut document = doc("«a\n\nb\nc»");
    indent(&mut document, &rules());
    // The empty line is left empty; the lines stay selected.
    assert_eq!(marked(&document), "«    a\n\n    b\n    c»");
}

#[test]
fn a_selection_ending_at_a_lines_start_leaves_that_line_alone() {
    let mut document = doc("«a\nb\n»c");
    indent(&mut document, &rules());
    assert_eq!(marked(&document), "«    a\n    b»\nc");
}

#[test]
fn shift_tab_outdents_by_a_step_or_what_there_is() {
    let mut document = doc("      x‸");
    outdent(&mut document, &rules());
    assert_eq!(marked(&document), "  x‸");
    let mut document = doc("  x‸");
    outdent(&mut document, &rules());
    assert_eq!(marked(&document), "x‸");
    let mut document = doc("\tx‸");
    outdent(&mut document, &rules());
    assert_eq!(marked(&document), "x‸", "a tab is a step");
    let mut document = doc("  ‸  x");
    outdent(&mut document, &rules());
    assert_eq!(marked(&document), "‸x", "the caret stays on the line");
}

#[test]
fn shift_tab_outdents_every_line_of_a_selection() {
    let mut document = doc("«    a\n  b\nc»");
    outdent(&mut document, &rules());
    assert_eq!(marked(&document), "«a\nb\nc»");
}

#[test]
fn backspace_takes_a_pair_together() {
    let mut document = doc("f(‸)");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "f‸");
}

#[test]
fn backspace_in_an_indent_takes_a_whole_step() {
    let mut document = doc("      ‸x");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "    ‸x");
    let mut document = doc("    ‸");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "‸");
}

#[test]
fn backspace_takes_one_character_otherwise() {
    let mut document = doc("a🇳🇱‸");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "a‸", "a flag is one character");
    let mut document = doc("a  b ‸");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "a  b‸");
}

#[test]
fn backspace_at_a_lines_start_joins_it_to_the_one_before() {
    let mut document = doc("a\r\n‸b");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "a‸b");
    let mut document = doc("‸a");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "‸a", "nothing before the start");
}

#[test]
fn backspace_and_delete_take_the_selection() {
    let mut document = doc("a«bc»d");
    backspace(&mut document, &rules());
    assert_eq!(marked(&document), "a‸d");
    let mut document = doc("a»bc«d");
    delete_forward(&mut document);
    assert_eq!(marked(&document), "a‸d");
}

#[test]
fn delete_takes_the_next_character_or_line_ending() {
    let mut document = doc("‸éa");
    delete_forward(&mut document);
    assert_eq!(marked(&document), "‸a");
    let mut document = doc("a‸\r\nb");
    delete_forward(&mut document);
    assert_eq!(marked(&document), "a‸b");
    let mut document = doc("a‸");
    delete_forward(&mut document);
    assert_eq!(marked(&document), "a‸");
}

#[test]
fn deleting_by_a_motion_takes_what_it_steps_over() {
    let mut document = doc("let value‸ = 1;");
    delete_to(&mut document, Motion::WordLeft);
    assert_eq!(marked(&document), "let ‸ = 1;");
    delete_to(&mut document, Motion::WordRight);
    assert_eq!(marked(&document), "let ‸ 1;");
    // A selection goes whole, whatever the motion.
    let mut document = doc("«let» value");
    delete_to(&mut document, Motion::WordRight);
    assert_eq!(marked(&document), "‸ value");
    // Undone in one step.
    let mut document = doc("one two‸");
    delete_to(&mut document, Motion::WordLeft);
    document.undo();
    assert_eq!(document.text(), "one two");
}

#[test]
fn deleting_to_the_line_start_takes_its_indent_and_then_the_line_ending() {
    let mut document = doc("one\n    two‸ three");
    delete_to(&mut document, Motion::LineStart);
    assert_eq!(marked(&document), "one\n‸ three");
    delete_to(&mut document, Motion::LineStart);
    assert_eq!(marked(&document), "one‸ three");
}

#[test]
fn line_start_is_the_start_however_indented() {
    let mut document = doc("    two‸");
    move_caret(&mut document, Motion::LineStart, false);
    assert_eq!(marked(&document), "‸    two");
}

#[test]
fn left_and_right_step_a_character_and_cross_lines() {
    let mut document = doc("a\r\né‸b");
    move_caret(&mut document, Motion::Left, false);
    assert_eq!(marked(&document), "a\r\n‸éb");
    move_caret(&mut document, Motion::Left, false);
    assert_eq!(marked(&document), "a‸\r\néb", "over the whole line ending");
    move_caret(&mut document, Motion::Right, false);
    assert_eq!(marked(&document), "a\r\n‸éb");
}

#[test]
fn left_and_right_without_shift_collapse_a_selection_to_its_side() {
    let mut document = doc("a«bc»d");
    move_caret(&mut document, Motion::Left, false);
    assert_eq!(marked(&document), "a‸bcd");
    let mut document = doc("a»bc«d");
    move_caret(&mut document, Motion::Right, false);
    assert_eq!(marked(&document), "abc‸d");
}

#[test]
fn shift_extends_the_selection() {
    let mut document = doc("ab‸cd");
    move_caret(&mut document, Motion::Right, true);
    move_caret(&mut document, Motion::Right, true);
    assert_eq!(marked(&document), "ab«cd»");
    move_caret(&mut document, Motion::Left, true);
    move_caret(&mut document, Motion::Left, true);
    move_caret(&mut document, Motion::Left, true);
    assert_eq!(marked(&document), "a»b«cd");
}

#[test]
fn word_steps_stop_at_words_and_runs_of_punctuation() {
    let mut document = doc("‸let snake_case = a::b(1);");
    let mut stops = Vec::new();
    for _ in 0..7 {
        move_caret(&mut document, Motion::WordRight, false);
        stops.push(document.selection().head);
    }
    // After `let`, `snake_case`, `=`, `a`, `::`, `b`, `(`… each run's end.
    assert_eq!(stops, [3, 14, 16, 18, 20, 21, 22]);
    let mut document = doc("let snake_case‸ = 1");
    move_caret(&mut document, Motion::WordLeft, false);
    assert_eq!(marked(&document), "let ‸snake_case = 1");
    move_caret(&mut document, Motion::WordLeft, false);
    assert_eq!(marked(&document), "‸let snake_case = 1");
}

#[test]
fn word_steps_cross_lines() {
    let mut document = doc("a\n‸  b");
    move_caret(&mut document, Motion::WordLeft, false);
    assert_eq!(marked(&document), "a‸\n  b");
    move_caret(&mut document, Motion::WordRight, false);
    assert_eq!(marked(&document), "a\n‸  b");
    move_caret(&mut document, Motion::WordRight, false);
    assert_eq!(marked(&document), "a\n  b‸");
}

#[test]
fn home_goes_to_the_indent_then_the_lines_start() {
    let mut document = doc("x\n    let y‸ = 1;");
    move_caret(&mut document, Motion::Home, false);
    assert_eq!(marked(&document), "x\n    ‸let y = 1;");
    move_caret(&mut document, Motion::Home, false);
    assert_eq!(marked(&document), "x\n‸    let y = 1;");
    move_caret(&mut document, Motion::Home, false);
    assert_eq!(marked(&document), "x\n    ‸let y = 1;");
    move_caret(&mut document, Motion::End, false);
    assert_eq!(marked(&document), "x\n    let y = 1;‸");
}

#[test]
fn the_documents_ends() {
    let mut document = doc("a\nb‸\nc");
    move_caret(&mut document, Motion::DocumentEnd, true);
    assert_eq!(marked(&document), "a\nb«\nc»");
    move_caret(&mut document, Motion::DocumentStart, false);
    assert_eq!(marked(&document), "‸a\nb\nc");
}

#[test]
fn up_and_down_keep_their_column_over_short_lines() {
    let mut document = doc("long line‸\nab\nlonger line");
    let mut column = None;
    move_lines(&mut document, 1, false, &mut column, 4);
    assert_eq!(marked(&document), "long line\nab‸\nlonger line");
    move_lines(&mut document, 1, false, &mut column, 4);
    assert_eq!(marked(&document), "long line\nab\nlonger li‸ne");
    move_lines(&mut document, -2, false, &mut column, 4);
    assert_eq!(marked(&document), "long line‸\nab\nlonger line");
}

#[test]
fn up_and_down_count_tabs_as_their_columns() {
    let mut document = doc("\tx‸\n12345678");
    move_lines(&mut document, 1, false, &mut None, 4);
    assert_eq!(marked(&document), "\tx\n12345‸678");
}

#[test]
fn up_from_the_first_line_goes_to_its_start_and_down_from_the_last_to_its_end() {
    let mut document = doc("ab‸c\nd");
    move_lines(&mut document, -1, false, &mut None, 4);
    assert_eq!(marked(&document), "‸abc\nd");
    let mut document = doc("a\nb‸cd");
    move_lines(&mut document, 1, true, &mut None, 4);
    assert_eq!(marked(&document), "a\nb«cd»");
}

#[test]
fn a_page_is_many_lines_at_once() {
    let text: String = (0..50).map(|n| format!("{n}\n")).collect();
    let mut document = Document::new(&text);
    move_lines(&mut document, 20, false, &mut None, 4);
    assert_eq!(document.line_of(document.selection().head), 20);
}

#[test]
fn a_double_click_selects_the_word_and_a_triple_its_line() {
    let document = doc("‸let snake_case = a::b;\nnext");
    assert_eq!(word_at(&document, 6), 4..14);
    assert_eq!(word_at(&document, 4), 4..14, "at its start");
    assert_eq!(word_at(&document, 18), 18..20, "a run of punctuation");
    assert_eq!(
        word_at(&document, 22),
        21..22,
        "the line's end belongs to the run before it"
    );
    assert_eq!(line_at(&document, 1), 0..23, "with its line ending");
    assert_eq!(line_at(&document, 25), 23..27, "the last line to the end");
}

#[test]
fn copy_and_cut_take_the_selection() {
    let mut document = doc("a«bc»d");
    assert_eq!(copied(&document).as_deref(), Some("bc"));
    assert_eq!(cut(&mut document).as_deref(), Some("bc"));
    assert_eq!(marked(&document), "a‸d");
    assert_eq!(copied(&document), None, "nothing selected");
}

#[test]
fn paste_puts_the_files_line_endings_in() {
    let mut document = doc("a\r\n‸");
    paste(&mut document, "x\ny\r\nz");
    assert_eq!(marked(&document), "a\r\nx\r\ny\r\nz‸");
    let mut document = doc("«old»");
    paste(&mut document, "new\r\n");
    assert_eq!(marked(&document), "new\n‸");
}

#[test]
fn select_all_selects_all() {
    let mut document = doc("a‸\nb");
    select_all(&mut document);
    assert_eq!(marked(&document), "«a\nb»");
}

#[test]
fn a_run_of_typing_with_closers_undoes_at_once() {
    let mut document = doc("‸");
    for key in ["f", "(", "x", ")"] {
        type_text(&mut document, key, &rules());
    }
    assert_eq!(marked(&document), "f(x)‸");
    document.undo();
    assert_eq!(marked(&document), "‸");
}
