//! Tests for the tree: the layout rules, dirt, what a frame does (counted),
//! and the frame oracle: after any sequence of changes, the frame's list is
//! the list the same tree draws from nothing, and its layout the layout it
//! gets from nothing.

use wgpu_gameui::{BoxShadow, DrawList, FontSystemHandle, TextAlign, shared_font_system};

use crate::{
    Align, Border, Button, FrameContext, Insets, Look, Node, Painter, Signal, Size, Spec, Stack,
    Text, Ui,
};

pub(crate) fn fonts() -> FontSystemHandle {
    shared_font_system()
}

pub(crate) fn at(now: f64) -> FrameContext {
    FrameContext {
        now,
        viewport: [640.0, 480.0],
        fonts: 0,
        scale: 1.0,
    }
}

thread_local! {
    /// State outside the tree that the random oracle's volatile painters
    /// draw from; each step moves it on.
    static OUTSIDE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(crate) fn fixed(w: f32, h: f32) -> Spec {
    Spec::new().width(Size::Fixed(w)).height(Size::Fixed(h))
}

/// The frame's list is the tree drawn from nothing, its layout the layout
/// from nothing, and the pointer's state what that layout and the pointer
/// give.
pub(crate) fn check(ui: &mut Ui) {
    let fresh = ui.draw_from_nothing();
    let frame = ui.list().expect("a frame was drawn");
    if let Some(difference) = frame.difference(&fresh) {
        panic!("the frame differs from the tree drawn from nothing: {difference}");
    }
    ui.check_layout();
    ui.check_pointer();
}

pub(crate) fn rect(ui: &Ui, node: Node) -> [f32; 4] {
    let (pos, size) = ui.rect(node);
    [pos[0], pos[1], size[0], size[1]]
}

// ---- Layout ----

#[test]
fn a_column_stacks_fixed_children_with_gap_and_padding() {
    let mut ui = Ui::new(fonts());
    let col = ui.add(
        ui.root(),
        Spec::new().stack(Stack::column().gap(3.0).padding(Insets::xy(4.0, 6.0))),
    );
    let a = ui.add(col, fixed(30.0, 10.0));
    let b = ui.add(col, fixed(50.0, 20.0));
    ui.frame(at(0.0));
    assert_eq!(rect(&ui, a), [4.0, 6.0, 30.0, 10.0]);
    assert_eq!(rect(&ui, b), [4.0, 19.0, 50.0, 20.0]);
    // It hugs: the widest child and the stack, plus padding.
    assert_eq!(rect(&ui, col), [0.0, 0.0, 58.0, 45.0]);
    check(&mut ui);
}

#[test]
fn fill_children_share_what_is_left_by_weight_with_snapped_edges() {
    let mut ui = Ui::new(fonts());
    let row = ui.add(
        ui.root(),
        Spec::new()
            .width(Size::Fixed(300.0))
            .height(Size::Fixed(20.0))
            .stack(Stack::row().gap(5.0).padding(Insets::xy(10.0, 0.0))),
    );
    let fixed_child = ui.add(row, Spec::new().width(Size::Fixed(40.0)));
    let one = ui.add(
        row,
        Spec::new().width(Size::Fill(1.0)).height(Size::Fill(1.0)),
    );
    let two = ui.add(row, Spec::new().width(Size::Fill(2.0)));
    ui.frame(at(0.0));
    // 300 - 20 padding - 40 - 10 gaps = 230, shared 1:2.
    assert_eq!(rect(&ui, fixed_child), [10.0, 0.0, 40.0, 0.0]);
    assert_eq!(rect(&ui, one), [55.0, 0.0, 77.0, 20.0]);
    assert_eq!(rect(&ui, two), [137.0, 0.0, 153.0, 0.0]);
    check(&mut ui);
}

#[test]
fn a_fill_child_of_a_hugging_parent_hugs() {
    let mut ui = Ui::new(fonts());
    let col = ui.add(ui.root(), Spec::new());
    let filler = ui.add(
        col,
        Spec::new().width(Size::Fill(1.0)).height(Size::Fill(1.0)),
    );
    let inner = ui.add(filler, fixed(12.0, 7.0));
    ui.frame(at(0.0));
    assert_eq!(rect(&ui, filler), [0.0, 0.0, 12.0, 7.0]);
    assert_eq!(rect(&ui, inner), [0.0, 0.0, 12.0, 7.0]);
    check(&mut ui);
}

#[test]
fn children_align_along_and_across_the_stack() {
    let mut ui = Ui::new(fonts());
    let row = ui.add(
        ui.root(),
        fixed(100.0, 50.0).stack(Stack::row().align(Align::Center, Align::Center)),
    );
    let child = ui.add(row, fixed(20.0, 11.0));
    let end = ui.add(
        ui.root(),
        fixed(100.0, 50.0).stack(Stack::column().align(Align::End, Align::End)),
    );
    let at_end = ui.add(end, fixed(20.0, 10.0));
    ui.frame(at(0.0));
    assert_eq!(rect(&ui, child), [40.0, 20.0, 20.0, 11.0]);
    assert_eq!(rect(&ui, at_end), [80.0, 40.0, 20.0, 10.0]);
    check(&mut ui);
}

#[test]
fn an_absolute_child_sits_at_its_offset_and_takes_no_room() {
    let mut ui = Ui::new(fonts());
    let col = ui.add(
        ui.root(),
        Spec::new().stack(Stack::column().padding(Insets::all(5.0))),
    );
    let stacked = ui.add(col, fixed(10.0, 10.0));
    let floating = ui.add(col, fixed(30.0, 30.0).absolute([7.5, -2.0]));
    ui.frame(at(0.0));
    assert_eq!(rect(&ui, stacked), [5.0, 5.0, 10.0, 10.0]);
    assert_eq!(rect(&ui, floating), [13.0, 3.0, 30.0, 30.0]);
    assert_eq!(rect(&ui, col), [0.0, 0.0, 20.0, 20.0]);
    ui.set_offset(floating, [1.0, 1.0]);
    ui.frame(at(0.0));
    assert_eq!(rect(&ui, floating), [6.0, 6.0, 30.0, 30.0]);
    check(&mut ui);
}

#[test]
fn hugging_text_is_as_big_as_it_measures() {
    let fonts = fonts();
    let mut ui = Ui::new(fonts.clone());
    let label = ui.add(
        ui.root(),
        Spec::new().text(Text::new("Hello there", 16.0).no_wrap(), [1.0; 4]),
    );
    ui.frame(at(0.0));
    let mut list = DrawList::with_font_system(fonts);
    let (w, h) = list.measure_block(
        &wgpu_gameui::TextBlock::new("Hello there", 0.0, 0.0)
            .with_size(16.0)
            .with_line_height(20.0)
            .with_max_width(1.0e6)
            .with_wrap(wgpu_gameui::WrapMode::None),
    );
    assert_eq!(rect(&ui, label), [0.0, 0.0, w.ceil(), h.ceil()]);
    check(&mut ui);
}

/// Text layouts the font system has shaped so far.
fn shaped(fonts: &FontSystemHandle) -> u64 {
    fonts.lock().expect("font system").layout_stats().shaped
}

/// Look each of `nodes`' texts up as the renderer does when it prepares the
/// frame: the block the node records, through the shared layouts. The tests
/// draw no frame on a GPU, so this is the draw's share of the shaping.
fn prepare_texts(ui: &Ui, nodes: &[Node]) {
    let mut list = DrawList::with_font_system(ui.fonts.clone());
    for &node in nodes {
        let d = &ui.nodes[ui.index(node) as usize];
        let t = d.text.as_ref().expect("a text node");
        let (x, y, width) = crate::layout::text_place(d);
        list.measure_block(&crate::layout::text_block(&t.text, t.color, x, y, width));
    }
}

/// Today's cost, pinned until prepared text lands
/// (`docs/design/prepared-text.md`): a hugging label is measured at the
/// width it is offered and drawn at its own, two layouts each shaped; one of
/// exact width is measured and drawn at the same width, shaped once.
#[test]
fn a_new_or_changed_label_is_shaped_once_per_width_it_is_laid_out_at() {
    let fonts = fonts();
    let mut ui = Ui::new(fonts.clone());
    let hugging = ui.add(
        ui.root(),
        Spec::new().text(Text::new("Hugging 0", 16.0).no_wrap(), [1.0; 4]),
    );
    let exact = ui.add(
        ui.root(),
        Spec::new()
            .width(Size::Fixed(120.0))
            .text(Text::new("Exact 0", 16.0).no_wrap(), [1.0; 4]),
    );
    let texts = [hugging, exact];
    let start = shaped(&fonts);
    ui.frame(at(0.0));
    prepare_texts(&ui, &texts);
    assert_eq!(shaped(&fonts) - start, 3, "two hugging, one exact");
    for (node, content, shapes) in [(hugging, "Hugging 1", 2), (exact, "Exact 1", 1)] {
        let start = shaped(&fonts);
        ui.set_text(node, Text::new(content, 16.0).no_wrap());
        ui.frame(at(0.0));
        prepare_texts(&ui, &texts);
        assert_eq!(shaped(&fonts) - start, shapes, "{content}");
    }
    let start = shaped(&fonts);
    ui.set_text_color(hugging, [0.5; 4]);
    ui.frame(at(0.0));
    prepare_texts(&ui, &texts);
    ui.frame(at(0.0));
    prepare_texts(&ui, &texts);
    assert_eq!(shaped(&fonts), start, "a colour and an unchanged frame");
    check(&mut ui);
}

#[test]
fn layout_dirt_stops_at_a_node_whose_size_does_not_follow_its_content() {
    let mut ui = Ui::new(fonts());
    let fixed_box = ui.add(ui.root(), fixed(200.0, 200.0));
    let hug = ui.add(fixed_box, Spec::new());
    let label = ui.add(hug, Spec::new().text(Text::new("a", 14.0), [1.0; 4]));
    let other = ui.add(ui.root(), Spec::new());
    for _ in 0..20 {
        ui.add(other, fixed(5.0, 5.0));
    }
    ui.frame(at(0.0));
    ui.set_text(label, Text::new("a longer text", 14.0));
    ui.frame(at(0.0));
    // The text, the box hugging it and the fixed box that holds it; not the
    // root, nor the other column, nor its children.
    let stats = ui.frame_stats();
    assert_eq!(stats.laid_out, 3);
    // Those and the root, whose cached layout leads to them: the other
    // column is not entered.
    assert_eq!(stats.visited, 4);
    assert_eq!(stats.measured, 1);
    assert!(rect(&ui, hug)[2] > rect(&ui, label)[2] - 1.0);
    check(&mut ui);
}

// ---- What a frame does ----

/// A column of `rows` rows of `cols` cells, each a box with a label.
fn grid(ui: &mut Ui, rows: usize, cols: usize) -> Vec<Node> {
    let root = ui.root();
    grid_in(ui, root, rows, cols)
}

/// [`grid`], in `parent`.
fn grid_in(ui: &mut Ui, parent: Node, rows: usize, cols: usize) -> Vec<Node> {
    let column = ui.add(parent, Spec::new().stack(Stack::column().gap(2.0)));
    let mut cells = Vec::new();
    for r in 0..rows {
        let row = ui.add(column, Spec::new().stack(Stack::row().gap(2.0)));
        for c in 0..cols {
            let cell = ui.add(
                row,
                fixed(40.0, 20.0)
                    .fill([0.2, 0.3, 0.4, 1.0])
                    .look(Look {
                        radius: 3.0,
                        ..Look::default()
                    })
                    .stack(Stack::column().align(Align::Center, Align::Center)),
            );
            ui.add(
                cell,
                Spec::new().text(Text::new(format!("{}", r * cols + c), 12.0), [1.0; 4]),
            );
            cells.push(cell);
        }
    }
    cells
}

#[test]
fn a_frame_with_nothing_changed_does_nothing_and_keeps_the_list() {
    let mut ui = Ui::new(fonts());
    grid(&mut ui, 4, 4);
    let id = ui.frame(at(0.0)).id();
    assert!(!ui.needs_frame(0.0));
    let again = ui.frame(at(1.0)).id();
    assert_eq!(again, id);
    let stats = ui.frame_stats();
    assert_eq!(
        (stats.laid_out, stats.painted, stats.recorded, stats.placed),
        (0, 0, 0, 0)
    );
    check(&mut ui);
}

/// Only the root and nodes that clip keep a list; the rest are drawn into
/// it, so a grid of a hundred cells keeps one.
#[test]
fn a_grid_keeps_the_root_list_alone() {
    let mut ui = Ui::new(fonts());
    grid(&mut ui, 10, 10);
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    // The root, the column, the rows, the cells and their labels.
    assert_eq!(stats.painted, 1 + 1 + 10 + 200);
    assert_eq!((stats.recorded, stats.placed), (1, 0));
    assert_eq!(ui.tree_stats().lists, 1);
    check(&mut ui);
}

#[test]
fn one_changed_cell_is_patched_where_the_root_holds_it() {
    let mut ui = Ui::new(fonts());
    let cells = grid(&mut ui, 10, 10);
    ui.frame(at(0.0));
    ui.set_fill(cells[37], [0.9, 0.1, 0.1, 1.0]);
    assert!(ui.needs_frame(0.0));
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    assert_eq!(stats.laid_out, 0);
    // The cell's own drawing alone, its box: drawn with the shape it had,
    // so its record is overwritten where the root's list holds it, and no
    // list is recorded or placed.
    assert_eq!((stats.recorded, stats.painted), (0, 1));
    assert_eq!((stats.placed, stats.copied), (0, 0));
    assert_eq!(stats.patched, 1);
    assert_eq!(stats.memo_runs, 1);
    check(&mut ui);
}

/// In a node that clips, a cell's change is patched in the clip's list and
/// then, the clip's list keeping its shape, in the root's.
#[test]
fn a_cell_in_a_clip_is_patched_in_its_list_and_in_the_root() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let clip = ui.add(root, fixed(500.0, 300.0).clip());
    let cells = grid_in(&mut ui, clip, 10, 10);
    ui.frame(at(0.0));
    assert_eq!(ui.tree_stats().lists, 2);
    ui.set_fill(cells[37], [0.9, 0.1, 0.1, 1.0]);
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    assert_eq!((stats.recorded, stats.painted, stats.placed), (0, 1, 0));
    assert_eq!(stats.patched, 2);
    check(&mut ui);
}

