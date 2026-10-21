use super::tests_support::{Rng, Unit, random_units};
use super::*;
use crate::render::draw_box::{Target, analytic};
use crate::widgets::AnalyticInstance;

const A: BatchKey = BatchKey(0);
const B: BatchKey = BatchKey(1);
const C: BatchKey = BatchKey(2);

fn px(x0: u32, y0: u32, x1: u32, y1: u32) -> PixelBox {
    PixelBox { x0, y0, x1, y1 }
}

/// Orders `units` on a `size` target, checks the grid agrees with the brute
/// force, and returns the orders.
fn order_on(size: (u32, u32), units: &[Unit]) -> Vec<Option<u32>> {
    let mut order = DrawOrder::default();
    order.begin(size);
    let orders: Vec<_> = units.iter().map(|(k, b)| order.add(*k, b)).collect();
    assert_eq!(
        orders,
        brute_force_orders(units),
        "grid against brute force"
    );
    orders
}

fn order(units: &[Unit]) -> Vec<Option<u32>> {
    order_on((256, 256), units)
}

/// An untransformed chrome record's box at scale 1.
fn chrome(x: f32, y: f32, w: f32, h: f32) -> PixelBox {
    let mut payload = [[0.0; 4]; 10];
    payload[0] = [1.0, 0.0, 0.0, 1.0];
    payload[2] = [x, y, w, h];
    let record = AnalyticInstance {
        payload,
        kind: 0,
        padding: [0; 3],
    };
    analytic(&Target::new((256, 256), 1.0, [0.0, 0.0]), &record)
}

#[test]
fn a_unit_over_an_earlier_one_of_another_key_draws_after_it() {
    let below = px(10, 10, 50, 50);
    let above = px(40, 40, 80, 80);
    assert_eq!(
        order(&[(A, vec![below]), (B, vec![above])]),
        [Some(0), Some(1)]
    );
}

#[test]
fn a_unit_over_an_earlier_one_of_its_own_key_shares_its_order() {
    // Same order and key: the paint index keeps them in order in one call.
    let below = px(10, 10, 50, 50);
    let above = px(40, 40, 80, 80);
    assert_eq!(
        order(&[(A, vec![below]), (A, vec![above])]),
        [Some(0), Some(0)]
    );
    // Over a unit of another key that was itself raised, it climbs with it.
    assert_eq!(
        order(&[
            (A, vec![below]),
            (B, vec![above]),
            (B, vec![px(70, 70, 90, 90)])
        ]),
        [Some(0), Some(1), Some(1)]
    );
}

#[test]
fn units_that_overlap_nothing_share_order_zero() {
    let units: Vec<_> = (0..6)
        .map(|i| {
            let key = [A, B, C][i % 3];
            (key, vec![px(i as u32 * 40, 0, i as u32 * 40 + 30, 30)])
        })
        .collect();
    assert_eq!(order(&units), [Some(0); 6]);
}

#[test]
fn chrome_two_px_apart_doesnt_chain_and_one_px_apart_does() {
    // 10..20, widened 0.61 px: pixels 9..21. At 22 the next starts at 21.
    let left = chrome(10.0, 10.0, 10.0, 10.0);
    let apart = chrome(22.0, 10.0, 10.0, 10.0);
    assert_eq!(
        order(&[(A, vec![left]), (B, vec![apart])]),
        [Some(0), Some(0)]
    );
    // At 21 both reach pixel 20, whose centre lies 0.5 px from each edge.
    let near = chrome(21.0, 10.0, 10.0, 10.0);
    assert_eq!(
        order(&[(A, vec![left]), (B, vec![near])]),
        [Some(0), Some(1)]
    );
}

#[test]
fn a_box_that_is_not_a_number_is_a_barrier_across_the_whole_target() {
    let whole = chrome(f32::NAN, 0.0, 10.0, 10.0);
    assert_eq!(whole, px(0, 0, 256, 256));
    let units = [
        (A, vec![px(0, 0, 10, 10)]),
        (B, vec![whole]),
        (A, vec![px(200, 200, 210, 210)]),
    ];
    assert_eq!(order(&units), [Some(0), Some(1), Some(2)]);
}

