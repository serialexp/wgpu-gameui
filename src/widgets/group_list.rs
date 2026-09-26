//! Group list — a sidebar list whose items sit under collapsible group
//! headers (Forge's session sidebar): a project header, its sessions, and a
//! "… N older" row that folds the rest away.
//!
//! Three kinds of row, each a fixed height (including the 1 px rule under it):
//!
//! - **Header** ([`GroupHeader`], 23 px): a disclosure triangle, an optional
//!   [`Thumb`], the group's name and a right-aligned count, on an opaque
//!   raised bar. Clicking it asks to toggle the group.
//! - **Item** ([`GroupItem`], 37 px): a [`status_dot`], a title with a mono
//!   sub line under it, and on the right a compact hue [`Badge`](crate::Badge) over a meta text (an
//!   age). While hovered, the meta gives way to a ghost `⋯` key that asks for
//!   the item's menu. The selected item wears the accent.
//! - **More** ([`GroupMore`], 23 px): a quiet mono "… 3 older" row with a
//!   meta on the right. Clicking it asks to show what it hides.
//!
//! The widget owns no data and no grouping rules. The caller decides which
//! rows exist, builds a [`GroupLayout`] from their kinds whenever they change
//! (a prefix sum of row tops, so finding the visible rows is a binary
//! search), and hands each frame a closure that builds row `i` from borrowed
//! strings. Only visible rows are built and drawn, so 10 000 rows cost what a
//! screenful does.
//!
//! The caller owns the selection (by index, mapped from its own ids each
//! frame). [`GroupListOutput`] reports what the user asked for: a new
//! selection, an opened item, a header to toggle, a "more" row to expand, or
//! a menu for an item (with the point to open it at). While
//! [`focused`](GroupList::focused), the arrow keys walk every row: landing on
//! an item selects it, `Enter` opens an item, toggles a header or expands a
//! "more" row, and `←` / `→` fold and unfold a header (`←` on an item jumps to
//! its header).
//!
//! Gated behind the `phosphor-icons` feature.
//!
//! # Example
//! ```ignore
//! if rows_changed { layout.rebuild(rows.iter().map(Row::kind)); }
//! let out = GroupList::new().focused(focused).draw(
//!     rect, &layout, selected, &mut state, list, &style, &mut input,
//!     |i| rows[i].as_group_row());
//! if let Some(i) = out.toggle { open.toggle(rows[i].group()); }
//! ```

use std::ops::Range;

use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::{TextBlock, vcentered_line_y};
use crate::{Edge, InputState};

use super::badge::{BADGE_COMPACT_HEIGHT, Badge, BadgeTone};
use super::material::{self, Material, Tone};
use super::scroll_view::{ScrollState, ScrollView};
use super::status_dot::{STATUS_DOT_SIZE, Status, status_dot};
use super::tree::{CARET_HALF, draw_disclosure};
use super::{DrawList, Icon, Thumb};

/// Height of a header row, including the rule under it.
pub const GROUP_HEADER_HEIGHT: f32 = 23.0;
/// Height of an item row, including the rule under it.
pub const GROUP_ITEM_HEIGHT: f32 = 37.0;
/// Height of a "more" row, including the rule under it.
pub const GROUP_MORE_HEIGHT: f32 = 23.0;
/// How far each nesting depth moves a row's content right.
pub const GROUP_DEPTH_INDENT: f32 = 11.0;

/// Left padding of a header, an item and a "more" row at depth 0.
const HEADER_PAD_LEFT: f32 = 8.0;
const ITEM_PAD_LEFT: f32 = 12.0;
/// Right padding of a header, and of items and "more" rows.
const HEADER_PAD_RIGHT: f32 = 10.0;
const ITEM_PAD_RIGHT: f32 = 9.0;
/// Gaps between a header's parts, and an item's.
const HEADER_GAP: f32 = 6.0;
const ITEM_GAP: f32 = 8.0;
/// Width of a header's disclosure column.
const CARET_W: f32 = 10.0;
/// A header's thumb size.
const HEADER_THUMB: f32 = 14.0;
/// An item's content height (without its rule), and the centres of its two
/// text lines: a 14.3 px title line, a 2 px gap and a 13 px sub line,
/// centred in the row.
const ITEM_BODY: f32 = 36.0;
const TITLE_CY: f32 = 10.5;
const SUB_CY: f32 = 26.15;
/// The chip's top, and the centre of the meta (age) line under it.
const CHIP_TOP: f32 = 4.0;
const META_CY: f32 = 26.0;
/// The hover `⋯` key: its size, and how far it reaches past the right padding.
const KEY_SIZE: f32 = 15.0;
const KEY_OVERHANG: f32 = 3.0;
/// Width of a "more" row's leading `…` column.
const MORE_MARK_W: f32 = 6.0;
/// Top and bottom lines on the selected item (`--row-select-inset`).
const SELECT_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.28];
const SELECT_BOTTOM: [f32; 4] = [0.0, 0.0, 0.0, 0.25];
/// The carve under header names (`--carve`).
const CARVE_ALPHA: u8 = 128;

/// Which kind a row is. Fixes the row's height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupRowKind {
    /// A group header.
    Header,
    /// An item in a group.
    Item,
    /// A "… N older" row.
    More,
}

impl GroupRowKind {
    /// The row's height, including the rule under it.
    pub const fn height(self) -> f32 {
        match self {
            GroupRowKind::Header => GROUP_HEADER_HEIGHT,
            GroupRowKind::Item => GROUP_ITEM_HEIGHT,
            GroupRowKind::More => GROUP_MORE_HEIGHT,
        }
    }
}

