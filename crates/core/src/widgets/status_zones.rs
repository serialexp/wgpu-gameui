//! Zoned status bar — Forge `StatusBar` with dock toggles and rich zones.
//!
//! A 26px band in 10px mono. At the left, dock toggle keys (latched to the
//! accent while their dock shows); then zones, hairline-divided, left to
//! right; and a group of zones pushed to the right end. A zone is a row of
//! parts: text in any ink, a status dot, an inline meter, or a value held
//! right-aligned in a minimum width (so changing numbers don't jitter). A
//! zone can be clickable, and drawn pressed while what it opens shows.
//! A drop-up zone is a key with a `▴` after its parts, held in while the
//! detail it opens ([`StatusDetail`](super::StatusDetail)) shows above it;
//! the bar says where it was drawn ([`StatusBarOutput::drop_up`]).
//!
//! The left zones give way to the right group: their text is ellipsized
//! where it would run under it.

use crate::InputState;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleResolver, TextSize};

use super::DrawList;
use super::glyphs::up_triangle;
use super::meter::{INLINE_METER_HEIGHT, MeterFill, inline_meter};
use super::status_dot::{Status, status_dot};

/// Space inside the band's ends.
const BAND_PAD: f32 = 6.0;
/// A toggle key's side (`--key-status`), the space between toggles, and
/// after the last one.
const TOGGLE: f32 = 18.0;
const TOGGLE_GAP: f32 = 2.0;
const TOGGLE_AFTER: f32 = 9.0;
const TOGGLE_GLYPH: f32 = 11.0;
/// Space inside a left zone's and a right zone's ends, and between parts.
const LEFT_PAD: f32 = 11.0;
const RIGHT_PAD: f32 = 9.0;
const PART_GAP: f32 = 6.0;
/// A divider: 1px by 12px, with a light line to its right.
const DIVIDER_H: f32 = 12.0;
const DIVIDER: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const DIVIDER_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// A pressed zone's plate: 18px tall, 6px past the zone's parts each side.
const PRESSED_H: f32 = 18.0;
const PRESSED_OUT: f32 = 6.0;
const PRESSED: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
const PRESSED_RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// A drop-up zone's key: as tall as a toggle (`--key-status`), its parts
/// inset 5px in it; held in while open, lit while hovered.
const DROP_UP_INSET: f32 = 5.0;
const DROP_UP_OPEN: [f32; 4] = [0.0, 0.0, 0.0, 0.38];
const DROP_UP_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const DROP_UP_RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const DROP_UP_LIP: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// Its `▴` (7px text in Forge): this wide and tall, 5px after the parts.
const MARKER_W: f32 = 5.0;
const MARKER_H: f32 = 3.0;
const MARKER_GAP: f32 = PART_GAP - 1.0;
/// A toggle latched on (`--accent-toggle-*`, `--key-inset-latched`), and
/// hovered while off.
const TOGGLE_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const TOGGLE_HOVER: [f32; 4] = [0.184, 0.2, 0.22, 1.0];
const ON_LATCH: [f32; 4] = [0.918, 0.98, 1.0, 1.0];

/// One part of a [`StatusZone`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatusPart<'a> {
    /// Text in the band's ink (`--ink-muted`).
    Text(&'a str),
    /// Text in its own colour.
    Tinted(&'a str, [f32; 4]),
    /// Text in its own colour, right-aligned in at least this width.
    Value(&'a str, [f32; 4], f32),
    /// A 6px status dot.
    Dot(Status),
    /// An inline meter this wide.
    Meter(f32, MeterFill),
}

/// A zone of a [`ZonedStatusBar`]: parts in a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusZone<'a> {
    /// Its parts, left to right.
    pub parts: &'a [StatusPart<'a>],
    /// Report clicks on it.
    pub clickable: bool,
    /// Draw it held in (what it opens is showing).
    pub pressed: bool,
    /// A key that opens a detail above it (see [`Self::drop_up`]).
    pub drop_up: bool,
}

