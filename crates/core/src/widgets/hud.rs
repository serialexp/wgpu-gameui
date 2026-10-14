//! HUD bars: strips of keys floating over a 3D viewport (Forge `HudBar`,
//! `HudKey`, `HudSep` and `HudSelect`), on HUD glass.
//!
//! Everything over the scene sits on [`hud_glass`]: dark, hard-edged, because
//! the pixels behind it are unknown. Keep bars in the corners and along the
//! edges; the centre belongs to the scene. A toggle latches with
//! [`HudKey::on`], so its state reads from across the room.
//!
//! A [`HudSelect`] opens a radio sheet, which floats on a layer above
//! everything else. Like [`DropdownState`](crate::DropdownState), a
//! caller-owned [`HudState`] (one per surface, shared by all its bars) holds
//! the open sheet, and the frame is split around the base draw:
//!
//! ```ignore
//! hud.begin_frame(&mut raw_input);                    // before focus.begin_frame
//! focus.begin_frame(&raw_input);
//! let sheet = hud.push_open_layer(&mut layers, &styles, viewport);
//! let input = layers.input_for_base(&raw_input);
//! // … base widgets, including the bars:
//! let items = [
//!     HudItem::Select(HudSelect::new(&SHADING, shading).caption("View")),
//!     HudItem::Separator,
//!     HudItem::Key(HudKey::new("#").on(grid)),
//! ];
//! let out = HudBar::new(TOP_BAR, &items).draw([12.0, 12.0], &mut hud, &mut ctx);
//! if out.clicked == Some(2) { grid = !grid; }
//! // after the base scope:
//! if let Some(choice) = hud.draw_open_layer(&mut layers, sheet, &styles, &raw_input, &mut focus, viewport) {
//!     if choice.bar == TOP_BAR && choice.item == 0 { shading = choice.option; }
//! }
//! ```
//!
//! The glass has no backdrop blur yet: the draw list has no backdrop
//! primitive (see `TODO.md`).

use crate::layout::Rect;
use crate::text::TextBlock;
use crate::{
    ContextMenu, ContextMenuState, FocusState, Ink, InputState, LayerStack, MenuItem,
    StyleResolver, SurfacePainter,
};

use super::material::{Tone, latch_ink};
use super::{DrawContext, DrawList, Pressable, Stroke};

/// Identity of one bar within a [`HudState`]; unique per bar on the surface.
pub type HudBarId = u64;

/// Padding inside a bar, around its keys.
const PAD: f32 = 2.0;
/// Space between neighbouring keys.
const GAP: f32 = 2.0;
/// Height of every key's face; a single-glyph key is square.
const FACE: f32 = 20.0;
/// Horizontal padding of a text key (`pad="3px 7px"`).
const TEXT_PAD: f32 = 7.0;
/// A select key's right padding, before its chevron (`3px 5px 3px 7px`).
const SELECT_PAD_RIGHT: f32 = 5.0;
/// Gap between a select key's caption, value and chevron.
const SELECT_GAP: f32 = 5.0;
/// Key label size (`fontSize={10.5}`).
const LABEL_SIZE: f32 = 10.5;
/// A select key's caption size (`8.5px`, letter-spaced 0.1em).
const CAPTION_SIZE: f32 = 8.5;
const CAPTION_TRACKING: f32 = 0.1;
/// Half-width of a select key's chevron.
const CHEVRON: f32 = 2.5;
/// A separator: a 1 px rule with this much margin along the bar and across.
const SEP_MARGIN_ALONG: f32 = 2.0;
const SEP_MARGIN_ACROSS: f32 = 3.0;
/// Space between a select key and the sheet it opens.
const SHEET_GAP: f32 = 5.0;
/// The sheet's minimum width (`MenuSheet minWidth={140}`).
const SHEET_MIN_WIDTH: f32 = 140.0;

