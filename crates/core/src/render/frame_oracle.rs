//! The frame oracle (`docs/design/frame-reuse.md`, Phase 0): plan a *sequence*
//! of lists on state that lives from frame to frame, and again on state that
//! has seen nothing, and require the same plan, instances and draws.
//!
//! Nothing keeps state across frames yet, so today the two always agree. The
//! oracle is here first so each reuse phase is checked, the day it lands, against
//! every sequence below rather than against a handful of hand-picked frames.
//!
//! What "state that has seen nothing" shares with the kept state is the glyph
//! atlas: an instance names atlas texels, and where a glyph sits depends on the
//! order glyphs were first drawn, not on the frame being planned. The prepared
//! text, the plan and the draws are what reuse will keep, so those are the ones
//! rebuilt from nothing.

use crate::layout::Rect;
use crate::render::batch_draws::{BatchDraw, BatchDraws};
use crate::render::draw_box::Target;
use crate::render::draw_plan::tests_support::Lists;
use crate::render::draw_plan::{Batch, DrawPlan, PlannedUnit};
use crate::text::{MsdfInstance, MsdfPrepared, PrepareWork, TextBlock};
use crate::widgets::{AnalyticInstance, CircleInstance, DrawList};

/// Everything a frame's CPU side produces, as plain data to compare.
#[derive(Debug, PartialEq)]
struct Fingerprint {
    units: Vec<PlannedUnit>,
    batches: Vec<Batch>,
    text_instances: Vec<MsdfInstance>,
    text_units: Vec<(std::ops::Range<u32>, std::ops::Range<u32>)>,
    text_commands: Vec<std::ops::Range<u32>>,
    block_sweeps: Vec<[std::ops::Range<u32>; 3]>,
    indices: Vec<u32>,
    analytic: Vec<AnalyticInstance>,
    circles: Vec<CircleInstance>,
    text: Vec<MsdfInstance>,
    draws: Vec<BatchDraw>,
}

impl Fingerprint {
    fn of(plan: &DrawPlan, prepared: &MsdfPrepared, draws: &BatchDraws) -> Self {
        Self {
            units: plan.units().to_vec(),
            batches: plan.batches().to_vec(),
            text_instances: prepared.text_instances.clone(),
            text_units: prepared
                .text_units
                .iter()
                .map(|u| (u.blocks.clone(), u.instances.clone()))
                .collect(),
            text_commands: prepared.text_commands.clone(),
            block_sweeps: prepared.block_sweeps.clone(),
            indices: draws.indices.clone(),
            analytic: draws.analytic.clone(),
            circles: draws.circles.clone(),
            text: draws.text.clone(),
            draws: draws.draws.clone(),
        }
    }
}

/// The work one frame took on the kept state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Work {
    pub(crate) text: PrepareWork,
    pub(crate) boxes_computed: u64,
    pub(crate) units: usize,
}

/// State kept from frame to frame, checked against a recompute from nothing.
pub(crate) struct Oracle {
    kept: Lists,
    kept_plan: DrawPlan,
    kept_draws: BatchDraws,
    /// Prepared text, plan and draws that never see a second frame.
    fresh_plan: DrawPlan,
    fresh_draws: BatchDraws,
    fresh_prepared: MsdfPrepared,
}

impl Oracle {
    pub(crate) fn new() -> Self {
        let mut kept_plan = DrawPlan::new();
        // The order of every plan is checked against the brute-force oracle too.
        kept_plan.set_check(true);
        Self {
            kept: Lists::new(),
            kept_plan,
            kept_draws: BatchDraws::default(),
            fresh_plan: DrawPlan::new(),
            fresh_draws: BatchDraws::default(),
            fresh_prepared: MsdfPrepared::default(),
        }
    }

    pub(crate) fn list(&self) -> DrawList {
        self.kept.list()
    }