/// A change that alters a node's shape records the list that holds it, and
/// each list above it as far as one keeps its shape.
#[test]
fn a_change_of_shape_records_the_lists_above_it() {
    let mut ui = Ui::new(fonts());
    let root = ui.root();
    let clip = ui.add(root, fixed(500.0, 300.0).clip());
    let cells = grid_in(&mut ui, clip, 10, 10);
    ui.frame(at(0.0));
    // A transparent cell draws no box: one record fewer.
    ui.set_fill(cells[37], [0.0; 4]);
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    // The clip's list and the root's.
    assert_eq!(stats.recorded, 2);
    assert_eq!(stats.patched, 0);
    check(&mut ui);
    // Back: the same again.
    ui.set_fill(cells[37], [0.9, 0.1, 0.1, 1.0]);
    ui.frame(at(0.0));
    assert_eq!(ui.frame_stats().recorded, 2);
    check(&mut ui);
    // A recolour now keeps the shape.
    ui.set_fill(cells[37], [0.1, 0.9, 0.1, 1.0]);
    ui.frame(at(0.0));
    assert_eq!(ui.frame_stats().recorded, 0);
    assert_eq!(ui.frame_stats().patched, 2);
    check(&mut ui);
    // So does another cell's.
    ui.set_fill(cells[38], [0.3, 0.3, 0.3, 1.0]);
    ui.frame(at(0.0));
    assert_eq!(ui.frame_stats().recorded, 0);
    check(&mut ui);
}

