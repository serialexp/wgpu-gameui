use super::*;

fn press(key: Key, text: Option<&str>) -> HostEvent {
    HostEvent::Key {
        key,
        down: true,
        repeat: false,
        text: text.map(str::to_owned),
    }
}

fn release(key: Key) -> HostEvent {
    HostEvent::Key {
        key,
        down: false,
        repeat: false,
        text: None,
    }
}

fn button(button: PointerButton, down: bool) -> HostEvent {
    HostEvent::Button { button, down }
}

fn primary() -> HeldModifiers {
    if cfg!(target_os = "macos") {
        HeldModifiers {
            logo: true,
            ..HeldModifiers::default()
        }
    } else {
        HeldModifiers {
            control: true,
            ..HeldModifiers::default()
        }
    }
}

#[test]
fn a_click_inside_one_frame_has_both_edges_and_ends_up() {
    let mut input = InputState::default();
    input.apply(&button(PointerButton::Primary, true));
    input.apply(&button(PointerButton::Primary, false));
    assert!(input.mouse_clicked && input.mouse_released && !input.mouse_down);
    input.end_frame();
    assert!(!input.mouse_clicked && !input.mouse_released);
}

#[test]
fn a_held_button_has_its_press_edge_once() {
    let mut input = InputState::default();
    input.apply(&button(PointerButton::Secondary, true));
    assert!(input.mouse_right_clicked && input.mouse_right_down);
    input.end_frame();
    input.apply(&button(PointerButton::Secondary, true));
    assert!(!input.mouse_right_clicked && input.mouse_right_down);
    input.apply(&button(PointerButton::Secondary, false));
    assert!(input.mouse_right_released && !input.mouse_right_down);
}

#[test]
fn each_button_has_its_own_fields() {
    let mut input = InputState::default();
    input.apply(&button(PointerButton::Middle, true));
    assert!(input.mouse_middle_clicked && input.mouse_middle_down);
    assert!(!input.mouse_clicked && !input.mouse_right_clicked);
}

#[test]
fn the_pointer_leaving_moves_it_away_unless_a_button_is_held() {
    let mut input = InputState::default();
    input.apply(&HostEvent::PointerMoved { x: 10.0, y: 20.0 });
    assert_eq!((input.mouse_x, input.mouse_y), (10.0, 20.0));
    input.apply(&button(PointerButton::Primary, true));
    input.apply(&HostEvent::PointerLeft);
    assert_eq!((input.mouse_x, input.mouse_y), (10.0, 20.0));
    input.apply(&button(PointerButton::Primary, false));
    input.apply(&HostEvent::PointerLeft);
    assert_eq!((input.mouse_x, input.mouse_y), (POINTER_AWAY, POINTER_AWAY));
}

#[test]
fn wheel_events_in_one_frame_add_up() {
    let mut input = InputState::default();
    input.apply(&HostEvent::Wheel { dx: 1.0, dy: -2.0 });
    input.apply(&HostEvent::Wheel { dx: 0.5, dy: -1.0 });
    assert_eq!((input.scroll_delta_x, input.scroll_delta), (1.5, -3.0));
}

#[test]
fn a_key_tapped_between_frames_still_sets_its_flag() {
    let mut input = InputState::default();
    input.apply(&press(Key::Left, None));
    input.apply(&release(Key::Left));
    assert!(input.key_left);
    assert!(input.keys.was_pressed(Key::Left) && input.keys.was_released(Key::Left));
}

/// Reads one of `InputState`'s key flags.
type Flag = fn(&InputState) -> bool;