/// Paint a HUD glass surface (Forge `hudGlass`) over `rect`: the
/// [`hud`](StyleResolver::hud) chrome's fill, shadow, border and lit top edge.
/// For anything floating over a 3D scene.
pub fn hud_glass(list: &mut DrawList, s: &StyleResolver, rect: Rect) {
    let chrome = s.hud();
    let border = chrome.surface.border_widths.left;
    let mut painter = SurfacePainter::new(
        list,
        rect,
        rect.inset(border),
        chrome.surface.corner_radii,
        chrome.surface,
        std::slice::from_ref(&chrome.shadow),
        &chrome.lines,
    );
    painter.paint_pre_content();
    painter.paint_post_content();
}

/// A HUD toggle or action key (Forge `HudKey`). A single-character label
/// makes a square key; longer labels size from padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudKey<'a> {
    label: &'a str,
    on: bool,
    mono: bool,
    enabled: bool,
    width: Option<f32>,
}

impl<'a> HudKey<'a> {
    /// A ghost key showing `label`.
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            on: false,
            mono: false,
            enabled: true,
            width: None,
        }
    }

    /// Latch the key: a toggle that is on wears the latched accent face.
    pub const fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// Set the label in the mono face (`fps`, units).
    pub const fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Enable or disable the key.
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// A fixed face width, in px, instead of sizing from the label.
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    fn is_glyph(&self) -> bool {
        let mut chars = self.label.chars();
        chars.next().is_some() && chars.next().is_none()
    }
}

/// One choice of a [`HudSelect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudOption<'a> {
    label: &'a str,
    short: Option<&'a str>,
    shortcut: Option<&'a str>,
    enabled: bool,
    reason: Option<&'a str>,
}

impl<'a> HudOption<'a> {
    /// An option labelled `label` in the sheet and on the key.
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            short: None,
            shortcut: None,
            enabled: true,
            reason: None,
        }
    }

    /// A shorter label for the key while this option is chosen
    /// ("Density" for "Lightmap density").
    pub const fn short(mut self, short: &'a str) -> Self {
        self.short = Some(short);
        self
    }

    /// A shortcut hint shown in the sheet ("Alt 1").
    pub const fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Disable the option, saying why on hover.
    pub const fn disabled(mut self, reason: &'a str) -> Self {
        self.enabled = false;
        self.reason = Some(reason);
        self
    }

    fn key_label(&self) -> &'a str {
        self.short.unwrap_or(self.label)
    }
}

/// A HUD key that shows its current value and opens a radio sheet below it,
/// or above it for a bar along the bottom edge (Forge `HudSelect`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudSelect<'a> {
    options: &'a [HudOption<'a>],
    selected: usize,
    caption: Option<&'a str>,
    up: bool,
}

impl<'a> HudSelect<'a> {
    /// A select over `options` with `options[selected]` chosen.
    pub const fn new(options: &'a [HudOption<'a>], selected: usize) -> Self {
        Self {
            options,
            selected,
            caption: None,
            up: false,
        }
    }

    /// A tiny upper-case prefix on the key ("Grid").
    pub const fn caption(mut self, caption: &'a str) -> Self {
        self.caption = Some(caption);
        self
    }

    /// Open the sheet upward, for a bar along the bottom edge.
    pub const fn up(mut self) -> Self {
        self.up = true;
        self
    }

    fn value(&self) -> &'a str {
        self.options
            .get(self.selected)
            .map_or("", HudOption::key_label)
    }
}

/// One entry of a [`HudBar`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HudItem<'a> {
    /// A toggle or action key.
    Key(HudKey<'a>),
    /// A thin rule between groups of keys (Forge `HudSep`).
    Separator,
    /// A key that opens a radio sheet.
    Select(HudSelect<'a>),
}

/// What a [`HudBar`] reports for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudBarOutput {
    /// Where the bar was drawn.
    pub rect: Rect,
    /// The pointer is over the bar: the scene under it should not take the
    /// pointer.
    pub hovered: bool,
    /// The index (into the bar's items) of a [`HudItem::Key`] clicked or
    /// activated from the keyboard this frame.
    pub clicked: Option<usize>,
}

