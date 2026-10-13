//! Virtualized list / grid widget.
//!
//! [`List`] renders a flat collection of `count` items into a scroll viewport,
//! packing them as a single column (a list) or `N` columns (a grid via
//! [`List::columns`]). Only the items intersecting the viewport are drawn, so a
//! 10k-item collection costs the same as a screenful — the widget culls to the
//! visible index range exactly like [`Table`](super::Table), reusing
//! [`ScrollView`](super::ScrollView) for clipping / wheel / scrollbar drag.
//!
//! The widget owns no data: the caller passes the item `count` and a per-item
//! **content** closure `FnMut(&mut DrawList, Rect, ListItem)`. The list itself
//! draws the row background (selection / hover / zebra) and handles all
//! interaction (click-to-select, keyboard navigation); the closure only fills
//! the item rect with its content.
//!
//! Rows are painted in the design's list recipe: a faint zebra on odd rows
//! (opt-in), a hover wash, and a selection that depends on focus — an accent
//! fill while the list has the keyboard, a neutral held fill otherwise, so
//! with two lists stacked the one taking the arrow keys is obvious. Rows the
//! caller marks [`disabled`](List::disabled) never hover, select or activate,
//! and the arrow keys step over them.
//!
//! The scroll position is kept as the row at the top of the view and how far
//! into it the view starts, not as a pixel offset: rows changing above the
//! view never move what is on screen, and nothing is corrected per frame.
//! [`ListState::follow_end`] keeps the last row in view as rows arrive, until
//! the reader scrolls up ([`List::stick_to_end`] picks it up again when they
//! scroll back down).
//!
//! Persistent state — scroll position, the selected set, and the keyboard
//! cursor — lives in a caller-owned [`ListState`], matching the immediate-mode,
//! caller-owns-state style of the rest of the crate
//! ([`ScrollState`](super::ScrollState), [`TreeState`](super::TreeState)).
//!
//! Like [`Table`](super::Table) and [`ScrollView`](super::ScrollView), this is a
//! *scrollable* widget and so takes a raw `&mut InputState` (the `ScrollView`
//! consumes the wheel) rather than a [`DrawContext`](super::DrawContext), whose
//! `input` is shared-only.
//!
//! ```ignore
//! let mut state = ListState::new();
//! let items = ["Sword", "Shield", "Potion"];
//! let out = List::new()
//!     .with_item_height(22.0)
//!     .selection(SelectionMode::Multi)
//!     .focused(panel_has_focus)
//!     .draw(rect, items.len(), &mut state, list, &style, &mut input,
//!         |list, cell, it| {
//!             let color = if it.selected { [255, 255, 255] } else { [200, 210, 230] };
//!             list.text(TextBlock::new(items[it.index], cell.x + 6.0, cell.y + 3.0)
//!                 .with_color(color[0], color[1], color[2]));
//!         });
//! if let Some(i) = out.activated { launch(items[i]); }
//! ```

use std::collections::BTreeSet;

use crate::layout::Rect;
use crate::{InputState, StyleKey, StyleResolver};

use super::DrawList;
use super::row_heights::{RowHeights, RowId};
use super::scroll_view::{ScrollState, ScrollView};

/// How a [`List`] responds to clicks / keyboard selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// Items are never selected; the list only reports `clicked` / `hovered`.
    None,
    /// At most one selected item (default). Any click replaces the selection.
    #[default]
    Single,
    /// Multiple items: plain click replaces, `Ctrl`+click toggles, `Shift`+click
    /// selects the inclusive range from the anchor.
    Multi,
}

/// This frame's rising-edge navigation keys. Held keys stay `true` across frames
/// at the `InputState` level, so we derive a one-shot edge per press (mirrors the
/// `Tree` widget's nav handling).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NavKeys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    home: bool,
    end: bool,
    activate: bool,
}

impl NavKeys {
    fn raw(input: &InputState) -> Self {
        Self {
            up: input.nav.up,
            down: input.nav.down,
            left: input.nav.left,
            right: input.nav.right,
            // Home/End have no device-agnostic nav intent — they stay keyboard-only.
            home: input.key_home,
            end: input.key_end,
            activate: input.nav.confirm,
        }
    }
}

/// Caller-owned, persistent list state: scroll offset + selection + keyboard
/// cursor. Build once and thread the same `&mut ListState` every frame.
#[derive(Debug, Clone, Default)]
pub struct ListState {
    /// The pixel offset the embedded [`ScrollView`] glides and drags, worked
    /// out from `top` and `target` at the start of each draw.
    scroll: ScrollState,
    /// Where the view starts.
    top: RowAnchor,
    /// Where a glide is heading; `top` when there is none.
    target: RowAnchor,
    /// Keep the last row in view as rows arrive.
    following: bool,
    /// An item to scroll into view at the next draw.
    reveal: Option<usize>,
    /// Pixels to move the view by at the next draw.
    nudge: f32,
    /// How far into their rows `top` and `target` were, as shares of the
    /// rows' heights, taken by [`keep_share`](Self::keep_share) for the next
    /// draw.
    shares: Option<(Share, Share)>,
    /// The selected item indices (ordered, so the caller can iterate cheaply).
    selected: BTreeSet<usize>,
    /// Origin of a `Shift`-range selection (the last plainly-clicked item).
    anchor: Option<usize>,
    /// The keyboard "current" item — the one arrow keys move and `Enter`
    /// activates. Distinct from `selected`: plain arrow nav replaces the
    /// selection with the cursor, `Shift`+arrow extends from `anchor`.
    cursor: Option<usize>,
    /// This frame's rising-edge nav keys.
    keys: NavKeys,
    /// Last frame's raw key state, for edge detection.
    prev_keys: NavKeys,
}

/// A place in a list: row `row`, `into` pixels below its top. A row is a
/// line of the list: one item, or in a grid one line of items. In a measured
/// list the row's [`RowId`] is kept too, and the row found by it again at the
/// next draw, so rows put in front of it don't move the view.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RowAnchor {
    row: usize,
    into: f32,
    id: Option<RowId>,
}

impl Default for RowAnchor {
    /// The very top, top padding included: as far above row 0 as the view
    /// can go, which the first draw places at the top of the content.
    fn default() -> Self {
        Self {
            row: 0,
            into: f32::NEG_INFINITY,
            id: None,
        }
    }
}

/// How far into a row an anchor was, as a share of the row's height, and
/// the row's id; none at the very top of a row or above it.
type Share = Option<(RowId, f32)>;

/// Where a list's rows are, for turning a [`RowAnchor`] into pixels and back.
/// Pixels count from the top of the content, padding included; rows count
/// from below the top padding.
#[derive(Clone, Copy)]
struct Rows<'h> {
    kind: RowKind<'h>,
    /// Space kept free above the first row.
    pad_top: f32,
}

#[derive(Clone, Copy)]
enum RowKind<'h> {
    /// Rows `height` tall, `pitch` apart (the height and the gap after it).
    Fixed {
        height: f32,
        pitch: f32,
        count: usize,
    },
    Measured(&'h RowHeights),
}