/// Where every row of a [`GroupList`] sits: the rows' kinds and the prefix
/// sum of their heights. Rebuild it when the rows change, not every frame;
/// [`rebuild`](Self::rebuild) reuses its buffers.
#[derive(Clone, Debug, Default)]
pub struct GroupLayout {
    kinds: Vec<GroupRowKind>,
    /// `tops[i]` is row `i`'s top; `tops[len]` is the total height.
    tops: Vec<f32>,
}

impl GroupLayout {
    /// An empty layout.
    pub fn new() -> Self {
        Self::default()
    }

    /// A layout of rows of these kinds.
    pub fn from_kinds(kinds: impl IntoIterator<Item = GroupRowKind>) -> Self {
        let mut layout = Self::new();
        layout.rebuild(kinds);
        layout
    }

    /// Replace the rows with rows of these kinds.
    pub fn rebuild(&mut self, kinds: impl IntoIterator<Item = GroupRowKind>) {
        self.kinds.clear();
        self.kinds.extend(kinds);
        self.tops.clear();
        self.tops.reserve(self.kinds.len() + 1);
        let mut y = 0.0;
        self.tops.push(y);
        for kind in &self.kinds {
            y += kind.height();
            self.tops.push(y);
        }
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Row `i`'s kind.
    pub fn kind(&self, i: usize) -> GroupRowKind {
        self.kinds[i]
    }

    /// Row `i`'s top, from the top of the first row.
    pub fn top(&self, i: usize) -> f32 {
        self.tops[i]
    }

    /// The height of all rows together.
    pub fn height(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0)
    }

    /// The row at `y` (from the top of the first row), if any.
    pub fn row_at(&self, y: f32) -> Option<usize> {
        if self.is_empty() || !(0.0..self.height()).contains(&y) {
            return None;
        }
        // The last top at or above `y`.
        Some(self.tops.partition_point(|&top| top <= y) - 1)
    }

    /// The rows that overlap `[y0, y1)`.
    pub fn rows_between(&self, y0: f32, y1: f32) -> Range<usize> {
        if self.is_empty() || y1 <= y0 {
            return 0..0;
        }
        let first = self.tops[1..].partition_point(|&bottom| bottom <= y0);
        let end = self.tops[..self.len()].partition_point(|&top| top < y1);
        first.min(end)..end
    }
}

/// A group header row. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct GroupHeader<'a> {
    /// The group's name.
    pub name: &'a str,
    /// The right-aligned count ("2 / 5").
    pub count: &'a str,
    /// Whether the group is open (the triangle points down).
    pub open: bool,
    /// The tile before the name.
    pub thumb: Option<Thumb<'a>>,
    /// Set the name in the mono font (a project) rather than sans (a
    /// section such as "quiet projects").
    pub mono: bool,
    /// Dim the name (a secondary section).
    pub dim: bool,
    /// Nesting depth; each step indents by [`GROUP_DEPTH_INDENT`].
    pub depth: u8,
}

impl<'a> GroupHeader<'a> {
    /// An open, mono, undimmed header at depth 0 with no thumb.
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            count: "",
            open: true,
            thumb: None,
            mono: true,
            dim: false,
            depth: 0,
        }
    }

    /// Set the count.
    #[must_use]
    pub fn count(mut self, count: &'a str) -> Self {
        self.count = count;
        self
    }

    /// Open or close the group.
    #[must_use]
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Put a tile before the name (its size is set to 14 px).
    #[must_use]
    pub fn thumb(mut self, thumb: Thumb<'a>) -> Self {
        self.thumb = Some(thumb);
        self
    }

    /// Set the name in mono (`true`, the default) or sans.
    #[must_use]
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// Dim the name.
    #[must_use]
    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Set the nesting depth.
    #[must_use]
    pub fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }
}

/// An item row. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct GroupItem<'a> {
    /// The title.
    pub title: &'a str,
    /// The mono line under the title.
    pub subtitle: &'a str,
    /// A hue-tinted chip on the right: its text and hue in degrees.
    pub chip: Option<(&'a str, f32)>,
    /// The meta under the chip (an age), replaced by the `⋯` key on hover.
    pub meta: &'a str,
    /// The dot before the title.
    pub status: Status,
    /// Dim the title (an older item).
    pub dim: bool,
    /// Offer a menu: right-click and the hover `⋯` key ask for it.
    pub menu: bool,
    /// Nesting depth; each step indents by [`GROUP_DEPTH_INDENT`].
    pub depth: u8,
}

impl<'a> GroupItem<'a> {
    /// An idle, undimmed item at depth 0 with a menu, no subtitle, chip or
    /// meta.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            subtitle: "",
            chip: None,
            meta: "",
            status: Status::Idle,
            dim: false,
            menu: true,
            depth: 0,
        }
    }

    /// Set the mono line under the title.
    #[must_use]
    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = subtitle;
        self
    }

    /// Show a chip with `text`, tinted by `hue`.
    #[must_use]
    pub fn chip(mut self, text: &'a str, hue: f32) -> Self {
        self.chip = Some((text, hue));
        self
    }

    /// Set the meta.
    #[must_use]
    pub fn meta(mut self, meta: &'a str) -> Self {
        self.meta = meta;
        self
    }

    /// Set the status dot.
    #[must_use]
    pub fn status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }

    /// Dim the title.
    #[must_use]
    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Offer (the default) or withhold the item's menu.
    #[must_use]
    pub fn menu(mut self, menu: bool) -> Self {
        self.menu = menu;
        self
    }

    /// Set the nesting depth.
    #[must_use]
    pub fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }
}

