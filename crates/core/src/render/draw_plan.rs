//! The batches a draw list is drawn in (`docs/design/draw-call-batching.md`,
//! "New: the plan"). Every unit gets its boxes and an order from
//! [`DrawOrder`]; the units are sorted by (order, batch key, paint index), and
//! each stretch of one key is a batch: one draw call. Nothing here touches the
//! GPU, so a list's plan can be tested without one.
//!
//! Only a list's paint stream is planned. A list filled through the public
//! arrays with no paint stream, and soup recorded after the last command, keep
//! passes of their own in paint order (the design's non-goals).

use std::ops::Range;

use super::draw_box::{self, PixelBox, Target};
use super::draw_order::{BatchKey, DrawOrder, OrderStats};
use super::textures::{Binding, Resolved};
use crate::text::MsdfPrepared;
use crate::widgets::{DrawList, IconDraw, NineSliceDraw, PaintCmd};

/// What a unit draws: a record of one family, or a soup command or text unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitRef {
    /// The soup of paint command `cmd`.
    Soup { cmd: u32 },
    /// Into `analytic_instances`.
    Analytic(u32),
    /// Into `circle_instances`.
    Circle(u32),
    /// Into `icons` (an image, icon or YUV sprite), drawn from
    /// [`DrawPlan::textured`]`(at)`.
    Sprite { draw: u32, at: u32 },
    /// Into `nine_slices`, drawn from [`DrawPlan::textured`]`(at)`.
    NineSlice { draw: u32, at: u32 },
    /// Into [`MsdfPrepared::text_units`].
    Text(u32),
    /// Into `icons_msdf`.
    #[cfg(feature = "phosphor-icons")]
    IconMsdf(u32),
}

/// The families as batch keys tell them apart. Text and MSDF icons share a
/// pipeline but sample different atlases; YUV sprites have a pipeline of
/// their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Soup = 0,
    Analytic = 1,
    Circle = 2,
    Sprite = 3,
    Yuv = 4,
    NineSlice = 5,
    Text = 6,
    #[cfg(feature = "phosphor-icons")]
    IconMsdf = 7,
}

impl Family {
    fn key(self, texture: u64) -> BatchKey {
        debug_assert!(texture < 1 << 56, "texture code {texture} too large");
        BatchKey((self as u64) << 56 | texture)
    }
}

/// Where a sprite or nine-slice samples from, as the renderer resolved it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Textured {
    pub(crate) at: Resolved,
    /// A nine-slice's border insets in source px (left, top, right, bottom);
    /// zero for a sprite.
    pub(crate) border: [u32; 4],
}

/// The texture each sprite and nine-slice draws from, as the renderer resolves
/// it. `None` for one that doesn't draw: unresolved, or a nine-slice backed by
/// a YUV sprite.
pub(crate) trait ResolveTextures {
    fn sprite(&self, draw: &IconDraw) -> Option<Resolved>;
    /// The texture and the nine-slice's border insets.
    fn nine_slice(&self, draw: &NineSliceDraw) -> Option<(Resolved, [u32; 4])>;
}

/// A unit as planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlannedUnit {
    pub(crate) order: u32,
    pub(crate) key: BatchKey,
    /// Its place among the list's units in paint order, culled ones included.
    pub(crate) paint: u32,
    pub(crate) unit: UnitRef,
}

/// Units drawn in one call: a range of [`DrawPlan::units`], all of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Batch {
    pub(crate) key: BatchKey,
    pub(crate) units: Range<u32>,
}

