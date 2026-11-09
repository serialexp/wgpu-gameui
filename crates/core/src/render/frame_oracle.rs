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
use crate::render::draw_box::{PixelBox, Target};
use crate::render::draw_order::BatchKey;
use crate::render::draw_order::tests_support::Rng;
use crate::render::draw_plan::tests_support::Lists;
use crate::render::draw_plan::{Batch, DrawPlan, PlannedUnit};
use crate::render::frame_diff::{ContextKey, FrameDiff, FrameSnapshot};
use crate::text::{MsdfInstance, MsdfPrepared, PrepareWork, TextBlock};
use crate::widgets::{AnalyticInstance, CircleInstance, DrawList};

/// Everything a frame's CPU side produces, as plain data to compare.
#[derive(Clone, Debug, PartialEq)]
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
    /// Every unit's key and boxes, in paint order.
    boxes: Vec<(BatchKey, Vec<PixelBox>)>,
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
            boxes: plan.checked.clone().unwrap_or_default(),
        }
    }
}

/// The work one frame took on the kept state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Work {
    pub(crate) text: PrepareWork,
    pub(crate) boxes_computed: u64,
    pub(crate) units: usize,
    /// How the list differed from the frame before, as `FrameSnapshot` saw it.
    pub(crate) diff: FrameDiff,
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
    /// The frame before, and what it was compared against.
    snapshot: FrameSnapshot,
    last: Option<Fingerprint>,
}

impl Oracle {
    pub(crate) fn new() -> Self {
        let mut kept_plan = DrawPlan::new();
        // The order of every plan is checked against the brute-force oracle too.
        kept_plan.set_check(true);
        let mut fresh_plan = DrawPlan::new();
        fresh_plan.set_check(true);
        Self {
            kept: Lists::new(),
            kept_plan,
            kept_draws: BatchDraws::default(),
            fresh_plan,
            fresh_draws: BatchDraws::default(),
            fresh_prepared: MsdfPrepared::default(),
            snapshot: FrameSnapshot::default(),
            last: None,
        }
    }

    pub(crate) fn list(&self) -> DrawList {
        self.kept.list()
    }

