//! Where a field puts its caret, its IME rect and a click, checked against the
//! glyphs it draws (`docs/design/shared-text-layout.md`, Phase 0). Phase 1
//! changes where the caret's positions come from; these say what it must
//! keep doing.

use super::*;
use crate::shaping::LayoutSpec;
use crate::{DrawList, FocusState, InputState, Theme};

const ID: FocusId = 7;
/// How far a caret may sit from the edge it belongs at, in px.
const NEAR: f32 = 0.5;

/// A frame of `ti`, focused, under `input`: the list it drew into.
fn frame(ti: &mut TextInput, focus: &mut FocusState, input: &InputState) -> DrawList {
    let mut list = DrawList::new();
    let theme = Theme::default();
    focus.focus(ID);
    focus.begin_frame(input);
    {
        let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
        ti.draw(ID, &mut ctx);
    }
    list
}

fn idle(ti: &mut TextInput, focus: &mut FocusState) -> DrawList {
    frame(ti, focus, &InputState::default())
}

/// The IME rect's left edge and top with the caret at `byte`.
fn ime_at(ti: &mut TextInput, focus: &mut FocusState, byte: usize) -> (f32, f32) {
    ti.cursor_pos = byte;
    ti.selection_start = None;
    idle(ti, focus);
    let rect = focus
        .ime_request()
        .expect("a focused field asks for the IME");
    (rect.x, rect.y)
}

/// A glyph of the text a frame drew: its first byte, and its cell's left and
/// right edges on screen.
#[derive(Clone, Copy, Debug)]
struct Cell {
    byte: usize,
    left: f32,
    right: f32,
}

/// The cells of the text `list` drew, from the layout it is drawn with, and
/// where each of its lines starts. A multi-line field draws a block per
/// paragraph: their cells and lines follow one another, each byte counted
/// within its own paragraph.
fn drawn(list: &DrawList) -> (Vec<Cell>, Vec<usize>) {
    let handle = list.text_measurer.font_system_handle();
    let mut shared = handle.lock().unwrap();
    let (mut cells, mut lines) = (Vec::new(), Vec::new());
    for block in list.texts.iter().filter(|block| !block.content.is_empty()) {
        let layout = shared.layout(&LayoutSpec::of_block(block), &block.content);
        cells.extend(layout.glyphs.iter().map(|g| Cell {
            byte: g.byte_start as usize,
            left: block.x + g.cell_x,
            right: block.x + g.cell_x + g.advance,
        }));
        // Where each line, wrapped ones included, starts: its first glyph's byte.
        lines.extend(layout.lines.iter().map(|l| {
            layout
                .glyphs
                .get(l.glyphs.start as usize)
                .filter(|_| !l.glyphs.is_empty())
                .map_or(l.byte_start as usize, |g| g.byte_start as usize)
        }));
    }
    assert!(!cells.is_empty(), "the field drew its text");
    (cells, lines)
}

fn click(ti: &mut TextInput, focus: &mut FocusState, x: f32, y: f32) {
    let press = InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_down: true,
        mouse_clicked: true,
        mouse_click_count: 1,
        ..Default::default()
    };
    frame(ti, focus, &press);
    let release = InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_released: true,
        ..Default::default()
    };
    frame(ti, focus, &release);
}

fn assert_near(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() <= NEAR, "{what}: {got} against {want}");
}

#[test]
fn a_click_just_inside_a_glyph_puts_the_caret_before_it() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value("hello wide world");
    let mut focus = FocusState::new();
    let (cells, _) = drawn(&idle(&mut ti, &mut focus));
    for cell in &cells {
        click(&mut ti, &mut focus, cell.left + 0.5, 12.0);
        assert_eq!(ti.cursor_pos, cell.byte, "{cell:?}");
    }
}

