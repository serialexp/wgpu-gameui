use std::cell::Cell;
use std::rc::Rc;

use super::*;

/// A counter a memo's closure bumps each run, to see which ran.
fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let c = Rc::new(Cell::new(0));
    (c.clone(), c)
}

#[test]
fn a_memo_runs_when_first_read_and_again_only_after_a_change() {
    let mut g = Graph::new();
    let s = g.signal(2);
    let (runs, seen) = counter();
    let double = g.memo(move |r| {
        runs.set(runs.get() + 1);
        r.get(s) * 2
    });
    assert_eq!(seen.get(), 0, "memos are lazy");
    assert_eq!(g.get(double), 4);
    assert_eq!(g.get(double), 4);
    assert_eq!(seen.get(), 1);
    g.set(s, 5);
    assert_eq!(seen.get(), 1, "a write runs nothing");
    assert_eq!(g.get(double), 10);
    assert_eq!(seen.get(), 2);
}

#[test]
fn setting_an_equal_value_marks_nothing() {
    let mut g = Graph::new();
    let s = g.signal(String::from("a"));
    let m = g.memo(move |r| r.get(s).len());
    g.observe(m, 1);
    let mut keys = Vec::new();
    g.take_notified(&mut keys);
    assert_eq!(g.get(m), 1);
    let runs = g.stats().runs;
    assert!(!g.set(s, String::from("a")));
    g.take_notified(&mut keys);
    assert_eq!(keys, [1], "only the first, unrun, report");
    assert_eq!(g.get(m), 1);
    assert_eq!(g.stats().runs, runs);
}

#[test]
fn a_diamond_runs_each_memo_once() {
    let mut g = Graph::new();
    let s = g.signal(1);
    let (a_runs, a_seen) = counter();
    let (b_runs, b_seen) = counter();
    let (c_runs, c_seen) = counter();
    let a = g.memo(move |r| {
        a_runs.set(a_runs.get() + 1);
        r.get(s) + 1
    });
    let b = g.memo(move |r| {
        b_runs.set(b_runs.get() + 1);
        r.get(s) * 10
    });
    let c = g.memo(move |r| {
        c_runs.set(c_runs.get() + 1);
        r.get(a) + r.get(b)
    });
    g.observe(c, 7);
    assert_eq!(g.get(c), 12);
    let mut keys = Vec::new();
    g.take_notified(&mut keys);
    keys.clear();
    g.set(s, 2);
    g.set(s, 3);
    g.take_notified(&mut keys);
    assert_eq!(keys, [7], "one report however many writes");
    assert_eq!(g.get(c), 34);
    assert_eq!((a_seen.get(), b_seen.get(), c_seen.get()), (2, 2, 2));
}

#[test]
fn a_memo_whose_value_did_not_change_stops_propagation() {
    let mut g = Graph::new();
    let s = g.signal(2);
    let parity = g.memo(move |r| r.get(s) % 2);
    let (runs, seen) = counter();
    let label = g.memo(move |r| {
        runs.set(runs.get() + 1);
        if r.get(parity) == 0 { "even" } else { "odd" }
    });
    assert_eq!(g.get(label), "even");
    let parity_version = g.version(parity);
    g.set(s, 4);
    assert_eq!(g.get(label), "even");
    // Parity ran and found the same value; the label did not run.
    assert_eq!(seen.get(), 1);
    assert_eq!(g.version(parity), parity_version);
    g.set(s, 5);
    assert_eq!(g.get(label), "odd");
    assert_eq!(seen.get(), 2);
}

#[test]
fn a_branch_switched_to_reads_a_memo_written_in_the_same_batch_fresh() {
    let mut g = Graph::new();
    let flag = g.signal(false);
    let x = g.signal(1);
    let doubled = g.memo(move |r| r.get(x) * 2);
    let shown = g.memo(move |r| if r.get(flag) { r.get(doubled) } else { 0 });
    assert_eq!(g.get(shown), 0);
    // `doubled` has never run and `shown` does not read it; one batch writes
    // both what `doubled` reads and the switch to it.
    g.set(x, 5);
    g.set(flag, true);
    assert_eq!(g.get(shown), 10);
    // And the other way: `doubled` is current and read, then changed and
    // switched away from in one batch.
    g.set(x, 6);
    g.set(flag, false);
    assert_eq!(g.get(shown), 0);
    g.set(flag, true);
    assert_eq!(g.get(shown), 12);
}

#[test]
fn a_source_no_longer_read_no_longer_marks() {
    let mut g = Graph::new();
    let flag = g.signal(true);
    let a = g.signal(1);
    let (runs, seen) = counter();
    let m = g.memo(move |r| {
        runs.set(runs.get() + 1);
        if r.get(flag) { r.get(a) } else { -1 }
    });
    g.observe(m, 3);
    assert_eq!(g.get(m), 1);
    assert_eq!(g.stats().edges, 2);
    g.set(flag, false);
    assert_eq!(g.get(m), -1);
    assert_eq!(g.stats().edges, 1);
    let mut keys = Vec::new();
    g.take_notified(&mut keys);
    keys.clear();
    g.set(a, 50);
    g.take_notified(&mut keys);
    assert!(keys.is_empty());
    assert_eq!(g.get(m), -1);
    assert_eq!(seen.get(), 2);
}

