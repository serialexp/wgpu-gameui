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
//! square cells over the target ([`CellShift`] picks their side from the
//! boxes of the list before). Each cell keeps the boxes touching it
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

/// The side of a grid cell, as a power of two, from 16 to 64 px. A box reads
/// and writes every cell it touches, so a cell much bigger than the boxes
/// holds many that don't overlap (each lookup tests them all), and one much
/// smaller makes a box visit many cells. Which is cheaper depends on the list:
/// a grid of small disjoint boxes wants small cells, boxes that overlap their
/// neighbours anyway (shadows) want large ones. [`CellTuner`] picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellShift(u32);

impl CellShift {
    const MIN: u32 = 4;
    const MAX: u32 = 6;
    /// The most cells a grid is allowed, whatever the target's size: larger
    /// targets get larger cells.
    const MAX_CELLS: u64 = 1 << 15;

    /// Where in [`CellTuner::cost`] this cell's cost is kept.
    fn slot(self) -> usize {
        (self.0 - Self::MIN) as usize
    }

    /// This cell, or the smallest larger one that keeps a grid over `size`
    /// within [`MAX_CELLS`](Self::MAX_CELLS).
    fn fitting(mut self, size: (u32, u32)) -> Self {
        let cells = |shift: u32| {
            u64::from(size.0.div_ceil(1 << shift).max(1))
                * u64::from(size.1.div_ceil(1 << shift).max(1))
        };
        while self.0 < Self::MAX && cells(self.0) > Self::MAX_CELLS {
            self.0 += 1;
        }
        self
    }

    /// The side of a cell, in physical px.
    pub(crate) fn px(self) -> u32 {
        1 << self.0
    }
}

impl Default for CellShift {
    fn default() -> Self {
        Self(Self::MAX)
    }
}

/// Picks the grid's cell by trying them. How much work ordering a list takes,
/// in cell visits plus overlap tests ([`OrderStats`]), is counted exactly, so
/// the cost of a cell is the cost the list before it had with that cell.
/// Lists that follow one another in a UI are alike, so it moves to the cheapest
/// cell it has measured, tries a neighbour it hasn't, and measures afresh
/// every [`RETUNE_AFTER`] lists and when the number of units changes much. The
/// grid only finds overlaps, so a trial of a worse cell costs time and never
/// changes an order.
#[derive(Debug, Default)]
struct CellTuner {
    /// The cost of a list at each cell size (16, 32, 64 px) when last tried.
    cost: [Option<u64>; 3],
    /// Units in the list the costs are of.
    units: u32,
    /// Lists since the costs were taken afresh.
    age: u32,
}

/// Lists between measuring every cell again, so a UI that settles into a
/// different shape is followed.
const RETUNE_AFTER: u32 = 120;

impl CellTuner {
    /// The cell for the next list, after one of `units` units cost `cost` with
    /// `cell`.
    fn next(&mut self, cell: CellShift, cost: u64, units: u32) -> CellShift {
        self.age += 1;
        // A list a quarter longer or shorter is another list.
        if self.age >= RETUNE_AFTER || units.abs_diff(self.units) * 4 > self.units {
            self.cost = [None; 3];
            self.age = 0;
        }
        self.units = units;
        self.cost[cell.slot()] = Some(cost);
        // The cheapest measured; of equal ones the larger cell, which needs
        // fewer cells.
        let cheapest = (CellShift::MIN..=CellShift::MAX)
            .filter_map(|s| Some((self.cost[CellShift(s).slot()]?, CellShift::MAX - s)))
            .min()
            .map_or(cell.0, |(_, from_max)| CellShift::MAX - from_max);
        if cheapest != cell.0 {
            return CellShift(cheapest);
        }
        [cell.0.checked_sub(1), cell.0.checked_add(1)]
            .into_iter()
            .flatten()
            .filter(|s| (CellShift::MIN..=CellShift::MAX).contains(s))
            .find(|&s| self.cost[CellShift(s).slot()].is_none())
            .map_or(cell, CellShift)
    }
}

/// What units must share to be drawn in one call: the pipeline and what it
/// samples (see `DrawPlan`). Units sort by it within an order, so its value
/// also decides which of two batches at one order draws first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// Units that were given their order instead of looking for it.
    pub(crate) orders_given: u32,
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
    /// The cells written since [`begin`](Self::begin), so the next list resets
    /// those rather than every cell of a grid that may be mostly untouched.
    written: Vec<u32>,
    cols: u32,
    rows: u32,
    entries: Vec<Entry>,
    keys: Keys,
    /// The boxes of the units ordered while they have all had one key, in
    /// paint order.
    pending: Vec<PixelBox>,
    stats: OrderStats,
    cell: CellShift,
    tuner: CellTuner,
}

impl DrawOrder {
    /// Start ordering a list drawn onto a target of `size` physical px.
    pub(crate) fn begin(&mut self, size: (u32, u32)) {
        for index in self.written.drain(..) {
            let cell = &mut self.cells[index as usize];
            cell.entries.clear();
            cell.top = None;
            cell.other = None;
        }
        // Only a list that used the grid says what its cell cost, and one
        // that was given orders didn't look anything up, so it says nothing.
        if self.keys == Keys::Many && self.stats.orders_given == 0 {
            let OrderStats {
                units_ordered,
                overlap_tests,
                cell_visits,
                ..
            } = self.stats;
            self.cell = self
                .tuner
                .next(self.cell, cell_visits + overlap_tests, units_ordered);
        }
        self.cell = self.cell.fitting(size);
        let px = self.cell.px();
        self.cols = size.0.div_ceil(px).max(1);
        self.rows = size.1.div_ceil(px).max(1);
        let cells = (self.cols * self.rows) as usize;
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

    /// Order the next unit in paint order, of `key`, drawn within `boxes`,
    /// finding what it overlaps.
    #[cfg(test)]
    pub(crate) fn add(&mut self, key: BatchKey, boxes: &[PixelBox]) -> Option<u32> {
        self.add_given(key, boxes, None)
    }

    /// Order the next unit in paint order, of `key`, drawn within `boxes`.
    /// Empty boxes are left out; a unit whose boxes are all empty draws
    /// nothing and gets no order. Every box is looked up before any is
    /// stored, so each is stored with the unit's final order.
    ///
    /// `given` is the order the unit is known to get, when the caller knows
    /// it (`DrawPlan` does, for a unit no change reaches): it is stored as
    /// that without looking at what the unit overlaps. It must be what the
    /// lookup would have found; a wrong one is a wrong plan.
    pub(crate) fn add_given(
        &mut self,
        key: BatchKey,
        boxes: &[PixelBox],
        given: Option<u32>,
    ) -> Option<u32> {
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
        let order = match given {
            Some(order) => {
                self.stats.orders_given += 1;
                order
            }
            None => {
                let mut order = 0;
                for b in boxes.iter().filter(|b| !b.is_empty()) {
                    order = self.highest(*b, key, order);
                }
                order
            }
        };
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
                let at = row * self.cols + col;
                let cell = &mut self.cells[at as usize];
                cell.entries.push(index);
                let Some(top) = cell.top else {
                    cell.top = Some(index);
                    self.written.push(at);
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
        let shift = self.cell.0;
        let col = |x: u32| (x >> shift).min(self.cols - 1);
        let row = |y: u32| (y >> shift).min(self.rows - 1);
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