/// A choice made in an open [`HudSelect`] sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudChoice {
    /// The bar the select is on.
    pub bar: HudBarId,
    /// The select's index among the bar's items.
    pub item: usize,
    /// The chosen option's index.
    pub option: usize,
}

/// A floating strip of HUD keys pinned to a viewport edge or corner (Forge
/// `HudBar`). Built fresh each frame over borrowed items.
#[derive(Clone, Copy, Debug)]
pub struct HudBar<'a> {
    id: HudBarId,
    items: &'a [HudItem<'a>],
    vertical: bool,
}

impl<'a> HudBar<'a> {
    /// A horizontal bar `id` of `items`.
    pub const fn new(id: HudBarId, items: &'a [HudItem<'a>]) -> Self {
        Self {
            id,
            items,
            vertical: false,
        }
    }

    /// Stack the items top to bottom, keys stretched to the widest.
    pub const fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    /// The bar's size, in px: what [`draw`](Self::draw) will cover, so a
    /// caller can pin it to a right or bottom edge.
    pub fn size(&self, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        let mut along: f32 = 0.0;
        let mut across: f32 = 0.0;
        let mut count: usize = 0;
        for item in self.items {
            let [w, h] = self.item_size(item, list, s);
            let (a, c) = if self.vertical { (h, w) } else { (w, h) };
            along += a;
            across = across.max(c);
            count += 1;
        }
        along += GAP * count.saturating_sub(1) as f32 + PAD * 2.0;
        across += PAD * 2.0;
        if self.vertical {
            [across, along]
        } else {
            [along, across]
        }
    }

    /// One item's outer size (a key's face plus its travel), in px.
    fn item_size(&self, item: &HudItem, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        let travel = Pressable::new().travel_px(s);
        match item {
            HudItem::Key(key) => {
                let width = key.width.unwrap_or_else(|| {
                    if key.is_glyph() {
                        FACE
                    } else {
                        label_width(list, s, key.label, key.mono) + TEXT_PAD * 2.0 + 2.0
                    }
                });
                [width, FACE + travel]
            }
            HudItem::Separator => {
                if self.vertical {
                    [1.0, 1.0 + SEP_MARGIN_ALONG * 2.0]
                } else {
                    [1.0 + SEP_MARGIN_ALONG * 2.0, 1.0]
                }
            }
            HudItem::Select(select) => [select_width(list, s, select), FACE + travel],
        }
    }

    /// Draw the bar with its top-left corner at `origin` and handle its keys.
    /// A select key opens or closes its sheet in `hud`; the choice made in
    /// the sheet comes back from [`HudState::draw_open_layer`].
    pub fn draw(
        &self,
        origin: [f32; 2],
        hud: &mut HudState,
        ctx: &mut DrawContext,
    ) -> HudBarOutput {
        let s = ctx.styles();
        let [width, height] = self.size(ctx.draw_list, &s);
        let rect = Rect::new(origin[0], origin[1], width, height);
        ctx.push_debug_scope_rect("HudBar", rect);
        hud_glass(ctx.draw_list, &s, rect);

        let mut clicked = None;
        let mut at = if self.vertical { rect.y } else { rect.x } + PAD;
        let inner_across = if self.vertical { width } else { height } - PAD * 2.0;
        for (index, item) in self.items.iter().enumerate() {
            let [w, h] = self.item_size(item, ctx.draw_list, &s);
            let r = if self.vertical {
                // Keys stretch across a vertical bar; rules span it.
                Rect::new(rect.x + PAD, at, inner_across, h)
            } else {
                Rect::new(at, rect.y + PAD + (inner_across - h) * 0.5, w, h)
            };
            at += if self.vertical { h } else { w } + GAP;
            let id = key_id(self.id, index);
            match item {
                HudItem::Key(key) => {
                    if draw_key(key, id, r, ctx) {
                        clicked = Some(index);
                    }
                }
                HudItem::Separator => draw_separator(ctx.draw_list, r, self.vertical),
                HudItem::Select(select) => {
                    let open = hud.is_open(self.id, index);
                    if draw_select(select, open, id, r, ctx) {
                        if open {
                            hud.close();
                        } else {
                            let screen = ctx.draw_list.current_transform().transform_rect_aabb(r);
                            hud.open(self.id, index, select, screen);
                        }
                    }
                }
            }
        }

        ctx.pop_debug_scope();
        let input = ctx.input;
        HudBarOutput {
            rect,
            hovered: !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y),
            clicked,
        }
    }
}