    /// Plan `list` on the kept state and on state that has seen nothing, and
    /// panic unless they agree. Returns the work the kept state did.
    pub(crate) fn step(&mut self, list: &DrawList, target: &Target) -> Work {
        let context = ContextKey {
            grid: Lists::grid(target),
            viewport: target.size,
            fonts: 0,
            resolution: 0,
        };
        let diff = self.snapshot.compare(list, &context);
        self.snapshot.keep(list, &context);
        // What is kept is the list, however little of it was copied.
        let again = self.snapshot.compare(list, &context);
        assert!(
            again.identical(),
            "the kept frame is not the list: {again:?}"
        );
        self.kept.plan(list, target, &mut self.kept_plan);
        self.kept_plan.check();
        self.kept_draws
            .fill(&self.kept_plan, list, &self.kept.prepared);
        let kept = Fingerprint::of(&self.kept_plan, &self.kept.prepared, &self.kept_draws);
        let work = Work {
            text: self.kept.prepared.work,
            boxes_computed: self.kept_plan.boxes_computed(),
            units: self.kept_plan.units().len(),
            diff,
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
        if let Some(previous) = &self.last {
            check_diff(&diff, previous, &kept);
        }
        self.last = Some(kept);
        work
    }
}

/// What a comparison may promise, checked against the plans it was about:
///
/// - when it says every record is where it was, the plan is the plan: keys and
///   boxes, orders, batches, unit and instance layout, and the draws' ranges;
/// - when it says the frame is the last one again, everything is, byte for byte.
///
/// It may be wrong the other way (call a frame changed that was not); these are
/// the two ways it must not be wrong.
fn check_diff(diff: &FrameDiff, previous: &Fingerprint, now: &Fingerprint) {
    if diff.keeps_places() {
        assert_eq!(previous.boxes, now.boxes, "{diff:?}: keys or boxes moved");
        assert_eq!(previous.units, now.units, "{diff:?}: orders or units moved");
        assert_eq!(previous.batches, now.batches, "{diff:?}: batches moved");
        assert_eq!(previous.text_units, now.text_units, "{diff:?}: text units");
        assert_eq!(previous.text_commands, now.text_commands, "{diff:?}");
        assert_eq!(previous.block_sweeps, now.block_sweeps, "{diff:?}: sweeps");
        assert_eq!(previous.draws, now.draws, "{diff:?}: draw ranges");
        assert_eq!(previous.text_instances.len(), now.text_instances.len());
        assert_eq!(previous.analytic.len(), now.analytic.len());
        assert_eq!(previous.circles.len(), now.circles.len());
        assert_eq!(previous.indices, now.indices, "{diff:?}: soup indices");
    }
    if diff.identical() {
        assert_eq!(previous, now, "{diff:?}: a frame called identical differs");
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

/// A list described as data, so a frame can be changed a little and drawn again.
#[derive(Clone, Debug)]
struct Cell {
    kind: Kind,
    rect: Rect,
    colour: [f32; 4],
    text: String,
    size: f32,
    shadow: Option<[u8; 4]>,
    glow: Option<[u8; 4]>,
    clip: Option<Rect>,
    sprite: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Chrome,
    Circle,
    Text,
    Sprite,
    NineSlice,
    Soup,
}

const KINDS: [Kind; 6] = [
    Kind::Chrome,
    Kind::Circle,
    Kind::Text,
    Kind::Sprite,
    Kind::NineSlice,
    Kind::Soup,
];

impl Cell {
    fn draw(&self, list: &mut DrawList) {
        let r = self.rect;
        if let Some(clip) = self.clip {
            list.push_clip(clip);
        }
        match self.kind {
            Kind::Chrome => list.chrome_rect(r, 3.0, 1.0, self.colour, [0.5; 4]),
            Kind::Circle => list.circle((r.x, r.y), r.width * 0.3, self.colour),
            Kind::Text => {
                let mut block = TextBlock::new(self.text.as_str(), r.x, r.y)
                    .with_size(self.size)
                    .with_color_f32(self.colour);
                if let Some([r, g, b, a]) = self.shadow {
                    block = block.with_shadow(r, g, b, a, 1.0, 1.0, 1.0);
                }
                if let Some([r, g, b, a]) = self.glow {
                    block = block.with_glow(r, g, b, a, 3.0);
                }
                list.text(block);
            }
            Kind::Sprite => {
                list.icon(self.sprite, r.x, r.y, r.width, r.height);
                // `icon` takes no tint, so its record is tinted after the fact.
                if let Some(last) = list.icons.last_mut() {
                    last.tint = self.colour;
                }
            }
            Kind::NineSlice => {
                list.nine_slice(r.x, r.y, r.width, r.height, self.sprite);
                if let Some(last) = list.nine_slices.last_mut() {
                    last.tint = self.colour;
                }
            }
            Kind::Soup => list.rounded_rect(r, 4.0, self.colour),
        }
        if self.clip.is_some() {
            list.pop_clip();
        }
    }
}

fn random_cell(rng: &mut Rng, kind: Kind) -> Cell {
    let unit = |rng: &mut Rng, n: u32| rng.below(n) as f32;
    let colour = [
        unit(rng, 100) / 100.0,
        unit(rng, 100) / 100.0,
        unit(rng, 100) / 100.0,
        0.4 + unit(rng, 60) / 100.0,
    ];
    Cell {
        kind,
        rect: Rect::new(
            unit(rng, 560),
            unit(rng, 420),
            20.0 + unit(rng, 80),
            14.0 + unit(rng, 40),
        ),
        colour,
        text: format!("cell {}", rng.below(1000)),
        size: 10.0 + unit(rng, 8),
        shadow: (rng.below(4) == 0).then_some([0, 0, 0, 180]),
        glow: (rng.below(8) == 0).then_some([0, 120, 255, 200]),
        clip: (rng.below(4) == 0).then(|| {
            Rect::new(
                unit(rng, 400),
                unit(rng, 300),
                60.0 + unit(rng, 200),
                40.0 + unit(rng, 120),
            )
        }),
        sprite: ["a", "b", "video", "missing"][rng.below(4) as usize],
    }
}

fn draw_cells(oracle: &Oracle, cells: &[Cell]) -> DrawList {
    let mut list = oracle.list();
    for cell in cells {
        cell.draw(&mut list);
    }
    list
}

/// Change a cell the way a frame to frame change does, in `how`: a look
/// (colour), or anything.
fn change_look(rng: &mut Rng, cell: &mut Cell) {
    cell.colour = [
        rng.below(100) as f32 / 100.0,
        rng.below(100) as f32 / 100.0,
        rng.below(100) as f32 / 100.0,
        0.4 + rng.below(60) as f32 / 100.0,
    ];
    if let Some(shadow) = &mut cell.shadow {
        shadow[3] = 100 + rng.below(150) as u8;
    }
}

fn change_anything(rng: &mut Rng, cell: &mut Cell) {
    match rng.below(9) {
        0 => cell.rect.x += 1.0 + rng.below(20) as f32,
        1 => cell.rect.y -= 1.0 + rng.below(20) as f32,
        2 => cell.rect.width += 1.0 + rng.below(10) as f32,
        3 => cell.text = format!("changed {}", rng.below(1000)),
        4 => cell.size += 1.0,
        5 => cell.shadow = cell.shadow.is_none().then_some([0, 0, 0, 200]),
        6 => cell.glow = cell.glow.is_none().then_some([255, 0, 0, 200]),
        7 => {
            cell.clip = cell
                .clip
                .is_none()
                .then(|| Rect::new(30.0, 30.0, 200.0, 100.0))
        }
        _ => cell.sprite = ["a", "b", "video", "missing"][rng.below(4) as usize],
    }
}

/// Frames mutated at random: nothing, only looks, or anything, with cells
/// added and removed now and then. `step` checks each against the plan.
#[test]
fn what_the_comparison_promises_holds_over_random_changes() {
    let t = target();
    let (mut identical, mut keeps, mut moved, mut reshaped, mut frames) = (0, 0, 0, 0, 0);
    for seed in 0..6u64 {
        let mut rng = Rng(100 + seed);
        let mut oracle = Oracle::new();
        let mut cells: Vec<Cell> = (0..40)
            .map(|i| random_cell(&mut rng, KINDS[i % KINDS.len()]))
            .collect();
        for _ in 0..60 {
            match rng.below(5) {
                0 => {}
                1 | 2 => {
                    for _ in 0..1 + rng.below(6) {
                        let i = rng.below(cells.len() as u32) as usize;
                        change_look(&mut rng, &mut cells[i]);
                    }
                }
                3 => {
                    let i = rng.below(cells.len() as u32) as usize;
                    change_anything(&mut rng, &mut cells[i]);
                }
                _ => {
                    if rng.below(2) == 0 && cells.len() > 5 {
                        cells.remove(rng.below(cells.len() as u32) as usize);
                    } else {
                        let kind = KINDS[rng.below(KINDS.len() as u32) as usize];
                        cells.insert(
                            rng.below(cells.len() as u32) as usize,
                            random_cell(&mut rng, kind),
                        );
                    }
                }
            }
            let list = draw_cells(&oracle, &cells);
            let diff = oracle.step(&list, &t).diff;
            frames += 1;
            identical += usize::from(diff.identical());
            keeps += usize::from(diff.keeps_places() && !diff.identical());
            moved += usize::from(diff.compared && diff.topology_same && diff.place > 0);
            reshaped += usize::from(diff.compared && !diff.topology_same);
        }
    }
    // The promises were tested on every kind of frame, not only the easy ones.
    assert!(identical > 20, "{identical} identical of {frames}");
    assert!(keeps > 20, "{keeps} kept their places of {frames}");
    assert!(moved > 20, "{moved} moved of {frames}");
    assert!(reshaped > 20, "{reshaped} changed shape of {frames}");
}

/// The other half: a colour is a look in every family, so the plan is kept.
#[test]
fn a_colour_change_in_any_family_is_a_look_and_a_move_is_a_place() {
    let t = target();
    for kind in KINDS {
        let mut rng = Rng(5);
        let mut oracle = Oracle::new();
        let mut cell = random_cell(&mut rng, kind);
        cell.clip = None;
        cell.shadow = None;
        cell.glow = None;
        // Opaque, so that nothing about the colour could make it not draw.
        cell.colour = [0.2, 0.4, 0.6, 1.0];
        oracle.step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t);

        let again = oracle.step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t);
        assert!(again.diff.identical(), "{kind:?}: {:?}", again.diff);

        cell.colour = [0.9, 0.1, 0.3, 0.8];
        let looked = oracle.step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t);
        let d = looked.diff;
        assert!(
            d.keeps_places() && !d.identical() && d.look == 1,
            "{kind:?}: {d:?}"
        );

        cell.rect.x += 7.0;
        let moved = oracle.step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t);
        let d = moved.diff;
        assert!(
            d.compared && d.topology_same && !d.keeps_places() && d.place == 1,
            "{kind:?}: {d:?}"
        );
    }
}

