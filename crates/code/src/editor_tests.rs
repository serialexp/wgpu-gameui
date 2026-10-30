use std::cell::RefCell;
use std::rc::Rc;

use wgpu_gameui::{CursorState, FocusState, Theme};

use super::*;
use crate::CompletionKind;

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
    cursor: CursorState,
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
            cursor: CursorState::new(),
            clipboard,
        }
    }

    /// Draw a frame under `input`.
    fn frame(&mut self, input: &InputState) -> CodeEditorOutput {
        self.frame_in(RECT, input)
    }

    /// A frame with the editor in `rect`.
    fn frame_in(&mut self, rect: Rect, input: &InputState) -> CodeEditorOutput {
        self.list = DrawList::new();
        self.focus.begin_frame(input);
        self.cursor.begin_frame();
        let mut cx = DrawContext::new(
            &mut self.list,
            &mut self.focus,
            &self.theme,
            input,
            800.0,
            600.0,
        )
        .with_cursor(&mut self.cursor);
        let output = self.editor.draw(ID, rect, &mut cx);
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

    /// A frame with `key` pressed and `modifiers` held.
    fn press(&mut self, key: Key, modifiers: impl FnOnce(&mut InputState)) -> CodeEditorOutput {
        let mut input = away();
        modifiers(&mut input);
        input.keys.key_event(key, true);
        self.frame(&input)
    }

    /// A frame holding the button down at `x`, `y`, `dt` seconds after the
    /// last.
    fn hold(&mut self, x: f32, y: f32, dt: f32) {
        self.frame(&InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            frame_dt: dt,
            ..InputState::default()
        });
    }

    /// The width it scrolls across.
    fn content_width(&self) -> f32 {
        self.editor.scroll.content_size[0]
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

#[test]
fn deleting_a_long_line_gives_its_width_back() {
    let long = "x".repeat(500);
    let mut rig = Rig::new(&format!("short\n{long}"), Language::Plain).focused();
    let advance = rig.editor.drawn.advance;
    assert!(rig.content_width() > 500.0 * advance);
    rig.click(1, 3.0, 3);
    rig.frame(&InputState {
        backspace_pressed: true,
        ..away()
    });
    rig.idle();
    assert!(
        rig.content_width() < 100.0 * advance,
        "{} after the edit",
        rig.content_width()
    );
}

#[test]
fn scrolling_past_a_long_line_keeps_its_width() {
    let text = format!("{}\n{}", "x".repeat(500), numbered(1_000));
    let mut rig = Rig::new(&text, Language::Plain);
    rig.idle();
    let width = rig.content_width();
    rig.editor.scroll.snap_to(1, 5_000.0);
    rig.idle();
    assert!(!rig.editor.drawn.lines.contains(&0), "scrolled away");
    assert_eq!(rig.content_width(), width);
}

#[test]
fn dragging_past_an_edge_scrolls_towards_it() {
    let long = "x".repeat(500);
    let text = format!("{long}\n{}", numbered(1_000));
    let mut rig = Rig::new(&text, Language::Plain);
    rig.idle();
    rig.click(0, 1.0, 1);
    rig.hold(RECT.width + 30.0, rig.at(0, 0.0).1, 0.1);
    assert!(rig.editor.scroll.target[0] > 0.0, "sideways");
    assert_eq!(rig.editor.scroll.target[1], 0.0);
    rig.hold(rig.at(0, 1.0).0, RECT.height + 30.0, 0.1);
    assert!(rig.editor.scroll.target[1] > 0.0, "down");
}

#[test]
fn a_drag_scrolls_by_the_time_held_not_the_frames_drawn() {
    let scrolled = |frames: usize| {
        let mut rig = Rig::new(&numbered(1_000), Language::Plain);
        rig.idle();
        rig.click(0, 1.0, 1);
        let x = rig.at(0, 1.0).0;
        for _ in 0..frames {
            rig.hold(x, RECT.height + 30.0, 0.2 / frames as f32);
        }
        rig.editor.scroll.target[1]
    };
    let (slow, fast) = (scrolled(2), scrolled(4));
    assert!(slow > 0.0);
    assert!(
        (slow - fast).abs() < 0.01,
        "{slow} at 10 Hz, {fast} at 20 Hz"
    );
}

#[test]
fn page_down_scrolls_the_view_with_the_caret() {
    let mut rig = Rig::new(&numbered(500), Language::Rust).focused();
    rig.press(Key::PageDown, |_| {});
    let line = rig.editor.document().line_of(rig.selection().head);
    let height = rig.editor.drawn.line_height;
    let target = rig.editor.scroll.target[1];
    assert!(
        (target - line as f32 * height).abs() < 0.01,
        "{target} for line {line}"
    );
    rig.press(Key::PageUp, |_| {});
    assert_eq!(rig.editor.scroll.target[1], 0.0);
    assert_eq!(rig.selection(), Selection::caret(0));
}

#[test]
fn a_click_on_a_half_hidden_line_brings_it_into_view() {
    let mut rig = Rig::new(&numbered(1_000), Language::Plain);
    rig.idle();
    // Just above the bottom of the view, on a line it cuts through.
    let (x, _) = rig.at(0, 1.0);
    let y = RECT.height - STATUS_HEIGHT - 2.0;
    let height = rig.editor.drawn.line_height;
    let line = ((y - rig.editor.drawn.origin[1]) / height).floor();
    assert!(
        rig.editor.drawn.origin[1] + (line + 1.0) * height > y + 2.0,
        "cut"
    );
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_down: true,
        mouse_clicked: true,
        mouse_click_count: 1,
        ..InputState::default()
    });
    rig.idle();
    assert!(rig.editor.scroll.target[1] > 0.0);
}

