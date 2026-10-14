//! FilterChip — a latching pill key (Forge `FilterChip`).
//!
//! Up on the key face while off, held down into the well in the accent chip
//! colours while on. Several can be on at once: it is a filter toggle, not a
//! choice. A click (or Space / Enter while focused) asks to flip it; the
//! caller owns whether it is on. Hover and press show as the bare key's
//! wash over the pill.

use crate::color::{HUE_ACCENT, oklch};
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::{TextBlock, vcentered_line_y};

use super::{DrawContext, DrawList, FocusId, Pressable, material};

/// The pill's height (`--h-filter-chip`).
pub const FILTER_CHIP_HEIGHT: f32 = 18.0;
/// Room left and right of the label (`padding: 2px 9px 3px`).
const PAD_X: f32 = 9.0;
/// Between the label and its count.
const COUNT_GAP: f32 = 5.0;
/// The count's size, smaller than the dense step.
const COUNT_SIZE: f32 = 9.0;
/// The count's opacity against the label's ink.
const COUNT_ALPHA: f32 = 0.7;
/// The pill's edge, off and on.
const EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
const EDGE_ON: [f32; 4] = [0.0, 0.0, 0.0, 0.65];
/// The shade pressing the on pill into the well.
const HELD_SHADE: BoxShadow = BoxShadow {
    offset: [0.0, 2.0],
    blur: 4.0,
    spread: 0.0,
    color: [0.0, 0.0, 0.0, 0.5],
    inset: true,
};

/// A latching pill key. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct FilterChip<'a> {
    label: &'a str,
    count: Option<usize>,
    on: bool,
    focus_id: Option<FocusId>,
}

