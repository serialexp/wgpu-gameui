//! Frame state, the activation state machine, and the measured geometry of the
//! open chain.
//!
//! Three things live here, in the order the frame uses them:
//!
//! 1. **Captured input + intent claiming** ([`MenuBarState::begin_frame`]). Every
//!    navigation intent the menu will act on is *zeroed in the shared
//!    [`InputState`]* at frame-top, so [`FocusState`] and any focused list or
//!    text field never see it. That is what stops one Escape from both closing a
//!    menu and blurring the focused field, and it is why `begin_frame` takes
//!    `&mut InputState`.
//! 2. **Trigger and bar-level traversal** — the Alt tap that arms the bar, and
//!    (during the bar's own draw, which is the only place with the menus and
//!    their label rects) left/right/down on the strip.
//! 3. **Measured chain geometry** ([`MenuBarState::collect_chain`]), staged a
//!    frame ahead so the popup layers exist before the base layer is drawn.

use crate::layout::Rect;
use crate::measure::{MeasureConstraints, MeasureContext, MeasuredText};
use crate::style::{StyleKey, StyleOverlay};
use crate::text::WrapMode;
use crate::{
    AnimationState, CursorState, DrawList, FocusState, FontSpec, InputState, InteractionScene,
    LayerStack, StyleResolver, Theme,
};

use super::model::{
    AccelPlatform, ActivatedItem, Menu, MenuBarId, MenuItem, MenuTrigger, SubmenuSide,
};
use super::paint;
use super::placement;

/// Literal Forge menu-sheet padding around the row stack.
const SHEET_PADDING: f32 = 3.0;
/// Horizontal row inset from the sheet edge.
const ROW_PADDING: f32 = 8.0;
/// Menu item typography from the handoff.
const ROW_FONT_SIZE: f32 = 11.5;
const HINT_FONT_SIZE: f32 = 10.0;
const CHECK_WIDTH: f32 = 10.0;
const CHEVRON_WIDTH: f32 = 8.0;
/// A separator is a 1px rule with 3px breathing room above and below.
const SEPARATOR_HEIGHT: f32 = 7.0;
/// Maximum number of simultaneously open columns. A parent at this depth still
/// renders, but cannot open another child.
pub const MAX_MENU_DEPTH: usize = 8;

/// The popup layers pushed for one frame's open chain.
///
/// A `Copy` token rather than a borrow of [`MenuBarState`]: the caller has to pass
/// `&mut MenuBarState` again to draw the columns, and a borrow would not compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuLayers {
    /// Layer index of the **viewport blocker**, which is what stops an outside
    /// press from reaching whatever is underneath the open chain.
    pub blocker: usize,
    /// Number of open column layers, occupying `blocker + 1 ..= blocker + count`.
    /// They are contiguous because they are pushed consecutively at frame-top.
    pub count: usize,
}

/// Everything each popup column's [`DrawContext`](crate::DrawContext) needs
/// except the draw list, which differs per level.
///
/// A multi-layer widget cannot take a single `&mut DrawContext` plus a
/// `&mut LayerStack` (a `DrawContext` owns one draw list), so the resources are
/// spelled out and a context is constructed per column internally.
pub struct MenuDrawEnv<'a> {
    /// Theme backing style resolution.
    pub theme: &'a Theme,
    /// Optional scoped style overlay, as on the base layer's context.
    pub style: Option<&'a StyleOverlay>,
    /// This frame's input. Pass the **raw** input, not the base layer's consumed
    /// copy: the blocker is what keeps the base layer from seeing a press, and
    /// the menu has to see it to dismiss itself.
    pub input: &'a InputState,
    /// Focus state, so a row click can be claimed instead of blurring the
    /// focused widget the item acts on.
    pub focus: &'a mut FocusState,
    /// Retained dispatch, used to resolve row hover/press. Required: without it a
    /// menu has no pointer support at all.
    pub interactions: &'a mut InteractionScene,
    /// Optional caller-owned animation clock.
    pub animations: Option<&'a mut AnimationState>,
    /// Optional caller-owned cursor accumulator.
    pub cursor: Option<&'a mut CursorState>,
    /// Viewport width in pixels.
    pub screen_width: f32,
    /// Viewport height in pixels.
    pub screen_height: f32,
}

/// One row of an open column: its measured text, consumed into paint.
///
/// The text is measured in the frame the geometry is collected and consumed in
/// the *next* frame's paint pass, so a row is one measurement and one
/// [`TextBlock`](crate::TextBlock) per frame rather than a measure-then-rebuild.
pub(super) struct RowGeom {
    /// Index of the item within its menu.
    pub item_index: usize,
    /// Vertical offset from the top of the row stack.
    pub y: f32,
    /// Painted and interactive height. Separators use the handoff's compact 7px
    /// wrapper rather than consuming a full menu row.
    pub height: f32,
    /// Separators carry no text.
    pub separator: bool,
    /// Label, measured unconstrained.
    pub label: Option<MeasuredText>,
    /// Intrinsic label width in pixels.
    pub label_w: f32,
    /// Hint, measured unconstrained.
    pub hint: Option<MeasuredText>,
    /// Whether the item opens a child column.
    pub submenu: bool,
    /// Whether the item is disabled (dimmed, never highlighted).
    pub disabled: bool,
    /// Whether the item draws a check mark.
    pub checked: bool,
}

