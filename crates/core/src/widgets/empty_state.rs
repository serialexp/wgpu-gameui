//! Empty state — the message a list, tree or pane shows when it has nothing
//! to show (Forge `EmptyState`).
//!
//! A sunken box (the deep well colour, a hard 1 px edge and an inset shadow)
//! with a centred column, 9 px apart:
//! - a 20 px glyph in the empty ink (a text glyph such as "◫", or, with
//!   `phosphor-icons`, a [`PhosphorIcon`]);
//! - an optional title at the menu size in the tab ink;
//! - an optional hint, dense and centred at a 1.5 line height, in the
//!   caption ink;
//! - an optional accent key (the call to action).
//!
//! The box fills the rect it is given; [`EmptyState::HEIGHT`] is the design's
//! default height. Content that doesn't fit starts at the padding and runs
//! past the bottom rather than above the top.
//!
//! ```ignore
//! let clicked = EmptyState::new()
//!     .glyph("◫")
//!     .title("No session selected")
//!     .hint("Pick a session, or start a new one with +")
//!     .draw(rect, &mut ctx);
//! ```

use crate::layout::Rect;
#[cfg(feature = "phosphor-icons")]
use crate::render::PhosphorIcon;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::{LINE_HEIGHT_RATIO, TextAlign, TextBlock};

use super::material::Tone;
use super::{Button, DrawContext, DrawList, FocusId};

/// Padding inside the box.
const PAD: f32 = 14.0;
/// Space between the column's parts.
const GAP: f32 = 9.0;
/// Glyph size.
const GLYPH: f32 = 20.0;
/// Hint line height, as a multiple of its size.
const HINT_LINE: f32 = 1.5;
/// The action key's horizontal padding inside its 1 px border.
const ACTION_PAD_X: f32 = 9.0;
/// The box's inset shadow: `inset 0 2px 5px` (coloured by
/// [`StyleKey::InnerShadow`], the design's `rgba(0,0,0,.6)`).
const INSET_SHADOW: ([f32; 2], f32) = ([0.0, 2.0], 5.0);

/// What an [`EmptyState`] shows above its title.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmptyGlyph<'a> {
    /// A text glyph, drawn in the theme font.
    Text(&'a str),
    /// A Phosphor icon.
    #[cfg(feature = "phosphor-icons")]
    Icon(PhosphorIcon),
}

/// An empty-state message (see the [module docs](self)). Build it per frame
/// and [`draw`](Self::draw) it into the rect it should fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmptyState<'a> {
    glyph: EmptyGlyph<'a>,
    title: Option<&'a str>,
    hint: Option<&'a str>,
    action: Option<&'a str>,
    action_focus: Option<FocusId>,
}

impl Default for EmptyState<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Where an [`EmptyState`]'s parts go inside its rect.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    /// The column's content box (the rect less the padding).
    inner: Rect,
    glyph: Rect,
    title: Option<Rect>,
    hint: Option<Rect>,
    action: Option<Rect>,
}

impl<'a> EmptyState<'a> {
    /// The design's default box height.
    pub const HEIGHT: f32 = 142.0;

    /// An empty state showing only the design's default glyph ("▤").
    pub fn new() -> Self {
        Self {
            glyph: EmptyGlyph::Text("▤"),
            title: None,
            hint: None,
            action: None,
            action_focus: None,
        }
    }

    /// Show a text glyph above the title.
    pub fn glyph(mut self, glyph: &'a str) -> Self {
        self.glyph = EmptyGlyph::Text(glyph);
        self
    }

    /// Show a Phosphor icon above the title.
    #[cfg(feature = "phosphor-icons")]
    pub fn icon(mut self, icon: PhosphorIcon) -> Self {
        self.glyph = EmptyGlyph::Icon(icon);
        self
    }

    /// The headline ("No session selected").
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// The explanation under the title, wrapped to the box.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Add an accent key labelled `label`; [`draw`](Self::draw) reports its
    /// click.
    pub fn action(mut self, label: &'a str) -> Self {
        self.action = Some(label);
        self
    }

    /// Put the action key in the Tab ring under `id`, so Space and Enter
    /// activate it.
    pub fn action_focus(mut self, id: FocusId) -> Self {
        self.action_focus = Some(id);
        self
    }

    /// [`action_focus`](Self::action_focus) unless the caller chose an id.
    pub(crate) fn action_focus_or(mut self, id: FocusId) -> Self {
        self.action_focus.get_or_insert(id);
        self
    }