#[test]
fn a_scrolling_column_draws_only_the_rows_in_view() {
    let mut ui = Ui::new(fonts());
    let list = ui.add(ui.root(), fixed(200.0, 100.0).scroll());
    let mut rows = Vec::new();
    for i in 0..1000 {
        let row = ui.add(list, fixed(200.0, 20.0).fill([0.1, 0.1, 0.1, 1.0]));
        ui.add(
            row,
            Spec::new().text(Text::new(format!("row {i}"), 12.0), [1.0; 4]),
        );
        rows.push(row);
    }
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    // Five rows fill the 100 px; binary search looks at only those (and the
    // root at the list, and each row drawn at its label).
    assert_eq!(stats.examined, 1 + 5 + 5);
    // The root's list and the scroll container's; the rows are drawn into
    // the latter.
    assert_eq!(stats.recorded, 2);
    assert_eq!(stats.painted, 2 + 2 * 5);
    check(&mut ui);

    ui.scroll_by(list, 30.0);
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    // Rows 1 to 6 are in view now: the list is recorded with them, and,
    // holding a row more, placed in the root's list recorded again.
    assert_eq!(stats.examined, 1 + 6 + 6);
    assert_eq!(stats.recorded, 2);
    assert_eq!(stats.placed, 1);
    check(&mut ui);

    // Past the end it stops at the end, and the signal says so.
    ui.scroll_by(list, 1.0e6);
    ui.frame(at(0.0));
    assert_eq!(ui.get(ui.scroll_offset(list)), 20_000.0 - 100.0);
    check(&mut ui);
}

#[test]
fn a_child_outside_the_clip_whose_shadow_reaches_in_is_drawn() {
    let mut ui = Ui::new(fonts());
    let clip = ui.add(ui.root(), fixed(100.0, 100.0).clip());
    let mover = ui.add(
        clip,
        fixed(10.0, 10.0).absolute([110.0, 0.0]).look(Look {
            shadow: Some(BoxShadow {
                offset: [-8.0, 0.0],
                blur: 4.0,
                spread: 0.0,
                color: [0.0, 0.0, 0.0, 0.5],
                inset: false,
            }),
            ..Look::default()
        }),
    );
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().shadow_instance_count(), 1);
    check(&mut ui);
    ui.set_offset(mover, [200.0, 0.0]);
    ui.frame(at(0.0));

    assert_eq!(ui.list().unwrap().shadow_instance_count(), 0);
    check(&mut ui);
}

/// The search for the rows in view widens by how far any row paints
/// outside its box, both ways along the stack, and learns when that
/// changes.
#[test]
fn rows_out_of_view_whose_shadows_reach_in_are_drawn_on_either_side() {
    let mut ui = Ui::new(fonts());
    let list = ui.add(ui.root(), fixed(200.0, 100.0).scroll());
    let shadow = |dy: f32| Look {
        shadow: Some(BoxShadow {
            offset: [0.0, dy],
            blur: 4.0,
            spread: 0.0,
            color: [0.0, 0.0, 0.0, 0.5],
            inset: false,
        }),
        ..Look::default()
    };
    let mut rows = Vec::new();
    for i in 0..10 {
        // Row 3, at 60..80, is above the view once scrolled by 100, its
        // shadow reaching down into it.
        let look = if i == 3 {
            shadow(30.0)
        } else {
            Look::default()
        };
        rows.push(ui.add(list, fixed(200.0, 20.0).look(look)));
    }
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().shadow_instance_count(), 1);
    // Row 6, at 120..140, is below the view; its new shadow reaches up.
    ui.set_look(rows[6], shadow(-30.0));
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().shadow_instance_count(), 2);
    check(&mut ui);
    ui.scroll_to(list, 100.0);
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().shadow_instance_count(), 2);
    check(&mut ui);
}

/// Ten thousand rows of 20 px in a 400 px scroll list, scrolled to row
/// 5000; returns the list and the rows' labels.
fn long_list(ui: &mut Ui) -> (Node, Vec<Node>) {
    let list = ui.add(ui.root(), fixed(300.0, 400.0).scroll());
    let mut labels = Vec::new();
    for i in 0..10_000 {
        let row = ui.add(list, fixed(300.0, 20.0).fill([0.1, 0.1, 0.1, 1.0]));
        labels.push(ui.add(
            row,
            Spec::new().text(Text::new(format!("row {i}"), 12.0), [1.0; 4]),
        ));
    }
    ui.frame(at(0.0));
    ui.scroll_to(list, 100_000.0);
    ui.frame(at(0.0));
    (list, labels)
}

/// At scale, a change costs the path to it and the rows in view, not the
/// rows.
#[test]
fn a_change_in_ten_thousand_rows_costs_the_rows_in_view() {
    let mut ui = Ui::new(fonts());
    let (_, labels) = long_list(&mut ui);
    // In view: rows 5000 to 5019.
    ui.set_text(labels[5003], Text::new("changed", 12.0));
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    assert_eq!(stats.measured, 1);
    // The label, and the row that holds it, whose size is its own.
    assert_eq!(stats.laid_out, 2);
    // Those, the list and the root.
    assert_eq!(stats.visited, 4);
    // The label's own drawing alone; its record is patched in the list and
    // the root.
    assert_eq!((stats.recorded, stats.examined, stats.placed), (0, 0, 0));
    assert_eq!((stats.painted, stats.patched), (1, 2));
    check(&mut ui);

    // Out of view, and more than a viewport from it: the label is not laid
    // out (the list's walk for what is dirty stops at the rows near the
    // view), and its row is not drawn, but the list is recorded again (it
    // cannot tell the label stayed out of view) with its twenty rows and
    // their labels, keeping its shape, so the root is patched with it.
    ui.set_text(labels[0], Text::new("changed", 12.0));
    ui.frame(at(0.0));
    let stats = ui.frame_stats();
    assert_eq!((stats.measured, stats.laid_out, stats.visited), (0, 0, 2));
    assert_eq!((stats.recorded, stats.examined, stats.placed), (1, 40, 0));
    assert_eq!(stats.patched, 40);
    check(&mut ui);
}

