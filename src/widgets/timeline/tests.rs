use std::collections::HashSet;

use super::model::{all_keys, delete_keys, move_keys, next_key, prev_key, tick_steps};
use super::*;
use crate::{FocusState, InputState, Theme};

const ID: u64 = 7;
const RECT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 800.0,
    height: 300.0,
};

const DOOR: TrackId = 1;
const POSITION: TrackId = 2;
const ROTATION: TrackId = 3;
const LIGHT: TrackId = 4;

/// A group with two tracks, then a track of its own.
fn sample() -> Vec<Track> {
    vec![
        Track::group(DOOR, "Door"),
        Track::new(POSITION, "Position").depth(1).keys([
            Keyframe::new(0),
            Keyframe::new(12).interp(Interp::Linear),
            Keyframe::new(24).interp(Interp::Hold),
            Keyframe::new(48),
        ]),
        Track::new(ROTATION, "Rotation")
            .depth(1)
            .keys([Keyframe::new(12), Keyframe::new(36)]),
        Track::new(LIGHT, "Light").keys([Keyframe::new(6), Keyframe::new(30)]),
    ]
}

fn keys(track: &Track) -> Vec<u32> {
    track.keys.iter().map(|k| k.t).collect()
}

fn set(refs: &[(TrackId, u32)]) -> HashSet<KeyRef> {
    refs.iter().map(|&(id, t)| KeyRef::new(id, t)).collect()
}

struct Rig {
    tracks: Vec<Track>,
    markers: Vec<Marker>,
    state: TimelineState,
    drag: DragCapture,
    focus: FocusState,
    theme: Theme,
    timeline: Timeline,
}

impl Rig {
    fn new() -> Self {
        let mut rig = Self {
            tracks: sample(),
            markers: vec![Marker::new(40, "door opens")],
            state: TimelineState::new(),
            drag: DragCapture::new(),
            focus: FocusState::new(),
            theme: Theme::default(),
            timeline: Timeline::new(ID).length(120),
        };
        rig.frame(&away());
        rig
    }

    fn frame(&mut self, input: &InputState) -> (DrawList, TimelineOutput) {
        let mut list = DrawList::new();
        self.focus.begin_frame(input);
        let out = {
            let mut ctx =
                DrawContext::new(&mut list, &mut self.focus, &self.theme, input, 800.0, 600.0);
            self.timeline.draw(
                RECT,
                &mut self.tracks,
                &mut self.markers,
                &mut self.state,
                &mut self.drag,
                &mut ctx,
            )
        };
        self.focus.end_frame(None);
        (list, out)
    }

    fn layout(&self) -> Layout {
        let s = StyleResolver::new(&self.theme);
        self.timeline
            .layout(RECT, &self.state, self.state.rows.rows.len(), &s)
    }

    /// The centre of the key at frame `t` on row `row`.
    fn key(&self, row: usize, t: u32) -> (f32, f32) {
        let l = self.layout();
        (l.xc(t), l.row_y(row) + ROW * 0.5)
    }

    /// Press at `from`, move to `to`, release there; the edits of all three
    /// frames.
    fn drag_to(&mut self, from: (f32, f32), to: (f32, f32)) -> Vec<TimelineEdit> {
        let mut edits = self.frame(&press(from.0, from.1)).1.edits;
        edits.extend(self.frame(&held(to.0, to.1)).1.edits);
        edits.extend(self.frame(&release(to.0, to.1)).1.edits);
        edits
    }

    fn click(&mut self, at: (f32, f32)) -> TimelineOutput {
        let out = self.frame(&press(at.0, at.1)).1;
        let more = self.frame(&release(at.0, at.1)).1;
        TimelineOutput {
            edits: out.edits.into_iter().chain(more.edits).collect(),
            frame_changed: out.frame_changed || more.frame_changed,
            hovered: more.hovered,
        }
    }

    fn track(&self, id: TrackId) -> &Track {
        self.tracks.iter().find(|t| t.id == id).unwrap()
    }