    /// The glyph's text block, positioned at `y` in the column `inner`.
    fn glyph_block(&self, text: &str, inner: Rect, y: f32, s: &StyleResolver) -> TextBlock {
        TextBlock::new(text, inner.x, y)
            .with_size(GLYPH)
            .with_color_f32(s.ink(Ink::Empty))
            .with_font_opt(s.theme().font.clone())
            .with_max_width(inner.width)
            .with_align(TextAlign::Center)
    }

    fn title_block(title: &str, inner: Rect, y: f32, s: &StyleResolver) -> TextBlock {
        s.sans_block(title, inner.x, y, TextSize::Menu, Ink::Tab)
            .with_max_width(inner.width)
            .with_align(TextAlign::Center)
    }

    fn hint_block(hint: &str, inner: Rect, y: f32, s: &StyleResolver) -> TextBlock {
        let size = s.text_size(TextSize::Dense);
        s.sans_block(hint, inner.x, y, TextSize::Dense, Ink::Caption)
            .with_line_height(size * HINT_LINE)
            .with_max_width(inner.width)
            .with_align(TextAlign::Center)
    }

    /// Lay the column out in `rect`: each part's height, then the whole
    /// column centred vertically (but never above the padding).
    fn layout(&self, list: &mut DrawList, s: &StyleResolver, rect: Rect) -> Layout {
        let inner = rect.inset(PAD);
        let glyph_h = GLYPH * LINE_HEIGHT_RATIO;
        let title_h = self
            .title
            .map(|t| list.measure_block(&Self::title_block(t, inner, 0.0, s)).1);
        let hint_h = self
            .hint
            .map(|h| list.measure_block(&Self::hint_block(h, inner, 0.0, s)).1);
        let action_size = self.action.map(|label| {
            let border = s.scalar(StyleKey::BorderWidth);
            let label_w = list.measure_block(&s.text_block(label, 0.0, 0.0)).0;
            (
                label_w + 2.0 * (ACTION_PAD_X + border),
                s.scalar(StyleKey::ButtonHeight),
            )
        });

        let heights = [Some(glyph_h), title_h, hint_h, action_size.map(|(_, h)| h)];
        let parts = heights.iter().flatten().count();
        let total: f32 = heights.iter().flatten().sum::<f32>() + GAP * (parts - 1) as f32;
        let mut y = inner.y + ((inner.height - total) * 0.5).max(0.0);

        let mut next = |h: f32, x: f32, w: f32| {
            let r = Rect::new(x, y, w, h);
            y += h + GAP;
            r
        };
        Layout {
            inner,
            glyph: next(glyph_h, inner.x, inner.width),
            title: title_h.map(|h| next(h, inner.x, inner.width)),
            hint: hint_h.map(|h| next(h, inner.x, inner.width)),
            action: action_size.map(|(w, h)| next(h, inner.x + (inner.width - w) * 0.5, w)),
        }
    }

    /// Paint the sunken box: the deep well fill inside a hard edge, and the
    /// inset shadow under its top.
    fn draw_box(list: &mut DrawList, s: &StyleResolver, rect: Rect) {
        let border = s.scalar(StyleKey::BorderWidth);
        let radius = s
            .scalar(StyleKey::BorderRadius)
            .min(rect.width.min(rect.height) * 0.5)
            .max(0.0);
        list.chrome_rect(
            rect,
            radius,
            border,
            s.color(StyleKey::WellDeep),
            s.color(StyleKey::EdgeHard),
        );
        let (offset, blur) = INSET_SHADOW;
        list.box_shadow_inset(
            rect.inset(border),
            CornerRadii::uniform((radius - border).max(0.0)),
            BoxShadow {
                offset,
                blur,
                color: s.color(StyleKey::InnerShadow),
                inset: true,
                ..BoxShadow::default()
            },
        );
    }

    /// Draw the empty state filling `rect`. Returns whether the action key
    /// was clicked (or activated from the keyboard) this frame.
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> bool {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return false;
        }
        let s = ctx.styles();
        Self::draw_box(ctx.draw_list, &s, rect);
        let layout = self.layout(ctx.draw_list, &s, rect);

