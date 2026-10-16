use super::*;
use crate::{DrawList, FocusState, InputState, Theme};

const RECT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 100.0,
    height: 12.0,
};

/// Draw `scrubber` at `value` under `input`; the list and the output.
fn draw(
    scrubber: &Scrubber,
    value: f32,
    capture: &mut DragCapture,
    focus: &mut FocusState,
    input: &InputState,
) -> (DrawList, ScrubberOutput) {
    let theme = Theme::default();
    let mut list = DrawList::new();
    let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
    let output = scrubber.draw(value, 7, capture, RECT, &mut ctx);
    (list, output)
}

fn mouse(x: f32, down: bool, clicked: bool) -> InputState {
    InputState {
        mouse_x: x,
        mouse_y: 6.0,
        mouse_down: down,
        mouse_clicked: clicked,
        ..InputState::default()
    }
}

fn away() -> InputState {
    InputState {
        mouse_x: 500.0,
        mouse_y: 500.0,
        ..InputState::default()
    }
}

fn texts(list: &DrawList) -> Vec<&str> {
    list.texts.iter().map(|t| t.content.as_str()).collect()
}

/// Its fills and knob, by colour.
fn fills(list: &DrawList, color: [f32; 4]) -> Vec<[f32; 4]> {
    list.chrome_instances()
        .filter(|c| c.bg == color)
        .map(|c| c.rect)
        .collect()
}

#[test]
fn a_press_seeks_there_and_a_drag_seeks_live_until_let_go() {
    let scrubber = Scrubber::new(60.0);
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();

    let (_, pressed) = draw(
        &scrubber,
        0.0,
        &mut capture,
        &mut focus,
        &mouse(25.0, true, true),
    );
    assert_eq!(pressed.seek, Some(15.0));
    assert!(pressed.started && pressed.dragging && !pressed.ended);

    let (_, dragged) = draw(
        &scrubber,
        15.0,
        &mut capture,
        &mut focus,
        &mouse(50.0, true, false),
    );
    assert_eq!(dragged.seek, Some(30.0));
    assert!(dragged.dragging && !dragged.started);

    // Past the end it holds at the end.
    let (_, past) = draw(
        &scrubber,
        30.0,
        &mut capture,
        &mut focus,
        &mouse(400.0, true, false),
    );
    assert_eq!(past.seek, Some(60.0));

    let (_, let_go) = draw(
        &scrubber,
        60.0,
        &mut capture,
        &mut focus,
        &mouse(400.0, false, false),
    );
    assert_eq!(let_go.seek, None);
    assert!(let_go.ended && !let_go.dragging);
    assert!(capture.is_free());
}

#[test]
fn a_drag_held_still_asks_for_no_seek() {
    let scrubber = Scrubber::new(60.0);
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    draw(
        &scrubber,
        0.0,
        &mut capture,
        &mut focus,
        &mouse(25.0, true, true),
    );
    let (_, held) = draw(
        &scrubber,
        15.0,
        &mut capture,
        &mut focus,
        &mouse(25.0, true, false),
    );
    assert_eq!(held.seek, None);
    assert!(held.dragging);
}

#[test]
fn hovering_shows_the_time_under_the_pointer() {
    let scrubber = Scrubber::new(120.0);
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    let (list, output) = draw(
        &scrubber,
        0.0,
        &mut capture,
        &mut focus,
        &mouse(75.0, false, false),
    );
    assert!(output.hovered);
    assert_eq!(texts(&list), ["1:30"]);
    let (list, _) = draw(&scrubber, 0.0, &mut capture, &mut focus, &away());
    assert!(texts(&list).is_empty());

    let percent = |value: f32| format!("{}%", (value * 100.0).round());
    let volume = Scrubber::new(1.0).format(&percent).tip(true);
    let (list, _) = draw(
        &volume,
        0.5,
        &mut capture,
        &mut focus,
        &mouse(25.0, false, false),
    );
    assert_eq!(texts(&list), ["25%"]);
    let quiet = Scrubber::new(1.0).tip(false);
    let (list, _) = draw(
        &quiet,
        0.5,
        &mut capture,
        &mut focus,
        &mouse(25.0, false, false),
    );
    assert!(texts(&list).is_empty());
}