    /// Focus the timeline, as a click on its corner would.
    fn focus(&mut self) {
        let l = self.layout();
        self.click((l.corner.x + 4.0, l.corner.y + 4.0));
        assert!(self.focus.is_focused(ID));
    }
}

fn at(x: f32, y: f32) -> InputState {
    InputState {
        mouse_x: x,
        mouse_y: y,
        ..Default::default()
    }
}

fn away() -> InputState {
    at(790.0, 590.0)
}

fn press(x: f32, y: f32) -> InputState {
    InputState {
        mouse_down: true,
        mouse_clicked: true,
        ..at(x, y)
    }
}

fn held(x: f32, y: f32) -> InputState {
    InputState {
        mouse_down: true,
        ..at(x, y)
    }
}

fn release(x: f32, y: f32) -> InputState {
    InputState {
        mouse_released: true,
        ..at(x, y)
    }
}

fn typed(key: Key) -> InputState {
    let mut input = away();
    input.keys.key_event(key, true);
    input
}

// ---- Model ----

#[test]
fn track_keys_are_sorted_and_one_per_frame() {
    let track = Track::new(1, "x").keys([
        Keyframe::new(9),
        Keyframe::new(3).interp(Interp::Linear),
        Keyframe::new(3).interp(Interp::Hold),
    ]);
    assert_eq!(keys(&track), [3, 9]);
    assert_eq!(
        track.key_at(3).unwrap().interp,
        Interp::Hold,
        "the last wins"
    );
}

#[test]
fn a_group_without_keys_sums_up_its_tracks() {
    let tracks = sample();
    let mut rows = Rows::default();
    rows.build(&tracks, &HashSet::new());
    assert_eq!(rows.rows.len(), 4);
    let door = &rows.rows[0];
    assert!(door.summary);
    assert_eq!(door.kids, 1..3);
    assert_eq!(&rows.times[door.times.clone()], [0, 12, 24, 36, 48]);
    assert!(!rows.rows[3].summary, "Light is not a group");
}

#[test]
fn a_collapsed_group_hides_its_tracks() {
    let tracks = sample();
    let mut rows = Rows::default();
    rows.build(&tracks, &[DOOR].into_iter().collect());
    let shown: Vec<usize> = rows.rows.iter().map(|r| r.track).collect();
    assert_eq!(shown, [0, 3]);
    assert!(rows.rows[0].collapsed);
    assert!(rows.rows[0].summary, "still sums up what it hides");
}

#[test]
fn a_summary_key_stands_for_the_keys_beneath() {
    let tracks = sample();
    let mut rows = Rows::default();
    rows.build(&tracks, &HashSet::new());
    let mut out = Vec::new();
    rows.keys_at(&rows.rows[0], &tracks, 12, &mut out);
    assert_eq!(out, [KeyRef::new(POSITION, 12), KeyRef::new(ROTATION, 12)]);
    rows.keys_at(&rows.rows[0], &tracks, 36, &mut out);
    assert_eq!(out, [KeyRef::new(ROTATION, 36)]);
}

#[test]
fn moving_keys_replaces_what_they_land_on_and_follows_the_selection() {
    let mut tracks = sample();
    let mut selection = set(&[(POSITION, 0), (POSITION, 12)]);
    let edit = move_keys(&mut tracks, &mut selection, 12).unwrap();
    assert_eq!(keys(&tracks[1]), [12, 24, 48]);
    // 12 (linear) moved onto 24 (hold) and replaced it; 0 moved onto 12.
    assert_eq!(tracks[1].key_at(24).unwrap().interp, Interp::Linear);
    assert_eq!(tracks[1].key_at(12).unwrap().interp, Interp::Ease);
    assert_eq!(selection, set(&[(POSITION, 12), (POSITION, 24)]));
    assert_eq!(
        edit,
        TimelineEdit::KeysMoved {
            delta: 12,
            keys: vec![KeyRef::new(POSITION, 0), KeyRef::new(POSITION, 12)],
            replaced: vec![(POSITION, Keyframe::new(24).interp(Interp::Hold))],
        }
    );
}

