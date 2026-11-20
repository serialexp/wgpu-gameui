//! Panel — a grouped container with a mono caption (Forge `Panel`), plus the
//! plain label/title helpers.

use crate::layout::Rect;
use crate::style::{Ink, TextSize, Tracking};
use crate::{StyleKey, StyleResolver};

use super::DrawList;

/// The lit line along the top, inside the edge (`--hi-card`).
const HI_CARD: [f32; 4] = [1.0, 1.0, 1.0, 0.055];

/// A grouped container on the app ground with an optional mono caption
/// (Forge `Panel`).
///
/// The panel draws its card (surface, edge, lit top line) and caption, then
/// **returns the content rect** for the caller to lay children in:
///
/// ```ignore
/// let body = Panel::new().title("Physics").aside("3").draw(rect, list, &style);
/// ```
///
/// Use [`Group`](super::Group) for a card inside a pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel<'a> {
    title: Option<&'a str>,
    aside: Option<&'a str>,
    padding: Option<f32>,
    gap: Option<f32>,
}

impl Default for Panel<'_> {
    fn default() -> Self {
        Self {
            title: None,
            aside: None,
            padding: None,
            gap: None,
        }
    }
}

impl<'a> Panel<'a> {
    /// A panel with no caption.
    pub fn new() -> Self {
        Self::default()
    }

    /// The mono-capitals caption across the top.
    #[must_use]
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// A mono note at the right end of the caption row (a count, a unit).
    /// Shown only with a [`title`](Self::title).
    #[must_use]
    pub fn aside(mut self, aside: &'a str) -> Self {
        self.aside = Some(aside);
        self
    }

    /// Space between the edge and the content (default: the theme's
    /// `PanelPadding`).
    #[must_use]
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Some(padding.max(0.0));
        self
    }

    /// Space between the caption row and the content (default: the theme's
    /// `SectionGap`).
    #[must_use]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = Some(gap.max(0.0));
        self
    }

    /// The edge padding: the override, else the theme's `PanelPadding`.
    fn resolved_padding(&self, s: &StyleResolver) -> f32 {
        self.padding
            .unwrap_or_else(|| s.scalar(StyleKey::PanelPadding).max(0.0))
    }

    /// The caption-to-content gap: the override, else the theme's
    /// `SectionGap`.
    fn resolved_gap(&self, s: &StyleResolver) -> f32 {
        self.gap
            .unwrap_or_else(|| s.scalar(StyleKey::SectionGap).max(0.0))
    }

    /// The caption and aside blocks, positioned for `rect`.
    fn caption(
        &self,
        rect: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> Option<(f32, crate::text::TextBlock, Option<crate::text::TextBlock>)> {
        let title = self.title?;
        let padding = self.resolved_padding(s);
        let (x, y) = (rect.x + padding, rect.y + padding);
        let mut title = s.caption_block(title, x, y, Tracking::Section, Ink::Caption);
        let title_h = list.measure_block(&title).1;
        let aside = self
            .aside
            .map(|a| s.mono_block(a, 0.0, y, TextSize::Meta, Ink::Caption));
        let aside_size = aside.as_ref().map(|a| list.measure_block(a));
        let row_h = title_h.max(aside_size.map_or(0.0, |(_, h)| h)).ceil();
        // `align-items: center`: each sits on the row's middle.
        title.y = y + (row_h - title_h) * 0.5;
        let aside = aside.zip(aside_size).map(|(mut a, (w, h))| {
            a.x = rect.right() - padding - w;
            a.y = y + (row_h - h) * 0.5;
            a
        });
        Some((row_h, title, aside))
    }

    /// The rect children go in, without drawing.
    pub fn content_rect(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) -> Rect {
        let padding = self.resolved_padding(s);
        let gap = self.resolved_gap(s);
        let top = self
            .caption(rect, list, s)
            .map_or(0.0, |(row_h, ..)| row_h + gap);
        Rect::new(
            rect.x + padding,
            rect.y + padding + top,
            (rect.width - 2.0 * padding).max(0.0),
            (rect.height - 2.0 * padding - top).max(0.0),
        )
    }

    /// Draw the panel filling `rect`; returns the content rect.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) -> Rect {
        list.push_debug_scope_rect(
            super::scope_name("Panel", self.title.unwrap_or_default()),
            rect,
        );
        let border = s.scalar(StyleKey::BorderWidth).max(1.0);
        list.chrome_rect(
            rect,
            s.scalar(StyleKey::PanelRadius),
            border,
            s.color(StyleKey::Panel),
            s.color(StyleKey::PanelBorder),
        );
        // `inset 0 1px 0`: a 1 px band along the top, inside the edge. A
        // plain quad rather than a panel-sized inset shadow, so the panel
        // doesn't leave a box behind that later content appears to sit in.
        let inner = rect.inset(border);
        if inner.width > 0.0 && inner.height > 0.0 {
            // Start where the rounded corner is 1 px down, so a big radius
            // doesn't leave the line standing outside the card's curve.
            let r = s.scalar(StyleKey::PanelRadius);
            let run = (r - (2.0 * r - 1.0).max(0.0).sqrt()).clamp(0.0, inner.width * 0.5);
            list.quad(inner.x + run, inner.y, inner.width - 2.0 * run, 1.0, HI_CARD);
        }
        if let Some((_, title, aside)) = self.caption(rect, list, s) {
            // The caption gives way to the aside rather than running under it.
            let right = aside
                .as_ref()
                .map_or(rect.right() - self.resolved_padding(s), |a| a.x - 10.0);
            let clip = Rect::new(title.x, rect.y, (right - title.x).max(0.0), rect.height);
            list.text(title.with_clip(clip));
            if let Some(aside) = aside {
                list.text(aside);
            }
        }
        list.pop_debug_scope();
        self.content_rect(rect, list, s)
    }

    /// Draw a nine-slice textured panel at a layout-computed rect.
    pub fn draw_nine_slice(rect: Rect, list: &mut DrawList, texture_key: &str) {
        list.push_debug_scope_rect("Panel", rect);
        list.nine_slice(rect.x, rect.y, rect.width, rect.height, texture_key);
        list.pop_debug_scope();
    }
}

