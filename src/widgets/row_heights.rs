//! Row heights for a [`List`](super::List) whose rows are not all one height.
//!
//! Each row starts with an estimate and gets its measured height when someone
//! measures it: the list, for the rows it shows, or the app, for rows out of
//! view (agent-ui measures those on a background thread). [`RowHeights`]
//! answers where a row starts, how tall it is, and which row covers a given
//! position, each in O(log n) for n rows, so a frame's work grows with the
//! rows in view and not with the history.
//!
//! Rows may carry a key: the identity of their content. A measured height is
//! remembered by key, so it outlives the rows being rebuilt (another page put
//! in front, the same conversation loaded again) until
//! [`RowHeights::reestimate`] says the width or fonts changed.
//!
//! Positions are `f64`. A long history runs to hundreds of thousands of
//! pixels, where an `f32` keeps only a few bits of fraction; summing and
//! subtracting there in `f32` would place rows by whole pixels.
//!
//! See `docs/design/measured-row-heights.md`.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicI64, Ordering};

/// One row as handed to [`RowHeights`]: its height, whether that is a
/// measurement or an estimate, and optionally the key it is remembered by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    height: f32,
    measured: bool,
    key: Option<u64>,
}

impl Row {
    /// A row whose height is a guess, until it is measured.
    pub fn estimated(height: f32) -> Self {
        Self {
            height: height.max(0.0),
            measured: false,
            key: None,
        }
    }

    /// A row whose height is known: no need to measure it.
    pub fn measured(height: f32) -> Self {
        Self {
            measured: true,
            ..Self::estimated(height)
        }
    }

    /// Remember this row's measured height by `key`: a row added later with
    /// the same key starts measured, at that height.
    pub fn keyed(self, key: u64) -> Self {
        Self {
            key: Some(key),
            ..self
        }
    }
}

/// A row's place in a [`RowHeights`] that stays its own while rows are added
/// at either end or replaced, so a height measured elsewhere finds its row
/// however many pages went in front meanwhile. Clearing or rebuilding gives
/// the rows new ones; an old one then finds no row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId(i64);

/// What a height measured elsewhere needs to find its row and be trusted:
/// the row's [`RowId`], its key, and the layout (width and fonts) it was
/// measured for. Take one with [`RowHeights::ticket`] when handing the row
/// out, and give it back with the height to [`RowHeights::apply_measured`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    id: RowId,
    key: Option<u64>,
    layout: u64,
}

impl Ticket {
    /// The row it was taken for.
    pub fn id(&self) -> RowId {
        self.id
    }
}

/// The next range of [`RowId`]s, shared by every [`RowHeights`] so an id
/// from one never finds a row in another.
static NEXT_EPOCH: AtomicI64 = AtomicI64::new(0);

/// The heights of a list's rows, estimated or measured. See the module docs.
#[derive(Debug, Clone)]
pub struct RowHeights {
    index: Deque,
    measured: VecDeque<bool>,
    keys: VecDeque<Option<u64>>,
    /// Row 0's [`RowId`]. A page put in front takes the ids below it.
    first: i64,
    /// Measured heights by key, for the current width and fonts.
    remembered: KeyMap<f32>,
    /// The width rows are measured at; 0 until it is first set.
    width: f32,
    /// Counts the times every measurement stopped holding (a new width,
    /// [`reestimate`](RowHeights::reestimate)), so a height measured before
    /// is not taken after.
    layout: u64,
}

impl Default for RowHeights {
    fn default() -> Self {
        let mut rows = Self {
            index: Deque::default(),
            measured: VecDeque::new(),
            keys: VecDeque::new(),
            first: 0,
            remembered: KeyMap::default(),
            width: 0.0,
            layout: 0,
        };
        rows.next_epoch();
        rows
    }
}

impl RowHeights {
    /// Each new list, clear or rebuild numbers its rows from its own
    /// 2³²-wide range, starting in the middle so pages can go in front.
    /// Rebuilding 60 times a second would last a year.
    const EPOCH: i64 = 1 << 32;

