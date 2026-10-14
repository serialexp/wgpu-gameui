use super::*;

fn rows(heights: &[f32]) -> RowHeights {
    let mut rows = RowHeights::new();
    rows.extend(heights.iter().map(|&h| Row::estimated(h)));
    rows
}

#[test]
fn a_row_starts_where_the_rows_above_it_end() {
    let rows = rows(&[10.0, 20.0, 30.0]);
    assert_eq!(rows.offset_of(0), 0.0);
    assert_eq!(rows.offset_of(1), 10.0);
    assert_eq!(rows.offset_of(2), 30.0);
    assert_eq!(rows.offset_of(3), 60.0);
    assert_eq!(rows.total(), 60.0);
}

#[test]
fn the_row_at_a_position_is_the_one_covering_it() {
    let rows = rows(&[10.0, 20.0, 30.0]);
    assert_eq!(rows.row_at(0.0), Some(0));
    assert_eq!(rows.row_at(9.9), Some(0));
    assert_eq!(rows.row_at(10.0), Some(1));
    assert_eq!(rows.row_at(59.9), Some(2));
    assert_eq!(rows.row_at(60.0), None, "past the last row");
    assert_eq!(rows.row_at(-0.1), None, "above the first");
    assert_eq!(RowHeights::new().row_at(0.0), None);
}

#[test]
fn a_row_of_no_height_covers_nothing() {
    // agent-ui hides a tool result drawn inside its call's row.
    let rows = rows(&[10.0, 0.0, 0.0, 30.0]);
    assert_eq!(rows.row_at(10.0), Some(3));
    assert_eq!(rows.offset_of(3), 10.0);
}

#[test]
fn measuring_a_row_moves_only_the_rows_below_it() {
    let mut rows = rows(&[10.0, 20.0, 30.0]);
    assert!(!rows.is_measured(1));
    rows.set_measured(1, 50.0);
    assert!(rows.is_measured(1));
    assert_eq!(rows.height_of(1), 50.0);
    assert_eq!(rows.offset_of(1), 10.0);
    assert_eq!(rows.offset_of(2), 60.0);
    assert_eq!(rows.total(), 90.0);
}

#[test]
fn a_page_put_in_front_moves_the_rows_down_with_their_heights() {
    let mut rows = rows(&[10.0, 20.0]);
    rows.set_measured(1, 25.0);
    rows.splice(0..0, [Row::estimated(5.0), Row::estimated(7.0)]);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows.offset_of(2), 12.0);
    assert_eq!(rows.height_of(3), 25.0);
    assert!(rows.is_measured(3));
    assert!(!rows.is_measured(0));
    assert_eq!(rows.row_at(12.0), Some(2));
}

#[test]
fn rows_taken_out_of_the_middle_close_the_gap() {
    let mut rows = rows(&[10.0, 20.0, 30.0, 40.0]);
    rows.splice(1..3, [Row::estimated(1.0)]);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.offset_of(2), 11.0);
    assert_eq!(rows.total(), 51.0);
}

#[test]
fn a_replaced_row_is_estimated_again() {
    let mut rows = rows(&[10.0, 20.0]);
    rows.set_measured(0, 12.0);
    rows.replace(0, Row::estimated(40.0));
    assert!(!rows.is_measured(0));
    assert_eq!(rows.offset_of(1), 40.0);
}

#[test]
fn a_measured_height_is_remembered_by_key_across_a_rebuild() {
    let mut rows = RowHeights::new();
    rows.push(Row::estimated(10.0).keyed(7));
    rows.push(Row::estimated(10.0).keyed(8));
    rows.set_measured(0, 33.0);
    rows.clear();
    rows.push(Row::estimated(10.0).keyed(8));
    rows.push(Row::estimated(10.0).keyed(7));
    assert!(!rows.is_measured(0));
    assert!(rows.is_measured(1));
    assert_eq!(rows.height_of(1), 33.0);
}

