//! The token inspector: every keyed theme value, grouped by what moves it,
//! with its resolved value and whether the preset overrides it. The selected
//! token opens an editor that overrides what the seeds derive.

use std::fmt::Write as _;

use wgpu_gameui::color::to_rgba8;
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Button, ColorFamily, ColorPicker, CursorIcon, DragCapture, DrawContext, FieldLabel, Hsva, Ink,
    NumberInput, ScrollState, SearchField, StyleKey, StyleValue, TextInput, TextSize, Theme,
    ThemePreset, Tone, Tracking,
};

use crate::fields::{self, LABEL_GAP};

/// Space around the panel's content.
const PAD: f32 = 10.0;
/// A token row's height.
const ROW_H: f32 = 22.0;
/// A group heading's height.
const HEADING_H: f32 = 28.0;
/// A colour row's swatch.
const SWATCH: f32 = 12.0;
/// The colour editor's picker height.
const PICKER_H: f32 = 150.0;
/// The Reset key's width.
const RESET_W: f32 = 64.0;

/// Focus and drag ids: one block, distinct from the other panels'.
const ID_FILTER: u64 = 0x70CE_0001;
const ID_PICKER: u64 = 0x70CE_0002;
const ID_HEX: u64 = 0x70CE_0003;
const ID_NUMBER: u64 = 0x70CE_0004;
const ID_RESET: u64 = 0x70CE_0005;

/// Caller-owned state of the token panel.
pub struct TokensPanel {
    rows: Vec<(TokenGroup, StyleKey)>,
    filter: TextInput,
    scroll: ScrollState,
    selected: Option<StyleKey>,
    /// The colour editor's picker state; kept apart from the stored colour
    /// so hue survives a trip through grey and 8-bit storage.
    hsva: Hsva,
    hex: TextInput,
    number: TextInput,
    /// Reused for value readouts.
    scratch: String,
}

impl TokensPanel {
    /// The panel for themes derived from `template` (Forge).
    pub fn new(template: &Theme) -> Self {
        Self {
            rows: token_rows(template),
            filter: TextInput::new(0.0, 0.0, 0.0, 0.0).with_placeholder("Filter tokens"),
            scroll: ScrollState::new(),
            selected: None,
            hsva: Hsva::opaque(0.0, 0.0, 0.0),
            hex: TextInput::new(0.0, 0.0, 0.0, 0.0),
            number: TextInput::new(0.0, 0.0, 0.0, 0.0),
            scratch: String::new(),
        }
    }

    /// When the panel next needs a frame on its own (a scroll glide).
    pub fn pending_deadline(&self) -> Option<f32> {
        self.scroll.pending_deadline()
    }

    /// Open `key` in the editor.
    pub fn select(&mut self, key: StyleKey) {
        self.selected = Some(key);
    }

    /// Draw the panel into `rect`. `theme` is `preset` as resolved; editing a
    /// token sets or clears its override in `preset`.
    pub fn draw(
        &mut self,
        rect: Rect,
        preset: &mut ThemePreset,
        theme: &Theme,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) {
        let search = Rect::new(
            rect.x + PAD,
            rect.y + PAD,
            (rect.width - 2.0 * PAD).max(0.0),
            SearchField::HEIGHT,
        );
        if SearchField::new().draw(&mut self.filter, ID_FILTER, search, ctx) {
            self.scroll.snap_to(1, 0.0);
        }

        let editor_h = self.selected.map_or(0.0, |key| editor_height(key, ctx));
        let list_top = search.bottom() + PAD;
        let list = Rect::new(
            rect.x,
            list_top,
            rect.width,
            (rect.bottom() - list_top - editor_h).max(0.0),
        );
        self.draw_list(list, preset, theme, ctx);

        if let Some(key) = self.selected {
            let editor = Rect::new(rect.x, list.bottom(), rect.width, editor_h);
            self.draw_editor(key, editor, preset, theme, capture, ctx);
        }
    }

    /// The grouped, filtered rows; a click selects one.
    fn draw_list(
        &mut self,
        rect: Rect,
        preset: &ThemePreset,
        theme: &Theme,
        ctx: &mut DrawContext,
    ) {
        let filter = self.filter.value.trim().to_ascii_lowercase();
        let Self {
            rows,
            scroll,
            selected,
            scratch,
            ..
        } = self;
        fields::scrolled(rect, scroll, ctx, |inner, visible, ctx| {
            let s = ctx.styles();
            let mut y = inner.y;
            let mut group = None;
            for &(row_group, key) in rows.iter() {
                if !filter.is_empty() && !key.name().contains(filter.as_str()) {
                    continue;
                }
                if group != Some(row_group) {
                    group = Some(row_group);
                    if y + HEADING_H > visible.y && y < visible.bottom() {
                        let block = s.caption_block(
                            row_group.title(),
                            inner.x + PAD,
                            y + HEADING_H - 16.0,
                            Tracking::Caption,
                            Ink::Second,
                        );
                        ctx.draw_list.text(block);
                    }
                    y += HEADING_H;
                }
                let row = Rect::new(inner.x, y, inner.width, ROW_H);
                y += ROW_H;
                if row.bottom() <= visible.y || row.y >= visible.bottom() {
                    continue;
                }

                let input = ctx.input;
                let hovered = !input.mouse_consumed && row.contains(input.mouse_x, input.mouse_y);
                if hovered {
                    ctx.request_cursor(CursorIcon::Pointer);
                    if input.mouse_clicked {
                        *selected = Some(key);
                    }
                }
                if *selected == Some(key) {
                    ctx.draw_list.quad(
                        row.x,
                        row.y,
                        row.width,
                        row.height,
                        s.color(StyleKey::RowHeld),
                    );
                } else if hovered {
                    ctx.draw_list.quad(
                        row.x,
                        row.y,
                        row.width,
                        row.height,
                        s.color(StyleKey::RowHover),
                    );
                }
                draw_row(
                    key,
                    row,
                    preset.override_of(key).is_some(),
                    theme,
                    scratch,
                    ctx,
                );
            }
            y - inner.y + PAD
        });
    }

