//! Span tabs — a strip of equal-width view tabs spanning its whole width
//! (Forge `SpanTabs`: an app's "› Chat · ◫ Forum · ◧ Files" switcher).
//!
//! Each tab shares the width, holds an optional glyph, a label, an optional
//! count and an unsaved dot. The active tab is lit and sits two pixels lower
//! (on the strip's bottom rule); the others are sunken and sit two pixels
//! higher. Disabled tabs fade and don't click.

use crate::InputState;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::DrawList;

/// Height of a span tab strip (`--h-doc-tab`).
pub const SPAN_TABS_HEIGHT: f32 = 27.0;
/// Space inside the strip's left and right edges, and between tabs.
const STRIP_PAD: f32 = 3.0;
const TAB_GAP: f32 = 1.0;
/// How far the active tab sits lower (and the others higher).
const STEP: f32 = 2.0;
/// Space inside a tab, and between its parts.
const TAB_PAD: f32 = 8.0;
const PART_GAP: f32 = 6.0;
/// The glyph's size.
const GLYPH_SIZE: f32 = 10.0;
/// The unsaved dot's size.
const DOT: f32 = 5.0;

/// The strip: a faint lit gradient, a lit top line, a rule underneath.
const STRIP_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.055];
const STRIP_BOTTOM: [f32; 4] = [1.0, 1.0, 1.0, 0.012];
const STRIP_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.1];
const RULE: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const ACTIVE_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.13];
const ACTIVE_BOTTOM: [f32; 4] = [1.0, 1.0, 1.0, 0.045];
const ACTIVE_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const ACTIVE_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.24];
const IDLE: [f32; 4] = [0.0, 0.0, 0.0, 0.22];
const IDLE_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.35];
const IDLE_RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.3];
const HOVER: [f32; 4] = [1.0, 1.0, 1.0, 0.07];
const DISABLED_ALPHA: f32 = 0.45;
/// The carve under a label (`--carve`).
const CARVE_ALPHA: u8 = 153;

/// One tab of a [`SpanTabs`] strip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpanTab<'a> {
    /// Its label.
    pub label: &'a str,
    /// A glyph before the label ("›", "◫").
    pub glyph: &'a str,
    /// A count after the label, if any.
    pub count: &'a str,
    /// Show the unsaved dot (only while it isn't the active tab).
    pub dirty: bool,
    /// Faded and not clickable.
    pub disabled: bool,
}

impl<'a> SpanTab<'a> {
    /// A tab labelled `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            ..Self::default()
        }
    }

    /// Put `glyph` before the label.
    pub fn glyph(mut self, glyph: &'a str) -> Self {
        self.glyph = glyph;
        self
    }

    /// Put `count` after the label.
    pub fn count(mut self, count: &'a str) -> Self {
        self.count = count;
        self
    }

    /// Show the unsaved dot.
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    /// Fade it and ignore clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// A strip of equal-width view tabs.
#[derive(Clone, Copy, Debug)]
pub struct SpanTabs<'a> {
    tabs: &'a [SpanTab<'a>],
}

