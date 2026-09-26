//! FieldLabel — the mono-capitals label above a field (Forge `FieldLabel`).

use crate::layout::Rect;
use crate::style::{Ink, StyleResolver, Tracking};

use super::DrawList;

/// The 9 px mono-capitals label above a field, with an optional
/// right-aligned readout (Forge `FieldLabel`).
///
/// Labels sit *outside* the well, on the line above it; the readout shows the
/// field's current value where a slider or scrub has no room for one.
///
/// ```ignore
/// let label = FieldLabel::new("Roughness").value("0.42").draw(list, &style, x, y, 180.0);
/// let well = Rect::new(x, label.bottom() + 4.0, 180.0, 24.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLabel<'a> {
    text: &'a str,
    value: Option<&'a str>,
}

impl<'a> FieldLabel<'a> {
    /// A label reading `text` (drawn upper-case).
    pub fn new(text: &'a str) -> Self {
        Self { text, value: None }
    }

    /// Show `value` at the right end of the line, in the brighter secondary
    /// ink.
    #[must_use]
    pub fn value(mut self, value: &'a str) -> Self {
        self.value = Some(value);
        self
    }

    /// The height of the label's line.
    pub fn height(list: &mut DrawList, s: &StyleResolver) -> f32 {
        let probe = s.caption_block("Ag", 0.0, 0.0, Tracking::Caption, Ink::Label);
        list.measure_block(&probe).1.ceil()
    }

    /// Draw the label across `width` px from `(x, y)`; returns the line's
    /// rect.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver, x: f32, y: f32, width: f32) -> Rect {
        let r = Rect::new(x, y, width, Self::height(list, s));
        list.push_debug_scope_rect(super::scope_name("FieldLabel", self.text), r);
        let mut value_w = 0.0;
        if let Some(value) = self.value {
            let mut block = s.caption_block(value, 0.0, y, Tracking::Caption, Ink::Second);
            value_w = list.measure_block(&block).0;
            block.x = r.right() - value_w;
            list.text(block);
        }
        // The caption gives way to the readout (an 8 px gap) rather than
        // running under it.
        let gap = if self.value.is_some() { 8.0 } else { 0.0 };
        let clip = Rect::new(x, y, (width - value_w - gap).max(0.0), r.height);
        let block = s
            .caption_block(self.text, x, y, Tracking::Caption, Ink::Label)
            .with_clip(clip);
        list.text(block);
        list.pop_debug_scope();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::color::text_color;

    #[test]
    fn caption_is_upper_case_label_ink() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        FieldLabel::new("Roughness").draw(&mut list, &s, 0.0, 0.0, 120.0);
        let text = &list.texts[0];
        assert_eq!(text.content, "ROUGHNESS");
        assert_eq!(text.color, text_color(s.ink(Ink::Label)));
    }

    #[test]
    fn readout_sits_flush_right_in_secondary_ink() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let r = FieldLabel::new("Roughness")
            .value("0.42")
            .draw(&mut list, &s, 10.0, 0.0, 120.0);
        let value = list.texts[0].clone();
        assert_eq!(value.content, "0.42");
        assert_eq!(value.color, text_color(s.ink(Ink::Second)));
        let (w, _) = list.measure_block(&value);
        assert!((value.x + w - r.right()).abs() < 0.01);
        assert!(r.height > 0.0);
    }
}