impl Rows<'_> {
    fn count(&self) -> usize {
        match self.kind {
            RowKind::Fixed { count, .. } => count,
            RowKind::Measured(heights) => heights.len(),
        }
    }

    /// Where row `row` starts, below the top padding; the end of the rows
    /// past the last.
    fn offset_of(&self, row: usize) -> f64 {
        match self.kind {
            RowKind::Fixed { pitch, count, .. } => row.min(count) as f64 * f64::from(pitch),
            RowKind::Measured(heights) => heights.offset_of(row.min(heights.len())),
        }
    }

    fn height_of(&self, row: usize) -> f32 {
        match self.kind {
            RowKind::Fixed { height, .. } => height,
            RowKind::Measured(heights) => heights.height_of(row),
        }
    }

    /// The rows' own extent, padding not included.
    fn extent(&self) -> f32 {
        match self.kind {
            RowKind::Fixed {
                height,
                pitch,
                count,
            } => {
                if count == 0 {
                    0.0
                } else {
                    (count - 1) as f32 * pitch + height
                }
            }
            RowKind::Measured(heights) => heights.total() as f32,
        }
    }

    /// The row covering `y` (counted below the top padding), or the count
    /// past the last.
    fn row_at(&self, y: f64) -> usize {
        match self.kind {
            RowKind::Fixed { pitch, count, .. } => {
                ((y / f64::from(pitch)).floor().max(0.0) as usize).min(count)
            }
            RowKind::Measured(heights) => heights.row_at(y).unwrap_or(heights.len()),
        }
    }

    /// The place at pixel `y`: the row covering it and how far into it `y`
    /// is. In the top padding that is row 0, a negative distance into it.
    fn locate(&self, y: f32) -> RowAnchor {
        let y = f64::from(y.max(0.0)) - f64::from(self.pad_top);
        let row = if y < 0.0 { 0 } else { self.row_at(y) };
        RowAnchor {
            row,
            into: (y - self.offset_of(row)) as f32,
            id: self.id_of(row),
        }
    }

    fn id_of(&self, row: usize) -> Option<RowId> {
        match self.kind {
            RowKind::Measured(heights) if row < heights.len() => Some(heights.id_of(row)),
            _ => None,
        }
    }

    /// Find `at`'s row again by its id, wherever rows put in front moved it,
    /// and note the id of a row given by index.
    fn resolve(&self, at: &mut RowAnchor) {
        if let RowKind::Measured(heights) = self.kind
            && let Some(row) = at.id.and_then(|id| heights.index_of(id))
        {
            at.row = row;
        }
        at.id = self.id_of(at.row);
    }

    fn pixels(&self, at: RowAnchor) -> f32 {
        (f64::from(self.pad_top) + self.offset_of(at.row) + f64::from(at.into)) as f32
    }

    /// Where row `row` starts, in pixels.
    fn top_of(&self, row: usize) -> f32 {
        self.pixels(RowAnchor {
            row,
            into: 0.0,
            id: None,
        })
    }
}

/// How close to the end the reader has to scroll for a list that sticks to
/// its end to follow it again. Pixels on screen are f32: a few million
/// pixels down a long list they come a whole pixel apart, so the end can
/// only be reached to within one.
const END_SLACK: f32 = 2.0;

// ---- selection primitives (single source of truth) ----------------------
//
// These operate on the destructured field borrows so the draw loop (which can't
// hold `&mut ListState` while `ScrollView` borrows `state.scroll`) and the
// public methods share identical semantics.

fn sel_one(
    sel: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: &mut Option<usize>,
    i: usize,
) {
    sel.clear();
    sel.insert(i);
    *anchor = Some(i);
    *cursor = Some(i);
}

fn sel_toggle(
    sel: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: &mut Option<usize>,
    i: usize,
) {
    if !sel.remove(&i) {
        sel.insert(i);
    }
    *anchor = Some(i);
    *cursor = Some(i);
}

fn sel_range(
    sel: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: &mut Option<usize>,
    a: usize,
    b: usize,
) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    sel.clear();
    for k in lo..=hi {
        sel.insert(k);
    }
    *anchor = Some(a);
    *cursor = Some(b);
}

impl ListState {
    /// A fresh state: nothing selected, scrolled to the top.
    pub fn new() -> Self {
        Self::default()
    }

    /// How far down the content the view started at the last draw, in
    /// pixels.
    pub fn offset(&self) -> f32 {
        self.scroll.offset[1]
    }

    /// The row at the top of the view and how many of its pixels are above
    /// the view's top edge; negative in the top padding. Before the first
    /// draw it is row 0 at minus infinity: the very top, padding included.
    pub fn top_row(&self) -> (usize, f32) {
        (self.top.row, self.top.into)
    }

    /// Put row `row` at the top of the view, `into` pixels into it, at the
    /// next draw. Stops following the end.
    pub fn scroll_to_row(&mut self, row: usize, into: f32) {
        self.top = RowAnchor {
            row,
            into,
            id: None,
        };
        self.target = self.top;
        self.following = false;
        self.nudge = 0.0;
        self.shares = None;
    }

    /// Glide to row `row`, `into` pixels into it, over the next draws. Stops
    /// following the end.
    pub fn glide_to_row(&mut self, row: usize, into: f32) {
        self.target = RowAnchor {
            row,
            into,
            id: None,
        };
        self.following = false;
        self.nudge = 0.0;
        self.shares = None;
    }

    /// Show the last row, and keep it in view as rows arrive, until the
    /// reader scrolls up. Jumps there, ending any glide.
    pub fn follow_end(&mut self) {
        self.following = true;
        self.top = self.target;
        self.nudge = 0.0;
    }

    /// `heights` are about to change under the view: rows estimated again
    /// for another width or other fonts. The next draw keeps the reader as
    /// far into the row at the top as the same share of its new height, as
    /// it does when it sees the width change itself. Call it before
    /// changing them.
    pub fn keep_share(&mut self, heights: &RowHeights) {
        self.shares = Some(self.take_shares(heights));
    }

    /// The shares of their rows `top` and `target` are into, with the rows
    /// found by id first, in case rows went in front since the last draw.
    fn take_shares(&mut self, heights: &RowHeights) -> (Share, Share) {
        let share = |at: &mut RowAnchor| {
            if let Some(row) = at.id.and_then(|id| heights.index_of(id)) {
                at.row = row;
            }
            let id = (at.row < heights.len()).then(|| heights.id_of(at.row))?;
            at.id = Some(id);
            let height = heights.height_of(at.row);
            (height > 0.0 && at.into > 0.0).then(|| (id, at.into / height))
        };
        (share(&mut self.top), share(&mut self.target))
    }

    /// Scroll item `index` into view at the next draw, gliding no further
    /// than it takes.
    pub fn reveal_item(&mut self, index: usize) {
        self.reveal = Some(index);
    }

    /// Move the view `dy` pixels down (up when negative) at the next draw,
    /// straight there: for a drag past the edge that scrolls with the clock.
    /// Following the end afterwards as if the reader had scrolled.
    pub fn scroll_by(&mut self, dy: f32) {
        self.nudge += dy;
    }

    /// Whether the view keeps to the end as rows arrive.
    pub fn is_following_end(&self) -> bool {
        self.following
    }

    /// Whether a glide is still under way, so the next frame is wanted.
    pub fn is_gliding(&self) -> bool {
        self.top != self.target || self.scroll.is_gliding()
    }

    /// The scroll view's state, for asking when the next frame of a glide is
    /// due.
    pub(crate) fn scroll_state(&self) -> &ScrollState {
        &self.scroll
    }

    /// Aim the glide so row `row` is in a view `view` tall, and the padding
    /// with it for the first and last rows.
    fn bring_into_view(&mut self, rows: &Rows, row: usize, view: f32) {
        let top = if row == 0 { 0.0 } else { rows.top_of(row) };
        let bottom = if row + 1 >= rows.count() {
            self.scroll.content_size[1]
        } else {
            rows.top_of(row) + rows.height_of(row)
        };
        self.scroll.scroll_range_into_view(1, top, bottom, view);
        let max = self.scroll.max_offset(1, view);
        self.scroll.target[1] = self.scroll.target[1].clamp(0.0, max);
    }

    /// Set the pixel offset and target from the anchors, for this draw's
    /// rows and a view `view` tall: the end when following, and inside the
    /// content either way (an anchor the content no longer reaches is moved to
    /// where the view ends up, so the content growing back doesn't throw the
    /// view there). Returns the offset and target set.
    fn place(&mut self, rows: &Rows, view: f32) -> (f32, f32) {
        let max = self.scroll.max_offset(1, view);
        rows.resolve(&mut self.top);
        rows.resolve(&mut self.target);
        if self.following {
            // A glide under way (the wheel's last notch reached the end)
            // finishes on its own; a view already there moves with the end.
            let end = rows.locate(max);
            if self.top == self.target {
                self.top = end;
            }
            self.target = end;
        }
        let place = |anchor: &mut RowAnchor| {
            let wanted = rows.pixels(*anchor);
            let placed = wanted.clamp(0.0, max);
            if placed != wanted {
                *anchor = rows.locate(placed);
            }
            placed
        };
        let offset = place(&mut self.top);
        let target = place(&mut self.target);
        // Two anchors can name one pixel (the end of a row and the start of
        // the next); there is no glide left then.
        if offset == target {
            self.target = self.top;
        }
        self.scroll.offset[1] = offset;
        self.scroll.target[1] = target;
        (offset, target)
    }

