//! Badges, keycaps, and chips — the design's small "callout" atoms (Gallery III).
//!
//! - [`Badge`]: a short mono label on a sunken plate, tinted by a
//!   [`BadgeTone`] — one of Forge's status tones or any hue. `.compact()`
//!   makes the small, case-keeping version for tight rows (a session row's
//!   provider).
//! - [`keycap`]: a keyboard-key cap (`⇧` `Ctrl` `F`) — raised face over a
//!   black edge with a bottom drop line.
//! - [`chip`]: a toggleable filter pill — raised at rest, held-in when on.

use crate::color::{HUE_ACCENT, HUE_DANGER, HUE_OK, HUE_WARN, HUE_WARN_INK, oklch};
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::DrawList;
use super::material;

/// Draw a keycap: a keyboard-key cap with `label` (design: raised white-sheen
/// face over a black edge, plus a dark line under the bottom edge selling the
/// key's side). `min_w` floors the width so single-glyph caps read as keys.
pub fn keycap(list: &mut DrawList, s: &StyleResolver, rect: Rect, label: &str, min_w: f32) -> Rect {
    let font_size = s.scalar(StyleKey::FontSize) * 0.85;
    let (tw, _) = list.measure_text(label, font_size, None);
    let h = 18.0f32.max(font_size + 8.0).min(rect.height);
    let w = (min_w.max(tw + 10.0)).min(rect.width);
    let r = Rect::new(rect.x, rect.y + (rect.height - h) * 0.5, w, h);
    let radius = s.scalar(StyleKey::BorderRadius);

    // Side: a dark line under the cap.
    list.quad(r.x, r.y + h - 1.0, w, 1.0, [0.0, 0.0, 0.0, 0.5]);

    let base = s.color(StyleKey::Button);
    let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
    let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
    list.chrome_rect_gradient(
        Rect::new(r.x, r.y, w, h - 1.0),
        radius,
        1.0,
        top,
        bottom,
        [0.0, 0.0, 0.0, 0.6],
    );
    // Inset highlight under the top edge.
    let hl = s.color(StyleKey::EdgeHighlight);
    list.quad(r.x + 1.0, r.y + 1.0, (w - 2.0).max(0.0), 1.0, hl);

    let text_color = s.color(StyleKey::Text);
    let text_y = list.vcentered_text_y(r.y, h - 1.0, font_size, s.theme().font.as_ref(), label);
    list.text(
        TextBlock::new(label, r.x + (w - tw) * 0.5, text_y)
            .with_size(font_size)
            .with_color_f32(text_color)
            .with_font_opt(s.theme().font.clone()),
    );
    r
}

/// Height of a regular [`Badge`] (Forge Badge: 9px mono caps, `1px 7px 2px`
/// padding inside a 1px edge).
pub const BADGE_HEIGHT: f32 = 15.0;
/// Height of a [`compact`](Badge::compact) [`Badge`].
pub const BADGE_COMPACT_HEIGHT: f32 = 13.0;
/// Space left and right of a regular badge's text.
const BADGE_PAD: f32 = 7.0;
/// Space left and right of a compact badge's text.
const BADGE_COMPACT_PAD: f32 = 4.0;
/// Letter spacing of a regular badge (`--track-badge`), in em.
const BADGE_TRACKING: f32 = 0.08;
/// Letter spacing of a compact badge, in em.
const BADGE_COMPACT_TRACKING: f32 = 0.02;
/// The light line under a badge's plate: the lower lip of its recess.
const BADGE_LIP: [f32; 4] = [1.0, 1.0, 1.0, 0.07];

/// The color of a [`Badge`]'s plate and text: one of Forge's status tones, or
/// any hue.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BadgeTone {
    /// Neutral (`draft`): a dark well with the neutral chip ink.
    #[default]
    Draft,
    /// Done / succeeded (`baked`, green).
    Baked,
    /// Out of date / attention (`stale`, amber).
    Stale,
    /// Failed (`error`, red).
    Error,
    /// Happening now (`live`, accent).
    Live,
    /// Any hue, in degrees (OKLCH): a darker, quieter plate than the status
    /// tones, for labels that tell things apart rather than report a state
    /// (a session's provider).
    Hue(f32),
}