impl<'a> SpanTabs<'a> {
    /// A strip of `tabs`.
    pub fn new(tabs: &'a [SpanTab<'a>]) -> Self {
        Self { tabs }
    }

    /// The rect of tab `i` in a strip filling `rect`, before its step.
    fn slot(&self, rect: Rect, i: usize) -> Rect {
        let n = self.tabs.len().max(1) as f32;
        let inner = (rect.width - STRIP_PAD * 2.0 - TAB_GAP * (n - 1.0)).max(0.0);
        let w = inner / n;
        Rect::new(
            rect.x + STRIP_PAD + i as f32 * (w + TAB_GAP),
            rect.y,
            w,
            rect.height,
        )
    }

    /// Draw the strip filling `rect` with tab `active` selected. Returns the
    /// index of a tab clicked this frame (never a disabled or the active one).
    pub fn draw(
        &self,
        rect: Rect,
        active: usize,
        list: &mut DrawList,
        s: &StyleResolver,
        input: &InputState,
    ) -> Option<usize> {
        list.vertical_gradient(rect, STRIP_TOP, STRIP_BOTTOM);
        list.quad(rect.x, rect.y, rect.width, 1.0, STRIP_HI);
        list.quad(rect.x, rect.bottom() - 1.0, rect.width, 1.0, RULE);
        let radius = s.scalar(StyleKey::BorderRadius);
        let mut clicked = None;
        for (i, tab) in self.tabs.iter().enumerate() {
            let slot = self.slot(rect, i);
            let on = i == active;
            // The active tab reaches the rule; the others stop above it.
            let r = if on {
                Rect::new(slot.x, slot.y + STEP, slot.width, slot.height - STEP - 1.0)
            } else {
                Rect::new(slot.x, slot.y, slot.width, slot.height - STEP - 1.0)
            };
            let hovered =
                !tab.disabled && !input.mouse_consumed && r.contains(input.mouse_x, input.mouse_y);
            if hovered && !on && input.mouse_clicked {
                clicked = Some(i);
            }
            let fade = if tab.disabled { DISABLED_ALPHA } else { 1.0 };
            let a = |mut c: [f32; 4]| {
                c[3] *= fade;
                c
            };
            if on {
                list.chrome_rect_gradient(r, radius, 1.0, ACTIVE_TOP, ACTIVE_BOTTOM, ACTIVE_EDGE);
                list.quad(
                    r.x + 1.0,
                    r.y + 1.0,
                    (r.width - 2.0).max(0.0),
                    1.0,
                    ACTIVE_HI,
                );
            } else {
                let face = if hovered { HOVER } else { IDLE };
                list.chrome_rect(r, radius, 1.0, a(face), a(IDLE_EDGE));
                list.box_shadow_inset(
                    r.inset(1.0),
                    CornerRadii::uniform(0.0),
                    BoxShadow {
                        offset: [0.0, 2.0],
                        blur: 3.0,
                        color: a(IDLE_RECESS),
                        inset: true,
                        ..BoxShadow::default()
                    },
                );
            }
            draw_parts(list, s, tab, r, on, fade);
        }
        clicked
    }
}