    /// Take the anchors back from the pixel offset and target after the
    /// scroll view moved them (the wheel, the bar, a glide), for a view
    /// `view` tall: following the end again if the view reached it and
    /// `stick` says to.
    fn take_pixels(&mut self, rows: &Rows, view: f32, stick: bool) {
        self.top = rows.locate(self.scroll.offset[1]);
        self.target = rows.locate(self.scroll.target[1]);
        let at_end = self.scroll.target[1] >= self.scroll.max_offset(1, view) - END_SLACK;
        self.following = at_end && (self.following || stick);
    }

    /// True when item `i` is currently selected.
    pub fn is_selected(&self, i: usize) -> bool {
        self.selected.contains(&i)
    }

    /// Iterate the selected indices in ascending order.
    pub fn selected(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected.iter().copied()
    }

    /// How many items are selected.
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// The keyboard cursor (current) item, if any.
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// Move the keyboard cursor without changing the selection.
    pub fn set_cursor(&mut self, i: usize) {
        self.cursor = Some(i);
    }

    /// Select exactly `i`, replacing any prior selection.
    pub fn select_one(&mut self, i: usize) {
        sel_one(&mut self.selected, &mut self.anchor, &mut self.cursor, i);
    }

    /// Toggle `i` in the selection (multi-select).
    pub fn toggle(&mut self, i: usize) {
        sel_toggle(&mut self.selected, &mut self.anchor, &mut self.cursor, i);
    }

    /// Select the inclusive range `a..=b` (order-independent), replacing the
    /// prior selection. The anchor becomes `a`, the cursor `b`.
    pub fn select_range(&mut self, a: usize, b: usize) {
        sel_range(&mut self.selected, &mut self.anchor, &mut self.cursor, a, b);
    }

    /// Clear the selection (cursor and anchor are preserved).
    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Select every item in `0..count`.
    pub fn select_all(&mut self, count: usize) {
        self.selected.clear();
        self.selected.extend(0..count);
    }

    /// The only selected item, when exactly one is selected.
    pub fn single_selected(&self) -> Option<usize> {
        match self.selected.len() {
            1 => self.selected.first().copied(),
            _ => None,
        }
    }

    /// Make the selection exactly `selection`, for a caller that owns which
    /// item is selected (a selection kept by id rather than index). `Some(i)`
    /// selects `i` and puts the cursor there; `None` clears the selection and
    /// the cursor, so the next arrow key starts from the top. A no-op when the
    /// selection already matches.
    pub fn sync_selection(&mut self, selection: Option<usize>) {
        if self.single_selected() == selection && (selection.is_some() || self.selected.is_empty())
        {
            return;
        }
        match selection {
            Some(i) => self.select_one(i),
            None => {
                self.clear_selection();
                self.cursor = None;
            }
        }
    }
}

/// Per-item info handed to the content closure so it can style by state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListItem {
    /// The item's index in `0..count`.
    pub index: usize,
    /// True if this item is in the selected set.
    pub selected: bool,
    /// True if the pointer is over this item this frame.
    pub hovered: bool,
    /// True if this item is the keyboard cursor.
    pub cursor: bool,
    /// True if the list has keyboard focus (a selected row then wears the
    /// accent; content should switch to on-accent inks).
    pub focused: bool,
    /// True if the caller marked this item [`disabled`](List::disabled).
    pub disabled: bool,
}

/// Interaction results from one [`List::draw`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListOutput {
    /// The item clicked this frame, if any.
    pub clicked: Option<usize>,
    /// The item activated this frame: double-clicked, or the cursor when the
    /// focused list gets `Enter` / `Space`.
    pub activated: Option<usize>,
    /// The item under the pointer this frame, if any.
    pub hovered: Option<usize>,
    /// True if the pointer is over the list's content area (for scroll routing).
    pub mouse_over_content: bool,
}

/// Top and bottom edge lines of a selected row while the list has focus
/// (`--row-select-inset`).
const SELECT_EDGES_FOCUSED: [[f32; 4]; 2] = [[1.0, 1.0, 1.0, 0.28], [0.0, 0.0, 0.0, 0.25]];
/// The same lines on the held selection of an unfocused list.
const SELECT_EDGES_HELD: [[f32; 4]; 2] = [[1.0, 1.0, 1.0, 0.08], [0.0, 0.0, 0.0, 0.3]];

/// Paint a selected row: the accent bar while its list or tree has focus,
/// the held wash otherwise, each with its top and bottom edge lines. Shared
/// by [`List`] and [`TreeNode`](super::TreeNode).
pub(crate) fn paint_selected_row(
    list: &mut DrawList,
    style: &StyleResolver,
    row: Rect,
    focused: bool,
) {
    let (fill, [top, bottom]) = if focused {
        (style.color(StyleKey::Accent), SELECT_EDGES_FOCUSED)
    } else {
        (style.color(StyleKey::RowHeld), SELECT_EDGES_HELD)
    };
    list.quad(row.x, row.y, row.width, row.height, fill);
    if row.height > 2.0 {
        list.quad(row.x, row.y, row.width, 1.0, top);
        list.quad(row.x, row.y + row.height - 1.0, row.width, 1.0, bottom);
    }
}

/// A virtualized list / grid. Transient — rebuild it each frame; all persistent
/// state lives in the caller-owned [`ListState`].
pub struct List<'a> {
    item_height: Option<f32>,
    columns: usize,
    col_gap: f32,
    row_gap: f32,
    selection: SelectionMode,
    focused: bool,
    zebra: bool,
    disabled: Option<&'a dyn Fn(usize) -> bool>,
    overlay_bar: bool,
    stick_to_end: bool,
    /// Tint the row under the pointer.
    hover: bool,
    /// How far a wheel notch scrolls, when not the scroll view's own.
    wheel_speed: Option<f32>,
    /// Space kept free above the first row and below the last, inside the
    /// scroll.
    padding: [f32; 2],
}

impl Default for List<'_> {
    fn default() -> Self {
        Self {
            item_height: None,
            columns: 1,
            col_gap: 0.0,
            row_gap: 0.0,
            selection: SelectionMode::Single,
            focused: false,
            zebra: false,
            disabled: None,
            overlay_bar: false,
            stick_to_end: false,
            hover: true,
            wheel_speed: None,
            padding: [0.0, 0.0],
        }
    }
}

impl<'a> List<'a> {
    /// Create a list with default single-selection settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fixed pixel height of every item row. Defaults to a theme-derived value.
    pub fn with_item_height(mut self, h: f32) -> Self {
        self.item_height = Some(h);
        self
    }

    /// Number of columns: `1` (default) is a list, `>1` packs a grid
    /// left-to-right, top-to-bottom.
    pub fn columns(mut self, n: usize) -> Self {
        self.columns = n.max(1);
        self
    }

    /// Gap between columns and between rows, in pixels.
    pub fn with_gap(mut self, col: f32, row: f32) -> Self {
        self.col_gap = col.max(0.0);
        self.row_gap = row.max(0.0);
        self
    }

    /// Selection behavior. Defaults to [`SelectionMode::Single`].
    pub fn selection(mut self, mode: SelectionMode) -> Self {
        self.selection = mode;
        self
    }

    /// Whether this list currently holds keyboard focus. Only a focused list
    /// reacts to arrow / Home / End / Enter keys (so background lists don't
    /// hijack navigation).
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Alternate-row background tint (only meaningful for a single-column list).
    pub fn with_zebra(mut self, zebra: bool) -> Self {
        self.zebra = zebra;
        self
    }

    /// Mark items as disabled: `is_disabled(i)` is asked for the visible rows
    /// and while the arrow keys look for the next enabled item. Disabled items
    /// never hover, select or activate, and the arrow keys, `Home` and `End`
    /// skip them.
    pub fn disabled(mut self, is_disabled: &'a dyn Fn(usize) -> bool) -> Self {
        self.disabled = Some(is_disabled);
        self
    }

    /// Keep `top` pixels free above the first row and `bottom` below the
    /// last, inside the scroll: they scroll with the rows.
    pub fn with_padding(mut self, top: f32, bottom: f32) -> Self {
        self.padding = [top.max(0.0), bottom.max(0.0)];
        self
    }

    /// Follow the end again whenever the reader scrolls back down to it (see
    /// [`ListState::follow_end`]), the way a chat or a log does.
    pub fn stick_to_end(mut self, stick: bool) -> Self {
        self.stick_to_end = stick;
        self
    }

    /// Whether the row under the pointer is tinted (the default). A list
    /// whose rows show their own hover turns it off.
    pub fn with_hover(mut self, hover: bool) -> Self {
        self.hover = hover;
        self
    }

