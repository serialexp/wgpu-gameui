//! Cursor-anchored context menus using the shared [`MenuItem`](crate::MenuItem)
//! model and menu-sheet visual language.

use crate::color::rgb8;
use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{
    CornerRadii, Edge, EdgeWidths, FocusState, InputState, LayerStack, StyleKey, StyleResolver,
    SurfacePainter,
};

use super::DrawList;
use super::menubar::{
    AccelPlatform, ActivatedItem, MAX_MENU_DEPTH, MenuItem, SubmenuSide, place_submenu,
};

const SHEET_PADDING: f32 = 3.0;
const ROW_PADDING: f32 = 8.0;
const CHECK_WIDTH: f32 = 10.0;
const CHEVRON_WIDTH: f32 = 8.0;
const SEPARATOR_HEIGHT: f32 = 7.0;
const FONT_SIZE: f32 = 11.5;
const HINT_FONT_SIZE: f32 = 10.0;

/// A borrowed context-menu description. It deliberately reuses [`MenuItem`]
/// rather than introducing another action/separator/shortcut model.
#[derive(Clone, Copy, Debug)]
pub struct ContextMenu<'a> {
    items: &'a [MenuItem<'a>],
    platform: AccelPlatform,
}

impl<'a> ContextMenu<'a> {
    /// Describe a context menu with `items`.
    pub const fn new(items: &'a [MenuItem<'a>]) -> Self {
        Self {
            items,
            platform: AccelPlatform::Pc,
        }
    }

    /// Select how accelerator hints are formatted.
    pub const fn with_platform(mut self, platform: AccelPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// The borrowed menu items.
    pub fn items(&self) -> &'a [MenuItem<'a>] {
        self.items
    }
}

/// Persistent state for one context-menu surface.
#[derive(Debug, Default, Clone)]
pub struct ContextMenuState {
    anchor: [f32; 2],
    open: bool,
    highlighted: Option<usize>,
    rect: Option<Rect>,
    open_path: Vec<usize>,
    highlights: Vec<Option<usize>>,
    rects: Vec<Rect>,
    hint_scratch: String,
    previous_pointer: Option<(f32, f32)>,
    last_pointer: Option<(f32, f32)>,
    depth_truncations: usize,
    /// The row height the open columns were last laid out with.
    row_h: f32,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    confirm: bool,
    cancel: bool,
}

impl ContextMenuState {
    /// Construct closed state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open (or reposition) the menu at a screen-space pointer position.
    pub fn open_at(&mut self, x: f32, y: f32) {
        self.anchor = [x, y];
        self.open = true;
        self.highlighted = None;
        self.rect = None;
        self.open_path.clear();
        self.highlights.clear();
        self.highlights.push(None);
        self.rects.clear();
        self.previous_pointer = None;
        self.last_pointer = None;
    }