    /// No rows, and nothing remembered.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.measured.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.measured.is_empty()
    }

    /// Take every row away. Remembered heights stay, for the rows that come
    /// back.
    pub fn clear(&mut self) {
        self.index = Deque::default();
        self.measured.clear();
        self.keys.clear();
        self.next_epoch();
    }

    /// Add `row` at the end.
    pub fn push(&mut self, row: Row) {
        self.extend([row]);
    }

    /// Add `rows` at the end. Loading a whole list this way builds its
    /// offsets in one pass.
    pub fn extend(&mut self, rows: impl IntoIterator<Item = Row>) {
        let rows = rows.into_iter();
        let mut heights = Vec::with_capacity(rows.size_hint().0);
        for row in rows {
            let row = self.recall(row);
            heights.push(row.height);
            self.measured.push_back(row.measured);
            self.keys.push_back(row.key);
            self.remember(&row);
        }
        self.index.extend(&heights);
    }

    /// Put `rows` in place of the rows in `range`. Adding at either end costs
    /// O(log n) a row, so does replacing rows one for one (see
    /// [`replace`](Self::replace)); any other change rebuilds, O(n), and
    /// gives every row a new [`RowId`].
    pub fn splice(&mut self, range: Range<usize>, rows: impl IntoIterator<Item = Row>) {
        let rows: Vec<Row> = rows.into_iter().map(|row| self.recall(row)).collect();
        if rows.len() == range.len() {
            for (offset, row) in rows.into_iter().enumerate() {
                self.replace(range.start + offset, row);
            }
        } else if range.is_empty() && range.start == self.len() {
            self.extend(rows);
        } else if range.is_empty() && range.start == 0 {
            self.prepend(rows);
        } else {
            self.rebuild(range, rows);
        }
    }

    /// Put `row` in place of row `index`: its content changed. O(log n). The
    /// row keeps its [`RowId`]; give rows keys so a height measured for the
    /// old content isn't taken for the new.
    pub fn replace(&mut self, index: usize, row: Row) {
        let row = self.recall(row);
        self.index.set(index, row.height);
        self.measured[index] = row.measured;
        self.keys[index] = row.key;
        self.remember(&row);
    }

    /// Give row `index` its measured `height`, and remember it by the row's
    /// key. O(log n).
    pub fn set_measured(&mut self, index: usize, height: f32) {
        let height = height.max(0.0);
        self.index.set(index, height);
        self.measured[index] = true;
        if let Some(key) = self.keys[index] {
            self.remembered.insert(key, height);
        }
    }

    /// What to hand out with row `index` to measure it elsewhere.
    pub fn ticket(&self, index: usize) -> Ticket {
        Ticket {
            id: self.id_of(index),
            key: self.keys[index],
            layout: self.layout,
        }
    }

    /// A height measured elsewhere (a background thread) for the row
    /// `ticket` was taken for. If that row is still here with the same key
    /// and still estimated, it takes the height; a row already measured keeps
    /// its own measurement, which is the newer one. Either way the height is
    /// remembered by the key, for when the row comes back. A height measured
    /// for an earlier width or fonts is dropped.
    pub fn apply_measured(&mut self, ticket: Ticket, height: f32) {
        if ticket.layout != self.layout {
            return;
        }
        let Ticket { id, key, .. } = ticket;
        let height = height.max(0.0);
        match self.index_of(id) {
            Some(index) if self.keys[index] == key => {
                if !self.measured[index] {
                    self.set_measured(index, height);
                }
            }
            _ => {
                if let Some(key) = key {
                    self.remembered.insert(key, height);
                }
            }
        }
    }

    /// The width or the fonts changed, so no measurement holds: every row
    /// is the [`Row`] `row` gives for its index (an estimate, mostly; a row
    /// whose height is known regardless, such as one of no height, can say
    /// so), and remembered heights are forgotten. O(n). Rows keep their
    /// [`RowId`]s.
    pub fn reestimate(&mut self, mut row: impl FnMut(usize) -> Row) {
        self.layout += 1;
        self.remembered.clear();
        let mut heights = Vec::with_capacity(self.len());
        for index in 0..self.len() {
            let row = row(index);
            heights.push(row.height);
            self.measured[index] = row.measured;
            self.keys[index] = row.key;
            self.remember(&row);
        }
        // The ids count from row 0, wherever the index's halves meet.
        self.index = Deque::from_heights(heights);
    }

    /// The width rows are measured at; a [`List`](super::List) drawing
    /// these rows sets it. A new width turns every measurement back into an
    /// estimate (the height stays, as a close guess, until the row is
    /// measured again) and forgets remembered heights. Returns whether it
    /// did: not for the first width, nor the same one again.
    pub fn set_width(&mut self, width: f32) -> bool {
        let changed = self.width != 0.0 && self.width != width;
        self.width = width;
        if changed {
            self.layout += 1;
            self.remembered.clear();
            self.measured
                .iter_mut()
                .for_each(|measured| *measured = false);
        }
        changed
    }

    /// The width rows are measured at (see [`set_width`](Self::set_width)).
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Row `index`'s [`RowId`].
    pub fn id_of(&self, index: usize) -> RowId {
        RowId(self.first + index as i64)
    }

    /// The row holding `id` now, if it still does.
    pub fn index_of(&self, id: RowId) -> Option<usize> {
        usize::try_from(id.0 - self.first)
            .ok()
            .filter(|&index| index < self.len())
    }

    /// Where row `index` starts: the heights of the rows above it. `index ==
    /// len()` gives the total.
    pub fn offset_of(&self, index: usize) -> f64 {
        self.index.prefix(index)
    }

    /// Row `index`'s height, measured or estimated.
    pub fn height_of(&self, index: usize) -> f32 {
        self.index.get(index)
    }

    /// Whether row `index`'s height is a measurement rather than an estimate.
    pub fn is_measured(&self, index: usize) -> bool {
        self.measured[index]
    }

    /// Row `index`'s key, if it has one.
    pub fn key_of(&self, index: usize) -> Option<u64> {
        self.keys[index]
    }

    /// Every row's height, together.
    pub fn total(&self) -> f64 {
        self.index.total()
    }

    /// The row covering `y`, counted from the top of the first row: `None`
    /// above it or past the last. A row of no height covers nothing.
    pub fn row_at(&self, y: f64) -> Option<usize> {
        if y < 0.0 || y >= self.total() {
            return None;
        }
        Some(self.index.find(y))
    }

    /// The rows still estimated, nearest `index` first (below before above
    /// at equal distance): the order to measure them in so the rows the
    /// reader reaches next settle first. It steps over measured rows one by
    /// one, so take what a batch needs from one call rather than calling it
    /// afresh for each row.
    pub fn unmeasured_near(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let len = self.len();
        let start = index.min(len);
        let mut below = start;
        let mut above = start;
        std::iter::from_fn(move || {
            loop {
                // `below` is the next row down, `above - 1` the next up.
                let take_below = below < len && (above == 0 || below - start <= start + 1 - above);
                let next = if take_below {
                    below += 1;
                    below - 1
                } else if above > 0 {
                    above -= 1;
                    above
                } else {
                    return None;
                };
                if !self.measured[next] {
                    return Some(next);
                }
            }
        })
    }

    /// Put `rows` above row 0, taking the ids below it.
    fn prepend(&mut self, rows: Vec<Row>) {
        let heights: Vec<f32> = rows.iter().map(|row| row.height).collect();
        self.index.prepend(&heights);
        self.first -= rows.len() as i64;
        for row in rows.iter().rev() {
            self.measured.push_front(row.measured);
            self.keys.push_front(row.key);
            self.remember(row);
        }
    }

    /// Any other change: lay every row out afresh, under new ids. O(n).
    fn rebuild(&mut self, range: Range<usize>, rows: Vec<Row>) {
        let mut heights: Vec<f32> = (0..self.len()).map(|i| self.index.get(i)).collect();
        heights.splice(range.clone(), rows.iter().map(|row| row.height));
        let mut measured: Vec<bool> = self.measured.drain(..).collect();
        measured.splice(range.clone(), rows.iter().map(|row| row.measured));
        let mut keys: Vec<Option<u64>> = self.keys.drain(..).collect();
        keys.splice(range, rows.iter().map(|row| row.key));
        self.index = Deque::from_heights(heights);
        self.measured = measured.into();
        self.keys = keys.into();
        rows.iter().for_each(|row| self.remember(row));
        self.next_epoch();
    }

    fn next_epoch(&mut self) {
        let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
        self.first = epoch
            .wrapping_mul(Self::EPOCH)
            .wrapping_add(Self::EPOCH / 2);
    }

    /// `row`, measured at its remembered height if its key has one.
    fn recall(&self, row: Row) -> Row {
        if self.remembered.is_empty() {
            return row;
        }
        match row.key.and_then(|key| self.remembered.get(&key)) {
            Some(&height) if !row.measured => Row {
                height,
                measured: true,
                key: row.key,
            },
            _ => row,
        }
    }

    /// Remember a row handed in already measured, by its key.
    fn remember(&mut self, row: &Row) {
        if let (true, Some(key)) = (row.measured, row.key) {
            self.remembered.insert(key, row.height);
        }
    }
}

