//! CountBubble — a round, raised notification count (Forge `CountBubble`).

use crate::color::{HUE_DANGER, oklch, rgb8};
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::StyleResolver;
use crate::text::TextBlock;

use super::DrawList;

/// Height of a [`CountBubble`] (and its minimum width, so one digit is a
/// circle).
pub const COUNT_BUBBLE_HEIGHT: f32 = 17.0;
/// Space left and right of the number, inside the edge.
const PAD: f32 = 5.0;
/// The number's size (`9.5px` mono).
const TEXT_SIZE: f32 = 9.5;
/// The edge (`--rule`).
const EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// The lit top line inside the edge (`inset 0 1px 0`).
const LIT_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.45];
/// The drop under the bubble (`0 1px 2px`).
const DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// The number's ink (`--danger-bubble-ink`).
const INK: [f32; 4] = rgb8([0xff, 0xf5, 0xf3]);
/// Alpha of the number's carved shadow (`0 -1px 0 rgba(0,0,0,.35)`).
const CARVE_ALPHA: u8 = 89;

/// A round, raised notification count (Forge `CountBubble`).
///
/// It is lit from above rather than sunk into the surface, because it
/// reports an *arriving* quantity (unread messages, new problems). A
/// [`Badge`](super::Badge) is the sunken, resting counterpart for state.
///
/// ```ignore
/// let r = CountBubble::new(12).draw(list, &style, x, y);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountBubble {
    count: u32,
}

impl CountBubble {
    /// A bubble showing `count`.
    pub fn new(count: u32) -> Self {
        Self { count }
    }

    fn block(&self, s: &StyleResolver) -> TextBlock {
        TextBlock::new(self.count.to_string(), 0.0, 0.0)
            .with_size(TEXT_SIZE)
            .with_font_opt(s.theme().mono_font.clone())
            .with_color_f32(INK)
            .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0)
    }

    /// The bubble's width: the number plus padding and edge, never narrower
    /// than it is tall.
    pub fn width(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let (w, _) = list.measure_block(&self.block(s));
        Self::width_for(w)
    }

    fn width_for(text_width: f32) -> f32 {
        (text_width + 2.0 * (PAD + 1.0))
            .ceil()
            .max(COUNT_BUBBLE_HEIGHT)
    }

    /// Draw the bubble with its top-left corner at `(x, y)`; returns its
    /// rect.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver, x: f32, y: f32) -> Rect {
        let mut block = self.block(s);
        let (text_w, _) = list.measure_block(&block);
        let r = Rect::new(x, y, Self::width_for(text_w), COUNT_BUBBLE_HEIGHT);
        let radius = r.height * 0.5;
        list.box_shadow_outset(
            r,
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 2.0,
                color: DROP,
                ..BoxShadow::default()
            },
        );
        list.chrome_rect_gradient(
            r,
            radius,
            1.0,
            oklch(0.7, 0.19, HUE_DANGER, 1.0),
            oklch(0.54, 0.18, HUE_DANGER, 1.0),
            EDGE,
        );
        list.box_shadow_inset(
            r.inset(1.0),
            CornerRadii::uniform(radius - 1.0),
            BoxShadow {
                offset: [0.0, 1.0],
                color: LIT_TOP,
                inset: true,
                ..BoxShadow::default()
            },
        );
        block.x = r.x + (r.width - text_w) * 0.5;
        block.y = crate::text::vcentered_line_y(r.y, r.height, TEXT_SIZE);
        list.text(block);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn one_digit_is_nearly_round() {
        // A 9.5 px mono digit plus padding and edge is 17.7 px in the design
        // too; the width rounds up to whole pixels.
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let r = CountBubble::new(3).draw(&mut list, &s, 10.0, 20.0);
        assert_eq!((r.x, r.y, r.height), (10.0, 20.0, COUNT_BUBBLE_HEIGHT));
        assert!(r.width >= COUNT_BUBBLE_HEIGHT && r.width <= COUNT_BUBBLE_HEIGHT + 1.0);
        assert_eq!(r.width, r.width.round());
    }

    #[test]
    fn more_digits_grow_the_pill() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let one = CountBubble::new(3).width(&mut list, &s);
        let three = CountBubble::new(128).width(&mut list, &s);
        assert!(three > one, "{three} > {one}");
        let r = CountBubble::new(128).draw(&mut list, &s, 0.0, 0.0);
        assert_eq!(r.width, three);
    }

    #[test]
    fn draws_the_number_centred() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let r = CountBubble::new(42).draw(&mut list, &s, 0.0, 0.0);
        let text = list.texts.last().expect("the number");
        assert_eq!(text.content, "42");
        let text = text.clone();
        let (w, _) = list.measure_block(&text);
        assert!((text.x + w * 0.5 - (r.x + r.width * 0.5)).abs() < 0.01);
    }
}