#[test]
fn the_caret_sits_on_the_drawn_edge_before_each_glyph_and_after_the_last() {
    let value = "hello wide world";
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value(value);
    let mut focus = FocusState::new();
    let (cells, _) = drawn(&idle(&mut ti, &mut focus));
    for cell in &cells {
        let (x, _) = ime_at(&mut ti, &mut focus, cell.byte);
        assert_near(x, cell.left, &format!("before byte {}", cell.byte));
    }
    let (x, _) = ime_at(&mut ti, &mut focus, value.len());
    assert_near(x, cells.last().unwrap().right, "after the last glyph");
}

#[test]
fn in_right_to_left_text_the_caret_sits_on_the_edge_where_its_byte_begins() {
    let value = "אבג";
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value(value);
    let mut focus = FocusState::new();
    // The glyphs as drawn in the frame placing the caret: a caret at the
    // field's right edge scrolls it to show all of the caret.
    let mut caret_and_cells = |byte| {
        ti.cursor_pos = byte;
        let list = idle(&mut ti, &mut focus);
        (focus.ime_request().unwrap().x, drawn(&list).0)
    };
    let (x, cells) = caret_and_cells(0);
    let first = cells.iter().find(|c| c.byte == 0).unwrap();
    let last = cells.iter().max_by_key(|c| c.byte).unwrap();
    assert!(first.left > last.left, "drawn right to left: {cells:?}");
    assert_near(x, first.right, "before the first letter: its right edge");
    let (x, cells) = caret_and_cells(value.len());
    let last = cells.iter().max_by_key(|c| c.byte).unwrap();
    assert_near(x, last.left, "after the last letter: its left edge");
}

#[test]
fn a_click_just_inside_the_start_of_a_right_to_left_letter_puts_the_caret_before_it() {
    let value = "אבג";
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value(value);
    let mut focus = FocusState::new();
    for byte in [0, 2, 4] {
        let (cells, _) = drawn(&idle(&mut ti, &mut focus));
        let cell = cells.iter().find(|c| c.byte == byte).unwrap();
        // A right-to-left letter starts at its right edge.
        click(&mut ti, &mut focus, cell.right - 0.5, 12.0);
        assert_eq!(ti.cursor_pos, byte, "{cell:?}");
    }
    let (cells, _) = drawn(&idle(&mut ti, &mut focus));
    let last = cells.iter().max_by_key(|c| c.byte).unwrap();
    click(&mut ti, &mut focus, last.left + 0.5, 12.0);
    assert_eq!(ti.cursor_pos, value.len(), "past its last letter's end");
}

#[test]
fn a_click_on_a_bullet_of_a_password_lands_on_its_letter() {
    // Bytes 0, 1 and 3 in the value; each bullet is three bytes.
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0)
        .password()
        .with_value("añb");
    let mut focus = FocusState::new();
    let (cells, _) = drawn(&idle(&mut ti, &mut focus));
    assert_eq!(cells.len(), 3, "{cells:?}");
    for (cell, byte) in cells.iter().zip([0, 1, 3]) {
        click(&mut ti, &mut focus, cell.left + 0.5, 12.0);
        assert_eq!(ti.cursor_pos, byte, "{cell:?}");
    }
}

#[test]
fn while_composing_the_caret_sits_in_the_preedit_where_the_ime_asks() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value("ab");
    let mut focus = FocusState::new();
    ti.cursor_pos = 2;
    let composing = |cursor| InputState {
        preedit: "xy".into(),
        preedit_cursor: cursor,
        ..Default::default()
    };
    // At the end of the preedit, then inside it, after its first letter.
    let list = frame(&mut ti, &mut focus, &composing(None));
    let (cells, _) = drawn(&list);
    let x = focus.ime_request().unwrap().x;
    assert_near(x, cells.last().unwrap().right, "after the preedit");
    frame(&mut ti, &mut focus, &composing(Some([1, 1])));
    let x = focus.ime_request().unwrap().x;
    let y_cell = cells.iter().find(|c| c.byte == 3).unwrap();
    assert_near(x, y_cell.left, "inside the preedit");
}