#[test]
fn locked_tracks_keep_their_keys() {
    let mut tracks = sample();
    tracks[3].locked = true;
    let mut selection = set(&[(LIGHT, 6), (ROTATION, 12)]);
    move_keys(&mut tracks, &mut selection, 1).unwrap();
    assert_eq!(keys(&tracks[3]), [6, 30]);
    assert_eq!(keys(&tracks[2]), [13, 36]);
    assert_eq!(selection, set(&[(LIGHT, 6), (ROTATION, 13)]));

    let edit = delete_keys(&mut tracks, &mut selection).unwrap();
    assert_eq!(
        edit,
        TimelineEdit::KeysDeleted {
            keys: vec![(ROTATION, Keyframe::new(13))]
        }
    );
    assert_eq!(keys(&tracks[3]), [6, 30]);
    assert_eq!(
        selection,
        set(&[(LIGHT, 6)]),
        "the locked key stays selected"
    );

    let mut locked_only = set(&[(LIGHT, 6)]);
    assert_eq!(move_keys(&mut tracks, &mut locked_only, 3), None);
}

#[test]
fn previous_and_next_key_look_across_every_track() {
    let tracks = sample();
    assert_eq!(prev_key(&tracks, 12), Some(6));
    assert_eq!(next_key(&tracks, 12), Some(24));
    assert_eq!(prev_key(&tracks, 0), None);
    assert_eq!(next_key(&tracks, 48), None);
    assert_eq!(all_keys(&tracks).len(), 8);
}

#[test]
fn ruler_steps_follow_the_zoom() {
    // 6 px a frame: labels every 10 frames, a grid line every frame.
    assert_eq!(tick_steps(6.0), [10, 2, 1]);
    // 1 px a frame: labels every 100, ticks every 10.
    assert_eq!(tick_steps(1.0), [100, 10, 10]);
    // 40 px a frame: every frame is labelled.
    assert_eq!(tick_steps(40.0), [2, 1, 1]);
}

#[test]
fn timecode_is_minutes_seconds_frames() {
    assert_eq!(timecode(0, 24), "00:00:00");
    assert_eq!(timecode(50, 24), "00:02:02");
    assert_eq!(timecode(24 * 61 + 5, 24), "01:01:05");
}

// ---- Pointer ----

#[test]
fn clicking_a_key_selects_it_and_shift_toggles() {
    let mut rig = Rig::new();
    rig.click(rig.key(1, 12));
    assert_eq!(rig.state.selection, set(&[(POSITION, 12)]));
    let shift_click = |rig: &mut Rig, (x, y): (f32, f32)| {
        let mut input = press(x, y);
        input.shift_pressed = true;
        rig.frame(&input);
        rig.frame(&release(x, y));
    };
    let (rotation_36, position_12) = (rig.key(2, 36), rig.key(1, 12));
    shift_click(&mut rig, rotation_36);
    assert_eq!(rig.state.selection, set(&[(POSITION, 12), (ROTATION, 36)]));
    shift_click(&mut rig, position_12);
    assert_eq!(rig.state.selection, set(&[(ROTATION, 36)]));
    assert_eq!(keys(rig.track(POSITION)), [0, 12, 24, 48], "nothing moved");
}

#[test]
fn clicking_a_summary_key_selects_the_keys_beneath() {
    let mut rig = Rig::new();
    rig.click(rig.key(0, 12));
    assert_eq!(rig.state.selection, set(&[(POSITION, 12), (ROTATION, 12)]));
}

