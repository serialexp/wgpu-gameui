use super::super::Picture;
use super::super::tests::Fake;
use super::*;
use crate::{FocusState, InputState, Theme};

const WIDTH: f32 = 320.0;
const ID: FocusId = 40;

fn toolbar() -> f32 {
    let theme = Theme::default();
    VideoPlayer::toolbar_height(&StyleResolver::new(&theme))
}

/// A full screen player's rect: no border, so the frame is the top
/// `height - toolbar`.
fn rect(frame_height: f32) -> Rect {
    Rect::new(0.0, 0.0, WIDTH, frame_height + toolbar())
}

struct Run {
    focus: FocusState,
    state: VideoPlayerState,
    player: VideoPlayer<'static>,
    rect: Rect,
}

impl Run {
    fn new(player: VideoPlayer<'static>) -> Self {
        Self {
            focus: FocusState::new(),
            state: VideoPlayerState::new(),
            player,
            rect: rect(180.0),
        }
    }

    fn frame(&mut self, source: &mut Fake, input: &InputState) -> (DrawList, VideoPlayerOutput) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut self.focus, &theme, input, 800.0, 600.0);
        let output = self
            .player
            .draw(source, &mut self.state, self.rect, &mut ctx);
        (list, output)
    }

    /// Press and let go at `at`.
    fn click(&mut self, source: &mut Fake, at: (f32, f32)) -> VideoPlayerOutput {
        let (_, pressed) = self.frame(
            source,
            &InputState {
                mouse_down: true,
                mouse_clicked: true,
                ..pointer(at)
            },
        );
        let (_, released) = self.frame(
            source,
            &InputState {
                mouse_released: true,
                ..pointer(at)
            },
        );
        VideoPlayerOutput {
            toggle_fullscreen: pressed.toggle_fullscreen || released.toggle_fullscreen,
        }
    }

    /// The middle of the debug scope named `name`, as last drawn.
    fn find(&mut self, source: &mut Fake, name: &str) -> (f32, f32) {
        let (list, _) = self.frame(source, &away());
        let rect = list
            .debug_scopes()
            .iter()
            .find(|scope| scope.name == name)
            .and_then(|scope| scope.declared)
            .unwrap_or_else(|| panic!("no {name} drawn"));
        (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5)
    }

    fn press_key(&mut self, source: &mut Fake, set: fn(&mut InputState)) {
        let mut input = away();
        set(&mut input);
        self.frame(source, &input);
    }
}

fn pointer(at: (f32, f32)) -> InputState {
    InputState {
        mouse_x: at.0,
        mouse_y: at.1,
        ..InputState::default()
    }
}

fn away() -> InputState {
    pointer((900.0, 900.0))
}

fn letter(input: &mut InputState, key: char) {
    input.keys.key_event(Key::Char(key), true);
}

fn texts(list: &DrawList) -> Vec<&str> {
    list.texts.iter().map(|t| t.content.as_str()).collect()
}

/// Where each image of `sprite` was drawn, as `[x, y, width, height]`.
fn images(list: &DrawList, sprite: crate::SpriteId) -> Vec<[f32; 4]> {
    list.icons
        .iter()
        .filter(|icon| icon.sprite == Some(sprite))
        .map(|icon| {
            let [tl, _, br, _] = icon.corners;
            [tl[0], tl[1], br[0] - tl[0], br[1] - tl[1]]
        })
        .collect()
}

fn picture(width: u32, height: u32) -> Option<Picture> {
    Some(Picture {
        sprite: 9,
        width,
        height,
    })
}

#[test]
fn the_picture_is_fitted_inside_the_frame() {
    let mut run = Run::new(VideoPlayer::new(ID).fullscreen(true));
    // 16:9 into 320 × 180 fills it.
    let mut wide = Fake {
        picture: picture(1920, 1080),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut wide, &away());
    assert_eq!(images(&list, 9), [[0.0, 0.0, 320.0, 180.0]]);
    // 4:3 is pillarboxed.
    let mut square = Fake {
        picture: picture(640, 480),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut square, &away());
    assert_eq!(images(&list, 9), [[40.0, 0.0, 240.0, 180.0]]);
}

#[test]
fn its_height_is_the_pictures_and_the_toolbars() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let full = VideoPlayer::new(ID).fullscreen(true);
    assert_eq!(
        full.height_for(320.0, 16.0 / 9.0, &s),
        (180.0 + toolbar()).round()
    );
    // The border is inside the width and adds to the height.
    let framed = VideoPlayer::new(ID);
    assert_eq!(
        framed.height_for(322.0, 16.0 / 9.0, &s),
        (180.0 + toolbar() + 2.0).round()
    );
}

