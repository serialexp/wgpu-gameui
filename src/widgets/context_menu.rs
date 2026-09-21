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
use super::menubar::{AccelPlatform, ActivatedItem, MenuItem};

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
    up: bool,
    down: bool,
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
    }

    /// Close the menu and discard its transient selection/geometry.
    pub fn close(&mut self) {
        self.open = false;
        self.highlighted = None;
        self.rect = None;
    }

    /// Whether the menu is currently open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Current placed sheet rectangle, once measured this frame.
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }

    /// Capture navigation intents used by an open menu. Call before focus and
    /// base widgets consume the shared input.
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.up = false;
        self.down = false;
        self.confirm = false;
        self.cancel = false;
        if !self.open {
            return;
        }
        self.up = input.nav.up;
        self.down = input.nav.down;
        self.confirm = input.nav.confirm;
        self.cancel = input.nav.cancel;
        input.nav.up = false;
        input.nav.down = false;
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
        let size = measure_menu(layers.base_mut(), menu, styles, viewport.width);
        let rect = place_context_menu(self.anchor, size, viewport);
        self.rect = Some(rect);
        let index = layers.push_modal(rect);
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
                let size = measure_menu(layers.base_mut(), menu, styles, viewport.width);
                let rect = place_context_menu(self.anchor, size, viewport);
                self.rect = Some(rect);
                let index = layers.push_modal(rect);
                layers.pop_layer();
                index
            }
        };
        let rect = self.rect.expect("open context menu has measured geometry");
        let layer_input = layers.input_for_layer(index, input);

        if self.cancel
            || ((layer_input.mouse_clicked || layer_input.mouse_right_clicked)
                && !rect.contains(layer_input.mouse_x, layer_input.mouse_y))
        {
            self.close();
            return None;
        }

        step_highlight(&mut self.highlighted, menu.items, self.up, self.down);
        let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
        let mut y = rect.y + SHEET_PADDING;
        let mut clicked = None;
        for (item_index, item) in menu.items.iter().enumerate() {
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
            if item.is_enabled() && row.contains(layer_input.mouse_x, layer_input.mouse_y) {
                self.highlighted = Some(item_index);
                if layer_input.mouse_clicked {
                    clicked = Some(item_index);
                }
            }
            y += height;
        }

        paint_menu(
            &mut layers.layers_mut()[index].list,
            rect,
            menu,
            styles,
            self.highlighted,
        );

        let chosen = clicked.or(self.confirm.then_some(self.highlighted).flatten());
        let item_index = chosen?;
        let item = menu.items.get(item_index)?;
        if !item.is_enabled() || item.is_submenu() {
            return None;
        }
        let id = item.activation_id(&[item.label()]);
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

fn measure_menu(
    list: &mut DrawList,
    menu: &ContextMenu<'_>,
    styles: &StyleResolver<'_>,
    viewport_width: f32,
) -> [f32; 2] {
    let mut label_width: f32 = 0.0;
    let mut hint_width: f32 = 0.0;
    let mut hint = String::new();
    let row_h = styles.scalar(StyleKey::MenuRowHeight).max(1.0);
    let mut height = SHEET_PADDING * 2.0;
    for item in menu.items {
        if item.is_separator() {
            height += SEPARATOR_HEIGHT;
            continue;
        }
        label_width = label_width.max(list.measure_text(item.label(), FONT_SIZE, None).0);
        hint.clear();
        item.write_hint(menu.platform, &mut hint);
        hint_width = hint_width.max(list.measure_text(&hint, HINT_FONT_SIZE, None).0);
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

fn paint_menu(
    list: &mut DrawList,
    rect: Rect,
    menu: &ContextMenu<'_>,
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
    for (index, item) in menu.items.iter().enumerate() {
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
            .with_font_opt(styles.theme().font.clone()),
        );
        hint.clear();
        item.write_hint(menu.platform, &mut hint);
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

        paint_menu(&mut list, rect, &menu, &styles, None);

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
