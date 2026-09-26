//! Pressable — the key every clickable key is built on (Forge `Key`).
//!
//! A `Pressable` owns everything about being pressed: hover, press, click and
//! keyboard activation, focus, and the key material (a face on a plinth that
//! drops `travel` px while pressed). It knows nothing about what its face
//! shows. The caller draws that in a closure, which gets the face rect and
//! the key's [`PressState`]. [`Button`](super::Button) is a pressable with a
//! label and [`IconKey`](super::IconKey) one with an icon; an image button is
//! a pressable with an [`Image`](super::Image) in it:
//!
//! ```ignore
//! let clicked = Pressable::new()
//!     .tone(Tone::Ghost)
//!     .draw(rect, &mut ctx, |key, ctx| {
//!         Image::key("play.png")
//!             .fit(ImageFit::Contain)
//!             .draw(key.face.inset(4.0), ctx.draw_list);
//!     })
//!     .clicked;
//! ```
//!
//! Everything the closure draws fades with the key while it is disabled. The
//! fade goes through the draw list's tint, so it reaches text, icons and
//! sprites looked up by name alike. A [`bare`](Pressable::bare) pressable has
//! no material; its hover and press feedback is a translucent wash laid over
//! the content.

use crate::layout::Rect;
use crate::{CursorIcon, Response, StyleKey, StyleResolver, WidgetId};

use super::material::{self, DISABLED_ALPHA, Material, Tone};
use super::{DrawContext, DrawList, FocusId};

/// What a [`Pressable`]'s content closure needs to know about the key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressState {
    /// Where the content goes: the key's face, which drops `travel` px while
    /// the key is down. A bare pressable's face is its whole rect.
    pub face: Rect,
    /// The pointer is over the key (never while disabled).
    pub hovered: bool,
    /// The pointer is holding the key down (never while disabled).
    pub pressed: bool,
    /// The key looks down: pressed, or [`held`](Pressable::held), and enabled.
    pub down: bool,
    /// The key takes input. Content does not need to fade itself when this is
    /// false; the pressable already does.
    pub enabled: bool,
    /// The material tone the face wears, for picking a content color.
    pub tone: Tone,
}

/// A clickable key with caller-drawn content. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Pressable {
    enabled: bool,
    /// Draw no material; feedback is a wash over the content.
    bare: bool,
    /// Corner radius override (`None` = `StyleKey::BorderRadius`).
    radius: Option<f32>,
    tone: Tone,
    /// Joins the Tab ring under this id and activates on Space/Enter.
    focus_id: Option<FocusId>,
    /// Travel override (`None` = `StyleKey::Travel`).
    travel: Option<f32>,
    /// Wear the pressed face while not pressed (a latched key).
    held: bool,
    /// No plinth under the face.
    hollow: bool,
    /// Name of the key's debug scope (`None` = "Pressable").
    name: Option<String>,
}

impl Default for Pressable {
    fn default() -> Self {
        Self::new()
    }
}

impl Pressable {
    /// An enabled, default-tone key.
    pub fn new() -> Self {
        Self {
            enabled: true,
            bare: false,
            radius: None,
            tone: Tone::default(),
            focus_id: None,
            travel: None,
            held: false,
            hollow: false,
            name: None,
        }
    }

    /// Paint the face in `tone`: `Accent` (primary), `Danger`
    /// (destructive), `Ghost` (a transparent face on the plinth), `Sunken`,
    /// or the default raised neutral.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Leave out the plinth under the face (the design's `hollow` keys, such
    /// as steppers and keys sunk in a well). A hollow ghost key only shows a
    /// face while hovered or pressed.
    pub fn hollow(mut self, hollow: bool) -> Self {
        self.hollow = hollow;
        self
    }

    /// Override the corner radius (default `StyleKey::BorderRadius`); `0.0`
    /// for square corners.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// Override how far the face drops while pressed (default
    /// `StyleKey::Travel`).
    pub fn travel(mut self, travel: f32) -> Self {
        self.travel = Some(travel);
        self
    }