/// A list's units in the order they are drawn, and its batches. Reused from
/// list to list: cleared, never freed.
#[derive(Debug, Default)]
pub(crate) struct DrawPlan {
    order: DrawOrder,
    /// One unit's boxes, while it is ordered.
    boxes: Vec<PixelBox>,
    units: Vec<PlannedUnit>,
    batches: Vec<Batch>,
    textured: Vec<Textured>,
    next_paint: u32,
    /// The paint command being walked, which the paint-order plan cuts
    /// batches at.
    cmd: u32,
    /// The paint command the last batch belongs to, in paint order.
    batch_cmd: u32,
    /// Planning in paint order: no boxes, no culling, a batch per command.
    paint_order: bool,
    /// Every unit's key and boxes in paint order, culled ones included, kept
    /// for [`check`](Self::check) while checking is on.
    #[cfg(any(test, feature = "paint-order-oracle"))]
    checked: Option<Vec<(BatchKey, Vec<PixelBox>)>>,
}

impl DrawPlan {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Plan `list`'s paint stream onto `target`. `prepared` is its text and
    /// MSDF icons as prepared for that target, the text in one command per
    /// `Text` paint command, in paint order.
    pub(crate) fn build(
        &mut self,
        list: &DrawList,
        target: &Target,
        prepared: &MsdfPrepared,
        textures: &impl ResolveTextures,
    ) {
        self.begin(target.size);
        self.walk(list, Some(target), prepared, textures);
        self.finish();
    }

    /// Plan `list`'s paint stream in paint order, as it was drawn before
    /// batching: one batch per paint command, split where the texture
    /// changes, after the draws that don't resolve are left out. It reads no
    /// boxes and culls nothing, so it is the reference the batches are
    /// compared with (`docs/design/draw-call-batching.md`, "The paint-order
    /// path is the identity plan").
    pub(crate) fn build_paint_order(
        &mut self,
        list: &DrawList,
        prepared: &MsdfPrepared,
        textures: &impl ResolveTextures,
    ) {
        self.begin((0, 0));
        self.paint_order = true;
        self.walk(list, None, prepared, textures);
    }

    /// Make a unit of every draw in `list`'s paint stream, in paint order,
    /// boxed on `target` unless planning in paint order.
    fn walk(
        &mut self,
        list: &DrawList,
        target: Option<&Target>,
        prepared: &MsdfPrepared,
        textures: &impl ResolveTextures,
    ) {
        let mut text_unit = 0;
        for (cmd, paint) in list.paint_cmds.iter().enumerate() {
            self.cmd = cmd as u32;
            match paint {
                PaintCmd::Soup { indices } => {
                    if let Some(t) = target {
                        let indices = &list.indices[indices.start as usize..indices.end as usize];
                        self.boxes.push(draw_box::soup(t, &list.vertices, indices));
                    }
                    self.add(Family::Soup.key(0), UnitRef::Soup { cmd: cmd as u32 });
                }
                PaintCmd::Analytic { instances } => {
                    for i in instances.clone() {
                        if let Some(t) = target {
                            let record = &list.analytic_instances[i as usize];
                            self.boxes.push(draw_box::analytic(t, record));
                        }
                        self.add(Family::Analytic.key(0), UnitRef::Analytic(i));
                    }
                }
                PaintCmd::Circle { instances } => {
                    for i in instances.clone() {
                        if let Some(t) = target {
                            let c = &list.circle_instances[i as usize];
                            self.boxes.push(draw_box::circle(t, c));
                        }
                        self.add(Family::Circle.key(0), UnitRef::Circle(i));
                    }
                }
                PaintCmd::Icon { draws } => {
                    for i in draws.clone() {
                        let draw = &list.icons[i as usize];
                        // One that can't be resolved doesn't draw.
                        let Some(at) = textures.sprite(draw) else {
                            self.skip();
                            continue;
                        };
                        let family = match at.binding {
                            Binding::Yuv(_) => Family::Yuv,
                            Binding::Own(_) | Binding::Atlas(_) => Family::Sprite,
                        };
                        if let Some(t) = target {
                            self.boxes.push(draw_box::icon(t, draw));
                        }
                        let unit = UnitRef::Sprite {
                            draw: i,
                            at: self.textured.len() as u32,
                        };
                        if self.add(family.key(at.binding.code()), unit) {
                            self.textured.push(Textured { at, border: [0; 4] });
                        }
                    }
                }
                PaintCmd::NineSlice { draws } => {
                    for i in draws.clone() {
                        let draw = &list.nine_slices[i as usize];
                        let Some((at, border)) = textures.nine_slice(draw) else {
                            self.skip();
                            continue;
                        };
                        if let Some(t) = target {
                            self.boxes.push(draw_box::nine_slice(t, draw));
                        }
                        let unit = UnitRef::NineSlice {
                            draw: i,
                            at: self.textured.len() as u32,
                        };
                        if self.add(Family::NineSlice.key(at.binding.code()), unit) {
                            self.textured.push(Textured { at, border });
                        }
                    }
                }
                #[cfg(feature = "phosphor-icons")]
                PaintCmd::IconMsdf { draws } => {
                    for i in draws.clone() {
                        // An icon from an unregistered font has no vertices
                        // and its box is empty, so it isn't ordered.
                        if let Some(t) = target {
                            self.boxes.push(draw_box::icon_msdf(t, prepared, i));
                        }
                        self.add(Family::IconMsdf.key(0), UnitRef::IconMsdf(i));
                    }
                }
                PaintCmd::Text { draws } => {
                    // The command's units, one after another: every block
                    // in it belongs to exactly one.
                    while let Some(unit) = prepared.text_units.get(text_unit)
                        && unit.blocks.start < draws.end
                    {
                        debug_assert!(unit.blocks.start >= draws.start);
                        if let Some(t) = target {
                            for block in unit.blocks.clone() {
                                let b = draw_box::text_block(t, prepared, block as usize);
                                self.boxes.push(b);
                            }
                        }
                        self.add(Family::Text.key(0), UnitRef::Text(text_unit as u32));
                        text_unit += 1;
                    }
                }
            }
        }
    }