/// The rows of a scroll that it can size without laying them out (a fixed
/// length along it, filling across it) are laid out only within a viewport
/// of its view; the rest wait until a scroll brings them near, a change in
/// one of them too.
#[test]
fn a_scroll_lays_out_only_the_rows_near_its_view() {
    let mut ui = Ui::new(fonts());
    let list = ui.add(ui.root(), fixed(300.0, 400.0).scroll());
    let mut labels = Vec::new();
    for i in 0..10_000 {
        let row = ui.add(
            list,
            Spec::new().width(Size::Fill(1.0)).height(Size::Fixed(20.0)),
        );
        labels.push(ui.add(
            row,
            Spec::new().text(Text::new(format!("row {i}"), 12.0), [1.0; 4]),
        ));
    }
    let counts = |ui: &Ui| {
        let s = ui.frame_stats();
        (s.deferred, s.laid_out, s.measured)
    };
    // In view: rows 0 to 19; near it, to row 39. The rest are sized; laid
    // out: the root, the list, and forty rows and their labels.
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (10_000, 82, 40));
    check(&mut ui);

    // Five rows more come near.
    ui.scroll_by(list, 100.0);
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (0, 10, 5));
    check(&mut ui);

    // Far down: in view, rows 5000 to 5019; near it, 4980 to 5039.
    ui.scroll_to(list, 100_000.0);
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (0, 120, 60));
    check(&mut ui);

    // A row laid out before, now far from the view, changes: it waits.
    ui.set_text(labels[0], Text::new("changed", 12.0));
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (0, 0, 0));
    check(&mut ui);

    // Back near it: that label and its row are laid out (the rest near are
    // as they were), and drawn as it is now.
    ui.scroll_to(list, 0.0);
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (0, 2, 1));
    check(&mut ui);
    let changed = rect(&ui, labels[0]);
    assert!(
        changed[2] > rect(&ui, labels[1])[2],
        "the label was measured as it is"
    );

    // Narrower: every row has other room, so every row is sized again, and
    // those near the view are laid out.
    ui.set_width(list, Size::Fixed(200.0));
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (10_000, 82, 40));
    check(&mut ui);
    assert_eq!(rect(&ui, ui.parent(labels[7000]).unwrap())[2], 200.0);

    // Far down again: rows laid out for the old width are laid out for the
    // new one as they come near.
    ui.scroll_to(list, 140_000.0);
    ui.frame(at(0.0));
    assert_eq!(counts(&ui), (0, 120, 60));
    check(&mut ui);
}

/// A scroll shorter than half the window lays out its rows to half the
/// window past its view, not to its own length.
#[test]
fn a_short_scroll_lays_out_to_half_the_window_past_its_view() {
    let mut ui = Ui::new(fonts());
    let list = ui.add(ui.root(), fixed(300.0, 100.0).scroll());
    for i in 0..1_000 {
        let row = ui.add(
            list,
            Spec::new().width(Size::Fill(1.0)).height(Size::Fixed(20.0)),
        );
        ui.add(
            row,
            Spec::new().text(Text::new(format!("row {i}"), 12.0), [1.0; 4]),
        );
    }
    // In view: rows 0 to 4; half the window (240 px) past it: to row 16.
    ui.frame(at(0.0));
    assert_eq!(ui.frame_stats().measured, 17);
    check(&mut ui);
    // At row 500: rows 488 to 516.
    ui.scroll_to(list, 10_000.0);
    ui.frame(at(0.0));
    assert_eq!(ui.frame_stats().measured, 29);
    check(&mut ui);
}

/// What frames over ten thousand rows take on this machine. Prints only;
/// run with `--ignored --nocapture` in release.
#[test]
#[ignore]
fn ten_thousand_rows_timings() {
    use std::time::Instant;
    let mut ui = Ui::new(fonts());
    let start = Instant::now();
    let (list, labels) = long_list(&mut ui);
    println!("build and two frames: {:?}", start.elapsed());
    let start = Instant::now();
    for i in 0..100 {
        ui.set_text(
            labels[5000 + i % 20],
            Text::new(format!("changed {i}"), 12.0),
        );
        ui.frame(at(0.0));
    }
    println!(
        "a row in view changed, per frame: {:?}",
        start.elapsed() / 100
    );
    let start = Instant::now();
    for _ in 0..100 {
        ui.scroll_by(list, 7.0);
        ui.frame(at(0.0));
    }
    println!("scrolled by 7 px, per frame: {:?}", start.elapsed() / 100);
    let start = Instant::now();
    for _ in 0..100 {
        ui.frame(at(0.0));
    }
    println!("nothing changed, per frame: {:?}", start.elapsed() / 100);
    let start = Instant::now();
    for i in 0..1000 {
        ui.pointer_moved(150.0, (i % 400) as f32);
    }
    println!("a pointer move, per event: {:?}", start.elapsed() / 1000);
    let tree = ui.tree_stats();
    println!("{tree:?}");
}

/// A scroll container whose last row goes is drawn into its parent's list
/// as a leaf: its clip, with nothing in it, goes too.
#[test]
fn a_scroll_container_that_loses_its_last_row_draws_no_clip() {
    let mut ui = Ui::new(fonts());
    let list = ui.add(ui.root(), fixed(200.0, 100.0).scroll());
    let row = ui.add(list, fixed(200.0, 20.0).fill([1.0; 4]));
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().viewport_clips().len(), 1);
    check(&mut ui);
    ui.remove(row);
    ui.frame(at(0.0));
    assert!(ui.list().unwrap().viewport_clips().is_empty());
    check(&mut ui);
}

/// A culled node is not drawn, so what it drew stays out of date; a later
/// change to it must still reach the lists above it.
#[test]
fn a_change_to_a_culled_node_that_is_already_out_of_date_reaches_the_lists_above() {
    let mut ui = Ui::new(fonts());
    let clip = ui.add(ui.root(), fixed(100.0, 100.0).clip());
    let mover = ui.add(clip, fixed(10.0, 10.0).absolute([110.0, 0.0]));
    ui.frame(at(0.0));
    ui.set_fill(mover, [1.0, 0.0, 0.0, 1.0]);
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().shadow_instance_count(), 0);
    ui.set_look(
        mover,
        Look {
            shadow: Some(BoxShadow {
                offset: [-8.0, 0.0],
                blur: 4.0,
                spread: 0.0,
                color: [0.0, 0.0, 0.0, 0.5],
                inset: false,
            }),
            ..Look::default()
        },
    );
    ui.frame(at(0.0));
    assert_eq!(ui.list().unwrap().shadow_instance_count(), 1);
    check(&mut ui);
}