/// A "more" row. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct GroupMore<'a> {
    /// The label ("3 older").
    pub label: &'a str,
    /// The right-aligned meta ("3d – 8d").
    pub meta: &'a str,
    /// Nesting depth; each step indents by [`GROUP_DEPTH_INDENT`].
    pub depth: u8,
}

impl<'a> GroupMore<'a> {
    /// A "more" row at depth 0.
    pub fn new(label: &'a str, meta: &'a str) -> Self {
        Self {
            label,
            meta,
            depth: 0,
        }
    }

    /// Set the nesting depth.
    #[must_use]
    pub fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }
}

/// One row of a [`GroupList`], borrowed from the caller's data. Its kind
/// must match the [`GroupLayout`]'s kind for the same index.
#[derive(Clone, Copy, Debug)]
pub enum GroupRow<'a> {
    /// A group header.
    Header(GroupHeader<'a>),
    /// An item.
    Item(GroupItem<'a>),
    /// A "more" row.
    More(GroupMore<'a>),
}

impl GroupRow<'_> {
    /// The row's kind.
    pub fn kind(&self) -> GroupRowKind {
        match self {
            GroupRow::Header(_) => GroupRowKind::Header,
            GroupRow::Item(_) => GroupRowKind::Item,
            GroupRow::More(_) => GroupRowKind::More,
        }
    }
}

/// A request to open an item's menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupMenuRequest {
    /// The item's row.
    pub row: usize,
    /// Where to open the menu, in screen pixels (the pointer).
    pub at: [f32; 2],
}

/// What the user did with a [`GroupList`] this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupListOutput {
    /// An item the user selected (a click or an arrow key), when it differs
    /// from the `selected` passed in.
    pub select: Option<usize>,
    /// An item opened: double-clicked, or `Enter` while focused.
    pub open: Option<usize>,
    /// A header to open or close.
    pub toggle: Option<usize>,
    /// A "more" row to expand.
    pub more: Option<usize>,
    /// An item whose menu the user asked for.
    pub menu: Option<GroupMenuRequest>,
    /// The row under the pointer.
    pub hovered: Option<usize>,
    /// Whether the pointer is over the rows (for scroll routing).
    pub mouse_over_content: bool,
}

/// Rising-edge navigation keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Keys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    home: bool,
    end: bool,
    confirm: bool,
}

impl Keys {
    fn raw(input: &InputState) -> Self {
        Self {
            up: input.nav.up,
            down: input.nav.down,
            left: input.nav.left,
            right: input.nav.right,
            home: input.key_home,
            end: input.key_end,
            confirm: input.nav.confirm,
        }
    }

    fn edges(self, prev: Self) -> Self {
        Self {
            up: self.up && !prev.up,
            down: self.down && !prev.down,
            left: self.left && !prev.left,
            right: self.right && !prev.right,
            home: self.home && !prev.home,
            end: self.end && !prev.end,
            confirm: self.confirm && !prev.confirm,
        }
    }
}

/// Caller-owned, persistent state of a [`GroupList`]: the scroll offset and
/// the keyboard cursor.
#[derive(Clone, Debug, Default)]
pub struct GroupListState {
    /// Scroll offset and content extent.
    pub scroll: ScrollState,
    /// The row the arrow keys move from.
    cursor: Option<usize>,
    /// The selection the caller passed last frame, to follow its changes.
    last_selected: Option<usize>,
    /// Last frame's raw keys, for edge detection.
    prev_keys: Keys,
}

impl GroupListState {
    /// A fresh state: scrolled to the top, no cursor.
    pub fn new() -> Self {
        Self::default()
    }

    /// The keyboard cursor's row.
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// Move the keyboard cursor.
    pub fn set_cursor(&mut self, row: Option<usize>) {
        self.cursor = row;
    }

    /// The caller rebuilt its rows, so their indices moved: the cursor's row
    /// is now `cursor`, and the selected row the caller passes is now
    /// `selected`. The moved selection isn't a new one, so the cursor stays
    /// on its row rather than jumping to the selection.
    pub fn rows_moved(&mut self, cursor: Option<usize>, selected: Option<usize>) {
        self.cursor = cursor;
        self.last_selected = selected;
    }
}

/// A grouped sidebar list. See the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct GroupList {
    focused: bool,
}

/// Per-frame facts one row's painter needs.
#[derive(Clone, Copy)]
struct RowPaint {
    /// The row in world space (the scroll view's transform applies).
    rect: Rect,
    hovered: bool,
    selected: bool,
    cursor: bool,
    /// The pointer is on the item's `⋯` key, and the mouse is down.
    key_hot: bool,
    key_down: bool,
}