#[test]
fn a_text_effect_colour_is_a_look_and_its_size_is_a_place() {
    let t = target();
    let mut oracle = Oracle::new();
    let mut rng = Rng(9);
    let mut cell = random_cell(&mut rng, Kind::Text);
    cell.clip = None;
    cell.shadow = Some([0, 0, 0, 200]);
    cell.glow = Some([0, 100, 255, 200]);
    oracle.step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t);
    cell.shadow = Some([200, 0, 0, 90]);
    cell.glow = Some([255, 255, 0, 255]);
    let d = oracle
        .step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t)
        .diff;
    assert!(d.keeps_places() && d.look == 1, "{d:?}");
    cell.glow = None;
    let d = oracle
        .step(&draw_cells(&oracle, std::slice::from_ref(&cell)), &t)
        .diff;
    assert!(d.compared && d.topology_same && d.place == 1, "{d:?}");
}

#[test]
fn a_changed_target_or_a_changed_shape_is_not_compared_one_for_one() {
    let mut oracle = Oracle::new();
    let mut rng = Rng(3);
    let cells: Vec<Cell> = (0..10)
        .map(|i| random_cell(&mut rng, KINDS[i % 6]))
        .collect();
    let list = draw_cells(&oracle, &cells);
    let first = oracle.step(&list, &target()).diff;
    assert!(!first.compared, "nothing to compare the first frame with");
    // Another scale: the records are the same, the context is not.
    let rescaled = oracle
        .step(&list, &Target::new((640, 480), 1.5, [0.0, 0.0]))
        .diff;
    assert!(rescaled.compared && !rescaled.context_same && !rescaled.keeps_places());
    assert_eq!(rescaled.place, 0, "{rescaled:?}");
    // One cell fewer: the paint stream has another shape.
    let list = draw_cells(&oracle, &cells[1..]);
    let fewer = oracle
        .step(&list, &Target::new((640, 480), 1.5, [0.0, 0.0]))
        .diff;
    assert!(fewer.compared && !fewer.topology_same);
    assert_eq!((fewer.same, fewer.look, fewer.place), (0, 0, 0));
}