/// A badge's plate: gradient top and bottom, ink, edge, and recess.
struct BadgePaint {
    top: [f32; 4],
    bottom: [f32; 4],
    ink: [f32; 4],
    edge: [f32; 4],
    /// The inner shadow's blur and alpha (`--chip-inset[-neutral]`).
    inset: (f32, f32),
}

impl BadgeTone {
    /// The tone's paint. `on_accent` flattens the plate so it reads on a
    /// selected (accent) row's brighter fill.
    fn paint(self, s: &StyleResolver, on_accent: bool) -> BadgePaint {
        let edge_hard = [0.0, 0.0, 0.0, 0.65];
        let mut paint = match self {
            BadgeTone::Draft => BadgePaint {
                top: [0.0, 0.0, 0.0, 0.45],
                bottom: [1.0, 1.0, 1.0, 0.05],
                ink: s.ink(Ink::Chip),
                edge: [0.0, 0.0, 0.0, 0.6],
                inset: (2.0, 0.55),
            },
            BadgeTone::Baked => BadgePaint {
                top: oklch(0.42, 0.1, HUE_OK, 1.0),
                bottom: oklch(0.5, 0.12, HUE_OK, 1.0),
                ink: oklch(0.92, 0.11, HUE_OK, 1.0),
                edge: edge_hard,
                inset: (3.0, 0.5),
            },
            BadgeTone::Stale => BadgePaint {
                top: oklch(0.44, 0.09, HUE_WARN, 1.0),
                bottom: oklch(0.53, 0.11, HUE_WARN, 1.0),
                ink: oklch(0.93, 0.11, HUE_WARN_INK, 1.0),
                edge: edge_hard,
                inset: (3.0, 0.5),
            },
            BadgeTone::Error => BadgePaint {
                top: oklch(0.36, 0.13, HUE_DANGER, 1.0),
                bottom: oklch(0.45, 0.15, HUE_DANGER, 1.0),
                ink: oklch(0.9, 0.11, HUE_DANGER, 1.0),
                edge: [0.0, 0.0, 0.0, 0.7],
                inset: (3.0, 0.55),
            },
            BadgeTone::Live => BadgePaint {
                top: oklch(0.4, 0.07, HUE_ACCENT, 1.0),
                bottom: oklch(0.48, 0.08, HUE_ACCENT, 1.0),
                ink: oklch(0.92, 0.09, HUE_ACCENT, 1.0),
                edge: edge_hard,
                inset: (3.0, 0.5),
            },
            // On the accent a hue plate goes flat at its own, slightly more
            // chromatic color, with a brighter ink to hold contrast.
            BadgeTone::Hue(hue) if on_accent => {
                let flat = oklch(0.30, 0.06, hue, 1.0);
                BadgePaint {
                    top: flat,
                    bottom: flat,
                    ink: oklch(0.88, 0.08, hue, 1.0),
                    edge: edge_hard,
                    inset: (3.0, 0.5),
                }
            }
            BadgeTone::Hue(hue) => BadgePaint {
                top: oklch(0.30, 0.055, hue, 1.0),
                bottom: oklch(0.36, 0.07, hue, 1.0),
                ink: oklch(0.86, 0.09, hue, 1.0),
                edge: edge_hard,
                inset: (3.0, 0.5),
            },
        };
        if on_accent {
            paint.bottom = paint.top;
        }
        paint
    }
}

/// A Forge `Badge`: a short mono label on a sunken plate stamped into the
/// surface, colored by its [`BadgeTone`]. It sizes itself to its text.
///
/// A regular badge is Forge's: capitals, [`BADGE_HEIGHT`] tall, inside a dark
/// edge. A [`compact`](Self::compact) one is [`BADGE_COMPACT_HEIGHT`] tall,
/// keeps the text's case, and has no edge — for tight rows such as a session
/// list's provider label.
///
/// ```ignore
/// Badge::new(BadgeTone::Live).draw(list, s, x, y, "running");
/// Badge::new(BadgeTone::Hue(160.0))
///     .compact()
///     .on_accent(row_selected)
///     .draw_right(list, s, column_right, y, "codex");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Badge {
    tone: BadgeTone,
    compact: bool,
    on_accent: bool,
}