impl GroupList {
    /// An unfocused group list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the list has the keyboard (arrow keys, `Enter`).
    #[must_use]
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Draw the rows of `layout` into `rect`. `selected` is the caller's
    /// selected item row, `row(i)` builds row `i` (asked only for visible
    /// rows, and for the cursor's row on a key press).
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'r, F>(
        &self,
        rect: Rect,
        layout: &GroupLayout,
        selected: Option<usize>,
        state: &mut GroupListState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        row: F,
    ) -> GroupListOutput
    where
        F: Fn(usize) -> GroupRow<'r>,
    {
        list.push_debug_scope_rect("GroupList", rect);
        let count = layout.len();
        let selected = selected.filter(|&i| i < count && layout.kind(i) == GroupRowKind::Item);
        if selected != state.last_selected {
            if selected.is_some() {
                state.cursor = selected;
            }
            state.last_selected = selected;
        }
        state.cursor = state.cursor.filter(|&c| c < count);
        state.scroll.content_size = [rect.width, layout.height()];

        let mut out = GroupListOutput::default();
        let raw = Keys::raw(input);
        let keys = raw.edges(state.prev_keys);
        state.prev_keys = raw;
        if self.focused && count > 0 {
            self.navigate(keys, layout, selected, state, rect, &row, &mut out);
        }

        let mouse = (input.mouse_x, input.mouse_y);
        let over = rect.contains(mouse.0, mouse.1) && !input.mouse_consumed;
        out.mouse_over_content = over;

        let view = ScrollView::new(rect).vertical_only().overlay();
        let begun = view.begin(&mut state.scroll, list, input);
        let vp = begun.inner;
        let scroll_y = state.scroll.offset[1];
        // An overlay thumb under the pointer has taken it from the rows.
        let rows_hot = over && !input.mouse_consumed && vp.contains(mouse.0, mouse.1);
        let hovered = if rows_hot {
            layout.row_at(mouse.1 - vp.y + scroll_y)
        } else {
            None
        };
        out.hovered = hovered;

        for i in layout.rows_between(scroll_y, scroll_y + vp.height) {
            let kind = layout.kind(i);
            let world = Rect::new(vp.x, vp.y + layout.top(i), vp.width, kind.height());
            let screen = Rect::new(world.x, world.y - scroll_y, world.width, world.height);
            let r = row(i);
            debug_assert_eq!(r.kind(), kind, "row {i} does not match the layout");
            let is_hovered = hovered == Some(i);
            let key = match r {
                GroupRow::Item(item) if item.menu && is_hovered => Some(key_rect(screen)),
                _ => None,
            };
            let key_hot = key.is_some_and(|k| k.contains(mouse.0, mouse.1));
            let paint = RowPaint {
                rect: world,
                hovered: is_hovered,
                selected: selected == Some(i),
                cursor: self.focused && state.cursor == Some(i),
                key_hot,
                key_down: key_hot && input.mouse_down,
            };
            if is_hovered {
                self.click(r, i, selected, key_hot, input, state, &mut out);
            }
            match r {
                GroupRow::Header(h) => draw_header(list, style, &h, paint),
                GroupRow::Item(item) => draw_item(list, style, &item, paint),
                GroupRow::More(m) => draw_more(list, style, &m, paint),
            }
        }
        view.end(&mut state.scroll, list, style, input, begun);
        list.pop_debug_scope();
        out
    }

    /// Handle this frame's click on row `i` (the hovered row).
    #[allow(clippy::too_many_arguments)]
    fn click(
        &self,
        r: GroupRow,
        i: usize,
        selected: Option<usize>,
        on_key: bool,
        input: &InputState,
        state: &mut GroupListState,
        out: &mut GroupListOutput,
    ) {
        let at = [input.mouse_x, input.mouse_y];
        match r {
            GroupRow::Item(item) => {
                // The `⋯` key only exists on items with a menu.
                if (item.menu && input.mouse_right_clicked) || (input.mouse_clicked && on_key) {
                    out.menu = Some(GroupMenuRequest { row: i, at });
                } else if input.mouse_clicked {
                    state.cursor = Some(i);
                    if selected != Some(i) {
                        out.select = Some(i);
                    }
                    if input.mouse_double_clicked {
                        out.open = Some(i);
                    }
                }
            }
            GroupRow::Header(_) if input.mouse_clicked => {
                state.cursor = Some(i);
                out.toggle = Some(i);
            }
            GroupRow::More(_) if input.mouse_clicked => {
                state.cursor = Some(i);
                out.more = Some(i);
            }
            _ => {}
        }
    }

    /// Apply this frame's navigation keys.
    #[allow(clippy::too_many_arguments)]
    fn navigate<'r, F>(
        &self,
        keys: Keys,
        layout: &GroupLayout,
        selected: Option<usize>,
        state: &mut GroupListState,
        rect: Rect,
        row: &F,
        out: &mut GroupListOutput,
    ) where
        F: Fn(usize) -> GroupRow<'r>,
    {
        let last = layout.len() - 1;
        let target = match state.cursor {
            None if keys.up || keys.down => Some(0),
            None => None,
            Some(c) if keys.down => Some((c + 1).min(last)),
            Some(c) if keys.up => Some(c.saturating_sub(1)),
            Some(c) if keys.left || keys.right => {
                match row(c) {
                    GroupRow::Header(h) if h.open == keys.left => out.toggle = Some(c),
                    GroupRow::Item(_) | GroupRow::More(_) if keys.left => {
                        // Jump to the group's header.
                        return self.move_to(
                            (0..c)
                                .rev()
                                .find(|&j| layout.kind(j) == GroupRowKind::Header),
                            layout,
                            selected,
                            state,
                            rect,
                            out,
                        );
                    }
                    _ => {}
                }
                None
            }
            Some(_) => None,
        };
        let target = if keys.home {
            Some(0)
        } else if keys.end {
            Some(last)
        } else {
            target
        };
        self.move_to(target, layout, selected, state, rect, out);

        if keys.confirm
            && let Some(c) = state.cursor
        {
            match layout.kind(c) {
                GroupRowKind::Header => out.toggle = Some(c),
                GroupRowKind::Item => out.open = Some(c),
                GroupRowKind::More => out.more = Some(c),
            }
        }
    }

    /// Put the cursor on `target`, selecting it if it's an item, and scroll
    /// it into view.
    fn move_to(
        &self,
        target: Option<usize>,
        layout: &GroupLayout,
        selected: Option<usize>,
        state: &mut GroupListState,
        rect: Rect,
        out: &mut GroupListOutput,
    ) {
        let Some(t) = target else {
            return;
        };
        state.cursor = Some(t);
        if layout.kind(t) == GroupRowKind::Item && selected != Some(t) {
            out.select = Some(t);
        }
        let top = layout.top(t);
        state
            .scroll
            .scroll_range_into_view(1, top, top + layout.kind(t).height(), rect.height);
        state.scroll.clamp([rect.width, rect.height]);
    }
}