impl<'a> FilterChip<'a> {
    /// A chip reading `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            count: None,
            on: false,
            focus_id: None,
        }
    }

    /// A count after the label, smaller and dimmer.
    #[must_use]
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// Whether the chip is latched on.
    #[must_use]
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// Join the Tab ring as `id`; Space / Enter then flips it.
    #[must_use]
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    fn count_text(&self) -> Option<String> {
        self.count.map(|n| n.to_string())
    }

    /// The pill's width for its label and count.
    pub fn width(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let mut w = s.sans_width(list, self.label, TextSize::Dense);
        if let Some(count) = self.count_text() {
            w += COUNT_GAP
                + list
                    .measure_text_with_font(&count, COUNT_SIZE, None, s.theme().mono_font.as_ref())
                    .0;
        }
        (w + PAD_X * 2.0).ceil()
    }

    /// The pill drawn in `rect`: at its left, centred vertically, as wide as
    /// [`width`](Self::width) and [`FILTER_CHIP_HEIGHT`] tall (both clamped
    /// to `rect`).
    pub fn pill(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) -> Rect {
        let h = FILTER_CHIP_HEIGHT.min(rect.height);
        let w = self.width(list, s).min(rect.width);
        Rect::new(rect.x, (rect.y + (rect.height - h) * 0.5).round(), w, h)
    }

    /// Draw the chip in `rect` (see [`pill`](Self::pill)); returns whether it
    /// was asked to flip. Only the pill takes clicks.
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> bool {
        let s = ctx.styles();
        let pill = self.pill(rect, ctx.draw_list, &s);
        let mut key = Pressable::new()
            .bare()
            .radius(pill.height * 0.5)
            .name("FilterChip");
        if let Some(id) = self.focus_id {
            key = key.focusable(id);
        }
        key.draw(pill, ctx, |_, ctx| self.paint(pill, ctx)).clicked
    }

    fn paint(&self, pill: Rect, ctx: &mut DrawContext) {
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        let radius = pill.height * 0.5;
        let ink = if self.on {
            // Held in the well: the accent chip, pressed in.
            list.chrome_rect_gradient(
                pill,
                radius,
                1.0,
                oklch(0.42, 0.07, HUE_ACCENT, 1.0),
                oklch(0.52, 0.09, HUE_ACCENT, 1.0),
                EDGE_ON,
            );
            list.box_shadow_inset(
                pill.inset(1.0),
                CornerRadii::uniform((radius - 1.0).max(0.0)),
                HELD_SHADE,
            );
            oklch(0.93, 0.08, HUE_ACCENT, 1.0)
        } else {
            // Up: the key face. A pill's top edge is curved; a straight,
            // full-width 1px highlight reads as a white slash, so the face
            // gradient carries the sheen alone.
            let base = s.color(StyleKey::Button);
            list.chrome_rect_gradient(
                pill,
                radius,
                1.0,
                material::sheen_over(base, s.color(StyleKey::FaceTop)),
                material::sheen_over(base, s.color(StyleKey::FaceBottom)),
                EDGE,
            );
            s.ink(Ink::Icon)
        };

        // `padding: 2px 9px 3px`: the line sits half a pixel above centre.
        let y = vcentered_line_y(pill.y, pill.height - 1.0, s.text_size(TextSize::Dense));
        let label = s
            .sans_block(self.label, pill.x + PAD_X, y, TextSize::Dense, Ink::Icon)
            .with_color_f32(ink)
            .with_shadow(0, 0, 0, 153, 0.0, -1.0, 0.0);
        let (label_w, _) = list.measure_block(&label);
        list.text(label);
        if let Some(count) = self.count_text() {
            let mut dim = ink;
            dim[3] *= COUNT_ALPHA;
            list.text(
                TextBlock::new(
                    count,
                    pill.x + PAD_X + label_w + COUNT_GAP,
                    vcentered_line_y(pill.y, pill.height - 1.0, COUNT_SIZE),
                )
                .with_size(COUNT_SIZE)
                .with_color_f32(dim)
                .with_font_opt(s.theme().mono_font.clone()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    fn draw(rect: Rect, chip: &FilterChip, input: &InputState) -> (bool, DrawList) {
        let theme = Theme::default();
        let mut focus = FocusState::new();
        let mut list = DrawList::new();
        let clicked = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 400.0, 300.0);
            chip.draw(rect, &mut ctx)
        };
        (clicked, list)
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    fn away() -> InputState {
        InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..InputState::default()
        }
    }

    #[test]
    fn a_click_asks_to_flip_either_way() {
        let rect = Rect::new(10.0, 10.0, 60.0, FILTER_CHIP_HEIGHT);
        let click = click_at(14.0, 14.0);
        assert!(draw(rect, &FilterChip::new(".lvl").count(3), &click).0);
        assert!(draw(rect, &FilterChip::new(".lvl").on(true), &click).0);
        assert!(!draw(rect, &FilterChip::new(".lvl"), &away()).0);
    }

    #[test]
    fn only_the_pill_takes_clicks() {
        // A wide rect: the pill sits at its left, centred.
        let rect = Rect::new(10.0, 0.0, 200.0, 28.0);
        let beside = click_at(205.0, 14.0);
        assert!(!draw(rect, &FilterChip::new("all sessions"), &beside).0);
    }

    #[test]
    fn the_pill_sits_left_and_centred_at_its_own_width() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let chip = FilterChip::new("all sessions");
        let pill = chip.pill(Rect::new(10.0, 0.0, 200.0, 28.0), &mut list, &s);
        assert_eq!(pill.x, 10.0);
        assert_eq!(pill.y, 5.0);
        assert_eq!(pill.height, FILTER_CHIP_HEIGHT);
        assert_eq!(pill.width, chip.width(&mut list, &s));
    }

    #[test]
    fn off_is_the_key_face_without_a_white_top_line() {
        let rect = Rect::new(0.0, 0.0, 100.0, 24.0);
        let (_, list) = draw(rect, &FilterChip::new("info"), &away());
        assert_eq!(list.chrome_instance_count(), 1, "only the pill face");
        let face = list.chrome_instance(0).unwrap();
        assert_ne!(face.bg, face.bg2, "the face keeps its vertical sheen");
    }

    #[test]
    fn on_is_the_held_accent_chip() {
        let rect = Rect::new(0.0, 0.0, 100.0, 24.0);
        let (_, on) = draw(rect, &FilterChip::new("info").on(true), &away());
        let (_, off) = draw(rect, &FilterChip::new("info"), &away());
        assert_eq!(
            on.chrome_instance(0).unwrap().bg,
            oklch(0.42, 0.07, HUE_ACCENT, 1.0)
        );
        assert_ne!(
            on.chrome_instance(0).unwrap().bg,
            off.chrome_instance(0).unwrap().bg
        );
    }

    #[test]
    fn the_count_widens_the_chip() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let bare = FilterChip::new(".png").width(&mut list, &s);
        let counted = FilterChip::new(".png").count(12).width(&mut list, &s);
        assert!(counted > bare + COUNT_GAP);
        assert!(bare > PAD_X * 2.0);
    }
}
