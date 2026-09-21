//! Cursor-anchored context menus using the shared [`MenuItem`](crate::MenuItem)
//! model and menu-sheet visual language.

use crate::color::opaque_srgb8;
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
    hover_level: Option<usize>,
    hover_item: Option<usize>,
    hover_elapsed: f32,
    frame_dt: f32,
    previous_pointer: Option<(f32, f32)>,
    last_pointer: Option<(f32, f32)>,
    depth_truncations: usize,
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
    }

    /// Close the menu and discard its transient selection/geometry.
    pub fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
        self.rect = None;
        self.open_path.clear();
        self.highlights.clear();
        self.rects.clear();
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

    /// Capture navigation intents used by an open menu. Call before focus and
    /// base widgets consume the shared input. This compatibility entry point does
    /// not advance timed hover intent.
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.begin_frame_with_dt(input, 0.0);
    }

    /// Capture input with an elapsed time for submenu hover/close intent.
    /// Invalid/negative deltas become zero and long stalls are capped at
    /// [`crate::MAX_DT`].
    pub fn begin_frame_with_dt(&mut self, input: &mut InputState, dt: f32) {
        self.frame_dt = crate::frame_result::sanitize_dt(dt);
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
        let rect = self.rect.expect("open context menu has measured geometry");
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
        let mut clicked = None;
        let mut hovered = None;
        self.rects.truncate(1);
        let mut draw_items = menu.items;
        for draw_level in 0..=self.open_path.len() {
            let draw_rect = if draw_level == 0 {
                rect
            } else {
                let parent_index = self.open_path[draw_level - 1];
                let parent_rect = self.rects[draw_level - 1];
                let mut y = parent_rect.y + SHEET_PADDING;
                for item in draw_items.iter().take(parent_index) {
                    y += if item.is_separator() {
                        SEPARATOR_HEIGHT
                    } else {
                        row_h
                    };
                }
                let row = Rect::new(parent_rect.x, y, parent_rect.width, row_h);
                let size = measure_items(
                    &mut layers.layers_mut()[index].list,
                    draw_items[parent_index].children(),
                    menu.platform,
                    styles,
                    viewport.width,
                    &mut self.hint_scratch,
                );
                let placed = place_submenu(row, size, viewport, SubmenuSide::Auto).0;
                self.rects.push(placed);
                placed
            };
            if draw_level > 0 {
                let parent = self.open_path[draw_level - 1];
                draw_items = draw_items[parent].children();
            }
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
                if item.is_enabled() && row.contains(layer_input.mouse_x, layer_input.mouse_y) {
                    hovered = Some((draw_level, item_index));
                    self.highlights[draw_level] = Some(item_index);
                    if draw_level == 0 {
                        self.highlighted = Some(item_index);
                    }
                    if layer_input.mouse_clicked {
                        clicked = Some((draw_level, item_index));
                    }
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

        if let Some((hover_level, item_index)) = hovered {
            let mut hover_items = menu.items;
            for &parent in self.open_path.iter().take(hover_level) {
                hover_items = hover_items[parent].children();
            }
            let item = &hover_items[item_index];
            let corridor = self.open_path.get(hover_level).is_some_and(|_| {
                self.rects.get(hover_level + 1).is_some_and(|child| {
                    self.previous_pointer.is_some_and(|from| {
                        safe_corridor(from, (layer_input.mouse_x, layer_input.mouse_y), *child)
                    })
                })
            });
            if !corridor {
                if self.hover_level == Some(hover_level) && self.hover_item == Some(item_index) {
                    self.hover_elapsed += self.frame_dt;
                } else {
                    self.hover_level = Some(hover_level);
                    self.hover_item = Some(item_index);
                    self.hover_elapsed = 0.0;
                }
                let delay = styles.scalar(StyleKey::MenuHoverDelay).max(0.0);
                let open_parent = self.open_path.get(hover_level).copied();
                let child_is_open = open_parent.is_some();
                if item.is_submenu() && open_parent.is_some_and(|parent| parent != item_index) {
                    self.open_path.truncate(hover_level);
                    self.open_path.push(item_index);
                    self.highlights.truncate(hover_level + 1);
                    self.highlights
                        .push(item.children().iter().position(MenuItem::is_enabled));
                } else if self.hover_elapsed >= delay {
                    if item.is_submenu() {
                        if hover_level + 1 < MAX_MENU_DEPTH {
                            self.open_path.truncate(hover_level);
                            self.open_path.push(item_index);
                            self.highlights.truncate(hover_level + 1);
                            self.highlights
                                .push(item.children().iter().position(MenuItem::is_enabled));
                        } else {
                            self.depth_truncations += 1;
                        }
                    } else if child_is_open {
                        self.open_path.truncate(hover_level);
                        self.highlights.truncate(hover_level + 1);
                        self.rects.truncate(hover_level + 1);
                    }
                }
            }
        } else {
            self.hover_level = None;
            self.hover_item = None;
            self.hover_elapsed = 0.0;
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
            list.quad(row_x, y, row_w, row_h, opaque_srgb8([0x79, 0xc6, 0xd8]));
        }
        if item.is_checked() {
            list.circle(
                (row_x + ROW_PADDING + CHECK_WIDTH * 0.5, y + row_h * 0.5),
                2.0,
                styles.color(StyleKey::Accent),
            );
        }
        let color = if !item.is_enabled() {
            (0x5d, 0x65, 0x6c)
        } else if selected {
            (4, 20, 24)
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
        list.text(
            TextBlock::new(
                item.label(),
                row_x + ROW_PADDING + CHECK_WIDTH + 7.0,
                text_y,
            )
            .with_size(FONT_SIZE)
            .with_color(color.0, color.1, color.2)
            .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
            .with_font_opt(styles.theme().font.clone()),
        );
        hint.clear();
        item.write_hint(platform, &mut hint);
        if !hint.is_empty() {
            let width = list.measure_text(&hint, HINT_FONT_SIZE, None).0;
            let hint_color = if selected {
                (4, 20, 24)
            } else {
                (0x78, 0x81, 0x8a)
            };
            list.text(
                TextBlock::new(
                    hint.clone(),
                    row_x + row_w - ROW_PADDING - CHEVRON_WIDTH - width,
                    y + (row_h - HINT_FONT_SIZE) * 0.5,
                )
                .with_size(HINT_FONT_SIZE)
                .with_color(hint_color.0, hint_color.1, hint_color.2)
                .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                .with_font_opt(styles.theme().font.clone()),
            );
        }
        if item.is_submenu() {
            let cx = row_x + row_w - ROW_PADDING - 3.0;
            let cy = y + row_h * 0.5;
            list.triangle(
                (cx - 2.0, cy - 4.0),
                (cx - 2.0, cy + 4.0),
                (cx + 3.0, cy),
                if selected {
                    opaque_srgb8([4, 20, 24])
                } else {
                    styles.color(StyleKey::Text)
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
    fn frame_dt_is_sanitized_and_legacy_begin_frame_keeps_zero_dt() {
        let mut state = ContextMenuState::new();
        state.open_at(0.0, 0.0);
        let mut input = InputState::default();
        for (dt, expected) in [
            (f32::NAN, 0.0),
            (f32::INFINITY, 0.0),
            (-1.0, 0.0),
            (10.0, crate::MAX_DT),
        ] {
            state.begin_frame_with_dt(&mut input, dt);
            assert_eq!(state.frame_dt, expected);
        }
        state.begin_frame(&mut input);
        assert_eq!(state.frame_dt, 0.0);
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
    fn timed_hover_opens_then_leaf_hover_closes_after_delay() {
        const CHILD: &[MenuItem<'static>] = &[MenuItem::new("Leaf")];
        const ROOT: &[MenuItem<'static>] = &[
            MenuItem::new("Parent").with_children(CHILD),
            MenuItem::new("Sibling leaf"),
        ];
        let menu = ContextMenu::new(ROOT);
        let mut theme = Theme::default();
        theme.menu_hover_delay = 0.05;
        let mut state = ContextMenuState::new();
        state.open_at(20.0, 20.0);

        let parent = InputState {
            mouse_x: 40.0,
            mouse_y: 30.0,
            ..Default::default()
        };
        context_frame(&mut state, &menu, &theme, parent.clone(), 0.03);
        assert_eq!(state.open_levels(), 1);
        context_frame(&mut state, &menu, &theme, parent, 0.05);
        assert_eq!(state.open_levels(), 2);

        let sibling = InputState {
            mouse_x: 40.0,
            mouse_y: 30.0 + theme.menu_row_height,
            ..Default::default()
        };
        context_frame(&mut state, &menu, &theme, sibling.clone(), 0.03);
        assert_eq!(state.open_levels(), 2);
        context_frame(&mut state, &menu, &theme, sibling, 0.05);
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
}
