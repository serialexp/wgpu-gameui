use super::*;

const PLAYER: u64 = 7;
const VIEWER: u64 = 9;

/// A frame in which the widgets `drawn` are drawn and ask whether they own
/// the screen; returns the window change the frame asks for.
fn frame_drawing(state: &mut FullscreenState, drawn: &[u64]) -> Option<bool> {
    let input = InputState::default();
    for &id in drawn {
        state.owns(id, &input);
    }
    state.end_frame()
}

fn escape() -> InputState {
    let mut input = InputState::default();
    input.nav.cancel = true;
    input
}

#[test]
fn a_request_takes_the_screen_and_asks_for_a_full_screen_window_once() {
    let mut state = FullscreenState::default();
    let input = InputState::default();
    state.request(PLAYER);
    assert!(state.owns(PLAYER, &input));
    assert!(!state.owns(VIEWER, &input));
    assert_eq!(state.end_frame(), Some(true));
    assert_eq!(frame_drawing(&mut state, &[PLAYER]), None, "asked once");
    assert_eq!(state.owner(), Some(PLAYER));
}

#[test]
fn exit_gives_the_screen_back_and_asks_to_leave_full_screen() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    state.set_window_fullscreen(true);
    state.exit();
    assert_eq!(state.owner(), None);
    assert_eq!(frame_drawing(&mut state, &[PLAYER]), Some(false));
    state.exit();
    assert_eq!(frame_drawing(&mut state, &[]), None, "nothing to end");
}

#[test]
fn escape_reaching_the_owner_ends_full_screen() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    assert!(!state.owns(PLAYER, &escape()));
    assert_eq!(state.owner(), None);
    assert_eq!(state.end_frame(), Some(false));
}

#[test]
fn escape_taken_by_a_menu_over_the_owner_leaves_full_screen_alone() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    // A menu open over the player claims the intent at the top of the frame,
    // so the input the player draws with no longer has it.
    assert!(state.owns(PLAYER, &InputState::default()));
    assert_eq!(state.end_frame(), None);
}

#[test]
fn an_owner_no_longer_drawn_loses_the_screen() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    assert_eq!(frame_drawing(&mut state, &[VIEWER]), Some(false));
    assert_eq!(state.owner(), None);
}

#[test]
fn another_widget_can_take_the_screen_without_the_window_leaving_full_screen() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    state.request(VIEWER);
    assert_eq!(state.end_frame(), None, "the window is full screen already");
    assert_eq!(state.owner(), Some(VIEWER));
}

#[test]
fn a_window_already_full_screen_stays_so_when_the_widget_leaves() {
    let mut state = FullscreenState::default();
    state.set_window_fullscreen(true);
    state.request(PLAYER);
    assert_eq!(state.end_frame(), None);
    state.exit();
    assert_eq!(state.end_frame(), None);
}

#[test]
fn the_window_leaving_full_screen_by_other_means_ends_it() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    state.set_window_fullscreen(true);
    state.set_window_fullscreen(false);
    assert_eq!(state.owner(), None);
    assert_eq!(frame_drawing(&mut state, &[PLAYER]), None, "already out");
    // And full screen can be asked for again.
    state.request(PLAYER);
    assert_eq!(state.end_frame(), Some(true));
}

#[test]
fn a_request_and_exit_in_one_frame_ask_for_nothing() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.exit();
    assert_eq!(state.end_frame(), None);
}

#[test]
fn reports_from_before_a_change_is_done_are_ignored() {
    let mut state = FullscreenState::default();
    // Before the app has seen the request.
    state.request(PLAYER);
    state.set_window_fullscreen(false);
    assert_eq!(state.owner(), Some(PLAYER));
    assert_eq!(state.end_frame(), Some(true));
    // While the window animates into full screen.
    state.set_window_fullscreen(false);
    assert_eq!(state.owner(), Some(PLAYER));
    state.set_window_fullscreen(true);
    // Once it is there, leaving by other means counts again.
    state.set_window_fullscreen(false);
    assert_eq!(state.owner(), None);
}

#[test]
fn a_late_full_screen_report_after_leaving_does_not_block_the_next_request() {
    let mut state = FullscreenState::default();
    state.request(PLAYER);
    state.end_frame();
    state.set_window_fullscreen(true);
    state.exit();
    assert_eq!(state.end_frame(), Some(false));
    state.set_window_fullscreen(true);
    state.set_window_fullscreen(false);
    state.request(PLAYER);
    assert_eq!(state.end_frame(), Some(true));
}