#[test]
fn dragging_a_key_moves_the_selection_by_whole_frames() {
    let mut rig = Rig::new();
    let ppf = rig.layout().ppf;
    let from = rig.key(0, 12);
    let edits = rig.drag_to(from, (from.0 + 3.4 * ppf, from.1));
    assert_eq!(keys(rig.track(POSITION)), [0, 15, 24, 48]);
    assert_eq!(keys(rig.track(ROTATION)), [15, 36]);
    assert!(matches!(
        edits.as_slice(),
        [TimelineEdit::KeysMoved { delta: 3, .. }]
    ));
    assert_eq!(rig.state.selection, set(&[(POSITION, 15), (ROTATION, 15)]));
    assert!(rig.drag.is_free());
}

#[test]
fn a_key_drag_stops_at_frame_zero_and_the_end() {
    let mut rig = Rig::new();
    let from = rig.key(3, 6);
    rig.drag_to(from, (from.0 - 400.0, from.1));
    assert_eq!(keys(rig.track(LIGHT)), [0, 30]);
    rig.frame(&away());
    let from = rig.key(3, 30);
    rig.drag_to(from, (from.0 + 2000.0, from.1));
    assert_eq!(keys(rig.track(LIGHT)), [0, 120]);
}

#[test]
fn escape_cancels_a_key_drag() {
    let mut rig = Rig::new();
    let from = rig.key(3, 6);
    rig.frame(&press(from.0, from.1));
    rig.frame(&held(from.0 + 60.0, from.1));
    let mut esc = held(from.0 + 60.0, from.1);
    esc.key_escape = true;
    let out = rig.frame(&esc).1;
    assert!(out.edits.is_empty());
    assert_eq!(keys(rig.track(LIGHT)), [6, 30]);
    assert!(rig.drag.is_free());
}

#[test]
fn a_locked_tracks_keys_select_but_do_not_move() {
    let mut rig = Rig::new();
    rig.tracks[3].locked = true;
    let from = rig.key(3, 6);
    let edits = rig.drag_to(from, (from.0 + 60.0, from.1));
    assert!(edits.is_empty());
    assert_eq!(keys(rig.track(LIGHT)), [6, 30]);
    assert_eq!(rig.state.selection, set(&[(LIGHT, 6)]));
}

#[test]
fn a_read_only_timeline_selects_but_does_not_edit() {
    let mut rig = Rig::new();
    rig.timeline = rig.timeline.editable(false);
    let from = rig.key(3, 6);
    assert!(rig.drag_to(from, (from.0 + 60.0, from.1)).is_empty());
    assert_eq!(rig.state.selection, set(&[(LIGHT, 6)]));
    let (x, y) = rig.key(3, 50);
    let mut double = press(x, y);
    double.mouse_double_clicked = true;
    assert!(rig.frame(&double).1.edits.is_empty());
}

#[test]
fn a_marquee_selects_the_keys_it_covers() {
    let mut rig = Rig::new();
    let a = rig.key(1, 10);
    let b = rig.key(2, 13);
    rig.frame(&press(a.0, a.1 - 4.0));
    rig.frame(&held(b.0, b.1 + 4.0));
    assert_eq!(rig.state.selection, set(&[(POSITION, 12), (ROTATION, 12)]));
    rig.frame(&release(b.0, b.1 + 4.0));
    assert_eq!(rig.state.selection, set(&[(POSITION, 12), (ROTATION, 12)]));

    // A click on empty lane space clears it.
    let empty = rig.key(3, 80);
    rig.click(empty);
    assert!(rig.state.selection.is_empty());
}

#[test]
fn double_clicking_a_lane_adds_a_key() {
    let mut rig = Rig::new();
    let (x, y) = rig.key(3, 50);
    let mut double = press(x, y);
    double.mouse_double_clicked = true;
    let out = rig.frame(&double).1;
    assert_eq!(
        out.edits,
        [TimelineEdit::KeyAdded {
            key: KeyRef::new(LIGHT, 50)
        }]
    );
    assert_eq!(keys(rig.track(LIGHT)), [6, 30, 50]);
    assert_eq!(rig.state.selection, set(&[(LIGHT, 50)]));
    // Not on a summary row.
    let (x, y) = rig.key(0, 70);
    let mut double = press(x, y);
    double.mouse_double_clicked = true;
    rig.frame(&release(0.0, 0.0));
    assert!(rig.frame(&double).1.edits.is_empty());
}