#[test]
fn no_line_below_the_view_is_drawn() {
    let mut rig = Rig::new(&numbered(1_000), Language::Plain);
    rig.idle();
    let bottom = RECT.height - STATUS_HEIGHT;
    let drawn = &rig.editor.drawn;
    let last = drawn.lines.end - 1;
    let top = drawn.origin[1] + last as f32 * drawn.line_height;
    assert!(top < bottom, "line {last} starts at {top}, below {bottom}");
    assert!(top + drawn.line_height >= bottom, "and reaches the bottom");
}

/// Holds the key that moves and deletes by words: Option on macOS, Ctrl
/// elsewhere.
fn by_words(input: &mut InputState) {
    if cfg!(target_os = "macos") {
        input.alt_down = true;
    } else {
        input.ctrl_pressed = true;
    }
}

#[test]
fn a_word_is_deleted_either_side_of_the_caret() {
    let mut rig = Rig::new("let value = one;", Language::Plain).focused();
    rig.click(0, 9.0, 1);
    let mut input = away();
    by_words(&mut input);
    input.backspace_pressed = true;
    assert!(rig.frame(&input).changed);
    assert_eq!(rig.editor.text(), "let  = one;");
    let mut input = away();
    by_words(&mut input);
    input.key_delete = true;
    rig.frame(&input);
    assert_eq!(rig.editor.text(), "let  one;");
    // A selection goes whole, as with a plain Backspace.
    rig.click(0, 0.0, 2);
    let mut input = away();
    by_words(&mut input);
    input.backspace_pressed = true;
    rig.frame(&input);
    assert_eq!(rig.editor.text(), "  one;");
}

#[cfg(target_os = "macos")]
#[test]
fn cmd_backspace_deletes_to_the_start_of_the_line() {
    let mut rig = Rig::new("one\n    two three", Language::Plain).focused();
    rig.click(1, 7.0, 1);
    rig.frame(&InputState {
        ctrl_pressed: true,
        backspace_pressed: true,
        ..away()
    });
    assert_eq!(rig.editor.text(), "one\n three");
}

#[test]
fn the_pointer_is_a_text_cursor_over_the_code_and_not_the_numbers() {
    let mut rig = Rig::new("one", Language::Plain);
    rig.idle();
    let (x, y) = rig.at(0, 1.0);
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        ..InputState::default()
    });
    assert_eq!(rig.cursor.resolve(), CursorIcon::Text);
    rig.frame(&InputState {
        mouse_x: RECT.x + 4.0,
        mouse_y: y,
        ..InputState::default()
    });
    assert_eq!(rig.cursor.resolve(), CursorIcon::Default);
}

fn problem(range: Range<usize>, severity: Severity, message: &str) -> Diagnostic {
    Diagnostic::new(range, severity, message)
}

/// A frame with the pointer resting at `column` of `line`.
fn rest(rig: &mut Rig, line: usize, column: f32) {
    let (x, y) = rig.at(line, column);
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        ..InputState::default()
    });
}