#[test]
fn a_frame_that_moved_everywhere_is_given_up_part_way() {
    let t = target();
    let mut oracle = Oracle::new();
    let mut rng = Rng(21);
    let mut cells: Vec<Cell> = (0..400)
        .map(|_| random_cell(&mut rng, Kind::Chrome))
        .collect();
    oracle.step(&draw_cells(&oracle, &cells), &t);
    for cell in &mut cells {
        cell.rect.x += 1.0;
    }
    let d = oracle.step(&draw_cells(&oracle, &cells), &t).diff;
    assert!(d.abandoned && !d.keeps_places(), "{d:?}");
    assert!(d.place < 400, "gave up after {} of 400", d.place);
    // A still header and a moving body is given up inside the body.
    for cell in &mut cells {
        cell.rect.x -= 1.0;
    }
    oracle.step(&draw_cells(&oracle, &cells), &t);
    for cell in &mut cells[100..] {
        cell.rect.y += 1.0;
    }
    let d = oracle.step(&draw_cells(&oracle, &cells), &t).diff;
    assert!(d.abandoned && d.same >= 100, "{d:?}");
}

/// A list that keeps moving everywhere is compared on fewer and fewer frames,
/// and one that settles is compared again within a few.
#[test]
fn a_list_that_keeps_moving_rests_between_comparisons_and_one_that_settles_resumes() {
    let t = target();
    let context = ContextKey {
        grid: Lists::grid(&t),
        viewport: t.size,
        fonts: 0,
        resolution: 0,
    };
    let oracle = Oracle::new();
    let mut rng = Rng(8);
    let mut cells: Vec<Cell> = (0..200)
        .map(|_| random_cell(&mut rng, Kind::Chrome))
        .collect();
    let mut snapshot = FrameSnapshot::default();
    // The renderer's protocol: rest, or compare and then keep unless given up.
    let frame = |snapshot: &mut FrameSnapshot, cells: &[Cell]| {
        let list = draw_cells(&oracle, cells);
        if snapshot.resting() {
            return None;
        }
        let diff = snapshot.compare(&list, &context);
        if diff.abandoned {
            snapshot.forget();
        } else {
            snapshot.keep(&list, &context);
        }
        Some(diff)
    };
    let mut compared = 0;
    for _ in 0..60 {
        for cell in &mut cells {
            cell.rect.x += 1.0;
        }
        compared += frame(&mut snapshot, &cells).is_some_and(|d| d.compared) as usize;
    }
    assert!(
        (3..=9).contains(&compared),
        "{compared} of 60 frames compared"
    );
    // Now nothing moves: reuse is back within the longest rest and a frame to
    // keep one.
    let mut first_identical = None;
    for n in 0..40 {
        if frame(&mut snapshot, &cells).is_some_and(|d| d.identical()) {
            first_identical = Some(n);
            break;
        }
    }
    let n = first_identical.expect("never compared the settled list");
    assert!(n <= 20, "reuse resumed after {n} frames");
}