#[test]
fn the_ruler_scrubs_the_playhead() {
    let mut rig = Rig::new();
    let l = rig.layout();
    let y = l.ruler.bottom() - 6.0;
    let out = rig.frame(&press(l.xc(20), y)).1;
    assert_eq!(rig.state.frame, 20);
    assert!(out.frame_changed);
    rig.frame(&held(l.xc(33), y));
    assert_eq!(rig.state.frame, 33);
    rig.frame(&release(l.xc(33), y));
    assert!(rig.drag.is_free());
}

#[test]
fn a_marker_drags_to_a_frame_and_a_click_jumps_to_it() {
    let mut rig = Rig::new();
    let l = rig.layout();
    let flag = (l.xc(40), l.ruler.y + 6.0);
    let edits = rig.drag_to(flag, (flag.0 + 5.0 * l.ppf, flag.1));
    assert_eq!(
        edits,
        [TimelineEdit::MarkerMoved {
            index: 0,
            from: 40,
            to: 45
        }]
    );
    assert_eq!(rig.markers[0].t, 45);
    assert_eq!(rig.state.marker_selected, Some(0));
    assert_eq!(rig.state.frame, 0, "moving a marker never scrubs");

    rig.click((l.xc(45), flag.1));
    assert_eq!(rig.state.frame, 45);
}

#[test]
fn double_clicking_a_marker_renames_it() {
    let mut rig = Rig::new();
    let l = rig.layout();
    let flag = (l.xc(40), l.ruler.y + 6.0);
    rig.click(flag);
    let mut double = press(flag.0, flag.1);
    double.mouse_double_clicked = true;
    rig.frame(&double);
    rig.frame(&release(flag.0, flag.1));
    assert_eq!(rig.state.renaming(), Some(0));
    rig.state.rename.as_mut().unwrap().input.value = "  door shut ".into();
    let mut enter = away();
    enter.enter_pressed = true;
    let out = rig.frame(&enter).1;
    assert_eq!(
        out.edits,
        [TimelineEdit::MarkerRenamed {
            index: 0,
            from: "door opens".into(),
            to: "door shut".into()
        }]
    );
    assert_eq!(rig.state.renaming(), None);
}

#[test]
fn a_range_bracket_drags_and_reports_the_change() {
    let mut rig = Rig::new();
    rig.state.range = Some([10, 60]);
    rig.frame(&away());
    let l = rig.layout();
    let y = l.ruler.bottom() - 3.0;
    let edits = rig.drag_to((l.x(61.0), y), (l.xc(80), y));
    assert_eq!(
        edits,
        [TimelineEdit::RangeChanged {
            from: [10, 60],
            to: [10, 80]
        }]
    );
    // The start can't pass the end.
    let l = rig.layout();
    rig.drag_to((l.x(10.0), y), (l.xc(110), y));
    assert_eq!(rig.state.range, Some([79, 80]));
}

#[test]
fn ctrl_wheel_zooms_around_the_pointer() {
    let mut rig = Rig::new();
    let l = rig.layout();
    let x = l.xc(30);
    let before = l.frame_at(x);
    let mut wheel = at(x, l.lanes.y + 5.0);
    wheel.ctrl_pressed = true;
    wheel.scroll_delta = 1.0;
    rig.frame(&wheel);
    let l = rig.layout();
    assert!((l.ppf - TIMELINE_DEFAULT_ZOOM * WHEEL_ZOOM).abs() < 1e-4);
    assert!((l.frame_at(x) - before).abs() < 1e-3);
}