/// Keys are already hashes (agent-ui's are a row's content hashed), so the
/// map by key mixes them with one multiply rather than hashing them again.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(self.0.rotate_left(8) ^ u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        // Fibonacci hashing: spreads keys that differ only in low bits.
        self.0 = value.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

type KeyMap<V> = HashMap<u64, V, BuildHasherDefault<KeyHasher>>;

/// Prefix sums over row heights, the way [`RowHeights`] needs them.
trait HeightIndex: Default {
    fn from_heights(heights: Vec<f32>) -> Self;
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> f32;
    fn push(&mut self, height: f32);
    fn set(&mut self, index: usize, height: f32);
    fn splice(&mut self, range: Range<usize>, heights: impl Iterator<Item = f32>);
    /// Add `heights` at the end.
    fn extend(&mut self, heights: &[f32]) {
        for &height in heights {
            self.push(height);
        }
    }
    /// Put `heights` above row 0.
    fn prepend(&mut self, heights: &[f32]) {
        self.splice(0..0, heights.iter().copied());
    }
    /// The heights of rows `0..index`.
    fn prefix(&self, index: usize) -> f64;
    fn total(&self) -> f64 {
        self.prefix(self.len())
    }
    /// The first row whose end is past `y` (`len()` when none is): the row
    /// covering `y`, skipping rows of no height.
    fn find(&self, y: f64) -> usize;
}