/// A focus id for item `index` of bar `bar`.
fn key_id(bar: HudBarId, index: usize) -> u64 {
    bar ^ (index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn label_width(list: &mut DrawList, s: &StyleResolver, text: &str, mono: bool) -> f32 {
    let font = if mono {
        s.theme().mono_font.as_ref()
    } else {
        s.theme().font.as_ref()
    };
    list.measure_text_with_font(text, LABEL_SIZE, None, font).0
}

fn caption_width(list: &mut DrawList, s: &StyleResolver, caption: &str) -> f32 {
    let block = caption_text(s, caption, 0.0, 0.0, [0.0; 4]);
    list.measure_block(&block).0
}

fn caption_text(s: &StyleResolver, caption: &str, x: f32, y: f32, color: [f32; 4]) -> TextBlock {
    TextBlock::new(caption.to_uppercase(), x, y)
        .with_size(CAPTION_SIZE)
        .with_letter_spacing(CAPTION_SIZE * CAPTION_TRACKING)
        .with_color_f32(color)
        .with_font_opt(s.theme().mono_font.clone())
}

fn select_width(list: &mut DrawList, s: &StyleResolver, select: &HudSelect) -> f32 {
    let caption = select
        .caption
        .map_or(0.0, |c| caption_width(list, s, c) + SELECT_GAP);
    1.0 + TEXT_PAD
        + caption
        + label_width(list, s, select.value(), false)
        + SELECT_GAP
        + CHEVRON * 2.0
        + SELECT_PAD_RIGHT
        + 1.0
}

/// The ink of a key's label: the latch ink while latched, else the ghost
/// key's.
fn key_ink(s: &StyleResolver, latched: bool, hovered: bool, down: bool) -> [f32; 4] {
    if latched {
        latch_ink(s, hovered, down)
    } else if down {
        s.ink(Ink::Glyph)
    } else {
        s.ink(Ink::Cell)
    }
}

fn hud_pressable(latched: bool, enabled: bool, id: u64) -> Pressable {
    Pressable::new()
        .tone(if latched { Tone::Latch } else { Tone::Ghost })
        .held(latched)
        .enabled(enabled)
        .focusable(id)
}

/// Draw one key; true when it was activated.
fn draw_key(key: &HudKey, id: u64, rect: Rect, ctx: &mut DrawContext) -> bool {
    hud_pressable(key.on, key.enabled, id)
        .draw(rect, ctx, |state, ctx| {
            let s = ctx.styles();
            let color = key_ink(&s, key.on, state.hovered, state.down);
            let font = if key.mono {
                s.theme().mono_font.clone()
            } else {
                s.theme().font.clone()
            };
            let list = &mut *ctx.draw_list;
            let face = state.face;
            let width = list
                .measure_text_with_font(key.label, LABEL_SIZE, None, font.as_ref())
                .0;
            let y =
                list.vcentered_text_y(face.y, face.height, LABEL_SIZE, font.as_ref(), key.label);
            list.text(
                TextBlock::new(key.label, face.x + (face.width - width) * 0.5, y)
                    .with_size(LABEL_SIZE)
                    .with_weight(crate::Weight::MEDIUM)
                    .with_color_f32(color)
                    .with_font_opt(font),
            );
        })
        .clicked
}

/// Draw a select's key; true when it was activated.
fn draw_select(select: &HudSelect, open: bool, id: u64, rect: Rect, ctx: &mut DrawContext) -> bool {
    hud_pressable(open, true, id)
        .draw(rect, ctx, |state, ctx| {
            let s = ctx.styles();
            let ink = key_ink(&s, open, state.hovered, state.down);
            let list = &mut *ctx.draw_list;
            let face = state.face;
            let mut x = face.x + 1.0 + TEXT_PAD;
            if let Some(caption) = select.caption {
                let color = if open { ink } else { s.ink(Ink::Caption) };
                let y = list.vcentered_text_y(
                    face.y,
                    face.height,
                    CAPTION_SIZE,
                    s.theme().mono_font.as_ref(),
                    "G",
                );
                let block = caption_text(&s, caption, x, y, color);
                x += list.measure_block(&block).0 + SELECT_GAP;
                list.text(block);
            }
            let value = select.value();
            let font = s.theme().font.clone();
            let y = list.vcentered_text_y(face.y, face.height, LABEL_SIZE, font.as_ref(), value);
            let value_width = list
                .measure_text_with_font(value, LABEL_SIZE, None, font.as_ref())
                .0;
            list.text(
                TextBlock::new(value, x, y)
                    .with_size(LABEL_SIZE)
                    .with_weight(crate::Weight::MEDIUM)
                    .with_color_f32(ink)
                    .with_font_opt(font),
            );
            x += value_width + SELECT_GAP;
            // ▾ at 70 % opacity.
            let cy = face.y + face.height * 0.5;
            let mut chevron_ink = ink;
            chevron_ink[3] *= 0.7;
            list.stroke_polyline(
                &[
                    [x, cy - CHEVRON * 0.5],
                    [x + CHEVRON, cy + CHEVRON * 0.5],
                    [x + CHEVRON * 2.0, cy - CHEVRON * 0.5],
                ],
                &Stroke::new(1.25),
                chevron_ink,
            );
        })
        .clicked
}

/// A separator's rule: dark, with a faint light line beside it.
fn draw_separator(list: &mut DrawList, rect: Rect, vertical: bool) {
    let dark = [0.0, 0.0, 0.0, 0.6];
    let light = [1.0, 1.0, 1.0, 0.06];
    if vertical {
        let r = Rect::new(
            rect.x + SEP_MARGIN_ACROSS,
            rect.y + SEP_MARGIN_ALONG,
            rect.width - SEP_MARGIN_ACROSS * 2.0,
            1.0,
        );
        list.quad(r.x, r.y, r.width, 1.0, dark);
        list.quad(r.x, r.y + 1.0, r.width, 1.0, light);
    } else {
        let r = Rect::new(
            rect.x + SEP_MARGIN_ALONG,
            rect.y + SEP_MARGIN_ACROSS,
            1.0,
            rect.height - SEP_MARGIN_ACROSS * 2.0,
        );
        list.quad(r.x, r.y, 1.0, r.height, dark);
        list.quad(r.x + 1.0, r.y, 1.0, r.height, light);
    }
}

/// An option of the open sheet, copied when it opened (the bar's items are
/// borrowed for one frame only).
#[derive(Clone, Debug, PartialEq, Eq)]
struct SheetOption {
    label: String,
    shortcut: Option<String>,
    enabled: bool,
    reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct OpenSelect {
    bar: HudBarId,
    item: usize,
    selected: usize,
    options: Vec<SheetOption>,
}

/// The open [`HudSelect`] sheet of one surface, shared by all its bars (at
/// most one sheet is open). Caller-owned; persists across frames. See the
/// [module docs](self) for the per-frame calls.
#[derive(Clone, Debug, Default)]
pub struct HudState {
    menu: ContextMenuState,
    open: Option<OpenSelect>,
}

impl HudState {
    /// Nothing open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture the navigation an open sheet acts on. Call once per frame,
    /// before `FocusState::begin_frame`.
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.menu.begin_frame(input);
        if !self.menu.is_open() {
            self.open = None;
        }
    }

    /// True while item `item` of bar `bar` has its sheet open.
    pub fn is_open(&self, bar: HudBarId, item: usize) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.bar == bar && o.item == item)
    }

    /// Close any open sheet.
    pub fn close(&mut self) {
        self.menu.close();
        self.open = None;
    }

    /// Open `select`'s sheet beside its key at `key` (screen space).
    fn open(&mut self, bar: HudBarId, item: usize, select: &HudSelect, key: Rect) {
        self.open = Some(OpenSelect {
            bar,
            item,
            selected: select.selected,
            options: select
                .options
                .iter()
                .map(|o| SheetOption {
                    label: o.label.to_string(),
                    shortcut: o.shortcut.map(str::to_string),
                    enabled: o.enabled,
                    reason: o.reason.map(str::to_string),
                })
                .collect(),
        });
        if select.up {
            self.menu.open_above(key.x, key.y - SHEET_GAP);
        } else {
            self.menu.open_at(key.x, key.bottom() + SHEET_GAP);
        }
    }

    /// Push the open sheet's layer before base input is dispatched (so it
    /// blocks the widgets under it). Returns its layer index, or `None` while
    /// closed.
    pub fn push_open_layer(
        &mut self,
        layers: &mut LayerStack,
        styles: &StyleResolver<'_>,
        viewport: Rect,
    ) -> Option<usize> {
        let open = self.open.as_ref()?;
        let items = sheet_items(open);
        let menu = ContextMenu::new(&items).with_min_width(SHEET_MIN_WIDTH);
        self.menu.push_open_layer(layers, &menu, styles, viewport)
    }

    /// Draw the open sheet and resolve a choice. Call after the base widgets.
    /// Returns the choice and closes the sheet when an option is picked; an
    /// outside click or Escape closes it with no choice.
    pub fn draw_open_layer(
        &mut self,
        layers: &mut LayerStack,
        layer: Option<usize>,
        styles: &StyleResolver<'_>,
        input: &InputState,
        focus: &mut FocusState,
        viewport: Rect,
    ) -> Option<HudChoice> {
        let open = self.open.as_ref()?;
        let items = sheet_items(open);
        let menu = ContextMenu::new(&items).with_min_width(SHEET_MIN_WIDTH);
        let chosen = self
            .menu
            .draw_open_layer(layers, layer, &menu, styles, input, focus, viewport)
            .map(|activated| HudChoice {
                bar: open.bar,
                item: open.item,
                option: activated.id as usize,
            });
        if !self.menu.is_open() {
            self.open = None;
        }
        chosen
    }

    /// Test/seed helper: open a select's sheet so it renders this frame.
    #[doc(hidden)]
    pub fn open_for_test(&mut self, bar: HudBarId, item: usize, select: &HudSelect, key: Rect) {
        self.open(bar, item, select, key);
    }
}