#[test]
fn a_height_measured_elsewhere_finds_its_row_after_pages_go_in_front() {
    let mut rows = RowHeights::new();
    rows.extend((0..10).map(|key| Row::estimated(10.0).keyed(key)));
    let ticket = rows.ticket(4);
    rows.splice(0..0, (100..105).map(|key| Row::estimated(10.0).keyed(key)));
    let front = rows.ticket(1);
    rows.splice(0..0, (200..203).map(|key| Row::estimated(10.0).keyed(key)));
    rows.apply_measured(ticket, 44.0);
    rows.apply_measured(front, 11.0);
    assert_eq!(rows.height_of(3 + 5 + 4), 44.0);
    assert_eq!(rows.height_of(3 + 1), 11.0);
    assert_eq!(rows.index_of(ticket.id()), Some(12));
}

#[test]
fn a_row_measured_here_keeps_its_newer_measurement() {
    let mut rows = RowHeights::new();
    rows.extend((0..3).map(|key| Row::estimated(10.0).keyed(key)));
    let ticket = rows.ticket(1);
    rows.set_measured(1, 15.0);
    rows.apply_measured(ticket, 99.0);
    assert_eq!(rows.height_of(1), 15.0);
}

#[test]
fn a_height_for_content_that_changed_is_not_taken() {
    let mut rows = RowHeights::new();
    rows.extend((0..3).map(|key| Row::estimated(10.0).keyed(key)));
    let ticket = rows.ticket(1);
    rows.replace(1, Row::estimated(10.0).keyed(77));
    rows.apply_measured(ticket, 99.0);
    assert!(!rows.is_measured(1));
    // It is remembered for the old content, should that come back.
    rows.replace(1, Row::estimated(10.0).keyed(1));
    assert_eq!(rows.height_of(1), 99.0);
}

#[test]
fn a_height_for_a_row_gone_waits_for_its_key_to_come_back() {
    let mut rows = RowHeights::new();
    rows.extend((0..3).map(|key| Row::estimated(10.0).keyed(key)));
    let ticket = rows.ticket(2);
    rows.clear();
    rows.extend((0..2).map(|key| Row::estimated(10.0).keyed(key + 10)));
    rows.apply_measured(ticket, 30.0);
    assert_eq!(
        rows.index_of(ticket.id()),
        None,
        "a cleared list's ids find nothing"
    );
    assert!(!rows.is_measured(0) && !rows.is_measured(1));
    rows.push(Row::estimated(10.0).keyed(2));
    assert!(rows.is_measured(2));
    assert_eq!(rows.height_of(2), 30.0);
}

#[test]
fn a_change_in_the_middle_gives_new_ids_and_keeps_measurements() {
    let mut rows = RowHeights::new();
    rows.extend((0..6).map(|key| Row::estimated(10.0).keyed(key)));
    rows.splice(0..0, [Row::estimated(10.0).keyed(50)]);
    rows.set_measured(3, 30.0); // key 2
    let old = rows.ticket(5);
    rows.splice(1..3, [Row::estimated(5.0).keyed(70)]); // keys 0 and 1 go
    assert_eq!(rows.len(), 6);
    assert_eq!(rows.height_of(2), 30.0);
    assert!(rows.is_measured(2));
    assert_eq!(rows.index_of(old.id()), None);
    rows.apply_measured(old, 44.0);
    // Key 4 is remembered, not applied, until a row of it comes along.
    assert!(!rows.is_measured(4));
}

#[test]
fn a_new_width_makes_every_height_an_estimate_and_forgets_the_remembered() {
    let mut rows = RowHeights::new();
    rows.push(Row::estimated(10.0).keyed(1));
    rows.push(Row::estimated(10.0));
    rows.set_measured(0, 12.0);
    rows.reestimate(|index| match index {
        0 => Row::estimated(100.0).keyed(1),
        _ => Row::measured(0.0),
    });
    assert!(!rows.is_measured(0));
    assert_eq!(rows.height_of(0), 100.0);
    assert!(rows.is_measured(1), "a row known regardless stays measured");
    assert_eq!(rows.total(), 100.0);
    rows.clear();
    rows.push(Row::estimated(10.0).keyed(1));
    assert!(!rows.is_measured(0), "the old width's height is gone");
}

