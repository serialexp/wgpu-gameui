//! Random edits, caret moves, clicks, drags and IME composition over a
//! [`TextInput`]. After each step its caret and selection must still sit on its
//! text, on character boundaries. A caret left past the end of the text fails
//! here with the seed and the step that broke it, rather than in
//! `delete_selection`.

use super::*;
use crate::{DrawContext, DrawList, FocusState, InputState, Theme};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

const ID: FocusId = 9;
const SEEDS: u64 = 20;
const STEPS: usize = 300;

/// Deterministic xorshift, so a failing seed replays exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

/// Text covering one-byte, multi-byte and line-break characters.
const PIECES: &[&str] = &[
    "a",
    "b",
    " ",
    "\n",
    "é",
    "漢",
    "x y",
    "longer words",
    "\n\n",
    "🙂",
];

fn random_text(rng: &mut Rng) -> String {
    (0..rng.below(8))
        .map(|_| PIECES[rng.below(PIECES.len())])
        .collect()
}

/// One frame's input at a random spot in the field. The primary button stays
/// held across frames once pressed (`held`), so a drag or a word selection can
/// be grown while other edits land, as a real hold does. Returns what it is,
/// for the failure message.
fn random_frame(
    rng: &mut Rng,
    clip: &RefCell<String>,
    height: f32,
    held: &mut bool,
) -> (&'static str, InputState) {
    let mut input = InputState {
        mouse_x: rng.below(200) as f32,
        mouse_y: rng.below(height as usize) as f32,
        shift_pressed: rng.chance(30),
        ..Default::default()
    };
    let what = match rng.below(22) {
        0 => {
            input.text_input = random_text(rng);
            "typed"
        }
        1 => {
            input.backspace_pressed = true;
            "backspace"
        }
        2 => {
            input.key_delete = true;
            "delete"
        }
        3 => {
            input.key_left = true;
            "left"
        }
        4 => {
            input.key_right = true;
            "right"
        }
        5 => {
            input.key_up = true;
            "up"
        }
        6 => {
            input.key_down = true;
            "down"
        }
        7 => {
            input.key_home = true;
            "home"
        }
        8 => {
            input.key_end = true;
            "end"
        }
        9 => {
            input.enter_pressed = true;
            "enter"
        }
        10 => {
            input.key_select_all = true;
            "select all"
        }
        11 => {
            input.key_cut = true;
            "cut"
        }
        12 => {
            input.key_copy = true;
            "copy"
        }
        13 => {
            *clip.borrow_mut() = random_text(rng);
            input.key_paste = true;
            "paste"
        }
        14 | 15 => {
            *held = true;
            input.mouse_clicked = true;
            input.mouse_click_count = 1 + rng.below(3) as u32;
            "click"
        }
        16 => "move",
        17 => {
            *held = false;
            input.mouse_released = true;
            "release"
        }
        18 => {
            input.preedit = random_text(rng);
            "composing"
        }
        19 => {
            input.preedit.clear();
            "composed"
        }
        _ => "idle",
    };
    input.mouse_down = *held;
    (what, input)
}

/// One frame of `ti`, focused, under `input`.
fn frame(ti: &mut TextInput, focus: &mut FocusState, input: &InputState) {
    let mut list = DrawList::new();
    let theme = Theme::default();
    focus.focus(ID);
    focus.begin_frame(input);
    let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
    ti.draw(ID, &mut ctx);
}

fn assert_on_the_text(ti: &TextInput) {
    let len = ti.value.len();
    assert!(
        ti.cursor_pos <= len && ti.value.is_char_boundary(ti.cursor_pos),
        "caret {} is off the {len}-byte text",
        ti.cursor_pos
    );
    if let Some(anchor) = ti.selection_start {
        assert!(
            anchor <= len && ti.value.is_char_boundary(anchor),
            "selection anchor {anchor} is off the {len}-byte text"
        );
    }
}

/// Run one seeded sequence. The caller's buffer is kept in step with the field
/// the way `UiContext::text_input` does it: `set_value` when they differ before
/// each frame, the buffer taken from the field after it.
fn run(seed: u64, multiline: bool) {
    let height = if multiline { 80.0 } else { 24.0 };
    let clip = Rc::new(RefCell::new(String::new()));
    let paste_from = clip.clone();
    let mut ti = TextInput::new(0.0, 0.0, 180.0, height).with_multiline(multiline);
    ti.set_clipboard_get(move || paste_from.borrow().clone());
    ti.set_clipboard_set(|_| {});

    let mut focus = FocusState::new();
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761) | 1);
    let mut held = false;
    let mut buffer = random_text(&mut rng);
    for step in 0..STEPS {
        let (what, input) = if rng.chance(8) {
            // The app replaces or clears its buffer between frames, as after a send.
            buffer = if rng.chance(50) {
                String::new()
            } else {
                random_text(&mut rng)
            };
            ("caller sets its buffer", InputState::default())
        } else {
            random_frame(&mut rng, &clip, height, &mut held)
        };
        let before = format!(
            "value {:?}, caret {}, selection {:?}",
            ti.value, ti.cursor_pos, ti.selection_start
        );
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            if ti.value != buffer {
                ti.set_value(&buffer);
            }
            assert_on_the_text(&ti);
            frame(&mut ti, &mut focus, &input);
            assert_on_the_text(&ti);
            if ti.value != buffer {
                buffer = ti.value.clone();
            }
        }));
        if let Err(payload) = outcome {
            eprintln!(
                "seed {seed}, multiline {multiline}, step {step}: {what}\n  before: {before}\n  buffer: {buffer:?}"
            );
            resume_unwind(payload);
        }
    }
}

/// The drag a held button started is over once the text changes under it: its
/// anchor is a byte offset into the text it was taken from.
#[test]
fn an_edit_under_a_held_button_ends_the_drag() {
    let mut ti = TextInput::new(0.0, 0.0, 180.0, 24.0).with_value("hello world");
    let mut focus = FocusState::new();
    let press = InputState {
        mouse_x: 150.0,
        mouse_y: 12.0,
        mouse_down: true,
        mouse_clicked: true,
        mouse_click_count: 1,
        ..Default::default()
    };
    frame(&mut ti, &mut focus, &press);
    assert!(
        ti.drag_selection_anchor.is_some(),
        "the press starts a drag"
    );
    let typed = InputState {
        text_input: "!".into(),
        mouse_down: true,
        ..Default::default()
    };
    frame(&mut ti, &mut focus, &typed);
    assert_eq!(ti.drag_selection_anchor, None);
    assert_eq!(ti.drag_unit, None);
}

#[test]
fn a_single_line_field_keeps_its_caret_on_its_text() {
    for seed in 0..SEEDS {
        run(seed, false);
    }
}

#[test]
fn a_multi_line_field_keeps_its_caret_on_its_text() {
    for seed in 0..SEEDS {
        run(seed, true);
    }
}