#[test]
fn composing_on_a_later_line_puts_the_caret_and_ime_rect_on_that_line() {
    let value = "first\nsecond";
    let mut ti = TextInput::new(0.0, 0.0, 300.0, 120.0)
        .with_multiline(true)
        .with_value(value);
    let mut focus = FocusState::new();
    let (_, top) = ime_at(&mut ti, &mut focus, 0);
    let (_, second_top) = ime_at(&mut ti, &mut focus, value.len());
    assert!(second_top > top, "the second line is lower");
    let composing = InputState {
        preedit: "xy".into(),
        ..Default::default()
    };
    let list = frame(&mut ti, &mut focus, &composing);
    let (cells, _) = drawn(&list);
    let rect = focus.ime_request().unwrap();
    assert_eq!(rect.y, second_top, "on the composition's line");
    assert_near(rect.x, cells.last().unwrap().right, "after the preedit");
}

#[test]
fn after_a_space_at_a_soft_wrap_the_next_letter_starts_the_second_line() {
    let value = "wrap these words";
    let mut ti = TextInput::new(0.0, 0.0, 90.0, 120.0)
        .with_multiline(true)
        .with_value(value);
    let mut focus = FocusState::new();
    let (_, lines) = drawn(&idle(&mut ti, &mut focus));
    assert!(lines.len() >= 2, "it wraps: {lines:?}");
    let wrap = lines[1];
    let (start_x, top) = ime_at(&mut ti, &mut focus, 0);
    let (x, y) = ime_at(&mut ti, &mut focus, wrap - 1);
    assert_eq!(y, top, "the space is on the first line");
    assert!(x > start_x, "at its end");
    let (x, y) = ime_at(&mut ti, &mut focus, wrap);
    assert!(y > top, "the letter after it is on the second line");
    assert_near(x, start_x, "at its start");
}

#[test]
fn where_a_word_is_broken_by_the_wrap_its_byte_ends_the_first_line() {
    // The same byte ends one line and starts the next; the caret takes the
    // first.
    let value = "abcdefghijklmnopqrstuvwxyz";
    let mut ti = TextInput::new(0.0, 0.0, 90.0, 120.0)
        .with_multiline(true)
        .with_value(value);
    let mut focus = FocusState::new();
    let (cells, lines) = drawn(&idle(&mut ti, &mut focus));
    assert!(
        lines.len() >= 2 && lines[1] > 0,
        "it breaks the word: {lines:?}"
    );
    let wrap = lines[1];
    let (start_x, top) = ime_at(&mut ti, &mut focus, 0);
    let (x, y) = ime_at(&mut ti, &mut focus, wrap);
    assert_eq!(y, top, "the break's byte is on the first line");
    let before = cells.iter().find(|c| c.byte == wrap - 1).unwrap();
    assert_near(x, before.right, "after its last letter");
    let (x, y) = ime_at(&mut ti, &mut focus, wrap + 1);
    assert!(y > top, "the next byte is on the second line");
    assert!(x > start_x, "after its first letter");
}

#[test]
fn after_a_final_newline_the_caret_starts_the_empty_last_line() {
    let mut ti = TextInput::new(0.0, 0.0, 300.0, 120.0)
        .with_multiline(true)
        .with_value("ab\n");
    let mut focus = FocusState::new();
    let (start_x, top) = ime_at(&mut ti, &mut focus, 0);
    let (x, y) = ime_at(&mut ti, &mut focus, 3);
    assert!(y > top, "on the line after the break");
    assert_near(x, start_x, "at its start");
}

#[test]
fn a_caret_inside_a_cluster_is_drawn_at_the_cluster_end() {
    // "e" and a combining acute are one cluster, bytes 0..3.
    let mut ti = TextInput::new(0.0, 0.0, 300.0, 120.0)
        .with_multiline(true)
        .with_value("e\u{301}x");
    let mut focus = FocusState::new();
    let (start, _) = ime_at(&mut ti, &mut focus, 0);
    let (inside, _) = ime_at(&mut ti, &mut focus, 1);
    let (end, _) = ime_at(&mut ti, &mut focus, 3);
    assert!(end > start, "the cluster has width");
    assert_near(inside, end, "rounded forward to the cluster's end");
}