impl RowGeom {
    fn separator(item_index: usize, y: f32) -> Self {
        Self {
            item_index,
            y,
            height: SEPARATOR_HEIGHT,
            separator: true,
            label: None,
            label_w: 0.0,
            hint: None,
            submenu: false,
            disabled: true,
            checked: false,
        }
    }
}

/// One open column's measured layout.
pub(super) struct ColumnGeom {
    /// The column's rect, already placed inside the viewport.
    pub rect: Rect,
    /// Height of one row.
    pub row_h: f32,
    /// Width of the check gutter.
    pub check_w: f32,
    /// Right edge of the hint column (uniform across rows).
    pub hint_right: f32,
    /// Width available to a label before it is ellipsized.
    pub label_avail: f32,
    /// Total content height, which may exceed `rect.height`.
    pub content_h: f32,
    /// Outer inset between the sheet edge and its row stack.
    pub sheet_padding: f32,
    /// One entry per item, in menu order, including separators.
    pub rows: Vec<RowGeom>,
    /// Index of the open top-level menu this column belongs to.
    pub menu_index: usize,
    /// Index path from the root column to this column.
    pub path: [usize; MAX_MENU_DEPTH],
}

/// The open chain's frame-invariant geometry: which menu is open, where the bar
/// is, and what viewport the chain was placed in.
///
/// The columns themselves live on the state (see
/// [`MenuBarState::columns`](MenuBarState)) so a frame's paint can take them out
/// and put them back without a borrow fight, and so a test can read back what was
/// last drawn.
#[derive(Clone, Copy)]
pub(super) struct MenuGeom {
    /// The bar strip's rect, so the blocker can exclude it.
    pub bar_rect: Rect,
    /// The label rect the root column hangs from.
    pub anchor: Rect,
    /// The viewport the chain must stay inside.
    pub viewport: Rect,
    /// Index of the open top-level menu.
    pub menu_index: usize,
    /// Accelerator vocabulary used while measuring row hints.
    pub platform: AccelPlatform,
    /// Preferred side for root and child columns.
    pub side: SubmenuSide,
}

/// What the geometry pass is asked to measure: the menus, where the bar and the
/// open label are, and the viewport the column has to stay inside — plus the
/// vocabulary (accelerator platform and flip side) the column's own layout depends
/// on.
///
/// Bundled because it is all "the request"; the pass's remaining arguments are the
/// environment it measures with.
#[derive(Clone, Copy)]
pub(super) struct ChainGeometry<'a> {
    /// The bar's menus, in bar order.
    pub menus: &'a [Menu<'a>],
    /// The bar strip's rect.
    pub bar_rect: Rect,
    /// The open menu's label rect: the column hangs off it.
    pub anchor: Rect,
    /// The viewport the chain must stay inside.
    pub viewport: Rect,
    /// Which platform's accelerator vocabulary to render.
    pub platform: AccelPlatform,
    /// Which side a column prefers when it does not fit.
    pub side: SubmenuSide,
}

/// Caller-owned menubar state: whether the bar is armed, which menu is open,
/// what is highlighted, where each column is scrolled, and the measured geometry
/// of the open chain.
///
/// Persists across frames; construct one per bar and thread `&mut` into
/// [`MenuBar::draw`](super::MenuBar::draw), the same way the crate threads
/// [`crate::DropdownState`] / [`crate::FocusState`].
pub struct MenuBarState {
    trigger: MenuTrigger,
    /// The bar this state belongs to, recorded by
    /// [`MenuBar::draw`](super::MenuBar::draw). It namespaces every hit region the
    /// chain registers, which is why the state and the widget must agree.
    pub(super) bar: MenuBarId,
    /// The bar is in "menu mode": arrows move a bar highlight, and the strip
    /// renders that highlight. A *mode*, not a held key.
    pub(super) armed: bool,
    /// The open top-level menu, if a chain is open.
    pub(super) open: Option<usize>,
    /// Which bar label the armed mode has highlighted.
    pub(super) highlighted_menu: Option<usize>,
    /// Which row of the root column is highlighted (kept as the compatibility
    /// view of `highlights[0]`).
    pub(super) highlighted_item: Option<usize>,
    /// Vertical scroll offset of the root column (the compatibility view of
    /// `scrolls[0]`).
    pub(super) scroll: f32,
    /// Parent-row indices selecting each child after the root column.
    pub(super) open_path: Vec<usize>,
    /// Per-level highlighted rows and vertical scrolling.
    pub(super) highlights: Vec<Option<usize>>,
    pub(super) scrolls: Vec<f32>,
    /// Number of attempts to open a child beyond [`MAX_MENU_DEPTH`].
    depth_truncations: usize,
    /// Previous pointer sample, used by the safe-triangle corridor while moving
    /// from a parent row toward its open child.
    pub(super) previous_pointer: Option<(f32, f32)>,
    /// Bar label under the pointer this frame.
    pub(super) hovered_menu: Option<usize>,
    /// Row under the pointer this frame.
    pub(super) hovered_item: Option<usize>,
    /// Whether the pointer moved since the previous frame. The highlight is only
    /// reclaimed from the keyboard when it actually moved, so a stationary
    /// pointer over the column doesn't fight arrow keys.
    pub(super) pointer_moved: bool,
    /// A menu-row interaction consumed this frame's press: it is handed to
    /// [`FocusState::claim_click`] so the item can act on the widget that was
    /// focused (Copy/Paste must not blur their target).
    pub(super) click_claimed: bool,
    /// Geometry promoted from the previous frame, used to push and paint this
    /// frame's popup layers.
    pub(super) geom: Option<MenuGeom>,
    /// The open columns, promoted by [`begin_frame`](Self::begin_frame) and
    /// repainted in place, so a caller (or a test) can read back what was drawn.
    pub(super) columns: Vec<ColumnGeom>,
    /// Geometry collected this frame, promoted at the next `begin_frame`. The
    /// one-frame delay is what lets [`push_open_layers`](Self::push_open_layers)
    /// know the column rects at frame-top, before anything is drawn.
    pub(super) next_geom: Option<MenuGeom>,
    /// The buffer [`next_geom`](Self::next_geom)'s columns are rebuilt in. After
    /// the promotion swap it holds the previous frame's painted columns, whose
    /// `Vec` capacity is then reused — so an open chain allocates nothing on the
    /// geometry path beyond the text content itself.
    next_columns: Vec<ColumnGeom>,
    /// The measured width of each bar label, in menu order. Retained so the bar's
    /// three passes share one buffer instead of allocating per frame.
    pub(super) label_pool: Vec<f32>,
    /// Scratch for one row's hint text, reused across rows and frames.
    hint_scratch: String,
    /// The bar strip's rect from the frame it was last drawn in.
    pub(super) bar_rect: Rect,
    /// The viewport from the frame the geometry was collected in.
    pub(super) viewport: Rect,