        match self.glyph {
            EmptyGlyph::Text(text) => {
                let block = self.glyph_block(text, layout.inner, layout.glyph.y, &s);
                ctx.draw_list.text(block);
            }
            #[cfg(feature = "phosphor-icons")]
            EmptyGlyph::Icon(icon) => {
                let g = layout.glyph;
                super::Icon::new(icon).tint(s.ink(Ink::Empty)).draw(
                    Rect::new(
                        g.x + (g.width - GLYPH) * 0.5,
                        g.y + (g.height - GLYPH) * 0.5,
                        GLYPH,
                        GLYPH,
                    ),
                    ctx.draw_list,
                );
            }
        }
        if let (Some(title), Some(r)) = (self.title, layout.title) {
            ctx.draw_list
                .text(Self::title_block(title, layout.inner, r.y, &s));
        }
        if let (Some(hint), Some(r)) = (self.hint, layout.hint) {
            ctx.draw_list
                .text(Self::hint_block(hint, layout.inner, r.y, &s));
        }
        match (self.action, layout.action) {
            (Some(label), Some(r)) => {
                let mut key = Button::new(label).tone(Tone::Accent);
                if let Some(id) = self.action_focus {
                    key = key.focusable(id);
                }
                key.draw(r, ctx)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::text_color;
    use crate::{FocusState, InputState, Theme};

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
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

    /// Draw `es` into `rect` with `input`; returns the list, the click and the
    /// layout the draw used.
    fn draw(es: &EmptyState, rect: Rect, input: &InputState) -> (DrawList, bool, Layout) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        let clicked = es.draw(rect, &mut ctx);
        let layout = es.layout(ctx.draw_list, &ctx.styles(), rect);
        (list, clicked, layout)
    }

    fn full() -> EmptyState<'static> {
        EmptyState::new()
            .glyph("◫")
            .title("No session selected")
            .hint("Pick a session, or start a new one with +")
            .action("New session")
    }

    const RECT: Rect = Rect {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: EmptyState::HEIGHT,
    };