    /// How many pixels a wheel notch scrolls (see
    /// [`ScrollView::with_wheel_speed`]).
    pub fn with_wheel_speed(mut self, speed: f32) -> Self {
        self.wheel_speed = Some(speed);
        self
    }

    /// The width rows are measured and drawn at in `rect`: what is left
    /// beside a docked scrollbar, in whole pixels (see
    /// [`draw_measured`](Self::draw_measured)).
    pub fn measure_width(&self, rect: Rect) -> f32 {
        let reserve = if self.overlay_bar {
            0.0
        } else {
            ScrollView::new(rect).vertical_only().reserve()
        };
        (rect.width - reserve).floor().max(1.0)
    }

    /// Float the scrollbar over the rows instead of docking it beside them
    /// (see [`ScrollView::overlay`]): the rows keep the full width, and the
    /// thin thumb takes the pointer from the row under it.
    pub fn overlay_scrollbar(mut self) -> Self {
        self.overlay_bar = true;
        self
    }

    fn is_disabled(&self, i: usize) -> bool {
        self.disabled.is_some_and(|f| f(i))
    }

    /// The first enabled item from `from` stepping by `step` (inclusive of
    /// `from`), or `None` when the steps leave `0..count` first.
    fn enabled_from(&self, from: usize, step: isize, count: usize) -> Option<usize> {
        let mut i = from;
        loop {
            if i >= count {
                return None;
            }
            if !self.is_disabled(i) {
                return Some(i);
            }
            i = i.checked_add_signed(step)?;
        }
    }

    /// Draw the list and return interaction results.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<F>(
        &self,
        rect: Rect,
        count: usize,
        state: &mut ListState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        item: F,
    ) -> ListOutput
    where
        F: FnMut(&mut DrawList, Rect, ListItem),
    {
        self.draw_rows(rect, count, Sizing::Fixed, state, list, style, input, item)
    }

    /// Draw a list whose rows are not all one height: one row for each in
    /// `heights`, one column. Rows in view (and a couple either side) that
    /// are still estimated are measured with `measure(index, width, list)`,
    /// which returns the row's height laid out `width` wide; rows out of view
    /// keep their estimates until the app measures them (see
    /// [`RowHeights::apply_measured`]). Rows are measured, and drawn, at the
    /// width left beside a docked scrollbar whether or not it shows, so the
    /// bar appearing doesn't change any height; the width is in whole
    /// pixels, so a layout wobbling by a fraction doesn't re-measure every
    /// row. A row of no height is not drawn. `measure` may be called inside
    /// the scroll view's clip and offset, so it should only measure, not
    /// draw.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_measured<M, F>(
        &self,
        rect: Rect,
        heights: &mut RowHeights,
        mut measure: M,
        state: &mut ListState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        item: F,
    ) -> ListOutput
    where
        M: FnMut(usize, f32, &mut DrawList) -> f32,
        F: FnMut(&mut DrawList, Rect, ListItem),
    {
        debug_assert!(self.columns <= 1, "a measured list has one column");
        let count = heights.len();
        let sizing = Sizing::Measured {
            heights,
            measure: &mut measure,
        };
        self.draw_rows(rect, count, sizing, state, list, style, input, item)
    }

    /// The rows in view at `offset` in a view `view` tall, and `OVERSCAN`
    /// either side, that are still estimated; none for fixed rows.
    fn unmeasured_in_view(rows: &Rows, offset: f32, view: f32) -> Vec<usize> {
        const OVERSCAN: usize = 2;
        let RowKind::Measured(heights) = rows.kind else {
            return Vec::new();
        };
        let first = rows.locate(offset).row.saturating_sub(OVERSCAN);
        let last = rows.locate(offset + view).row.saturating_add(OVERSCAN + 1);
        (first..last.min(heights.len()))
            .filter(|&row| !heights.is_measured(row))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_rows<F>(
        &self,
        rect: Rect,
        count: usize,
        mut sizing: Sizing,
        state: &mut ListState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        mut item: F,
    ) -> ListOutput
    where
        F: FnMut(&mut DrawList, Rect, ListItem),
    {
        list.push_debug_scope_rect("List", rect);
        let item_h = self
            .item_height
            .unwrap_or(style.scalar(StyleKey::FontSize) + 10.0)
            .max(1.0);
        let cols = match sizing {
            Sizing::Fixed => self.columns.max(1),
            Sizing::Measured { .. } => 1,
        };
        let [pad_top, pad_bottom] = self.padding;
        let view_h = rect.height;
        // `begin` eases the drawn offset toward the target and clamps it, so
        // culling and hit-testing must read the offset after it, not before.
        let mut view = ScrollView::new(rect).vertical_only();
        if self.overlay_bar {
            view = view.overlay();
        }
        if let Some(speed) = self.wheel_speed {
            view = view.with_wheel_speed(speed);
        }
        let measure_w = self.measure_width(rect);

        // A new width keeps the reader's place as the same share of the row
        // at the top: its pixels into it no longer mean the same text.
        // Shares the caller took (see `keep_share`) were taken before it
        // changed the heights, so they win over any taken here.
        let mut shares = state.shares.take();
        if let Sizing::Measured { heights, .. } = &mut sizing {
            // As `set_width` sees a change: not the first width.
            if shares.is_none() && heights.width() != 0.0 && heights.width() != measure_w {
                shares = Some(state.take_shares(heights));
            }
            heights.set_width(measure_w);
        }

        // --- Geometry: measure the rows in view, then place the view ------
        // Pixels from the anchors. Anything below that moves them (the
        // cursor scrolling into view, the wheel, the bar, a glide) is turned
        // back into anchors once the scroll view has had its say. Measuring
        // rows changes the offsets of the rows below them, and so what is in
        // view; a few rounds settle it.
        // Each round places from the anchors asked for: one clamped to rows
        // still estimated would otherwise stay clamped once they are measured.
        let wanted = (state.top, state.target);
        let mut placed;
        let mut rounds = 0;
        loop {
            let rows = sizing.rows(count, cols, item_h, self.row_gap, pad_top);
            state.scroll.content_size = [rect.width, pad_top + rows.extent() + pad_bottom];
            (state.top, state.target) = wanted;
            if let Some((top, target)) = shares {
                let reshare = |anchor: &mut RowAnchor, share: Option<(RowId, f32)>| {
                    if let Some((id, share)) = share
                        && anchor.id == Some(id)
                    {
                        anchor.into = share * rows.height_of(anchor.row);
                    }
                };
                reshare(&mut state.top, top);
                reshare(&mut state.target, target);
            }
            placed = state.place(&rows, view_h);
            rounds += 1;
            let missing = Self::unmeasured_in_view(&rows, placed.0, view_h);
            if missing.is_empty() || rounds > 4 {
                break;
            }
            sizing.measure(missing, measure_w, list);
        }
        let rows = sizing.rows(count, cols, item_h, self.row_gap, pad_top);
        if let Some(index) = state.reveal.take()
            && index < count
        {
            state.bring_into_view(&rows, index / cols, view_h);
        }

        // --- Keyboard nav (edge detection runs every frame; we only *act*
        //     on edges when focused so prev_keys never goes stale) ----------
        let raw = NavKeys::raw(input);
        state.keys = NavKeys {
            up: raw.up && !state.prev_keys.up,
            down: raw.down && !state.prev_keys.down,
            left: raw.left && !state.prev_keys.left,
            right: raw.right && !state.prev_keys.right,
            home: raw.home && !state.prev_keys.home,
            end: raw.end && !state.prev_keys.end,
            activate: raw.activate && !state.prev_keys.activate,
        };
        state.prev_keys = raw;

        let selectable = self.selection != SelectionMode::None;
        let mut activated = None;

        if self.focused && count > 0 {
            let k = state.keys;
            let last = count - 1;
            // A cursor past the end (the list shrank) counts from the last item.
            let base = state.cursor.map(|c| c.min(last));
            let cols_step = cols as isize;
            // The item a key asks for, stepping over disabled ones; `None` when
            // there is no enabled item that way (the cursor then stays put).
            let target = match base {
                // First key press lands on the first item rather than stepping
                // off it.
                None if k.up || k.down || k.left || k.right => self.enabled_from(0, 1, count),
                None => None,
                Some(b) if k.down => b
                    .checked_add(cols)
                    .and_then(|n| self.enabled_from(n, cols_step, count)),
                Some(b) if k.up => b
                    .checked_sub(cols)
                    .and_then(|n| self.enabled_from(n, -cols_step, count)),
                Some(b) if cols > 1 && k.right => self.enabled_from(b + 1, 1, count),
                Some(b) if cols > 1 && k.left => b
                    .checked_sub(1)
                    .and_then(|n| self.enabled_from(n, -1, count)),
                Some(_) => None,
            };
            let target = if k.home {
                self.enabled_from(0, 1, count)
            } else if k.end {
                self.enabled_from(last, -1, count)
            } else {
                target
            };

            if let Some(next) = target {
                if selectable {
                    if self.selection == SelectionMode::Multi && input.shift_pressed {
                        let a = state.anchor.unwrap_or(next);
                        sel_range(
                            &mut state.selected,
                            &mut state.anchor,
                            &mut state.cursor,
                            a,
                            next,
                        );
                    } else {
                        sel_one(
                            &mut state.selected,
                            &mut state.anchor,
                            &mut state.cursor,
                            next,
                        );
                    }
                } else {
                    state.cursor = Some(next);
                }
                // Glide the cursor into view.
                state.bring_into_view(&rows, next / cols, view_h);
            }

            if k.activate {
                activated = state.cursor.filter(|&c| c < count && !self.is_disabled(c));
            }
        }

        // --- Snapshot input before ScrollView borrows it mutably ----------
        let mouse_x = input.mouse_x;
        let mouse_y = input.mouse_y;
        let mouse_clicked = input.mouse_clicked;
        let mouse_double_clicked = input.mouse_double_clicked;
        let ctrl = input.ctrl_pressed;
        let shift = input.shift_pressed;
        let mouse_over_content = rect.contains(mouse_x, mouse_y) && !input.mouse_consumed;

        // A move by pixels asked for since the last draw: straight there.
        if state.nudge != 0.0 {
            let max = state.scroll.max_offset(1, view_h);
            let to = (state.scroll.offset[1] + std::mem::take(&mut state.nudge)).clamp(0.0, max);
            state.scroll.offset[1] = to;
            state.scroll.target[1] = to;
        }
        let begun = view.begin(&mut state.scroll, list, input);
        if (state.scroll.offset[1], state.scroll.target[1]) != placed {
            state.take_pixels(&rows, view_h, self.stick_to_end);
            // The view moved onto rows not measured yet. Measure them, and
            // place the view on its row again: the content is already
            // translated by the offset `begin` read, so move it by as much
            // again for this frame to show it in place.
            let missing = Self::unmeasured_in_view(&rows, state.scroll.offset[1], view_h);
            if !missing.is_empty() {
                sizing.measure(missing, measure_w, list);
                let rows = sizing.rows(count, cols, item_h, self.row_gap, pad_top);
                state.scroll.content_size = [rect.width, pad_top + rows.extent() + pad_bottom];
                let before = state.scroll.offset[1];
                state.place(&rows, view_h);
                list.translate(0.0, before - state.scroll.offset[1]);
            }
        }
        let rows = sizing.rows(count, cols, item_h, self.row_gap, pad_top);
        let scroll_y = state.scroll.offset[1];

        // Disjoint field borrows so the content closure can mutate selection.
        let ListState {
            selected,
            anchor,
            cursor,
            ..
        } = state;
        let cursor_now = *cursor;

        let mut clicked = None;
        let mut hovered = None;
        let selection_mode = self.selection;
        let zebra = self.zebra;
        let col_gap = self.col_gap;
        let focused = self.focused;

        // An overlay thumb under the pointer has taken it from the rows.
        let rows_hot = mouse_over_content && !input.mouse_consumed;
        {
            let vp = begun.inner;
            let cell_w = match sizing {
                Sizing::Measured { .. } => measure_w,
                Sizing::Fixed if cols == 1 => vp.width,
                Sizing::Fixed => ((vp.width - (cols - 1) as f32 * col_gap) / cols as f32).max(1.0),
            };
            let bottom = scroll_y + vp.height;
            let mut row = rows.locate(scroll_y).row;
            while row < rows.count() && rows.top_of(row) < bottom {
                let world_y = vp.y + rows.top_of(row);
                let screen_y = world_y - scroll_y;
                let row_h = rows.height_of(row);
                for col in 0..cols {
                    let idx = row * cols + col;
                    if idx >= count || row_h <= 0.0 {
                        break;
                    }
                    let cell_x = vp.x + col as f32 * (cell_w + col_gap);
                    let cell = Rect::new(cell_x, world_y, cell_w, row_h);

                    let disabled = self.is_disabled(idx);
                    // Hit-test in screen space; gaps are dead zones.
                    let over_cell = !disabled
                        && rows_hot
                        && mouse_x >= cell_x
                        && mouse_x < cell_x + cell_w
                        && mouse_y >= screen_y
                        && mouse_y < screen_y + row_h
                        && mouse_y >= vp.y
                        && mouse_y < vp.y + vp.height;

                    if over_cell {
                        hovered = Some(idx);
                        if mouse_clicked {
                            clicked = Some(idx);
                            if selectable {
                                match selection_mode {
                                    SelectionMode::Multi if ctrl => {
                                        sel_toggle(selected, anchor, cursor, idx)
                                    }
                                    SelectionMode::Multi if shift => {
                                        let a = anchor.unwrap_or(idx);
                                        sel_range(selected, anchor, cursor, a, idx)
                                    }
                                    _ => sel_one(selected, anchor, cursor, idx),
                                }
                            } else {
                                *cursor = Some(idx);
                            }
                            if mouse_double_clicked {
                                activated = Some(idx);
                            }
                        }
                    }

                    let is_selected = !disabled && selected.contains(&idx);
                    if is_selected {
                        paint_selected_row(list, style, cell, focused);
                    } else {
                        let bg = if over_cell && self.hover {
                            style.color(StyleKey::RowHover)
                        } else if zebra && cols == 1 && idx % 2 == 1 {
                            style.color(StyleKey::RowZebra)
                        } else {
                            [0.0, 0.0, 0.0, 0.0]
                        };
                        if bg[3] > 0.0 {
                            list.quad(cell.x, cell.y, cell.width, cell.height, bg);
                        }
                    }

                    item(
                        list,
                        cell,
                        ListItem {
                            index: idx,
                            selected: is_selected,
                            hovered: over_cell,
                            cursor: cursor_now == Some(idx),
                            focused,
                            disabled,
                        },
                    );
                }
                row += 1;
            }
        }
        // The bar's step buttons move the target in `end`.
        let before_end = (state.scroll.offset[1], state.scroll.target[1]);
        view.end(&mut state.scroll, list, style, input, begun);
        if (state.scroll.offset[1], state.scroll.target[1]) != before_end {
            state.take_pixels(&rows, view_h, self.stick_to_end);
        }

        list.pop_debug_scope();
        ListOutput {
            clicked,
            activated,
            hovered,
            mouse_over_content,
        }
    }
}