    // ---- this frame's captured edges (mirrors DropdownState / FocusState) ----
    pub(super) up: bool,
    pub(super) down: bool,
    pub(super) left: bool,
    pub(super) right: bool,
    pub(super) confirm: bool,
    pub(super) cancel: bool,
    pub(super) mouse_clicked: bool,
    pub(super) mouse_x: f32,
    pub(super) mouse_y: f32,
    /// The open menu as it stood at frame-top. The chain's keyboard handling only
    /// runs when this still matches `open`, so the press that *opened* a menu
    /// cannot also select a row of it.
    pub(super) was_open: Option<usize>,
    last_pointer: Option<(f32, f32)>,
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MenuBarState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuBarState")
            .field("trigger", &self.trigger)
            .field("armed", &self.armed)
            .field("open", &self.open)
            .field("highlighted_menu", &self.highlighted_menu)
            .field("highlighted_item", &self.highlighted_item)
            .field("scroll", &self.scroll)
            .field("promoted", &self.geom.is_some())
            .finish_non_exhaustive()
    }
}

impl MenuBarState {
    /// A fresh, disarmed bar with nothing open.
    pub fn new() -> Self {
        Self {
            trigger: MenuTrigger::AltTap,
            bar: 0,
            armed: false,
            open: None,
            highlighted_menu: None,
            highlighted_item: None,
            scroll: 0.0,
            open_path: Vec::with_capacity(MAX_MENU_DEPTH - 1),
            highlights: Vec::with_capacity(MAX_MENU_DEPTH),
            scrolls: Vec::with_capacity(MAX_MENU_DEPTH),
            depth_truncations: 0,
            previous_pointer: None,
            hovered_menu: None,
            hovered_item: None,
            pointer_moved: false,
            click_claimed: false,
            geom: None,
            columns: Vec::new(),
            next_geom: None,
            next_columns: Vec::new(),
            label_pool: Vec::new(),
            hint_scratch: String::new(),
            bar_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            viewport: Rect::new(0.0, 0.0, 0.0, 0.0),
            up: false,
            down: false,
            left: false,
            right: false,
            confirm: false,
            cancel: false,
            mouse_clicked: false,
            mouse_x: 0.0,
            mouse_y: 0.0,
            was_open: None,
            last_pointer: None,
        }
    }