    /// The selected token's editor: a picker and hex field for a colour, a
    /// number field for a size, and Reset.
    fn draw_editor(
        &mut self,
        key: StyleKey,
        rect: Rect,
        preset: &mut ThemePreset,
        theme: &Theme,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) {
        let s = ctx.styles();
        let edge = s.color(StyleKey::PanelBorder);
        ctx.draw_list.quad(rect.x, rect.y, rect.width, 1.0, edge);

        let x = rect.x + PAD;
        let width = (rect.width - 2.0 * PAD).max(0.0);
        let overridden = preset.override_of(key).is_some();
        let state = if overridden { "override" } else { "derived" };
        let label = FieldLabel::new(key.name()).value(state).draw(
            ctx.draw_list,
            &s,
            x,
            rect.y + PAD,
            width,
        );
        let y = label.bottom() + LABEL_GAP;
        let row_h = s.scalar(StyleKey::InputHeight);
        let field_w = (width - RESET_W - 8.0).max(0.0);
        let reset = Rect::new(x + width - RESET_W, y, RESET_W, row_h);

        match theme.get(key) {
            Some(StyleValue::Color(current)) => {
                if to_rgba8(self.hsva.to_rgba()) != to_rgba8(current) {
                    self.hsva = Hsva::from_rgba(current);
                }
                self.scratch.clear();
                fields::hex(&mut self.scratch, current);
                fields::sync_text(&mut self.hex, ID_HEX, &self.scratch, ctx.focus);
                fields::draw_text(&mut self.hex, ID_HEX, Rect::new(x, y, field_w, row_h), ctx);
                if ctx.focus.is_focused(ID_HEX)
                    && let Some(typed) = fields::parse_hex(&self.hex.value)
                    && to_rgba8(typed) != to_rgba8(current)
                {
                    preset.set_override(key, StyleValue::Color(typed));
                }

                let picker = Rect::new(x, y + row_h + PAD, width, PICKER_H);
                let out = ColorPicker::new()
                    .with_alpha(true)
                    .draw(self.hsva, ID_PICKER, capture, picker, ctx);
                if out.changed {
                    self.hsva = out.hsva;
                    preset.set_override(key, StyleValue::Color(out.rgba));
                }
            }
            Some(StyleValue::Scalar(current)) => {
                let out = NumberInput::new()
                    .with_decimals(2)
                    .with_step(0.5)
                    .with_range(0.0, 10_000.0)
                    .draw(
                        current as f64,
                        ID_NUMBER,
                        &mut self.number,
                        Rect::new(x, y, field_w, row_h),
                        ctx,
                    );
                if out.changed {
                    preset.set_override(key, StyleValue::Scalar(out.value as f32));
                }
            }
            None => {}
        }

        if overridden
            && Button::new("Reset")
                .tone(Tone::Ghost)
                .focusable(ID_RESET)
                .draw(reset, ctx)
        {
            preset.clear_override(key);
        }
    }
}

/// The editor's height for `key`.
fn editor_height(key: StyleKey, ctx: &mut DrawContext) -> f32 {
    let s = ctx.styles();
    let fixed = FieldLabel::height(ctx.draw_list, &s) + LABEL_GAP + s.scalar(StyleKey::InputHeight);
    let picker = if key.is_color() { PAD + PICKER_H } else { 0.0 };
    PAD + fixed + picker + PAD
}