#[test]
fn a_problem_is_underlined_in_its_ink_across_its_columns() {
    let mut rig = Rig::new("let x = plyer;\nfine\n", Language::Rust);
    rig.editor
        .set_diagnostics(vec![problem(8..13, Severity::Error, "unknown")]);
    rig.idle();
    let drawn = &rig.editor.drawn;
    assert_eq!(drawn.waves.len(), 1);
    let (line, across, severity) = drawn.waves[0].clone();
    assert_eq!((line, severity), (0, Severity::Error));
    assert_eq!(across.start, drawn.origin[0] + 8.0 * drawn.advance);
    assert_eq!(across.end, drawn.origin[0] + 13.0 * drawn.advance);
    assert_eq!(drawn.washes, vec![(0, Severity::Error)]);
    assert_eq!(drawn.dots, vec![0]);
}

#[test]
fn a_note_gets_a_dot_and_an_underline_but_no_wash() {
    let mut rig = Rig::new("value\n", Language::Plain);
    rig.editor
        .set_diagnostics(vec![problem(0..5, Severity::Info, "unused")]);
    rig.idle();
    assert_eq!(rig.editor.drawn.waves.len(), 1);
    assert!(rig.editor.drawn.washes.is_empty());
    assert_eq!(rig.editor.drawn.dots, vec![0]);
}

#[test]
fn a_line_with_several_is_washed_for_the_worst() {
    let mut rig = Rig::new("aa bb\n", Language::Plain);
    rig.editor.set_diagnostics(vec![
        problem(0..2, Severity::Warning, "w"),
        problem(3..5, Severity::Error, "e"),
    ]);
    rig.idle();
    assert_eq!(rig.editor.drawn.waves.len(), 2);
    assert_eq!(rig.editor.drawn.washes, vec![(0, Severity::Error)]);
}

#[test]
fn the_caret_lines_band_gives_way_to_a_problems_wash() {
    let mut rig = Rig::new("bad\ngood\n", Language::Plain).focused();
    rig.editor
        .set_diagnostics(vec![problem(0..3, Severity::Error, "e")]);
    rig.idle();
    assert!(
        rig.editor.drawn.band.is_none(),
        "the caret is on the washed line"
    );
    rig.frame(&InputState {
        key_down: true,
        ..away()
    });
    assert!(rig.editor.drawn.band.is_some());
}

#[test]
fn a_problem_follows_the_text_typed_before_it() {
    let mut rig = Rig::new("x = plyer\n", Language::Plain).focused();
    rig.editor
        .set_diagnostics(vec![problem(4..9, Severity::Error, "unknown")]);
    rig.type_text("let ");
    let range = rig.editor.diagnostics().all()[0].range.clone();
    assert_eq!(rig.editor.document().slice(range), "plyer");
    let drawn = &rig.editor.drawn;
    assert_eq!(
        drawn.waves[0].1.start,
        drawn.origin[0] + 8.0 * drawn.advance
    );
}

#[test]
fn problems_set_after_unseen_edits_are_where_the_text_is_now() {
    let mut rig = Rig::new("x = plyer\n", Language::Plain).focused();
    rig.type_text("let ");
    // Set before the next draw has told anything of the typing: the ranges
    // are the text's as it is now, and mustn't be moved again by it.
    rig.editor
        .set_diagnostics(vec![problem(8..13, Severity::Error, "unknown")]);
    rig.idle();
    let range = rig.editor.diagnostics().all()[0].range.clone();
    assert_eq!(rig.editor.document().slice(range), "plyer");
}

#[test]
fn an_empty_problem_at_a_lines_end_is_drawn_as_one_column() {
    let mut rig = Rig::new("call(\n", Language::Plain);
    rig.editor
        .set_diagnostics(vec![problem(5..5, Severity::Error, "expected `)`")]);
    rig.idle();
    let drawn = &rig.editor.drawn;
    let across = drawn.waves[0].1.clone();
    assert_eq!(across.start, drawn.origin[0] + 5.0 * drawn.advance);
    assert_eq!(across.end, across.start + drawn.advance);
}

#[test]
fn a_problem_over_lines_is_underlined_on_each() {
    let mut rig = Rig::new("one\ntwo\nthree\n", Language::Plain);
    rig.editor
        .set_diagnostics(vec![problem(1..10, Severity::Warning, "w")]);
    rig.idle();
    let lines: Vec<usize> = rig.editor.drawn.waves.iter().map(|w| w.0).collect();
    assert_eq!(lines, vec![0, 1, 2]);
}