    /// Configure what arms the bar. Alt is the default and the only trigger the
    /// crate's input model can currently detect.
    pub fn with_trigger(mut self, trigger: MenuTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    /// Change what arms the bar.
    pub fn set_trigger(&mut self, trigger: MenuTrigger) {
        self.trigger = trigger;
    }

    /// The trigger that arms this bar.
    pub fn trigger(&self) -> MenuTrigger {
        self.trigger
    }

    /// Whether the bar is armed — in menu mode, with a bar highlight visible.
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// How many levels of the chain are open.
    pub fn open_levels(&self) -> usize {
        self.open.map_or(0, |_| 1 + self.open_path.len())
    }

    /// The index of the open top-level menu, if a chain is open.
    pub fn open_menu(&self) -> Option<usize> {
        self.open
    }

    /// The bar label the armed mode has highlighted, if any.
    pub fn highlighted_menu(&self) -> Option<usize> {
        self.highlighted_menu
    }

    /// The highlighted row of the open column, if any.
    pub fn highlighted_item(&self) -> Option<usize> {
        self.highlighted_item
    }

    /// Select the row highlighted in an already-open menu.
    ///
    /// This is primarily useful to seed a static preview or restore retained
    /// navigation state. Invalid, separator, and disabled rows clear the
    /// highlight rather than creating an impossible interaction state.
    pub fn set_highlighted_item(&mut self, menus: &[Menu<'_>], item_index: Option<usize>) {
        self.highlighted_item = item_index.filter(|&index| {
            self.open
                .and_then(|menu_index| menus.get(menu_index))
                .and_then(|menu| menu.items().get(index))
                .is_some_and(|item| item.is_enabled())
        });
        if let Some(slot) = self.highlights.first_mut() {
            *slot = self.highlighted_item;
        }
    }

    /// Open a submenu path for a static preview or restored state. Every index
    /// must name an enabled submenu parent; invalid paths stop at the last valid
    /// level and return `false`.
    pub fn set_open_path(&mut self, menus: &[Menu<'_>], path: &[usize]) -> bool {
        let Some(menu) = self.open.and_then(|index| menus.get(index)) else {
            return false;
        };
        self.open_path.clear();
        self.highlights.truncate(1);
        self.scrolls.truncate(1);
        for &parent in path.iter().take(MAX_MENU_DEPTH - 1) {
            let level = self.open_path.len();
            if !self.open_child(menu, level, parent) {
                return false;
            }
        }
        if path.len() >= MAX_MENU_DEPTH {
            self.depth_truncations += 1;
            return false;
        }
        true
    }

    /// Whether the bar is armed or a chain is open.
    ///
    /// Hosts gate text/raw-key routing on this: the lib cannot suppress
    /// [`InputState::text_input`], so a host that feeds a game's raw keys while a
    /// menu is open has to check here first. (Navigation intents are already
    /// claimed by the menu and never reach other widgets.)
    pub fn wants_keyboard(&self) -> bool {
        self.armed || self.open.is_some()
    }

    /// Close every level and disarm.
    pub fn close(&mut self) {
        self.armed = false;
        self.open = None;
        self.highlighted_menu = None;
        self.highlighted_item = None;
        self.scroll = 0.0;
        self.open_path.clear();
        self.highlights.clear();
        self.scrolls.clear();
        self.geom = None;
        self.next_geom = None;
        // The measured columns (and the text they hold) belong to a chain that is
        // gone. The buffer itself is kept, so reopening reuses the allocation.
        self.columns.clear();
        self.previous_pointer = None;
        self.last_pointer = None;
    }

    /// Frame-top: promote last frame's measured geometry, capture this frame's
    /// edges, resolve the arming trigger, and **claim** the navigation intents
    /// the menu will act on.
    ///
    /// Claiming is the whole point of taking `&mut InputState`: zeroing the
    /// `nav` fields here means a consumer later in the frame — [`FocusState`], a
    /// focused list, the base layer — never sees an intent the menu handled, and
    /// it works without a new consumption flag because the menu runs first.
    ///
    /// Tab (`next`/`prev`) is deliberately **never** claimed: a menu is not a
    /// focus trap.
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.begin_frame_with_dt(input, 0.0);
    }

    /// Frame-top entry point accepting elapsed seconds for API compatibility.
    /// Submenu hover is immediate; `dt` is no longer used.
    pub fn begin_frame_with_dt(&mut self, input: &mut InputState, _dt: f32) {
        self.previous_pointer = self.last_pointer;
        // Promote last frame's measured geometry. With nothing open there is nothing
        // to promote — and nothing may survive either: a closed chain keeps no
        // measured columns (the buffers keep their capacity, though).
        if self.open.is_some() {
            self.geom = self.next_geom.take();
        } else {
            self.geom = None;
            self.next_geom = None;
        }
        // Promote the staged columns, and keep the painted ones as the buffer the
        // next collection rebuilds in (so its `Vec`s are reused, not reallocated).
        std::mem::swap(&mut self.columns, &mut self.next_columns);
        if self.open.is_none() {
            self.columns.clear();
        }
        self.was_open = self.open;
        self.click_claimed = false;
        self.hovered_menu = None;
        self.hovered_item = None;

        let pointer = (input.mouse_x, input.mouse_y);
        self.pointer_moved = self.last_pointer != Some(pointer);
        self.last_pointer = Some(pointer);
        self.mouse_x = input.mouse_x;
        self.mouse_y = input.mouse_y;
        self.mouse_clicked = input.mouse_clicked;

        // The arming trigger. Alt carries a press edge precisely because a bare
        // tap can land entirely between two frames; the release is what makes it
        // a *tap*, so it deliberately does nothing (the bar stays armed).
        if matches!(self.trigger, MenuTrigger::AltTap) && input.alt_pressed {
            self.toggle_arm();
        }

        // Capture and claim. A disarmed bar with nothing open owns no keyboard:
        // Escape must still blur a focused widget, and arrows must still move it.
        let owns_keyboard = self.armed || self.open.is_some() || input.alt_pressed;
        self.up = owns_keyboard && input.nav.up;
        self.down = owns_keyboard && input.nav.down;
        self.left = owns_keyboard && input.nav.left;
        self.right = owns_keyboard && input.nav.right;
        self.confirm = owns_keyboard && input.nav.confirm;
        self.cancel = owns_keyboard && input.nav.cancel;
        if owns_keyboard {
            input.nav.up = false;
            input.nav.down = false;
            input.nav.left = false;
            input.nav.right = false;
            input.nav.confirm = false;
            input.nav.cancel = false;
        }

        // Escape with the bar armed but nothing open leaves menu mode. Escape
        // that unwinds a level is handled where the columns are drawn, and Escape
        // with nothing going on is not claimed at all.
        if self.armed && self.open.is_none() && self.cancel {
            self.disarm();
        }
    }

    /// Resolve dismissal, and tell [`FocusState`] that the menu's own pointer
    /// gestures were not clicks elsewhere.
    ///
    /// Call once per frame, after the base layer and the columns are drawn, and
    /// **before** `FocusState::end_frame`.
    pub fn end_frame(&mut self, focus: &mut FocusState) {
        if self.click_claimed {
            // A row (or a bar label, or the column's own padding) consumed the
            // press, so the widget it acts on keeps focus.
            focus.claim_click();
        }
        if self.mouse_clicked && !self.click_claimed && (self.armed || self.open.is_some()) {
            // An outside press. It is swallowed by the blocker layer — the base
            // layer never saw it — so it must not *also* read as a click
            // elsewhere and blur the focused widget.
            focus.claim_click();
            self.close();
        }
    }

    /// After the base UI: draw the open chain into the layers pushed by
    /// [`push_open_layers`](Self::push_open_layers), register each column's hit
    /// regions, and resolve activation.
    ///
    /// This is the **single** activation channel: rows live in popup layers and
    /// are resolved here, later in the frame than the bar, so
    /// [`MenuBar::draw`](super::MenuBar::draw) reports only bar-level facts.
    ///
    /// Returns `None` when nothing is drawn — nothing open, or the frame a menu
    /// opened (its geometry is measured after the bar draws and promoted next
    /// frame).
    pub fn draw_open_layers<'a>(
        &mut self,
        layers: &mut LayerStack,
        slots: Option<MenuLayers>,
        menus: &'a [Menu<'a>],
        env: &mut MenuDrawEnv<'_>,
    ) -> Option<ActivatedItem<'a>> {
        let bar = self.bar;
        paint::draw_columns(self, bar, menus, layers, slots, env)
    }

    /// Frame-top: push one popup layer per open column, plus the viewport
    /// blocker, returning a `Copy` token naming the range.
    ///
    /// Call before [`LayerStack::input_for_base`], which is what makes an
    /// outside press miss the widgets underneath. Each layer is pushed and
    /// immediately popped so the stack stays balanced while the layer remains in
    /// the stack for input resolution and can be drawn into by index.
    ///
    /// Returns `None` when nothing is open, or on the very frame a menu opens
    /// (its geometry is only known after the bar draws) — the same one-frame
    /// popup latency [`crate::DropdownState`] has.
    pub fn push_open_layers(&mut self, layers: &mut LayerStack) -> Option<MenuLayers> {
        self.open?;
        let geom = self.geom.as_ref()?;
        let blocker = layers.push_popup(geom.viewport);
        layers.pop_layer();
        for column in &self.columns {
            let index = layers.push_popup(column.rect);
            layers.pop_layer();
            debug_assert!(index > blocker, "columns must be pushed above the blocker");
        }
        Some(MenuLayers {
            blocker,
            count: self.columns.len(),
        })
    }

    /// Measure the open chain and stage it for the next frame's popup layers and
    /// paint pass.
    ///
    /// Runs while the bar is drawn (that is where the label rects and the draw
    /// list's text measurer are), and is skipped when nothing is open.
    pub(super) fn collect_chain(
        &mut self,
        geom: ChainGeometry<'_>,
        list: &mut DrawList,
        theme: &Theme,
        style: Option<&StyleOverlay>,
    ) {
        let ChainGeometry {
            menus,
            bar_rect,
            anchor,
            viewport,
            platform,
            side,
        } = geom;
        self.next_geom = None;
        self.bar_rect = bar_rect;
        self.viewport = viewport;
        let Some(menu_index) = self.open else {
            return;
        };
        let Some(menu) = menus.get(menu_index) else {
            // The bar shrank under the open menu: drop the chain rather than
            // index past it.
            self.close();
            return;
        };

        let styles = StyleResolver::with_overlay_opt(theme, style);
        let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
        let gap = styles.scalar(StyleKey::MenuAccelGap).max(0.0);
        let min_width = styles.scalar(StyleKey::MenuItemMinWidth).max(0.0);
        let check_w = CHECK_WIDTH;
        let chevron_w = CHEVRON_WIDTH;
        let font = FontSpec {
            font: theme.font.clone(),
            size: ROW_FONT_SIZE,
            ..FontSpec::default()
        };
        let mono = crate::bundled_mono_font(&list.text_measurer_mut().font_system_handle());

        // One measuring pass over the column. `MeasureConstraints::UNBOUNDED`
        // keeps every measurement at its intrinsic size, which is what the
        // column width is derived from, and keeps the measured block byte-identical
        // to the one painted next frame — the shape cache keys on `max_width`, so
        // re-configuring a block between measure and paint would re-shape it every
        // frame.
        let mut cx = MeasureContext::new(
            list.text_measurer_mut(),
            styles,
            font,
            MeasureConstraints::UNBOUNDED,
            1.0,
            WrapMode::None,
        );

        // Rebuild every open level while retaining each level's row allocation.
        let mut row_buffers: [Vec<RowGeom>; MAX_MENU_DEPTH] = std::array::from_fn(|level| {
            self.next_columns
                .get_mut(level)
                .map(|column| std::mem::take(&mut column.rows))
                .unwrap_or_default()
        });
        self.next_columns.clear();
        let levels = self.open_levels().min(MAX_MENU_DEPTH);
        let mut items = menu.items();
        let mut path = [usize::MAX; MAX_MENU_DEPTH];
        for level in 0..levels {
            if level > 0 {
                let parent = self.open_path[level - 1];
                path[level - 1] = parent;
                let Some(parent_item) = items.get(parent).filter(|item| item.is_submenu()) else {
                    self.open_path.truncate(level - 1);
                    self.highlights.truncate(level);
                    self.scrolls.truncate(level);
                    break;
                };
                items = parent_item.children();
            }

            let rows = &mut row_buffers[level];
            rows.clear();
            let mut label_max = 0.0f32;
            let mut hint_max = 0.0f32;
            let mut any_hint = false;
            let mut any_submenu = false;
            let mut content_h = 0.0;
            for (item_index, item) in items.iter().enumerate() {
                if item.is_separator() {
                    rows.push(RowGeom::separator(item_index, content_h));
                    content_h += SEPARATOR_HEIGHT;
                    continue;
                }
                let label = cx.measure_text(cx.text_block(item.label()));
                let label_w = label.metrics.size[0];
                let hint = if item.accelerator().is_some() || item.shortcut_text().is_some() {
                    self.hint_scratch.clear();
                    item.write_hint(platform, &mut self.hint_scratch);
                    let block = cx
                        .text_block(self.hint_scratch.as_str())
                        .with_size(HINT_FONT_SIZE)
                        .with_font_opt(mono.clone());
                    let measured = cx.measure_text(block);
                    hint_max = hint_max.max(measured.metrics.size[0]);
                    any_hint = true;
                    Some(measured)
                } else {
                    None
                };
                label_max = label_max.max(label_w);
                any_submenu |= item.is_submenu();
                rows.push(RowGeom {
                    item_index,
                    y: content_h,
                    height: row_h,
                    separator: false,
                    label: Some(label),
                    label_w,
                    hint,
                    submenu: item.is_submenu() && level + 1 < MAX_MENU_DEPTH,
                    disabled: !item.is_enabled(),
                    checked: item.is_checked(),
                });
                content_h += row_h;
            }

            let hint_area = if any_hint { gap + hint_max } else { 0.0 };
            let chevron_reserve = if any_submenu { gap + chevron_w } else { 0.0 };
            let intrinsic = SHEET_PADDING * 2.0
                + ROW_PADDING * 2.0
                + check_w
                + gap
                + label_max
                + hint_area
                + chevron_reserve;
            let width = intrinsic.max(min_width).min(viewport.width).max(1.0);
            let full_height = content_h + SHEET_PADDING * 2.0;
            let height = full_height
                .min(viewport.height)
                .max(viewport.height.min(row_h + SHEET_PADDING * 2.0));
            let rect = if level == 0 {
                placement::place_popup(anchor, [width, height], viewport, side).0
            } else {
                let parent = &self.next_columns[level - 1];
                let parent_index = self.open_path[level - 1];
                let parent_row = parent
                    .rows
                    .iter()
                    .find(|row| row.item_index == parent_index);
                let Some(parent_row) = parent_row else { break };
                let scroll = self.scrolls.get(level - 1).copied().unwrap_or(0.0);
                let row_rect = Rect::new(
                    parent.rect.x,
                    parent.rect.y + parent.sheet_padding + parent_row.y - scroll,
                    parent.rect.width,
                    parent_row.height,
                );
                placement::place_submenu(row_rect, [width, height], viewport, side).0
            };
            let content_left = rect.x + SHEET_PADDING + ROW_PADDING;
            let content_right = rect.right() - SHEET_PADDING - ROW_PADDING;
            let hint_right = content_right - chevron_reserve;
            let label_x = content_left + check_w + gap;
            self.next_columns.push(ColumnGeom {
                rect,
                row_h,
                check_w,
                hint_right,
                label_avail: (hint_right - hint_area - label_x).max(0.0),
                content_h,
                sheet_padding: SHEET_PADDING,
                rows: std::mem::take(rows),
                menu_index,
                path,
            });
        }
        drop(cx);
        self.next_geom = Some(MenuGeom {
            bar_rect,
            anchor,
            viewport,
            menu_index,
            platform,
            side,
        });
    }

    /// Rebuild the open chain after popup interaction changes its path.
    ///
    /// The normal bar pass stages geometry before popup rows resolve. Calling
    /// this from the popup pass makes a newly hovered/clicked child available
    /// immediately and stages the same complete chain for the following frame.
    pub(super) fn recollect_open_chain(
        &mut self,
        menus: &[Menu<'_>],
        list: &mut DrawList,
        theme: &Theme,
        style: Option<&StyleOverlay>,
        geom: MenuGeom,
    ) {
        self.stage_open_chain(menus, list, theme, style, geom);
        std::mem::swap(&mut self.columns, &mut self.next_columns);
        // Promotion consumed the freshly measured buffer. Stage the same path
        // once more so the following frame promotes identical geometry instead
        // of briefly reverting to the pre-interaction chain.
        self.stage_open_chain(menus, list, theme, style, geom);
    }

    /// Stage the current path without promoting it into this frame's columns.
    pub(super) fn stage_open_chain(
        &mut self,
        menus: &[Menu<'_>],
        list: &mut DrawList,
        theme: &Theme,
        style: Option<&StyleOverlay>,
        geom: MenuGeom,
    ) {
        self.collect_chain(
            ChainGeometry {
                menus,
                bar_rect: geom.bar_rect,
                anchor: geom.anchor,
                viewport: geom.viewport,
                platform: geom.platform,
                side: geom.side,
            },
            list,
            theme,
            style,
        );
    }

    /// The geometry of the column last painted, as `(rect, row height, content
    /// height)`. `None` when no column has been painted since the chain opened.
    ///
    /// Exposed for tests and gallery tooling: the geometry is deliberately kept
    /// around after a frame so a caller can reason about what was drawn without
    /// re-deriving it.
    #[doc(hidden)]
    pub fn debug_geometry(&self) -> Option<(Rect, f32, f32)> {
        let column = self.columns.first()?;
        Some((column.rect, column.row_h, column.content_h))
    }

    /// The open column's vertical scroll offset. Exposed for tests.
    #[doc(hidden)]
    pub fn debug_scroll(&self) -> f32 {
        self.scroll
    }

    /// The measured width of each bar label. Exposed for tests.
    #[doc(hidden)]
    pub fn debug_label_widths(&self) -> &[f32] {
        &self.label_pool
    }

    /// Return the retained capacity of the geometry scratch buffers as the
    /// column-vector capacity and one row-vector capacity per possible level.
    ///
    /// Exposed so allocation-reuse tests can assert that the whole open chain,
    /// rather than only its root, keeps its buffers across frames. The fixed-size
    /// array avoids allocating merely to inspect the hot path.
    #[doc(hidden)]
    pub fn scratch_capacities(&self) -> (usize, [usize; MAX_MENU_DEPTH]) {
        let columns = self.columns.capacity().max(self.next_columns.capacity());
        let rows = std::array::from_fn(|level| {
            self.columns
                .get(level)
                .map_or(0, |column| column.rows.capacity())
                .max(
                    self.next_columns
                        .get(level)
                        .map_or(0, |column| column.rows.capacity()),
                )
        });
        (columns, rows)
    }

    /// Number of child-open attempts truncated at [`MAX_MENU_DEPTH`].
    pub fn depth_truncations(&self) -> usize {
        self.depth_truncations
    }

    // ---- internal transitions -------------------------------------------------

    /// The arming trigger was tapped: close a chain, disarm, or arm.
    fn toggle_arm(&mut self) {
        if self.open.is_some() {
            // A tap while a chain is open closes the whole chain and leaves menu
            // mode — the tap is a toggle of the *mode*, not of one level.
            self.close();
        } else if self.armed {
            self.disarm();
        } else {
            self.armed = true;
            // Resolved when the bar draws, which is where the menus' enabled
            // flags are known.
            self.highlighted_menu = None;
        }
    }

    /// Leave menu mode without touching an open chain's state (that is
    /// [`close`](Self::close)'s job).
    pub(super) fn disarm(&mut self) {
        self.armed = false;
        self.highlighted_menu = None;
    }

    /// Walk the bar from `from` by `delta`, wrapping and skipping disabled menus.
    pub(super) fn step_menu(&self, menus: &[Menu<'_>], from: usize, delta: isize) -> Option<usize> {
        let count = menus.len();
        if count == 0 {
            return None;
        }
        let mut index = from.min(count - 1);
        for _ in 0..count {
            index = ((index as isize + delta).rem_euclid(count as isize)) as usize;
            if menus[index].is_enabled() {
                return Some(index);
            }
        }
        None
    }

    /// The first enabled menu, for arming and for recovering a stale highlight.
    pub(super) fn first_enabled_menu(&self, menus: &[Menu<'_>]) -> Option<usize> {
        menus.iter().position(Menu::is_enabled)
    }

    /// Open `menu_index`, highlighting its first enabled item. Opening is what
    /// puts the bar into menu mode (a bar click enters it too, not only the
    /// trigger tap).
    pub(super) fn open_menu_at(&mut self, menus: &[Menu<'_>], menu_index: usize) {
        let Some(menu) = menus.get(menu_index) else {
            return;
        };
        if !menu.is_enabled() {
            return;
        }
        self.armed = true;
        self.open = Some(menu_index);
        self.highlighted_menu = Some(menu_index);
        self.highlighted_item = menu.first_enabled_item();
        self.scroll = 0.0;
        self.open_path.clear();
        self.highlights.clear();
        self.highlights.push(self.highlighted_item);
        self.scrolls.clear();
        self.scrolls.push(0.0);
        // A column hangs off its label: moving the chain to another menu
        // invalidates the measured columns, which the bar re-measures during the
        // same frame's draw.
        self.geom = None;
        self.next_geom = None;
        self.columns.clear();
    }

    /// Move an open chain to the previous/next enabled menu. Returns whether it
    /// moved.
    ///
    /// At the top level, left/right walk the bar rather than closing the level: the
    /// chain stays open, which is the mirror of "right on a leaf moves to the next
    /// top-level menu". With only one enabled menu there is nothing to move to —
    /// re-opening the same menu would reset its highlight for no reason.
    ///
    /// Called from the bar's own draw, which runs before the chain's geometry pass
    /// — a switch has to land before the new column is measured, or the chain would
    /// spend a second frame with nothing to paint.
    pub(super) fn switch_menu(&mut self, menus: &[Menu<'_>], delta: isize) -> bool {
        let Some(current) = self.open else {
            return false;
        };
        let from = self.highlighted_menu.unwrap_or(current);
        let Some(target) = self.step_menu(menus, from, delta) else {
            return false;
        };
        if target == current {
            return false;
        }
        self.open_menu_at(menus, target);
        true
    }

    pub(super) fn items_at_level<'a>(
        &self,
        menu: &'a Menu<'a>,
        level: usize,
    ) -> Option<&'a [MenuItem<'a>]> {
        let mut items = menu.items();
        for &parent in self.open_path.iter().take(level) {
            items = items.get(parent)?.children();
        }
        Some(items)
    }

    pub(super) fn open_child(&mut self, menu: &Menu<'_>, level: usize, parent: usize) -> bool {
        if level + 1 >= MAX_MENU_DEPTH {
            self.depth_truncations += 1;
            return false;
        }
        let Some(children) = self
            .items_at_level(menu, level)
            .and_then(|items| items.get(parent))
            .map(MenuItem::children)
            .filter(|items| !items.is_empty())
        else {
            return false;
        };
        self.open_path.truncate(level);
        self.open_path.push(parent);
        self.highlights.truncate(level + 1);
        self.highlights
            .push(children.iter().position(MenuItem::is_enabled));
        self.scrolls.truncate(level + 1);
        self.scrolls.push(0.0);
        true
    }

    pub(super) fn unwind_level(&mut self) -> bool {
        if self.open_path.pop().is_some() {
            self.highlights.truncate(1 + self.open_path.len());
            self.scrolls.truncate(1 + self.open_path.len());
            self.highlighted_item = self.highlights.first().copied().flatten();
            self.scroll = self.scrolls.first().copied().unwrap_or(0.0);
            true
        } else {
            false
        }
    }

    /// Move one level's row highlight, wrapping and skipping unavailable rows.
    pub(super) fn step_item(&mut self, items: &[MenuItem<'_>], level: usize, delta: isize) -> bool {
        let count = items.len();
        if count == 0 {
            return false;
        }
        let from = self.highlights.get(level).copied().flatten().unwrap_or(0);
        let mut index = from;
        for _ in 0..count {
            index = ((index as isize + delta).rem_euclid(count as isize)) as usize;
            if items[index].is_enabled() {
                if let Some(slot) = self.highlights.get_mut(level) {
                    *slot = Some(index);
                }
                if level == 0 {
                    self.highlighted_item = Some(index);
                }
                self.open_path.truncate(level);
                self.highlights.truncate(level + 1);
                self.scrolls.truncate(level + 1);
                return true;
            }
        }
        false
    }

    /// Keep one level's highlight inside the column's visible band.
    pub(super) fn scroll_into_view(&mut self, column: &ColumnGeom, level: usize) {
        let Some(index) = self.highlights.get(level).copied().flatten() else {
            return;
        };
        let visible_height = (column.rect.height - column.sheet_padding * 2.0).max(0.0);
        let max_scroll = (column.content_h - visible_height).max(0.0);
        let Some(row) = column.rows.iter().find(|row| row.item_index == index) else {
            return;
        };
        let top = row.y;
        let bottom = top + row.height;
        let scroll = self
            .scrolls
            .get_mut(level)
            .expect("open level has scroll state");
        if top < *scroll {
            *scroll = top;
        } else if bottom > *scroll + visible_height {
            *scroll = bottom - visible_height;
        }
        *scroll = scroll.clamp(0.0, max_scroll);
        if level == 0 {
            self.scroll = *scroll;
        }
    }
}

/// The activation id of `item` within `menu`: its explicit id, else one derived
/// from the label path `(menu label, item label)`.
///
/// Derived ids exist so an item the caller never named still has a stable
/// identity to report; explicit ids are what a caller can match on in advance.
pub(super) fn activation_id_for_path(menu: &Menu<'_>, path: &[usize], item: &MenuItem<'_>) -> u64 {
    let mut labels = [""; MAX_MENU_DEPTH + 1];
    labels[0] = menu.label();
    let mut items = menu.items();
    let mut count = 1;
    for &step in path {
        let Some(parent) = items.get(step) else { break };
        labels[count] = parent.label();
        count += 1;
        items = parent.children();
    }
    labels[count] = item.label();
    item.activation_id(&labels[..=count])
}

#[cfg(test)]
pub(super) fn activation_id(menu: &Menu<'_>, item: &MenuItem<'_>) -> u64 {
    activation_id_for_path(menu, &[], item)
}