/// One token: the override mark, a swatch for a colour, the name and the
/// value.
fn draw_row(
    key: StyleKey,
    row: Rect,
    overridden: bool,
    theme: &Theme,
    scratch: &mut String,
    ctx: &mut DrawContext,
) {
    ctx.push_debug_scope_rect(key.name(), row);
    let s = ctx.styles();
    let mid = row.y + row.height * 0.5;
    if overridden {
        ctx.draw_list.circle(
            (row.x + PAD * 0.5, mid),
            2.5,
            s.color(StyleKey::AccentDirty),
        );
    }
    let mut x = row.x + PAD;
    scratch.clear();
    match theme.get(key) {
        Some(StyleValue::Color(color)) => {
            let r = Rect::new(x, (mid - SWATCH * 0.5).round(), SWATCH, SWATCH);
            ctx.draw_list
                .rounded_rect(r, 2.0, s.color(StyleKey::EdgeHard));
            ctx.draw_list.rounded_rect(r.inset(1.0), 1.0, color);
            x += SWATCH + 6.0;
            fields::hex(scratch, color);
        }
        Some(StyleValue::Scalar(value)) => number(scratch, value),
        None => {}
    }

    let role = if overridden {
        Ink::Primary
    } else {
        Ink::Second
    };
    let mut name = s.mono_block(key.name(), x, 0.0, TextSize::Dense, role);
    let text_y = (mid - ctx.draw_list.measure_block(&name).1 * 0.5).round();
    name.y = text_y;
    ctx.draw_list.text(name);

    let value_w = s.mono_width(ctx.draw_list, scratch, TextSize::Dense);
    let mut value = s.mono_block(scratch.as_str(), 0.0, text_y, TextSize::Dense, Ink::Dim);
    value.x = row.right() - PAD - value_w;
    ctx.draw_list.text(value);
    ctx.pop_debug_scope();
}

/// A size as a person would write it: `4`, `1.5`, `0.12`.
fn number(out: &mut String, value: f32) {
    let _ = write!(out, "{value:.2}");
    let trimmed = out.trim_end_matches('0').trim_end_matches('.').len();
    out.truncate(trimmed);
}

/// What a token's row is filed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenGroup {
    /// A colour one seed moves.
    Family(ColorFamily),
    /// Pure white, black or grey: light and shade no seed tints.
    Shade,
    /// Paddings, heights, radii and edge widths.
    Sizes,
    /// Font sizes and letter spacing.
    Type,
    /// Durations.
    Motion,
}

impl TokenGroup {
    /// Every group, in the order the inspector lists them.
    pub fn all() -> impl Iterator<Item = TokenGroup> {
        ColorFamily::ALL.into_iter().map(TokenGroup::Family).chain([
            TokenGroup::Shade,
            TokenGroup::Sizes,
            TokenGroup::Type,
            TokenGroup::Motion,
        ])
    }

    /// The group's heading.
    pub fn title(self) -> &'static str {
        match self {
            TokenGroup::Family(ColorFamily::Surface) => "Surface colours",
            TokenGroup::Family(ColorFamily::Ink) => "Ink colours",
            TokenGroup::Family(ColorFamily::Accent) => "Accent colours",
            TokenGroup::Family(ColorFamily::Danger) => "Danger colours",
            TokenGroup::Family(ColorFamily::Warning) => "Warning colours",
            TokenGroup::Family(ColorFamily::Success) => "Success colours",
            TokenGroup::Family(ColorFamily::Info) => "Info colours",
            TokenGroup::Shade => "Light and shade",
            TokenGroup::Sizes => "Sizes",
            TokenGroup::Type => "Type",
            TokenGroup::Motion => "Motion",
        }
    }

    /// The group a key belongs to, judged by its value in `template`
    /// (Forge), the way the seeds judge it.
    pub fn of(key: StyleKey, template: &Theme) -> TokenGroup {
        if key.is_color() {
            return ColorFamily::of_key(key, template)
                .map_or(TokenGroup::Shade, TokenGroup::Family);
        }
        match key {
            StyleKey::FontSize
            | StyleKey::FontSizeTitle
            | StyleKey::TextSize(_)
            | StyleKey::Tracking(_) => TokenGroup::Type,
            StyleKey::AnimationDuration | StyleKey::MenuHoverDelay => TokenGroup::Motion,
            _ => TokenGroup::Sizes,
        }
    }
}

/// The inspector's rows: every built-in key, grouped, in [`TokenGroup::all`]
/// order and declaration order inside a group. Built once; the template
/// never changes.
pub fn token_rows(template: &Theme) -> Vec<(TokenGroup, StyleKey)> {
    let keyed: Vec<(TokenGroup, StyleKey)> = StyleKey::builtin()
        .map(|key| (TokenGroup::of(key, template), key))
        .collect();
    TokenGroup::all()
        .flat_map(|group| keyed.iter().copied().filter(move |(g, _)| *g == group))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_listed_once() {
        let template = Theme::default();
        let rows = token_rows(&template);
        assert_eq!(rows.len(), StyleKey::builtin().count());
        assert!(rows.contains(&(TokenGroup::Family(ColorFamily::Accent), StyleKey::Accent)));
        assert!(rows.contains(&(TokenGroup::Shade, StyleKey::FaceTop)));
        assert!(rows.contains(&(TokenGroup::Type, StyleKey::FontSize)));
        assert!(rows.contains(&(TokenGroup::Sizes, StyleKey::Padding)));
    }

    #[test]
    fn numbers_drop_trailing_zeros() {
        for (value, text) in [
            (0.0, "0"),
            (4.0, "4"),
            (100.0, "100"),
            (1.5, "1.5"),
            (0.12, "0.12"),
        ] {
            let mut out = String::new();
            number(&mut out, value);
            assert_eq!(out, text);
        }
    }
}
