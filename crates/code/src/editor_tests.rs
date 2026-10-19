use std::cell::RefCell;
use std::rc::Rc;

use wgpu_gameui::{FocusState, Theme};

use super::*;

const ID: FocusId = 7;
/// Another focusable, after the editor in the Tab ring.
const NEXT: FocusId = 8;
const RECT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 600.0,
    height: 400.0,
};

/// An editor drawn frame by frame, with focus kept between frames.
struct Rig {
    editor: CodeEditor,
    theme: Theme,
    focus: FocusState,
    list: DrawList,
    clipboard: Rc<RefCell<String>>,
}

impl Rig {
    fn new(text: &str, language: Language) -> Self {
        Self::with(CodeEditor::new(text, language))
    }

    fn with(mut editor: CodeEditor) -> Self {
        let clipboard = Rc::new(RefCell::new(String::new()));
        let (from, to) = (clipboard.clone(), clipboard.clone());
        editor.set_clipboard(
            Rc::new(RefCell::new(move || from.borrow().clone())),
            Rc::new(RefCell::new(move |text: String| *to.borrow_mut() = text)),
        );
        Self {
            editor,
            theme: Theme::default(),
            focus: FocusState::new(),
            list: DrawList::new(),
            clipboard,
        }
    }

    /// Draw a frame under `input`.
    fn frame(&mut self, input: &InputState) -> CodeEditorOutput {
        self.list = DrawList::new();
        self.focus.begin_frame(input);
        let mut cx = DrawContext::new(
            &mut self.list,
            &mut self.focus,
            &self.theme,
            input,
            800.0,
            600.0,
        );
        let output = self.editor.draw(ID, RECT, &mut cx);
        self.focus.register(NEXT);
        self.focus.end_frame(None);
        output
    }

    /// A frame with nothing happening.
    fn idle(&mut self) -> CodeEditorOutput {
        self.frame(&away())
    }

    fn focused(mut self) -> Self {
        self.focus.focus(ID);
        self.idle();
        self
    }

    /// A frame typing `text`.
    fn type_text(&mut self, text: &str) -> CodeEditorOutput {
        self.frame(&InputState {
            text_input: text.into(),
            ..away()
        })
    }

    /// A frame with Ctrl (Cmd on macOS) and `letter` down.
    fn shortcut(&mut self, letter: char, shift: bool) -> CodeEditorOutput {
        let mut input = InputState {
            ctrl_pressed: true,
            shift_pressed: shift,
            ..away()
        };
        input.keys.key_event(Key::Char(letter), true);
        self.frame(&input)
    }

    /// The screen point inside `column` of `line`, unscrolled.
    fn at(&self, line: usize, column: f32) -> (f32, f32) {
        let drawn = &self.editor.drawn;
        (
            drawn.origin[0] + column * drawn.advance,
            drawn.origin[1] + (line as f32 + 0.5) * drawn.line_height,
        )
    }

    /// A frame pressing at `column` of `line`, the `count`th press in a row.
    fn click(&mut self, line: usize, column: f32, count: u32) {
        let (x, y) = self.at(line, column);
        self.frame(&InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: count,
            ..InputState::default()
        });
    }

    fn texts(&self) -> Vec<&str> {
        self.list
            .texts
            .iter()
            .map(|text| text.content.as_str())
            .collect()
    }

    fn block(&self, content: &str) -> &TextBlock {
        self.list
            .texts
            .iter()
            .find(|text| text.content == content)
            .unwrap_or_else(|| panic!("{content:?} in {:?}", self.texts()))
    }

    fn selection(&self) -> Selection {
        self.editor.document().selection()
    }
}

/// Input with the pointer off the editor.
fn away() -> InputState {
    InputState {
        mouse_x: 900.0,
        mouse_y: 900.0,
        ..InputState::default()
    }
}

/// A text block in `colour`, to compare another's ink with.
fn inked(colour: [f32; 4]) -> TextBlock {
    TextBlock::new("", 0.0, 0.0).with_color_f32(colour)
}