/// Lay a tab's glyph, label, count and dot out centred in `r`.
fn draw_parts(list: &mut DrawList, s: &StyleResolver, tab: &SpanTab, r: Rect, on: bool, fade: f32) {
    let (ink, glyph_ink) = match (tab.disabled, on) {
        (true, _) => (Ink::Disabled, Ink::DisabledGlyph),
        (false, true) => (Ink::Max, Ink::Icon),
        (false, false) => (Ink::Tab, Ink::Caption),
    };
    let tone = |mut c: [f32; 4]| {
        c[3] *= fade;
        c
    };
    let label_size = s.text_size(TextSize::Menu);
    let meta = s.text_size(TextSize::Meta);
    // Measured in the faces they are drawn in.
    let sans = s.theme().font.as_ref();
    let glyph_w = if tab.glyph.is_empty() {
        0.0
    } else {
        list.measure_text_with_font(tab.glyph, GLYPH_SIZE, None, sans)
            .0
            + PART_GAP
    };
    let count_w = if tab.count.is_empty() {
        0.0
    } else {
        s.mono_width(list, tab.count, TextSize::Meta) + PART_GAP
    };
    let dot = tab.dirty && !on;
    let dot_w = if dot { DOT + PART_GAP } else { 0.0 };
    let room = (r.width - TAB_PAD * 2.0 - glyph_w - count_w - dot_w).max(0.0);
    let label_w = s.sans_width(list, tab.label, TextSize::Menu).min(room);
    let total = glyph_w + label_w + count_w + dot_w;
    let mut x = r.x + ((r.width - total) * 0.5).max(TAB_PAD);
    let cy = r.y + r.height * 0.5;
    if !tab.glyph.is_empty() {
        list.text(
            s.sans_block(
                tab.glyph,
                x,
                crate::text::vcentered_line_y(r.y, r.height, GLYPH_SIZE),
                TextSize::Meta,
                glyph_ink,
            )
            .with_size(GLYPH_SIZE)
            .with_color_f32(tone(s.ink(glyph_ink))),
        );
        x += glyph_w;
    }
    list.text(
        s.sans_block(
            tab.label,
            x,
            crate::text::vcentered_line_y(r.y, r.height, label_size),
            TextSize::Menu,
            ink,
        )
        .with_color_f32(tone(s.ink(ink)))
        .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0)
        .with_max_width(label_w + 0.5)
        .with_ellipsis(),
    );
    x += label_w + PART_GAP;
    if !tab.count.is_empty() {
        list.text(
            s.mono_block(
                tab.count,
                x,
                crate::text::vcentered_line_y(r.y, r.height, meta),
                TextSize::Meta,
                Ink::Caption,
            )
            .with_color_f32(tone(s.ink(Ink::Caption))),
        );
        x += count_w;
    }
    if dot {
        let color = s.color(StyleKey::AccentDirty);
        let mut glow = s.color(StyleKey::Accent);
        glow[3] = 0.65;
        let dot = Rect::new(x, cy - DOT * 0.5, DOT, DOT);
        list.box_shadow_outset(
            dot,
            CornerRadii::uniform(DOT * 0.5),
            BoxShadow {
                blur: 5.0,
                color: glow,
                ..BoxShadow::default()
            },
        );
        list.rounded_rect(dot, DOT * 0.5, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn tabs() -> [SpanTab<'static>; 4] {
        [
            SpanTab::new("Chat").glyph("›"),
            SpanTab::new("Forum").glyph("◫").dirty(true),
            SpanTab::new("Files").glyph("◧").count("12"),
            SpanTab::new("Terminal").glyph("⌗").disabled(true),
        ]
    }

    fn click(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    #[test]
    fn tabs_share_the_width_and_the_active_one_sits_lower() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let tabs = tabs();
        let strip = SpanTabs::new(&tabs);
        let rect = Rect::new(0.0, 0.0, 403.0, SPAN_TABS_HEIGHT);
        let a = strip.slot(rect, 0);
        let d = strip.slot(rect, 3);
        assert_eq!(a.x, STRIP_PAD);
        assert!(
            (d.right() - (rect.width - STRIP_PAD)).abs() < 0.01,
            "the last ends at the pad"
        );
        assert!((a.width - d.width).abs() < 0.01);

        let mut list = DrawList::new();
        strip.draw(rect, 0, &mut list, &s, &InputState::default());
        let active = list
            .chrome_instances()
            .find(|c| c.bg == ACTIVE_TOP && c.bg2 == ACTIVE_BOTTOM)
            .unwrap();
        assert_eq!(active.rect[1], STEP, "the active tab sits lower");
        for text in ["Chat", "Forum", "Files", "12", "›"] {
            assert!(list.texts.iter().any(|t| t.content == text), "{text}");
        }
    }

    #[test]
    fn a_click_picks_an_enabled_other_tab() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let tabs = tabs();
        let strip = SpanTabs::new(&tabs);
        let rect = Rect::new(0.0, 0.0, 400.0, SPAN_TABS_HEIGHT);
        let centre = |i: usize| {
            let slot = strip.slot(rect, i);
            (slot.x + slot.width * 0.5, 10.0)
        };
        let mut list = DrawList::new();
        let (x, y) = centre(2);
        assert_eq!(strip.draw(rect, 0, &mut list, &s, &click(x, y)), Some(2));
        let (x, y) = centre(3);
        assert_eq!(
            strip.draw(rect, 0, &mut list, &s, &click(x, y)),
            None,
            "disabled"
        );
        let (x, y) = centre(0);
        assert_eq!(
            strip.draw(rect, 0, &mut list, &s, &click(x, y)),
            None,
            "already active"
        );
    }
}
