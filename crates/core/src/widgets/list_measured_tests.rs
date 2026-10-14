//! A [`List`] drawing rows of different heights from a [`RowHeights`].

use super::*;
use crate::Theme;
use crate::widgets::row_heights::Row;

/// The view every test draws into: 100 px tall, 200 wide, at y = 50 so
/// screen and content positions differ.
const VIEW: Rect = Rect {
    x: 0.0,
    y: 50.0,
    width: 200.0,
    height: 100.0,
};

/// One drawn row: its index and where it was on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drawn {
    index: usize,
    y: f32,
    height: f32,
}

struct Rig {
    widget: List<'static>,
    state: ListState,
    heights: RowHeights,
    /// The height each row measures at; estimates start elsewhere.
    real: Vec<f32>,
    /// Rows the list asked to measure, in order.
    measured: Vec<usize>,
    width: f32,
}

impl Rig {
    /// `count` rows estimated at 10 px that measure at 20 + (i % 5) * 10.
    fn new(count: usize) -> Self {
        let mut heights = RowHeights::new();
        heights.extend((0..count).map(|i| Row::estimated(10.0).keyed(i as u64)));
        Self {
            widget: List::new(),
            state: ListState::new(),
            heights,
            real: (0..count).map(|i| 20.0 + (i % 5) as f32 * 10.0).collect(),
            measured: Vec::new(),
            width: VIEW.width,
        }
    }

    fn frame_with(&mut self, input: &mut InputState) -> (Vec<Drawn>, ListOutput) {
        crate::map_keyboard(input);
        let mut list = DrawList::new();
        let theme = Theme::default();
        let mut drawn = Vec::new();
        let real = &self.real;
        let measured = &mut self.measured;
        let rect = Rect {
            width: self.width,
            ..VIEW
        };
        let out = self.widget.draw_measured(
            rect,
            &mut self.heights,
            |row, _width, _list| {
                measured.push(row);
                real[row]
            },
            &mut self.state,
            &mut list,
            &StyleResolver::new(&theme),
            input,
            |list, cell, item| {
                // Cells come in content space; where they land on screen is
                // what the draw list's transform makes of them.
                let [_, y] = list.current_transform().transform_point([cell.x, cell.y]);
                drawn.push(Drawn {
                    index: item.index,
                    y,
                    height: cell.height,
                })
            },
        );
        (drawn, out)
    }

    fn frame(&mut self) -> Vec<Drawn> {
        self.frame_with(&mut InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..InputState::default()
        })
        .0
    }

    fn settle(&mut self) -> Vec<Drawn> {
        for _ in 0..120 {
            self.frame();
        }
        self.frame()
    }
}

/// The drawn rows sit edge to edge, at their heights, from the one covering
/// the top of the view to the one covering its bottom.
fn assert_in_place(drawn: &[Drawn], heights: &RowHeights) {
    assert!(!drawn.is_empty());
    let first = drawn[0];
    assert!(
        first.y <= VIEW.y && first.y + first.height > VIEW.y,
        "the first row covers the top: {first:?}"
    );
    for pair in drawn.windows(2) {
        assert_eq!(pair[1].index, pair[0].index + 1);
        assert_eq!(pair[1].y, pair[0].y + pair[0].height, "{pair:?}");
    }
    for row in drawn {
        assert_eq!(row.height, heights.height_of(row.index));
    }
    let last = drawn[drawn.len() - 1];
    assert!(last.y + last.height >= VIEW.bottom() || last.index == heights.len() - 1);
}

