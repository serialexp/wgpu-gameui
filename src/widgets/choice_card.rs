//! Choice card — one answer in a list of answers to pick from, as a bordered
//! card rather than a bare radio row: a radio dot or a checkbox, a label, and
//! a wrapped description under it. The Agent Desktop design's question
//! dialog lists its options this way, ending with a dashed "Other" card.
//!
//! The card is only the picture and the click: the caller owns what is
//! picked, and tells the card with [`selected`](ChoiceCard::selected).
//!
//! # Example
//! ```ignore
//! let card = ChoiceCard::new("Postgres")
//!     .description("Runs as its own service.")
//!     .selected(picked == 0);
//! let h = card.height(width, list, &style);
//! if card.draw(Rect::new(x, y, width, h), list, &style, &input).clicked {
//!     picked = 0;
//! }
//! ```

use crate::InputState;
use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::DrawList;
use super::checkbox::draw_vector_box;
use super::radio::radio_mark;

/// Padding: 6 px above, 7 below, 10 at the sides.
const PAD_TOP: f32 = 6.0;
const PAD_BOTTOM: f32 = 7.0;
const PAD_X: f32 = 10.0;
/// The mark's box, its drop from the top, and the gap to the text.
const MARK: f32 = 13.0;
const MARK_DROP: f32 = 1.0;
const MARK_GAP: f32 = 10.0;
/// Between the label and the description.
const LINE_GAP: f32 = 1.0;
/// The dashes of an [`other`](ChoiceCard::dashed) card's border.
const DASH: f32 = 3.0;
/// How much of the accent a selected card is washed in (`--accent-wash`).
const WASH_ALPHA: f32 = 0.2;

/// Which mark a card wears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChoiceMark {
    /// One of the list may be picked.
    #[default]
    Radio,
    /// Any of the list may be picked.
    Check,
}

/// What a card's frame saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChoiceResponse {
    /// The pointer is over the card.
    pub hovered: bool,
    /// Clicked this frame: the caller picks (or unpicks) it.
    pub clicked: bool,
}

/// One answer to pick. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChoiceCard<'a> {
    label: &'a str,
    description: &'a str,
    mark: ChoiceMark,
    selected: bool,
    dashed: bool,
}