    /// Plan `list` on the kept state and on state that has seen nothing, and
    /// panic unless they agree. Returns the work the kept state did.
    pub(crate) fn step(&mut self, list: &DrawList, target: &Target) -> Work {
        self.kept.plan(list, target, &mut self.kept_plan);
        self.kept_plan.check();
        self.kept_draws
            .fill(&self.kept_plan, list, &self.kept.prepared);
        let kept = Fingerprint::of(&self.kept_plan, &self.kept.prepared, &self.kept_draws);
        let work = Work {
            text: self.kept.prepared.work,
            boxes_computed: self.kept_plan.boxes_computed(),
            units: self.kept_plan.units().len(),
        };

        // The same cpu, so the same atlas; a prepared list, plan and draws of
        // their own.
        std::mem::swap(&mut self.kept.prepared, &mut self.fresh_prepared);
        self.kept.plan(list, target, &mut self.fresh_plan);
        self.fresh_draws
            .fill(&self.fresh_plan, list, &self.kept.prepared);
        let fresh = Fingerprint::of(&self.fresh_plan, &self.kept.prepared, &self.fresh_draws);
        std::mem::swap(&mut self.kept.prepared, &mut self.fresh_prepared);

        assert_eq!(kept, fresh, "the kept state drew a different frame");
        work
    }
}

fn target() -> Target {
    Target::new((640, 480), 1.0, [0.0, 0.0])
}

/// A grid of cells, each a rounded panel with a label.
fn grid(list: &mut DrawList, cells: usize, shade: impl Fn(usize) -> f32) {
    for i in 0..cells {
        let (x, y) = (8.0 + 70.0 * (i % 8) as f32, 8.0 + 30.0 * (i / 8) as f32);
        let g = shade(i);
        list.chrome_rect(
            Rect::new(x, y, 64.0, 24.0),
            3.0,
            1.0,
            [g, g, 0.3, 1.0],
            [0.5; 4],
        );
        list.text(TextBlock::new(format!("cell {i}"), x + 4.0, y + 4.0).with_size(12.0));
    }
}

#[test]
fn the_same_frame_again_and_a_colour_only_change_agree_with_a_recompute() {
    let mut oracle = Oracle::new();
    let t = target();
    for frame in 0..4 {
        let mut list = oracle.list();
        grid(&mut list, 24, |i| ((i + frame) % 5) as f32 / 5.0);
        oracle.step(&list, &t);
    }
    let mut list = oracle.list();
    grid(&mut list, 24, |_| 0.5);
    let first = oracle.step(&list, &t);
    let mut list = oracle.list();
    grid(&mut list, 24, |_| 0.5);
    let again = oracle.step(&list, &t);
    // The counters see the work: the same list is the same work today, and a
    // reuse phase that skips it must show up here as fewer.
    assert_eq!(first.text, again.text);
    assert_eq!(first.text.blocks_looked_up, 24);
    assert_eq!(first.boxes_computed, again.boxes_computed);
    assert_eq!(first.units, 48);
}

#[test]
fn units_inserted_removed_and_reordered() {
    let mut oracle = Oracle::new();
    let t = target();
    for cells in [10, 11, 5, 0, 12, 12, 3] {
        let mut list = oracle.list();
        grid(&mut list, cells, |_| 0.4);
        oracle.step(&list, &t);
    }
    // The same cells in another paint order.
    for rev in [false, true, false] {
        let mut list = oracle.list();
        let order: Vec<usize> = if rev {
            (0..16).rev().collect()
        } else {
            (0..16).collect()
        };
        for i in order {
            let (x, y) = (8.0 + 20.0 * i as f32, 40.0);
            list.chrome_rect(Rect::new(x, y, 64.0, 24.0), 3.0, 1.0, [0.2; 4], [0.5; 4]);
            list.text(TextBlock::new(format!("n{i}"), x + 2.0, y + 2.0).with_size(12.0));
        }
        oracle.step(&list, &t);
    }
}

#[test]
fn a_list_that_goes_from_empty_to_visible_and_back() {
    let mut oracle = Oracle::new();
    let t = target();
    for visible in [false, true, true, false, true] {
        let mut list = oracle.list();
        if visible {
            grid(&mut list, 16, |_| 0.6);
        }
        oracle.step(&list, &t);
    }
}