fn numbered(lines: usize) -> String {
    (0..lines).map(|n| format!("let v{n} = {n};\n")).collect()
}

#[test]
fn only_the_lines_in_view_are_drawn() {
    let mut rig = Rig::new(&numbered(10_000), Language::Rust);
    rig.idle();
    let lines = rig.editor.drawn.lines.clone();
    assert_eq!(lines.start, 0);
    assert!(lines.len() < 40, "{lines:?}");
    assert!(rig.list.texts.len() < 100, "{} texts", rig.list.texts.len());
    assert!(rig.texts().contains(&"let v0 = 0;"));
    assert!(!rig.texts().contains(&"let v500 = 500;"));
}

#[test]
fn each_line_has_its_number_beside_it() {
    let mut rig = Rig::new("a\nb\nc", Language::Plain);
    rig.idle();
    for (number, line) in [("1", "a"), ("2", "b"), ("3", "c")] {
        let number = rig.block(number);
        let line = rig.block(line);
        assert_eq!(number.y, line.y);
        assert!(number.x < line.x);
        assert_eq!(number.align, TextAlign::Right);
    }
}

#[test]
fn a_click_places_the_caret_and_focuses() {
    let mut rig = Rig::new("first\nsecond\n", Language::Plain);
    rig.idle();
    rig.click(1, 3.4, 1);
    assert!(rig.focus.is_focused(ID));
    assert_eq!(rig.selection(), Selection::caret(6 + 3));
    // Past the end of a line, its end.
    rig.click(0, 40.0, 1);
    assert_eq!(rig.selection(), Selection::caret(5));
    // Below the last line, the last line.
    rig.click(5, 0.0, 1);
    assert_eq!(rig.selection(), Selection::caret(13));
}

#[test]
fn a_click_in_a_tab_goes_to_its_nearer_end() {
    let mut rig = Rig::new("\tx", Language::Plain);
    rig.idle();
    rig.click(0, 1.0, 1);
    assert_eq!(rig.selection(), Selection::caret(0));
    rig.click(0, 3.0, 1);
    assert_eq!(rig.selection(), Selection::caret(1));
}

#[test]
fn a_double_click_selects_a_word_and_a_triple_click_its_line() {
    let mut rig = Rig::new("let value = 1;\nnext", Language::Plain);
    rig.idle();
    rig.click(0, 6.0, 2);
    assert_eq!(rig.selection(), Selection { anchor: 4, head: 9 });
    rig.click(0, 6.0, 3);
    assert_eq!(
        rig.selection(),
        Selection {
            anchor: 0,
            head: 15
        }
    );
}

#[test]
fn dragging_grows_the_selection() {
    let mut rig = Rig::new("one two three", Language::Plain);
    rig.idle();
    rig.click(0, 0.0, 1);
    let (x, y) = rig.at(0, 7.0);
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_down: true,
        ..InputState::default()
    });
    assert_eq!(rig.selection(), Selection { anchor: 0, head: 7 });
    // Once let go, moving the pointer leaves it be.
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_released: true,
        ..InputState::default()
    });
    let (x, y) = rig.at(0, 12.0);
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        ..InputState::default()
    });
    assert_eq!(rig.selection(), Selection { anchor: 0, head: 7 });
}

#[test]
fn dragging_after_a_double_click_grows_by_words() {
    let mut rig = Rig::new("one two three", Language::Plain);
    rig.idle();
    rig.click(0, 5.0, 2);
    let (x, y) = rig.at(0, 9.0);
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_down: true,
        ..InputState::default()
    });
    assert_eq!(
        rig.selection(),
        Selection {
            anchor: 4,
            head: 13
        }
    );
    let (x, y) = rig.at(0, 1.0);
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_down: true,
        ..InputState::default()
    });
    assert_eq!(rig.selection(), Selection { anchor: 7, head: 0 });
}