#[test]
fn the_wheel_scrolls_rows_and_shift_scrolls_time() {
    let mut rig = Rig::new();
    rig.tracks
        .extend((10..40).map(|i| Track::new(i, format!("Track {i}"))));
    rig.frame(&away());
    let l = rig.layout();
    let mut wheel = at(l.lanes.x + 50.0, l.lanes.y + 50.0);
    wheel.scroll_delta = -2.0;
    rig.frame(&wheel);
    assert_eq!(rig.state.scroll, [0.0, 2.0 * WHEEL_PX]);
    wheel.shift_pressed = true;
    rig.frame(&wheel);
    assert_eq!(rig.state.scroll, [2.0 * WHEEL_PX, 2.0 * WHEEL_PX]);
}

#[test]
fn a_group_caret_collapses_it() {
    let mut rig = Rig::new();
    let l = rig.layout();
    rig.click((l.labels.x + 6.0 + 4.0, l.row_y(0) + ROW * 0.5));
    assert!(rig.state.collapsed.contains(&DOOR));
    assert_eq!(rig.state.rows.rows.len(), 2);
}

#[test]
fn hovering_a_label_offers_mute_and_lock() {
    let mut rig = Rig::new();
    let l = rig.layout();
    let cy = l.row_y(3) + ROW * 0.5;
    let lock = (l.labels.right() - 4.0 - TRACK_KEY * 0.5, cy);
    let out = rig.click(lock);
    assert_eq!(
        out.edits,
        [TimelineEdit::TrackLocked {
            track: LIGHT,
            locked: true
        }]
    );
    assert!(rig.track(LIGHT).locked);
    let mute = (lock.0 - 5.0 - TRACK_KEY, cy);
    rig.click(mute);
    assert!(rig.track(LIGHT).muted);
}

// ---- Playback ----

#[test]
fn playback_advances_with_frame_time_and_loops_in_the_range() {
    let mut rig = Rig::new();
    rig.state.range = Some([10, 20]);
    rig.state.frame = 18;
    rig.state.playing = true;
    let mut tick = away();
    tick.frame_dt = 1.0 / 24.0;
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 19);
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 20);
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 10, "wraps to the range start");
    tick.frame_dt = 0.5; // 12 frames: 10 → 22, wraps to 11.
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 11);
}

#[test]
fn playback_without_loop_stops_at_the_end() {
    let mut rig = Rig::new();
    rig.state.looping = false;
    rig.state.frame = 118;
    rig.state.playing = true;
    let mut tick = away();
    tick.frame_dt = 0.5;
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 120);
    assert!(!rig.state.playing);
}

#[test]
fn a_frame_set_while_playing_plays_on_from_there() {
    let mut rig = Rig::new();
    rig.state.playing = true;
    let mut tick = away();
    tick.frame_dt = 1.0 / 24.0;
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 1);
    rig.state.frame = 60;
    rig.frame(&tick);
    assert_eq!(rig.state.frame, 61);
}

// ---- Keyboard ----

#[test]
fn keys_step_jump_and_play_when_focused() {
    let mut rig = Rig::new();
    // Not focused: keys do nothing.
    let mut right = away();
    right.key_right = true;
    rig.frame(&right);
    assert_eq!(rig.state.frame, 0);

    rig.focus();
    rig.frame(&right);
    assert_eq!(rig.state.frame, 1);
    right.shift_pressed = true;
    rig.frame(&right);
    assert_eq!(rig.state.frame, 11);
    rig.frame(&typed(Key::Char('.')));
    assert_eq!(rig.state.frame, 12);
    rig.frame(&typed(Key::Char('.')));
    assert_eq!(rig.state.frame, 24);
    rig.frame(&typed(Key::Char(',')));
    assert_eq!(rig.state.frame, 12);
    let mut end = away();
    end.key_end = true;
    rig.frame(&end);
    assert_eq!(rig.state.frame, 120);
    let mut space = away();
    space.key_space = true;
    rig.frame(&space);
    assert!(rig.state.playing);
}