    /// Close the menu and discard its transient selection/geometry.
    pub fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
        self.rect = None;
        self.open_path.clear();
        self.highlights.clear();
        self.rects.clear();
        self.previous_pointer = None;
        self.last_pointer = None;
    }

    /// Whether the menu is currently open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Current placed sheet rectangle, once measured this frame.
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }

    /// Number of currently open context-menu columns.
    pub fn open_levels(&self) -> usize {
        usize::from(self.open) + self.open_path.len()
    }

    /// Seed a validated submenu path for a static preview or restored state.
    /// Returns `false` when the menu is closed, a path step is invalid, or the
    /// requested path exceeds [`MAX_MENU_DEPTH`].
    pub fn set_open_path(&mut self, menu: &ContextMenu<'_>, path: &[usize]) -> bool {
        if !self.open {
            return false;
        }
        self.open_path.clear();
        self.highlights.clear();
        self.highlights.push(None);
        let mut items = menu.items;
        for &parent in path.iter().take(MAX_MENU_DEPTH - 1) {
            let Some(item) = items
                .get(parent)
                .filter(|item| item.is_enabled() && item.is_submenu())
            else {
                return false;
            };
            self.open_path.push(parent);
            items = item.children();
            self.highlights
                .push(items.iter().position(MenuItem::is_enabled));
        }
        if path.len() >= MAX_MENU_DEPTH {
            self.depth_truncations += 1;
            return false;
        }
        true
    }

    /// Number of child-open attempts truncated at [`MAX_MENU_DEPTH`].
    pub fn depth_truncations(&self) -> usize {
        self.depth_truncations
    }

    /// The [`reason`](MenuItem::reason) of the disabled item under the point
    /// `(x, y)` in an open column, with the item's row rect, so the host can
    /// show it as a tooltip. `menu` must be the menu last drawn. `None` when
    /// the menu is closed or the point isn't on a disabled item with a reason.
    pub fn hovered_reason<'m>(
        &self,
        menu: &ContextMenu<'m>,
        x: f32,
        y: f32,
    ) -> Option<(&'m str, Rect)> {
        if !self.open || self.row_h <= 0.0 {
            return None;
        }
        let mut items = menu.items;
        for (level, rect) in self.rects.iter().enumerate() {
            if level > 0 {
                items = items.get(*self.open_path.get(level - 1)?)?.children();
            }
            if !rect.contains(x, y) {
                continue;
            }
            let mut top = rect.y + SHEET_PADDING;
            for item in items {
                let height = if item.is_separator() {
                    SEPARATOR_HEIGHT
                } else {
                    self.row_h
                };
                let row = Rect::new(
                    rect.x + SHEET_PADDING,
                    top,
                    rect.width - SHEET_PADDING * 2.0,
                    height,
                );
                if row.contains(x, y) {
                    return item.disabled_reason().map(|reason| (reason, row));
                }
                top += height;
            }
            return None;
        }
        None
    }

    /// Capture navigation intents used by an open menu. Call before focus and
    /// base widgets consume the shared input. This compatibility entry point does
    /// not advance timed hover intent.
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.begin_frame_with_dt(input, 0.0);
    }

    /// Capture input while retaining the elapsed-time parameter for API
    /// compatibility. Submenu hover is immediate; `dt` is no longer used.
    pub fn begin_frame_with_dt(&mut self, input: &mut InputState, _dt: f32) {
        self.previous_pointer = self.last_pointer;
        self.last_pointer = Some((input.mouse_x, input.mouse_y));
        self.up = false;
        self.down = false;
        self.left = false;
        self.right = false;
        self.confirm = false;
        self.cancel = false;
        if !self.open {
            return;
        }
        self.up = input.nav.up;
        self.down = input.nav.down;
        self.left = input.nav.left;
        self.right = input.nav.right;
        self.confirm = input.nav.confirm;
        self.cancel = input.nav.cancel;
        input.nav.up = false;
        input.nav.down = false;
        input.nav.left = false;
        input.nav.right = false;
        input.nav.confirm = false;
        input.nav.cancel = false;
    }

    /// Push the context menu's input-blocking layer before base input dispatch.
    /// Returns its layer index, or `None` while closed.
    pub fn push_open_layer(
        &mut self,
        layers: &mut LayerStack,
        menu: &ContextMenu<'_>,
        styles: &StyleResolver<'_>,
        viewport: Rect,
    ) -> Option<usize> {
        if !self.open {
            return None;
        }
        let size = measure_items(
            layers.base_mut(),
            menu.items,
            menu.platform,
            styles,
            viewport.width,
            &mut self.hint_scratch,
        );
        let rect = place_context_menu(self.anchor, size, viewport);
        self.rect = Some(rect);
        self.rects.clear();
        self.rects.push(rect);
        let index = layers.push_modal(viewport);
        layers.pop_layer();
        Some(index)
    }

    /// Paint and interact with the open menu. Returns exactly one activation and
    /// closes on activation, Escape, or a primary/secondary click outside.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_open_layer<'m>(
        &mut self,
        layers: &mut LayerStack,
        layer: Option<usize>,
        menu: &'m ContextMenu<'m>,
        styles: &StyleResolver<'_>,
        input: &InputState,
        focus: &mut FocusState,
        viewport: Rect,
    ) -> Option<ActivatedItem<'m>> {
        if !self.open {
            return None;
        }
        let index = match layer {
            Some(index) => index,
            None => {
                let size = measure_items(
                    layers.base_mut(),
                    menu.items,
                    menu.platform,
                    styles,
                    viewport.width,
                    &mut self.hint_scratch,
                );
                let rect = place_context_menu(self.anchor, size, viewport);
                self.rect = Some(rect);
                self.rects.clear();
                self.rects.push(rect);
                let index = layers.push_modal(viewport);
                layers.pop_layer();
                index
            }
        };
        self.rect.expect("open context menu has measured geometry");
        let layer_input = layers.input_for_layer(index, input);

        if (self.cancel && self.open_path.is_empty())
            || ((layer_input.mouse_clicked || layer_input.mouse_right_clicked)
                && !self
                    .rects
                    .iter()
                    .any(|rect| rect.contains(layer_input.mouse_x, layer_input.mouse_y)))
        {
            self.close();
            return None;
        }

        let level = self.open_path.len();
        let mut items = menu.items;
        for &parent in &self.open_path {
            items = items[parent].children();
        }
        while self.highlights.len() <= level {
            self.highlights.push(None);
        }
        step_highlight(&mut self.highlights[level], items, self.up, self.down);
        if level == 0 {
            self.highlighted = self.highlights[0];
        }

        if (self.left || self.cancel) && level > 0 {
            self.open_path.pop();
            self.highlights.truncate(level);
            self.rects.truncate(level);
            return None;
        }
        let selected = self.highlights[level];
        if (self.right || self.confirm)
            && let Some(parent) = selected
            && items.get(parent).is_some_and(MenuItem::is_submenu)
        {
            if level + 1 < MAX_MENU_DEPTH {
                self.open_path.push(parent);
                self.highlights.push(
                    items[parent]
                        .children()
                        .iter()
                        .position(MenuItem::is_enabled),
                );
            } else {
                self.depth_truncations += 1;
            }
        }

        let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
        self.row_h = row_h;
        layout_context_chain(self, layers, index, menu, styles, viewport, row_h);
        let keyboard_handled =
            self.up || self.down || self.left || self.right || self.confirm || self.cancel;
        if !keyboard_handled && reconcile_context_pointer(self, menu, &layer_input, row_h) {
            layout_context_chain(self, layers, index, menu, styles, viewport, row_h);
        }

        let mut clicked = None;
        let mut draw_items = menu.items;
        for draw_level in 0..=self.open_path.len() {
            if draw_level > 0 {
                let parent = self.open_path[draw_level - 1];
                draw_items = draw_items[parent].children();
            }
            let draw_rect = self.rects[draw_level];
            let mut y = draw_rect.y + SHEET_PADDING;
            for (item_index, item) in draw_items.iter().enumerate() {
                let height = if item.is_separator() {
                    SEPARATOR_HEIGHT
                } else {
                    row_h
                };
                let row = Rect::new(
                    draw_rect.x + SHEET_PADDING,
                    y,
                    draw_rect.width - SHEET_PADDING * 2.0,
                    height,
                );
                if item.is_enabled()
                    && row.contains(layer_input.mouse_x, layer_input.mouse_y)
                    && layer_input.mouse_clicked
                {
                    clicked = Some((draw_level, item_index));
                }
                y += height;
            }
            paint_items(
                &mut layers.layers_mut()[index].list,
                draw_rect,
                draw_items,
                menu.platform,
                styles,
                self.highlights[draw_level],
            );
        }

        let chosen = clicked.or_else(|| self.confirm.then_some((level, selected?)));
        let (chosen_level, item_index) = chosen?;
        let mut chosen_items = menu.items;
        for &parent in self.open_path.iter().take(chosen_level) {
            chosen_items = chosen_items[parent].children();
        }
        let item = chosen_items.get(item_index)?;
        if !item.is_enabled() {
            return None;
        }
        if item.is_submenu() {
            self.open_path.truncate(chosen_level);
            self.open_path.push(item_index);
            self.highlights.truncate(chosen_level + 1);
            self.highlights
                .push(item.children().iter().position(MenuItem::is_enabled));
            return None;
        }
        let mut labels = [""; MAX_MENU_DEPTH];
        let mut path_items = menu.items;
        let mut count = 0;
        for &parent in self.open_path.iter().take(chosen_level) {
            labels[count] = path_items[parent].label();
            count += 1;
            path_items = path_items[parent].children();
        }
        labels[count] = item.label();
        let id = item.activation_id(&labels[..=count]);
        if clicked.is_some() {
            focus.claim_click();
        }
        self.close();
        Some(ActivatedItem { id, item })
    }
}