impl Badge {
    /// A regular badge in `tone`.
    pub fn new(tone: BadgeTone) -> Self {
        Self {
            tone,
            ..Self::default()
        }
    }

    /// Make it the compact badge: smaller, case kept, no edge.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Whether the badge sits on a selected (accent) row, which flattens its
    /// plate so it reads on the brighter fill.
    pub fn on_accent(mut self, on_accent: bool) -> Self {
        self.on_accent = on_accent;
        self
    }

    /// The badge's height: [`BADGE_HEIGHT`], or [`BADGE_COMPACT_HEIGHT`] when
    /// compact.
    pub fn height(&self) -> f32 {
        if self.compact {
            BADGE_COMPACT_HEIGHT
        } else {
            BADGE_HEIGHT
        }
    }

    /// Space left and right of the text.
    fn pad(&self) -> f32 {
        if self.compact {
            BADGE_COMPACT_PAD
        } else {
            BADGE_PAD
        }
    }

    /// The badge's text block at the origin: capitals (Forge's
    /// `text-transform`) unless compact.
    fn text_block(&self, s: &StyleResolver, text: &str) -> TextBlock {
        let size = s.text_size(TextSize::Caption);
        let (content, tracking) = if self.compact {
            (text.to_owned(), BADGE_COMPACT_TRACKING)
        } else {
            (text.to_uppercase(), BADGE_TRACKING)
        };
        s.mono_block(content, 0.0, 0.0, TextSize::Caption, Ink::Chip)
            .with_letter_spacing(size * tracking)
    }

    /// The width the badge takes when showing `text`.
    pub fn width(&self, list: &mut DrawList, s: &StyleResolver, text: &str) -> f32 {
        let (w, _) = list.measure_block(&self.text_block(s, text));
        w + self.pad() * 2.0
    }

    /// Draw the badge showing `text` with its top-left corner at `(x, y)`.
    /// Returns the badge's rect.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver, x: f32, y: f32, text: &str) -> Rect {
        let block = self.text_block(s, text);
        let (w, _) = list.measure_block(&block);
        let r = Rect::new(x, y, w + self.pad() * 2.0, self.height());
        self.paint(list, s, r, block);
        r
    }

    /// [`draw`](Self::draw), placed by its top-right corner `(right, y)`
    /// instead: for a badge ending at a column edge, without measuring its
    /// text twice.
    pub fn draw_right(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        right: f32,
        y: f32,
        text: &str,
    ) -> Rect {
        let block = self.text_block(s, text);
        let (w, _) = list.measure_block(&block);
        let width = w + self.pad() * 2.0;
        let r = Rect::new(right - width, y, width, self.height());
        self.paint(list, s, r, block);
        r
    }

    /// Paint the plate in `r`, and the measured text `block` on it.
    fn paint(&self, list: &mut DrawList, s: &StyleResolver, r: Rect, mut block: TextBlock) {
        let paint = self.tone.paint(s, self.on_accent);
        let radius = s.scalar(StyleKey::BorderRadius);
        let recess = |(blur, alpha): (f32, f32)| BoxShadow {
            offset: [0.0, 1.0],
            blur,
            color: [0.0, 0.0, 0.0, alpha],
            inset: true,
            ..BoxShadow::default()
        };
        if self.compact {
            // No edge: the recess follows the plate's own rounded outline.
            list.chrome_rect_gradient(r, radius, 0.0, paint.top, paint.bottom, [0.0; 4]);
            list.box_shadow_inset(r, CornerRadii::uniform(radius), recess(paint.inset));
            block = block.with_color_f32(paint.ink);
            block.y = crate::text::vcentered_line_y(r.y, r.height, block.font_size);
        } else {
            list.chrome_rect_gradient(r, radius, 1.0, paint.top, paint.bottom, paint.edge);
            list.box_shadow_inset(r.inset(1.0), CornerRadii::uniform(0.0), recess(paint.inset));
            block = block
                .with_color_f32(paint.ink)
                .with_shadow(0, 0, 0, 153, 0.0, -1.0, 0.0);
            // `padding: 1px 7px 2px`: the line sits half a pixel above centre.
            block.y = crate::text::vcentered_line_y(r.y, r.height - 1.0, block.font_size);
        }
        list.quad(r.x, r.bottom(), r.width, 1.0, BADGE_LIP);
        block.x = r.x + self.pad();
        list.text(block);
    }
}