/// The hover `⋯` key's rect in an item row (screen or world, as `row` is).
fn key_rect(row: Rect) -> Rect {
    Rect::new(
        row.right() - ITEM_PAD_RIGHT + KEY_OVERHANG - KEY_SIZE,
        (row.y + META_CY - KEY_SIZE * 0.5).round(),
        KEY_SIZE,
        KEY_SIZE,
    )
}

/// The top `y` of a one-line block of `size` whose line box is centred on `cy`
/// (how the design's flex rows place their text).
fn line_y(cy: f32, size: f32) -> f32 {
    vcentered_line_y(cy, 0.0, size)
}

/// Draw a text block right-aligned so it ends at `right`; returns its left.
fn text_right(list: &mut DrawList, mut block: TextBlock, right: f32) -> f32 {
    let (w, _) = list.measure_block(&block);
    block.x = right - w;
    list.text(block);
    right - w
}

fn row_rule(list: &mut DrawList, s: &StyleResolver, rect: Rect) {
    let rule = s.group_list().row_rule;
    list.edge_line(rect, Edge::Bottom, rule.thickness, rule.color);
}

fn draw_header(list: &mut DrawList, s: &StyleResolver, h: &GroupHeader, p: RowPaint) {
    let chrome = s.group_list();
    let r = p.rect;
    list.paint_quad_background(r, chrome.header, crate::CornerRadii::uniform(0.0));
    if p.cursor {
        list.quad(r.x, r.y, r.width, r.height, s.color(StyleKey::RowHover));
    }
    list.edge_line(
        r,
        Edge::Top,
        chrome.header_highlight.thickness,
        chrome.header_highlight.color,
    );
    list.edge_line(
        r,
        Edge::Bottom,
        chrome.header_rule.thickness,
        chrome.header_rule.color,
    );
    let cy = r.y + (r.height - chrome.header_rule.thickness) * 0.5;
    let mut x = r.x + HEADER_PAD_LEFT + f32::from(h.depth) * GROUP_DEPTH_INDENT;
    draw_disclosure(
        list,
        x + CARET_W * 0.5,
        cy,
        CARET_HALF,
        h.open,
        s.ink(Ink::Body2),
    );
    x += CARET_W + HEADER_GAP;
    if let Some(thumb) = h.thumb {
        thumb
            .size(HEADER_THUMB)
            .draw(x, (cy - HEADER_THUMB * 0.5).round(), list, s);
        x += HEADER_THUMB + HEADER_GAP;
    }
    let mut right = r.right() - HEADER_PAD_RIGHT;
    if !h.count.is_empty() {
        let size = s.text_size(TextSize::Meta);
        let block = s.mono_block(h.count, 0.0, line_y(cy, size), TextSize::Meta, Ink::Caption);
        right = text_right(list, block, right) - HEADER_GAP;
    }
    let size = s.text_size(TextSize::Dense);
    let ink = if h.dim { Ink::Caption } else { Ink::Title };
    let block = if h.mono {
        s.mono_block(h.name, x, line_y(cy, size), TextSize::Dense, ink)
    } else {
        s.sans_block(h.name, x, line_y(cy, size), TextSize::Dense, ink)
    };
    list.text(
        block
            .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0)
            .with_max_width((right - x).max(0.0))
            .with_ellipsis(),
    );
}

