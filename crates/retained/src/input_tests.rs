//! Tests for pointer input: hover written to the app's signals and kept
//! current by layout, presses and clicks, the wheel, and hit testing at
//! scale. `check` also holds the pointer's state against hit testing from
//! nothing; the random oracle in `tests.rs` moves, presses and scrolls.

use std::cell::Cell;
use std::rc::Rc;

use crate::tests::{at, check, fixed, fonts};
use crate::{Button, Node, Signal, Size, Spec, Stack, Text, Ui};

/// A node taking the pointer through a hover signal, and the signal.
fn hoverable(ui: &mut Ui, parent: Node, spec: Spec) -> (Node, Signal<bool>) {
    let hovered = ui.signal(false);
    (ui.add(parent, spec.hover(hovered)), hovered)
}

#[test]
fn hover_writes_the_apps_signal_and_a_prop_follows_it() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let hovered = ui.signal(false);
    let fill = ui.memo(move |r| {
        if r.get(hovered) {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.2, 0.2, 0.2, 1.0]
        }
    });
    let button = ui.add(root, fixed(50.0, 20.0).fill(fill).hover(hovered));
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    assert!(ui.get(hovered));
    assert_eq!(ui.hovered(), Some(button));
    assert!(ui.needs_frame(0.0));
    ui.frame(at(0.0));
    // The button's own drawing is patched where the root's list holds it.
    assert_eq!(ui.frame_stats().recorded, 0);
    assert_eq!(ui.frame_stats().patched, 1);
    check(&mut ui);
    ui.pointer_moved(60.0, 10.0);
    assert!(!ui.get(hovered));
    assert_eq!(ui.hovered(), None);
    ui.frame(at(0.0));
    check(&mut ui);
}

#[test]
fn the_topmost_node_taking_the_pointer_is_hovered_and_others_let_it_through() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let (outer, outer_hovered) = hoverable(&mut ui, root, fixed(100.0, 100.0));
    // Over the outer box's top half, taking nothing.
    ui.add(outer, fixed(100.0, 50.0));
    let (inner, inner_hovered) = hoverable(&mut ui, outer, fixed(100.0, 50.0));
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    assert_eq!(ui.hovered(), Some(outer));
    ui.pointer_moved(10.0, 60.0);
    assert_eq!(ui.hovered(), Some(inner));
    assert!(ui.get(inner_hovered));
    assert!(!ui.get(outer_hovered));
    ui.frame(at(0.0));
    check(&mut ui);
}

#[test]
fn a_child_overflowing_its_parent_is_hit_and_one_clipped_away_is_not() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    // A column: the open box at 0..50, the clipping one at 50..100.
    let open = ui.add(root, fixed(50.0, 50.0));
    let (out, _) = hoverable(&mut ui, open, fixed(20.0, 20.0).absolute([100.0, 0.0]));
    let clipping = ui.add(root, fixed(50.0, 50.0).clip());
    hoverable(&mut ui, clipping, fixed(20.0, 20.0).absolute([100.0, 0.0]));
    ui.frame(at(0.0));
    assert_eq!(ui.hit_test(110.0, 10.0), Some(out));
    assert_eq!(ui.hit_test(110.0, 60.0), None);
    assert_eq!(ui.hit_test(10.0, 60.0), None);
    check(&mut ui);
}

/// The search in a stack that does not clip widens by how far its children
/// reach, and learns when that changes.
#[test]
fn children_moved_out_of_their_rows_in_an_open_stack_are_still_found() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let column = ui.add(root, Spec::new());
    let first = ui.add(column, fixed(50.0, 20.0));
    let second = ui.add(column, fixed(50.0, 20.0));
    let (mover, _) = hoverable(&mut ui, first, fixed(10.0, 10.0).absolute([0.0, 0.0]));
    let (back, _) = hoverable(&mut ui, second, fixed(10.0, 10.0).absolute([20.0, 0.0]));
    ui.frame(at(0.0));
    assert_eq!(ui.hit_test(5.0, 5.0), Some(mover));
    // One past the second row, out of the column's box; one back over the
    // first.
    ui.set_offset(mover, [0.0, 50.0]);
    ui.set_offset(back, [20.0, -15.0]);
    ui.frame(at(0.0));
    assert_eq!(ui.hit_test(5.0, 55.0), Some(mover));
    assert_eq!(ui.hit_test(25.0, 8.0), Some(back));
    check(&mut ui);
}

