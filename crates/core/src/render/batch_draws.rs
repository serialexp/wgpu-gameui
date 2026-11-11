//! From a plan's batches to draw calls (`docs/design/draw-call-batching.md`,
//! "Each family uploads once per list, in batch order"): every family's
//! records copied in draw order, ready for one upload each, and a draw per
//! batch over its range of them. Nothing here touches the GPU.
//!
//! A fill can also note where each of the list's records went
//! ([`Positions`]), so a place that keeps its arrays from frame to frame can
//! rewrite only the records that changed (`docs/design/frame-reuse.md`,
//! Phase 5).

use std::ops::Range;

use super::draw_plan::{DrawPlan, UnitRef};
use super::textures::{Binding, Resolved, SpriteId};
use super::ui_renderer::{IconInstance, NineSliceInstance, icon_instance, nine_slice_instance};
use crate::text::{MsdfInstance, MsdfPrepared};
use crate::widgets::{AnalyticInstance, CircleInstance, DrawList, PaintCmd};

/// The pipeline a draw uses, and what it samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawKind {
    /// Indices into the list's soup vertices.
    Soup,
    Analytic,
    Circle,
    /// Sprite instances sampling a texture; a YUV binding draws through the
    /// YUV pipeline.
    Sprite(Binding),
    NineSlice(Binding),
    Text,
    #[cfg(feature = "phosphor-icons")]
    IconMsdf,
}

/// One draw call: `range` of its family's records in [`BatchDraws`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BatchDraw {
    pub(crate) kind: DrawKind,
    pub(crate) range: Range<u32>,
    /// Drawing small sprites from a texture of their own, which a shared
    /// atlas would save a texture switch on
    /// ([`RenderStats::small_texture_batches`](crate::RenderStats::small_texture_batches)).
    pub(crate) small_texture: Option<SpriteId>,
}

/// A record the plan did not draw (culled), in [`Positions`].
pub(crate) const NOT_DRAWN: u32 = u32::MAX;

/// Where a fill put each of a list's records in the family arrays of
/// [`BatchDraws`], [`NOT_DRAWN`] for one it did not.
#[derive(Default)]
pub(crate) struct Positions {
    /// By analytic record.
    pub(crate) analytic: Vec<u32>,
    /// By circle record.
    pub(crate) circles: Vec<u32>,
    /// By sprite draw: its position, and the plan's texture entry.
    pub(crate) sprites: Vec<[u32; 2]>,
    /// By nine-slice draw: its position, and the plan's texture entry.
    pub(crate) nine_slices: Vec<[u32; 2]>,
    /// By prepared text unit: what to add to an index into
    /// `MsdfPrepared::text_instances` within the unit to find it in the
    /// batch's text array. `None` for a unit not drawn.
    pub(crate) text_shift: Vec<Option<i64>>,
    /// By text block: its unit in `MsdfPrepared::text_units`, or
    /// [`NOT_DRAWN`] for a block in none.
    pub(crate) block_unit: Vec<u32>,
    /// By MSDF icon: its first instance's position and its instance count.
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icons: Vec<[u32; 2]>,
}

impl Positions {
    /// Every record of `list` (whose text and icons are `prepared`) not drawn.
    fn reset(&mut self, list: &DrawList, prepared: &MsdfPrepared) {
        let not_drawn = |v: &mut Vec<u32>, n: usize| {
            v.clear();
            v.resize(n, NOT_DRAWN);
        };
        not_drawn(&mut self.analytic, list.analytic_instances.len());
        not_drawn(&mut self.circles, list.circle_instances.len());
        self.sprites.clear();
        self.sprites.resize(list.icons.len(), [NOT_DRAWN; 2]);
        self.nine_slices.clear();
        self.nine_slices
            .resize(list.nine_slices.len(), [NOT_DRAWN; 2]);
        self.text_shift.clear();
        self.text_shift.resize(prepared.text_units.len(), None);
        not_drawn(&mut self.block_unit, list.texts.len());
        for (u, unit) in prepared.text_units.iter().enumerate() {
            for b in unit.blocks.clone() {
                if let Some(slot) = self.block_unit.get_mut(b as usize) {
                    *slot = u as u32;
                }
            }
        }
        #[cfg(feature = "phosphor-icons")]
        {
            self.icons.clear();
            self.icons.resize(list.icons_msdf.len(), [NOT_DRAWN; 2]);
        }
    }

    /// Bytes reserved.
    pub(crate) fn bytes(&self) -> usize {
        use std::mem::size_of;
        let icons = {
            #[cfg(feature = "phosphor-icons")]
            {
                self.icons.capacity() * size_of::<[u32; 2]>()
            }
            #[cfg(not(feature = "phosphor-icons"))]
            0
        };
        (self.analytic.capacity() + self.circles.capacity() + self.block_unit.capacity())
            * size_of::<u32>()
            + (self.sprites.capacity() + self.nine_slices.capacity()) * size_of::<[u32; 2]>()
            + self.text_shift.capacity() * size_of::<Option<i64>>()
            + icons
    }
}