impl<'a> StatusZone<'a> {
    /// A zone of `parts`.
    pub const fn new(parts: &'a [StatusPart<'a>]) -> Self {
        Self {
            parts,
            clickable: false,
            pressed: false,
            drop_up: false,
        }
    }

    /// Report clicks on it, and draw it held in while `pressed`.
    pub const fn button(mut self, pressed: bool) -> Self {
        self.clickable = true;
        self.pressed = pressed;
        self
    }

    /// A key that opens a detail above it (Forge's zone with a `detail`):
    /// clicks are reported, it draws a `▴`, lights while hovered, and is
    /// held in while `open`. The bar says where it was drawn.
    pub const fn drop_up(mut self, open: bool) -> Self {
        self.clickable = true;
        self.pressed = open;
        self.drop_up = true;
        self
    }
}

/// Which end of the bar a zone is in: a drop-up aligns to its zone's left
/// edge in the left zones, and to its right edge in the right group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneSide {
    /// Among the zones after the toggles.
    Left,
    /// In the group at the bar's right end.
    Right,
}

/// Where a drop-up zone's key was drawn, for its detail to open above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneAnchor {
    /// The key.
    pub rect: Rect,
    /// Which end of the bar its zone is in.
    pub side: ZoneSide,
}

/// A dock toggle key at the band's left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusToggle<'a> {
    /// Its glyph ("◧", "◨").
    pub glyph: &'a str,
    /// Latched: its dock shows.
    pub on: bool,
}

/// What a [`ZonedStatusBar`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatusBarOutput {
    /// A toggle clicked (its index).
    pub toggled: Option<usize>,
    /// A clickable left zone clicked (its index).
    pub left: Option<usize>,
    /// A clickable right zone clicked (its index).
    pub right: Option<usize>,
    /// The drop-up zone clicked this frame, else the one drawn open: where
    /// its detail goes.
    pub drop_up: Option<ZoneAnchor>,
}

/// Forge's status bar: toggles, left zones, and a right-hand group.
#[derive(Clone, Copy, Debug)]
pub struct ZonedStatusBar<'a> {
    toggles: &'a [StatusToggle<'a>],
    left: &'a [StatusZone<'a>],
    right: &'a [StatusZone<'a>],
}

fn part_width(list: &mut DrawList, s: &StyleResolver, part: &StatusPart) -> f32 {
    match *part {
        StatusPart::Text(text) | StatusPart::Tinted(text, _) => mono_width(list, s, text),
        StatusPart::Value(text, _, min) => mono_width(list, s, text).max(min),
        StatusPart::Dot(status) => status.size(),
        StatusPart::Meter(width, _) => width,
    }
}

/// The width of `text` in the parts' mono face.
fn mono_width(list: &mut DrawList, s: &StyleResolver, text: &str) -> f32 {
    s.mono_width(list, text, TextSize::Meta)
}

fn parts_width(list: &mut DrawList, s: &StyleResolver, zone: &StatusZone) -> f32 {
    let parts: f32 = zone.parts.iter().map(|p| part_width(list, s, p)).sum();
    let marker = if zone.drop_up {
        MARKER_GAP + MARKER_W
    } else {
        0.0
    };
    parts + PART_GAP * zone.parts.len().saturating_sub(1) as f32 + marker
}

/// A drop-up zone's key in its zone `r`, whose parts are inset by `pad`.
fn drop_up_key(r: Rect, pad: f32) -> Rect {
    let inset = pad - DROP_UP_INSET;
    Rect::new(
        r.x + inset,
        r.y + (r.height - TOGGLE) * 0.5,
        r.width - 2.0 * inset,
        TOGGLE,
    )
}

/// Say where `zone`, drawn in `r`, opens its detail: always when it was
/// clicked, else while it is open and nothing was clicked.
fn note_drop_up(
    out: &mut StatusBarOutput,
    zone: &StatusZone,
    r: Rect,
    pad: f32,
    side: ZoneSide,
    clicked: bool,
) {
    if zone.drop_up && (clicked || (zone.pressed && out.drop_up.is_none())) {
        out.drop_up = Some(ZoneAnchor {
            rect: drop_up_key(r, pad),
            side,
        });
    }
}

fn divider(list: &mut DrawList, x: f32, cy: f32) {
    let y = cy - DIVIDER_H * 0.5;
    list.quad(x, y, 1.0, DIVIDER_H, DIVIDER);
    list.quad(x + 1.0, y, 1.0, DIVIDER_H, DIVIDER_HI);
}