#[test]
fn a_unit_whose_boxes_are_all_empty_is_not_ordered() {
    let mut order = DrawOrder::default();
    order.begin((256, 256));
    assert_eq!(order.add(A, &[px(10, 10, 50, 50)]), Some(0));
    assert_eq!(order.add(B, &[PixelBox::EMPTY]), None);
    assert_eq!(order.add(B, &[]), None);
    assert_eq!(order.add(B, &[PixelBox::EMPTY, px(5, 5, 5, 9)]), None);
    // Nothing was stored for them: a later unit isn't raised past them.
    assert_eq!(order.add(A, &[px(10, 10, 50, 50)]), Some(0));
    assert_eq!(order.stats().units_ordered, 2);
}

#[test]
fn a_unit_with_an_empty_box_and_one_on_the_target_is_ordered_by_that_one() {
    let units = [
        (A, vec![px(10, 10, 50, 50)]),
        (B, vec![PixelBox::EMPTY, px(40, 40, 60, 60)]),
    ];
    assert_eq!(order(&units), [Some(0), Some(1)]);
}

#[test]
fn a_unit_raised_by_a_later_box_has_that_order_for_all_of_them() {
    // The text unit's first block overlaps nothing; its second lies over a
    // raised B, so the whole unit draws after it.
    let units = [
        (A, vec![px(100, 100, 140, 140)]),
        (B, vec![px(120, 120, 160, 160)]),
        (C, vec![px(10, 10, 30, 30), px(150, 150, 170, 170)]),
    ];
    assert_eq!(order(&units), [Some(0), Some(1), Some(2)]);
}

#[test]
fn a_unit_over_only_the_first_box_of_a_raised_unit_draws_after_it() {
    // The C unit is raised to 2 by its second box. A later unit touching only
    // its first box must still sort after it: of another key it is raised past
    // it, and of the same key it shares its order (the paint index decides).
    let raised = (C, vec![px(10, 10, 30, 30), px(150, 150, 170, 170)]);
    let earlier = [
        (A, vec![px(100, 100, 140, 140)]),
        (B, vec![px(120, 120, 160, 160)]),
    ];
    let mut units = earlier.to_vec();
    units.push(raised.clone());
    units.push((A, vec![px(20, 20, 40, 40)]));
    assert_eq!(order(&units), [Some(0), Some(1), Some(2), Some(3)]);

    let mut units = earlier.to_vec();
    units.push(raised);
    units.push((C, vec![px(20, 20, 40, 40)]));
    assert_eq!(order(&units), [Some(0), Some(1), Some(2), Some(2)]);
}

#[test]
fn a_box_across_many_cells_finds_what_it_overlaps_in_any_of_them() {
    let units = [
        (A, vec![px(200, 10, 210, 20)]),
        (B, vec![px(0, 0, 256, 30)]),
        (A, vec![px(5, 25, 8, 28)]),
    ];
    assert_eq!(order(&units), [Some(0), Some(1), Some(2)]);
}

#[test]
fn boxes_reaching_past_the_target_are_held_to_its_last_cells() {
    let units = [
        (A, vec![px(250, 250, 400, 400)]),
        (B, vec![px(255, 255, 300, 300)]),
    ];
    assert_eq!(order_on((256, 256), &units), [Some(0), Some(1)]);
}

#[test]
fn ordering_again_starts_afresh_whatever_the_target() {
    let units = [
        (A, vec![px(10, 10, 50, 50)]),
        (B, vec![px(40, 40, 600, 300)]),
        (A, vec![px(500, 250, 520, 260)]),
    ];
    let mut order = DrawOrder::default();
    let mut run = |size| {
        order.begin(size);
        let orders: Vec<_> = units.iter().map(|(k, b)| order.add(*k, b)).collect();
        (orders, order.stats())
    };
    let first = run((640, 320));
    assert_eq!(first.0, [Some(0), Some(1), Some(2)]);
    // A smaller target, then the first again: nothing is left from before.
    assert_eq!(run((64, 64)).0, [Some(0), Some(1), Some(2)]);
    assert_eq!(run((640, 320)), first);
}

#[test]
fn orders_match_the_brute_force_on_random_layouts() {
    let mut rng = Rng(7);
    for layout in 0..300 {
        let (size, units) = random_units(&mut rng);
        let mut order = DrawOrder::default();
        order.begin(size);
        let orders: Vec<_> = units.iter().map(|(k, b)| order.add(*k, b)).collect();
        assert_eq!(orders, brute_force_orders(&units), "layout {layout}");
        let ordered = orders.iter().filter(|o| o.is_some()).count();
        assert_eq!(order.stats().units_ordered as usize, ordered);
    }
}