/// The open sheet as menu rows: radio items whose ids are their indices.
fn sheet_items(open: &OpenSelect) -> Vec<MenuItem<'_>> {
    open.options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let mut item = MenuItem::new(&o.label)
                .id(i as u64)
                .checked(i == open.selected)
                .enabled(o.enabled);
            if let Some(shortcut) = &o.shortcut {
                item = item.shortcut(shortcut);
            }
            if let Some(reason) = &o.reason {
                item = item.reason(reason);
            }
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    const BAR: HudBarId = 7;
    const SHADING: &[HudOption<'static>] = &[
        HudOption::new("Lit"),
        HudOption::new("Unlit").shortcut("Alt 2"),
        HudOption::new("Lightmap density").short("Density"),
        HudOption::new("Path traced").disabled("Needs RTX"),
    ];

    fn at(x: f32, y: f32, clicked: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: clicked,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    fn away() -> InputState {
        at(700.0, 500.0, false)
    }

    /// Draw `items` as a bar at (10, 10) under `input`.
    fn draw(
        items: &[HudItem],
        vertical: bool,
        hud: &mut HudState,
        input: &InputState,
    ) -> (DrawList, HudBarOutput) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        let mut bar = HudBar::new(BAR, items);
        if vertical {
            bar = bar.vertical();
        }
        let out = bar.draw([10.0, 10.0], hud, &mut ctx);
        (list, out)
    }

    fn travel() -> f32 {
        let theme = Theme::default();
        Pressable::new().travel_px(&StyleResolver::new(&theme))
    }

    #[test]
    fn glyph_keys_are_square_and_the_bar_wraps_them_in_padding() {
        let items = [
            HudItem::Key(HudKey::new("#")),
            HudItem::Key(HudKey::new("W")),
        ];
        let (_, out) = draw(&items, false, &mut HudState::new(), &away());
        let t = travel();
        assert_eq!(out.rect.width, PAD * 2.0 + FACE * 2.0 + GAP);
        assert_eq!(out.rect.height, PAD * 2.0 + FACE + t);

        let (_, out) = draw(&items, true, &mut HudState::new(), &away());
        assert_eq!(out.rect.width, PAD * 2.0 + FACE);
        assert_eq!(out.rect.height, PAD * 2.0 + (FACE + t) * 2.0 + GAP);
    }

    #[test]
    fn size_matches_what_draw_covers() {
        let items = [
            HudItem::Select(HudSelect::new(SHADING, 2).caption("View")),
            HudItem::Separator,
            HudItem::Key(HudKey::new("Snap").on(true)),
            HudItem::Key(HudKey::new("fps").mono().width(40.0)),
        ];
        let theme = Theme::default();
        let mut list = DrawList::new();
        let size = HudBar::new(BAR, &items).size(&mut list, &StyleResolver::new(&theme));
        let (_, out) = draw(&items, false, &mut HudState::new(), &away());
        assert_eq!([out.rect.width, out.rect.height], size);
        // A text key outgrows a glyph key, and a fixed width wins.
        assert!(size[0] > PAD * 2.0 + FACE * 3.0 + 40.0);
    }

    #[test]
    fn a_select_shows_the_short_label_of_its_choice() {
        assert_eq!(HudSelect::new(SHADING, 2).value(), "Density");
        assert_eq!(HudSelect::new(SHADING, 1).value(), "Unlit");
        assert_eq!(HudSelect::new(SHADING, 9).value(), "", "out of range");
    }

    #[test]
    fn a_click_on_a_key_reports_its_item_index() {
        let items = [
            HudItem::Key(HudKey::new("#")),
            HudItem::Separator,
            HudItem::Key(HudKey::new("W")),
        ];
        // Second key: past the first, both gaps and the separator.
        let x = 10.0 + PAD + FACE + GAP + 1.0 + SEP_MARGIN_ALONG * 2.0 + GAP + FACE * 0.5;
        let y = 10.0 + PAD + FACE * 0.5;
        let (_, out) = draw(&items, false, &mut HudState::new(), &at(x, y, true));
        assert_eq!(out.clicked, Some(2));
        assert!(out.hovered);

        let (_, out) = draw(
            &items,
            false,
            &mut HudState::new(),
            &at(10.0 + PAD + 1.0, y, true),
        );
        assert_eq!(out.clicked, Some(0));

        let mut consumed = at(x, y, true);
        consumed.mouse_consumed = true;
        let (_, out) = draw(&items, false, &mut HudState::new(), &consumed);
        assert_eq!(out.clicked, None, "a layer above has the pointer");
        assert!(!out.hovered);
    }

    #[test]
    fn a_disabled_key_does_not_click() {
        let items = [HudItem::Key(HudKey::new("#").enabled(false))];
        let (_, out) = draw(
            &items,
            false,
            &mut HudState::new(),
            &at(10.0 + PAD + 5.0, 20.0, true),
        );
        assert_eq!(out.clicked, None);
    }

    #[test]
    fn clicking_a_select_toggles_its_sheet_without_reporting_a_click() {
        let items = [HudItem::Select(HudSelect::new(SHADING, 0))];
        let mut hud = HudState::new();
        let press = at(10.0 + PAD + 5.0, 10.0 + PAD + 5.0, true);
        let (_, out) = draw(&items, false, &mut hud, &press);
        assert_eq!(out.clicked, None);
        assert!(hud.is_open(BAR, 0));
        assert!(!hud.is_open(BAR, 1) && !hud.is_open(BAR + 1, 0));

        let (_, _) = draw(&items, false, &mut hud, &press);
        assert!(!hud.is_open(BAR, 0), "a second click closes it");
    }

    /// Run the layer half of a frame with `input`; returns the sheet rect
    /// and any choice.
    fn sheet_frame(hud: &mut HudState, input: &InputState) -> (Option<Rect>, Option<HudChoice>) {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut captured = input.clone();
        hud.begin_frame(&mut captured);
        let mut layers = LayerStack::new();
        let layer = hud.push_open_layer(&mut layers, &styles, viewport);
        let rect = hud.menu.rect();
        let mut focus = FocusState::new();
        let choice = hud.draw_open_layer(&mut layers, layer, &styles, input, &mut focus, viewport);
        (rect, choice)
    }

    #[test]
    fn the_sheet_drops_below_the_key_or_rises_above_it() {
        let key = Rect::new(40.0, 300.0, 60.0, 21.0);
        let mut hud = HudState::new();
        hud.open_for_test(BAR, 0, &HudSelect::new(SHADING, 0), key);
        let (rect, _) = sheet_frame(&mut hud, &away());
        let rect = rect.expect("open");
        assert_eq!(rect.x, key.x);
        assert_eq!(rect.y, key.bottom() + SHEET_GAP);
        assert!(rect.width >= SHEET_MIN_WIDTH);

        let mut hud = HudState::new();
        hud.open_for_test(BAR, 0, &HudSelect::new(SHADING, 0).up(), key);
        let (rect, _) = sheet_frame(&mut hud, &away());
        assert_eq!(rect.expect("open").bottom(), key.y - SHEET_GAP);
    }

    #[test]
    fn picking_an_option_reports_bar_item_and_option_and_closes() {
        let theme = Theme::default();
        let key = Rect::new(40.0, 40.0, 60.0, 21.0);
        let mut hud = HudState::new();
        hud.open_for_test(BAR, 3, &HudSelect::new(SHADING, 0), key);
        let (rect, choice) = sheet_frame(&mut hud, &away());
        assert_eq!(choice, None);
        let rect = rect.unwrap();
        let row = theme.menu_row_height;
        let pick = at(rect.x + rect.width * 0.5, rect.y + 3.0 + row * 1.5, true);
        let (_, choice) = sheet_frame(&mut hud, &pick);
        assert_eq!(
            choice,
            Some(HudChoice {
                bar: BAR,
                item: 3,
                option: 1
            })
        );
        assert!(!hud.is_open(BAR, 3));
    }

    #[test]
    fn the_sheet_marks_the_choice_and_carries_hints() {
        let mut hud = HudState::new();
        hud.open_for_test(
            BAR,
            0,
            &HudSelect::new(SHADING, 2),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let open = hud.open.as_ref().unwrap();
        let items = sheet_items(open);
        let checked: Vec<bool> = items.iter().map(MenuItem::is_checked).collect();
        assert_eq!(checked, [false, false, true, false]);
        assert!(!items[3].is_enabled());
        assert_eq!(items[1].shortcut_text(), Some("Alt 2"));
    }

    #[test]
    fn an_outside_click_closes_the_sheet_with_no_choice() {
        let mut hud = HudState::new();
        hud.open_for_test(
            BAR,
            0,
            &HudSelect::new(SHADING, 0),
            Rect::new(40.0, 40.0, 60.0, 21.0),
        );
        let (_, choice) = sheet_frame(&mut hud, &at(700.0, 500.0, true));
        assert_eq!(choice, None);
        assert!(!hud.is_open(BAR, 0));
    }

    #[test]
    fn a_latched_key_wears_the_latch_face() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let off = draw(
            &[HudItem::Key(HudKey::new("#"))],
            false,
            &mut HudState::new(),
            &away(),
        )
        .0;
        let on = draw(
            &[HudItem::Key(HudKey::new("#").on(true))],
            false,
            &mut HudState::new(),
            &away(),
        )
        .0;
        let ink = |list: &DrawList| list.texts.last().map(|b| b.color);
        let text = crate::color::text_color;
        assert_eq!(ink(&on), Some(text(s.color(crate::StyleKey::OnLatch))));
        assert_eq!(ink(&off), Some(text(s.ink(Ink::Cell))));
    }
}