#[test]
fn a_source_read_twice_is_one_edge() {
    let mut g = Graph::new();
    let s = g.signal(3);
    let m = g.memo(move |r| r.get(s) + r.get(s));
    assert_eq!(g.get(m), 6);
    assert_eq!(g.stats().edges, 1);
    // Past the point where reads are deduplicated by a set as well.
    let many: Vec<Signal<u32>> = (0..100).map(|i| g.signal(i)).collect();
    let sum = g.memo(move |r| {
        let once: u32 = many.iter().map(|&s| r.get(s)).sum();
        let twice: u32 = many.iter().map(|&s| r.get(s)).sum();
        once + twice
    });
    assert_eq!(g.get(sum), 2 * 4950);
    assert_eq!(g.stats().edges, 101);
}

#[test]
#[should_panic(expected = "read itself")]
fn a_cycle_panics() {
    let mut g = Graph::new();
    let cell: Signal<Option<Memo<i32>>> = g.signal(None);
    let m = g.memo(move |r| match r.get(cell) {
        Some(other) => r.get(other) + 1,
        None => 0,
    });
    assert_eq!(g.get(m), 0);
    g.set(cell, Some(m));
    g.get(m);
}

#[test]
fn disposing_of_an_owner_leaves_no_edges_closures_or_queued_work() {
    let mut g = Graph::new();
    let theme = g.signal(1u32);
    let before = g.stats();
    let row = g.owner(g.root());
    let cell = g.owner(row);
    let mut memos = Vec::new();
    for i in 0..100u64 {
        let own = g.signal_in(cell, i);
        let m = g.memo_in(cell, move |r| r.get(theme) as u64 + r.get(own));
        g.observe(m, i);
        memos.push(m);
    }
    let total = g.memo_in(row, move |r| memos.iter().map(|&m| r.get(m)).sum::<u64>());
    g.observe(total, 1000);
    assert_eq!(g.get(total), 100 + 4950);
    let mut keys = Vec::new();
    g.take_notified(&mut keys);
    g.set(theme, 2);
    assert_eq!(g.stats().notified, 101);

    g.dispose(row);
    let after = g.stats();
    assert_eq!(
        after,
        GraphStats {
            runs: after.runs,
            ..before
        }
    );
    // The slots are reused, under new generations.
    let again = g.signal(9);
    assert_eq!(g.get(again), 9);
}

#[test]
#[should_panic(expected = "disposed of")]
fn a_handle_to_something_disposed_of_panics() {
    let mut g = Graph::new();
    let owner = g.owner(g.root());
    let s = g.signal_in(owner, 1);
    g.dispose(owner);
    // The slot is taken again; the old handle still must not read it.
    let _reuse = g.signal(2);
    g.get(s);
}

#[test]
#[should_panic(expected = "still reads")]
fn disposing_of_what_a_live_memo_reads_panics() {
    let mut g = Graph::new();
    let owner = g.owner(g.root());
    let s = g.signal_in(owner, 1);
    let outside = g.memo(move |r| r.get(s));
    g.get(outside);
    g.dispose(owner);
}

#[test]
fn many_readers_of_one_signal_disposed_of_one_by_one_leave_the_rest_reading_it() {
    // 10,000 rows, each its own owner with a memo reading one shared signal.
    // Removing a row swaps the last reader into its place in the shared
    // list and fixes that reader's pointer back (no search, so the cost per
    // row does not grow with the list); a wrong pointer would later remove
    // the wrong reader, and the survivors would miss the writes below.
    let mut g = Graph::new();
    let theme = g.signal(1u32);
    let rows: Vec<(Owner, Memo<u32>)> = (0..10_000u32)
        .map(|i| {
            let owner = g.owner(g.root());
            let m = g.memo_in(owner, move |r| r.get(theme) + i);
            (owner, m)
        })
        .collect();
    for &(_, m) in &rows {
        g.get(m);
    }
    assert_eq!(g.stats().edges, 10_000);
    // Remove from the middle out, so moved entries are exercised.
    for (k, &(owner, _)) in rows.iter().enumerate().step_by(2) {
        g.dispose(owner);
        assert_eq!(g.stats().edges, 10_000 - (k / 2 + 1));
    }
    g.set(theme, 5);
    for &(_, m) in rows.iter().skip(1).step_by(2).take(50) {
        assert!(g.get(m) >= 5);
    }
    let runs_before = g.stats().runs;
    g.set(theme, 6);
    let survivors: Vec<Memo<u32>> = rows.iter().skip(1).step_by(2).map(|&(_, m)| m).collect();
    for &m in &survivors {
        g.get(m);
    }
    assert_eq!(g.stats().runs - runs_before, 5_000);
}

#[test]
fn a_memo_created_and_disposed_of_repeatedly_does_not_grow_the_graph() {
    let mut g = Graph::new();
    let s = g.signal(0);
    for round in 0..1_000 {
        let owner = g.owner(g.root());
        let m = g.memo_in(owner, move |r| r.get(s) + round);
        g.get(m);
        g.dispose(owner);
    }
    assert_eq!(g.slots.len(), 2, "the signal and one reused slot");
    assert_eq!(g.owners.len(), 2, "the root and one reused owner");
}