impl<'a> ChoiceCard<'a> {
    /// An unselected radio card reading `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            description: "",
            mark: ChoiceMark::Radio,
            selected: false,
            dashed: false,
        }
    }

    /// The wrapped text under the label.
    #[must_use]
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = description;
        self
    }

    /// A radio dot (the default) or a checkbox.
    #[must_use]
    pub fn mark(mut self, mark: ChoiceMark) -> Self {
        self.mark = mark;
        self
    }

    /// Picked: an accent border on an accent wash, and the mark set.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A dashed border: the card that stands for "something else".
    #[must_use]
    pub fn dashed(mut self, dashed: bool) -> Self {
        self.dashed = dashed;
        self
    }

    /// Where the text column starts, from the card's left.
    fn text_left() -> f32 {
        PAD_X + MARK + MARK_GAP
    }

    fn label_block(&self, s: &StyleResolver, x: f32, y: f32, width: f32) -> TextBlock {
        let ink = if self.selected { Ink::Title } else { Ink::Row };
        s.sans_block(self.label, x, y, TextSize::Row, ink)
            .with_max_width(width)
    }

    fn description_block(&self, s: &StyleResolver, x: f32, y: f32, width: f32) -> TextBlock {
        s.sans_block(self.description, x, y, TextSize::Meta, Ink::Caption)
            .with_max_width(width)
    }

    /// The card's height at `width`: its label and description wrapped to
    /// fit, or the mark, whichever is taller.
    pub fn height(&self, width: f32, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let text_w = (width - Self::text_left() - PAD_X).max(1.0);
        let mut text_h = list.measure_block(&self.label_block(s, 0.0, 0.0, text_w)).1;
        if !self.description.is_empty() {
            text_h += LINE_GAP
                + list
                    .measure_block(&self.description_block(s, 0.0, 0.0, text_w))
                    .1;
        }
        (PAD_TOP + text_h.max(MARK_DROP + MARK) + PAD_BOTTOM).ceil()
    }

    /// Draw the card in `rect` (its height from [`height`](Self::height)).
    pub fn draw(
        &self,
        rect: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
        input: &InputState,
    ) -> ChoiceResponse {
        list.push_debug_scope_rect("ChoiceCard", rect);
        let hovered = input.is_hovered(rect.x, rect.y, rect.width, rect.height);
        let accent = s.color(StyleKey::Accent);
        let fill = if self.selected {
            [accent[0], accent[1], accent[2], accent[3] * WASH_ALPHA]
        } else if hovered {
            s.color(StyleKey::RowHover)
        } else {
            s.color(StyleKey::InputBackground)
        };
        let edge = if self.selected {
            accent
        } else {
            s.color(StyleKey::EdgeHard)
        };
        list.quad(rect.x, rect.y, rect.width, rect.height, fill);
        if self.dashed {
            list.dashed_rect_outline(rect, DASH, edge);
        } else {
            list.rect_outline(rect, 1.0, edge);
        }

        let mark = Rect::new(rect.x + PAD_X, rect.y + PAD_TOP + MARK_DROP, MARK, MARK);
        match self.mark {
            ChoiceMark::Radio => {
                let center = (mark.x + MARK * 0.5, mark.y + MARK * 0.5);
                radio_mark(list, s, center, MARK * 0.5 - 0.5, self.selected);
            }
            ChoiceMark::Check => {
                let fill = if self.selected {
                    accent
                } else {
                    s.color(StyleKey::InputBackground)
                };
                draw_vector_box(list, s, mark.inset(0.5), self.selected, fill);
            }
        }

        let x = rect.x + Self::text_left();
        let text_w = (rect.right() - PAD_X - x).max(1.0);
        let label = self.label_block(s, x, rect.y + PAD_TOP, text_w);
        let label_h = list.measure_block(&label).1;
        list.text(label);
        if !self.description.is_empty() {
            let y = rect.y + PAD_TOP + label_h + LINE_GAP;
            list.text(self.description_block(s, x, y, text_w));
        }
        list.pop_debug_scope();
        ChoiceResponse {
            hovered,
            clicked: hovered && input.mouse_clicked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn frame(card: ChoiceCard, input: &InputState) -> (DrawList, Rect, ChoiceResponse) {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let h = card.height(300.0, &mut list, &s);
        let rect = Rect::new(0.0, 0.0, 300.0, h);
        let out = card.draw(rect, &mut list, &s, input);
        (list, rect, out)
    }

    fn at(x: f32, y: f32, clicked: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: clicked,
            ..InputState::default()
        }
    }

    #[test]
    fn a_description_makes_the_card_taller() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let bare = ChoiceCard::new("Postgres").height(300.0, &mut list, &s);
        let described = ChoiceCard::new("Postgres")
            .description("Runs as its own service, and keeps every write.")
            .height(300.0, &mut list, &s);
        assert!(described > bare);
        let narrow = ChoiceCard::new("Postgres")
            .description("Runs as its own service, and keeps every write.")
            .height(120.0, &mut list, &s);
        assert!(narrow > described, "the description wraps");
    }

    #[test]
    fn a_click_on_the_card_is_reported_and_one_off_it_is_not() {
        let (_, rect, out) = frame(ChoiceCard::new("SQLite"), &at(150.0, 8.0, true));
        assert!(out.hovered && out.clicked);
        let below = rect.bottom() + 2.0;
        let (_, _, out) = frame(ChoiceCard::new("SQLite"), &at(150.0, below, true));
        assert!(!out.hovered && !out.clicked);
        let mut consumed = at(150.0, 8.0, true);
        consumed.mouse_consumed = true;
        let (_, _, out) = frame(ChoiceCard::new("SQLite"), &consumed);
        assert!(!out.clicked, "a layer above took the pointer");
    }

    #[test]
    fn a_picked_card_reads_brighter_than_one_that_is_not() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let label = |selected| {
            let (list, _, _) = frame(
                ChoiceCard::new("SQLite").selected(selected),
                &at(-1.0, -1.0, false),
            );
            let text = list.texts.iter().find(|t| t.content == "SQLite").unwrap();
            text.color
        };
        assert_eq!(
            label(true),
            crate::color::text_color(s.ink(Ink::Title)),
            "picked"
        );
        assert_eq!(label(false), crate::color::text_color(s.ink(Ink::Row)));
    }

    #[test]
    fn the_other_card_is_dashed() {
        let solid = frame(ChoiceCard::new("Other"), &at(-1.0, -1.0, false)).0;
        let dashed = frame(
            ChoiceCard::new("Other").dashed(true),
            &at(-1.0, -1.0, false),
        )
        .0;
        let quads = |list: &DrawList| {
            let counts = list.prim_counts();
            counts.vertices + counts.chrome_instances
        };
        assert!(
            quads(&dashed) > quads(&solid),
            "dashes are many short quads"
        );
    }
}
