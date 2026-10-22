//! From a plan's batches to draw calls (`docs/design/draw-call-batching.md`,
//! "Each family uploads once per list, in batch order"): every family's
//! records copied in draw order, ready for one upload each, and a draw per
//! batch over its range of them. Nothing here touches the GPU.

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
    /// that has something to draw.
    pub(crate) fn fill(&mut self, plan: &DrawPlan, list: &DrawList, prepared: &MsdfPrepared) {
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