/// The lowest set bit of `i` (`0` for `0`): how many values Fenwick slot `i`
/// covers. `usize::isolate_lowest_one` once it is stable.
fn lowest_bit(i: usize) -> usize {
    i & i.wrapping_neg()
}

/// A Fenwick tree (binary indexed tree) over `f64` values: `sums[i]` holds
/// the sum of the `i & -i` values ending at value `i - 1`, so a prefix sum
/// or a change to one value walks about log₂ n slots.
#[derive(Debug, Clone, Default)]
struct Sums {
    /// One-based: `sums[0]` is unused.
    sums: Vec<f64>,
}

impl Sums {
    fn from_values(values: impl ExactSizeIterator<Item = f64>) -> Self {
        let n = values.len();
        let mut sums = Vec::with_capacity(n + 1);
        sums.push(0.0);
        sums.extend(values);
        for i in 1..=n {
            let parent = i + lowest_bit(i);
            if parent <= n {
                sums[parent] += sums[i];
            }
        }
        Self { sums }
    }

    fn len(&self) -> usize {
        self.sums.len().saturating_sub(1)
    }

    fn push(&mut self, value: f64) {
        if self.sums.is_empty() {
            self.sums.push(0.0);
        }
        let i = self.sums.len();
        // Slot `i` covers values `i - lowest_bit(i) .. i`; all but the new one
        // are already in the tree.
        let covered = self.prefix(i - 1) - self.prefix(i - lowest_bit(i));
        self.sums.push(covered + value);
    }

    fn add(&mut self, index: usize, delta: f64) {
        let mut i = index + 1;
        while i < self.sums.len() {
            self.sums[i] += delta;
            i += lowest_bit(i);
        }
    }

    fn prefix(&self, count: usize) -> f64 {
        let mut i = count.min(self.len());
        let mut sum = 0.0;
        while i > 0 {
            sum += self.sums[i];
            i -= lowest_bit(i);
        }
        sum
    }

    /// How many leading values sum to at most `y`. Values are never
    /// negative, so the prefix sums only grow and one walk down finds it.
    fn count_within(&self, y: f64) -> usize {
        self.descend(|sum, left| sum <= left, y)
    }

    /// How many leading values sum to less than `y`.
    fn count_below(&self, y: f64) -> usize {
        self.descend(|sum, left| sum < left, y)
    }

    /// The longest run of leading values whose sum `fits` `y`, found by one
    /// walk down the tree.
    fn descend(&self, fits: impl Fn(f64, f64) -> bool, y: f64) -> usize {
        let n = self.len();
        let mut position = 0;
        let mut left = y;
        let mut step = if n == 0 { 0 } else { 1 << n.ilog2() };
        while step > 0 {
            let next = position + step;
            if next <= n && fits(self.sums[next], left) {
                position = next;
                left -= self.sums[next];
            }
            step >>= 1;
        }
        position
    }
}