#[test]
fn only_the_problems_in_view_are_drawn() {
    // Representative scale: one on each of 10,000 lines.
    let text = numbered(10_000);
    let mut rig = Rig::new(&text, Language::Rust);
    let document = Document::new(&text);
    let list = (0..10_000)
        .map(|line| {
            let start = document.line_start(line) + 4;
            problem(start..start + 2, Severity::Warning, "w")
        })
        .collect();
    rig.editor.set_diagnostics(list);
    rig.idle();
    let lines = rig.editor.drawn.lines.clone();
    assert_eq!(rig.editor.drawn.waves.len(), lines.len());
    assert_eq!(rig.editor.diagnostics().count(Severity::Warning), 10_000);
}

#[test]
fn resting_on_an_underline_shows_its_message_above_everything() {
    let mut rig = Rig::new("let x = plyer;\n", Language::Rust);
    rig.editor.set_diagnostics(vec![
        problem(8..13, Severity::Error, "cannot find value `plyer`").with_code("E0425"),
    ]);
    rig.idle();
    rest(&mut rig, 0, 10.5);
    let hover = rig.editor.hover.clone().expect("a message to show");
    assert_eq!(hover.diagnostic.message, "cannot find value `plyer`");
    let mut layers = LayerStack::new();
    let styles = StyleResolver::new(&rig.theme);
    rig.editor
        .draw_layers(&mut layers, &styles, Rect::new(0.0, 0.0, 800.0, 600.0));
    let texts: Vec<&str> = layers.layers()[0]
        .list
        .texts
        .iter()
        .map(|t| t.content.as_str())
        .collect();
    assert_eq!(texts, vec!["ERROR E0425", "cannot find value `plyer`"]);
    // Below the line.
    assert!(layers.layers()[0].rect.y >= hover.anchor.bottom());
}

#[test]
fn off_the_underline_nothing_shows() {
    let mut rig = Rig::new("let x = plyer;\n", Language::Rust);
    rig.editor
        .set_diagnostics(vec![problem(8..13, Severity::Error, "e")]);
    rig.idle();
    rest(&mut rig, 0, 2.5);
    assert!(rig.editor.hover.is_none());
    let mut layers = LayerStack::new();
    let styles = StyleResolver::new(&rig.theme);
    rig.editor
        .draw_layers(&mut layers, &styles, Rect::new(0.0, 0.0, 800.0, 600.0));
    assert!(layers.layers().is_empty());
}

#[test]
fn resting_on_the_gutter_dot_shows_the_lines_worst() {
    let mut rig = Rig::new("aa bb\n", Language::Plain);
    rig.editor.set_diagnostics(vec![
        problem(0..2, Severity::Warning, "a warning"),
        problem(3..5, Severity::Error, "an error"),
    ]);
    rig.idle();
    let (_, y) = rig.at(0, 0.0);
    let x = RECT.x + DOT_X;
    rig.frame(&InputState {
        mouse_x: x,
        mouse_y: y,
        ..InputState::default()
    });
    let hover = rig.editor.hover.clone().expect("the gutter's message");
    assert_eq!(hover.diagnostic.message, "an error");
}

#[test]
fn the_status_strip_counts_errors_and_warnings() {
    let mut rig = Rig::new("aa bb cc\n", Language::Plain);
    rig.editor.set_diagnostics(vec![
        problem(0..2, Severity::Error, "e"),
        problem(3..5, Severity::Error, "e"),
        problem(6..8, Severity::Warning, "w"),
    ]);
    rig.idle();
    let counts = |rig: &Rig, severity| {
        let rect = rig
            .editor
            .drawn
            .counts
            .iter()
            .find(|c| c.0 == severity)
            .unwrap()
            .1;
        rig.list
            .texts
            .iter()
            .find(|t| rect.contains(t.x + 0.5, t.y + 1.0))
            .map(|t| t.content.clone())
    };
    assert_eq!(counts(&rig, Severity::Error).as_deref(), Some("2"));
    assert_eq!(counts(&rig, Severity::Warning).as_deref(), Some("1"));
}