fn draw_item(list: &mut DrawList, s: &StyleResolver, item: &GroupItem, p: RowPaint) {
    let r = p.rect;
    if p.selected {
        list.quad(r.x, r.y, r.width, r.height, s.color(StyleKey::Accent));
        list.quad(r.x, r.y, r.width, 1.0, SELECT_TOP);
        list.quad(r.x, r.y + ITEM_BODY - 1.0, r.width, 1.0, SELECT_BOTTOM);
    } else if p.hovered || p.cursor {
        list.quad(r.x, r.y, r.width, r.height, s.color(StyleKey::RowHover));
    }
    row_rule(list, s, r);

    let left = r.x + ITEM_PAD_LEFT + f32::from(item.depth) * GROUP_DEPTH_INDENT;
    status_dot(
        list,
        s,
        (left + STATUS_DOT_SIZE * 0.5, r.y + ITEM_BODY * 0.5),
        item.status,
    );
    let text_x = left + STATUS_DOT_SIZE + ITEM_GAP;
    let right = r.right() - ITEM_PAD_RIGHT;
    let (title_ink, sub_ink) = if p.selected {
        (s.color(StyleKey::OnAccent), s.ink(Ink::OnAccentSecond))
    } else if item.dim {
        (s.ink(Ink::Body2), s.ink(Ink::Caption))
    } else {
        (s.ink(Ink::Row), s.ink(Ink::Caption))
    };

    // The right column: the chip over the meta (or, hovered, the key).
    let meta_size = s.text_size(TextSize::Meta);
    let mut column = 0.0f32;
    if let Some((text, hue)) = item.chip {
        let chip = Badge::new(BadgeTone::Hue(hue))
            .compact()
            .on_accent(p.selected)
            .draw_right(list, s, right, r.y + CHIP_TOP, text);
        column = column.max(chip.width);
    }
    const { assert!(CHIP_TOP + BADGE_COMPACT_HEIGHT < META_CY) };
    if item.menu && p.hovered {
        let key = key_rect(r);
        let face = material::draw(
            list,
            s,
            key,
            &Material::new(Tone::Ghost)
                .hovered(p.key_hot)
                .pressed(p.key_down)
                .travel(0.0),
        );
        let ink = if p.key_down {
            s.ink(Ink::Glyph)
        } else {
            s.ink(Ink::Cell)
        };
        let side = 9.0;
        Icon::new(PhosphorIcon::DotsThree).tint(ink).draw(
            Rect::new(
                face.x + (face.width - side) * 0.5,
                face.y + (face.height - side) * 0.5,
                side,
                side,
            ),
            list,
        );
        column = column.max(KEY_SIZE - KEY_OVERHANG);
    } else if !item.meta.is_empty() {
        let block = s
            .mono_block(
                item.meta,
                0.0,
                line_y(r.y + META_CY, meta_size),
                TextSize::Meta,
                Ink::Caption,
            )
            .with_color_f32(sub_ink);
        let left = text_right(list, block, right);
        column = column.max(right - left);
    }

    let max_w = (right - column - if column > 0.0 { ITEM_GAP } else { 0.0 } - text_x).max(0.0);
    let title_size = s.text_size(TextSize::Row);
    list.text(
        s.sans_block(
            item.title,
            text_x,
            line_y(r.y + TITLE_CY, title_size),
            TextSize::Row,
            Ink::Row,
        )
        .with_color_f32(title_ink)
        .with_max_width(max_w)
        .with_ellipsis(),
    );
    if !item.subtitle.is_empty() {
        list.text(
            s.mono_block(
                item.subtitle,
                text_x,
                line_y(r.y + SUB_CY, meta_size),
                TextSize::Meta,
                Ink::Caption,
            )
            .with_color_f32(sub_ink)
            .with_max_width(max_w)
            .with_ellipsis(),
        );
    }
}