impl<'a> ZonedStatusBar<'a> {
    /// A bar with `toggles`, then `left` zones, then the `right` group.
    pub fn new(
        toggles: &'a [StatusToggle<'a>],
        left: &'a [StatusZone<'a>],
        right: &'a [StatusZone<'a>],
    ) -> Self {
        Self {
            toggles,
            left,
            right,
        }
    }

    /// The rect of toggle `i` in a bar filling `rect`.
    fn toggle_rect(rect: Rect, i: usize) -> Rect {
        Rect::new(
            rect.x + BAND_PAD + i as f32 * (TOGGLE + TOGGLE_GAP),
            rect.y + (rect.height - TOGGLE) * 0.5,
            TOGGLE,
            TOGGLE,
        )
    }

    /// Draw the bar filling `rect`.
    pub fn draw(
        &self,
        rect: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
        input: &InputState,
    ) -> StatusBarOutput {
        list.push_debug_scope_rect("StatusBar", rect);
        let chrome = s.status_bar();
        list.paint_background_opaque(rect, chrome.surface.background);
        for line in chrome.lines {
            let r = Rect::new(
                rect.x,
                rect.y + line.offset,
                rect.width,
                (rect.height - line.offset).max(0.0),
            );
            list.edge_line(r, line.edge, line.style.thickness, line.style.color);
        }
        let mut out = StatusBarOutput::default();
        let over = |r: Rect| !input.mouse_consumed && r.contains(input.mouse_x, input.mouse_y);
        let cy = rect.y + rect.height * 0.5;

        let mut x = rect.x + BAND_PAD;
        for (i, toggle) in self.toggles.iter().enumerate() {
            let r = Self::toggle_rect(rect, i);
            let hovered = over(r);
            if hovered && input.mouse_clicked {
                out.toggled = Some(i);
            }
            draw_toggle(list, s, r, toggle, hovered);
            x = r.right() + TOGGLE_GAP;
        }
        if !self.toggles.is_empty() {
            x += TOGGLE_AFTER - TOGGLE_GAP;
        }

        // The right group first, so the left zones know where to stop.
        let widths: f32 = self
            .right
            .iter()
            .map(|z| parts_width(list, s, z) + RIGHT_PAD * 2.0)
            .sum();
        let right_w = widths + 2.0 * self.right.len().saturating_sub(1) as f32;
        let right_x = rect.right() - BAND_PAD - right_w;

        for (i, zone) in self.left.iter().enumerate() {
            if x >= right_x {
                break;
            }
            if i > 0 || !self.toggles.is_empty() {
                divider(list, x, cy);
                x += 2.0;
            }
            let w = parts_width(list, s, zone).min((right_x - x - LEFT_PAD * 2.0).max(0.0));
            let r = Rect::new(x, rect.y, w + LEFT_PAD * 2.0, rect.height);
            let clicked = draw_zone(list, s, zone, r, LEFT_PAD, over(r), input);
            if clicked {
                out.left = Some(i);
            }
            note_drop_up(&mut out, zone, r, LEFT_PAD, ZoneSide::Left, clicked);
            x = r.right();
        }

        let mut x = right_x;
        for (i, zone) in self.right.iter().enumerate() {
            if i > 0 {
                divider(list, x, cy);
                x += 2.0;
            }
            let w = parts_width(list, s, zone);
            let r = Rect::new(x, rect.y, w + RIGHT_PAD * 2.0, rect.height);
            let clicked = draw_zone(list, s, zone, r, RIGHT_PAD, over(r), input);
            if clicked {
                out.right = Some(i);
            }
            note_drop_up(&mut out, zone, r, RIGHT_PAD, ZoneSide::Right, clicked);
            x = r.right();
        }
        list.pop_debug_scope();
        out
    }
}