#[test]
fn rows_to_measure_come_nearest_first() {
    let mut rows = rows(&[1.0; 8]);
    rows.set_measured(4, 1.0);
    let order: Vec<usize> = rows.unmeasured_near(3).collect();
    assert_eq!(order, [3, 2, 5, 1, 6, 0, 7]);
    assert_eq!(
        rows.unmeasured_near(8).collect::<Vec<_>>(),
        [7, 6, 5, 3, 2, 1, 0]
    );
}

/// A small deterministic generator, so the model tests need no crate.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// Whole-pixel heights, a tenth of them zero, so sums are exact.
    fn height(&mut self) -> f32 {
        if self.below(10) == 0 {
            0.0
        } else {
            (1 + self.below(400)) as f32
        }
    }
}

/// Both index layouts agree with a plain list of heights through a long run
/// of random edits, at sizes that span several chunks.
fn agrees_with_a_plain_list<I: HeightIndex>() {
    let mut random = Lcg(42);
    let mut model: Vec<f32> = (0..700).map(|_| random.height()).collect();
    let mut index = I::from_heights(model.clone());
    for step in 0..3000 {
        match random.below(6) {
            0 => {
                let h = random.height();
                model.push(h);
                index.push(h);
            }
            1 => {
                let at = random.below(model.len());
                let h = random.height();
                model[at] = h;
                index.set(at, h);
            }
            2 => {
                let start = random.below(model.len() + 1);
                let end = (start + random.below(40)).min(model.len());
                let new: Vec<f32> = (0..random.below(600)).map(|_| random.height()).collect();
                model.splice(start..end, new.iter().copied());
                index.splice(start..end, new.into_iter());
            }
            3 => {
                let new: Vec<f32> = (0..random.below(600)).map(|_| random.height()).collect();
                model.splice(0..0, new.iter().copied());
                index.prepend(&new);
            }
            4 => {
                let new: Vec<f32> = (0..random.below(600)).map(|_| random.height()).collect();
                model.extend_from_slice(&new);
                index.extend(&new);
            }
            _ => {}
        }
        assert_eq!(index.len(), model.len(), "step {step}");
        let at = random.below(model.len() + 1);
        let expected: f64 = model[..at].iter().map(|&h| f64::from(h)).sum();
        assert_eq!(index.prefix(at), expected, "prefix({at}) at step {step}");
        if at < model.len() {
            assert_eq!(index.get(at), model[at]);
        }
        let total: f64 = model.iter().map(|&h| f64::from(h)).sum();
        assert_eq!(index.total(), total);
        let y = random.below(total as usize + 1) as f64;
        let mut end = 0.0;
        let found = model
            .iter()
            .position(|&h| {
                end += f64::from(h);
                end > y
            })
            .unwrap_or(model.len());
        assert_eq!(index.find(y), found, "find({y}) at step {step}");
    }
}

#[test]
fn the_fenwick_index_agrees_with_a_plain_list() {
    agrees_with_a_plain_list::<Fenwick>();
}

#[test]
fn the_double_ended_index_agrees_with_a_plain_list() {
    agrees_with_a_plain_list::<Deque>();
}