    /// Keep the key down: it wears the pressed face without being pressed,
    /// as a latched toggle does. Clicks still report normally; a disabled key
    /// never looks held.
    pub fn held(mut self, held: bool) -> Self {
        self.held = held;
        self
    }

    /// Make the key keyboard-focusable under `id`: it joins the Tab ring,
    /// draws a focus ring while focused, and activates on Space/Enter as well
    /// as clicks. Clicking it also moves focus to it. With a retained
    /// interaction scene, `id` also identifies its hit region.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// Enable or disable the key. A disabled key fades (material and
    /// content) and never reports hover or clicks.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draw no material, only the content, with a translucent wash over it
    /// on hover and press.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    /// Name the key's debug scope (default "Pressable"), so debug reports
    /// say which key a problem is in.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// How far the face drops while pressed under `s`: the override, or
    /// `StyleKey::Travel`. A key's rect is its face plus this.
    pub fn travel_px(&self, s: &StyleResolver) -> f32 {
        self.travel.unwrap_or_else(|| s.scalar(StyleKey::Travel))
    }

    /// Draw the key in `rect`, calling `content` to draw its face's content,
    /// and return its interaction response. `clicked` includes keyboard
    /// activation while focused.
    pub fn draw(
        &self,
        rect: Rect,
        ctx: &mut DrawContext,
        content: impl FnOnce(&PressState, &mut DrawContext),
    ) -> Response {
        let response_id = WidgetId(self.focus_id.unwrap_or(0));
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Response::idle(response_id, rect);
        }
        ctx.push_debug_scope_rect(self.name.as_deref().unwrap_or("Pressable"), rect);

        // With a retained scene, a focusable key's id identifies its hit
        // region and the previous frame's topmost winner decides; otherwise
        // (and on the first frame) it is an immediate rect test.
        let retained = self
            .focus_id
            .filter(|_| ctx.has_interactions())
            .map(|id| ctx.interact(WidgetId(id), rect, self.enabled))
            .filter(|response| response.resolved);
        let input = ctx.input;
        let hovered = retained.as_ref().map_or_else(
            || self.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y),
            |response| response.hovered,
        );
        if hovered {
            ctx.request_cursor(CursorIcon::Pointer);
        }
        let pressed = retained
            .as_ref()
            .map_or(hovered && input.mouse_down, |r| r.pressed);
        let clicked = retained
            .as_ref()
            .map_or(hovered && input.mouse_clicked, |r| r.clicked);
        let down = self.enabled && (pressed || self.held);

        let s = ctx.styles();
        // A bare key has no plinth to drop into: its face is its rect.
        let face = if self.bare {
            rect
        } else {
            let m = Material::new(self.tone)
                .enabled(self.enabled)
                .hovered(hovered)
                .pressed(pressed || self.held)
                .travel(self.travel_px(&s))
                .hollow(self.hollow);
            let radius = self
                .radius
                .unwrap_or_else(|| s.scalar(StyleKey::BorderRadius));
            material::draw_with_radius(ctx.draw_list, &s, rect, radius, &m)
        };

        let state = PressState {
            face,
            hovered,
            pressed,
            down,
            enabled: self.enabled,
            tone: self.tone,
        };
        if self.enabled {
            content(&state, ctx);
        } else {
            ctx.draw_list.push_tint();
            ctx.draw_list.multiply_tint([1.0, 1.0, 1.0, DISABLED_ALPHA]);
            content(&state, ctx);
            ctx.draw_list.pop_tint();
        }
        if self.bare {
            draw_bare_wash(ctx.draw_list, rect, hovered, pressed);
        }

        // Keyboard focus and Space/Enter activation (opt-in via `focusable`).
        let mut activated = clicked;
        if let Some(id) = self.focus_id {
            ctx.register_focus(id);
            if clicked {
                ctx.focus.request(id);
            }
            if ctx.focus.is_focused(id) {
                if self.enabled && input.nav.confirm {
                    activated = true;
                }
                ctx.draw_focus_ring(rect);
            }
        }

        ctx.pop_debug_scope();
        let mut response = retained.unwrap_or_else(|| Response {
            id: self.focus_id.map(WidgetId),
            rect,
            resolved: false,
            hovered,
            pressed,
            clicked,
            released: hovered && input.mouse_released,
            held: hovered && input.mouse_held,
            double_clicked: hovered && input.mouse_double_clicked,
            local_pos: hovered.then_some([input.mouse_x - rect.x, input.mouse_y - rect.y]),
            scroll_delta: 0.0,
        });
        response.clicked = activated;
        response
    }
}

