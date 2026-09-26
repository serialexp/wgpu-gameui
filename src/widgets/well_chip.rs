//! Well chip — a small clickable readout sunk into a toolbar (Forge app
//! headers: the context chip "opus[1m] 79%", the rate chip "▮ 7% 5h").
//!
//! A 20px mono chip on the well surface with the chip recess. Its parts are
//! text runs and inline meters, laid out left to right with a 6px gap. While
//! the popover it opens is showing, pass [`WellChip::open`] and its edge
//! turns accent.

use crate::InputState;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::DrawList;
use super::meter::{INLINE_METER_HEIGHT, MeterFill, inline_meter};

/// Height of a [`WellChip`].
pub const WELL_CHIP_HEIGHT: f32 = 20.0;
/// Space inside the chip's left and right edges.
const PAD: f32 = 7.0;
/// Space between parts.
const GAP: f32 = 6.0;
/// `--well` and `--well-border`.
const WELL: [f32; 4] = [0.0, 0.0, 0.0, 0.42];
const WELL_BORDER: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// `--chip-inset`: the recess and the light line under the chip.
const RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
const LIP: [f32; 4] = [1.0, 1.0, 1.0, 0.07];

/// One part of a [`WellChip`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WellChipPart<'a> {
    /// Mono text. `None` takes the chip's ink (`--ink-2`, `--ink-max` while
    /// hovered); `Some` keeps its own (e.g. a caption-coloured unit).
    Text(&'a str, Option<Ink>),
    /// An inline meter `width` wide.
    Meter(f32, MeterFill),
}

/// What a [`WellChip`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WellChipOutput {
    /// The chip's rect.
    pub rect: Rect,
    /// The pointer is over it.
    pub hovered: bool,
    /// It was clicked (the caller opens or closes its popover).
    pub clicked: bool,
}

/// A clickable mono readout on the well surface.
#[derive(Clone, Copy, Debug)]
pub struct WellChip<'a> {
    parts: &'a [WellChipPart<'a>],
    open: bool,
}

impl<'a> WellChip<'a> {
    /// A chip showing `parts`.
    pub fn new(parts: &'a [WellChipPart<'a>]) -> Self {
        Self { parts, open: false }
    }

    /// Whether what it opens is showing (an accent edge).
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    fn part_width(list: &mut DrawList, s: &StyleResolver, part: &WellChipPart) -> f32 {
        match *part {
            WellChipPart::Text(text, _) => s.mono_width(list, text, TextSize::Meta),
            WellChipPart::Meter(width, _) => width,
        }
    }

    /// The width the chip takes.
    pub fn width(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let parts: f32 = self
            .parts
            .iter()
            .map(|p| Self::part_width(list, s, p))
            .sum();
        let gaps = GAP * self.parts.len().saturating_sub(1) as f32;
        parts + gaps + PAD * 2.0
    }

    /// Draw the chip with its top-left corner at `(x, y)`.
    pub fn draw(
        &self,
        x: f32,
        y: f32,
        list: &mut DrawList,
        s: &StyleResolver,
        input: &InputState,
    ) -> WellChipOutput {
        let rect = Rect::new(x, y, self.width(list, s), WELL_CHIP_HEIGHT);
        let hovered = rect.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let radius = s.scalar(StyleKey::BorderRadius);
        let edge = if self.open {
            s.color(StyleKey::Accent)
        } else {
            WELL_BORDER
        };
        list.chrome_rect(rect, radius, 1.0, WELL, edge);
        list.box_shadow_inset(
            rect.inset(1.0),
            CornerRadii::uniform(0.0),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 3.0,
                color: RECESS,
                inset: true,
                ..BoxShadow::default()
            },
        );
        list.quad(rect.x, rect.bottom(), rect.width, 1.0, LIP);

        let ink = if hovered { Ink::Max } else { Ink::Second };
        let size = s.text_size(TextSize::Meta);
        let cy = rect.y + rect.height * 0.5;
        let mut px = rect.x + PAD;
        for part in self.parts {
            let w = Self::part_width(list, s, part);
            match *part {
                WellChipPart::Text(text, own) => {
                    let ty = crate::text::vcentered_line_y(rect.y, rect.height, size);
                    list.text(s.mono_block(text, px, ty, TextSize::Meta, own.unwrap_or(ink)));
                }
                WellChipPart::Meter(width, fill) => {
                    let my = (cy - INLINE_METER_HEIGHT * 0.5).round();
                    inline_meter(list, s, Rect::new(px, my, width, INLINE_METER_HEIGHT), fill);
                }
            }
            px += w + GAP;
        }
        WellChipOutput {
            rect,
            hovered,
            clicked: hovered && input.mouse_clicked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn a_chip_lays_its_parts_out_and_reports_clicks() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let parts = [
            WellChipPart::Meter(30.0, MeterFill::neutral(0.07, &s)),
            WellChipPart::Text("7%", None),
            WellChipPart::Text("5h", Some(Ink::Caption)),
        ];
        let chip = WellChip::new(&parts);
        let mut list = DrawList::new();
        let w = chip.width(&mut list, &s);
        // Measured the way the chip draws them: mono.
        let seven = list
            .measure_block(&s.mono_block("7%", 0.0, 0.0, TextSize::Meta, Ink::Max))
            .0;
        let five = list
            .measure_block(&s.mono_block("5h", 0.0, 0.0, TextSize::Meta, Ink::Max))
            .0;
        assert!((w - (30.0 + seven + five + 2.0 * GAP + 2.0 * PAD)).abs() < 0.01);

        let click = InputState {
            mouse_x: 20.0,
            mouse_y: 15.0,
            mouse_clicked: true,
            ..Default::default()
        };
        let out = chip.draw(10.0, 5.0, &mut list, &s, &click);
        assert_eq!(out.rect, Rect::new(10.0, 5.0, w, WELL_CHIP_HEIGHT));
        assert!(out.hovered && out.clicked);
        let seven_text = list.texts.iter().find(|t| t.content == "7%").unwrap();
        assert_eq!(seven_text.x, 10.0 + PAD + 30.0 + GAP);
        assert_eq!(
            seven_text.color,
            crate::color::text_color(s.ink(Ink::Max)),
            "hovered text brightens"
        );
        let unit = list.texts.iter().find(|t| t.content == "5h").unwrap();
        assert_eq!(unit.color, crate::color::text_color(s.ink(Ink::Caption)));
    }

    #[test]
    fn an_open_chip_has_an_accent_edge() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let parts = [WellChipPart::Text("opus", None)];
        let mut closed = DrawList::new();
        WellChip::new(&parts).draw(0.0, 0.0, &mut closed, &s, &InputState::default());
        assert_eq!(closed.chrome_instance(0).unwrap().border, WELL_BORDER);
        let mut open = DrawList::new();
        WellChip::new(&parts)
            .open(true)
            .draw(0.0, 0.0, &mut open, &s, &InputState::default());
        assert_eq!(
            open.chrome_instance(0).unwrap().border,
            s.color(StyleKey::Accent)
        );
    }
}