#[test]
fn before_the_first_frame_nothing_is_hit_and_the_frame_finds_the_hover() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let (button, hovered) = hoverable(&mut ui, root, fixed(50.0, 20.0));
    ui.pointer_moved(10.0, 10.0);
    assert_eq!(ui.hovered(), None);
    ui.frame(at(0.0));
    assert_eq!(ui.hovered(), Some(button));
    assert!(ui.get(hovered));
    check(&mut ui);
}

#[test]
fn a_scroll_under_a_still_pointer_moves_the_hover() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let list = ui.add(root, fixed(200.0, 100.0).scroll());
    let rows: Vec<_> = (0..20)
        .map(|_| hoverable(&mut ui, list, fixed(200.0, 20.0)))
        .collect();
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    assert_eq!(ui.hovered(), Some(rows[0].0));
    ui.wheel(0.0, 40.0);
    // Input is handled against what was drawn: the first row, until a
    // frame lays the scroll out.
    assert_eq!(ui.hovered(), Some(rows[0].0));
    ui.frame(at(0.0));
    assert_eq!(ui.hovered(), Some(rows[2].0));
    assert!(!ui.get(rows[0].1));
    assert!(ui.get(rows[2].1));
    check(&mut ui);
}

#[test]
fn a_node_moving_away_from_a_still_pointer_loses_the_hover() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let spacer = ui.add(root, fixed(10.0, 10.0));
    let (target, hovered) = hoverable(&mut ui, root, fixed(50.0, 20.0));
    ui.frame(at(0.0));
    ui.pointer_moved(5.0, 15.0);
    assert_eq!(ui.hovered(), Some(target));
    ui.set_height(spacer, Size::Fixed(40.0));
    ui.frame(at(0.0));
    assert_eq!(ui.hovered(), None);
    assert!(!ui.get(hovered));
    check(&mut ui);
}

#[test]
fn removing_the_hovered_node_writes_its_signal_false_and_hovers_what_is_under() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let (outer, outer_hovered) = hoverable(&mut ui, root, fixed(100.0, 100.0));
    let (inner, inner_hovered) = hoverable(&mut ui, outer, fixed(100.0, 100.0));
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    assert_eq!(ui.hovered(), Some(inner));
    ui.remove(inner);
    assert!(!ui.get(inner_hovered));
    assert_eq!(ui.hovered(), None);
    ui.frame(at(0.0));
    assert_eq!(ui.hovered(), Some(outer));
    assert!(ui.get(outer_hovered));
    check(&mut ui);
}

/// A button counting its clicks, with a press signal.
fn button(ui: &mut Ui, parent: Node, y: f32) -> (Node, Signal<bool>, Rc<Cell<u32>>) {
    let pressed = ui.signal(false);
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    let node = ui.add(
        parent,
        fixed(50.0, 20.0)
            .absolute([0.0, y])
            .press(pressed)
            .on_click(move |_, _| count.set(count.get() + 1)),
    );
    (node, pressed, clicks)
}

#[test]
fn a_press_and_a_release_on_the_same_node_is_a_click() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let (_, pressed, clicks) = button(&mut ui, root, 0.0);
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    ui.pointer_button(Button::Primary, true);
    assert!(ui.get(pressed));
    assert_eq!(clicks.get(), 0);
    ui.pointer_button(Button::Primary, false);
    assert!(!ui.get(pressed));
    assert_eq!(clicks.get(), 1);
    // Pressed on it and released elsewhere: it holds the press until the
    // release, and there is no click.
    ui.pointer_button(Button::Primary, true);
    ui.pointer_moved(100.0, 100.0);
    assert!(ui.get(pressed));
    ui.pointer_button(Button::Primary, false);
    assert!(!ui.get(pressed));
    assert_eq!(clicks.get(), 1);
    // Pressed elsewhere and released on it: no click.
    ui.pointer_button(Button::Primary, true);
    ui.pointer_moved(10.0, 10.0);
    assert!(!ui.get(pressed));
    ui.pointer_button(Button::Primary, false);
    assert_eq!(clicks.get(), 1);
    ui.frame(at(0.0));
    check(&mut ui);
}

#[test]
fn each_button_owns_its_own_press() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let (_, a_pressed, a_clicks) = button(&mut ui, root, 0.0);
    let (_, b_pressed, b_clicks) = button(&mut ui, root, 30.0);
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    ui.pointer_button(Button::Primary, true);
    ui.pointer_moved(10.0, 40.0);
    ui.pointer_button(Button::Secondary, true);
    assert!(ui.get(a_pressed) && ui.get(b_pressed));
    check(&mut ui);
    ui.pointer_button(Button::Primary, false);
    assert!(!ui.get(a_pressed) && ui.get(b_pressed));
    assert_eq!((a_clicks.get(), b_clicks.get()), (0, 0));
    ui.pointer_button(Button::Secondary, false);
    assert_eq!((a_clicks.get(), b_clicks.get()), (0, 1));
    check(&mut ui);
}