/// Shaping passes, counting the renderer's lookup of the text drawn: none for
/// a frame where nothing changed, one for a frame with an edit.
fn assert_idle_shapes_nothing_and_an_edit_once(mut ti: TextInput) {
    let mut focus = FocusState::new();
    let idle = InputState::default();
    // Every frame of a test draws into a new list, with a font system of its
    // own: draw into one list throughout, as an app does.
    let mut list = DrawList::new();
    let theme = Theme::default();
    let mut passes = |ti: &mut TextInput, input: &InputState| {
        let before = crate::text::shaping_passes();
        list.clear();
        focus.focus(ID);
        focus.begin_frame(input);
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            ti.draw(ID, &mut ctx);
        }
        let handle = list.text_measurer.font_system_handle();
        let mut shared = handle.lock().unwrap();
        for block in &list.texts {
            shared.layout(&LayoutSpec::of_block(block), &block.content);
        }
        crate::text::shaping_passes() - before
    };
    passes(&mut ti, &idle);
    assert_eq!(passes(&mut ti, &idle), 0, "an idle frame");
    let typed = InputState {
        text_input: "x".into(),
        ..Default::default()
    };
    assert_eq!(passes(&mut ti, &typed), 1, "a frame typing a letter");
    assert_eq!(passes(&mut ti, &idle), 0, "the idle frame after it");
}

#[test]
fn a_single_line_field_shapes_nothing_while_idle_and_once_per_edit() {
    assert_idle_shapes_nothing_and_an_edit_once(
        TextInput::new(0.0, 0.0, 400.0, 24.0).with_value("hello world"),
    );
}

#[test]
fn a_masked_field_shapes_nothing_while_idle_and_once_per_edit() {
    assert_idle_shapes_nothing_and_an_edit_once(
        TextInput::new(0.0, 0.0, 400.0, 24.0)
            .with_value("hunter2")
            .with_mask('•'),
    );
}

#[test]
fn a_multi_line_field_shapes_nothing_while_idle_and_once_per_edit() {
    assert_idle_shapes_nothing_and_an_edit_once(
        TextInput::new(0.0, 0.0, 300.0, 200.0)
            .with_multiline(true)
            .with_value("first line\nsecond line, long enough to wrap in the field"),
    );
}

#[test]
fn a_selection_over_a_long_field_draws_only_the_lines_in_view() {
    let value = (0..2_000)
        .map(|i| format!("line {i} of the field"))
        .collect::<Vec<_>>()
        .join("\n");
    let len = value.len();
    let mut ti = TextInput::new(0.0, 0.0, 300.0, 200.0)
        .with_multiline(true)
        .with_value(value);
    let mut focus = FocusState::new();
    // The caret at the end, so the field scrolls to its last lines.
    ti.cursor_pos = len;
    ti.selection_start = None;
    let plain = idle(&mut ti, &mut focus).prim_counts().chrome_instances;
    ti.selection_start = Some(0);
    let mut list = idle(&mut ti, &mut focus);
    let selected = list.prim_counts().chrome_instances;
    let block = list
        .texts
        .iter()
        .find(|block| !block.content.is_empty())
        .expect("the field drew its text")
        .clone();
    let line_height = list.caret_layout(&block)[0].line_height;
    let in_view = (200.0 / line_height).ceil() as usize + 1;
    let drawn = selected - plain;
    assert!(
        (1..=in_view).contains(&drawn),
        "{drawn} selection rects for at most {in_view} lines in view"
    );
}

/// "ab", then "גד" right to left: byte 2 has two caret stops, the end of "b"
/// and the start of "ג", which is the right end of the line.
const MIXED: &str = "abגד";

fn caret_x(ti: &mut TextInput, focus: &mut FocusState) -> f32 {
    idle(ti, focus);
    focus
        .ime_request()
        .expect("a focused field asks for the IME")
        .x
}