fn layout_context_chain(
    state: &mut ContextMenuState,
    layers: &mut LayerStack,
    layer: usize,
    menu: &ContextMenu<'_>,
    styles: &StyleResolver<'_>,
    viewport: Rect,
    row_h: f32,
) {
    state.rects.truncate(1);
    let mut items = menu.items;
    for level in 1..=state.open_path.len() {
        let parent_index = state.open_path[level - 1];
        let parent_rect = state.rects[level - 1];
        let mut y = parent_rect.y + SHEET_PADDING;
        for item in items.iter().take(parent_index) {
            y += if item.is_separator() {
                SEPARATOR_HEIGHT
            } else {
                row_h
            };
        }
        let row = Rect::new(parent_rect.x, y, parent_rect.width, row_h);
        let children = items[parent_index].children();
        let size = measure_items(
            &mut layers.layers_mut()[layer].list,
            children,
            menu.platform,
            styles,
            viewport.width,
            &mut state.hint_scratch,
        );
        state
            .rects
            .push(place_submenu(row, size, viewport, SubmenuSide::Auto).0);
        items = children;
    }
}

fn reconcile_context_pointer(
    state: &mut ContextMenuState,
    menu: &ContextMenu<'_>,
    input: &InputState,
    row_h: f32,
) -> bool {
    let point = (input.mouse_x, input.mouse_y);
    let mut items = menu.items;
    let mut hovered = None;
    for level in 0..state.rects.len() {
        if level > 0 {
            items = items[state.open_path[level - 1]].children();
        }
        let rect = state.rects[level];
        if !rect.contains(point.0, point.1) {
            continue;
        }
        let mut y = rect.y + SHEET_PADDING;
        for (item_index, item) in items.iter().enumerate() {
            let height = if item.is_separator() {
                SEPARATOR_HEIGHT
            } else {
                row_h
            };
            let row = Rect::new(
                rect.x + SHEET_PADDING,
                y,
                rect.width - SHEET_PADDING * 2.0,
                height,
            );
            if item.is_enabled() && row.contains(point.0, point.1) {
                hovered = Some((level, item_index, item.is_submenu()));
                break;
            }
            y += height;
        }
    }

    if let Some((level, item_index, is_submenu)) = hovered {
        while state.highlights.len() <= level {
            state.highlights.push(None);
        }
        state.highlights[level] = Some(item_index);
        if level == 0 {
            state.highlighted = Some(item_index);
        }
        if is_submenu {
            if state.open_path.get(level).copied() == Some(item_index) {
                return false;
            }
            if level + 1 >= MAX_MENU_DEPTH {
                state.depth_truncations += 1;
                return false;
            }
            let mut level_items = menu.items;
            for &parent in state.open_path.iter().take(level) {
                level_items = level_items[parent].children();
            }
            let children = level_items[item_index].children();
            state.open_path.truncate(level);
            state.open_path.push(item_index);
            state.highlights.truncate(level + 1);
            state
                .highlights
                .push(children.iter().position(MenuItem::is_enabled));
            return true;
        }
        if state.open_path.len() > level {
            state.open_path.truncate(level);
            state.highlights.truncate(level + 1);
            state.rects.truncate(level + 1);
            return true;
        }
        return false;
    }

    for child in state.rects.iter().skip(1) {
        if child.contains(point.0, point.1)
            || state
                .previous_pointer
                .is_some_and(|from| safe_corridor(from, point, *child))
        {
            return false;
        }
    }
    let pointer_moved = state
        .previous_pointer
        .is_some_and(|previous| previous != point);
    if pointer_moved && !state.open_path.is_empty() {
        state.open_path.clear();
        state.highlights.truncate(1);
        state.rects.truncate(1);
        return true;
    }
    false
}