/// Every row's height in one Fenwick tree. Changing a height or reading an
/// offset is O(log n); putting rows anywhere but the end rebuilds, O(n).
#[derive(Debug, Clone, Default)]
struct Fenwick {
    heights: Vec<f32>,
    sums: Sums,
}

impl HeightIndex for Fenwick {
    fn from_heights(heights: Vec<f32>) -> Self {
        let sums = Sums::from_values(heights.iter().map(|&h| f64::from(h)));
        Self { heights, sums }
    }

    fn len(&self) -> usize {
        self.heights.len()
    }

    fn get(&self, index: usize) -> f32 {
        self.heights[index]
    }

    fn push(&mut self, height: f32) {
        self.heights.push(height);
        self.sums.push(f64::from(height));
    }

    fn set(&mut self, index: usize, height: f32) {
        let delta = f64::from(height) - f64::from(self.heights[index]);
        self.heights[index] = height;
        if delta != 0.0 {
            self.sums.add(index, delta);
        }
    }

    fn splice(&mut self, range: Range<usize>, heights: impl Iterator<Item = f32>) {
        let mut all = std::mem::take(&mut self.heights);
        all.splice(range, heights);
        *self = Self::from_heights(all);
    }

    fn extend(&mut self, heights: &[f32]) {
        // Adding a row walks the tree; laying it all out again is one pass.
        // Whichever is less work.
        if heights.len() >= self.len() {
            let mut all = std::mem::take(&mut self.heights);
            all.extend_from_slice(heights);
            *self = Self::from_heights(all);
        } else {
            for &height in heights {
                self.push(height);
            }
        }
    }

    fn prefix(&self, index: usize) -> f64 {
        self.sums.prefix(index)
    }

    fn find(&self, y: f64) -> usize {
        self.sums.count_within(y)
    }
}

/// Two [`Fenwick`] trees back to back: `back` holds row 0 on, in order, and
/// `front` the rows put above row 0, nearest it first. Adding rows at either
/// end is O(log n) a row, so a page put in front costs what the page holds,
/// not what the list holds; reading an offset or finding a row stays O(log n).
/// Only a change in the middle that adds or takes away rows lays everything
/// out again.
#[derive(Debug, Clone, Default)]
struct Deque {
    front: Fenwick,
    back: Fenwick,
}

impl HeightIndex for Deque {
    fn from_heights(heights: Vec<f32>) -> Self {
        Self {
            front: Fenwick::default(),
            back: Fenwick::from_heights(heights),
        }
    }

    fn len(&self) -> usize {
        self.front.len() + self.back.len()
    }

    fn get(&self, index: usize) -> f32 {
        let above = self.front.len();
        if index < above {
            self.front.get(above - 1 - index)
        } else {
            self.back.get(index - above)
        }
    }

    fn push(&mut self, height: f32) {
        self.back.push(height);
    }

    fn set(&mut self, index: usize, height: f32) {
        let above = self.front.len();
        if index < above {
            self.front.set(above - 1 - index, height);
        } else {
            self.back.set(index - above, height);
        }
    }

    fn splice(&mut self, range: Range<usize>, heights: impl Iterator<Item = f32>) {
        let mut all: Vec<f32> = (0..self.len()).map(|i| self.get(i)).collect();
        all.splice(range, heights);
        *self = Self::from_heights(all);
    }

    fn extend(&mut self, heights: &[f32]) {
        self.back.extend(heights);
    }

    fn prepend(&mut self, heights: &[f32]) {
        for &height in heights.iter().rev() {
            self.front.push(height);
        }
    }

    fn prefix(&self, index: usize) -> f64 {
        let above = self.front.len();
        if index <= above {
            // Rows `0..index` are the front's last `index`.
            self.front.total() - self.front.prefix(above - index)
        } else {
            self.front.total() + self.back.prefix(index - above)
        }
    }

    fn find(&self, y: f64) -> usize {
        let above = self.front.len();
        let front = self.front.total();
        if y >= front {
            return above + self.back.find(y - front);
        }
        // Row `j` above row 0 ends at `front - front.prefix(above - 1 - j)`,
        // past `y` while that prefix is under `front - y`: the first such row
        // is the one whose prefix is the last under it.
        let under = self.front.sums.count_below(front - y);
        above - 1 - under
    }
}

#[cfg(test)]
#[path = "row_heights_tests.rs"]
mod tests;
