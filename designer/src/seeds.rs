//! The seed editor: the handful of values the whole theme derives from —
//! one colour seed per family, the three scales and the fonts.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Button, ColorFamily, ColorSeed, DragCapture, DrawContext, FieldLabel, Ink, ScrollState,
    StyleKey, StyleValue, TextInput, Theme, ThemePreset, Tone, Tracking,
};

use crate::fields::{self, LABEL_GAP, SliderRow};

/// Space around the column.
const PAD: f32 = 12.0;
/// Space between two fields.
const GAP: f32 = 10.0;
/// Space above a section heading.
const SECTION_GAP: f32 = 18.0;
/// The family swatches' size.
const SWATCH: f32 = 14.0;
/// At most this many swatches per family.
const MAX_SWATCHES: usize = 10;
/// The Reset key's size.
const RESET_W: f32 = 56.0;
const RESET_H: f32 = 22.0;

/// Focus and drag ids: one block, distinct from the other panels'.
const ID_NAME: u64 = 0x5EED_0001;
const ID_SANS: u64 = 0x5EED_0002;
const ID_MONO: u64 = 0x5EED_0003;
const ID_DENSITY: u64 = 0x5EED_0010;
const ID_TEXT_SCALE: u64 = 0x5EED_0011;
const ID_RADIUS: u64 = 0x5EED_0012;
/// Family `f`'s ids are `ID_FAMILY + 8 * f + n`.
const ID_FAMILY: u64 = 0x5EED_0100;

/// Caller-owned state of the seed column.
pub struct SeedsPanel {
    scroll: ScrollState,
    name: TextInput,
    sans: TextInput,
    mono: TextInput,
    /// Per family, keys whose Forge colours differ: the swatch strip.
    swatches: [Vec<StyleKey>; 7],
    /// Reused for the value readouts.
    scratch: String,
}

impl SeedsPanel {
    /// The column for themes derived from `template` (Forge).
    pub fn new(template: &Theme) -> Self {
        let swatches = ColorFamily::ALL.map(|family| {
            let mut keys: Vec<StyleKey> = Vec::new();
            let mut seen: Vec<[f32; 4]> = Vec::new();
            for key in StyleKey::builtin() {
                if keys.len() == MAX_SWATCHES {
                    break;
                }
                if ColorFamily::of_key(key, template) != Some(family) {
                    continue;
                }
                let Some(color) = template.get(key).and_then(StyleValue::as_color) else {
                    continue;
                };
                if !seen.contains(&color) {
                    seen.push(color);
                    keys.push(key);
                }
            }
            keys
        });
        let field = || TextInput::new(0.0, 0.0, 0.0, 0.0);
        Self {
            scroll: ScrollState::new(),
            name: field(),
            sans: field().with_placeholder("Forge default (IBM Plex Sans)"),
            mono: field().with_placeholder("Forge default (IBM Plex Mono)"),
            swatches,
            scratch: String::new(),
        }
    }

    /// When the column next needs a frame on its own (a scroll glide).
    pub fn pending_deadline(&self) -> Option<f32> {
        self.scroll.pending_deadline()
    }