#[test]
fn rows_in_view_are_measured_and_drawn_at_their_heights() {
    let mut rig = Rig::new(1000);
    let drawn = rig.frame();
    // 20 + 30 + 40 + 50 = 140 > 100: four rows fill the view.
    assert_eq!(
        drawn.iter().map(|row| row.index).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_in_place(&drawn, &rig.heights);
    // The 10 px estimates put rows 0..10 in view at first, and two more
    // either side: what is measured is that, not the history.
    assert!(
        rig.measured.len() <= 13,
        "only the rows near the view: {:?}",
        rig.measured
    );
    assert!(rig.heights.is_measured(3) && !rig.heights.is_measured(50));
    let measured = rig.measured.len();
    rig.frame();
    assert_eq!(rig.measured.len(), measured, "a row is measured once");
}

#[test]
fn rows_above_getting_their_heights_leave_the_view_alone() {
    let mut rig = Rig::new(1000);
    rig.state.scroll_to_row(500, 7.0);
    let before = rig.frame();
    assert_eq!(before[0].index, 500);
    assert_eq!(before[0].y, VIEW.y - 7.0);
    // A background thread measures everything above.
    for row in 0..500 {
        let ticket = rig.heights.ticket(row);
        rig.heights.apply_measured(ticket, 99.0);
    }
    let after = rig.frame();
    assert_eq!(after, before);
    assert_eq!(rig.state.top_row(), (500, 7.0));
}

#[test]
fn a_page_put_in_front_keeps_the_reader_on_their_row() {
    let mut rig = Rig::new(100);
    rig.state.scroll_to_row(10, 5.0);
    let before = rig.frame();
    rig.heights
        .splice(0..0, (0..40).map(|i| Row::estimated(15.0).keyed(1000 + i)));
    rig.real.splice(0..0, std::iter::repeat_n(15.0, 40));
    let after = rig.frame();
    assert_eq!(after[0].index, 50);
    assert_eq!(after[0].y, before[0].y);
    assert_eq!(rig.state.top_row(), (50, 5.0));
}

#[test]
fn following_the_end_stays_at_the_end_as_rows_arrive_and_settle() {
    let mut rig = Rig::new(30);
    rig.state.follow_end();
    for _ in 0..5 {
        let drawn = rig.frame();
        let last = drawn[drawn.len() - 1];
        assert_eq!(last.index, rig.heights.len() - 1);
        assert_eq!(last.y + last.height, VIEW.bottom(), "the end at the bottom");
        rig.heights.push(Row::estimated(10.0));
        rig.real.push(60.0);
    }
}

#[test]
fn the_wheel_onto_rows_not_measured_yet_draws_them_in_place_that_frame() {
    let mut rig = Rig::new(1000);
    rig.frame();
    for _ in 0..10 {
        let (drawn, _) = rig.frame_with(&mut InputState {
            mouse_x: 10.0,
            mouse_y: 60.0,
            scroll_delta: -20.0,
            frame_dt: 1.0,
            ..InputState::default()
        });
        assert_in_place(&drawn, &rig.heights);
    }
    assert!(rig.state.top_row().0 > 20, "{:?}", rig.state.top_row());
}

#[test]
fn a_new_width_keeps_the_same_share_of_the_top_row() {
    let mut rig = Rig::new(100);
    rig.state.scroll_to_row(12, 10.0); // row 12 measures 40: a quarter in
    rig.frame();
    assert_eq!(rig.state.top_row(), (12, 10.0));
    // Half as wide, every row twice as tall.
    rig.width = 100.0;
    for height in &mut rig.real {
        *height *= 2.0;
    }
    let drawn = rig.frame();
    assert_eq!(rig.state.top_row(), (12, 20.0));
    assert_eq!(drawn[0].y, VIEW.y - 20.0);
    assert!(rig.heights.is_measured(12));
}

#[test]
fn the_keyboard_cursor_brings_a_tall_row_wholly_into_view() {
    let mut rig = Rig::new(100);
    rig.widget = List::new().focused(true);
    for _ in 0..4 {
        let mut down = InputState {
            key_down: true,
            ..InputState::default()
        };
        rig.frame_with(&mut down);
        rig.frame();
    }
    // The cursor is on row 3 (50 px); rows 0..3 are 90 px, so it ends at 140.
    assert_eq!(rig.state.cursor(), Some(3));
    let drawn = rig.settle();
    let row = drawn.iter().find(|row| row.index == 3).unwrap();
    assert_eq!(row.y + row.height, VIEW.bottom());
}

#[test]
fn a_click_lands_on_the_row_under_it_by_its_measured_height() {
    let mut rig = Rig::new(100);
    rig.frame();
    // Rows 0 and 1 are 20 and 30 px: y 50 + 45 is in row 1.
    let (_, out) = rig.frame_with(&mut InputState {
        mouse_x: 10.0,
        mouse_y: VIEW.y + 45.0,
        mouse_down: true,
        mouse_clicked: true,
        ..InputState::default()
    });
    assert_eq!(out.clicked, Some(1));
}

#[test]
fn a_row_of_no_height_is_not_drawn() {
    let mut rig = Rig::new(10);
    rig.real[1] = 0.0;
    let drawn = rig.frame();
    assert!(drawn.iter().all(|row| row.index != 1), "{drawn:?}");
    assert_eq!(drawn[1].index, 2);
    assert_eq!(drawn[1].y, VIEW.y + 20.0);
}

#[test]
fn padding_keeps_room_above_the_first_row_and_below_the_last() {
    let mut rig = Rig::new(20);
    rig.widget = List::new().with_padding(12.0, 8.0);
    let drawn = rig.frame();
    assert_eq!(drawn[0].y, VIEW.y + 12.0);
    assert_eq!(rig.state.top_row(), (0, -12.0));
    rig.state.follow_end();
    let drawn = rig.settle();
    let last = drawn[drawn.len() - 1];
    assert_eq!(last.index, 19);
    assert_eq!(last.y + last.height, VIEW.bottom() - 8.0);
}

#[test]
fn padding_works_for_fixed_rows_too() {
    let widget = List::new().with_item_height(20.0).with_padding(10.0, 30.0);
    let mut state = ListState::new();
    let mut list = DrawList::new();
    let theme = Theme::default();
    let mut tops = Vec::new();
    widget.draw(
        VIEW,
        10,
        &mut state,
        &mut list,
        &StyleResolver::new(&theme),
        &mut InputState::default(),
        |_, cell, item| tops.push((item.index, cell.y)),
    );
    assert_eq!(tops[0], (0, VIEW.y + 10.0));
    assert_eq!(state.scroll.content_size[1], 10.0 + 200.0 + 30.0);
}

#[test]
fn a_row_asked_for_near_the_end_is_reached_once_the_rows_are_measured() {
    // At 10 px estimates the last row can't be at the top: the view would
    // stop at row 10. Measured, the rows are taller and the end is further.
    let mut rig = Rig::new(20);
    rig.state.scroll_to_row(19, 0.0);
    let drawn = rig.frame();
    let last = drawn[drawn.len() - 1];
    assert_eq!(last.index, 19);
    assert_eq!(last.y + last.height, VIEW.bottom());
}

#[test]
fn a_new_width_and_a_page_in_front_in_one_frame_keep_the_share_of_the_right_row() {
    let mut rig = Rig::new(100);
    rig.state.scroll_to_row(12, 10.0); // row 12 measures 40: a quarter in
    rig.frame();
    rig.heights
        .splice(0..0, (0..40).map(|i| Row::estimated(15.0).keyed(1000 + i)));
    rig.real.splice(0..0, std::iter::repeat_n(15.0, 40));
    rig.width = 100.0;
    for height in &mut rig.real {
        *height *= 2.0;
    }
    rig.frame();
    assert_eq!(rig.state.top_row(), (52, 20.0));
}

#[test]
fn rows_estimated_again_by_their_owner_keep_the_share_of_the_top_row() {
    let mut rig = Rig::new(100);
    rig.state.scroll_to_row(12, 10.0); // row 12 measures 40: a quarter in
    rig.frame();
    // New fonts, say: the owner estimates every row again, and they will
    // measure twice as tall. The width is the same, so only the owner knows.
    rig.state.keep_share(&rig.heights);
    rig.heights
        .reestimate(|i| Row::estimated(15.0).keyed(i as u64));
    for height in &mut rig.real {
        *height *= 2.0;
    }
    let drawn = rig.frame();
    assert_eq!(rig.state.top_row(), (12, 20.0));
    assert_eq!(drawn[0].y, VIEW.y - 20.0);
}