#[test]
fn its_width_for_a_height_is_the_widest_that_fits_in_it() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    for player in [VideoPlayer::new(ID), VideoPlayer::new(ID).fullscreen(true)] {
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0, 2.39] {
            for height in [150.0, 300.5, 641.0] {
                let width = player.width_for(height, aspect, &s);
                assert_eq!(width, width.round(), "whole pixels");
                assert!(player.height_for(width, aspect, &s) <= height);
                assert!(player.height_for(width + 1.0, aspect, &s) > height);
            }
        }
        // Too short for any picture, or no picture shape: no width.
        assert_eq!(player.width_for(10.0, 16.0 / 9.0, &s), 0.0);
        assert_eq!(player.width_for(300.0, 0.0, &s), 0.0);
        // A panorama, where a pixel of height is many of width.
        let width = player.width_for(300.0, 50.0, &s);
        assert!(player.height_for(width, 50.0, &s) <= 300.0);
        assert!(player.height_for(width + 1.0, 50.0, &s) > 300.0);
    }
}

#[test]
fn at_its_narrowest_its_keys_and_the_time_are_side_by_side() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let player = VideoPlayer::new(ID);
    let mut measure = DrawList::new();
    let width = player.min_width(&s, &mut measure);
    for speed in SPEEDS {
        let mut source = Fake {
            speed,
            ..Fake::default()
        };
        let mut run = Run::new(player);
        run.rect = Rect::new(0.0, 0.0, width, player.height_for(width, 16.0 / 9.0, &s));
        let (list, _) = run.frame(&mut source, &away());
        let scope = |name: &str| {
            list.debug_scopes()
                .iter()
                .find(|scope| scope.name == name)
                .and_then(|scope| scope.declared)
                .unwrap_or_else(|| panic!("no {name} drawn"))
        };
        let (volume, speed_key) = (scope("Volume"), scope("Speed"));
        let time: f32 = ["0:00", " / 1:00"]
            .iter()
            .map(|text| s.mono_width(&mut measure, text, TextSize::Dense))
            .sum();
        assert!(
            volume.right() + time <= speed_key.x,
            "{speed}: {volume:?} {speed_key:?}"
        );
        assert!(speed_key.right() <= width, "{speed}");
    }
}

#[test]
fn before_a_picture_the_frame_shows_the_time() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake {
        time: Duration::from_secs(75),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut source, &away());
    // The placeholder's, then the readout's.
    assert_eq!(texts(&list)[0], "1:15");
    assert!(images(&list, 9).is_empty());
}

#[test]
fn the_time_reads_where_it_is_of_how_long() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake {
        time: Duration::from_secs(5),
        picture: picture(16, 9),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut source, &away());
    let texts = texts(&list);
    let at = texts.iter().position(|&t| t == "0:05").expect("the time");
    assert_eq!(texts[at + 1], " / 1:00");
}

#[test]
fn a_click_on_the_frame_plays_and_a_double_click_asks_for_full_screen() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    let output = run.click(&mut source, (100.0, 50.0));
    assert_eq!(source.told(), ["play"]);
    assert!(!output.toggle_fullscreen);
    let (_, output) = run.frame(
        &mut source,
        &InputState {
            mouse_down: true,
            mouse_clicked: true,
            mouse_double_clicked: true,
            ..pointer((100.0, 50.0))
        },
    );
    assert_eq!(source.told(), ["pause"], "the second click of the two");
    assert!(output.toggle_fullscreen);
}

#[test]
fn keys_work_while_the_player_has_the_keyboard() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake {
        time: Duration::from_secs(10),
        ..Fake::default()
    };
    // Unfocused, nothing.
    run.press_key(&mut source, |i| i.key_space = true);
    assert!(source.told().is_empty());

    run.click(&mut source, (100.0, 50.0));
    assert_eq!(source.told(), ["play"]);
    run.press_key(&mut source, |i| i.key_space = true);
    assert_eq!(source.told(), ["pause"]);
    run.press_key(&mut source, |i| letter(i, 'K'));
    assert_eq!(source.told(), ["play"]);
    run.press_key(&mut source, |i| i.nav.right = true);
    assert_eq!(source.told(), ["seek 15"]);
    source.time = Duration::from_secs(3);
    run.press_key(&mut source, |i| i.nav.left = true);
    assert_eq!(source.told(), ["seek 0"], "held at the start");
    source.time = Duration::from_secs(58);
    run.press_key(&mut source, |i| i.nav.right = true);
    assert_eq!(source.told(), ["seek 60"], "held at the end");
    run.press_key(&mut source, |i| letter(i, 'M'));
    assert_eq!(source.told(), ["muted true"]);

    let mut input = away();
    letter(&mut input, 'F');
    let (_, output) = run.frame(&mut source, &input);
    assert!(output.toggle_fullscreen);
}

