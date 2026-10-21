//! The order each unit of a draw list is drawn in, from what it overlaps
//! (`docs/design/draw-call-batching.md`, "New: draw order from overlap").
//!
//! Units are taken in paint order. Each gets the highest order among the
//! earlier units it overlaps, plus one where that unit's batch key differs:
//!
//! ```text
//! order(B) = max over earlier units U overlapping B of
//!            order(U) + (1 if key(U) != key(B) else 0)
//! ```
//!
//! and 0 when it overlaps none. Sorted by (order, key, paint index), two units
//! that overlap keep their paint order, while units of one key that overlap
//! nothing between them share an order, and so a draw call.
//!
//! Finding the earlier units a box overlaps goes through a uniform grid of
//! [`CELL_PX`] cells over the target. Each cell keeps the boxes touching it
//! and two summaries that bound what scanning it could find, so a cell that
//! can't raise the order is skipped. The summaries only make it faster; the
//! order is the same as [`brute_force_orders`] gives.
//!
//! While every unit so far has had one key, none can raise another: they all
//! get order 0, and their boxes are set aside rather than put in the grid. A
//! list of one key, such as a polyline's segments, never touches the grid. The
//! first unit of a second key puts the boxes set aside into the grid, as they
//! would have been stored, and ordering goes on from there.

use super::draw_box::PixelBox;

/// The side of a grid cell, in physical px.
pub(crate) const CELL_PX: u32 = 64;

/// What units must share to be drawn in one call: the pipeline and what it
/// samples (see `DrawPlan`). Units sort by it within an order, so its value
/// also decides which of two batches at one order draws first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BatchKey(pub(crate) u64);

/// How much work ordering a list took, and how many units it ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OrderStats {
    /// Units given an order: every unit with a box on the target.
    pub(crate) units_ordered: u32,
    /// Pairs of boxes tested for overlap.
    pub(crate) overlap_tests: u64,
    /// Grid cells looked at or written, one per box per cell.
    pub(crate) cell_visits: u64,
}

/// A box stored in the grid, with its unit's order and key.
#[derive(Clone, Copy, Debug)]
struct Entry {
    pixels: PixelBox,
    order: u32,
    key: BatchKey,
}

/// The keys the units ordered so far have had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Keys {
    #[default]
    None,
    /// All this one: their boxes are in [`DrawOrder::pending`], not the grid.
    One(BatchKey),
    /// More than one: every box is in the grid.
    Many,
}

#[derive(Clone, Debug, Default)]
struct Cell {
    /// Into [`DrawOrder::entries`].
    entries: Vec<u32>,
    /// The entry with the highest (order, key), if any.
    top: Option<u32>,
    /// At least the highest order among entries whose key differs from the
    /// top's. Too high only costs a scan; too low would skip a unit.
    other: Option<u32>,
}

/// Orders a list's units. Reused from list to list: its storage is cleared,
/// never freed.
#[derive(Debug, Default)]
pub(crate) struct DrawOrder {
    cells: Vec<Cell>,
    cols: u32,
    rows: u32,
    entries: Vec<Entry>,
    keys: Keys,
    /// The boxes of the units ordered while they have all had one key, in
    /// paint order.
    pending: Vec<PixelBox>,
    stats: OrderStats,
}

impl DrawOrder {
    /// Start ordering a list drawn onto a target of `size` physical px.
    pub(crate) fn begin(&mut self, size: (u32, u32)) {
        self.cols = size.0.div_ceil(CELL_PX).max(1);
        self.rows = size.1.div_ceil(CELL_PX).max(1);
        let cells = (self.cols * self.rows) as usize;
        for cell in self.cells.iter_mut().take(cells) {
            cell.entries.clear();
            cell.top = None;
            cell.other = None;
        }
        if self.cells.len() < cells {
            self.cells.resize_with(cells, Cell::default);
        }
        self.entries.clear();
        self.keys = Keys::None;
        self.pending.clear();
        self.stats = OrderStats::default();
    }

    /// Whether every unit ordered since [`begin`](Self::begin) has had one
    /// key: they all have order 0, so paint order is already draw order.
    pub(crate) fn one_key(&self) -> bool {
        matches!(self.keys, Keys::None | Keys::One(_))
    }

    /// The work done since [`begin`](Self::begin).
    pub(crate) fn stats(&self) -> OrderStats {
        self.stats
    }

    /// Order the next unit in paint order, of `key`, drawn within `boxes`.
    /// Empty boxes are left out; a unit whose boxes are all empty draws
    /// nothing and gets no order. Every box is looked up before any is
    /// stored, so each is stored with the unit's final order.
    pub(crate) fn add(&mut self, key: BatchKey, boxes: &[PixelBox]) -> Option<u32> {
        if boxes.iter().all(PixelBox::is_empty) {
            return None;
        }
        self.stats.units_ordered += 1;
        match self.keys {
            Keys::None => self.keys = Keys::One(key),
            Keys::One(one) if one == key => {}
            Keys::One(one) => {
                self.store_pending(one);
                self.keys = Keys::Many;
            }
            Keys::Many => {}
        }
        if self.keys != Keys::Many {
            let on_target = boxes.iter().filter(|b| !b.is_empty());
            self.pending.extend(on_target);
            return Some(0);
        }
        let mut order = 0;
        for b in boxes.iter().filter(|b| !b.is_empty()) {
            order = self.highest(*b, key, order);
        }
        for b in boxes.iter().filter(|b| !b.is_empty()) {
            self.store(*b, key, order);
        }
        Some(order)
    }