    /// Draw the column into `rect`, editing `preset`'s name and seeds.
    /// `theme` is the preset as resolved, for the swatches.
    pub fn draw(
        &mut self,
        rect: Rect,
        preset: &mut ThemePreset,
        theme: &Theme,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) {
        fields::sync_text(&mut self.name, ID_NAME, &preset.name, ctx.focus);
        let sans = preset.seeds.sans_font.as_deref().unwrap_or("");
        fields::sync_text(&mut self.sans, ID_SANS, sans, ctx.focus);
        let mono = preset.seeds.mono_font.as_deref().unwrap_or("");
        fields::sync_text(&mut self.mono, ID_MONO, mono, ctx.focus);

        let Self {
            scroll,
            name,
            sans,
            mono,
            swatches,
            scratch,
        } = self;
        fields::scrolled(rect, scroll, ctx, |inner, _, ctx| {
            let x = inner.x + PAD;
            let width = (inner.width - 2.0 * PAD).max(0.0);
            let mut y = inner.y + PAD;

            y = heading("Preset", x, y, ctx);
            y = text_field("Name", name, ID_NAME, x, y, width, ctx);
            if ctx.focus.is_focused(ID_NAME) && name.value != preset.name {
                preset.name.clone_from(&name.value);
            }

            y = heading("Scale", x + 0.0, y + SECTION_GAP, ctx);
            let seeds = &mut preset.seeds;
            let scales = [
                (
                    SliderRow {
                        label: "Density",
                        min: 0.6,
                        max: 1.6,
                        step: 0.05,
                    },
                    ID_DENSITY,
                ),
                (
                    SliderRow {
                        label: "Text size",
                        min: 0.7,
                        max: 1.6,
                        step: 0.05,
                    },
                    ID_TEXT_SCALE,
                ),
                (
                    SliderRow {
                        label: "Corner radius",
                        min: 0.0,
                        max: 8.0,
                        step: 0.25,
                    },
                    ID_RADIUS,
                ),
            ];
            for (row, id) in scales {
                let value = match id {
                    ID_DENSITY => &mut seeds.density,
                    ID_TEXT_SCALE => &mut seeds.text_scale,
                    _ => &mut seeds.radius_scale,
                };
                let (moved, bottom) = row.draw(
                    *value,
                    fields::times,
                    scratch,
                    id,
                    x,
                    y,
                    width,
                    capture,
                    ctx,
                );
                if let Some(v) = moved {
                    *value = v;
                }
                y = bottom + GAP;
            }

            y = heading("Fonts", x, y - GAP + SECTION_GAP, ctx);
            y = text_field("Sans family", sans, ID_SANS, x, y, width, ctx);
            bind_font(&mut seeds.sans_font, sans, ID_SANS, ctx);
            y = text_field("Mono family", mono, ID_MONO, x, y + GAP, width, ctx);
            bind_font(&mut seeds.mono_font, mono, ID_MONO, ctx);

            for (index, family) in ColorFamily::ALL.into_iter().enumerate() {
                let base = ID_FAMILY + 8 * index as u64;
                y = family_block(
                    family,
                    seeds.color_mut(family),
                    &swatches[index],
                    theme,
                    base,
                    (x, y + SECTION_GAP, width),
                    scratch,
                    capture,
                    ctx,
                );
            }
            y + PAD - inner.y
        });
    }
}

/// A section heading at `(x, y)`; returns where the section's content starts.
fn heading(text: &str, x: f32, y: f32, ctx: &mut DrawContext) -> f32 {
    let s = ctx.styles();
    let block = s.caption_block(text, x, y, Tracking::Caption, Ink::Second);
    let height = ctx.draw_list.measure_block(&block).1.ceil();
    ctx.draw_list.text(block);
    y + height + GAP
}

/// A labelled one-line text field; returns its bottom edge.
fn text_field(
    label: &str,
    field: &mut TextInput,
    id: u64,
    x: f32,
    y: f32,
    width: f32,
    ctx: &mut DrawContext,
) -> f32 {
    let s = ctx.styles();
    let label = FieldLabel::new(label).draw(ctx.draw_list, &s, x, y, width);
    let rect = Rect::new(
        x,
        label.bottom() + LABEL_GAP,
        width,
        s.scalar(StyleKey::InputHeight),
    );
    fields::draw_text(field, id, rect, ctx);
    rect.bottom()
}

/// Write a font field's edit back to its seed: blank means Forge's font.
fn bind_font(seed: &mut Option<String>, field: &TextInput, id: u64, ctx: &DrawContext) {
    if !ctx.focus.is_focused(id) {
        return;
    }
    let typed = field.value.trim();
    if seed.as_deref().unwrap_or("") != typed {
        *seed = (!typed.is_empty()).then(|| typed.to_string());
    }
}