#[test]
fn text_that_gains_and_loses_a_shadow_regroups_its_command() {
    let mut oracle = Oracle::new();
    let t = target();
    for shadowed in [None, Some(1), Some(2), None, Some(0), Some(3)] {
        let mut list = oracle.list();
        for i in 0..4 {
            let mut block =
                TextBlock::new(format!("line {i}"), 12.0, 12.0 + 22.0 * i as f32).with_size(14.0);
            if shadowed == Some(i) {
                block = block.with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0);
            }
            list.text(block);
        }
        oracle.step(&list, &t);
    }
}

#[test]
fn scrolled_text_crosses_the_edge_of_the_target() {
    let mut oracle = Oracle::new();
    let t = target();
    let lines: Vec<String> = (0..80)
        .map(|i| format!("a scrolling line, number {i}"))
        .collect();
    // Whole and fractional steps, down past the bottom edge and up past the top.
    for step in (0..60).chain((0..60).rev()) {
        let mut list = oracle.list();
        let y0 = -40.0 - step as f32 * 3.37;
        for (i, line) in lines.iter().enumerate() {
            list.text(TextBlock::new(line.as_str(), 10.0, y0 + 16.0 * i as f32).with_size(13.0));
        }
        oracle.step(&list, &t);
    }
}

#[test]
fn the_target_scale_origin_and_size_change_between_frames() {
    let mut oracle = Oracle::new();
    for (size, scale, origin) in [
        ((640, 480), 1.0, [0.0, 0.0]),
        ((640, 480), 1.25, [0.0, 0.0]),
        ((640, 480), 1.25, [3.5, -2.25]),
        ((800, 300), 1.25, [3.5, -2.25]),
        ((800, 300), 2.0, [0.0, 0.0]),
        ((640, 480), 1.0, [0.0, 0.0]),
    ] {
        let mut list = oracle.list();
        grid(&mut list, 30, |i| (i % 3) as f32 / 3.0);
        oracle.step(&list, &Target::new(size, scale, origin));
    }
}

#[test]
fn overlapping_units_that_change_their_keys_and_their_places() {
    let mut oracle = Oracle::new();
    let t = target();
    // A panel, text over it, a circle over the text; each frame one of them
    // moves or swaps family.
    for frame in 0..24 {
        let mut list = oracle.list();
        let shift = (frame % 6) as f32 * 9.0;
        list.chrome_rect(
            Rect::new(20.0, 20.0, 200.0, 80.0),
            4.0,
            1.0,
            [0.2; 4],
            [0.6; 4],
        );
        if frame % 2 == 0 {
            list.text(TextBlock::new("over the panel", 30.0 + shift, 40.0).with_size(16.0));
        }
        list.circle((120.0 + shift, 60.0), 14.0, [0.9, 0.2, 0.2, 1.0]);
        if frame % 3 == 0 {
            list.text(TextBlock::new("over the circle", 90.0, 50.0 + shift).with_size(12.0));
        }
        list.chrome_rect(
            Rect::new(40.0 + shift, 30.0, 60.0, 40.0),
            4.0,
            1.0,
            [0.5; 4],
            [0.1; 4],
        );
        oracle.step(&list, &t);
    }
}

#[test]
fn the_oracle_tells_two_different_frames_apart() {
    let mut oracle = Oracle::new();
    let t = target();
    let mut list = oracle.list();
    grid(&mut list, 8, |_| 0.4);
    oracle.step(&list, &t);
    let one = Fingerprint::of(&oracle.kept_plan, &oracle.kept.prepared, &oracle.kept_draws);
    let mut list = oracle.list();
    grid(&mut list, 8, |_| 0.9);
    oracle.step(&list, &t);
    let two = Fingerprint::of(&oracle.kept_plan, &oracle.kept.prepared, &oracle.kept_draws);
    assert_ne!(one, two, "a colour change must show in the fingerprint");
}
