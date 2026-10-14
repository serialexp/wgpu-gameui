//! DropZone — a dashed-outline drop target (Forge `DropZone`).

use crate::color::{HUE_ACCENT, oklch};
use crate::layout::Rect;
use crate::style::{Ink, StyleResolver, TextSize};
use crate::text::TextAlign;

use super::DrawList;

/// Forge's default size, for a zone with nothing to fit.
pub const DROP_ZONE_SIZE: (f32, f32) = (124.0, 78.0);
/// Space between the outline and the text.
const PAD: f32 = 8.0;
/// Dash and gap of the outline.
const DASH: f32 = 3.0;
/// The idle fill and outline.
const IDLE_FILL: [f32; 4] = [0.0, 0.0, 0.0, 0.3];
const IDLE_EDGE: [f32; 4] = [1.0, 1.0, 1.0, 0.18];

/// A dashed-outline drop target (Forge `DropZone`).
///
/// It outlines rather than fills, so whatever sits underneath stays
/// readable. While a drag hovers it, pass [`active`](DropZone::active): the
/// outline turns accent and a faint accent wash and inner ring appear.
/// The zone is only the picture; the caller owns the drag and decides when
/// it counts as "over".
///
/// ```ignore
/// let over = drag.is_some() && rect.contains(input.mouse_x, input.mouse_y);
/// DropZone::new("Drop to group").active(over).draw(rect, list, &style);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropZone<'a> {
    label: &'a str,
    active: bool,
}

impl<'a> DropZone<'a> {
    /// A zone reading `label` (centred, wrapped to fit).
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            active: false,
        }
    }

    /// A drag is over the zone: accent outline, wash and inner ring.
    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Draw the zone filling `rect`.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) {
        list.push_debug_scope_rect(super::scope_name("DropZone", self.label), rect);
        if self.active {
            list.quad(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                oklch(0.74, 0.11, HUE_ACCENT, 0.1),
            );
            list.rect_outline(rect.inset(1.0), 1.0, oklch(0.74, 0.11, HUE_ACCENT, 0.35));
            list.dashed_rect_outline(rect, DASH, oklch(0.8, 0.1, HUE_ACCENT, 1.0));
        } else {
            list.quad(rect.x, rect.y, rect.width, rect.height, IDLE_FILL);
            list.dashed_rect_outline(rect, DASH, IDLE_EDGE);
        }
        let inner = rect.inset(PAD);
        if !self.label.is_empty() && inner.width > 0.0 && inner.height > 0.0 {
            let mut block = s
                .sans_block(self.label, inner.x, inner.y, TextSize::Row, Ink::Glyph)
                .with_max_width(inner.width)
                .with_align(TextAlign::Center)
                .with_clip(inner);
            let (_, h) = list.measure_block(&block);
            block.y = inner.y + ((inner.height - h) * 0.5).max(0.0);
            list.text(block);
        }
        list.pop_debug_scope();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn counts(active: bool) -> (usize, usize) {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        DropZone::new("Drop to group").active(active).draw(
            Rect::new(0.0, 0.0, 124.0, 78.0),
            &mut list,
            &s,
        );
        (list.vertices.len(), list.chrome_instance_count())
    }

    #[test]
    fn active_adds_the_inner_ring() {
        let (idle_v, idle_c) = counts(false);
        let (active_v, active_c) = counts(true);
        assert_eq!(idle_v, active_v, "same dashes and fill");
        assert!(active_c > idle_c, "the ring is one more instance");
    }

    #[test]
    fn label_is_centred_in_the_zone() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rect = Rect::new(10.0, 20.0, 124.0, 78.0);
        DropZone::new("Drop").draw(rect, &mut list, &s);
        let text = list.texts[0].clone();
        assert_eq!(text.x, rect.x + PAD);
        assert_eq!(text.max_width, rect.width - 2.0 * PAD);
        let (_, h) = list.measure_block(&text);
        let mid = text.y + h * 0.5;
        assert!((mid - (rect.y + rect.height * 0.5)).abs() < 0.5, "{mid}");
    }
}