/// What each index layout costs at agent-ui's scale: a long conversation
/// loaded, pages put in front, a row measured per frame, and the lookups a
/// frame makes; then the same through [`RowHeights`], keys and all. Prints
/// only; run with `--ignored --nocapture`, in release.
#[test]
#[ignore = "measurement: prints timings, asserts nothing"]
fn compare_index_layouts() {
    use std::time::Instant;
    const ROWS: usize = 50_000;
    const PAGES: usize = 20;
    const PAGE: usize = 500;
    const FRAMES: u32 = 10_000;

    fn run<I: HeightIndex>(name: &str) {
        let mut random = Lcg(7);
        let load: Vec<f32> = (0..ROWS).map(|_| random.height()).collect();
        let pages: Vec<Vec<f32>> = (0..PAGES)
            .map(|_| (0..PAGE).map(|_| random.height()).collect())
            .collect();

        let start = Instant::now();
        let mut index = I::default();
        index.extend(&load);
        let loaded = start.elapsed();

        let start = Instant::now();
        for page in &pages {
            index.prepend(page);
        }
        let prepended = start.elapsed();

        let start = Instant::now();
        let mut sink = 0.0;
        for _ in 0..FRAMES {
            let at = random.below(index.len());
            index.set(at, random.height());
            let y = random.below(index.total() as usize) as f64;
            let row = index.find(y);
            for visible in row..(row + 20).min(index.len()) {
                sink += index.prefix(visible);
            }
        }
        let frames = start.elapsed();
        println!(
            "{name:>11}: load {ROWS} {loaded:?}, a page of {PAGE} in front {:?}, a frame {:?} [{sink}]",
            prepended / PAGES as u32,
            frames / FRAMES,
        );
    }
    run::<Fenwick>("fenwick");
    run::<Deque>("double-end");

    let mut random = Lcg(7);
    let mut key = 0u64;
    let mut keyed = |height: f32| {
        key += 1;
        Row::estimated(height).keyed(key.wrapping_mul(0x2545_f491_4f6c_dd1d))
    };
    let load: Vec<Row> = (0..ROWS).map(|_| keyed(random.height())).collect();
    let pages: Vec<Vec<Row>> = (0..PAGES)
        .map(|_| (0..PAGE).map(|_| keyed(random.height())).collect())
        .collect();
    let mut rows = RowHeights::new();
    let start = Instant::now();
    rows.extend(load);
    let loaded = start.elapsed();
    let start = Instant::now();
    for page in pages {
        rows.splice(0..0, page);
    }
    let prepended = start.elapsed();
    let start = Instant::now();
    let mut sink = 0.0;
    for _ in 0..FRAMES {
        let at = random.below(rows.len());
        rows.set_measured(at, random.height());
        let y = random.below(rows.total() as usize) as f64;
        let row = rows.row_at(y).unwrap_or(0);
        for visible in row..(row + 20).min(rows.len()) {
            sink += rows.offset_of(visible);
        }
    }
    let frames = start.elapsed();
    println!(
        "RowHeights: load {ROWS} {loaded:?}, a page of {PAGE} in front {:?}, a frame {:?} [{sink}]",
        prepended / PAGES as u32,
        frames / FRAMES,
    );
}

#[test]
fn a_new_width_keeps_the_heights_as_estimates() {
    let mut rows = RowHeights::new();
    rows.extend((0..3).map(|key| Row::estimated(10.0).keyed(key)));
    assert!(
        !rows.set_width(300.0),
        "the first width throws nothing away"
    );
    rows.set_measured(1, 40.0);
    assert!(!rows.set_width(300.0), "the same width changes nothing");
    assert!(rows.is_measured(1));
    assert!(rows.set_width(200.0));
    assert!(!rows.is_measured(1));
    assert_eq!(rows.height_of(1), 40.0, "kept, as a guess");
    rows.clear();
    rows.push(Row::estimated(10.0).keyed(1));
    assert!(!rows.is_measured(0), "the old width's height is gone");
}

#[test]
fn a_height_measured_for_another_width_or_fonts_is_dropped() {
    let mut rows = RowHeights::new();
    rows.set_width(100.0);
    rows.extend((0..3).map(|key| Row::estimated(10.0).keyed(key)));
    let before_resize = rows.ticket(1);
    rows.set_width(50.0);
    rows.apply_measured(before_resize, 99.0);
    assert!(!rows.is_measured(1));
    rows.push(Row::estimated(10.0).keyed(1));
    assert!(!rows.is_measured(3), "nor remembered");
    let before_fonts = rows.ticket(2);
    rows.reestimate(|index| Row::estimated(10.0).keyed(index as u64));
    rows.apply_measured(before_fonts, 99.0);
    assert!(!rows.is_measured(2));
    let now = rows.ticket(2);
    rows.apply_measured(now, 30.0);
    assert_eq!(rows.height_of(2), 30.0);
}

#[test]
fn ids_from_one_list_find_no_row_in_another() {
    let mut a = RowHeights::new();
    let mut b = RowHeights::new();
    a.extend((0..5).map(|_| Row::estimated(10.0)));
    b.extend((0..5).map(|_| Row::estimated(10.0)));
    assert_eq!(b.index_of(a.id_of(2)), None);
    assert_eq!(a.index_of(b.id_of(2)), None);
}