#[test]
fn units_of_one_key_are_ordered_without_the_grid() {
    let mut rng = Rng(11);
    let (size, mut units) = random_units(&mut rng);
    for unit in &mut units {
        unit.0 = A;
    }
    let mut order = DrawOrder::default();
    order.begin(size);
    let orders: Vec<_> = units.iter().map(|(k, b)| order.add(*k, b)).collect();
    assert_eq!(orders, brute_force_orders(&units));
    assert!(order.one_key());
    let stats = order.stats();
    assert_eq!(
        (stats.cell_visits, stats.overlap_tests),
        (0, 0),
        "{stats:?}"
    );
    let ordered = orders.iter().filter(|o| o.is_some()).count();
    assert_eq!(stats.units_ordered as usize, ordered);
}

#[test]
fn a_second_key_after_a_run_of_one_is_ordered_over_the_run() {
    // Units of A over one another, then a B over the first, then an A over
    // the B: the run is in the grid by the time the B looks.
    let units = [
        (A, vec![px(10, 10, 50, 50)]),
        (A, vec![px(30, 30, 70, 70)]),
        (A, vec![PixelBox::EMPTY]),
        (B, vec![px(0, 0, 20, 20)]),
        (A, vec![px(15, 15, 18, 18)]),
        (A, vec![px(200, 200, 210, 210)]),
    ];
    assert_eq!(
        order(&units),
        [Some(0), Some(0), None, Some(1), Some(2), Some(0)]
    );
}

#[test]
fn orders_match_the_brute_force_when_a_long_run_of_one_key_comes_first() {
    let mut rng = Rng(13);
    for layout in 0..300 {
        let (size, mut units) = random_units(&mut rng);
        let run = rng.below(units.len() as u32 + 1) as usize;
        let first = units[0].0;
        for unit in &mut units[..run] {
            unit.0 = first;
        }
        let mut order = DrawOrder::default();
        order.begin(size);
        let orders: Vec<_> = units.iter().map(|(k, b)| order.add(*k, b)).collect();
        assert_eq!(orders, brute_force_orders(&units), "layout {layout}");
    }
}

/// `rows` list rows in ten side-by-side columns, as tall as they need to be,
/// so a longer list changes only the height: each row a background, a label
/// inside it and an icon at its end, with 2 px between rows and 4 between
/// columns.
fn list(rows: u32) -> ((u32, u32), Vec<Unit>) {
    const ROW: u32 = 24;
    const WIDTH: u32 = 300;
    const COLUMNS: u32 = 10;
    let per_column = rows.div_ceil(COLUMNS);
    let size = (COLUMNS * (WIDTH + 4), per_column * ROW);
    let mut units = Vec::new();
    for r in 0..rows {
        let (x, y) = ((r / per_column) * (WIDTH + 4), (r % per_column) * ROW);
        units.push((A, vec![px(x, y, x + WIDTH, y + ROW - 2)]));
        units.push((B, vec![px(x + 8, y + 3, x + 200, y + ROW - 5)]));
        units.push((C, vec![px(x + WIDTH - 20, y + 3, x + WIDTH - 4, y + 19)]));
    }
    (size, units)
}

#[test]
fn a_list_takes_a_fixed_amount_of_work_per_unit_at_1k_and_10k_units() {
    for rows in [334, 3334] {
        let (size, units) = list(rows);
        let mut order = DrawOrder::default();
        order.begin(size);
        let orders: Vec<_> = units.iter().map(|(k, b)| order.add(*k, b)).collect();
        let stats = order.stats();
        // Every unit is on the target and ordered: none were culled away.
        assert_eq!(stats.units_ordered as usize, units.len());
        // Backgrounds, labels and icons: three levels, whatever the length.
        assert!(orders.iter().all(|o| o.is_some_and(|o| o <= 2)));
        let n = units.len() as u64;
        eprintln!(
            "{n} units: {} overlap tests, {} cell visits",
            stats.overlap_tests, stats.cell_visits
        );
        // About 8 tests and 9 visits a unit at either length: a background
        // overlaps nothing in its cells, so it is tested against all of them.
        assert!(stats.overlap_tests <= 10 * n, "{stats:?} for {n} units");
        assert!(stats.cell_visits <= 10 * n, "{stats:?} for {n} units");
    }
}