#[test]
fn typing_edits_only_while_focused() {
    let mut rig = Rig::new("", Language::Plain);
    assert!(!rig.type_text("x").changed);
    assert_eq!(rig.editor.text(), "");
    let mut rig = rig.focused();
    assert!(rig.type_text("ab").changed);
    assert_eq!(rig.editor.text(), "ab");
    assert_eq!(rig.selection(), Selection::caret(2));
}

#[test]
fn typing_follows_the_languages_rules() {
    let mut rig = Rig::new("", Language::Rust).focused();
    rig.type_text("f(");
    assert_eq!(rig.editor.text(), "f()");
    rig.frame(&InputState {
        enter_pressed: true,
        ..away()
    });
    assert_eq!(rig.editor.text(), "f(\n    \n)");
}

#[test]
fn a_read_only_editor_moves_and_selects_but_does_not_edit() {
    let mut rig = Rig::with(CodeEditor::new("abc", Language::Plain).read_only(true)).focused();
    for input in [
        InputState {
            text_input: "x".into(),
            ..away()
        },
        InputState {
            backspace_pressed: true,
            ..away()
        },
        InputState {
            key_delete: true,
            ..away()
        },
        InputState {
            enter_pressed: true,
            ..away()
        },
        InputState {
            key_paste: true,
            ..away()
        },
        InputState {
            key_cut: true,
            ..away()
        },
    ] {
        assert!(!rig.frame(&input).changed);
    }
    assert_eq!(rig.editor.text(), "abc");
    rig.frame(&InputState {
        key_right: true,
        ..away()
    });
    assert_eq!(rig.selection(), Selection::caret(1));
    rig.frame(&InputState {
        key_select_all: true,
        ..away()
    });
    rig.frame(&InputState {
        key_copy: true,
        ..away()
    });
    assert_eq!(*rig.clipboard.borrow(), "abc", "copying still works");
}

#[test]
fn tab_indents_and_keeps_the_focus() {
    let mut rig = Rig::new("x", Language::Plain).focused();
    let mut tab = InputState {
        key_tab: true,
        ..away()
    };
    tab.nav.next = true;
    rig.frame(&tab);
    assert_eq!(rig.editor.text(), "    x");
    assert!(rig.focus.is_focused(ID));
}

#[test]
fn tab_moves_on_from_a_read_only_editor() {
    let mut rig = Rig::with(CodeEditor::new("x", Language::Plain).read_only(true)).focused();
    let mut tab = InputState {
        key_tab: true,
        ..away()
    };
    tab.nav.next = true;
    rig.frame(&tab);
    assert_eq!(rig.editor.text(), "x");
    assert!(rig.focus.is_focused(NEXT));
}

#[test]
fn shift_tab_outdents() {
    let mut rig = Rig::new("        x", Language::Plain).focused();
    let mut tab = InputState {
        key_tab: true,
        shift_pressed: true,
        ..away()
    };
    tab.nav.prev = true;
    rig.frame(&tab);
    assert_eq!(rig.editor.text(), "    x");
    assert!(rig.focus.is_focused(ID));
}

#[test]
fn the_caret_is_kept_in_view() {
    let mut rig = Rig::new(&numbered(500), Language::Rust).focused();
    let view = RECT.height - STATUS_HEIGHT;
    for _ in 0..100 {
        rig.frame(&InputState {
            key_down: true,
            ..away()
        });
    }
    let line_height = rig.editor.drawn.line_height;
    let caret_top = PAD_Y + 100.0 * line_height;
    let target = rig.editor.scroll.target[1];
    assert!(target > 0.0);
    assert!(
        caret_top >= target && caret_top + line_height <= target + view,
        "line 100 at {caret_top} in {target}..{}",
        target + view
    );
    // And back to the top.
    // Cmd-Up on macOS, Ctrl-Home elsewhere.
    let mac = cfg!(target_os = "macos");
    rig.frame(&InputState {
        ctrl_pressed: true,
        key_up: mac,
        key_home: !mac,
        ..away()
    });
    assert_eq!(rig.selection(), Selection::caret(0));
    assert_eq!(rig.editor.scroll.target[1], 0.0);
}