/// A list's records in draw order, one array per family, and the draws over
/// them. Reused from list to list: cleared, never freed.
#[derive(Default)]
pub(crate) struct BatchDraws {
    /// Soup indices, into the list's vertices as they are.
    pub(crate) indices: Vec<u32>,
    pub(crate) analytic: Vec<AnalyticInstance>,
    pub(crate) circles: Vec<CircleInstance>,
    pub(crate) sprites: Vec<IconInstance>,
    pub(crate) nine_slices: Vec<NineSliceInstance>,
    pub(crate) text: Vec<MsdfInstance>,
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icons: Vec<MsdfInstance>,
    pub(crate) draws: Vec<BatchDraw>,
}

impl BatchDraws {
    /// Copy the records of `plan`'s units, planned from `list` with its text
    /// and icons `prepared`, in draw order, and make a draw of each batch
    /// that has something to draw; and, given `positions`, note where each
    /// record went.
    pub(crate) fn fill(
        &mut self,
        plan: &DrawPlan,
        list: &DrawList,
        prepared: &MsdfPrepared,
        mut positions: Option<&mut Positions>,
    ) {
        if let Some(positions) = positions.as_deref_mut() {
            positions.reset(list, prepared);
        }
        self.indices.clear();
        self.analytic.clear();
        self.circles.clear();
        self.sprites.clear();
        self.nine_slices.clear();
        self.text.clear();
        #[cfg(feature = "phosphor-icons")]
        self.icons.clear();
        self.draws.clear();

        for batch in plan.batches() {
            let units = &plan.units()[batch.units.start as usize..batch.units.end as usize];
            let Some(first) = units.first() else {
                continue;
            };
            let kind = match first.unit {
                UnitRef::Soup { .. } => DrawKind::Soup,
                UnitRef::Analytic(_) => DrawKind::Analytic,
                UnitRef::Circle(_) => DrawKind::Circle,
                UnitRef::Sprite { at, .. } => DrawKind::Sprite(plan.textured(at).at.binding),
                UnitRef::NineSlice { at, .. } => DrawKind::NineSlice(plan.textured(at).at.binding),
                UnitRef::Text(_) => DrawKind::Text,
                #[cfg(feature = "phosphor-icons")]
                UnitRef::IconMsdf(_) => DrawKind::IconMsdf,
            };
            let start = self.len(kind);
            for unit in units {
                if let Some(positions) = positions.as_deref_mut() {
                    self.note(prepared, unit.unit, positions);
                }
                self.push(plan, list, prepared, unit.unit);
            }
            let end = self.len(kind);
            if end == start {
                continue;
            }
            // Every unit of a batch samples the same texture.
            let small_texture = match first.unit {
                UnitRef::Sprite { at, .. } | UnitRef::NineSlice { at, .. } => {
                    match plan.textured(at).at {
                        at @ Resolved {
                            binding: Binding::Own(id),
                            ..
                        } if at.small_own() => Some(id),
                        _ => None,
                    }
                }
                _ => None,
            };
            self.draws.push(BatchDraw {
                kind,
                range: start..end,
                small_texture,
            });
        }
    }

    /// Add a draw of soup `indices` after every batch: soup recorded after a
    /// list's last paint command, which paints over everything else.
    pub(crate) fn push_soup(&mut self, indices: &[u32]) {
        if indices.is_empty() {
            return;
        }
        let start = self.indices.len() as u32;
        self.indices.extend_from_slice(indices);
        self.draws.push(BatchDraw {
            kind: DrawKind::Soup,
            range: start..self.indices.len() as u32,
            small_texture: None,
        });
    }

    /// How many records of `kind`'s family there are so far.
    fn len(&self, kind: DrawKind) -> u32 {
        (match kind {
            DrawKind::Soup => self.indices.len(),
            DrawKind::Analytic => self.analytic.len(),
            DrawKind::Circle => self.circles.len(),
            DrawKind::Sprite(_) => self.sprites.len(),
            DrawKind::NineSlice(_) => self.nine_slices.len(),
            DrawKind::Text => self.text.len(),
            #[cfg(feature = "phosphor-icons")]
            DrawKind::IconMsdf => self.icons.len(),
        }) as u32
    }