#[test]
fn delete_removes_the_selected_keys_then_the_selected_marker() {
    let mut rig = Rig::new();
    rig.click(rig.key(3, 30));
    assert!(rig.focus.is_focused(ID));
    let mut delete = away();
    delete.key_delete = true;
    let out = rig.frame(&delete).1;
    assert_eq!(
        out.edits,
        [TimelineEdit::KeysDeleted {
            keys: vec![(LIGHT, Keyframe::new(30))]
        }]
    );
    let l = rig.layout();
    rig.click((l.xc(40), l.ruler.y + 6.0));
    let out = rig.frame(&delete).1;
    assert_eq!(
        out.edits,
        [TimelineEdit::MarkerDeleted {
            index: 0,
            marker: Marker::new(40, "door opens")
        }]
    );
    assert!(rig.markers.is_empty());
}

#[test]
fn m_adds_a_marker_at_the_playhead_and_names_it() {
    let mut rig = Rig::new();
    rig.focus();
    rig.state.frame = 30;
    let out = rig.frame(&typed(Key::Char('M'))).1;
    assert_eq!(out.edits, [TimelineEdit::MarkerAdded { index: 1 }]);
    assert_eq!(rig.markers[1], Marker::new(30, "Marker 2"));
    assert_eq!(rig.state.renaming(), Some(1));
    // The field takes focus; Escape keeps the default name.
    rig.frame(&away());
    assert!(rig.focus.is_focused(part_id(ID, RENAME_FIELD)));
    let mut esc = away();
    esc.key_escape = true;
    assert!(rig.frame(&esc).1.edits.is_empty());
    assert_eq!(rig.state.renaming(), None);
    assert_eq!(rig.markers[1].label, "Marker 2");
}

#[test]
fn ctrl_a_selects_every_key_and_escape_clears() {
    let mut rig = Rig::new();
    rig.focus();
    let mut all = away();
    all.key_select_all = true;
    rig.frame(&all);
    assert_eq!(rig.state.selection.len(), 8);
    let mut esc = away();
    esc.key_escape = true;
    rig.frame(&esc);
    assert!(rig.state.selection.is_empty());
}

#[test]
fn zoom_keys_and_fit() {
    let mut rig = Rig::new();
    rig.focus();
    rig.frame(&typed(Key::Char('=')));
    assert!((rig.state.zoom - TIMELINE_DEFAULT_ZOOM * ZOOM_STEP).abs() < 1e-4);
    rig.frame(&typed(Key::Char('-')));
    assert!((rig.state.zoom - TIMELINE_DEFAULT_ZOOM).abs() < 1e-4);

    rig.state.range = Some([20, 59]);
    let l = rig.layout();
    rig.timeline.fit(&l, &mut rig.state);
    let l = rig.layout();
    assert!((l.x(20.0) - (l.lanes.x + LPAD)).abs() < 1e-3);
    assert!((l.x(60.0) - (l.lanes.right() - LPAD)).abs() < 1e-3);
}

// ---- View ----

#[test]
fn the_playhead_stays_in_view() {
    let mut rig = Rig::new();
    rig.state.zoom = 20.0;
    rig.state.frame = 100;
    rig.frame(&away());
    let l = rig.layout();
    let x = l.xc(100);
    assert!(
        x > l.lanes.x && x < l.lanes.right(),
        "{x} not in {:?}",
        l.lanes
    );
}

#[test]
fn only_keys_on_screen_are_drawn() {
    // Count the instances of a sparse track and of one with a key on every
    // frame: the dense one adds only the keys that fit on screen.
    fn instances(keys: u32) -> usize {
        let mut rig = Rig::new();
        rig.timeline = rig.timeline.length(100_000);
        rig.tracks = vec![Track::new(1, "dense").keys((0..keys).map(Keyframe::new))];
        let (list, _) = rig.frame(&away());
        list.analytic_instances.len()
    }
    let few = instances(2);
    let many = instances(100_000);
    let on_screen = (800.0 - 176.0) / TIMELINE_DEFAULT_ZOOM;
    assert!(
        ((many - few) as f32) < on_screen * 3.0 + 20.0,
        "{many} instances for 100 000 keys vs {few} for 2"
    );
}
