//! Icon key — a square key holding one Phosphor icon (Forge `IconKey`).
//!
//! The design uses three sizes: [`IconKey::TOOLBAR`] (24), [`IconKey::STATUS`]
//! (18, status-bar toggles) and [`IconKey::HEADER`] (17, dock and section
//! headers, inline keys). The size is the face; the key's rect is one
//! [`StyleKey::Travel`] taller, because the face rests on its plinth and drops
//! by `travel` while pressed or [`held`](IconKey::held) — the icon rides down
//! with it.
//!
//! Interaction, focus and the key material come from [`Button`]; this widget
//! adds the icon and the design's per-tone icon inks.
//!
//! Gated behind the `phosphor-icons` feature.
//!
//! # Example
//! ```ignore
//! let [w, h] = IconKey::new(PhosphorIcon::Plus, IconKey::HEADER).outer_size(&styles);
//! if IconKey::new(PhosphorIcon::Plus, IconKey::HEADER)
//!     .tone(Tone::Ghost)
//!     .draw(Rect::new(x, y, w, h), &mut ctx)
//!     .clicked
//! {
//!     new_session();
//! }
//! ```

use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::style::{Ink, StyleKey, StyleResolver};

use super::material::Tone;
use super::{Button, DrawContext, FocusId, Icon};

/// Disabled keys fade to this fraction (the design's `opacity: 0.45`).
const DISABLED_ALPHA: f32 = 0.45;

/// A square key with a vector icon. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct IconKey {
    icon: PhosphorIcon,
    size: f32,
    tone: Tone,
    held: bool,
    enabled: bool,
    focus_id: Option<FocusId>,
    travel: Option<f32>,
    radius: Option<f32>,
    hollow: bool,
}

impl IconKey {
    /// Toolbar key face size.
    pub const TOOLBAR: f32 = 24.0;
    /// Status-bar toggle face size.
    pub const STATUS: f32 = 18.0;
    /// Dock / section header and inline key face size.
    pub const HEADER: f32 = 17.0;

    /// A default-tone key showing `icon` on a `size` px square face.
    pub fn new(icon: PhosphorIcon, size: f32) -> Self {
        Self {
            icon,
            size,
            tone: Tone::Default,
            held: false,
            enabled: true,
            focus_id: None,
            travel: None,
            radius: None,
            hollow: false,
        }
    }

    /// Wear another material [`Tone`] (ghost keys sit in headers and fields).
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Leave out the plinth under the face; see [`Button::hollow`].
    pub fn hollow(mut self, hollow: bool) -> Self {
        self.hollow = hollow;
        self
    }

    /// Keep the key down (a latched toggle); see [`Button::held`].
    pub fn held(mut self, held: bool) -> Self {
        self.held = held;
        self
    }

    /// Enable or disable the key (disabled keys fade and never click).
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Join the Tab ring under `id`; Space/Enter activate the key while focused.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// Travel `travel` px instead of the theme's [`StyleKey::Travel`] (the
    /// design's small inline keys, such as a search field's clear key, travel
    /// 1).
    pub fn travel(mut self, travel: f32) -> Self {
        self.travel = Some(travel);
        self
    }

    /// Round the key's corners by `radius` px instead of the theme's
    /// [`StyleKey::BorderRadius`] (half the size gives a round key).
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// How far the face drops when pressed.
    fn travel_px(&self, s: &StyleResolver) -> f32 {
        self.travel.unwrap_or_else(|| s.scalar(StyleKey::Travel))
    }

    /// The rect size the key needs: its face plus the plinth travel beneath.
    pub fn outer_size(&self, s: &StyleResolver) -> [f32; 2] {
        [self.size, self.size + self.travel_px(s)]
    }

    /// Side of the square the icon is fitted into, for a face `face` px tall.
    /// Matches the design's glyph sizes at the three key sizes and scales past
    /// them.
    fn icon_box(face: f32) -> f32 {
        if face >= Self::TOOLBAR {
            face * 11.0 / 24.0
        } else if face >= Self::STATUS {
            10.0
        } else {
            face * 9.0 / 17.0
        }
    }

    /// The icon ink for this key's tone and state.
    fn ink(&self, s: &StyleResolver, hovered: bool, pressed: bool) -> [f32; 4] {
        let down = pressed || self.held;
        let mut ink = match self.tone {
            Tone::Accent => s.color(StyleKey::OnAccent),
            Tone::Danger => s.color(StyleKey::OnDanger),
            Tone::Sunken => s.ink(Ink::Muted),
            Tone::Ghost if down => s.ink(Ink::Glyph),
            Tone::Ghost => s.ink(Ink::Cell),
            Tone::Default if self.held => s.ink(Ink::Max),
            Tone::Default if pressed => s.ink(Ink::Second),
            Tone::Default if hovered => s.ink(Ink::Max),
            Tone::Default => s.ink(Ink::Emph),
        };
        if !self.enabled {
            ink[3] *= DISABLED_ALPHA;
        }
        ink
    }