#[test]
fn its_letters_work_from_its_toolbar_keys_and_space_stays_the_keys_own() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    run.focus.focus(ID + 3);
    run.frame(&mut source, &away());
    run.press_key(&mut source, |i| letter(i, 'K'));
    assert_eq!(source.told(), ["play"]);
    run.press_key(&mut source, |i| letter(i, 'M'));
    assert_eq!(source.told(), ["muted true"]);
    // Space presses the focused key, and doesn't also play or pause.
    run.press_key(&mut source, |i| {
        i.key_space = true;
        crate::nav::map_keyboard(i);
    });
    assert_eq!(source.told(), ["muted false"]);
    let mut input = away();
    letter(&mut input, 'F');
    let (_, output) = run.frame(&mut source, &input);
    assert!(output.toggle_fullscreen);

    // Another widget's focus: nothing.
    run.focus.focus(ID + 7);
    run.frame(&mut source, &away());
    run.press_key(&mut source, |i| letter(i, 'K'));
    assert!(source.told().is_empty());
}

#[test]
fn it_takes_as_many_focus_ids_as_it_says() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    // Focus on an id nothing draws goes at the end of a frame.
    let kept = |run: &mut Run, source: &mut Fake, id| {
        run.focus.focus(id);
        for _ in 0..2 {
            let input = away();
            run.focus.begin_frame(&input);
            run.frame(source, &input);
            run.focus.end_frame(None);
        }
        run.focus.is_focused(id)
    };
    for id in ID..ID + VideoPlayer::FOCUS_IDS {
        assert!(kept(&mut run, &mut source, id), "{id}");
    }
    assert!(!kept(&mut run, &mut source, ID + VideoPlayer::FOCUS_IDS));
}

#[test]
fn its_letters_leave_the_apps_shortcuts_alone() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    run.click(&mut source, (100.0, 50.0));
    assert_eq!(source.told(), ["play"]);
    let held: [fn(&mut InputState); 2] = [|i| i.ctrl_pressed = true, |i| i.alt_down = true];
    for (modifier, held) in held.into_iter().enumerate() {
        for key in ['K', 'M', 'F'] {
            let mut input = away();
            held(&mut input);
            letter(&mut input, key);
            let (_, output) = run.frame(&mut source, &input);
            assert!(source.told().is_empty(), "{modifier}: {key}");
            assert!(!output.toggle_fullscreen, "{modifier}: {key}");
        }
    }
}

#[test]
fn a_video_that_cant_play_says_so_and_takes_no_play() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake {
        state: PlayState::Error,
        error: Some("no decoder for this".into()),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut source, &away());
    let texts = texts(&list);
    assert!(texts.contains(&"Can’t play this video"), "{texts:?}");
    assert!(texts.contains(&"no decoder for this"), "{texts:?}");
    let times = texts.iter().filter(|&&t| t == "0:00").count();
    assert_eq!(
        times, 1,
        "the readout's, not the placeholder's under the words"
    );
    run.click(&mut source, (100.0, 50.0));
    let timeline = run.find(&mut source, "Timeline");
    run.click(&mut source, timeline);
    let play = run.find(&mut source, "PlayButton Error");
    run.click(&mut source, play);
    assert!(source.told().is_empty());
}

#[test]
fn the_toolbar_keys_play_mute_change_speed_and_ask_for_full_screen() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();

    let play = run.find(&mut source, "PlayButton Paused");
    run.click(&mut source, play);
    assert_eq!(source.told(), ["play"]);

    let mute = run.find(&mut source, "IconKey SpeakerHigh");
    run.click(&mut source, mute);
    assert_eq!(source.told(), ["muted true"]);
    let unmute = run.find(&mut source, "IconKey SpeakerX");
    run.click(&mut source, unmute);
    assert_eq!(source.told(), ["muted false"]);

    let speed = run.find(&mut source, "Speed");
    run.click(&mut source, speed);
    assert_eq!(source.told(), ["speed 1.5"]);

    let full = run.find(&mut source, "IconKey CornersOut");
    assert!(run.click(&mut source, full).toggle_fullscreen);

    run.player = run.player.fullscreen(true);
    run.find(&mut source, "IconKey CornersIn");
}