#[test]
fn a_painter_is_drawn_again_when_its_value_changes_and_at_the_time_it_asks() {
    let mut ui = Ui::new(fonts());
    let tone = ui.signal(0.25f32);
    let host = ui.add(ui.root(), Spec::new());
    ui.add(
        host,
        fixed(10.0, 10.0).custom(Painter::new(
            move |r| r.get(tone),
            |tone: &f32, paint| {
                let next = paint.now() + 1.0;
                paint.repaint_at(next);
                let [w, h] = paint.size();
                paint.list().quad(0.0, 0.0, w, h, [*tone, 0.0, 0.0, 1.0]);
            },
        )),
    );
    ui.frame(at(0.0));
    assert_eq!(ui.next_repaint(), Some(1.0));
    assert!(!ui.needs_frame(0.5));
    assert!(ui.needs_frame(1.0));
    ui.frame(at(1.0));
    assert_eq!(ui.frame_stats().painted, 3);
    assert_eq!(ui.next_repaint(), Some(2.0));
    check(&mut ui);

    // The painter alone, patched where the root's list holds it.
    ui.set(tone, 0.75);
    ui.frame(at(1.5));
    let stats = ui.frame_stats();
    assert_eq!((stats.painted, stats.memo_runs), (1, 1));
    assert_eq!((stats.recorded, stats.patched), (0, 1));
    check(&mut ui);
}

#[test]
fn a_volatile_painter_draws_every_frame() {
    let mut ui = Ui::new(fonts());
    let host = ui.add(ui.root(), Spec::new());
    let painter = ui.add(
        host,
        fixed(4.0, 4.0).custom(
            Painter::new(
                |_| (),
                |_: &(), paint| {
                    paint.list().quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]);
                },
            )
            .volatile(),
        ),
    );
    ui.frame(at(0.0));
    assert!(ui.needs_frame(0.0));
    ui.frame(at(0.0));
    // The root's list, which the host and the painter are drawn into.
    assert_eq!(ui.frame_stats().recorded, 1);
    ui.remove(painter);
    ui.frame(at(0.0));
    assert!(!ui.needs_frame(0.0));
    check(&mut ui);
}

/// The bytes counted are the lists' own and their logs': a node that clips
/// adds its list, and removing it takes the list's bytes away again.
#[test]
fn tree_stats_count_the_bytes_each_kept_list_holds() {
    let mut ui = Ui::new(fonts());
    ui.frame(at(0.0));
    let root_bytes = |ui: &Ui| {
        let kept = ui.nodes[ui.root as usize].kept.as_deref().unwrap();
        (std::mem::size_of::<crate::ui::Kept>() + kept.list.heap_bytes() + kept.log.heap_bytes())
            as u64
    };
    let alone = ui.tree_stats();
    assert_eq!(alone.lists, 1);
    assert_eq!(alone.list_bytes, root_bytes(&ui));
    assert!(alone.node_bytes > 0);
    let panel = ui.add(ui.root(), Spec::new().clip());
    ui.add(panel, fixed(3.0, 3.0).fill([1.0; 4]));
    ui.frame(at(0.0));
    let with_panel = ui.tree_stats();
    assert_eq!(with_panel.lists, 2);
    assert!(with_panel.list_bytes > alone.list_bytes);
    ui.remove(panel);
    ui.frame(at(0.0));
    let after = ui.tree_stats();
    assert_eq!(after.lists, 1);
    // The root's list keeps the capacity it grew to; the panel's is gone.
    assert_eq!(after.list_bytes, root_bytes(&ui));
}

/// A node's properties that follow memos each have a binding, chained from
/// the node, and each is taken from its own: a bound one, set ones, and
/// constants side by side, in a slot a removed node left.
#[test]
fn each_bound_property_follows_its_own_memo() {
    let mut ui = Ui::new(fonts());
    let black = ui.memo(|_| [0.0; 4]);
    let gone = ui.add(ui.root(), fixed(1.0, 1.0).fill(black));
    ui.frame(at(0.0));
    ui.remove(gone);
    let tone = ui.signal([0.5f32; 4]);
    let shade = ui.memo(move |r| r.get(tone));
    let node = ui.add(ui.root(), fixed(4.0, 3.0).fill(shade));
    assert_eq!(node.index, gone.index, "the slot is reused");
    ui.set_width(node, Size::Fixed(6.0));
    ui.set_height(node, Size::Fixed(5.0));
    assert_eq!(ui.bindings.len(), 3);
    ui.frame(at(0.0));
    assert_eq!(ui.rect(node).1, [6.0, 5.0]);
    ui.set_height(node, Size::Fixed(7.0));
    ui.graph().set(tone, [0.25; 4]);
    ui.frame(at(0.0));
    assert_eq!(ui.rect(node).1, [6.0, 7.0]);
    assert_eq!(ui.nodes[node.index as usize].fill, [0.25; 4]);
    // Setting again writes the signal the first set made.
    assert_eq!(ui.bindings.len(), 3);
    check(&mut ui);
    let bound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ui.set_fill(node, [1.0; 4]);
    }));
    assert!(bound.is_err(), "a bound property is not set");
}

#[test]
fn removing_a_subtree_disposes_of_what_it_owns() {
    let mut ui = Ui::new(fonts());
    ui.frame(at(0.0));
    let before = ui.graph().stats();
    let tone = ui.signal([0.5f32; 4]);
    let shade = ui.memo(move |r| r.get(tone));
    let panel = ui.add(ui.root(), Spec::new().fill(shade));
    let child = ui.add(panel, fixed(3.0, 3.0));
    ui.set_fill(child, [1.0; 4]);
    ui.frame(at(0.0));
    // The panel's bound fill and the child's set one.
    assert_eq!(ui.bindings.len(), 2);
    ui.remove(panel);
    ui.frame(at(0.0));
    assert_eq!(ui.bindings.len(), 0);
    let after = ui.graph().stats();
    // The root's signal and memo remain; what the nodes owned is gone.
    assert_eq!(after.nodes, before.nodes + 2);
    assert_eq!(after.edges, before.edges + 1);
    assert_eq!(after.notified, 0);
    assert_eq!(ui.tree_stats().nodes, 1);
    check(&mut ui);
    let stale = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ui.rect(child)));
    assert!(stale.is_err());
}

#[test]
fn a_new_font_generation_measures_every_text_again() {
    let mut ui = Ui::new(fonts());
    grid(&mut ui, 3, 3);
    ui.frame(at(0.0));
    let mut context = at(0.0);
    context.fonts = 1;
    ui.frame(context);
    assert_eq!(ui.frame_stats().measured, 9);
    check(&mut ui);
}

// ---- The frame oracle ----

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }

    fn colour(&mut self) -> [f32; 4] {
        let c = |r: &mut Rng| r.below(5) as f32 * 0.25;
        [c(self), c(self), c(self), self.pick(&[0.0, 0.5, 1.0])]
    }

    /// A point in quarter pixels, over the viewport and a little beyond.
    fn point(&mut self) -> [f32; 2] {
        [
            self.below(2_720) as f32 * 0.25 - 20.0,
            self.below(2_080) as f32 * 0.25 - 20.0,
        ]
    }
}