    /// Start a plan on a target of `size` physical px.
    pub(crate) fn begin(&mut self, size: (u32, u32)) {
        self.order.begin(size);
        self.boxes.clear();
        self.units.clear();
        self.batches.clear();
        self.textured.clear();
        self.next_paint = 0;
        self.cmd = 0;
        self.batch_cmd = 0;
        self.paint_order = false;
        #[cfg(any(test, feature = "paint-order-oracle"))]
        if let Some(checked) = &mut self.checked {
            checked.clear();
        }
    }

    /// Order the next unit in paint order, of `key`, drawn within the boxes
    /// pushed onto [`boxes`](Self::boxes) since the last unit. Whether it
    /// draws: `false` when it is culled.
    fn add(&mut self, key: BatchKey, unit: UnitRef) -> bool {
        let paint = self.next_paint;
        self.next_paint += 1;
        #[cfg(any(test, feature = "paint-order-oracle"))]
        if let Some(checked) = &mut self.checked {
            checked.push((key, self.boxes.clone()));
        }
        if self.paint_order {
            self.units.push(PlannedUnit {
                order: 0,
                key,
                paint,
                unit,
            });
            let end = self.units.len() as u32;
            match self.batches.last_mut() {
                Some(last) if last.key == key && self.batch_cmd == self.cmd => {
                    last.units.end = end;
                }
                _ => {
                    self.batches.push(Batch {
                        key,
                        units: end - 1..end,
                    });
                    self.batch_cmd = self.cmd;
                }
            }
            return true;
        }
        let order = self.order.add(key, &self.boxes);
        self.boxes.clear();
        let Some(order) = order else {
            return false;
        };
        self.units.push(PlannedUnit {
            order,
            key,
            paint,
            unit,
        });
        true
    }

    /// Pass over a draw that doesn't resolve: it takes a paint index, and draws
    /// nothing.
    fn skip(&mut self) {
        self.next_paint += 1;
        #[cfg(any(test, feature = "paint-order-oracle"))]
        if let Some(checked) = &mut self.checked {
            checked.push((BatchKey(0), Vec::new()));
        }
    }