/// One family's heading, swatches, Reset key and three sliders; returns the
/// block's bottom edge.
#[allow(clippy::too_many_arguments)]
fn family_block(
    family: ColorFamily,
    seed: &mut ColorSeed,
    swatches: &[StyleKey],
    theme: &Theme,
    id: u64,
    (x, y, width): (f32, f32, f32),
    scratch: &mut String,
    capture: &mut DragCapture,
    ctx: &mut DrawContext,
) -> f32 {
    let s = ctx.styles();
    let block = s.caption_block(family.name(), x, y, Tracking::Caption, Ink::Second);
    let caption_h = ctx.draw_list.measure_block(&block).1.ceil();
    let row_h = caption_h.max(RESET_H);
    let mut caption = block;
    caption.y = y + ((row_h - caption_h) * 0.5).round();
    ctx.draw_list.text(caption);

    let forge = ColorSeed::forge(family);
    let reset = Rect::new(x + width - RESET_W, y, RESET_W, RESET_H);
    if *seed != forge {
        let clicked = Button::new("Reset")
            .tone(Tone::Ghost)
            .focusable(id + 7)
            .draw(reset, ctx);
        if clicked {
            *seed = forge;
        }
    }

    // The family's colours as resolved, right-aligned before the Reset key.
    let strip_right = reset.x - 8.0;
    let mut sx = strip_right - swatches.len() as f32 * (SWATCH + 2.0);
    let sy = y + ((row_h - SWATCH) * 0.5).round();
    let edge = s.color(StyleKey::EdgeHard);
    for &key in swatches {
        if let Some(color) = theme.get(key).and_then(StyleValue::as_color) {
            let r = Rect::new(sx, sy, SWATCH, SWATCH);
            ctx.draw_list.rounded_rect(r, 2.0, edge);
            ctx.draw_list.rounded_rect(r.inset(1.0), 1.0, color);
        }
        sx += SWATCH + 2.0;
    }

    let mut y = y + row_h + GAP;
    let rows = [
        (
            SliderRow {
                label: "Hue",
                min: 0.0,
                max: 360.0,
                step: 1.0,
            },
            fields::degrees as fn(&mut String, f32),
        ),
        (
            SliderRow {
                label: "Chroma",
                min: 0.0,
                max: 4.0,
                step: 0.05,
            },
            fields::times,
        ),
        (
            SliderRow {
                label: "Lightness",
                min: -0.4,
                max: 0.4,
                step: 0.01,
            },
            fields::signed,
        ),
    ];
    for (n, (row, format)) in rows.into_iter().enumerate() {
        let value = match n {
            0 => &mut seed.hue,
            1 => &mut seed.chroma,
            _ => &mut seed.lightness,
        };
        let (moved, bottom) = row.draw(
            *value,
            format,
            scratch,
            id + n as u64,
            x,
            y,
            width,
            capture,
            ctx,
        );
        if let Some(v) = moved {
            *value = v;
        }
        y = bottom + GAP;
    }
    y - GAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_has_swatches() {
        let panel = SeedsPanel::new(&Theme::default());
        for (family, keys) in ColorFamily::ALL.iter().zip(&panel.swatches) {
            assert!(!keys.is_empty(), "{} has no swatches", family.name());
            assert!(keys.len() <= MAX_SWATCHES);
        }
    }

    #[test]
    fn ids_do_not_collide() {
        let mut ids = vec![
            ID_NAME,
            ID_SANS,
            ID_MONO,
            ID_DENSITY,
            ID_TEXT_SCALE,
            ID_RADIUS,
        ];
        for f in 0..7u64 {
            ids.extend([0, 1, 2, 7].map(|n| ID_FAMILY + 8 * f + n));
        }
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