#[test]
fn a_press_ends_without_a_click_when_cancelled_when_the_pointer_leaves_or_its_node_goes() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let (node, pressed, clicks) = button(&mut ui, root, 0.0);
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);

    ui.pointer_button(Button::Primary, true);
    ui.cancel_pointer();
    assert!(!ui.get(pressed));
    ui.pointer_button(Button::Primary, false);
    assert_eq!(clicks.get(), 0);

    ui.pointer_button(Button::Primary, true);
    ui.pointer_left();
    assert!(!ui.get(pressed));
    ui.pointer_moved(10.0, 10.0);
    ui.pointer_button(Button::Primary, false);
    assert_eq!(clicks.get(), 0);

    ui.pointer_button(Button::Primary, true);
    ui.remove(node);
    assert!(!ui.get(pressed));
    ui.pointer_button(Button::Primary, false);
    assert_eq!(clicks.get(), 0);
    ui.frame(at(0.0));
    check(&mut ui);
}

#[test]
fn a_click_handler_may_remove_its_own_node_and_add_others() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let list = ui.add(root, Spec::new());
    let me = Rc::new(Cell::new(None::<Node>));
    let added = Rc::new(Cell::new(None::<Node>));
    let (me_in, added_in) = (me.clone(), added.clone());
    let node = ui.add(
        list,
        fixed(50.0, 20.0).on_click(move |ui, _| {
            ui.remove(me_in.get().expect("set"));
            let hovered = ui.signal(false);
            added_in.set(Some(ui.add(list, fixed(50.0, 20.0).hover(hovered))));
        }),
    );
    me.set(Some(node));
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    ui.pointer_button(Button::Primary, true);
    ui.pointer_button(Button::Primary, false);
    assert_eq!(ui.children(list).count(), 1);
    ui.frame(at(0.0));
    assert_eq!(ui.hovered(), added.get());
    check(&mut ui);
}

#[test]
fn the_wheel_scrolls_the_innermost_container_that_scrolls_along_it() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let outer = ui.add(root, fixed(200.0, 100.0).scroll());
    let inner = ui.add(outer, fixed(100.0, 40.0).stack(Stack::row()).scroll());
    for _ in 0..10 {
        ui.add(inner, fixed(30.0, 40.0));
    }
    for _ in 0..10 {
        ui.add(outer, fixed(200.0, 20.0));
    }
    ui.frame(at(0.0));
    ui.pointer_moved(10.0, 10.0);
    // Down: the row does not scroll that way, the column does (and the row,
    // at -15..25 after it, stays under the pointer).
    ui.wheel(0.0, 15.0);
    ui.frame(at(0.0));
    assert_eq!(ui.get(ui.scroll_offset(outer)), 15.0);
    assert_eq!(ui.get(ui.scroll_offset(inner)), 0.0);
    // Across: the row.
    ui.wheel(25.0, 0.0);
    ui.frame(at(0.0));
    assert_eq!(ui.get(ui.scroll_offset(inner)), 25.0);
    // At its end (300 of content in 100) the step goes nowhere else.
    ui.scroll_to(inner, 200.0);
    ui.frame(at(0.0));
    ui.wheel(50.0, 0.0);
    ui.frame(at(0.0));
    assert_eq!(ui.get(ui.scroll_offset(inner)), 200.0);
    assert_eq!(ui.get(ui.scroll_offset(outer)), 15.0);
    check(&mut ui);
}

/// At scale, a hit enters the path to what it finds: binary search in each
/// stack, not a scan of the rows.
#[test]
fn a_hit_among_ten_thousand_rows_enters_the_path_to_it() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let list = ui.add(root, fixed(300.0, 400.0).scroll());
    let mut rows = Vec::new();
    for i in 0..10_000 {
        let (row, _) = hoverable(&mut ui, list, fixed(300.0, 20.0));
        ui.add(
            row,
            Spec::new().text(Text::new(format!("row {i}"), 12.0), [1.0; 4]),
        );
        rows.push(row);
    }
    ui.frame(at(0.0));
    ui.scroll_to(list, 100_000.0);
    ui.frame(at(0.0));
    let before = ui.input_stats();
    ui.pointer_moved(150.0, 70.0);
    let after = ui.input_stats();
    assert_eq!(ui.hovered(), Some(rows[5003]));
    assert_eq!(after.hit_tests - before.hit_tests, 1);
    // The root, the list, the row and its label.
    assert_eq!(after.entered - before.entered, 4);
    check(&mut ui);
}