/// Clamp a cursor-anchored sheet wholly inside `viewport`.
pub fn place_context_menu(anchor: [f32; 2], size: [f32; 2], viewport: Rect) -> Rect {
    let width = size[0].max(0.0).min(viewport.width.max(0.0));
    let height = size[1].max(0.0).min(viewport.height.max(0.0));
    Rect::new(
        anchor[0].min(viewport.right() - width).max(viewport.x),
        anchor[1].min(viewport.bottom() - height).max(viewport.y),
        width,
        height,
    )
}

fn measure_items(
    list: &mut DrawList,
    items: &[MenuItem<'_>],
    platform: AccelPlatform,
    styles: &StyleResolver<'_>,
    viewport_width: f32,
    hint: &mut String,
) -> [f32; 2] {
    let mut label_width: f32 = 0.0;
    let mut hint_width: f32 = 0.0;
    let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
    let mut height = SHEET_PADDING * 2.0;
    for item in items {
        if item.is_separator() {
            height += SEPARATOR_HEIGHT;
            continue;
        }
        label_width = label_width.max(list.measure_text(item.label(), FONT_SIZE, None).0);
        hint.clear();
        item.write_hint(platform, hint);
        hint_width = hint_width.max(list.measure_text(hint, HINT_FONT_SIZE, None).0);
        height += row_h;
    }
    let gap = if hint_width > 0.0 { 7.0 } else { 0.0 };
    let width = (SHEET_PADDING * 2.0
        + ROW_PADDING * 2.0
        + CHECK_WIDTH
        + 7.0
        + label_width
        + gap
        + hint_width
        + CHEVRON_WIDTH)
        .max(218.0)
        .min(viewport_width.max(0.0));
    [width, height]
}

fn step_highlight(highlighted: &mut Option<usize>, items: &[MenuItem<'_>], up: bool, down: bool) {
    if !up && !down {
        return;
    }
    let next = if up {
        (0..items.len())
            .rev()
            .filter(|index| items[*index].is_enabled())
            .find(|index| highlighted.is_none_or(|current| *index < current))
            .or_else(|| {
                (0..items.len())
                    .rev()
                    .find(|index| items[*index].is_enabled())
            })
    } else {
        (0..items.len())
            .filter(|index| items[*index].is_enabled())
            .find(|index| highlighted.is_none_or(|current| *index > current))
            .or_else(|| (0..items.len()).find(|index| items[*index].is_enabled()))
    };
    *highlighted = next;
}

fn paint_items(
    list: &mut DrawList,
    rect: Rect,
    items: &[MenuItem<'_>],
    platform: AccelPlatform,
    styles: &StyleResolver<'_>,
    highlighted: Option<usize>,
) {
    let chrome = styles.menu_sheet();
    let padding_box = sheet_padding_box(rect, chrome.surface.border_widths);
    let mut painter = SurfacePainter::new(
        list,
        rect,
        padding_box,
        CornerRadii::default(),
        chrome.surface,
        &chrome.shadows,
        &chrome.lines,
    );
    painter.paint_pre_content();
    let list = painter.draw_list();
    let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
    let row_x = rect.x + SHEET_PADDING;
    let row_w = rect.width - SHEET_PADDING * 2.0;
    let mut y = rect.y + SHEET_PADDING;
    let mut hint = String::new();
    for (index, item) in items.iter().enumerate() {
        if item.is_separator() {
            let rule = Rect::new(
                row_x + 6.0,
                y + 3.0,
                (row_w - 12.0).max(0.0),
                SEPARATOR_HEIGHT - 3.0,
            );
            list.edge_line(
                rule,
                Edge::Top,
                chrome.separator[0].thickness,
                chrome.separator[0].color,
            );
            list.edge_line(
                Rect::new(
                    rule.x,
                    rule.y + chrome.separator[0].thickness,
                    rule.width,
                    rule.height,
                ),
                Edge::Top,
                chrome.separator[1].thickness,
                chrome.separator[1].color,
            );
            y += SEPARATOR_HEIGHT;
            continue;
        }
        let selected = highlighted == Some(index) && item.is_enabled();
        if selected {
            chrome.paint_highlighted_row(
                list,
                Rect::new(row_x, y, row_w, row_h),
                styles.color(StyleKey::Accent),
            );
        }
        if item.is_checked() {
            list.circle(
                (row_x + ROW_PADDING + CHECK_WIDTH * 0.5, y + row_h * 0.5),
                2.0,
                if selected {
                    rgb8([4, 20, 24])
                } else {
                    styles.color(StyleKey::AccentTick)
                },
            );
        }
        let color = if !item.is_enabled() {
            (0x5d, 0x65, 0x6c)
        } else if selected {
            (4, 20, 24)
        } else if item.is_danger() {
            let [r, g, b, _] = crate::color::to_rgba8(styles.color(StyleKey::DangerText));
            (r, g, b)
        } else {
            (0xdb, 0xe1, 0xe7)
        };
        let text_y = list.vcentered_text_y(
            y,
            row_h,
            FONT_SIZE,
            styles.theme().font.as_ref(),
            item.label(),
        );
        let label = TextBlock::new(
            item.label(),
            row_x + ROW_PADDING + CHECK_WIDTH + 7.0,
            text_y,
        )
        .with_size(FONT_SIZE)
        .with_color(color.0, color.1, color.2)
        .with_font_opt(styles.theme().font.clone());
        // The highlighted row drops the carve (Forge MenuSheet).
        list.text(if selected {
            label
        } else {
            label.with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
        });
        hint.clear();
        item.write_hint(platform, &mut hint);
        if !hint.is_empty() {
            let width = list.measure_text(&hint, HINT_FONT_SIZE, None).0;
            let hint_color = if selected {
                // Forge `--ink-on-accent-2`.
                (0x29, 0x4d, 0x55)
            } else {
                (0x78, 0x81, 0x8a)
            };
            let hint_block = TextBlock::new(
                hint.clone(),
                row_x + row_w - ROW_PADDING - CHEVRON_WIDTH - width,
                y + (row_h - HINT_FONT_SIZE) * 0.5,
            )
            .with_size(HINT_FONT_SIZE)
            .with_color(hint_color.0, hint_color.1, hint_color.2)
            .with_font_opt(styles.theme().font.clone());
            list.text(if selected {
                hint_block
            } else {
                hint_block.with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
            });
        }
        if item.is_submenu() {
            let cx = row_x + row_w - ROW_PADDING - 3.0;
            let cy = y + row_h * 0.5;
            list.triangle(
                (cx - 2.0, cy - 4.0),
                (cx - 2.0, cy + 4.0),
                (cx + 3.0, cy),
                if selected {
                    rgb8([4, 20, 24])
                } else {
                    // Forge `--ink-shortcut`.
                    rgb8([0x78, 0x81, 0x8a])
                },
            );
        }
        y += row_h;
    }
    painter.paint_post_content();
}

fn safe_corridor(from: (f32, f32), point: (f32, f32), child: Rect) -> bool {
    if child.contains(point.0, point.1) {
        return true;
    }
    let edge_x = if child.x >= from.0 {
        if point.0 <= from.0 {
            return false;
        }
        child.x
    } else {
        if point.0 >= from.0 {
            return false;
        }
        child.right()
    };
    point_in_triangle(point, from, (edge_x, child.y), (edge_x, child.bottom()))
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    fn cross(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    }
    let (x, y, z) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    (x >= 0.0 && y >= 0.0 && z >= 0.0) || (x <= 0.0 && y <= 0.0 && z <= 0.0)
}

fn sheet_padding_box(rect: Rect, widths: EdgeWidths) -> Rect {
    Rect::new(
        rect.x + widths.left,
        rect.y + widths.top,
        (rect.width - widths.left - widths.right).max(0.0),
        (rect.height - widths.top - widths.bottom).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NavInput, Theme};

    const ITEMS: &[MenuItem<'static>] = &[
        MenuItem::new("Copy").id(7),
        MenuItem::separator(),
        MenuItem::new("Disabled").enabled(false),
        MenuItem::new("Delete").id(9),
    ];

    fn context_frame(
        state: &mut ContextMenuState,
        menu: &ContextMenu<'_>,
        theme: &Theme,
        input: InputState,
        dt: f32,
    ) {
        let styles = StyleResolver::new(theme);
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut captured = input.clone();
        state.begin_frame_with_dt(&mut captured, dt);
        let mut layers = LayerStack::new();
        let layer = state.push_open_layer(&mut layers, menu, &styles, viewport);
        let mut focus = FocusState::new();
        state.draw_open_layer(
            &mut layers,
            layer,
            menu,
            &styles,
            &input,
            &mut focus,
            viewport,
        );
    }

    #[test]
    fn elapsed_time_entry_point_preserves_pointer_history() {
        let mut state = ContextMenuState::new();
        state.open_at(0.0, 0.0);
        let mut input = InputState {
            mouse_x: 12.0,
            mouse_y: 34.0,
            ..Default::default()
        };
        state.begin_frame_with_dt(&mut input, f32::NAN);
        input.mouse_x = 20.0;
        state.begin_frame_with_dt(&mut input, 10.0);
        assert_eq!(state.previous_pointer, Some((12.0, 34.0)));
    }

    #[test]
    fn full_label_path_distinguishes_same_named_context_leaves() {
        const LEAF: &[MenuItem<'static>] = &[MenuItem::new("Run")];
        const ROOT: &[MenuItem<'static>] = &[
            MenuItem::new("First").with_children(LEAF),
            MenuItem::new("Second").with_children(LEAF),
        ];
        let first = LEAF[0].activation_id(&[ROOT[0].label(), LEAF[0].label()]);
        let second = LEAF[0].activation_id(&[ROOT[1].label(), LEAF[0].label()]);
        assert_ne!(first, second);
    }

    #[test]
    fn corridor_handles_children_opening_on_either_side() {
        let right = Rect::new(100.0, 20.0, 80.0, 100.0);
        assert!(safe_corridor((50.0, 40.0), (80.0, 50.0), right));
        let left = Rect::new(0.0, 20.0, 80.0, 100.0);
        assert!(safe_corridor((130.0, 40.0), (100.0, 50.0), left));
        assert!(!safe_corridor((130.0, 40.0), (100.0, 150.0), left));
    }

    #[test]
    fn placement_clamps_both_edges() {
        let viewport = Rect::new(10.0, 20.0, 300.0, 200.0);
        assert_eq!(
            place_context_menu([290.0, 210.0], [100.0, 80.0], viewport),
            Rect::new(210.0, 140.0, 100.0, 80.0)
        );
        assert_eq!(
            place_context_menu([-5.0, 2.0], [100.0, 80.0], viewport),
            Rect::new(10.0, 20.0, 100.0, 80.0)
        );
    }

    #[test]
    fn escape_and_outside_click_close() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        for input in [
            InputState {
                nav: NavInput {
                    cancel: true,
                    ..NavInput::default()
                },
                ..InputState::default()
            },
            InputState {
                mouse_x: 700.0,
                mouse_y: 500.0,
                mouse_clicked: true,
                ..InputState::default()
            },
        ] {
            let mut state = ContextMenuState::new();
            state.open_at(20.0, 20.0);
            let mut captured = input.clone();
            state.begin_frame(&mut captured);
            let mut layers = LayerStack::new();
            let menu = ContextMenu::new(ITEMS);
            let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
            let layer = state.push_open_layer(&mut layers, &menu, &styles, viewport);
            let mut focus = FocusState::new();
            assert!(
                state
                    .draw_open_layer(
                        &mut layers,
                        layer,
                        &menu,
                        &styles,
                        &input,
                        &mut focus,
                        viewport,
                    )
                    .is_none()
            );
            assert!(!state.is_open());
        }
    }

    #[test]
    fn hover_opens_then_leaf_hover_closes_immediately() {
        const CHILD: &[MenuItem<'static>] = &[MenuItem::new("Leaf")];
        const ROOT: &[MenuItem<'static>] = &[
            MenuItem::new("Parent").with_children(CHILD),
            MenuItem::new("Sibling leaf"),
        ];
        let menu = ContextMenu::new(ROOT);
        let theme = Theme::default();
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);

        context_frame(
            &mut state,
            &menu,
            &theme,
            InputState {
                mouse_x: 40.0,
                mouse_y: 30.0,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(state.open_levels(), 2);
        assert_eq!(state.rects.len(), 2, "child is laid out in the hover frame");

        context_frame(
            &mut state,
            &menu,
            &theme,
            InputState {
                mouse_x: 40.0,
                mouse_y: 30.0 + theme.menu_row_height,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(state.open_levels(), 1);
        assert_eq!(state.rects.len(), 1);
    }

    #[test]
    fn clicking_a_parent_lays_out_its_child_in_the_same_frame() {
        const CHILD: &[MenuItem<'static>] = &[MenuItem::new("Leaf")];
        const ROOT: &[MenuItem<'static>] = &[MenuItem::new("Parent").with_children(CHILD)];
        let menu = ContextMenu::new(ROOT);
        let theme = Theme::default();
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);

        context_frame(
            &mut state,
            &menu,
            &theme,
            InputState {
                mouse_x: 40.0,
                mouse_y: 30.0,
                mouse_clicked: true,
                mouse_down: true,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(state.open_path, [0]);
        assert_eq!(state.rects.len(), 2, "click frame lays out the child");
    }

    #[test]
    fn leaving_the_context_menu_chain_closes_children_immediately() {
        const CHILD: &[MenuItem<'static>] = &[MenuItem::new("Leaf")];
        const ROOT: &[MenuItem<'static>] = &[MenuItem::new("Parent").with_children(CHILD)];
        let menu = ContextMenu::new(ROOT);
        let theme = Theme::default();
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);
        assert!(state.set_open_path(&menu, &[0]));

        context_frame(
            &mut state,
            &menu,
            &theme,
            InputState {
                mouse_x: 40.0,
                mouse_y: 30.0,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(state.open_levels(), 2);
        context_frame(
            &mut state,
            &menu,
            &theme,
            InputState {
                mouse_x: 700.0,
                mouse_y: 500.0,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(state.open_levels(), 1);
    }

    #[test]
    fn open_child_parent_replaces_with_sibling_immediately() {
        const CHILD: &[MenuItem<'static>] = &[MenuItem::new("Leaf")];
        const ROOT: &[MenuItem<'static>] = &[
            MenuItem::new("First").with_children(CHILD),
            MenuItem::new("Second").with_children(CHILD),
        ];
        let menu = ContextMenu::new(ROOT);
        let theme = Theme::default();
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);
        assert!(state.set_open_path(&menu, &[0]));
        context_frame(
            &mut state,
            &menu,
            &theme,
            InputState {
                mouse_x: 40.0,
                mouse_y: 30.0 + theme.menu_row_height,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(state.open_path, [1]);
    }

    #[test]
    fn depth_limit_truncates_without_panicking_and_counts_attempts() {
        const LEVEL_8: &[MenuItem<'static>] = &[MenuItem::new("Leaf")];
        const LEVEL_7: &[MenuItem<'static>] = &[MenuItem::new("7").with_children(LEVEL_8)];
        const LEVEL_6: &[MenuItem<'static>] = &[MenuItem::new("6").with_children(LEVEL_7)];
        const LEVEL_5: &[MenuItem<'static>] = &[MenuItem::new("5").with_children(LEVEL_6)];
        const LEVEL_4: &[MenuItem<'static>] = &[MenuItem::new("4").with_children(LEVEL_5)];
        const LEVEL_3: &[MenuItem<'static>] = &[MenuItem::new("3").with_children(LEVEL_4)];
        const LEVEL_2: &[MenuItem<'static>] = &[MenuItem::new("2").with_children(LEVEL_3)];
        const LEVEL_1: &[MenuItem<'static>] = &[MenuItem::new("1").with_children(LEVEL_2)];
        const ROOT: &[MenuItem<'static>] = &[MenuItem::new("0").with_children(LEVEL_1)];
        let menu = ContextMenu::new(ROOT);
        let mut state = ContextMenuState::new();
        state.open_at(0.0, 0.0);
        assert!(!state.set_open_path(&menu, &[0; MAX_MENU_DEPTH]));
        assert_eq!(state.open_levels(), MAX_MENU_DEPTH);
        assert_eq!(state.depth_truncations(), 1);
    }

    #[test]
    fn keyboard_opens_and_unwinds_recursive_context_submenus() {
        const LEAF: &[MenuItem<'static>] = &[MenuItem::new("Leaf").id(77)];
        const ROOT: &[MenuItem<'static>] = &[MenuItem::new("More").with_children(LEAF)];
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let menu = ContextMenu::new(ROOT);
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);
        state.highlights[0] = Some(0);
        let mut layers = LayerStack::new();
        let layer = state.push_open_layer(&mut layers, &menu, &styles, viewport);
        let mut input = InputState {
            nav: NavInput {
                right: true,
                ..Default::default()
            },
            ..Default::default()
        };
        state.begin_frame(&mut input);
        let mut focus = FocusState::new();
        assert!(
            state
                .draw_open_layer(
                    &mut layers,
                    layer,
                    &menu,
                    &styles,
                    &input,
                    &mut focus,
                    viewport
                )
                .is_none()
        );
        assert_eq!(state.open_path, [0]);

        let mut input = InputState {
            nav: NavInput {
                left: true,
                ..Default::default()
            },
            ..Default::default()
        };
        state.begin_frame(&mut input);
        assert!(
            state
                .draw_open_layer(
                    &mut layers,
                    layer,
                    &menu,
                    &styles,
                    &input,
                    &mut focus,
                    viewport
                )
                .is_none()
        );
        assert!(state.open_path.is_empty());
        assert!(state.is_open());
    }

    #[test]
    fn sheet_uses_analytic_shadows_and_typed_overlay() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.menu_sheet;
        let override_color = [0.6, 0.1, 0.2, 1.0];
        chrome.surface.background = crate::Background::Solid(override_color);
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_menu_sheet(chrome);
        let styles = StyleResolver::with_overlay(&theme, &overlay);
        let menu = ContextMenu::new(ITEMS);
        let mut list = DrawList::new();
        let rect = Rect::new(20.0, 30.0, 218.0, 80.0);

        paint_items(&mut list, rect, menu.items, menu.platform, &styles, None);

        assert_eq!(list.shadow_instance_count(), 2);
        assert_eq!(
            list.shadow_instance(0).unwrap().color,
            chrome.shadows[1].color
        );
        assert_eq!(
            list.shadow_instance(1).unwrap().color,
            chrome.shadows[0].color
        );
        assert_eq!(list.chrome_instance(0).unwrap().bg, override_color);
    }

    #[test]
    fn highlighted_row_matches_the_forge_menu_sheet() {
        const ROWS: &[MenuItem<'static>] = &[
            MenuItem::new("Snap").checked(true).shortcut("S"),
            MenuItem::new("Grid").checked(true),
        ];
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let menu = ContextMenu::new(ROWS);
        let mut list = DrawList::new();
        let rect = Rect::new(20.0, 30.0, 218.0, 80.0);

        paint_items(&mut list, rect, menu.items, menu.platform, &styles, Some(0));

        assert!(
            list.chrome_instances().any(|quad| quad.bg == theme.accent),
            "accent row plate"
        );
        let edges: Vec<[f32; 4]> = list.shadow_instances().skip(2).map(|s| s.color).collect();
        for inset in theme.chrome.menu_sheet.row_highlight_insets {
            assert!(edges.contains(&inset.color), "{inset:?} painted");
        }
        // Tick: ink-on-accent on the highlighted row, accent-tick elsewhere.
        let dots: Vec<[f32; 4]> = list.circle_instances.iter().map(|c| c.color).collect();
        assert!(dots.contains(&rgb8([4, 20, 24])), "{dots:?}");
        assert!(dots.contains(&theme.accent_tick), "{dots:?}");

        let snap = list.texts.iter().find(|t| t.content == "Snap").unwrap();
        assert!(snap.shadow.is_none(), "no carve on the highlighted row");
        let grid = list.texts.iter().find(|t| t.content == "Grid").unwrap();
        assert!(grid.shadow.is_some(), "idle rows keep the carve");
        let hint = list.texts.iter().find(|t| t.content == "S").unwrap();
        assert_eq!(hint.color.as_rgba(), [0x29, 0x4d, 0x55, 0xff]);
    }

    #[test]
    fn clicking_enabled_row_activates_and_closes() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let menu = ContextMenu::new(ITEMS);
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);
        let mut layers = LayerStack::new();
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let layer = state.push_open_layer(&mut layers, &menu, &styles, viewport);
        let input = InputState {
            mouse_x: 40.0,
            mouse_y: 30.0,
            mouse_clicked: true,
            ..InputState::default()
        };
        let mut focus = FocusState::new();
        let activation = state
            .draw_open_layer(
                &mut layers,
                layer,
                &menu,
                &styles,
                &input,
                &mut focus,
                viewport,
            )
            .expect("activation");
        assert_eq!(activation.id, 7);
        assert!(!state.is_open());
    }

    #[test]
    fn danger_rows_use_the_danger_ink_until_highlighted() {
        const ROWS: &[MenuItem<'static>] = &[
            MenuItem::new("Rename…"),
            MenuItem::new("Close Session").danger(true),
        ];
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let menu = ContextMenu::new(ROWS);
        let rect = Rect::new(0.0, 0.0, 218.0, 60.0);
        let ink = |list: &DrawList, label: &str| {
            list.texts
                .iter()
                .find(|t| t.content == label)
                .unwrap()
                .color
                .as_rgba()
        };
        let mut list = DrawList::new();
        paint_items(&mut list, rect, menu.items, menu.platform, &styles, None);
        assert_eq!(
            ink(&list, "Close Session"),
            crate::color::to_rgba8(theme.danger_text)
        );
        assert_eq!(ink(&list, "Rename…"), [0xdb, 0xe1, 0xe7, 0xff]);
        let mut list = DrawList::new();
        paint_items(&mut list, rect, menu.items, menu.platform, &styles, Some(1));
        assert_eq!(ink(&list, "Close Session"), [4, 20, 24, 0xff]);
    }

    #[test]
    fn a_disabled_row_reports_its_reason_under_the_pointer() {
        const ROWS: &[MenuItem<'static>] = &[
            MenuItem::new("Rename…")
                .enabled(false)
                .reason("not available yet"),
            MenuItem::new("Regenerate Title").enabled(false),
            MenuItem::new("Close Session").reason("ignored while enabled"),
        ];
        let theme = Theme::default();
        let menu = ContextMenu::new(ROWS);
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);
        assert_eq!(
            state.hovered_reason(&menu, 40.0, 30.0),
            None,
            "not laid out yet"
        );
        context_frame(&mut state, &menu, &theme, InputState::default(), 0.0);
        let rect = state.rect().unwrap();
        let row_h = theme
            .get(StyleKey::MenuRowHeight)
            .unwrap()
            .as_scalar()
            .unwrap();
        let row_y = |i: f32| rect.y + SHEET_PADDING + row_h * (i + 0.5);
        let (reason, row) = state
            .hovered_reason(&menu, rect.x + 30.0, row_y(0.0))
            .expect("the disabled row's reason");
        assert_eq!(reason, "not available yet");
        assert_eq!(row.y, rect.y + SHEET_PADDING);
        assert_eq!(state.hovered_reason(&menu, rect.x + 30.0, row_y(1.0)), None);
        assert_eq!(state.hovered_reason(&menu, rect.x + 30.0, row_y(2.0)), None);
        assert_eq!(state.hovered_reason(&menu, rect.x - 5.0, row_y(0.0)), None);
        state.close();
        assert_eq!(state.hovered_reason(&menu, rect.x + 30.0, row_y(0.0)), None);
    }
}