/// A bare pressable's feedback over its content: a darken while pressed, a
/// faint lighten while hovered.
fn draw_bare_wash(list: &mut DrawList, rect: Rect, hovered: bool, pressed: bool) {
    let wash = if pressed {
        [0.0, 0.0, 0.0, 0.2]
    } else if hovered {
        [1.0, 1.0, 1.0, 0.08]
    } else {
        return;
    };
    list.quad(rect.x, rect.y, rect.width, rect.height, wash);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::TextBlock;
    use crate::{FocusState, InputState, Theme};

    const RECT: Rect = Rect {
        x: 10.0,
        y: 10.0,
        width: 40.0,
        height: 26.0,
    };

    fn input_at(x: f32, y: f32, down: bool, clicked: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: down,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    fn inside(down: bool, clicked: bool) -> InputState {
        input_at(30.0, 20.0, down, clicked)
    }

    /// Draw `key` at [`RECT`] under `input` with content that records the
    /// state it was given and writes one white text block on the face.
    /// Returns the list, the recorded state (if content ran) and the response.
    fn draw(
        key: &Pressable,
        input: &InputState,
        focus: &mut FocusState,
    ) -> (DrawList, Option<PressState>, Response) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut seen = None;
        let response = {
            let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
            key.draw(RECT, &mut ctx, |state, ctx| {
                seen = Some(*state);
                ctx.draw_list.text(
                    TextBlock::new("x", state.face.x, state.face.y)
                        .with_color_f32([1.0, 1.0, 1.0, 1.0]),
                );
            })
        };
        (list, seen, response)
    }

    fn travel() -> f32 {
        StyleResolver::new(&Theme::default()).scalar(StyleKey::Travel)
    }

    #[test]
    fn the_face_rests_on_the_plinth_and_drops_while_pressed() {
        let t = travel();
        let (_, idle, _) = draw(
            &Pressable::new(),
            &input_at(0.0, 0.0, false, false),
            &mut FocusState::new(),
        );
        let idle = idle.unwrap();
        assert_eq!(
            idle.face,
            Rect::new(RECT.x, RECT.y, RECT.width, RECT.height - t)
        );
        assert!(!idle.down && !idle.hovered);

        let (_, pressed, _) = draw(
            &Pressable::new(),
            &inside(true, false),
            &mut FocusState::new(),
        );
        let pressed = pressed.unwrap();
        assert_eq!(
            pressed.face,
            Rect::new(RECT.x, RECT.y + t, RECT.width, RECT.height - t)
        );
        assert!(pressed.hovered && pressed.pressed && pressed.down);

        let (_, custom, _) = draw(
            &Pressable::new().travel(1.0),
            &inside(true, false),
            &mut FocusState::new(),
        );
        assert_eq!(
            custom.unwrap().face.y,
            RECT.y + 1.0,
            "a travel override moves the face"
        );
    }

    #[test]
    fn a_held_key_looks_down_without_clicking() {
        let (_, state, response) = draw(
            &Pressable::new().held(true),
            &input_at(0.0, 0.0, false, false),
            &mut FocusState::new(),
        );
        let state = state.unwrap();
        assert!(state.down && !state.pressed);
        assert_eq!(state.face.y, RECT.y + travel());
        assert!(!response.clicked);

        let (_, disabled, _) = draw(
            &Pressable::new().held(true).enabled(false),
            &input_at(0.0, 0.0, false, false),
            &mut FocusState::new(),
        );
        assert!(!disabled.unwrap().down, "a disabled key never looks held");
    }

    #[test]
    fn clicks_report_only_inside_and_while_enabled() {
        let (_, _, hit) = draw(
            &Pressable::new(),
            &inside(true, true),
            &mut FocusState::new(),
        );
        assert!(hit.clicked && hit.hovered);
        let (_, _, miss) = draw(
            &Pressable::new(),
            &input_at(500.0, 500.0, true, true),
            &mut FocusState::new(),
        );
        assert!(!miss.clicked);
        let (_, state, off) = draw(
            &Pressable::new().enabled(false),
            &inside(true, true),
            &mut FocusState::new(),
        );
        assert!(!off.clicked && !off.hovered);
        assert!(!state.unwrap().hovered);
        let consumed = InputState {
            mouse_consumed: true,
            ..inside(true, true)
        };
        let (_, _, below) = draw(&Pressable::new(), &consumed, &mut FocusState::new());
        assert!(
            !below.clicked,
            "a consumed pointer belongs to a higher layer"
        );
    }

    #[test]
    fn a_disabled_key_fades_its_content_and_restores_the_tint() {
        let (list, state, _) = draw(
            &Pressable::new().enabled(false),
            &inside(false, false),
            &mut FocusState::new(),
        );
        assert!(!state.unwrap().enabled);
        let text = &list.texts[0];
        let expected = TextBlock::new("", 0.0, 0.0)
            .with_color_f32([1.0, 1.0, 1.0, DISABLED_ALPHA])
            .color;
        assert_eq!(
            text.color, expected,
            "content fades by the material's alpha"
        );
        assert_eq!(
            list.current_tint(),
            [1.0; 4],
            "the tint is popped after the content"
        );

        let (list, _, _) = draw(
            &Pressable::new(),
            &inside(false, false),
            &mut FocusState::new(),
        );
        assert_eq!(
            list.texts[0].color,
            TextBlock::new("", 0.0, 0.0).with_color_f32([1.0; 4]).color
        );
    }

    #[test]
    fn a_bare_key_draws_no_material_and_washes_over_its_content() {
        let (idle, state, _) = draw(
            &Pressable::new().bare(),
            &input_at(0.0, 0.0, false, false),
            &mut FocusState::new(),
        );
        assert_eq!(state.unwrap().face, RECT, "no plinth: the face is the rect");
        assert_eq!(idle.chrome_instance_count(), 0, "no material at rest");

        let (hovered, _, _) = draw(
            &Pressable::new().bare(),
            &inside(false, false),
            &mut FocusState::new(),
        );
        let (pressed, state, _) = draw(
            &Pressable::new().bare(),
            &inside(true, false),
            &mut FocusState::new(),
        );
        assert_eq!(state.unwrap().face, RECT, "a bare face does not drop");
        let wash = |list: &DrawList| list.chrome_instance(0).map(|c| c.bg);
        assert_eq!(wash(&hovered), Some([1.0, 1.0, 1.0, 0.08]));
        assert_eq!(wash(&pressed), Some([0.0, 0.0, 0.0, 0.2]));
    }

    #[test]
    fn a_zero_sized_key_draws_nothing_and_never_calls_its_content() {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = inside(true, true);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        let mut called = false;
        let response = Pressable::new().draw(Rect::new(0.0, 0.0, 0.0, 20.0), &mut ctx, |_, _| {
            called = true
        });
        assert!(!called && !response.clicked);
    }

    #[test]
    fn a_focused_key_activates_from_the_keyboard() {
        let mut keys = InputState {
            key_space: true,
            ..Default::default()
        };
        crate::map_keyboard(&mut keys);
        let key = Pressable::new().focusable(7);
        let (_, _, unfocused) = draw(&key, &keys, &mut FocusState::new());
        assert!(!unfocused.clicked);
        let mut focus = FocusState::new();
        focus.focus(7);
        let (_, _, focused) = draw(&key, &keys, &mut focus);
        assert!(focused.clicked);
        let (_, _, disabled) = draw(&key.clone().enabled(false), &keys, &mut focus);
        assert!(!disabled.clicked, "a disabled key ignores the keyboard");
    }
}