#[test]
fn page_down_moves_a_screenful() {
    let mut rig = Rig::new(&numbered(500), Language::Rust).focused();
    let mut input = away();
    input.keys.key_event(Key::PageDown, true);
    rig.frame(&input);
    let line = rig.editor.document().line_of(rig.selection().head);
    let lines = rig.editor.drawn.lines.len();
    assert!(line > 10 && line < lines, "line {line} of {lines} in view");
}

#[test]
fn up_and_down_keep_to_their_column() {
    let mut rig = Rig::new("long line\nab\nlong line", Language::Plain).focused();
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    for _ in 0..2 {
        rig.frame(&InputState {
            key_down: true,
            ..away()
        });
    }
    assert_eq!(rig.selection(), Selection::caret(13 + 9));
}

#[test]
fn the_status_strip_shows_the_line_column_and_language() {
    let mut rig = Rig::new("fn main() {\n\tx\n}", Language::Rust).focused();
    rig.click(1, 4.0, 1);
    assert!(rig.texts().contains(&"Ln 2, Col 5"), "{:?}", rig.texts());
    assert!(rig.texts().contains(&"Spaces: 4"));
    assert!(rig.texts().contains(&"Rust"));
    rig.frame(&InputState {
        key_select_all: true,
        ..away()
    });
    assert!(
        rig.texts()
            .iter()
            .any(|text| text.ends_with("(16 selected)")),
        "{:?}",
        rig.texts()
    );
}

#[test]
fn the_status_strip_can_be_left_out() {
    let mut rig = Rig::with(CodeEditor::new("x", Language::Rust).with_status(false));
    rig.idle();
    assert!(!rig.texts().contains(&"Rust"));
}

#[test]
fn code_is_coloured() {
    let mut rig = Rig::new("fn main() {}", Language::Rust);
    rig.idle();
    let syntax = SyntaxTheme::default();
    let block = rig.block("fn main() {}");
    assert_eq!(
        block.color,
        inked(syntax.variable).color,
        "the text's own ink"
    );
    let keyword = block
        .style_ranges
        .iter()
        .find(|range| range.range == (0..2))
        .expect("fn coloured");
    assert_eq!(keyword.color, Some(syntax.keyword));
    let function = block
        .style_ranges
        .iter()
        .find(|range| range.range == (3..7))
        .expect("main coloured");
    assert_eq!(function.color, Some(syntax.function));
}

#[test]
fn colours_follow_the_text_through_tabs() {
    let mut rig = Rig::new("\tlet x = 1;", Language::Rust);
    rig.idle();
    let block = rig.block("let x = 1;");
    assert_eq!(block.x, rig.at(0, 4.0).0, "after the tab's stop");
    let keyword = block
        .style_ranges
        .iter()
        .find(|range| range.color == Some(SyntaxTheme::default().keyword))
        .expect("let coloured");
    assert_eq!(keyword.range, 0..3);
}

#[test]
fn every_character_keeps_to_the_grid() {
    // A wide character takes two columns, and one drawn from another font
    // is placed at its column whatever width that font gives it.
    let mut rig = Rig::new("é日x", Language::Plain);
    rig.idle();
    assert_eq!(rig.block("é").x, rig.at(0, 0.0).0);
    assert_eq!(rig.block("日").x, rig.at(0, 1.0).0);
    assert_eq!(rig.block("x").x, rig.at(0, 3.0).0);
}