#[test]
fn a_click_on_a_count_goes_to_the_next_of_its_kind() {
    let mut rig = Rig::new("aa bb cc dd\n", Language::Plain);
    rig.editor.set_diagnostics(vec![
        problem(0..2, Severity::Error, "e"),
        problem(3..5, Severity::Warning, "w"),
        problem(6..8, Severity::Error, "e"),
    ]);
    rig.idle();
    let errors = rig.editor.drawn.counts[0];
    assert_eq!(errors.0, Severity::Error);
    let click = |rig: &mut Rig, rect: Rect| {
        rig.frame(&InputState {
            mouse_x: rect.x + 2.0,
            mouse_y: rect.y + rect.height / 2.0,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: 1,
            ..InputState::default()
        });
    };
    click(&mut rig, errors.1);
    assert_eq!(rig.selection(), Selection::caret(6));
    assert!(rig.focus.is_focused(ID), "the editor takes focus");
    click(&mut rig, errors.1);
    assert_eq!(rig.selection(), Selection::caret(0), "wraps round");
    let warnings = rig.editor.drawn.counts[1].1;
    click(&mut rig, warnings);
    assert_eq!(rig.selection(), Selection::caret(3));
}

fn offered(rig: &Rig) -> Vec<String> {
    rig.editor
        .completing
        .as_ref()
        .map(|open| open.items.iter().map(|c| c.label.clone()).collect())
        .unwrap_or_default()
}

/// The editor holding `text`, focused, the caret at its end.
fn completing_rig(text: &str, language: Language) -> Rig {
    let mut rig = Rig::new(text, language).focused();
    // Cmd-Down on macOS, Ctrl-End elsewhere.
    let mac = cfg!(target_os = "macos");
    rig.frame(&InputState {
        ctrl_pressed: true,
        key_down: mac,
        key_end: !mac,
        ..away()
    });
    rig
}

/// A frame pressing Ctrl-Space (Control itself on macOS).
fn ctrl_space(rig: &mut Rig) -> CodeEditorOutput {
    let mut input = InputState {
        ctrl_pressed: !cfg!(target_os = "macos"),
        control_down: true,
        key_space: true,
        ..away()
    };
    input.keys.key_event(Key::Space, true);
    rig.frame(&input)
}

#[test]
fn typing_a_word_offers_the_documents_words_and_keywords() {
    let mut rig = completing_rig("let player = 1;\nlet plan = 2;\n", Language::Rust);
    rig.type_text("p");
    assert_eq!(offered(&rig), vec!["pub", "player", "plan"]);
    rig.type_text("l");
    assert_eq!(offered(&rig), vec!["player", "plan"]);
}

#[test]
fn a_word_nothing_matches_offers_nothing() {
    let mut rig = completing_rig("let player = 1;\n", Language::Plain);
    rig.type_text("zq");
    assert!(!rig.editor.is_completing());
}

#[test]
fn up_and_down_choose_and_enter_accepts() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    rig.frame(&InputState {
        key_down: true,
        ..away()
    });
    assert_eq!(rig.editor.completing.as_ref().unwrap().chosen, 1);
    let before = rig.selection();
    assert_eq!(before, rig.selection(), "the caret stays put");
    let output = rig.frame(&InputState {
        enter_pressed: true,
        ..away()
    });
    assert!(output.changed);
    assert_eq!(rig.editor.text(), "player plan\nplan");
    assert!(!rig.editor.is_completing());
}

#[test]
fn tab_accepts_and_does_not_move_focus() {
    let mut rig = completing_rig("player\n", Language::Plain);
    rig.type_text("pl");
    rig.frame(&InputState {
        key_tab: true,
        ..away()
    });
    assert_eq!(rig.editor.text(), "player\nplayer");
    assert!(rig.focus.is_focused(ID));
}

#[test]
fn escape_closes_the_list_and_keeps_focus() {
    let mut rig = completing_rig("player\n", Language::Plain);
    rig.type_text("pl");
    let mut input = InputState {
        key_escape: true,
        ..away()
    };
    input.nav.cancel = true;
    rig.frame(&input);
    assert!(!rig.editor.is_completing());
    assert!(rig.focus.is_focused(ID), "the Escape was the list's");
}

#[test]
fn moving_the_caret_or_typing_past_the_word_closes_it() {
    let mut rig = completing_rig("player\n", Language::Plain);
    rig.type_text("pl");
    rig.frame(&InputState {
        key_left: true,
        ..away()
    });
    assert!(!rig.editor.is_completing());
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    rig.type_text("a");
    assert!(rig.editor.is_completing());
    rig.type_text(" ");
    assert!(!rig.editor.is_completing());
}