    /// Draw the key into `rect` (normally [`outer_size`](Self::outer_size))
    /// and return its interaction response; `clicked` includes keyboard
    /// activation while focused.
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> crate::Response {
        let mut button = Button::new("")
            .tone(self.tone)
            .held(self.held)
            .enabled(self.enabled)
            .hollow(self.hollow);
        if let Some(id) = self.focus_id {
            button = button.focusable(id);
        }
        if let Some(travel) = self.travel {
            button = button.with_travel(travel);
        }
        if let Some(radius) = self.radius {
            button = button.with_radius(radius);
        }
        let response = button.draw_response(rect, ctx);
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return response;
        }
        let s = ctx.styles();
        let travel = self.travel_px(&s);
        let down = self.enabled && (response.pressed || self.held);
        let face = Rect::new(
            rect.x,
            rect.y + if down { travel } else { 0.0 },
            rect.width,
            (rect.height - travel).max(0.0),
        );
        let side = Self::icon_box(face.height.min(face.width));
        let icon_rect = Rect::new(
            face.x + (face.width - side) * 0.5,
            face.y + (face.height - side) * 0.5,
            side,
            side,
        );
        let hovered = self.enabled && response.hovered;
        Icon::new(self.icon)
            .tint(self.ink(&s, hovered, response.pressed))
            .draw(icon_rect, ctx.draw_list);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn input_at(x: f32, y: f32, down: bool, clicked: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: down,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    /// Draw `key` at the origin in its outer size under `input`; returns the
    /// list and the response.
    fn draw(key: IconKey, input: &InputState) -> (DrawList, crate::Response) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        let [w, h] = key.outer_size(&ctx.styles());
        let response = key.draw(Rect::new(0.0, 0.0, w, h), &mut ctx);
        (list, response)
    }

    fn away() -> InputState {
        input_at(500.0, 500.0, false, false)
    }

    #[test]
    fn outer_size_is_the_face_plus_travel() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let travel = theme.get(StyleKey::Travel).unwrap().as_scalar().unwrap();
        for size in [IconKey::HEADER, IconKey::STATUS, IconKey::TOOLBAR] {
            assert_eq!(
                IconKey::new(PhosphorIcon::Plus, size).outer_size(&s),
                [size, size + travel]
            );
        }
    }

    #[test]
    fn the_icon_sits_centred_on_the_face_at_the_design_size() {
        for (size, side) in [(17.0, 9.0), (18.0, 10.0), (24.0, 11.0)] {
            let (list, _) = draw(IconKey::new(PhosphorIcon::Plus, size), &away());
            assert_eq!(list.icons_msdf.len(), 1);
            let r = list.icons_msdf[0].local;
            assert!((r.width - side).abs() < 1e-4, "{size}: {r:?}");
            assert!(
                (r.x + r.width * 0.5 - size * 0.5).abs() < 1e-4,
                "{size}: {r:?}"
            );
            assert!(
                (r.y + r.height * 0.5 - size * 0.5).abs() < 1e-4,
                "{size}: {r:?}"
            );
        }
    }

    #[test]
    fn the_icon_drops_with_the_face_when_pressed_or_held() {
        let theme = Theme::default();
        let travel = theme.get(StyleKey::Travel).unwrap().as_scalar().unwrap();
        let key = IconKey::new(PhosphorIcon::Plus, 17.0);
        let rest = draw(key, &away()).0.icons_msdf[0].local.y;
        let pressed = draw(key, &input_at(8.0, 8.0, true, false)).0.icons_msdf[0]
            .local
            .y;
        let held = draw(key.held(true), &away()).0.icons_msdf[0].local.y;
        assert_eq!(pressed - rest, travel);
        assert_eq!(held - rest, travel);
    }

    #[test]
    fn travel_and_radius_overrides_reach_the_key() {
        let key = IconKey::new(PhosphorIcon::X, 16.0).travel(1.0).radius(8.0);
        let theme = Theme::default();
        assert_eq!(key.outer_size(&StyleResolver::new(&theme)), [16.0, 17.0]);
        let rest = draw(key, &away()).0.icons_msdf[0].local.y;
        let pressed = draw(key, &input_at(8.0, 8.0, true, false)).0.icons_msdf[0]
            .local
            .y;
        assert_eq!(pressed - rest, 1.0);
    }

    #[test]
    fn clicks_report_and_disabled_keys_ignore_them() {
        let click = input_at(8.0, 8.0, true, true);
        let key = IconKey::new(PhosphorIcon::Plus, 17.0);
        assert!(draw(key, &click).1.clicked);
        assert!(!draw(key.enabled(false), &click).1.clicked);
    }

    #[test]
    fn icon_ink_follows_the_tone_and_state() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let tint = |key: IconKey, input: &InputState| draw(key, input).0.icons_msdf[0].tint;
        let hover = input_at(8.0, 8.0, false, false);
        let press = input_at(8.0, 8.0, true, false);
        let key = IconKey::new(PhosphorIcon::Plus, 17.0);

        assert_eq!(tint(key, &away()), s.ink(Ink::Emph));
        assert_eq!(tint(key, &hover), s.ink(Ink::Max));
        assert_eq!(tint(key, &press), s.ink(Ink::Second));
        assert_eq!(tint(key.held(true), &away()), s.ink(Ink::Max));

        let ghost = key.tone(Tone::Ghost);
        assert_eq!(tint(ghost, &away()), s.ink(Ink::Cell));
        assert_eq!(tint(ghost, &hover), s.ink(Ink::Cell));
        assert_eq!(tint(ghost, &press), s.ink(Ink::Glyph));

        assert_eq!(
            tint(key.tone(Tone::Accent), &away()),
            s.color(StyleKey::OnAccent)
        );
        let mut faded = s.ink(Ink::Emph);
        faded[3] *= DISABLED_ALPHA;
        assert_eq!(tint(key.enabled(false), &hover), faded);
    }
}