/// Height of a [`chip`] (`--h-chip`).
pub const CHIP_HEIGHT: f32 = 18.0;
/// Space left and right of a chip's label (`padding: 2px 9px 3px`).
const CHIP_PAD: f32 = 9.0;

/// Outcome of drawing a [`chip`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipOutput {
    /// The chip was clicked this frame (caller flips its `on` state).
    pub clicked: bool,
    /// The rect drawn.
    pub rect: Rect,
}

/// The width a [`chip`] labelled `label` takes.
pub fn chip_width(list: &mut DrawList, s: &StyleResolver, label: &str) -> f32 {
    let (w, _) = list.measure_text(label, s.text_size(TextSize::Dense), None);
    w + CHIP_PAD * 2.0
}

/// Draw a Forge `FilterChip` at the left of `rect`, centred vertically: a
/// latching pill key, up (key face) when off and held down in the well
/// (accent chip) when on. Click handling honors
/// [`InputState::mouse_consumed`](crate::InputState::mouse_consumed).
pub fn chip(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    label: &str,
    on: bool,
    input: &crate::InputState,
) -> ChipOutput {
    let font_size = s.text_size(TextSize::Dense);
    let (tw, _) = list.measure_text(label, font_size, None);
    let h = CHIP_HEIGHT.min(rect.height);
    let w = (tw + CHIP_PAD * 2.0).min(rect.width);
    let r = Rect::new(rect.x, rect.y + (rect.height - h) * 0.5, w, h);
    let radius = 9.0f32.min(h * 0.5);

    let hovered = r.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
    let clicked = hovered && input.mouse_clicked;

    let fg = if on {
        // Held in the well: the accent chip, pressed in.
        let top = oklch(0.42, 0.07, HUE_ACCENT, 1.0);
        let bottom = oklch(0.52, 0.09, HUE_ACCENT, 1.0);
        list.chrome_rect_gradient(r, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.65]);
        list.box_shadow_inset(
            r.inset(1.0),
            CornerRadii::uniform((radius - 1.0).max(0.0)),
            BoxShadow {
                offset: [0.0, 2.0],
                blur: 4.0,
                color: [0.0, 0.0, 0.0, 0.5],
                inset: true,
                ..BoxShadow::default()
            },
        );
        oklch(0.93, 0.08, HUE_ACCENT, 1.0)
    } else {
        // Up: the key face (`--key-face`, lighter on hover).
        let (top, bottom) = if hovered {
            (
                s.color(StyleKey::FaceTopHover),
                s.color(StyleKey::FaceBottomHover),
            )
        } else {
            (s.color(StyleKey::FaceTop), s.color(StyleKey::FaceBottom))
        };
        let base = s.color(StyleKey::Button);
        list.chrome_rect_gradient(
            r,
            radius,
            1.0,
            material::sheen_over(base, top),
            material::sheen_over(base, bottom),
            [0.0, 0.0, 0.0, 0.5],
        );
        // A pill's top edge is curved; a straight, full-width 1px highlight
        // reads as a conspicuous white slash, so the face gradient carries
        // the sheen alone.
        s.ink(Ink::Icon)
    };
    let text_y = crate::text::vcentered_line_y(r.y, r.height - 1.0, font_size);
    list.text(
        s.sans_block(label, r.x + CHIP_PAD, text_y, TextSize::Dense, Ink::Icon)
            .with_color_f32(fg)
            .with_shadow(0, 0, 0, 153, 0.0, -1.0, 0.0),
    );
    ChipOutput { clicked, rect: r }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn a_badge_sizes_to_its_text() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let badge = Badge::new(BadgeTone::Baked);
        let a = badge.draw(&mut list, &s, 0.0, 0.0, "ok");
        let b = badge.draw(&mut list, &s, 0.0, 0.0, "outdated");
        assert!(b.width > a.width, "wider text draws a wider badge");
        assert_eq!((a.height, b.height), (BADGE_HEIGHT, BADGE_HEIGHT));
    }

    #[test]
    fn keycap_is_at_least_min_wide() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let r = keycap(&mut list, &s, Rect::new(0.0, 0.0, 200.0, 24.0), "F", 24.0);
        assert!(r.width >= 24.0);
        assert!(r.height > 0.0);
        // Cap face + highlight band + side line are all instanced quads; the
        // label is a text block.
        assert!(
            list.chrome_instance_count() >= 2,
            "cap face + highlight/side bands instanced"
        );
        assert!(!list.texts.is_empty(), "label block");
    }

    #[test]
    fn idle_chip_uses_its_gradient_sheen_without_a_white_top_line() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        chip(
            &mut list,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            false,
            &crate::InputState::default(),
        );
        assert_eq!(
            list.chrome_instance_count(),
            1,
            "only the pill face is painted"
        );
        assert_ne!(
            list.chrome_instance(0).unwrap().bg,
            list.chrome_instance(0).unwrap().bg2,
            "face retains its vertical sheen"
        );
    }

    #[test]
    fn chip_on_and_off_paint_different_faces_and_report_clicks() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let input = crate::InputState {
            mouse_x: 10.0,
            mouse_y: 10.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut off = DrawList::new();
        let off_out = chip(
            &mut off,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            false,
            &input,
        );
        let mut on = DrawList::new();
        let on_out = chip(
            &mut on,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            true,
            &input,
        );
        assert!(off_out.clicked && on_out.clicked, "click inside reports");
        assert_ne!(
            off.chrome_instance(0).unwrap().bg,
            on.chrome_instance(0).unwrap().bg,
            "off is a raised neutral, on is the held accent"
        );

        let far = crate::InputState {
            mouse_x: 500.0,
            mouse_y: 500.0,
            ..Default::default()
        };
        let mut quiet = DrawList::new();
        let out = chip(
            &mut quiet,
            &s,
            Rect::new(0.0, 0.0, 100.0, 24.0),
            "info",
            false,
            &far,
        );
        assert!(!out.clicked);
    }

    #[test]
    fn a_compact_hue_badge_keeps_its_case_and_flattens_on_the_accent() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let badge = Badge::new(BadgeTone::Hue(160.0)).compact();
        let mut plain = DrawList::new();
        let r = badge.draw(&mut plain, &s, 5.0, 7.0, "codex");
        assert_eq!((r.x, r.y, r.height), (5.0, 7.0, BADGE_COMPACT_HEIGHT));
        assert!((r.width - badge.width(&mut plain, &s, "codex")).abs() < 0.01);
        let face = plain.chrome_instance(0).unwrap();
        assert_eq!(face.bg, oklch(0.30, 0.055, 160.0, 1.0));
        assert_ne!(face.bg, face.bg2, "a gradient at rest");
        assert_eq!(face.widths, [0.0; 4], "compact has no edge");
        assert_eq!(plain.shadow_instance_count(), 1, "the recess");
        let text = plain.texts.iter().find(|t| t.content == "codex").unwrap();
        assert_eq!(
            text.color,
            crate::color::text_color(oklch(0.86, 0.09, 160.0, 1.0))
        );
        assert_eq!(text.x, 5.0 + BADGE_COMPACT_PAD);

        let mut accent = DrawList::new();
        badge
            .on_accent(true)
            .draw(&mut accent, &s, 0.0, 0.0, "codex");
        let face = accent.chrome_instance(0).unwrap();
        assert_eq!(face.bg, face.bg2, "flat on the accent");
        assert_eq!(face.bg, oklch(0.30, 0.06, 160.0, 1.0));
    }

    #[test]
    fn compact_changes_the_size_and_case_but_not_the_tone() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut regular = DrawList::new();
        let r = Badge::new(BadgeTone::Error).draw(&mut regular, &s, 0.0, 0.0, "exit 1");
        let mut compact = DrawList::new();
        let c = Badge::new(BadgeTone::Error)
            .compact()
            .draw(&mut compact, &s, 0.0, 0.0, "exit 1");
        assert!(c.height < r.height && c.width < r.width);
        let (rf, cf) = (
            regular.chrome_instance(0).unwrap(),
            compact.chrome_instance(0).unwrap(),
        );
        assert_eq!((rf.bg, rf.bg2), (cf.bg, cf.bg2), "same plate colors");
        assert_eq!(rf.widths, [1.0; 4], "regular keeps its edge");
        assert!(regular.texts.iter().any(|t| t.content == "EXIT 1"));
        assert!(compact.texts.iter().any(|t| t.content == "exit 1"));
    }

    #[test]
    fn on_accent_flattens_a_status_tone_too() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        Badge::new(BadgeTone::Live)
            .on_accent(true)
            .draw(&mut list, &s, 0.0, 0.0, "running");
        let face = list.chrome_instance(0).unwrap();
        assert_eq!(face.bg, face.bg2);
        assert_eq!(face.bg, oklch(0.4, 0.07, HUE_ACCENT, 1.0));
    }

    #[test]
    fn a_toned_badge_is_uppercase_and_takes_its_tone() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let badge = Badge::new(BadgeTone::Live);
        let r = badge.draw(&mut list, &s, 4.0, 6.0, "running");
        assert_eq!((r.x, r.y, r.height), (4.0, 6.0, BADGE_HEIGHT));
        assert!((r.width - badge.width(&mut list, &s, "running")).abs() < 0.01);
        let face = list.chrome_instance(0).unwrap();
        assert_eq!(face.bg, oklch(0.4, 0.07, HUE_ACCENT, 1.0));
        assert_eq!(face.bg2, oklch(0.48, 0.08, HUE_ACCENT, 1.0));
        let text = list.texts.iter().find(|t| t.content == "RUNNING").unwrap();
        assert_eq!(text.x, 4.0 + BADGE_PAD);
        assert_eq!(
            text.color,
            crate::color::text_color(oklch(0.92, 0.09, HUE_ACCENT, 1.0))
        );
        assert_eq!(list.shadow_instance_count(), 1, "the recess");

        let mut other = DrawList::new();
        Badge::new(BadgeTone::Error).draw(&mut other, &s, 0.0, 0.0, "exit 1");
        assert_ne!(
            other.chrome_instance(0).unwrap().bg,
            face.bg,
            "each tone has its own plate"
        );
    }

    #[test]
    fn an_on_chip_is_the_accent_chip_and_reports_its_rect() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let out = chip(
            &mut list,
            &s,
            Rect::new(10.0, 0.0, 200.0, 28.0),
            "all sessions",
            true,
            &crate::InputState::default(),
        );
        assert_eq!(out.rect.height, CHIP_HEIGHT);
        assert_eq!(out.rect.y, 5.0, "centred in its rect");
        assert!((out.rect.width - chip_width(&mut list, &s, "all sessions")).abs() < 0.01);
        assert_eq!(
            list.chrome_instance(0).unwrap().bg,
            oklch(0.42, 0.07, HUE_ACCENT, 1.0)
        );
        // A click beside the pill but inside the rect is not a click on it.
        let beside = crate::InputState {
            mouse_x: 205.0,
            mouse_y: 14.0,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut quiet = DrawList::new();
        let out = chip(
            &mut quiet,
            &s,
            Rect::new(10.0, 0.0, 200.0, 28.0),
            "all sessions",
            false,
            &beside,
        );
        assert!(!out.clicked);
    }

    #[test]
    fn a_right_anchored_badge_ends_at_its_edge() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let badge = Badge::new(BadgeTone::Hue(160.0)).compact();
        let w = badge.width(&mut list, &s, "codex");
        let r = badge.draw_right(&mut list, &s, 100.0, 7.0, "codex");
        assert!((r.right() - 100.0).abs() < 0.01);
        assert!((r.width - w).abs() < 0.01);
        assert_eq!((r.y, r.height), (7.0, BADGE_COMPACT_HEIGHT));
        let text = list.texts.iter().find(|t| t.content == "codex").unwrap();
        assert_eq!(text.x, r.x + BADGE_COMPACT_PAD);
    }
}