fn random_size(rng: &mut Rng) -> Size {
    match rng.below(4) {
        0 => Size::Hug,
        1 => Size::Fill(rng.pick(&[0.0, 1.0, 2.0])),
        _ => Size::Fixed(rng.below(40) as f32 * 2.5),
    }
}

fn random_stack(rng: &mut Rng) -> Stack {
    let align = [Align::Start, Align::Center, Align::End];
    Stack {
        direction: if rng.chance(2) {
            crate::Direction::Row
        } else {
            crate::Direction::Column
        },
        gap: rng.pick(&[0.0, 2.0, 3.5]),
        padding: Insets {
            left: rng.pick(&[0.0, 1.0, 4.0]),
            top: rng.pick(&[0.0, 2.5]),
            right: rng.pick(&[0.0, 3.0]),
            bottom: rng.pick(&[0.0, 1.0]),
        },
        main: rng.pick(&align),
        cross: rng.pick(&align),
    }
}

fn random_look(rng: &mut Rng) -> Look {
    Look {
        border: rng.chance(3).then(|| Border {
            width: rng.pick(&[1.0, 2.0]),
            color: rng.colour(),
        }),
        radius: rng.pick(&[0.0, 2.0, 4.5]),
        shadow: rng.chance(4).then(|| BoxShadow {
            offset: [rng.pick(&[-3.0, 0.0, 2.5]), rng.pick(&[0.0, 4.0])],
            blur: rng.pick(&[0.0, 2.0, 6.0]),
            spread: rng.pick(&[0.0, 1.0]),
            color: [0.0, 0.0, 0.0, 0.5],
            inset: false,
        }),
    }
}

const WORDS: [&str; 6] = [
    "a",
    "hi",
    "two words",
    "wrapping text goes on",
    "9",
    "Ünïcødé",
];

fn random_text(rng: &mut Rng) -> Text {
    let mut text = Text::new(rng.pick(&WORDS), rng.pick(&[11.0, 14.0]));
    text.wrap = rng.chance(2);
    text.align = rng.pick(&[TextAlign::default(), TextAlign::Center]);
    text.valign = rng.pick(&[Align::Start, Align::Center, Align::End]);
    text
}

fn random_spec(rng: &mut Rng, leaf: bool) -> Spec {
    let mut spec = Spec::new()
        .width(random_size(rng))
        .height(random_size(rng))
        .stack(random_stack(rng))
        .fill(rng.colour())
        .look(random_look(rng));
    if rng.chance(6) {
        spec = spec.absolute([rng.below(60) as f32 - 20.0, rng.below(60) as f32 - 10.0]);
    }
    if leaf {
        match rng.below(5) {
            0 => spec = spec.text(random_text(rng), rng.colour()),
            1 => {
                let tone = rng.colour();
                spec = spec.custom(
                    Painter::new(
                        move |_| tone,
                        |tone: &[f32; 4], paint| {
                            let [w, h] = paint.size();
                            paint.list().quad(0.5, 0.25, w - 1.0, h * 0.5, *tone);
                            paint.list().circle((w * 0.5, h * 0.5), 2.0, *tone);
                        },
                    )
                    .overflow(Insets::all(1.0)),
                )
            }
            // Changes by the clock: a new shade each whole unit of time,
            // asked for in advance.
            2 => {
                spec = spec.custom(Painter::new(
                    |_| (),
                    |_: &(), paint| {
                        let tick = paint.now().floor();
                        paint.repaint_at(tick + 1.0);
                        let shade = (tick as f32 * 0.3).fract();
                        let [w, h] = paint.size();
                        paint.list().quad(0.0, 0.0, w, h, [shade, 0.2, 0.4, 1.0]);
                    },
                ))
            }
            // Changes with what is outside the tree.
            3 => {
                spec = spec.custom(
                    Painter::new(
                        |_| (),
                        |_: &(), paint| {
                            let at = (OUTSIDE.get() % 7) as f32;
                            paint.list().quad(at, 0.0, 2.0, 2.0, [1.0; 4]);
                        },
                    )
                    .volatile(),
                )
            }
            _ => {}
        }
    } else if rng.chance(3) {
        spec = if rng.chance(2) {
            spec.scroll()
        } else {
            spec.clip()
        };
    }
    spec
}

/// What the random oracle made: its nodes, and the hover and press signals
/// it gave them.
#[derive(Default)]
struct Made {
    nodes: Vec<Node>,
    signals: Vec<(Node, Signal<bool>)>,
}

/// A random subtree under `parent`, `depth` levels at most.
fn random_subtree(ui: &mut Ui, rng: &mut Rng, parent: Node, depth: u32, made: &mut Made) {
    let leaf = depth == 0 || rng.chance(3);
    let mut spec = random_spec(rng, leaf);
    let mut signals = Vec::new();
    if rng.chance(3) {
        let hovered = ui.signal(false);
        signals.push(hovered);
        spec = spec.hover(hovered);
    }
    if rng.chance(4) {
        let pressed = ui.signal(false);
        signals.push(pressed);
        spec = spec.press(pressed);
    }
    let node = ui.add(parent, spec);
    made.nodes.push(node);
    made.signals.extend(signals.into_iter().map(|s| (node, s)));
    if !leaf {
        for _ in 0..1 + rng.below(5) {
            random_subtree(ui, rng, node, depth - 1, made);
        }
    }
}

/// The middle of `node`'s box, in the root's coordinates.
fn middle(ui: &Ui, node: Node) -> [f32; 2] {
    let mut n = node.index;
    let d = &ui.nodes[n as usize];
    let mut at = [d.size[0] * 0.5, d.size[1] * 0.5];
    while let Some(p) = ui.nodes[n as usize].parent() {
        let o = crate::paint::offset(&ui.nodes[n as usize], ui.nodes[p as usize].scrolled());
        at = [at[0] + o[0], at[1] + o[1]];
        n = p;
    }
    at
}

fn is_live(ui: &Ui, node: Node) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ui.rect(node))).is_ok()
}