    /// Put the boxes set aside while every unit had `key` into the grid, each
    /// with order 0, as [`add`](Self::add) would have stored them.
    fn store_pending(&mut self, key: BatchKey) {
        let pending = std::mem::take(&mut self.pending);
        for b in &pending {
            self.store(*b, key, 0);
        }
        self.pending = pending;
        self.pending.clear();
    }

    /// The order a unit of `key` needs to draw after every stored box that
    /// overlaps `pixels`, or `floor` if that is higher.
    fn highest(&mut self, pixels: PixelBox, key: BatchKey, floor: u32) -> u32 {
        let mut best = floor;
        let (cx, cy) = self.cell_span(pixels);
        for row in cy.clone() {
            for col in cx.clone() {
                self.stats.cell_visits += 1;
                let cell = &self.cells[(row * self.cols + col) as usize];
                let Some(top) = cell.top else {
                    continue;
                };
                let top = self.entries[top as usize];
                // The most any entry here could ask for.
                let bound = if top.key == key {
                    cell.other.map_or(top.order, |o| top.order.max(o + 1))
                } else {
                    top.order + 1
                };
                if bound <= best {
                    continue;
                }
                // The top first, as it is the likeliest to be the highest
                // (as gpui does), then the rest.
                let top_index = cell.top;
                let rest = cell.entries.iter().filter(|&&i| Some(i) != top_index);
                for &i in top_index.iter().chain(rest) {
                    let e = self.entries[i as usize];
                    let asks = e.order + u32::from(e.key != key);
                    if asks <= best {
                        continue;
                    }
                    self.stats.overlap_tests += 1;
                    if e.pixels.overlaps(&pixels) {
                        best = asks;
                        if best >= bound {
                            break;
                        }
                    }
                }
            }
        }
        best
    }

    fn store(&mut self, pixels: PixelBox, key: BatchKey, order: u32) {
        let index = self.entries.len() as u32;
        self.entries.push(Entry { pixels, order, key });
        let (cx, cy) = self.cell_span(pixels);
        for row in cy {
            for col in cx.clone() {
                self.stats.cell_visits += 1;
                let cell = &mut self.cells[(row * self.cols + col) as usize];
                cell.entries.push(index);
                let Some(top) = cell.top else {
                    cell.top = Some(index);
                    continue;
                };
                let top = self.entries[top as usize];
                if (order, key) > (top.order, top.key) {
                    cell.top = Some(index);
                    if top.key != key {
                        cell.other = Some(cell.other.map_or(top.order, |o| o.max(top.order)));
                    }
                } else if key != top.key {
                    cell.other = Some(cell.other.map_or(order, |o| o.max(order)));
                }
            }
        }
    }

    /// The columns and rows of the cells `pixels` touches. Boxes come cut to
    /// the target; one that isn't is held to its last cells.
    fn cell_span(&self, pixels: PixelBox) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
        let col = |x: u32| (x / CELL_PX).min(self.cols - 1);
        let row = |y: u32| (y / CELL_PX).min(self.rows - 1);
        (
            col(pixels.x0)..col(pixels.x1 - 1) + 1,
            row(pixels.y0)..row(pixels.y1 - 1) + 1,
        )
    }
}

/// [`DrawOrder`]'s orders for `units` (each a key and its boxes, in paint
/// order), found by testing every earlier unit: what the grid must match.
/// Library code rather than a test helper, so `UiRenderer::set_check_plan`
/// can check a real list's plan with it.
#[cfg(any(test, feature = "paint-order-oracle"))]
pub(crate) fn brute_force_orders(units: &[(BatchKey, Vec<PixelBox>)]) -> Vec<Option<u32>> {
    let mut orders: Vec<Option<u32>> = Vec::with_capacity(units.len());
    for (b, (key, boxes)) in units.iter().enumerate() {
        if boxes.iter().all(PixelBox::is_empty) {
            orders.push(None);
            continue;
        }
        let mut order = 0;
        for (u, (other_key, other_boxes)) in units[..b].iter().enumerate() {
            let Some(other_order) = orders[u] else {
                continue;
            };
            let overlaps = boxes
                .iter()
                .any(|p| other_boxes.iter().any(|q| p.overlaps(q)));
            if overlaps {
                order = order.max(other_order + u32::from(other_key != key));
            }
        }
        orders.push(Some(order));
    }
    orders
}

#[cfg(test)]
#[path = "draw_order_support.rs"]
pub(crate) mod tests_support;

#[cfg(test)]
#[path = "draw_order_tests.rs"]
mod tests;