fn draw_toggle(
    list: &mut DrawList,
    s: &StyleResolver,
    r: Rect,
    toggle: &StatusToggle,
    hovered: bool,
) {
    let radius = s.scalar(crate::StyleKey::BorderRadius);
    let ink = if toggle.on {
        let top = crate::color::oklch(0.6, 0.1, 200.0, 1.0);
        let bottom = crate::color::oklch(0.68, 0.11, 200.0, 1.0);
        list.chrome_rect_gradient(r, radius, 1.0, top, bottom, TOGGLE_EDGE);
        // `--key-inset-latched`: a shade from above and a lit top line.
        list.box_shadow_inset(
            r.inset(1.0),
            CornerRadii::uniform(0.0),
            BoxShadow {
                offset: [0.0, 2.0],
                blur: 4.0,
                color: crate::color::oklch(0.32, 0.07, 200.0, 1.0),
                inset: true,
                ..BoxShadow::default()
            },
        );
        list.quad(
            r.x + 1.0,
            r.y + 1.0,
            r.width - 2.0,
            1.0,
            crate::color::oklch(0.55, 0.09, 200.0, 1.0),
        );
        ON_LATCH
    } else if hovered {
        list.chrome_rect(r, radius, 0.0, TOGGLE_HOVER, [0.0; 4]);
        s.ink(Ink::Value)
    } else {
        s.ink(Ink::Muted)
    };
    let (w, _) = list.measure_text(toggle.glyph, TOGGLE_GLYPH, None);
    list.text(
        s.sans_block(
            toggle.glyph,
            r.x + (r.width - w) * 0.5,
            crate::text::vcentered_line_y(r.y, r.height, TOGGLE_GLYPH),
            TextSize::Meta,
            Ink::Muted,
        )
        .with_size(TOGGLE_GLYPH)
        .with_color_f32(ink),
    );
}

/// Draw `zone` in `r` (its parts inset by `pad`); returns whether it was
/// clicked.
fn draw_zone(
    list: &mut DrawList,
    s: &StyleResolver,
    zone: &StatusZone,
    r: Rect,
    pad: f32,
    hovered: bool,
    input: &InputState,
) -> bool {
    let cy = r.y + r.height * 0.5;
    if zone.drop_up {
        draw_drop_up_key(list, s, drop_up_key(r, pad), zone.pressed, hovered);
    } else if zone.pressed {
        let plate = Rect::new(
            r.x + pad - PRESSED_OUT,
            cy - PRESSED_H * 0.5,
            r.width - 2.0 * (pad - PRESSED_OUT),
            PRESSED_H,
        );
        list.chrome_rect(plate, 1.0, 0.0, PRESSED, [0.0; 4]);
        list.box_shadow_inset(
            plate,
            CornerRadii::uniform(1.0),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 2.0,
                color: PRESSED_RECESS,
                inset: true,
                ..BoxShadow::default()
            },
        );
    }
    let size = s.text_size(TextSize::Meta);
    let ty = crate::text::vcentered_line_y(r.y, r.height, size);
    let end = r.right() - pad;
    // A drop-up's words light with its key.
    let ink = if zone.drop_up && (hovered || zone.pressed) {
        Ink::Value
    } else {
        Ink::Muted
    };
    let mut x = r.x + pad;
    for part in zone.parts {
        if x >= end {
            break;
        }
        let w = part_width(list, s, part);
        match *part {
            StatusPart::Text(text) => {
                list.text(
                    s.mono_block(text, x, ty, TextSize::Meta, ink)
                        .with_max_width(end - x)
                        .with_ellipsis(),
                );
            }
            StatusPart::Tinted(text, color) => {
                list.text(
                    s.mono_block(text, x, ty, TextSize::Meta, Ink::Muted)
                        .with_color_f32(color)
                        .with_max_width(end - x)
                        .with_ellipsis(),
                );
            }
            StatusPart::Value(text, color, _) => {
                let tw = mono_width(list, s, text);
                list.text(
                    s.mono_block(text, x + w - tw, ty, TextSize::Meta, Ink::Muted)
                        .with_color_f32(color),
                );
            }
            StatusPart::Dot(status) => {
                status_dot(list, s, (x + status.size() * 0.5, cy), status);
            }
            StatusPart::Meter(width, fill) => {
                let my = (cy - INLINE_METER_HEIGHT * 0.5).round();
                inline_meter(list, s, Rect::new(x, my, width, INLINE_METER_HEIGHT), fill);
            }
        }
        x += w + PART_GAP;
    }
    if zone.drop_up {
        let color = if zone.pressed {
            s.color(crate::StyleKey::Accent)
        } else {
            s.ink(Ink::Dim)
        };
        let marker = Rect::new(
            end - MARKER_W,
            (cy - MARKER_H * 0.5).round(),
            MARKER_W,
            MARKER_H,
        );
        up_triangle(list, marker, color);
    }
    zone.clickable && hovered && input.mouse_clicked
}

