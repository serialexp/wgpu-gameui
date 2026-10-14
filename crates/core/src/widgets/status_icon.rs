//! StatusIcon — a round raised badge carrying a tone glyph (Forge
//! `StatusIcon`).

use crate::color::{HUE_ACCENT, oklch};
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::{DrawList, Severity};

/// The default side of a [`StatusIcon`]: the dialog-title size.
pub const STATUS_ICON_SIZE: f32 = 22.0;
/// The inline size, for a 22 px row.
pub const STATUS_ICON_INLINE_SIZE: f32 = 14.0;
/// The edge (`--edge-hard`).
const EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.65];
/// The face: a faint white sheen, brighter at the top.
const FACE_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.1];
const FACE_BOTTOM: [f32; 4] = [1.0, 1.0, 1.0, 0.02];
/// The lit top line inside the edge (`inset 0 1px 0`).
const LIT_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
/// The drop under the badge (`0 1px 1px`).
const DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.5];

/// A round raised badge carrying a tone glyph (Forge `StatusIcon`).
///
/// 22 px beside a dialog title, 14 px inline in a 22 px row. The tone is told
/// by the glyph *and* its colour (`i`, `✓`, `!`, `×`), never by colour alone.
/// It has the same hard edge and lit top as a `Thumb` tile.
///
/// ```ignore
/// StatusIcon::new(Severity::Warning).size(14.0).draw(list, &style, x, y);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusIcon<'a> {
    tone: Severity,
    size: f32,
    glyph: Option<&'a str>,
}

impl<'a> StatusIcon<'a> {
    /// A 22 px badge for `tone`.
    pub fn new(tone: Severity) -> Self {
        Self {
            tone,
            size: STATUS_ICON_SIZE,
            glyph: None,
        }
    }

    /// The badge's side in pixels (see [`STATUS_ICON_SIZE`] and
    /// [`STATUS_ICON_INLINE_SIZE`]).
    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(1.0);
        self
    }

    /// Show `glyph` instead of the tone's own.
    #[must_use]
    pub fn glyph(mut self, glyph: &'a str) -> Self {
        self.glyph = Some(glyph);
        self
    }

    /// The tone's glyph: `i`, `✓`, `!` or `×`.
    pub fn tone_glyph(tone: Severity) -> &'static str {
        match tone {
            Severity::Info => "i",
            Severity::Success => "\u{2713}",
            Severity::Warning => "!",
            Severity::Error => "\u{d7}",
        }
    }

    /// The glyph's colour for `tone`.
    pub fn tone_ink(tone: Severity, s: &StyleResolver) -> [f32; 4] {
        match tone {
            Severity::Info => oklch(0.72, 0.09, HUE_ACCENT, 1.0),
            Severity::Success => s.color(StyleKey::StatusOk),
            Severity::Warning => s.color(StyleKey::WarnMeta),
            Severity::Error => s.color(StyleKey::DangerText),
        }
    }

    /// Draw the badge with its top-left corner at `(x, y)`; returns its
    /// rect.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver, x: f32, y: f32) -> Rect {
        let r = Rect::new(x, y, self.size, self.size);
        let radius = self.size * 0.5;
        list.box_shadow_outset(
            r,
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 1.0,
                color: DROP,
                ..BoxShadow::default()
            },
        );
        list.chrome_rect_gradient(r, radius, 1.0, FACE_TOP, FACE_BOTTOM, EDGE);
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
        let size = (self.size * 0.55).round();
        let glyph = self.glyph.unwrap_or(Self::tone_glyph(self.tone));
        let mut block = TextBlock::new(glyph, 0.0, 0.0)
            .with_size(size)
            .with_font_opt(s.theme().font.clone())
            .with_weight(crate::Weight::BOLD)
            .with_color_f32(Self::tone_ink(self.tone, s));
        // Centred on its advance, as the design's flex box centres it.
        let (w, _) = list.measure_block(&block);
        block.x = r.x + (r.width - w) * 0.5;
        block.y = crate::text::vcentered_line_y(r.y, r.height, size);
        list.text(block);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn each_tone_has_its_own_glyph_and_ink() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let tones = [
            Severity::Info,
            Severity::Success,
            Severity::Warning,
            Severity::Error,
        ];
        for (i, a) in tones.iter().enumerate() {
            for b in &tones[i + 1..] {
                assert_ne!(StatusIcon::tone_glyph(*a), StatusIcon::tone_glyph(*b));
                assert_ne!(StatusIcon::tone_ink(*a, &s), StatusIcon::tone_ink(*b, &s));
            }
        }
    }

    #[test]
    fn draws_a_round_badge_with_the_glyph() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let r = StatusIcon::new(Severity::Error)
            .size(14.0)
            .draw(&mut list, &s, 4.0, 6.0);
        assert_eq!(r, Rect::new(4.0, 6.0, 14.0, 14.0));
        let text = list.texts.last().expect("the glyph");
        assert_eq!(text.content, "\u{d7}");
        assert_eq!(text.font_size, 8.0);
    }

    #[test]
    fn a_custom_glyph_replaces_the_tones() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        StatusIcon::new(Severity::Info)
            .glyph("?")
            .draw(&mut list, &s, 0.0, 0.0);
        assert_eq!(list.texts.last().expect("glyph").content, "?");
    }
}