#[test]
fn every_frame_is_the_tree_drawn_from_nothing() {
    let fonts = fonts();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut patched = 0;
    // Children whose layout a scroll put off, and those of them laid out
    // later, when the scroll brought them near.
    let (mut deferred, mut caught_up) = (0, 0);
    for round in 0..40 {
        let mut ui = Ui::new(fonts.clone());
        let mut put_off: Vec<(u32, bool)> = Vec::new();
        let mut made = Made::default();
        let root = ui.root();
        for _ in 0..3 {
            random_subtree(&mut ui, &mut rng, root, 4, &mut made);
        }
        let mut context = at(0.0);
        ui.frame(context);
        std::panic::set_hook(Box::new(|_| {}));
        for step in 0..30 {
            // One change a frame, or now and then three: a list with more
            // than one child changed.
            let changes = if rng.chance(3) { 3 } else { 1 };
            for _ in 0..changes {
                made.nodes.retain(|&n| is_live(&ui, n));
                let all = &made.nodes;
                let Some(&node) = all.get(rng.below(all.len().max(1) as u64) as usize) else {
                    break;
                };
                match rng.below(14) {
                    0 => ui.set_fill(node, rng.colour()),
                    1 => ui.set_look(node, random_look(&mut rng)),
                    2 => ui.set_width(node, random_size(&mut rng)),
                    3 => ui.set_height(node, random_size(&mut rng)),
                    4 => ui.set_stack(node, random_stack(&mut rng)),
                    5 => {
                        let has_text = ui.nodes[node.index as usize].text.is_some();
                        if has_text {
                            if rng.chance(2) {
                                ui.set_text(node, random_text(&mut rng));
                            } else {
                                ui.set_text_color(node, rng.colour());
                            }
                        }
                    }
                    6 => {
                        let d = &ui.nodes[node.index as usize];
                        if d.text.is_none() && d.custom.is_none() {
                            random_subtree(&mut ui, &mut rng, node, 2, &mut made);
                        }
                    }
                    7 => match ui.hovered() {
                        // Now and then what the pointer is on.
                        Some(hovered) if hovered != root && rng.chance(2) => ui.remove(hovered),
                        _ => ui.remove(node),
                    },
                    8 => {
                        if ui.nodes[node.index as usize].scroll.is_some() {
                            ui.scroll_by(node, rng.below(80) as f32 - 30.0);
                        }
                    }
                    9 => {
                        if ui.nodes[node.index as usize].offset.is_some() {
                            ui.set_offset(node, [rng.below(80) as f32 - 20.0, 3.0]);
                        }
                    }
                    11 => {
                        // Half the time into the middle of a node.
                        let [x, y] = if rng.chance(2) {
                            middle(&ui, node)
                        } else {
                            rng.point()
                        };
                        ui.pointer_moved(x, y);
                    }
                    12 => {
                        let delta = rng.below(90) as f32 - 30.0;
                        if rng.chance(2) {
                            ui.wheel(0.0, delta);
                        } else {
                            ui.wheel(delta, 0.0);
                        }
                    }
                    13 => {
                        let button = rng.pick(&[Button::Primary, Button::Secondary]);
                        match rng.below(5) {
                            0 => ui.pointer_left(),
                            1 => ui.cancel_pointer(),
                            _ => ui.pointer_button(button, rng.chance(2)),
                        }
                    }
                    _ => {
                        context.viewport = [
                            rng.pick(&[640.0, 300.0, 151.5]),
                            rng.pick(&[480.0, 200.0, 97.0]),
                        ];
                    }
                }
            }
            OUTSIDE.set(OUTSIDE.get() + 1);
            context.now += rng.pick(&[0.0, 0.4, 1.0, 2.5]);
            ui.frame(context);
            patched += ui.frame_stats().patched;
            deferred += ui.frame_stats().deferred;
            put_off.resize(ui.nodes.len(), (0, false));
            for (d, was) in ui.nodes.iter().zip(&mut put_off) {
                if d.live && was.0 == d.generation && was.1 && !d.deferred {
                    caught_up += 1;
                }
                *was = (d.generation, d.live && d.deferred);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&mut ui)));
            if let Err(e) = result {
                std::panic::set_hook(quiet);
                let message = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                panic!("round {round}, step {step}: {message}");
            }
            // A removed node is neither hovered nor pressed: its signals,
            // which outlive it, say so.
            for &(node, signal) in &made.signals {
                if !is_live(&ui, node) {
                    assert!(
                        !ui.get(signal),
                        "round {round}, step {step}: a removed node's signal"
                    );
                }
            }
            // Hit testing, by binary search and pruning, finds what a scan
            // of every node finds.
            for _ in 0..8 {
                let p = rng.point();
                for want in [crate::input::takes_pointer, |_: &crate::ui::NodeData| true] {
                    assert_eq!(
                        ui.hit(p, want),
                        ui.hit_from_nothing(p, want),
                        "round {round}, step {step}: the hit at {p:?}"
                    );
                }
            }
            // A frame after it with nothing changed does nothing.
            if !ui.needs_frame(context.now) {
                ui.frame(context);
                let stats = ui.frame_stats();
                assert_eq!(
                    (stats.laid_out, stats.recorded, stats.painted),
                    (0, 0, 0),
                    "round {round}, step {step}: a frame with nothing changed"
                );
            }
        }
    }
    std::panic::set_hook(quiet);
    // Lists kept their shape and were patched in their ancestors', so the
    // oracle checked patching too.
    assert!(patched > 0, "nothing was patched");
    // Scrolls put layouts off and caught up with them, so the oracle checked
    // that too.
    assert!(
        deferred > 0 && caught_up > 0,
        "{deferred} put off, {caught_up} caught up"
    );
}

/// A random row for `scroll_of_rows`, stacked along axis `m`: mostly one its
/// scroll can defer (a fixed length along the stack), now and then one it
/// cannot. One that does not clip has nothing in it, so what it draws
/// reaches no further than its shadow, well inside the viewport the scroll
/// looks past its view.
fn random_row(ui: &mut Ui, rng: &mut Rng, list: Node, m: usize, made: &mut Made) -> Node {
    let clips = rng.below(3) > 0;
    let row = ui.add(list, random_row_spec(rng, m, clips));
    if clips {
        for _ in 0..rng.below(3) {
            random_subtree(ui, rng, row, 2, made);
        }
    }
    row
}

fn random_row_spec(rng: &mut Rng, m: usize, clips: bool) -> Spec {
    let mut sizes = [Size::Hug; 2];
    sizes[m] = if rng.chance(10) {
        random_size(rng)
    } else {
        Size::Fixed(rng.below(25) as f32 * 2.5)
    };
    sizes[1 - m] = random_size(rng);
    let spec = Spec::new()
        .width(sizes[0])
        .height(sizes[1])
        .stack(random_stack(rng))
        .fill(rng.colour())
        .look(random_look(rng));
    if clips { spec.clip() } else { spec }
}

/// A scroll, 100 to 300 pixels along its stack, of 30 to 180 random rows.
/// Its sizes may change to any (`random_size`), a few pixels included: what
/// a row draws past its box (a shadow, 15 pixels at most) still does not
/// reach the view from where the tree that defers stops laying rows out,
/// half the window (48.5 pixels at least) past it.
fn scroll_of_rows(ui: &mut Ui, rng: &mut Rng) -> (Node, Vec<(Node, bool)>, Made) {
    let stack = random_stack(rng);
    let m = stack.direction.main();
    let mut sizes = [random_size(rng); 2];
    sizes[m] = Size::Fixed(100.0 + rng.below(5) as f32 * 50.0);
    let list = ui.add(
        ui.root(),
        Spec::new()
            .width(sizes[0])
            .height(sizes[1])
            .stack(stack)
            .scroll(),
    );
    let mut made = Made::default();
    let mut rows = Vec::new();
    for _ in 0..30 + rng.below(150) {
        let row = random_row(ui, rng, list, m, &mut made);
        rows.push((row, ui.nodes[row.index as usize].clip));
    }
    (list, rows, made)
}

/// Make a change to both trees alike.
fn both(a: &mut Ui, b: &mut Ui, mut change: impl FnMut(&mut Ui)) {
    change(a);
    change(b);
}