#[test]
fn the_played_fill_reaches_the_value_over_the_buffered_layer() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let scrubber = Scrubber::new(100.0).buffered(80.0);
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    let (list, _) = draw(&scrubber, 25.0, &mut capture, &mut focus, &away());
    let played = fills(&list, s.color(StyleKey::AccentFaceTop));
    assert_eq!(played.len(), 1);
    // Inside the 1 px edge of the well.
    assert_eq!(played[0][2], (100.0 - 2.0) * 0.25);
    let buffered = fills(&list, BUFFERED);
    assert_eq!(buffered.len(), 1);
    assert_eq!(buffered[0][2], (100.0 - 2.0) * 0.8);
}

#[test]
fn the_knob_shows_unless_asked_not_to() {
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    let (with, _) = draw(&Scrubber::new(10.0), 5.0, &mut capture, &mut focus, &away());
    assert_eq!(fills(&with, KNOB_TOP).len(), 1);
    let (bare, _) = draw(
        &Scrubber::new(10.0).knob(false),
        5.0,
        &mut capture,
        &mut focus,
        &away(),
    );
    assert!(fills(&bare, KNOB_TOP).is_empty());
}

#[test]
fn focused_keys_step_and_jump_to_the_ends() {
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    let long = Scrubber::new(600.0).focusable(3);
    let short = Scrubber::new(10.0).focusable(3);
    // Registered, then focused.
    draw(&long, 0.0, &mut capture, &mut focus, &away());
    focus.request(3);
    let key = |set: fn(&mut InputState)| {
        let mut input = away();
        set(&mut input);
        input
    };
    let right = key(|i| i.nav.right = true);
    let left = key(|i| i.nav.left = true);
    let shift_right = key(|i| {
        i.nav.right = true;
        i.shift_pressed = true;
    });
    let home = key(|i| i.key_home = true);
    let end = key(|i| i.key_end = true);
    let cases: [(&Scrubber, f32, &InputState, Option<f32>); 7] = [
        // Over 30 s a step is 5 s; Shift steps six times as far.
        (&long, 100.0, &right, Some(105.0)),
        (&long, 100.0, &left, Some(95.0)),
        (&long, 100.0, &shift_right, Some(130.0)),
        (&long, 2.0, &left, Some(0.0)),
        (&long, 100.0, &home, Some(0.0)),
        (&long, 100.0, &end, Some(600.0)),
        // Shorter, a fiftieth.
        (&short, 5.0, &right, Some(5.2)),
    ];
    for (scrubber, value, input, expected) in cases {
        let (_, output) = draw(scrubber, value, &mut capture, &mut focus, input);
        let got = output.seek.map(|seek| (seek * 1000.0).round() / 1000.0);
        assert_eq!(got, expected, "from {value}");
    }
    // Unfocused, keys do nothing.
    focus.request(99);
    let (_, output) = draw(&long, 100.0, &mut capture, &mut focus, &right);
    assert_eq!(output.seek, None);
}

#[test]
fn a_disabled_scrubber_takes_no_input() {
    let scrubber = Scrubber::new(60.0).enabled(false).focusable(3);
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    let (list, output) = draw(
        &scrubber,
        0.0,
        &mut capture,
        &mut focus,
        &mouse(25.0, true, true),
    );
    assert_eq!(output.seek, None);
    assert!(!output.dragging && !output.hovered);
    assert!(capture.is_free());
    assert!(texts(&list).is_empty(), "no tip");
    assert!(fills(&list, KNOB_TOP).is_empty(), "no knob");
}

#[test]
fn a_scrubber_with_nothing_to_play_seeks_nowhere() {
    let scrubber = Scrubber::new(0.0);
    let mut capture = DragCapture::new();
    let mut focus = FocusState::new();
    let (_, output) = draw(
        &scrubber,
        0.0,
        &mut capture,
        &mut focus,
        &mouse(25.0, true, true),
    );
    assert_eq!(output.seek, Some(0.0));
}