#[test]
fn a_wide_character_is_two_columns_to_the_caret_and_the_pointer() {
    let mut rig = Rig::new("日x", Language::Plain).focused();
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    let caret = rig.editor.drawn.caret.expect("a caret");
    assert!((caret.x - rig.at(0, 3.0).0).abs() < 0.01);
    rig.click(0, 1.2, 1);
    assert_eq!(rig.selection(), Selection::caret(3), "nearer its end");
    rig.click(0, 0.8, 1);
    assert_eq!(rig.selection(), Selection::caret(0), "nearer its start");
    rig.frame(&InputState {
        key_select_all: true,
        ..away()
    });
    let selected = rig.editor.drawn.selection[0];
    assert!((selected.width - 3.0 * rig.editor.drawn.advance).abs() < 0.01);
}

#[test]
fn colours_follow_the_text_past_a_wide_character() {
    let mut rig = Rig::new("\"日\"; fn", Language::Rust);
    rig.idle();
    let string = SyntaxTheme::default().string;
    let wide = rig.block("日").style_ranges.first().expect("coloured");
    assert_eq!((wide.range.clone(), wide.color), (0..3, Some(string)));
    let block = rig.block("\"; fn");
    let keyword = block
        .style_ranges
        .iter()
        .find(|range| range.color == Some(SyntaxTheme::default().keyword))
        .expect("fn coloured");
    assert_eq!(keyword.range, 3..5);
}

#[test]
fn an_edit_is_coloured_at_once() {
    let mut rig = Rig::new("", Language::Rust).focused();
    rig.type_text("fn");
    let block = rig.block("fn");
    assert_eq!(
        block.style_ranges.first().and_then(|range| range.color),
        Some(SyntaxTheme::default().keyword)
    );
}

#[test]
fn control_characters_are_drawn_as_pictures() {
    let mut rig = Rig::new("a\u{0}b", Language::Plain);
    rig.idle();
    assert_eq!(rig.block("\u{2400}").x, rig.at(0, 1.0).0);
    assert_eq!(rig.block("b").x, rig.at(0, 2.0).0);
}

#[test]
fn the_caret_line_is_banded_while_focused() {
    let mut rig = Rig::new("a\nb", Language::Plain);
    rig.idle();
    assert!(rig.editor.drawn.band.is_none());
    assert!(rig.editor.drawn.caret.is_none());
    let mut rig = rig.focused();
    let band = rig.editor.drawn.band.expect("banded");
    let caret = rig.editor.drawn.caret.expect("a caret");
    assert_eq!(band.y, caret.y);
    assert_eq!(
        rig.block("1").color,
        inked(rig.editor.theme.number_current).color
    );
    assert_eq!(rig.block("2").color, inked(rig.editor.theme.number).color);
    // A selection, no band.
    rig.frame(&InputState {
        key_select_all: true,
        ..away()
    });
    assert!(rig.editor.drawn.band.is_none());
}

#[test]
fn a_selection_is_drawn_on_each_of_its_lines() {
    let mut rig = Rig::new("one\ntwo\nthree", Language::Plain).focused();
    rig.frame(&InputState {
        key_select_all: true,
        ..away()
    });
    let rects = &rig.editor.drawn.selection;
    assert_eq!(rects.len(), 3);
    let advance = rig.editor.drawn.advance;
    // The first two run past their text, over the line ending.
    assert!(rects[0].width > 3.0 * advance);
    assert!((rects[2].width - 5.0 * advance).abs() < 0.01);
}

#[test]
fn the_caret_follows_the_drawn_text() {
    let mut rig = Rig::new("\tx", Language::Plain).focused();
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    let drawn = &rig.editor.drawn;
    let caret = drawn.caret.expect("a caret");
    assert!((caret.x - (drawn.origin[0] + 5.0 * drawn.advance)).abs() < 0.01);
}

#[test]
fn copy_cut_and_paste_go_through_the_clipboard() {
    let mut rig = Rig::new("one two", Language::Plain).focused();
    rig.click(0, 5.0, 2);
    rig.frame(&InputState {
        key_cut: true,
        ..away()
    });
    assert_eq!(*rig.clipboard.borrow(), "two");
    assert_eq!(rig.editor.text(), "one ");
    *rig.clipboard.borrow_mut() = "three\r\nfour".into();
    rig.frame(&InputState {
        key_paste: true,
        ..away()
    });
    assert_eq!(rig.editor.text(), "one three\nfour");
}