/// A scroll of random rows built twice, once putting off the layout of the
/// rows away from its view and once laying every row out: through random
/// scrolls and changes (to the rows, what is in them, the scroll's stack
/// and size, the viewport), the two draw the same and hit the same, and the
/// one that defers is the tree laid out and drawn from nothing.
#[test]
fn a_scroll_that_puts_rows_off_draws_what_one_laying_them_out_draws() {
    let fonts = fonts();
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let (mut deferred, mut caught_up, mut mixed) = (0, 0, 0);
    for round in 0..30 {
        let seed = rng.next() | 1;
        let build = |no_defer: bool| {
            let mut ui = Ui::new(fonts.clone());
            ui.no_defer = no_defer;
            let built = scroll_of_rows(&mut ui, &mut Rng(seed));
            (ui, built)
        };
        let (mut a, (list, mut rows, mut made)) = build(false);
        let (mut b, _) = build(true);
        let mut context = at(0.0);
        let mut put_off: Vec<(u32, bool)> = Vec::new();
        for step in 0..40 {
            let changes = if rng.chance(3) { 3 } else { 1 };
            for _ in 0..changes {
                rows.retain(|&(n, _)| is_live(&a, n));
                made.nodes.retain(|&n| is_live(&a, n));
                let m = a.nodes[list.index as usize].stack.direction.main();
                let row = rows
                    .get(rng.below(rows.len().max(1) as u64) as usize)
                    .copied();
                let inner = made
                    .nodes
                    .get(rng.below(made.nodes.len().max(1) as u64) as usize)
                    .copied();
                let pick = rng.below(13);
                match pick {
                    0 | 1 => {
                        let delta = rng.below(600) as f32 - 300.0;
                        both(&mut a, &mut b, |ui| ui.scroll_by(list, delta));
                    }
                    2 => {
                        let to = rng.below(8_000) as f32;
                        both(&mut a, &mut b, |ui| ui.scroll_to(list, to));
                    }
                    3 | 4 => {
                        if let Some((row, _)) = row {
                            let size = if rng.chance(8) {
                                random_size(&mut rng)
                            } else {
                                Size::Fixed(rng.below(25) as f32 * 2.5)
                            };
                            let along = rng.chance(2);
                            both(&mut a, &mut b, |ui| {
                                if along == (m == 0) {
                                    ui.set_width(row, size);
                                } else {
                                    ui.set_height(row, size);
                                }
                            });
                        }
                    }
                    5 => {
                        if let Some((row, _)) = row {
                            let (fill, look) = (rng.colour(), random_look(&mut rng));
                            both(&mut a, &mut b, |ui| {
                                ui.set_fill(row, fill);
                                ui.set_look(row, look);
                            });
                        }
                    }
                    6 => {
                        if let Some(node) = inner {
                            let is_text = a.nodes[node.index as usize].text.is_some();
                            let text = random_text(&mut rng);
                            let size = random_size(&mut rng);
                            both(&mut a, &mut b, |ui| {
                                if is_text {
                                    ui.set_text(node, text.clone());
                                } else {
                                    ui.set_height(node, size);
                                }
                            });
                        }
                    }
                    7 => {
                        // Into a row that clips, or something in one.
                        let target = match (row, inner) {
                            (Some((row, true)), _) if rng.chance(2) => Some(row),
                            (_, Some(node)) => Some(node),
                            _ => None,
                        };
                        if let Some(node) = target {
                            let d = &a.nodes[node.index as usize];
                            if d.text.is_none() && d.custom.is_none() {
                                let seed = rng.next() | 1;
                                let mut added = Made::default();
                                for ui in [&mut a, &mut b] {
                                    added = Made::default();
                                    random_subtree(ui, &mut Rng(seed), node, 2, &mut added);
                                }
                                made.nodes.extend(added.nodes);
                            }
                        }
                    }
                    8 => {
                        let target = if rng.chance(2) {
                            row.map(|r| r.0)
                        } else {
                            inner
                        };
                        if let Some(node) = target {
                            both(&mut a, &mut b, |ui| ui.remove(node));
                        }
                    }
                    9 => {
                        let seed = rng.next() | 1;
                        let mut added = None;
                        for ui in [&mut a, &mut b] {
                            let mut made_here = Made::default();
                            let row = random_row(ui, &mut Rng(seed), list, m, &mut made_here);
                            added = Some((row, ui.nodes[row.index as usize].clip, made_here));
                        }
                        let (row, clips, made_here) = added.expect("added");
                        rows.push((row, clips));
                        made.nodes.extend(made_here.nodes);
                    }
                    10 => {
                        let stack = random_stack(&mut rng);
                        both(&mut a, &mut b, |ui| ui.set_stack(list, stack));
                    }
                    11 => {
                        let size = random_size(&mut rng);
                        let width = rng.chance(2);
                        both(&mut a, &mut b, |ui| {
                            if width {
                                ui.set_width(list, size);
                            } else {
                                ui.set_height(list, size);
                            }
                        });
                    }
                    _ => {
                        context.viewport = [
                            rng.pick(&[640.0, 300.0, 151.5]),
                            rng.pick(&[480.0, 200.0, 97.0]),
                        ];
                    }
                }
            }
            OUTSIDE.set(OUTSIDE.get() + 1);
            context.now += rng.pick(&[0.0, 0.4, 1.0]);
            a.frame(context);
            b.frame(context);
            deferred += a.frame_stats().deferred;
            mixed += u32::from(a.nodes[list.index as usize].scroll.as_ref().unwrap().mixed);
            put_off.resize(a.nodes.len(), (0, false));
            for (d, was) in a.nodes.iter().zip(&mut put_off) {
                if d.live && was.0 == d.generation && was.1 && !d.deferred {
                    caught_up += 1;
                }
                *was = (d.generation, d.live && d.deferred);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (fa, fb) = (a.list().expect("drawn"), b.list().expect("drawn"));
                if let Some(difference) = fa.difference(fb) {
                    panic!("the frame differs from the one laying every row out: {difference}");
                }
                for _ in 0..8 {
                    let p = rng.point();
                    assert_eq!(
                        a.hit(p, |_| true),
                        b.hit(p, |_| true),
                        "the hit at {p:?} differs"
                    );
                }
                check(&mut a);
            }));
            if let Err(e) = result {
                std::panic::set_hook(quiet);
                let message = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                panic!("round {round}, step {step}: {message}");
            }
        }
    }
    std::panic::set_hook(quiet);
    // Rows were put off by the thousand and caught up with by the hundred,
    // with rows the scroll cannot defer among them now and then.
    assert!(
        deferred > 10_000 && caught_up > 1_000 && mixed > 100,
        "{deferred} put off, {caught_up} caught up, {mixed} frames mixed"
    );
}

/// A node holds its properties' values and nothing for each that is a
/// constant: what a bound one needs is in `Ui::bindings`, and the log of a
/// kept list is beside the list. A field that grows it past this bound is a
/// deliberate change, made with the bound.
#[test]
fn a_node_stays_small() {
    let size = std::mem::size_of::<crate::ui::NodeData>();
    assert!(size <= 384, "NodeData is {size} bytes");
}