    /// Sort the units into draw order and cut them into batches.
    pub(crate) fn finish(&mut self) {
        // Units of one key all have order 0: they are in draw order already.
        // Otherwise the paint index makes every key unique, so stability
        // isn't needed.
        if !self.order.one_key() {
            self.units
                .sort_unstable_by_key(|u| (u.order, u.key, u.paint));
        }
        let mut start = 0;
        while let Some(first) = self.units.get(start) {
            let key = first.key;
            let end = start
                + self.units[start..]
                    .iter()
                    .take_while(|u| u.key == key)
                    .count();
            self.batches.push(Batch {
                key,
                units: start as u32..end as u32,
            });
            start = end;
        }
    }

    /// The units that draw, in draw order.
    pub(crate) fn units(&self) -> &[PlannedUnit] {
        &self.units
    }

    pub(crate) fn batches(&self) -> &[Batch] {
        &self.batches
    }

    /// Where the sprite or nine-slice unit with `at` samples from.
    pub(crate) fn textured(&self, at: u32) -> &Textured {
        &self.textured[at as usize]
    }

    /// The work ordering took, and how many units it ordered.
    pub(crate) fn stats(&self) -> OrderStats {
        self.order.stats()
    }

    /// Keep every unit's key and boxes from the next plan on, and check each
    /// plan against the brute-force oracle with [`check`](Self::check).
    #[cfg(any(test, feature = "paint-order-oracle"))]
    pub(crate) fn set_check(&mut self, on: bool) {
        self.checked = on.then(Vec::new);
    }

    /// Panic unless the last plan's orders and batch count are what the
    /// brute-force oracle gives for the same units. Does nothing when checking
    /// is off, or for a plan in paint order.
    #[cfg(any(test, feature = "paint-order-oracle"))]
    pub(crate) fn check(&self) {
        let Some(checked) = &self.checked else {
            return;
        };
        if self.paint_order {
            return;
        }
        let expected = super::draw_order::brute_force_orders(checked);
        let mut planned = vec![None; checked.len()];
        for unit in &self.units {
            planned[unit.paint as usize] = Some(unit.order);
        }
        assert_eq!(
            planned, expected,
            "the plan's orders differ from the oracle's"
        );
        assert_eq!(
            self.batches.len(),
            brute_force_batch_count(checked),
            "the plan's batch count differs from the oracle's"
        );
    }

    /// Plan units given directly, each a key and its boxes, in paint order.
    #[cfg(test)]
    pub(crate) fn plan(&mut self, size: (u32, u32), units: &[(BatchKey, Vec<PixelBox>)]) {
        self.begin(size);
        for (i, (key, boxes)) in units.iter().enumerate() {
            self.boxes.extend_from_slice(boxes);
            self.add(*key, UnitRef::Analytic(i as u32));
        }
        self.finish();
    }
}

/// How many batches `units` (each a key and its boxes, in paint order) make,
/// from the brute-force orders: what [`DrawPlan`] must match.
#[cfg(any(test, feature = "paint-order-oracle"))]
pub(crate) fn brute_force_batch_count(units: &[(BatchKey, Vec<PixelBox>)]) -> usize {
    let orders = super::draw_order::brute_force_orders(units);
    let mut sorted: Vec<(u32, BatchKey, usize)> = orders
        .iter()
        .enumerate()
        .filter_map(|(paint, order)| order.map(|o| (o, units[paint].0, paint)))
        .collect();
    sorted.sort();
    let mut count = 0;
    let mut last = None;
    for (_, key, _) in sorted {
        if last != Some(key) {
            count += 1;
            last = Some(key);
        }
    }
    count
}

#[cfg(test)]
#[path = "draw_plan_tests.rs"]
mod tests;