#[test]
fn a_click_on_either_stop_of_a_direction_boundary_puts_the_caret_there() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value(MIXED);
    let mut focus = FocusState::new();
    let (cells, _) = drawn(&idle(&mut ti, &mut focus));
    let gimel = *cells.iter().find(|c| c.byte == 2).unwrap();
    let b = *cells.iter().find(|c| c.byte == 1).unwrap();
    assert!(gimel.left > b.right, "ג is drawn right of b: {cells:?}");

    click(&mut ti, &mut focus, gimel.right - 0.5, 12.0);
    assert_eq!(ti.cursor_pos, 2);
    assert_near(
        caret_x(&mut ti, &mut focus),
        gimel.right,
        "at the start of ג",
    );

    click(&mut ti, &mut focus, b.right - 0.5, 12.0);
    assert_eq!(ti.cursor_pos, 2);
    assert_near(caret_x(&mut ti, &mut focus), b.right, "at the end of b");
}

#[test]
fn an_arrow_onto_a_direction_boundary_keeps_the_stop_it_moved_to() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value(MIXED);
    let mut focus = FocusState::new();
    let (cells, _) = drawn(&idle(&mut ti, &mut focus));
    let gimel = *cells.iter().find(|c| c.byte == 2).unwrap();
    let right = InputState {
        key_right: true,
        ..Default::default()
    };
    let left = InputState {
        key_left: true,
        ..Default::default()
    };
    // Between ג and ד, then right: onto the start of ג, the line's right end.
    ti.cursor_pos = 4;
    frame(&mut ti, &mut focus, &right);
    assert_eq!(ti.cursor_pos, 2);
    assert_near(
        caret_x(&mut ti, &mut focus),
        gimel.right,
        "at the start of ג",
    );
    // And left again: back between ג and ד, not from the end of b.
    frame(&mut ti, &mut focus, &left);
    assert_eq!(
        ti.cursor_pos, 4,
        "one step left of where the caret was drawn"
    );
}

#[test]
fn a_field_in_a_scrolled_view_asks_for_the_ime_where_it_shows() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 24.0).with_value("hello");
    let mut focus = FocusState::default();
    let (x, y) = ime_at(&mut ti, &mut focus, 2);

    let mut list = DrawList::new();
    let theme = Theme::default();
    let input = InputState::default();
    focus.focus(ID);
    focus.begin_frame(&input);
    list.push_transform();
    list.translate(30.0, -200.0);
    {
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        ti.draw(ID, &mut ctx);
    }
    list.pop_transform();
    let rect = focus
        .ime_request()
        .expect("a focused field asks for the IME");
    assert!(
        (rect.x - (x + 30.0)).abs() < NEAR,
        "{} vs {}",
        rect.x,
        x + 30.0
    );
    assert!(
        (rect.y - (y - 200.0)).abs() < NEAR,
        "{} vs {}",
        rect.y,
        y - 200.0
    );
}

/// A long value, as an 8,000-line paste leaves it: `line N of the pasted text`.
fn pasted(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("line {i} of the pasted text"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One frame of `ti` into `list`, as an app draws every frame into the same
/// one; `focused` or not. The layout cache's counts after it.
fn frame_into(
    list: &mut DrawList,
    ti: &mut TextInput,
    focus: &mut FocusState,
    input: &InputState,
    focused: bool,
) -> crate::shaping::LayoutStats {
    let theme = Theme::default();
    list.clear();
    if focused {
        focus.focus(ID);
    } else {
        focus.blur();
    }
    focus.begin_frame(input);
    {
        let mut ctx = DrawContext::new(list, focus, &theme, input, 800.0, 600.0);
        ti.draw(ID, &mut ctx);
    }
    list.text_measurer
        .font_system_handle()
        .lock()
        .unwrap()
        .layout_stats()
}

#[test]
fn a_long_field_lays_out_only_the_paragraphs_in_view() {
    let value = pasted(8_000);
    let len = value.len();
    for focused in [true, false] {
        let mut ti = TextInput::new(0.0, 0.0, 400.0, 120.0)
            .with_multiline(true)
            .with_value(value.clone());
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let stats = frame_into(
            &mut list,
            &mut ti,
            &mut focus,
            &InputState::default(),
            focused,
        );
        // 120 px holds six or so 20 px lines; a few more for the caret's
        // paragraph and the one cut by the box's edge.
        assert!(
            stats.layouts <= 12,
            "focused {focused}: {} layouts",
            stats.layouts
        );
        // Laid out whole, the value takes megabytes.
        assert!(
            stats.bytes < 64 * 1024,
            "focused {focused}: {} bytes of layout for a {len} byte value",
            stats.bytes
        );
        let drawn: usize = list.texts.iter().map(|block| block.content.len()).sum();
        assert!(
            drawn < 1_000,
            "focused {focused}: drew {drawn} bytes of text"
        );
    }
}

#[test]
fn typing_at_the_end_of_a_long_field_lays_out_one_paragraph() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 120.0)
        .with_multiline(true)
        .with_value(pasted(8_000));
    let mut focus = FocusState::new();
    let mut list = DrawList::new();
    let idle = InputState::default();
    let before = frame_into(&mut list, &mut ti, &mut focus, &idle, true).shaped;
    let typed = InputState {
        text_input: "x".into(),
        ..Default::default()
    };
    let after = frame_into(&mut list, &mut ti, &mut focus, &typed, true).shaped;
    assert_eq!(after - before, 1, "the edited paragraph alone");
    let still = frame_into(&mut list, &mut ti, &mut focus, &idle, true).shaped;
    assert_eq!(still, after, "nothing on the idle frame after it");
}

