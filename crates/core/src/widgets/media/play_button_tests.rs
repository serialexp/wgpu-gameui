use super::*;
use crate::{DrawList, FocusState, InputState, Theme};

fn input(x: f32, y: f32, down: bool, clicked: bool) -> InputState {
    InputState {
        mouse_x: x,
        mouse_y: y,
        mouse_down: down,
        mouse_clicked: clicked,
        ..InputState::default()
    }
}

fn away() -> InputState {
    input(500.0, 500.0, false, false)
}

/// Draw `button` at the origin in its outer size under `input`.
fn draw(button: PlayButton, input: &InputState) -> (DrawList, crate::Response) {
    let theme = Theme::default();
    let mut list = DrawList::new();
    let mut focus = FocusState::new();
    let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
    let [w, h] = button.outer_size(&ctx.styles());
    let response = button.draw(Rect::new(0.0, 0.0, w, h), &mut ctx);
    (list, response)
}

fn icons(list: &DrawList) -> Vec<crate::render::IconGlyph> {
    list.icons_msdf.iter().map(|icon| icon.glyph).collect()
}

fn glyph(icon: PhosphorIcon) -> crate::render::IconGlyph {
    icon.glyph().expect("in the font")
}

#[test]
fn each_state_shows_its_icon() {
    let cases = [
        (PlayState::Paused, Some(PhosphorIcon::Play)),
        (PlayState::Playing, Some(PhosphorIcon::Pause)),
        (PlayState::Ended, Some(PhosphorIcon::ArrowCounterClockwise)),
        (PlayState::Error, Some(PhosphorIcon::ExclamationMark)),
        // A spinner instead.
        (PlayState::Loading, None),
    ];
    for (state, icon) in cases {
        let (list, _) = draw(PlayButton::new(state), &away());
        let expected: Vec<_> = icon.into_iter().map(glyph).collect();
        assert_eq!(icons(&list), expected, "{state:?}");
    }
    let (loading, _) = draw(PlayButton::new(PlayState::Loading), &away());
    assert_eq!(loading.next_repaint(), Some(0.0), "the spinner turns");
}

#[test]
fn the_error_mark_is_red() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let (list, _) = draw(PlayButton::new(PlayState::Error), &away());
    assert_eq!(list.icons_msdf[0].tint, s.color(StyleKey::DangerText));
}

/// Playing (or waiting to) holds the key in, as a latch does.
#[test]
fn it_is_held_in_while_playing() {
    let face_y = |state| {
        let (list, _) = draw(PlayButton::new(state), &away());
        list.icons_msdf.first().map(|icon| icon.local.y)
    };
    let up = face_y(PlayState::Paused).unwrap();
    let down = face_y(PlayState::Playing).unwrap();
    assert!(down > up, "the face drops by its travel: {up} → {down}");
}

#[test]
fn a_click_answers_unless_it_cant_play() {
    let press = input(10.0, 10.0, true, true);
    let release = InputState {
        mouse_released: true,
        ..input(10.0, 10.0, false, false)
    };
    for (state, answers) in [(PlayState::Paused, true), (PlayState::Error, false)] {
        let theme = Theme::default();
        let mut focus = FocusState::new();
        let mut clicked = false;
        for input in [&press, &release] {
            let mut list = DrawList::new();
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            let button = PlayButton::new(state);
            let [w, h] = button.outer_size(&ctx.styles());
            clicked |= button.draw(Rect::new(0.0, 0.0, w, h), &mut ctx).clicked;
        }
        assert_eq!(clicked, answers, "{state:?}");
    }
}

#[test]
fn a_progress_strip_runs_along_the_bottom() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let strip = |state, progress| {
        let (list, _) = draw(PlayButton::new(state).progress(progress), &away());
        list.chrome_instances()
            .filter(|c| c.rect[3] == PROGRESS)
            .map(|c| (c.rect[2], c.bg))
            .collect::<Vec<_>>()
    };
    let paused = strip(PlayState::Paused, 0.5);
    assert_eq!(
        paused,
        [(PlayButton::SIZE * 0.5, s.color(StyleKey::AccentTick))]
    );
    assert_eq!(
        strip(PlayState::Paused, 2.0)[0].0,
        PlayButton::SIZE,
        "held to the face"
    );
    let playing = strip(PlayState::Playing, 0.5);
    assert_eq!(playing.len(), 1);
    assert_ne!(playing[0].1, paused[0].1, "the latch's ink while playing");
    assert!(strip(PlayState::Error, 0.5).is_empty());
}