#[test]
fn backspace_widens_the_match_and_closes_it_with_the_word_gone() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pla");
    assert_eq!(offered(&rig), vec!["player", "plan"]);
    rig.frame(&InputState {
        backspace_pressed: true,
        ..away()
    });
    assert_eq!(offered(&rig), vec!["player", "plan"]);
    for _ in 0..2 {
        rig.frame(&InputState {
            backspace_pressed: true,
            ..away()
        });
    }
    assert!(!rig.editor.is_completing());
}

#[test]
fn ctrl_space_opens_it_with_no_word_and_tells_the_host() {
    let mut rig = completing_rig("player\n", Language::Plain);
    let output = ctrl_space(&mut rig);
    assert!(output.completion_asked);
    assert_eq!(offered(&rig), vec!["player"]);
    assert!(!rig.editor.text().ends_with("  "), "no space typed");
}

#[test]
fn the_hosts_items_come_first_and_reach_an_open_list() {
    let mut rig = completing_rig("speed\n", Language::Plain);
    rig.type_text("sp");
    assert_eq!(offered(&rig), vec!["speed"]);
    rig.editor.set_completions(vec![
        Completion::new("spawn", CompletionKind::Function).with_detail("(kind, x, y)"),
    ]);
    assert_eq!(offered(&rig), vec!["spawn", "speed"]);
    rig.frame(&InputState {
        enter_pressed: true,
        ..away()
    });
    assert_eq!(rig.editor.text(), "speed\nspawn()");
    assert_eq!(
        rig.selection().head,
        "speed\nspawn(".len(),
        "the caret between"
    );
}

#[test]
fn the_list_is_drawn_under_the_word_above_everything() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    let popup = rig.editor.popup.clone().expect("laid out");
    let (_, y) = rig.at(1, 0.0);
    assert!(popup.rect.y >= y - rig.editor.drawn.line_height / 2.0 + rig.editor.drawn.line_height);
    let mut layers = LayerStack::new();
    let styles = StyleResolver::new(&rig.theme);
    rig.editor
        .draw_layers(&mut layers, &styles, Rect::new(0.0, 0.0, 800.0, 600.0));
    let texts: Vec<&str> = layers.layers()[0]
        .list
        .texts
        .iter()
        .map(|t| t.content.as_str())
        .collect();
    assert_eq!(texts, vec!["v", "pl", "ayer", "v", "pl", "an"]);
}

#[test]
fn a_click_on_a_row_accepts_it_and_not_the_code_under_it() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    let row = rig.editor.popup.clone().unwrap().rows[1];
    let caret = rig.selection();
    rig.frame(&InputState {
        mouse_x: row.x + 4.0,
        mouse_y: row.y + 4.0,
        mouse_down: true,
        mouse_clicked: true,
        mouse_click_count: 1,
        ..InputState::default()
    });
    assert_eq!(rig.editor.text(), "player plan\nplan");
    assert_eq!(
        rig.selection().head,
        caret.head + 2,
        "the caret after the word, not the click"
    );
    assert!(rig.focus.is_focused(ID));
}

#[test]
fn resting_on_a_row_chooses_it() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    let row = rig.editor.popup.clone().unwrap().rows[1];
    rig.frame(&InputState {
        mouse_x: row.x + 4.0,
        mouse_y: row.y + 4.0,
        ..InputState::default()
    });
    assert_eq!(rig.editor.completing.as_ref().unwrap().chosen, 1);
}

#[test]
fn a_click_in_the_code_closes_it() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    rig.click(0, 1.0, 1);
    assert!(!rig.editor.is_completing());
}

#[test]
fn losing_focus_closes_it() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    rig.focus.focus(NEXT);
    rig.idle();
    assert!(!rig.editor.is_completing());
}

#[test]
fn a_read_only_editor_never_completes() {
    let mut rig = Rig::with(CodeEditor::new("player\n", Language::Plain).read_only(true)).focused();
    ctrl_space(&mut rig);
    assert!(!rig.editor.is_completing());
}

