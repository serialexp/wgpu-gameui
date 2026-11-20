//! KeyPrompt — the light keycap that names an input ("E", "Space", "LMB")
//! (the design's `--kbd-*`). It is the one light object in a dark overlay,
//! so a prompt reads on any scene.

use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{StyleKey, StyleResolver, TextSize};
use crate::text::{TextBlock, vcentered_line_y};

use super::DrawList;

/// Space left and right of the label, inside the edge.
const PAD: f32 = 8.0;
/// The edge around the face (`0 0 0 1px` black at 55%).
const EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// The drop under the key (`0 1px 3px` black at 45%).
const DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
/// The shade along the face's bottom (`inset 0 -2px 0` black at 28%).
const BOTTOM_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.28];
/// The lit top line (`inset 0 1px 0 #fff`).
const LIT_TOP: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// A light keycap showing the name of an input.
///
/// ```ignore
/// let r = KeyPrompt::new("E").draw(list, &style, x, y);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPrompt<'a> {
    label: &'a str,
}

impl<'a> KeyPrompt<'a> {
    /// A keycap showing `label`.
    pub fn new(label: &'a str) -> Self {
        Self { label }
    }

    fn block(&self, s: &StyleResolver) -> TextBlock {
        TextBlock::new(self.label, 0.0, 0.0)
            .with_size(s.text_size(TextSize::Row))
            .with_font_opt(s.theme().mono_font.clone())
            .with_color_f32(s.color(StyleKey::KbdInk))
    }

    /// The key's height: the theme's status-key size (`--key-status`).
    pub fn height(s: &StyleResolver) -> f32 {
        s.scalar(StyleKey::KeyStatusSize)
    }

    /// The key's width: the label plus padding, never narrower than tall.
    pub fn width(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let (w, _) = list.measure_block(&self.block(s));
        (w + 2.0 * PAD).ceil().max(Self::height(s))
    }

    /// Draw the key with its top-left corner at `(x, y)`; returns its rect.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver, x: f32, y: f32) -> Rect {
        let mut block = self.block(s);
        let (text_w, _) = list.measure_block(&block);
        let h = Self::height(s);
        let r = Rect::new(x, y, ((text_w + 2.0 * PAD).ceil()).max(h), h);
        let radius = s.scalar(StyleKey::BorderRadius);
        list.box_shadow_outset(
            r,
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 3.0,
                color: DROP,
                ..BoxShadow::default()
            },
        );
        list.chrome_rect_gradient(
            r,
            radius,
            1.0,
            s.color(StyleKey::KbdFaceTop),
            s.color(StyleKey::KbdFaceBottom),
            EDGE,
        );
        let inner = r.inset(1.0);
        let inner_radii = CornerRadii::uniform((radius - 1.0).max(0.0));
        list.box_shadow_inset(
            inner,
            inner_radii,
            BoxShadow {
                offset: [0.0, -2.0],
                color: BOTTOM_SHADE,
                inset: true,
                ..BoxShadow::default()
            },
        );
        list.box_shadow_inset(
            inner,
            inner_radii,
            BoxShadow {
                offset: [0.0, 1.0],
                color: LIT_TOP,
                inset: true,
                ..BoxShadow::default()
            },
        );
        block.x = r.x + (r.width - text_w) * 0.5;
        block.y = vcentered_line_y(r.y, r.height, s.text_size(TextSize::Row));
        list.text(block);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn a_short_label_is_a_square_key_as_tall_as_the_theme_says() {
        for (theme, side) in [(Theme::forge(), 18.0), (Theme::overlay(), 28.0)] {
            let s = StyleResolver::new(&theme);
            let mut list = DrawList::new();
            let r = KeyPrompt::new("E").draw(&mut list, &s, 10.0, 20.0);
            assert_eq!(r.height, side);
            assert!(r.width >= side);
            assert_eq!((r.x, r.y), (10.0, 20.0));
        }
    }

    #[test]
    fn a_long_label_widens_the_key() {
        let theme = Theme::overlay();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let short = KeyPrompt::new("E").width(&mut list, &s);
        let long = KeyPrompt::new("Space").width(&mut list, &s);
        assert!(long > short, "{long} vs {short}");
    }

    #[test]
    fn the_face_is_the_themes_light_gradient() {
        let theme = Theme::overlay();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        KeyPrompt::new("E").draw(&mut list, &s, 0.0, 0.0);
        assert_eq!(list.chrome_instance_count(), 1);
        let face = list.chrome_instances().next().unwrap();
        assert_eq!(face.bg, theme.kbd_face_top);
        assert_eq!(list.texts.len(), 1);
    }
}