#[test]
fn the_caret_stays_in_the_box_walking_up_through_estimated_paragraphs() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 120.0)
        .with_multiline(true)
        .with_value(pasted(500));
    let mut focus = FocusState::new();
    let mut list = DrawList::new();
    frame_into(&mut list, &mut ti, &mut focus, &InputState::default(), true);
    let up = InputState {
        key_up: true,
        ..Default::default()
    };
    for step in 0..50 {
        let was = ti.cursor_pos;
        frame_into(&mut list, &mut ti, &mut focus, &up, true);
        assert!(ti.cursor_pos < was, "step {step}: up moved the caret");
        let rect = focus
            .ime_request()
            .expect("a focused field asks for the IME");
        assert!(
            rect.y >= 0.0 && rect.y + rect.height <= 120.0,
            "step {step}: the caret at {} is in the box",
            rect.y
        );
    }
}

#[test]
fn the_text_shown_stays_put_while_nothing_changes() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 120.0)
        .with_multiline(true)
        .with_value(pasted(2_000));
    let mut focus = FocusState::new();
    let mut list = DrawList::new();
    let idle = InputState::default();
    let shown = |list: &DrawList| {
        list.texts
            .iter()
            .map(|block| (block.content.clone(), block.y))
            .collect::<Vec<_>>()
    };
    frame_into(&mut list, &mut ti, &mut focus, &idle, true);
    let first = shown(&list);
    assert!(
        first
            .iter()
            .any(|(text, _)| text == "line 1999 of the pasted text"),
        "scrolled to the caret at the end: {first:?}"
    );
    frame_into(&mut list, &mut ti, &mut focus, &idle, true);
    assert_eq!(shown(&list), first);
}

#[test]
fn a_click_lands_on_the_paragraph_drawn_under_it() {
    let mut ti = TextInput::new(0.0, 0.0, 400.0, 120.0)
        .with_multiline(true)
        .with_value(pasted(2_000));
    let mut focus = FocusState::new();
    let list = idle(&mut ti, &mut focus);
    // Click halfway down each paragraph drawn, just right of its start.
    let rows: Vec<(String, f32)> = list
        .texts
        .iter()
        .map(|block| (block.content.clone(), block.y))
        .filter(|(_, y)| *y >= 0.0 && *y + 20.0 <= 120.0)
        .collect();
    assert!(!rows.is_empty());
    for (text, y) in rows {
        click(&mut ti, &mut focus, 12.0, y + 8.0);
        let start = ti.value[..ti.cursor_pos].rfind('\n').map_or(0, |i| i + 1);
        let line = &ti.value[start..];
        let line = &line[..line.find('\n').unwrap_or(line.len())];
        assert_eq!(line, text, "clicked at y {}", y + 8.0);
    }
}