#[test]
fn opening_reads_the_document_once_however_long_it_is_typed_into() {
    // Representative scale: 30,000 lines. The words are gathered as the list
    // opens; narrowing it matches what was gathered.
    let text: String = (0..30_000)
        .map(|n| format!("let value{n} = {n};\n"))
        .collect();
    let mut rig = completing_rig(&text, Language::Rust);
    rig.type_text("v");
    let pool = rig.editor.completing.as_ref().unwrap().pool.len();
    assert!(pool > 30_000, "{pool}");
    rig.type_text("alue2999");
    assert_eq!(
        rig.editor.completing.as_ref().unwrap().pool.len(),
        pool,
        "not gathered again"
    );
    let expected: Vec<String> = (29_990..29_998).map(|n| format!("value{n}")).collect();
    assert_eq!(offered(&rig), expected, "the word typed itself left out");
}

#[test]
fn a_jump_to_a_problem_closes_the_list_for_the_word_left_behind() {
    let mut rig = completing_rig("player\n", Language::Plain);
    rig.type_text("pl");
    rig.type_text("\nbad");
    rig.frame(&InputState {
        key_up: true,
        ..away()
    });
    rig.frame(&InputState {
        key_end: true,
        ..away()
    });
    rig.frame(&InputState {
        backspace_pressed: true,
        ..away()
    });
    rig.type_text("l");
    assert!(rig.editor.is_completing());
    rig.editor
        .set_diagnostics(vec![problem(10..13, Severity::Error, "bad")]);
    rig.idle();
    let errors = rig.editor.drawn.counts[0].1;
    rig.frame(&InputState {
        mouse_x: errors.x + 2.0,
        mouse_y: errors.y + errors.height / 2.0,
        mouse_down: true,
        mouse_clicked: true,
        mouse_click_count: 1,
        ..InputState::default()
    });
    assert!(!rig.editor.is_completing());
    rig.frame(&InputState {
        enter_pressed: true,
        ..away()
    });
    assert_eq!(
        rig.editor.text(),
        "player\npl\n\nbad",
        "Enter was a newline"
    );
}

#[test]
fn turning_read_only_closes_the_list() {
    let mut rig = completing_rig("player\n", Language::Plain);
    rig.type_text("pl");
    let row = rig.editor.popup.clone().unwrap().rows[0];
    rig.editor.set_read_only(true);
    assert!(!rig.editor.is_completing());
    rig.frame(&InputState {
        mouse_x: row.x + 4.0,
        mouse_y: row.y + 4.0,
        mouse_down: true,
        mouse_clicked: true,
        mouse_click_count: 1,
        ..InputState::default()
    });
    rig.frame(&InputState {
        enter_pressed: true,
        ..away()
    });
    assert_eq!(rig.editor.text(), "player\npl");
}

#[test]
fn a_short_editor_keeps_the_list_inside_it() {
    let text: String = (0..8).map(|n| format!("item{n} ")).collect();
    let mut rig = completing_rig(&text, Language::Plain);
    let short = Rect::new(0.0, 0.0, 600.0, 90.0);
    rig.frame_in(
        short,
        &InputState {
            text_input: "it".into(),
            ..away()
        },
    );
    let popup = rig.editor.popup.clone().unwrap();
    assert!(
        popup.rect.y >= 0.0 && popup.rect.bottom() <= short.height,
        "{popup:?}"
    );
    assert!(popup.rows.len() < 8);
    // Down to the last item scrolls the list to it.
    for _ in 0..7 {
        rig.frame_in(
            short,
            &InputState {
                key_down: true,
                ..away()
            },
        );
    }
    let popup = rig.editor.popup.clone().unwrap();
    let last = *popup.rows.last().unwrap();
    assert_eq!(popup.row_at(last.x + 1.0, last.y + 1.0), Some(7));
}

#[test]
fn a_click_where_the_list_was_before_a_resize_is_not_the_lists() {
    let mut rig = completing_rig("player plan\n", Language::Plain);
    rig.type_text("pl");
    let row = rig.editor.popup.clone().unwrap().rows[1];
    let moved = Rect::new(RECT.x, RECT.y + 40.0, RECT.width, RECT.height - 40.0);
    rig.frame_in(
        moved,
        &InputState {
            mouse_x: row.x + 4.0,
            mouse_y: row.y + 4.0,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: 1,
            ..InputState::default()
        },
    );
    assert_eq!(rig.editor.text(), "player plan\npl", "nothing accepted");
    assert!(!rig.editor.is_completing(), "the click went to the code");
}