/// A drop-up zone's key: held in while `open` (Forge's `rgba(0,0,0,0.38)`
/// with a dark edge, a shade from above and a light lip under it), lit
/// while hovered, bare otherwise.
fn draw_drop_up_key(list: &mut DrawList, s: &StyleResolver, key: Rect, open: bool, hovered: bool) {
    let radius = s.scalar(crate::StyleKey::BorderRadius);
    if open {
        list.box_shadow_outset(
            key,
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                color: DROP_UP_LIP,
                ..BoxShadow::default()
            },
        );
        list.chrome_rect(key, radius, 1.0, DROP_UP_OPEN, DROP_UP_EDGE);
        list.box_shadow_inset(
            key.inset(1.0),
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 2.0,
                color: DROP_UP_RECESS,
                inset: true,
                ..BoxShadow::default()
            },
        );
    } else if hovered {
        list.chrome_rect(key, radius, 0.0, TOGGLE_HOVER, [0.0; 4]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn click(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    #[test]
    fn toggles_zones_and_the_right_group_report_their_clicks() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let toggles = [
            StatusToggle {
                glyph: "◧",
                on: true,
            },
            StatusToggle {
                glyph: "◨",
                on: false,
            },
        ];
        let path = [StatusPart::Text("/home/bart/Projects/agent-ui")];
        let bg = [
            StatusPart::Dot(Status::Running),
            StatusPart::Tinted("1 bg", s.ink(Ink::Row)),
        ];
        let fps = [StatusPart::Text("59.9 fps")];
        let left = [StatusZone::new(&path), StatusZone::new(&bg).button(false)];
        let right = [StatusZone::new(&fps).button(false)];
        let bar = ZonedStatusBar::new(&toggles, &left, &right);
        let rect = Rect::new(0.0, 100.0, 800.0, 26.0);
        let mut list = DrawList::new();

        let t = ZonedStatusBar::toggle_rect(rect, 1);
        let out = bar.draw(rect, &mut list, &s, &click(t.x + 4.0, t.y + 4.0));
        assert_eq!(out.toggled, Some(1));

        // "1 bg" sits after the path zone.
        let path_w = mono_width(&mut list, &s, path_text(&path));
        let bg_x = BAND_PAD
            + 2.0 * TOGGLE
            + TOGGLE_GAP
            + TOGGLE_AFTER
            + 2.0
            + path_w
            + 2.0 * LEFT_PAD
            + 2.0
            + LEFT_PAD;
        let out = bar.draw(rect, &mut list, &s, &click(bg_x + 3.0, 113.0));
        assert_eq!(
            out,
            StatusBarOutput {
                left: Some(1),
                ..Default::default()
            }
        );

        let out = bar.draw(rect, &mut list, &s, &click(790.0, 113.0));
        assert_eq!(out.right, Some(0), "the right group ends at the band's end");
        // The path is not a button.
        let out = bar.draw(rect, &mut list, &s, &click(BAND_PAD + 60.0, 113.0));
        assert_eq!(out, StatusBarOutput::default());
    }

    fn path_text<'a>(parts: &[StatusPart<'a>]) -> &'a str {
        match parts[0] {
            StatusPart::Text(t) => t,
            _ => unreachable!(),
        }
    }

    #[test]
    fn values_hold_their_width_and_left_text_gives_way_to_the_right() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let long = [StatusPart::Text(
            "/a/very/long/path/that/would/run/under/the/right/hand/group/of/zones",
        )];
        let row = s.ink(Ink::Row);
        let mem = [
            StatusPart::Text("client"),
            StatusPart::Value("9 MB", row, 50.0),
        ];
        let left = [StatusZone::new(&long)];
        let right = [StatusZone::new(&mem)];
        let bar = ZonedStatusBar::new(&[], &left, &right);
        let mut list = DrawList::new();
        bar.draw(
            Rect::new(0.0, 0.0, 260.0, 26.0),
            &mut list,
            &s,
            &InputState::default(),
        );
        let vw = mono_width(&mut list, &s, "9 MB");
        let value = list.texts.iter().find(|t| t.content == "9 MB").unwrap();
        assert!(
            (value.x + vw - (260.0 - BAND_PAD - RIGHT_PAD)).abs() < 0.5,
            "right-aligned at the end"
        );
        let path = list
            .texts
            .iter()
            .find(|t| t.content.starts_with("/a/very"))
            .unwrap();
        let client = list.texts.iter().find(|t| t.content == "client").unwrap();
        assert!(
            path.x + path.max_width <= client.x,
            "ellipsized before the group"
        );
    }

    #[test]
    fn a_drop_up_zone_says_where_its_key_is_when_clicked_and_while_open() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let conn = [
            StatusPart::Dot(Status::Running),
            StatusPart::Text("connected"),
        ];
        let fps = [StatusPart::Text("59.9 fps")];
        let rect = Rect::new(0.0, 100.0, 800.0, 26.0);
        let mut list = DrawList::new();

        // Closed and not clicked: nowhere to open.
        let right = [StatusZone::new(&conn).drop_up(false), StatusZone::new(&fps)];
        let bar = ZonedStatusBar::new(&[], &[], &right);
        let out = bar.draw(rect, &mut list, &s, &InputState::default());
        assert_eq!(out.drop_up, None);

        // Open: its key, as tall as a toggle and centred in the band, at the
        // right end's side.
        let right = [StatusZone::new(&conn).drop_up(true), StatusZone::new(&fps)];
        let bar = ZonedStatusBar::new(&[], &[], &right);
        let out = bar.draw(rect, &mut list, &s, &InputState::default());
        let anchor = out.drop_up.expect("open drop-up anchors");
        assert_eq!(anchor.side, ZoneSide::Right);
        assert_eq!(anchor.rect.height, TOGGLE);
        assert_eq!(anchor.rect.y, rect.y + (rect.height - TOGGLE) * 0.5);
        let fps_w = mono_width(&mut list, &s, "59.9 fps") + 2.0 * RIGHT_PAD;
        assert!(anchor.rect.right() <= rect.right() - BAND_PAD - fps_w);

        // A click on it reports both the zone and the key, even while shut.
        let right = [StatusZone::new(&conn).drop_up(false), StatusZone::new(&fps)];
        let bar = ZonedStatusBar::new(&[], &[], &right);
        let (cx, cy) = (anchor.rect.x + 4.0, anchor.rect.y + 4.0);
        let out = bar.draw(rect, &mut list, &s, &click(cx, cy));
        assert_eq!(out.right, Some(0));
        assert_eq!(out.drop_up, Some(anchor));
    }

    #[test]
    fn a_drop_up_zone_makes_room_for_its_marker() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let conn = [StatusPart::Text("connected")];
        let mut list = DrawList::new();
        let plain = parts_width(&mut list, &s, &StatusZone::new(&conn));
        let drop_up = parts_width(&mut list, &s, &StatusZone::new(&conn).drop_up(false));
        assert_eq!(drop_up, plain + MARKER_GAP + MARKER_W);
    }

    #[test]
    fn parts_are_spaced_by_the_width_they_draw_at() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let row = s.ink(Ink::Row);
        let last = [
            StatusPart::Text("last:"),
            StatusPart::Tinted("open agent-ui", row),
        ];
        let mem = [
            StatusPart::Text("server"),
            StatusPart::Value("612 MB", row, 0.0),
            StatusPart::Text("59.9 fps"),
        ];
        let left = [StatusZone::new(&last)];
        let right = [StatusZone::new(&mem)];
        let bar = ZonedStatusBar::new(&[], &left, &right);
        let mut list = DrawList::new();
        bar.draw(
            Rect::new(0.0, 0.0, 900.0, 26.0),
            &mut list,
            &s,
            &InputState::default(),
        );
        let texts = list.texts.clone();
        for pair in texts.windows(2) {
            let (w, _) = list.measure_block(&pair[0]);
            assert!(
                pair[0].x + w + PART_GAP - 0.5 <= pair[1].x,
                "{:?} runs into {:?}",
                pair[0].content,
                pair[1].content
            );
        }
    }
}