fn draw_more(list: &mut DrawList, s: &StyleResolver, m: &GroupMore, p: RowPaint) {
    let r = p.rect;
    if p.hovered || p.cursor {
        list.quad(r.x, r.y, r.width, r.height, s.color(StyleKey::RowHover));
    }
    row_rule(list, s, r);
    let cy = r.y + (r.height - 1.0) * 0.5;
    let size = s.text_size(TextSize::Meta);
    let y = line_y(cy, size);
    let ink = if p.hovered { Ink::Row } else { Ink::Caption };
    let left = r.x + ITEM_PAD_LEFT + f32::from(m.depth) * GROUP_DEPTH_INDENT;
    let mut mark = s.mono_block("…", 0.0, y, TextSize::Meta, Ink::DisabledGlyph);
    let (mark_w, _) = list.measure_block(&mark);
    mark.x = left + (MORE_MARK_W - mark_w) * 0.5;
    list.text(mark);
    let right = r.right() - ITEM_PAD_RIGHT;
    let meta_left = if m.meta.is_empty() {
        right
    } else {
        text_right(
            list,
            s.mono_block(m.meta, 0.0, y, TextSize::Meta, ink),
            right,
        ) - ITEM_GAP
    };
    let x = left + MORE_MARK_W + ITEM_GAP;
    list.text(
        s.mono_block(m.label, x, y, TextSize::Meta, ink)
            .with_max_width((meta_left - x).max(0.0))
            .with_ellipsis(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::color::text_color;

    const RECT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 272.0,
        height: 300.0,
    };

    /// A small fixture: a header, two items, a "more" row, a closed header.
    fn rows() -> Vec<GroupRow<'static>> {
        vec![
            GroupRow::Header(
                GroupHeader::new("agent-ui")
                    .count("2 / 5")
                    .thumb(Thumb::new().name("agent-ui")),
            ),
            GroupRow::Item(
                GroupItem::new("Session context menu")
                    .subtitle("@merry-tiger · 447 msgs")
                    .chip("claude opus", 45.0)
                    .meta("now")
                    .status(Status::Running),
            ),
            GroupRow::Item(
                GroupItem::new("README repo description")
                    .subtitle("@warm-otter · 6002 msgs")
                    .chip("codex", 160.0)
                    .meta("40m"),
            ),
            GroupRow::More(GroupMore::new("3 older", "3d – 8d")),
            GroupRow::Header(GroupHeader::new("dotfiles").count("1").open(false)),
        ]
    }

    fn layout(rows: &[GroupRow]) -> GroupLayout {
        GroupLayout::from_kinds(rows.iter().map(GroupRow::kind))
    }

    fn at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn frame(
        rows: &[GroupRow],
        view: GroupList,
        selected: Option<usize>,
        state: &mut GroupListState,
        input: &mut InputState,
    ) -> (DrawList, GroupListOutput) {
        crate::map_keyboard(input);
        let theme = Theme::default();
        let mut list = DrawList::new();
        let layout = layout(rows);
        let out = view.draw(
            RECT,
            &layout,
            selected,
            state,
            &mut list,
            &StyleResolver::new(&theme),
            input,
            |i| rows[i],
        );
        (list, out)
    }

    fn text<'l>(list: &'l DrawList, content: &str) -> &'l TextBlock {
        list.texts
            .iter()
            .find(|t| t.content == content)
            .unwrap_or_else(|| panic!("no text {content:?}"))
    }

    fn has_text(list: &DrawList, content: &str) -> bool {
        list.texts.iter().any(|t| t.content == content)
    }

    #[test]
    fn layout_tops_are_a_prefix_sum_of_row_heights() {
        let l = layout(&rows());
        assert_eq!(l.len(), 5);
        assert_eq!(l.top(0), 0.0);
        assert_eq!(l.top(1), 23.0);
        assert_eq!(l.top(2), 60.0);
        assert_eq!(l.top(3), 97.0);
        assert_eq!(l.top(4), 120.0);
        assert_eq!(l.height(), 143.0);
    }

    #[test]
    fn row_at_and_rows_between_binary_search_the_tops() {
        let l = layout(&rows());
        assert_eq!(l.row_at(0.0), Some(0));
        assert_eq!(l.row_at(22.9), Some(0));
        assert_eq!(l.row_at(23.0), Some(1));
        assert_eq!(l.row_at(96.0), Some(2));
        assert_eq!(l.row_at(142.0), Some(4));
        assert_eq!(l.row_at(143.0), None);
        assert_eq!(l.row_at(-1.0), None);
        assert_eq!(l.rows_between(0.0, 23.0), 0..1);
        assert_eq!(l.rows_between(22.0, 61.0), 0..3);
        assert_eq!(l.rows_between(100.0, 1000.0), 3..5);
        assert_eq!(l.rows_between(500.0, 600.0), 5..5);
        assert_eq!(GroupLayout::new().rows_between(0.0, 10.0), 0..0);
    }

    #[test]
    fn rebuild_reuses_its_buffers() {
        let mut l = GroupLayout::from_kinds([GroupRowKind::Item; 100]);
        let cap = l.tops.capacity();
        l.rebuild([GroupRowKind::Header; 50]);
        assert_eq!(l.tops.capacity(), cap);
        assert_eq!(l.height(), 50.0 * GROUP_HEADER_HEIGHT);
    }

    #[test]
    fn rows_show_their_parts() {
        let rows = rows();
        let (list, _) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut GroupListState::new(),
            &mut at(-1.0, -1.0),
        );
        for t in [
            "agent-ui",
            "2 / 5",
            "AU",
            "Session context menu",
            "@merry-tiger · 447 msgs",
            "claude opus",
            "now",
            "…",
            "3 older",
            "3d – 8d",
            "dotfiles",
        ] {
            assert!(has_text(&list, t), "missing {t:?}");
        }
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        assert_eq!(text(&list, "agent-ui").font, theme.mono_font);
        assert_eq!(
            text(&list, "Session context menu").color,
            text_color(s.ink(Ink::Row))
        );
        // The running dot glows; the idle one doesn't.
        assert!(list.shadow_instance_count() >= 1);
    }

    #[test]
    fn the_title_stops_short_of_the_right_column() {
        let rows = rows();
        let (mut list, _) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut GroupListState::new(),
            &mut at(-1.0, -1.0),
        );
        let title = text(&list, "Session context menu").clone();
        let chip = text(&list, "claude opus").clone();
        let (chip_w, _) = list.measure_block(&chip);
        assert_eq!(title.x, ITEM_PAD_LEFT + STATUS_DOT_SIZE + ITEM_GAP);
        assert!(title.ellipsize);
        let chip_left = chip.x - 4.0;
        assert!(
            (title.x + title.max_width - (chip_left - ITEM_GAP)).abs() < 0.5,
            "the title ends a gap before the chip"
        );
        assert!((chip.x + chip_w + 4.0 - (RECT.width - ITEM_PAD_RIGHT)).abs() < 0.5);
    }

    #[test]
    fn hovering_an_item_swaps_its_age_for_the_key() {
        let rows = rows();
        let (list, out) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut GroupListState::new(),
            &mut at(100.0, 30.0),
        );
        assert_eq!(out.hovered, Some(1));
        assert!(!has_text(&list, "now"), "the age gives way to the key");
        assert!(has_text(&list, "40m"), "other rows keep theirs");
    }

    #[test]
    fn clicks_select_toggle_expand_and_ask_for_menus() {
        let rows = rows();
        let mut state = GroupListState::new();
        let click = |x: f32, y: f32| InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..at(x, y)
        };
        let (_, out) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut state,
            &mut click(100.0, 30.0),
        );
        assert_eq!(out.select, Some(1));
        let (_, out) = frame(
            &rows,
            GroupList::new(),
            Some(1),
            &mut state,
            &mut click(100.0, 30.0),
        );
        assert_eq!(out.select, None, "already selected");
        let (_, out) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut state,
            &mut click(100.0, 10.0),
        );
        assert_eq!(out.toggle, Some(0));
        let (_, out) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut state,
            &mut click(100.0, 105.0),
        );
        assert_eq!(out.more, Some(3));

        let key = key_rect(Rect::new(0.0, 60.0, RECT.width, GROUP_ITEM_HEIGHT));
        let (_, out) = frame(
            &rows,
            GroupList::new(),
            None,
            &mut state,
            &mut click(key.x + 5.0, key.y + 5.0),
        );
        assert_eq!(
            out.menu.map(|m| m.row),
            Some(2),
            "the key asks for the menu"
        );
        assert_eq!(out.select, None, "without selecting");

        let mut right = at(100.0, 70.0);
        right.mouse_right_clicked = true;
        let (_, out) = frame(&rows, GroupList::new(), None, &mut state, &mut right);
        assert_eq!(
            out.menu,
            Some(GroupMenuRequest {
                row: 2,
                at: [100.0, 70.0]
            })
        );

        let mut double = click(100.0, 30.0);
        double.mouse_double_clicked = true;
        let (_, out) = frame(&rows, GroupList::new(), Some(1), &mut state, &mut double);
        assert_eq!(out.open, Some(1));
    }

    #[test]
    fn the_selected_item_wears_the_accent_and_on_accent_ink() {
        let rows = rows();
        let theme = Theme::default();
        let (list, _) = frame(
            &rows,
            GroupList::new(),
            Some(1),
            &mut GroupListState::new(),
            &mut at(-1.0, -1.0),
        );
        assert_eq!(
            text(&list, "Session context menu").color,
            text_color(theme.on_accent)
        );
        let s = StyleResolver::new(&theme);
        assert_eq!(
            text(&list, "@merry-tiger · 447 msgs").color,
            text_color(s.ink(Ink::OnAccentSecond))
        );
    }

    #[test]
    fn a_header_can_not_be_selected() {
        let rows = rows();
        let mut state = GroupListState::new();
        let (list, _) = frame(
            &rows,
            GroupList::new(),
            Some(0),
            &mut state,
            &mut at(-1.0, -1.0),
        );
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        assert_eq!(text(&list, "agent-ui").color, text_color(s.ink(Ink::Title)));
        assert_eq!(state.cursor(), None);
    }

    fn press(
        state: &mut GroupListState,
        rows: &[GroupRow],
        selected: Option<usize>,
        set: impl Fn(&mut InputState),
    ) -> GroupListOutput {
        let mut input = at(-1.0, -1.0);
        set(&mut input);
        let view = GroupList::new().focused(true);
        let (_, out) = frame(rows, view, selected, state, &mut input);
        // Release, so the next press is a new edge.
        frame(rows, view, selected, state, &mut at(-1.0, -1.0));
        out
    }

    #[test]
    fn the_keyboard_walks_every_row_and_acts_on_it() {
        let rows = rows();
        let mut state = GroupListState::new();
        let out = press(&mut state, &rows, None, |i| i.nav.down = true);
        assert_eq!(state.cursor(), Some(0));
        assert_eq!(out.select, None, "a header isn't selected");
        let out = press(&mut state, &rows, None, |i| i.nav.down = true);
        assert_eq!(out.select, Some(1), "landing on an item selects it");
        let out = press(&mut state, &rows, Some(1), |i| i.nav.confirm = true);
        assert_eq!(out.open, Some(1));
        let out = press(&mut state, &rows, Some(1), |i| i.nav.left = true);
        assert_eq!(state.cursor(), Some(0), "← jumps to the header");
        assert_eq!(out.toggle, None);
        let out = press(&mut state, &rows, Some(1), |i| i.nav.left = true);
        assert_eq!(out.toggle, Some(0), "← folds an open header");
        let out = press(&mut state, &rows, Some(1), |i| i.nav.right = true);
        assert_eq!(out.toggle, None, "→ on an open header does nothing");
        let out = press(&mut state, &rows, Some(1), |i| i.key_end = true);
        assert_eq!(state.cursor(), Some(4));
        assert_eq!(out.select, None);
        let out = press(&mut state, &rows, Some(1), |i| i.nav.right = true);
        assert_eq!(out.toggle, Some(4), "→ unfolds a closed header");
        press(&mut state, &rows, Some(1), |i| i.nav.up = true);
        let out = press(&mut state, &rows, Some(1), |i| i.nav.confirm = true);
        assert_eq!(out.more, Some(3));
    }

    #[test]
    fn a_rebuild_that_moves_the_rows_keeps_the_cursor_off_the_selection() {
        let rows = rows();
        let mut state = GroupListState::new();
        // Item 1 is selected; the cursor walks on to item 2.
        frame(
            &rows,
            GroupList::new(),
            Some(1),
            &mut state,
            &mut at(-1.0, -1.0),
        );
        press(&mut state, &rows, Some(1), |i| i.nav.down = true);
        assert_eq!(state.cursor(), Some(2));
        // A new item lands above both: the selection is now row 2, the
        // cursor's item row 3.
        let mut moved = rows.clone();
        moved.insert(1, GroupRow::Item(GroupItem::new("Newest")));
        state.rows_moved(Some(3), Some(2));
        frame(
            &moved,
            GroupList::new(),
            Some(2),
            &mut state,
            &mut at(-1.0, -1.0),
        );
        assert_eq!(state.cursor(), Some(3));
        // A real change of selection still moves the cursor to it.
        frame(
            &moved,
            GroupList::new(),
            Some(1),
            &mut state,
            &mut at(-1.0, -1.0),
        );
        assert_eq!(state.cursor(), Some(1));
    }

    #[test]
    fn an_unfocused_list_ignores_keys() {
        let rows = rows();
        let mut state = GroupListState::new();
        let mut input = at(-1.0, -1.0);
        input.nav.down = true;
        let (_, out) = frame(&rows, GroupList::new(), None, &mut state, &mut input);
        assert_eq!(out, GroupListOutput::default());
        assert_eq!(state.cursor(), None);
    }

    #[test]
    fn only_visible_rows_are_built() {
        use std::cell::Cell;
        let rows: Vec<GroupRow> = (0..10_000)
            .map(|_| GroupRow::Item(GroupItem::new("x")))
            .collect();
        let layout = layout(&rows);
        let built = Cell::new(0usize);
        let theme = Theme::default();
        let mut list = DrawList::new();
        GroupList::new().draw(
            RECT,
            &layout,
            None,
            &mut GroupListState::new(),
            &mut list,
            &StyleResolver::new(&theme),
            &mut at(-1.0, -1.0),
            |i| {
                built.set(built.get() + 1);
                rows[i]
            },
        );
        let visible = (RECT.height / GROUP_ITEM_HEIGHT).ceil() as usize + 1;
        assert!(built.get() <= visible, "{} rows built", built.get());
    }
}