    /// Note in `positions` where `unit`'s records are about to go: at the end
    /// of their family's array.
    fn note(&self, prepared: &MsdfPrepared, unit: UnitRef, positions: &mut Positions) {
        match unit {
            UnitRef::Soup { .. } => {}
            UnitRef::Analytic(i) => positions.analytic[i as usize] = self.analytic.len() as u32,
            UnitRef::Circle(i) => positions.circles[i as usize] = self.circles.len() as u32,
            UnitRef::Sprite { draw, at } => {
                positions.sprites[draw as usize] = [self.sprites.len() as u32, at];
            }
            UnitRef::NineSlice { draw, at } => {
                positions.nine_slices[draw as usize] = [self.nine_slices.len() as u32, at];
            }
            UnitRef::Text(i) => {
                let start = prepared.text_units[i as usize].instances.start;
                positions.text_shift[i as usize] = Some(self.text.len() as i64 - i64::from(start));
            }
            #[cfg(feature = "phosphor-icons")]
            UnitRef::IconMsdf(i) => {
                let range = prepared.icon_range(i..i + 1);
                positions.icons[i as usize] = [self.icons.len() as u32, range.len() as u32];
            }
        }
    }

    /// Bytes reserved.
    pub(crate) fn bytes(&self) -> usize {
        use std::mem::size_of;
        let icons = {
            #[cfg(feature = "phosphor-icons")]
            {
                self.icons.capacity() * size_of::<MsdfInstance>()
            }
            #[cfg(not(feature = "phosphor-icons"))]
            0
        };
        self.indices.capacity() * size_of::<u32>()
            + self.analytic.capacity() * size_of::<AnalyticInstance>()
            + self.circles.capacity() * size_of::<CircleInstance>()
            + self.sprites.capacity() * size_of::<IconInstance>()
            + self.nine_slices.capacity() * size_of::<NineSliceInstance>()
            + self.text.capacity() * size_of::<MsdfInstance>()
            + self.draws.capacity() * size_of::<BatchDraw>()
            + icons
    }

    /// `Ok` when every array and draw is `other`'s to the byte, or the first
    /// family that is not.
    #[cfg(feature = "paint-order-oracle")]
    pub(crate) fn same_as(&self, other: &Self) -> Result<(), &'static str> {
        fn bytes<T: bytemuck::Pod>(v: &[T]) -> &[u8] {
            bytemuck::cast_slice(v)
        }
        let families: [(&'static str, &[u8], &[u8]); 6] = [
            ("indices", bytes(&self.indices), bytes(&other.indices)),
            ("analytic", bytes(&self.analytic), bytes(&other.analytic)),
            ("circles", bytes(&self.circles), bytes(&other.circles)),
            ("sprites", bytes(&self.sprites), bytes(&other.sprites)),
            (
                "nine-slices",
                bytes(&self.nine_slices),
                bytes(&other.nine_slices),
            ),
            ("text", bytes(&self.text), bytes(&other.text)),
        ];
        if let Some((name, ..)) = families.iter().find(|(_, a, b)| a != b) {
            return Err(name);
        }
        #[cfg(feature = "phosphor-icons")]
        if bytes(&self.icons) != bytes(&other.icons) {
            return Err("icons");
        }
        if self.draws != other.draws {
            return Err("draws");
        }
        Ok(())
    }

    /// Copy `unit`'s records onto its family's array.
    fn push(&mut self, plan: &DrawPlan, list: &DrawList, prepared: &MsdfPrepared, unit: UnitRef) {
        match unit {
            UnitRef::Soup { cmd } => {
                let PaintCmd::Soup { indices } = &list.paint_cmds[cmd as usize] else {
                    unreachable!("a soup unit is a soup command");
                };
                self.indices
                    .extend_from_slice(&list.indices[indices.start as usize..indices.end as usize]);
            }
            UnitRef::Analytic(i) => self.analytic.push(list.analytic_instances[i as usize]),
            UnitRef::Circle(i) => self.circles.push(list.circle_instances[i as usize]),
            UnitRef::Sprite { draw, at } => {
                let at = &plan.textured(at).at;
                self.sprites
                    .push(icon_instance(&list.icons[draw as usize], at));
            }
            UnitRef::NineSlice { draw, at } => {
                let textured = plan.textured(at);
                self.nine_slices.push(nine_slice_instance(
                    &list.nine_slices[draw as usize],
                    &textured.at,
                    textured.border,
                ));
            }
            UnitRef::Text(i) => {
                let range = prepared.text_units[i as usize].instances.clone();
                self.text.extend_from_slice(
                    &prepared.text_instances[range.start as usize..range.end as usize],
                );
            }
            #[cfg(feature = "phosphor-icons")]
            UnitRef::IconMsdf(i) => {
                let range = prepared.icon_range(i..i + 1);
                self.icons.extend_from_slice(
                    &prepared.icon_instances[range.start as usize..range.end as usize],
                );
            }
        }
    }
}