#[test]
fn named_keys_set_their_flags() {
    let cases: [(Key, Flag); 12] = [
        (Key::Enter, |i| i.enter_pressed),
        (Key::Backspace, |i| i.backspace_pressed),
        (Key::Left, |i| i.key_left),
        (Key::Right, |i| i.key_right),
        (Key::Up, |i| i.key_up),
        (Key::Down, |i| i.key_down),
        (Key::Home, |i| i.key_home),
        (Key::End, |i| i.key_end),
        (Key::Delete, |i| i.key_delete),
        (Key::Tab, |i| i.key_tab),
        (Key::Escape, |i| i.key_escape),
        (Key::Space, |i| i.key_space),
    ];
    for (key, flag) in cases {
        let mut input = InputState::default();
        input.apply(&press(key, None));
        assert!(flag(&input), "{key:?}");
        input.end_frame();
        assert!(!flag(&input), "{key:?} after the frame");
    }
}

#[test]
fn a_repeat_sets_the_flag_again_but_not_the_key_state_edge() {
    let mut input = InputState::default();
    input.apply(&press(Key::Char('f'), None));
    input.end_frame();
    input.apply(&HostEvent::Key {
        key: Key::Char('f'),
        down: true,
        repeat: true,
        text: None,
    });
    assert!(!input.keys.was_pressed(Key::Char('f')));
    assert!(input.keys.is_down(Key::Char('f')));
    input.apply(&HostEvent::Key {
        key: Key::Down,
        down: true,
        repeat: true,
        text: None,
    });
    assert!(input.key_down);
}

#[test]
fn typed_text_drops_control_characters() {
    let mut input = InputState::default();
    input.apply(&press(Key::Char('a'), Some("a")));
    input.apply(&press(Key::Enter, Some("\r")));
    input.apply(&press(Key::Tab, Some("\t")));
    input.apply(&press(Key::Char('é'), Some("é")));
    assert_eq!(input.text_input, "aé");
}

#[test]
fn the_shortcut_key_edits_instead_of_typing() {
    let mut input = InputState::default();
    input.apply(&HostEvent::Modifiers(primary()));
    assert!(input.ctrl_pressed);
    for (c, flag) in [
        ('a', (|i| i.key_select_all) as Flag),
        ('C', |i| i.key_copy),
        ('x', |i| i.key_cut),
        ('v', |i| i.key_paste),
    ] {
        input.apply(&press(Key::Char(c), Some(&c.to_string())));
        assert!(flag(&input), "{c}");
    }
    assert_eq!(input.text_input, "");
}

#[test]
fn modifiers_are_held_and_alt_has_edges() {
    let mut input = InputState::default();
    let alt = HeldModifiers {
        alt: true,
        shift: true,
        ..HeldModifiers::default()
    };
    input.apply(&HostEvent::Modifiers(alt));
    assert!(input.alt_down && input.alt_pressed && input.shift_pressed);
    input.end_frame();
    input.apply(&HostEvent::Modifiers(alt));
    assert!(input.alt_down && !input.alt_pressed);
    input.apply(&HostEvent::Modifiers(HeldModifiers::default()));
    assert!(!input.alt_down && input.alt_released && !input.shift_pressed);
}

#[test]
fn control_is_not_the_shortcut_key_on_macos() {
    let control = HeldModifiers {
        control: true,
        ..HeldModifiers::default()
    };
    assert_eq!(control.primary(), !cfg!(target_os = "macos"));
}

#[test]
fn a_composition_is_held_until_committed() {
    let mut input = InputState::default();
    input.apply(&HostEvent::Preedit {
        text: "にほ".into(),
        cursor: Some([3, 3]),
    });
    input.end_frame();
    assert_eq!(input.preedit, "にほ");
    assert_eq!(input.preedit_cursor, Some([3, 3]));
    input.apply(&HostEvent::Commit("日本".into()));
    assert_eq!(
        (input.preedit.as_str(), input.text_input.as_str()),
        ("", "日本")
    );
    assert_eq!(input.preedit_cursor, None);
}

#[test]
fn modifier_sets_combine() {
    let both = HeldModifiers::CONTROL | HeldModifiers::SHIFT;
    assert!(both.control && both.shift && !both.alt && !both.logo);
    assert!(HeldModifiers::default().is_empty() && !HeldModifiers::LOGO.is_empty());
}