    #[test]
    fn the_box_is_the_deep_well_with_a_hard_edge_and_an_inset_shadow() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let (list, _, _) = draw(&EmptyState::new(), RECT, &away());
        assert!(
            list.chrome_instances()
                .any(|c| c.bg == s.color(StyleKey::WellDeep)
                    && c.border == s.color(StyleKey::EdgeHard)),
            "a WellDeep box edged in EdgeHard"
        );
        let shadow = list
            .shadow_instances()
            .next()
            .expect("the box has an inset shadow");
        assert_eq!(shadow.translation[3], 1.0, "the shadow is inset");
        assert_eq!(shadow.color, s.color(StyleKey::InnerShadow));
        assert_eq!(
            shadow.element_rect,
            [
                RECT.x + 1.0,
                RECT.y + 1.0,
                RECT.width - 2.0,
                RECT.height - 2.0
            ],
            "inside the 1 px border"
        );
    }

    #[test]
    fn the_parts_use_the_design_sizes_and_inks() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let (list, _, _) = draw(&full(), RECT, &away());
        let text = |content: &str| {
            list.texts
                .iter()
                .find(|t| t.content == content)
                .unwrap_or_else(|| panic!("{content} was drawn"))
        };

        let glyph = text("◫");
        assert_eq!(glyph.font_size, GLYPH);
        assert_eq!(glyph.color, text_color(s.ink(Ink::Empty)));

        let title = text("No session selected");
        assert_eq!(title.font_size, s.text_size(TextSize::Menu));
        assert_eq!(title.color, text_color(s.ink(Ink::Tab)));

        let hint = text("Pick a session, or start a new one with +");
        let dense = s.text_size(TextSize::Dense);
        assert_eq!(hint.font_size, dense);
        assert_eq!(hint.line_height, dense * HINT_LINE);
        assert_eq!(hint.color, text_color(s.ink(Ink::Caption)));

        for t in [glyph, title, hint] {
            assert_eq!(t.align, TextAlign::Center, "{} is centred", t.content);
            assert_eq!(t.x, RECT.x + PAD);
            assert_eq!(t.max_width, RECT.width - 2.0 * PAD);
        }
        assert!(list.texts.iter().any(|t| t.content == "New session"));
    }

    #[test]
    fn the_action_is_an_accent_key() {
        let (list, _, l) = draw(&full(), RECT, &away());
        // The same key drawn on its own: its faces must all be in the list.
        let theme = Theme::default();
        let mut alone = DrawList::new();
        let mut focus = FocusState::new();
        let input = away();
        let mut ctx = DrawContext::new(&mut alone, &mut focus, &theme, &input, 800.0, 600.0);
        Button::new("New session")
            .tone(Tone::Accent)
            .draw(l.action.unwrap(), &mut ctx);
        let faces: Vec<_> = alone
            .chrome_instances()
            .map(|c| (c.rect, c.bg, c.bg2))
            .collect();
        assert!(!faces.is_empty());
        for face in faces {
            assert!(
                list.chrome_instances()
                    .any(|c| (c.rect, c.bg, c.bg2) == face),
                "{face:?} is drawn"
            );
        }
    }

    #[test]
    fn the_column_is_centred_nine_pixels_apart() {
        let (_, _, l) = draw(&full(), RECT, &away());
        let parts = [
            l.glyph,
            l.title.unwrap(),
            l.hint.unwrap(),
            l.action.unwrap(),
        ];
        for pair in parts.windows(2) {
            assert!(
                (pair[1].y - pair[0].bottom() - GAP).abs() < 1e-3,
                "{pair:?} are {GAP} px apart"
            );
        }
        let top_gap = parts[0].y - l.inner.y;
        let bottom_gap = l.inner.bottom() - parts[3].bottom();
        assert!(
            (top_gap - bottom_gap).abs() < 1e-3,
            "centred: {top_gap} above, {bottom_gap} below"
        );
        let action = parts[3];
        assert!(
            ((action.x + action.width * 0.5) - (RECT.x + RECT.width * 0.5)).abs() < 1e-3,
            "the key is centred"
        );
    }

    #[test]
    fn the_action_key_fits_its_label_with_the_design_padding() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let (mut list, _, l) = draw(&full(), RECT, &away());
        let label_w = list.measure_block(&s.text_block("New session", 0.0, 0.0)).0;
        let action = l.action.unwrap();
        assert!((action.width - (label_w + 20.0)).abs() < 1e-3);
        assert_eq!(action.height, s.scalar(StyleKey::ButtonHeight));
    }

    #[test]
    fn absent_parts_take_no_space() {
        let (list, _, l) = draw(&EmptyState::new().title("Nothing here"), RECT, &away());
        assert_eq!(l.hint, None);
        assert_eq!(l.action, None);
        assert_eq!(list.texts.len(), 2, "the default glyph and the title");
        assert_eq!(list.texts[0].content, "▤");
        let title = l.title.unwrap();
        let top_gap = l.glyph.y - l.inner.y;
        assert!((top_gap - (l.inner.bottom() - title.bottom())).abs() < 1e-3);
    }

    #[test]
    fn clicking_the_action_reports_it() {
        let (_, _, l) = draw(&full(), RECT, &away());
        let key = l.action.unwrap();
        let (_, clicked, _) = draw(
            &full(),
            RECT,
            &click_at(key.x + key.width * 0.5, key.y + key.height * 0.5),
        );
        assert!(clicked);
        let (_, clicked, _) = draw(&full(), RECT, &click_at(RECT.x + 4.0, RECT.y + 4.0));
        assert!(!clicked, "a click on the box is not the action");
    }

    #[test]
    fn content_taller_than_the_box_starts_at_the_padding() {
        let short = Rect::new(0.0, 0.0, 200.0, 60.0);
        let (_, _, l) = draw(&full(), short, &away());
        assert_eq!(l.glyph.y, PAD, "never above the padding");
    }

    #[test]
    fn an_empty_rect_draws_nothing() {
        let (list, clicked, _) = draw(&full(), Rect::new(0.0, 0.0, 0.0, 40.0), &away());
        assert!(!clicked);
        assert_eq!(list.chrome_instance_count(), 0);
        assert!(list.texts.is_empty());
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn an_icon_glyph_is_a_20px_icon_in_the_empty_ink() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let es = EmptyState::new()
            .icon(PhosphorIcon::MagnifyingGlass)
            .title("Nothing matches");
        let (list, _, l) = draw(&es, RECT, &away());
        let icon = list.icons_msdf.first().expect("the icon is drawn");
        assert_eq!(icon.tint, s.ink(Ink::Empty));
        assert_eq!(list.texts.len(), 1, "only the title is text");
        let (g, r) = (l.glyph, icon.local);
        assert!((r.width - GLYPH).abs() < 1e-3 && (r.height - GLYPH).abs() < 1e-3);
        assert!(((r.x + r.width * 0.5) - (g.x + g.width * 0.5)).abs() < 1e-3);
        assert!(((r.y + r.height * 0.5) - (g.y + g.height * 0.5)).abs() < 1e-3);
    }
}