#[test]
fn cut_keeps_the_text_when_there_is_no_clipboard_to_take_it() {
    let mut rig = Rig::new("one two", Language::Plain).focused();
    rig.editor.clipboard_set = None;
    rig.click(0, 5.0, 2);
    assert!(
        !rig.frame(&InputState {
            key_cut: true,
            ..away()
        })
        .changed
    );
    assert_eq!(rig.editor.text(), "one two");
}

#[test]
fn colours_follow_a_new_theme() {
    let mut rig = Rig::new("fn main() {}", Language::Rust);
    rig.idle();
    let keyword = [1.0, 0.0, 0.0, 1.0];
    rig.editor.theme.syntax.keyword = keyword;
    rig.idle();
    let block = rig.block("fn main() {}");
    let fn_range = block
        .style_ranges
        .iter()
        .find(|range| range.range == (0..2))
        .expect("fn coloured");
    assert_eq!(fn_range.color, Some(keyword));
}

#[test]
fn undo_and_redo() {
    let mut rig = Rig::new("a", Language::Plain).focused();
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    rig.type_text("bc");
    assert!(rig.shortcut('z', false).changed);
    assert_eq!(rig.editor.text(), "a");
    assert!(rig.shortcut('z', true).changed);
    assert_eq!(rig.editor.text(), "abc");
}

#[test]
fn a_shortcut_is_not_typed() {
    let mut rig = Rig::new("", Language::Plain).focused();
    let mut input = InputState {
        ctrl_pressed: true,
        text_input: "s".into(),
        ..away()
    };
    input.keys.key_event(Key::Char('s'), true);
    assert!(!rig.frame(&input).changed);
}

#[test]
fn a_line_runs_on_past_the_view_rather_than_wrapping() {
    let line = "word ".repeat(300);
    let mut rig = Rig::new(&format!("{line}\nnext"), Language::Plain);
    rig.idle();
    let first = rig
        .list
        .texts
        .iter()
        .find(|text| text.content.starts_with("word word"))
        .expect("the line drawn")
        .clone();
    let (_, height) = rig.list.measure_block(&first);
    assert!(height < 2.0 * first.line_height, "{height}: one line");
    assert_eq!(rig.block("next").y - first.y, first.line_height);
}

#[test]
fn a_long_line_is_drawn_only_where_it_can_be_seen() {
    let line = "a".repeat(100_000);
    let mut rig = Rig::new(&line, Language::Plain);
    rig.idle();
    let longest = rig
        .list
        .texts
        .iter()
        .map(|text| text.content.len())
        .max()
        .unwrap();
    assert!(longest < 1_000, "{longest} bytes drawn");
    // It still scrolls the whole way.
    let advance = rig.editor.drawn.advance;
    assert!(rig.editor.scroll.content_size[0] > 100_000.0 * advance);
}

#[test]
fn the_end_of_a_long_line_is_drawn_when_scrolled_to() {
    let line = format!("{}END", "a".repeat(100_000));
    let mut rig = Rig::new(&line, Language::Plain).focused();
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    // Let the scroll glide finish.
    let target = rig.editor.scroll.target;
    rig.editor.scroll.offset = target;
    rig.idle();
    assert!(
        rig.list
            .texts
            .iter()
            .any(|text| text.content.ends_with("END")),
        "the end in view"
    );
}

#[test]
fn the_ime_is_asked_for_only_while_editable() {
    let rig = Rig::new("x", Language::Plain).focused();
    assert!(rig.focus.ime_request().is_some());
    let rig = Rig::with(CodeEditor::new("x", Language::Plain).read_only(true)).focused();
    assert!(rig.focus.ime_request().is_none());
}