/// How a list's rows get their heights for one draw.
enum Sizing<'s> {
    /// All the item height (see [`List::with_item_height`]).
    Fixed,
    /// From `heights`, measuring the rows in view with `measure`.
    Measured {
        heights: &'s mut RowHeights,
        measure: &'s mut dyn FnMut(usize, f32, &mut DrawList) -> f32,
    },
}

impl Sizing<'_> {
    /// The rows of `count` items in `cols` columns.
    fn rows(&self, count: usize, cols: usize, item_h: f32, row_gap: f32, pad_top: f32) -> Rows<'_> {
        let kind = match self {
            Sizing::Fixed => RowKind::Fixed {
                height: item_h,
                pitch: item_h + row_gap,
                count: count.div_ceil(cols),
            },
            Sizing::Measured { heights, .. } => RowKind::Measured(heights),
        };
        Rows { kind, pad_top }
    }

    /// Measure `rows` with the callback and record their heights.
    fn measure(&mut self, rows: Vec<usize>, width: f32, list: &mut DrawList) {
        if let Sizing::Measured { heights, measure } = self {
            for row in rows {
                let height = measure(row, width, list);
                heights.set_measured(row, height);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..InputState::default()
        }
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    /// Drive one frame; return the output.
    fn frame(
        list_w: &List,
        rect: Rect,
        count: usize,
        st: &mut ListState,
        input: &mut InputState,
    ) -> ListOutput {
        // Tests set raw key edges; map them into `nav` intents (which the list
        // reads) the same way a real frame's `NavMap` would.
        crate::map_keyboard(input);
        let mut dl = DrawList::new();
        let th = theme();
        list_w.draw(
            rect,
            count,
            st,
            &mut dl,
            &StyleResolver::new(&th),
            input,
            |_, _, _| {},
        )
    }

    /// A frame of a 100 px tall list of 20 px rows, with `wheel` notches.
    fn wheel(w: &List, st: &mut ListState, count: usize, notches: f32) {
        let mut input = InputState {
            mouse_x: 10.0,
            mouse_y: 10.0,
            scroll_delta: notches,
            ..InputState::default()
        };
        frame(w, Rect::new(0.0, 0.0, 100.0, 100.0), count, st, &mut input);
    }

    fn still(w: &List, st: &mut ListState, count: usize) {
        frame(w, Rect::new(0.0, 0.0, 100.0, 100.0), count, st, &mut idle());
    }

    #[test]
    fn the_view_is_kept_as_the_row_at_its_top_and_how_far_into_it() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        st.scroll_to_row(7, 5.0);
        still(&w, &mut st, 100);
        assert_eq!(st.offset(), 145.0);
        assert_eq!(st.top_row(), (7, 5.0));
        // The wheel moves the pixels; the row and distance follow.
        wheel(&w, &mut st, 100, -1.0);
        for _ in 0..60 {
            still(&w, &mut st, 100);
        }
        let (row, into) = st.top_row();
        assert_eq!(row as f32 * 20.0 + into, st.offset());
        assert!(st.offset() > 145.0);
    }

    #[test]
    fn a_glide_to_a_row_gets_there_over_frames() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        st.glide_to_row(30, 0.0);
        assert!(st.is_gliding());
        still(&w, &mut st, 100);
        assert!(st.offset() > 0.0 && st.offset() < 600.0, "{}", st.offset());
        for _ in 0..120 {
            still(&w, &mut st, 100);
        }
        assert_eq!(st.offset(), 600.0);
        assert_eq!(st.top_row(), (30, 0.0));
        assert!(!st.is_gliding());
    }

    #[test]
    fn rows_added_below_leave_the_view_where_it_is() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        st.scroll_to_row(10, 0.0);
        still(&w, &mut st, 50);
        still(&w, &mut st, 500);
        assert_eq!(st.top_row(), (10, 0.0));
        assert_eq!(st.offset(), 200.0);
    }

    #[test]
    fn following_the_end_keeps_the_last_row_in_view_as_rows_arrive() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        st.follow_end();
        for count in [3, 10, 11, 40] {
            still(&w, &mut st, count);
            let end = (count as f32 * 20.0 - 100.0).max(0.0);
            assert_eq!(st.offset(), end, "{count} rows");
            assert!(st.is_following_end());
        }
    }

    #[test]
    fn scrolling_up_stops_following_and_back_down_starts_again_when_it_sticks() {
        for stick in [false, true] {
            let w = List::new().with_item_height(20.0).stick_to_end(stick);
            let mut st = ListState::new();
            st.follow_end();
            still(&w, &mut st, 40);
            wheel(&w, &mut st, 40, 2.0);
            assert!(!st.is_following_end(), "stick {stick}");
            for _ in 0..60 {
                still(&w, &mut st, 40);
            }
            let up = st.offset();
            still(&w, &mut st, 41);
            assert_eq!(
                st.offset(),
                up,
                "a row arriving doesn't pull the reader down"
            );
            wheel(&w, &mut st, 41, -20.0);
            for _ in 0..60 {
                still(&w, &mut st, 41);
            }
            assert_eq!(st.is_following_end(), stick, "stick {stick}");
            still(&w, &mut st, 45);
            let end = 45.0 * 20.0 - 100.0;
            assert_eq!(st.offset() == end, stick, "stick {stick}: {}", st.offset());
        }
    }

    #[test]
    fn the_bars_step_keys_move_the_view() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        still(&w, &mut st, 40);
        // The bottom step key fills the docked gutter's last 13 px.
        frame(
            &w,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            40,
            &mut st,
            &mut click_at(94.0, 94.0),
        );
        for _ in 0..120 {
            still(&w, &mut st, 40);
        }
        assert_eq!(st.offset(), ScrollView::STEP_SCROLL);
        assert_eq!(st.top_row(), (1, ScrollView::STEP_SCROLL - 20.0));
    }

    #[test]
    fn a_glide_that_reaches_the_end_finishes_gliding_while_it_follows() {
        let w = List::new().with_item_height(20.0).stick_to_end(true);
        let mut st = ListState::new();
        still(&w, &mut st, 40);
        let max = 40.0 * 20.0 - 100.0;
        let glide = |notches| InputState {
            mouse_x: 10.0,
            mouse_y: 10.0,
            scroll_delta: notches,
            frame_dt: 0.016,
            ..InputState::default()
        };
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        frame(&w, rect, 40, &mut st, &mut glide(-100.0));
        assert!(st.is_following_end());
        let mut last = st.offset();
        assert!(last < max);
        for _ in 0..5 {
            frame(&w, rect, 40, &mut st, &mut glide(0.0));
            assert!(st.offset() > last && st.offset() < max, "{}", st.offset());
            last = st.offset();
        }
        // A row arriving mid-glide moves where it ends, not where it is.
        frame(&w, rect, 41, &mut st, &mut glide(0.0));
        assert!(st.offset() < max);
        for _ in 0..200 {
            frame(&w, rect, 41, &mut st, &mut glide(0.0));
        }
        assert_eq!(st.offset(), max + 20.0);
        assert!(st.is_following_end());
    }

    #[test]
    fn a_move_by_pixels_jumps_and_follows_the_end_when_it_gets_there() {
        let w = List::new().with_item_height(20.0).stick_to_end(true);
        let mut st = ListState::new();
        still(&w, &mut st, 40);
        st.scroll_by(45.0);
        still(&w, &mut st, 40);
        assert_eq!(st.top_row(), (2, 5.0));
        assert!(!st.is_following_end());
        st.scroll_by(10_000.0);
        still(&w, &mut st, 40);
        assert_eq!(st.offset(), 40.0 * 20.0 - 100.0);
        assert!(st.is_following_end());
    }

    #[test]
    fn a_move_to_a_row_drops_a_move_by_pixels_not_drawn_yet() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        still(&w, &mut st, 40);
        st.scroll_by(45.0);
        st.scroll_to_row(10, 0.0);
        still(&w, &mut st, 40);
        assert_eq!(st.top_row(), (10, 0.0));
        st.scroll_by(45.0);
        st.glide_to_row(5, 0.0);
        for _ in 0..120 {
            still(&w, &mut st, 40);
        }
        assert_eq!(st.top_row(), (5, 0.0));
        st.scroll_by(-45.0);
        st.follow_end();
        still(&w, &mut st, 40);
        assert_eq!(st.offset(), 40.0 * 20.0 - 100.0);
    }

    #[test]
    fn rows_need_not_tint_under_the_pointer() {
        let theme = theme();
        let hover = StyleResolver::new(&theme).color(StyleKey::RowHover);
        let quads = |w: &List| {
            let mut dl = DrawList::new();
            let mut st = ListState::new();
            w.draw(
                Rect::new(0.0, 0.0, 100.0, 100.0),
                10,
                &mut st,
                &mut dl,
                &StyleResolver::new(&theme),
                &mut InputState {
                    mouse_x: 10.0,
                    mouse_y: 10.0,
                    ..InputState::default()
                },
                |_, _, _| {},
            );
            dl.chrome_instances().filter(|c| c.bg == hover).count()
        };
        let w = List::new().with_item_height(20.0);
        assert!(quads(&w) > 0);
        assert_eq!(quads(&w.with_hover(false)), 0);
    }

    #[test]
    fn a_glide_to_the_pixel_the_view_is_at_is_no_glide() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        st.scroll_to_row(5, 0.0);
        still(&w, &mut st, 40);
        // The end of row 4 is the start of row 5.
        st.glide_to_row(4, 20.0);
        still(&w, &mut st, 40);
        assert!(!st.is_gliding());
        assert_eq!(st.offset(), 100.0);
    }

    #[test]
    fn home_and_end_show_the_padding_with_the_first_and_last_rows() {
        let w = List::new()
            .with_item_height(20.0)
            .with_padding(10.0, 30.0)
            .focused(true);
        let mut st = ListState::new();
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let key = |st: &mut ListState, end: bool| {
            let mut input = InputState {
                key_end: end,
                key_home: !end,
                ..idle()
            };
            frame(&w, rect, 40, st, &mut input);
            for _ in 0..120 {
                still(&w, st, 40);
            }
        };
        key(&mut st, true);
        assert_eq!(st.cursor(), Some(39));
        assert_eq!(st.offset(), 10.0 + 800.0 + 30.0 - 100.0);
        key(&mut st, false);
        assert_eq!(st.cursor(), Some(0));
        assert_eq!(st.offset(), 0.0);
    }

    #[test]
    fn a_list_that_shrank_under_its_view_shows_its_end_and_stays_there() {
        let w = List::new().with_item_height(20.0);
        let mut st = ListState::new();
        st.scroll_to_row(80, 0.0);
        still(&w, &mut st, 100);
        still(&w, &mut st, 20);
        assert_eq!(st.offset(), 300.0);
        assert_eq!(st.top_row(), (15, 0.0));
        // Growing again doesn't throw the view back to where it was.
        still(&w, &mut st, 100);
        assert_eq!(st.offset(), 300.0);
    }

    #[test]
    fn an_item_asked_for_glides_into_view_and_no_further() {
        // A 4-column grid of 20 px rows in a 100 px view: item 41 is on row 10.
        let w = List::new().with_item_height(20.0).columns(4);
        let mut st = ListState::new();
        st.reveal_item(41);
        for _ in 0..120 {
            still(&w, &mut st, 200);
        }
        assert_eq!(st.offset(), 11.0 * 20.0 - 100.0, "row 10 at the bottom");
        st.reveal_item(1);
        for _ in 0..120 {
            still(&w, &mut st, 200);
        }
        assert_eq!(st.offset(), 0.0);
    }

    #[test]
    fn content_size_list_and_grid() {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);

        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0),
            rect,
            10,
            &mut st,
            &mut idle(),
        );
        assert_eq!(st.scroll.content_size[1], 200.0, "10 rows * 20px");

        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0).columns(4),
            rect,
            10,
            &mut st,
            &mut idle(),
        );
        // ceil(10/4) = 3 rows.
        assert_eq!(st.scroll.content_size[1], 60.0, "3 grid rows * 20px");
    }

    #[test]
    fn content_size_includes_row_gap() {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);
        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0).with_gap(0.0, 5.0),
            rect,
            4,
            &mut st,
            &mut idle(),
        );
        // 4*20 + 3*5
        assert_eq!(st.scroll.content_size[1], 95.0);
    }

    #[test]
    fn virtualizes_only_visible_items() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut st = ListState::new();
        let mut dl = DrawList::new();
        let th = theme();
        let mut input = idle();
        let mut calls = 0usize;
        List::new().with_item_height(20.0).draw(
            rect,
            1000,
            &mut st,
            &mut dl,
            &StyleResolver::new(&th),
            &mut input,
            |_, _, _| calls += 1,
        );
        // ~5 visible rows + 1 slack; nowhere near 1000.
        assert!(calls < 20, "expected a screenful, drew {calls}");
        assert!(calls >= 5, "expected the visible rows, drew {calls}");
    }

    #[test]
    fn single_click_selects() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        let mut input = click_at(50.0, 10.0); // row 0
        let out = frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut input,
        );
        assert_eq!(out.clicked, Some(0));
        assert!(st.is_selected(0));
        assert_eq!(st.selected_count(), 1);
    }

    #[test]
    fn single_select_replaces() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut click_at(50.0, 10.0),
        );
        frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut click_at(50.0, 50.0),
        ); // row 2
        assert!(!st.is_selected(0));
        assert!(st.is_selected(2));
        assert_eq!(st.selected_count(), 1);
    }

    #[test]
    fn multi_ctrl_click_toggles() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new()
            .with_item_height(20.0)
            .selection(SelectionMode::Multi);
        let mut st = ListState::new();
        // plain click row 1
        frame(&w, rect, 5, &mut st, &mut click_at(50.0, 30.0));
        // ctrl-click row 3 -> adds
        let mut ctrl_click = click_at(50.0, 70.0);
        ctrl_click.ctrl_pressed = true;
        frame(&w, rect, 5, &mut st, &mut ctrl_click);
        assert!(st.is_selected(1) && st.is_selected(3));
        assert_eq!(st.selected_count(), 2);
        // ctrl-click row 1 again -> removes
        let mut ctrl_click = click_at(50.0, 30.0);
        ctrl_click.ctrl_pressed = true;
        frame(&w, rect, 5, &mut st, &mut ctrl_click);
        assert!(!st.is_selected(1) && st.is_selected(3));
        assert_eq!(st.selected_count(), 1);
    }

    #[test]
    fn multi_shift_click_selects_range() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new()
            .with_item_height(20.0)
            .selection(SelectionMode::Multi);
        let mut st = ListState::new();
        frame(&w, rect, 8, &mut st, &mut click_at(50.0, 30.0)); // anchor row 1
        let mut shift_click = click_at(50.0, 90.0); // row 4
        shift_click.shift_pressed = true;
        frame(&w, rect, 8, &mut st, &mut shift_click);
        let sel: Vec<usize> = st.selected().collect();
        assert_eq!(sel, vec![1, 2, 3, 4]);
    }

    #[test]
    fn keyboard_down_moves_cursor_and_selects() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();

        // First Down lands on item 0.
        let mut down = idle();
        down.key_down = true;
        frame(&w, rect, 10, &mut st, &mut down);
        assert_eq!(st.cursor(), Some(0));
        assert!(st.is_selected(0));

        // Release, then Down again -> item 1 (edge detection).
        frame(&w, rect, 10, &mut st, &mut idle());
        let mut down = idle();
        down.key_down = true;
        frame(&w, rect, 10, &mut st, &mut down);
        assert_eq!(st.cursor(), Some(1));
        assert!(st.is_selected(1) && !st.is_selected(0));
    }

    #[test]
    fn keyboard_autoscrolls_cursor_into_view() {
        let rect = Rect::new(0.0, 0.0, 100.0, 60.0); // 3 rows visible
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();
        // Step down well past the viewport (release between presses for edges).
        for _ in 0..6 {
            let mut down = idle();
            down.key_down = true;
            frame(&w, rect, 20, &mut st, &mut down);
            frame(&w, rect, 20, &mut st, &mut idle());
        }
        assert!(st.cursor().unwrap() >= 4);
        assert!(st.offset() > 0.0, "cursor scrolled into view");
    }

    #[test]
    fn keyboard_enter_activates_cursor() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();
        let mut down = idle();
        down.key_down = true;
        frame(&w, rect, 10, &mut st, &mut down); // cursor -> 0
        frame(&w, rect, 10, &mut st, &mut idle());
        let mut enter = idle();
        enter.enter_pressed = true;
        let out = frame(&w, rect, 10, &mut st, &mut enter);
        assert_eq!(out.activated, Some(0));
    }

    #[test]
    fn grid_hit_test_resolves_row_col() {
        // width 100, 4 cols, no gap -> cell_w 25.
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).columns(4);
        let mut st = ListState::new();
        // col 2 (x 60 in [50,75)), row 1 (y 30 in [20,40)) -> idx 1*4+2 = 6.
        let out = frame(&w, rect, 16, &mut st, &mut click_at(60.0, 30.0));
        assert_eq!(out.clicked, Some(6));
        assert!(st.is_selected(6));
    }

    #[test]
    fn grid_gap_is_dead_zone() {
        // width 100, 4 cols, 10px col gap -> cell_w (100-30)/4 = 17.5.
        // Cell 0 = [0,17.5); gap = [17.5,27.5). Click at x=20 hits the gap.
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new()
            .with_item_height(20.0)
            .columns(4)
            .with_gap(10.0, 0.0);
        let mut st = ListState::new();
        let out = frame(&w, rect, 16, &mut st, &mut click_at(20.0, 5.0));
        assert_eq!(out.clicked, None);
        assert_eq!(st.selected_count(), 0);
    }

    fn press(key: fn(&mut InputState)) -> InputState {
        let mut input = idle();
        key(&mut input);
        input
    }

    /// Press `key` for a frame, then release it for one (edge detection).
    fn tap(w: &List, rect: Rect, count: usize, st: &mut ListState, key: fn(&mut InputState)) {
        frame(w, rect, count, st, &mut press(key));
        frame(w, rect, count, st, &mut idle());
    }

    #[test]
    fn arrow_keys_step_over_disabled_items() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let off = |i: usize| matches!(i, 0 | 2 | 3 | 5);
        let w = List::new()
            .with_item_height(20.0)
            .focused(true)
            .disabled(&off);
        let mut st = ListState::new();
        tap(&w, rect, 6, &mut st, |i| i.key_down = true);
        assert_eq!(
            st.cursor(),
            Some(1),
            "the first key lands on the first enabled item"
        );
        tap(&w, rect, 6, &mut st, |i| i.key_down = true);
        assert_eq!(st.cursor(), Some(4), "down skips 2 and 3");
        assert!(st.is_selected(4));
        tap(&w, rect, 6, &mut st, |i| i.key_down = true);
        assert_eq!(
            st.cursor(),
            Some(4),
            "nothing enabled below: the cursor stays"
        );
        tap(&w, rect, 6, &mut st, |i| i.key_up = true);
        assert_eq!(st.cursor(), Some(1));
        tap(&w, rect, 6, &mut st, |i| i.key_up = true);
        assert_eq!(st.cursor(), Some(1), "nothing enabled above");
        tap(&w, rect, 6, &mut st, |i| i.key_end = true);
        assert_eq!(st.cursor(), Some(4), "End goes to the last enabled item");
        tap(&w, rect, 6, &mut st, |i| i.key_home = true);
        assert_eq!(st.cursor(), Some(1), "Home to the first enabled one");
    }

    #[test]
    fn disabled_items_do_not_hover_select_or_activate() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let off = |i: usize| i == 1;
        let w = List::new().with_item_height(20.0).disabled(&off);
        let mut st = ListState::new();
        let mut double = click_at(50.0, 30.0); // row 1
        double.mouse_double_clicked = true;
        let out = frame(&w, rect, 4, &mut st, &mut double);
        assert_eq!(
            (out.hovered, out.clicked, out.activated),
            (None, None, None)
        );
        assert_eq!(st.selected_count(), 0);

        let mut th = theme();
        th.row_hover = [1.0, 0.0, 0.0, 1.0];
        let mut dl = DrawList::new();
        let mut items = Vec::new();
        w.draw(
            rect,
            4,
            &mut st,
            &mut dl,
            &StyleResolver::new(&th),
            &mut idle_at(50.0, 30.0),
            |_, _, it| items.push(it),
        );
        assert!(items[1].disabled && !items[1].hovered);
        assert!(!items[0].disabled);
        assert!(
            dl.chrome_instances().all(|c| c.bg != th.row_hover),
            "no hover wash on a disabled row"
        );
    }

    fn idle_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    #[test]
    fn double_click_activates_the_row() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        let mut double = click_at(50.0, 50.0); // row 2
        double.mouse_double_clicked = true;
        let out = frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut double,
        );
        assert_eq!(out.activated, Some(2));
        assert!(st.is_selected(2));
    }

    /// The fill painted behind row 0 when it is selected, with the list
    /// focused or not.
    fn selected_fill(focused: bool) -> [f32; 4] {
        let rect = Rect::new(0.0, 0.0, 100.0, 60.0);
        let th = theme();
        let mut st = ListState::new();
        st.select_one(0);
        let mut dl = DrawList::new();
        let mut seen = None;
        List::new().with_item_height(20.0).focused(focused).draw(
            rect,
            3,
            &mut st,
            &mut dl,
            &StyleResolver::new(&th),
            &mut idle(),
            |_, _, it| {
                if it.index == 0 {
                    seen = Some(it);
                }
            },
        );
        let it = seen.expect("row 0 drawn");
        assert!(it.selected);
        assert_eq!(it.focused, focused);
        dl.chrome_instances()
            .find(|c| c.rect[3] == 20.0)
            .map(|c| c.bg)
            .expect("a row-sized fill")
    }

    #[test]
    fn selection_is_accent_with_focus_and_held_without() {
        let th = theme();
        assert_eq!(selected_fill(true), th.accent);
        assert_eq!(selected_fill(false), th.row_held);
    }

    #[test]
    fn zebra_uses_the_row_zebra_token() {
        let rect = Rect::new(0.0, 0.0, 100.0, 60.0);
        let th = theme();
        let mut dl = DrawList::new();
        List::new().with_item_height(20.0).with_zebra(true).draw(
            rect,
            3,
            &mut ListState::new(),
            &mut dl,
            &StyleResolver::new(&th),
            &mut idle(),
            |_, _, _| {},
        );
        let fills: Vec<_> = dl.chrome_instances().map(|c| c.bg).collect();
        assert_eq!(fills, vec![th.row_zebra], "only the odd row");
    }

    #[test]
    fn sync_selection_follows_the_callers_choice() {
        let mut st = ListState::new();
        st.sync_selection(Some(3));
        assert_eq!((st.single_selected(), st.cursor()), (Some(3), Some(3)));
        st.sync_selection(None);
        assert_eq!((st.single_selected(), st.cursor()), (None, None));
        st.select_range(1, 2);
        st.sync_selection(None);
        assert_eq!(st.selected_count(), 0, "a multi-selection is cleared too");
    }

    #[test]
    fn rows_and_clicks_follow_the_offset_the_frame_draws_at() {
        // Only the target is set (a keyboard reveal or a wheel notch does
        // this); the scroll view moves the drawn offset toward it at the start
        // of the frame. Culling and hit-testing must use that moved offset, or
        // the frame draws the rows for the old position under a translation
        // for the new one — blank, and clicks land on the wrong row.
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let w = List::new().with_item_height(20.0);
        let th = theme();
        for frame_dt in [0.0, 1.0 / 60.0] {
            let mut st = ListState::new();
            st.glide_to_row(50, 0.0);
            let mut input = InputState {
                frame_dt,
                ..click_at(10.0, 30.0)
            };
            let mut dl = DrawList::new();
            let mut drawn_rows = Vec::new();
            let out = w.draw(
                rect,
                100,
                &mut st,
                &mut dl,
                &StyleResolver::new(&th),
                &mut input,
                |_, _, it| drawn_rows.push(it.index),
            );
            let offset = st.scroll.offset[1];
            assert!(
                offset > 0.0,
                "the frame moved toward the target (dt {frame_dt})"
            );
            let top = (offset / 20.0).floor() as usize;
            assert_eq!(
                drawn_rows.first(),
                Some(&top),
                "dt {frame_dt}, offset {offset}"
            );
            let bottom = ((offset + 100.0) / 20.0).ceil() as usize - 1;
            assert!(
                drawn_rows.contains(&bottom),
                "dt {frame_dt}: {drawn_rows:?}"
            );
            let under_mouse = ((30.0 + offset) / 20.0).floor() as usize;
            assert_eq!(
                out.clicked,
                Some(under_mouse),
                "dt {frame_dt}, offset {offset}"
            );
        }
    }

    #[test]
    fn an_overlay_thumb_keeps_its_click_from_the_row_under_it() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let w = List::new().with_item_height(20.0).overlay_scrollbar();
        // The thumb floats 2 px in from the right edge, 9 px wide.
        let mut st = ListState::new();
        let out = frame(&w, rect, 100, &mut st, &mut click_at(93.0, 10.0));
        assert_eq!((out.clicked, out.hovered), (None, None));
        assert_eq!(st.selected_count(), 0);
        // Rows keep the full width beside it.
        let mut st = ListState::new();
        let out = frame(&w, rect, 100, &mut st, &mut click_at(85.0, 10.0));
        assert_eq!(out.clicked, Some(0));
    }

    #[test]
    fn a_cursor_past_the_end_steps_from_the_last_item() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let w = List::new().with_item_height(20.0).focused(true);
        let mut st = ListState::new();
        st.select_one(9); // the list has since shrunk to 4 items
        tap(&w, rect, 4, &mut st, |i| i.key_up = true);
        assert_eq!(st.cursor(), Some(2));
    }

    #[test]
    fn mouse_consumed_suppresses_interaction() {
        let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
        let mut st = ListState::new();
        let mut input = click_at(50.0, 10.0);
        input.mouse_consumed = true;
        let out = frame(
            &List::new().with_item_height(20.0),
            rect,
            5,
            &mut st,
            &mut input,
        );
        assert_eq!(out.clicked, None);
        assert_eq!(out.hovered, None);
        assert_eq!(st.selected_count(), 0);
    }
}

#[cfg(test)]
#[path = "list_measured_tests.rs"]
mod measured_tests;