#[test]
fn unmuting_at_no_volume_comes_back_at_half() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake {
        volume: 0.0,
        muted: true,
        ..Fake::default()
    };
    let unmute = run.find(&mut source, "IconKey SpeakerX");
    run.click(&mut source, unmute);
    assert_eq!(source.told(), ["muted false", "volume 0.5"]);
}

#[test]
fn the_speaker_shows_the_volume_as_heard() {
    let cases = [
        (1.0, false, PhosphorIcon::SpeakerHigh),
        (0.5, false, PhosphorIcon::SpeakerHigh),
        (0.3, false, PhosphorIcon::SpeakerLow),
        (0.0, false, PhosphorIcon::SpeakerNone),
        (0.8, true, PhosphorIcon::SpeakerX),
    ];
    for (volume, muted, icon) in cases {
        let source = Fake {
            volume,
            muted,
            ..Fake::default()
        };
        assert_eq!(speaker(&source), icon, "{volume} {muted}");
    }
}

#[test]
fn the_volume_scrubber_sets_the_volume_and_mutes_at_none() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    let (x, y) = run.find(&mut source, "Volume");
    // Its middle is half volume.
    run.click(&mut source, (x, y));
    assert_eq!(source.told(), ["volume 0.5"]);
    // Its left end is none, which mutes.
    run.click(&mut source, (x - VOLUME * 0.5, y));
    assert_eq!(source.told(), ["volume 0", "muted true"]);
    // A muted player's volume reads as none; setting one unmutes.
    run.click(&mut source, (x + VOLUME * 0.25, y));
    assert_eq!(source.told(), ["volume 0.75", "muted false"]);
}

#[test]
fn a_drag_on_the_timeline_scrubs_live_and_seeks_exactly_where_let_go() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    let (x, y) = run.find(&mut source, "Timeline");
    run.frame(
        &mut source,
        &InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..pointer((x, y))
        },
    );
    let half = source.told();
    assert_eq!(half.len(), 1);
    let at: f64 = half[0].strip_prefix("scrub ").unwrap().parse().unwrap();
    assert!((at - 30.0).abs() < 0.01, "the middle of a minute: {at}");
    run.frame(
        &mut source,
        &InputState {
            mouse_down: true,
            ..pointer((WIDTH, y))
        },
    );
    assert_eq!(source.told(), ["scrub 60"]);
    // A scrub may land short of where it was asked to (on a keyframe).
    source.time = Duration::from_secs(58);
    run.frame(&mut source, &pointer((WIDTH, y)));
    assert_eq!(source.told(), ["seek 60"]);
}

#[test]
fn a_key_on_the_timeline_seeks_exactly() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake::default();
    run.focus.request(ID + 1);
    run.press_key(&mut source, |i| i.key_end = true);
    assert_eq!(source.told(), ["seek 60"]);
}

#[test]
fn it_asks_for_a_frame_when_its_source_says_something_changes() {
    let mut run = Run::new(VideoPlayer::new(ID));
    let mut source = Fake {
        next_change: Some(Duration::from_millis(40)),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut source, &away());
    assert_eq!(list.next_repaint(), Some(0.04));
    source.next_change = None;
    let (list, _) = run.frame(&mut source, &away());
    assert_eq!(list.next_repaint(), None);
}

#[test]
fn it_drops_a_shadow_unless_full_screen() {
    let mut source = Fake::default();
    // Its own, not its keys'.
    let drops = |player: VideoPlayer<'static>, source: &mut Fake| {
        let mut run = Run::new(player);
        let whole = run.rect;
        let (list, _) = run.frame(source, &away());
        list.shadow_instances()
            .filter(|shadow| shadow.element_rect[2] == whole.width)
            .map(|shadow| shadow.color)
            .collect::<Vec<_>>()
    };
    assert_eq!(drops(VideoPlayer::new(ID), &mut source), [DROP]);
    assert!(drops(VideoPlayer::new(ID).fullscreen(true), &mut source).is_empty());
}

#[test]
fn a_title_is_drawn_over_the_picture() {
    let mut run = Run::new(VideoPlayer::new(ID).title("intro.mp4"));
    let mut source = Fake {
        picture: picture(16, 9),
        ..Fake::default()
    };
    let (list, _) = run.frame(&mut source, &away());
    let title = list
        .texts
        .iter()
        .find(|t| t.content == "intro.mp4")
        .expect("the title");
    assert!(title.y < 180.0, "over the frame");
}