/// Label - simple text display.
pub fn label(list: &mut DrawList, style: &StyleResolver, text: &str, x: f32, y: f32) {
    list.text(style.text_block(text, x, y));
}

/// Label at a layout-computed rect (vertically centered).
pub fn label_at(list: &mut DrawList, style: &StyleResolver, text: &str, rect: Rect) {
    let y = list.vcentered_text_y(
        rect.y,
        rect.height,
        style.scalar(StyleKey::FontSize),
        style.theme().font.as_ref(),
        text,
    );
    list.text(style.text_block(text, rect.x + style.scalar(StyleKey::Padding), y));
}

/// Label centered horizontally and vertically in a rect.
pub fn label_centered_at(list: &mut DrawList, style: &StyleResolver, text: &str, rect: Rect) {
    let (text_width, _) = list.measure_text(text, style.scalar(StyleKey::FontSize), None);
    let x = rect.x + (rect.width - text_width) / 2.0;
    let y = list.vcentered_text_y(
        rect.y,
        rect.height,
        style.scalar(StyleKey::FontSize),
        style.theme().font.as_ref(),
        text,
    );
    list.text(style.text_block(text, x, y));
}

/// Title - larger text display.
pub fn title(list: &mut DrawList, style: &StyleResolver, text: &str, x: f32, y: f32) {
    list.text(style.title_block(text, x, y));
}

/// Title at a layout-computed rect (vertically centered).
pub fn title_at(list: &mut DrawList, style: &StyleResolver, text: &str, rect: Rect) {
    let y = list.vcentered_text_y(
        rect.y,
        rect.height,
        style.scalar(StyleKey::FontSizeTitle),
        style.theme().font.as_ref(),
        text,
    );
    list.text(style.title_block(text, rect.x + style.scalar(StyleKey::Padding), y));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn untitled_content_is_inset_by_the_padding() {
        let theme = Theme::forge();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rect = Rect::new(10.0, 20.0, 200.0, 100.0);
        let body = Panel::new().draw(rect, &mut list, &s);
        assert_eq!(body, Rect::new(24.0, 34.0, 172.0, 72.0));
        assert!(list.texts.is_empty());
    }

    #[test]
    fn caption_row_pushes_the_content_down() {
        let theme = Theme::forge();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        let body = Panel::new()
            .title("Physics")
            .aside("3")
            .draw(rect, &mut list, &s);
        assert_eq!(list.texts[0].content, "PHYSICS");
        assert_eq!(list.texts[1].content, "3");
        assert!(body.y > 14.0 + 11.0, "{body:?}");
        let aside = list.texts[1].clone();
        let (w, _) = list.measure_block(&aside);
        assert!((aside.x + w - (rect.right() - 14.0)).abs() < 0.01);
    }

    #[test]
    fn padding_and_gap_follow_the_theme() {
        let rect = Rect::new(10.0, 20.0, 200.0, 100.0);
        let forge = Theme::forge();
        let overlay = Theme::overlay();
        let mut list = DrawList::new();
        let f = Panel::new().content_rect(rect, &mut list, &StyleResolver::new(&forge));
        let o = Panel::new().content_rect(rect, &mut list, &StyleResolver::new(&overlay));
        assert_eq!(f, Rect::new(24.0, 34.0, 172.0, 72.0));
        assert_eq!(o, Rect::new(26.0, 36.0, 168.0, 68.0));
        let titled = Panel::new().title("T");
        let flush = titled.gap(0.0);
        for (theme, gap) in [(&forge, 11.0), (&overlay, 16.0)] {
            let s = StyleResolver::new(theme);
            let with = titled.content_rect(rect, &mut list, &s);
            let without = flush.content_rect(rect, &mut list, &s);
            assert!((with.y - without.y - gap).abs() < 0.01);
        }
        // Explicit overrides win on every theme.
        let ov = Panel::new().padding(5.0).gap(3.0);
        let r = ov.content_rect(rect, &mut list, &StyleResolver::new(&overlay));
        assert_eq!(r, Rect::new(15.0, 25.0, 190.0, 90.0));
    }

    #[test]
    fn aside_needs_a_title() {
        let theme = Theme::forge();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        Panel::new()
            .aside("3")
            .draw(Rect::new(0.0, 0.0, 100.0, 60.0), &mut list, &s);
        assert!(list.texts.is_empty());
    }
}
