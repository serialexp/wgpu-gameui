//! Search field — a round well holding a magnifier, the text and a clear key
//! (Forge `SearchField`). The round shape (`--radius-search`) is what tells
//! the user a field filters rather than stores a value.
//!
//! Editing (caret, selection, clipboard, scrolling) is the caller's retained
//! [`TextInput`], drawn inside the round well without its own; the placeholder
//! is the input's. Once there is text, a small round ghost key at the right
//! empties it.
//!
//! The design's scopes (a latching key inside the well that picks what the
//! search looks in) are not built yet.
//!
//! Gated behind the `phosphor-icons` feature.
//!
//! # Example
//! ```ignore
//! let rect = Rect::new(x, y, width, SearchField::HEIGHT);
//! if SearchField::new().draw(&mut search, SEARCH_ID, rect, &mut ctx) {
//!     refilter(&search.value);
//! }
//! ```

use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::style::{Ink, StyleKey};

use super::material::{self, Tone};
use super::{DrawContext, FocusId, Icon, IconKey, TextInput};

/// Space between the well's left edge and the magnifier.
const PAD_LEFT: f32 = 10.0;
/// Space between the clear key and the well's right edge.
const PAD_RIGHT: f32 = 4.0;
/// Space between the magnifier, the text and the clear key.
const GAP: f32 = 6.0;
/// Side of the magnifier's square.
const GLYPH: f32 = 10.0;
/// Face size of the clear key.
const CLEAR_KEY: f32 = 16.0;
/// The clear key's travel (the design's small inline keys travel 1 px).
const CLEAR_TRAVEL: f32 = 1.0;

/// A round search well around a [`TextInput`]. See the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct SearchField;

/// Where the parts of a search field sit inside its rect.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Parts {
    /// The magnifier's square.
    glyph: Rect,
    /// The text input's rect: the well's left edge up to the gap before the
    /// clear key, so a click on the magnifier focuses the field.
    input: Rect,
    /// The text's left inset inside `input`.
    text_inset: f32,
    /// The clear key's rect (face plus travel).
    clear: Rect,
}

impl SearchField {
    /// The field's height: the 18 px row the clear key sits in, 4 px of
    /// padding above and below it, and the 1 px border.
    pub const HEIGHT: f32 = 28.0;

    /// A search field.
    pub fn new() -> Self {
        Self
    }

    fn parts(rect: Rect, border: f32) -> Parts {
        let mid = rect.y + rect.height * 0.5;
        let glyph = Rect::new(rect.x + border + PAD_LEFT, mid - GLYPH * 0.5, GLYPH, GLYPH);
        let clear_h = CLEAR_KEY + CLEAR_TRAVEL;
        let clear = Rect::new(
            rect.x + rect.width - border - PAD_RIGHT - CLEAR_KEY,
            // Whole pixels, so the round key's edge stays crisp.
            (mid - clear_h * 0.5).floor(),
            CLEAR_KEY,
            clear_h,
        );
        let text_left = glyph.x + GLYPH + GAP;
        let input_right = (clear.x - GAP).max(text_left);
        Parts {
            glyph,
            input: Rect::new(rect.x, rect.y, input_right - rect.x, rect.height),
            text_inset: text_left - rect.x,
            clear,
        }
    }

    /// Draw the field into `rect` (normally [`HEIGHT`](Self::HEIGHT) tall),
    /// editing `input` under focus id `id`. Returns whether the text changed
    /// this frame, by typing or by the clear key.
    pub fn draw(
        &self,
        input: &mut TextInput,
        id: FocusId,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> bool {
        ctx.push_debug_scope_rect("SearchField", rect);
        let s = ctx.styles();
        let parts = Self::parts(rect, s.scalar(StyleKey::BorderWidth));

        // A click on the text area focuses the field in this frame's
        // `TextInput::draw`, which runs after the well is painted; count it
        // now so the well lights up in the same frame as the caret.
        let i = parts.input;
        let focusing = ctx.input.is_hovered(i.x, i.y, i.width, i.height) && ctx.input.mouse_clicked;
        let focused = ctx.focus.is_focused(id) || focusing;
        material::draw_well_rounded(ctx.draw_list, &s, rect, rect.height * 0.5, focused, false);
        Icon::new(PhosphorIcon::MagnifyingGlass)
            .tint(s.ink(Ink::Muted))
            .draw(parts.glyph, ctx.draw_list);

        input.x = i.x;
        input.y = i.y;
        input.width = i.width;
        input.height = i.height;
        input.well = false;
        input.insets = Some([parts.text_inset, 0.0]);
        let before = input.value.clone();
        input.draw(id, ctx);
        let mut changed = input.value != before;

        if !input.value.is_empty()
            && IconKey::new(PhosphorIcon::X, CLEAR_KEY)
                .tone(Tone::Ghost)
                .travel(CLEAR_TRAVEL)
                .radius(CLEAR_KEY * 0.5)
                .draw(parts.clear, ctx)
                .clicked
        {
            input.value.clear();
            input.cursor_pos = 0;
            input.selection_start = None;
            input.horizontal_scroll_offset = 0.0;
            changed = true;
        }
        ctx.pop_debug_scope();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    const RECT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: SearchField::HEIGHT,
    };
    const ID: FocusId = 0x5EA2;

    fn input_at(x: f32, y: f32, clicked: bool) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: clicked,
            mouse_clicked: clicked,
            ..Default::default()
        }
    }

    /// Draw one frame of `field` under `input` with `focus`; returns the list
    /// and whether the text changed.
    fn frame(
        field: &mut TextInput,
        focus: &mut FocusState,
        input: &InputState,
    ) -> (DrawList, bool) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
        let changed = SearchField::new().draw(field, ID, RECT, &mut ctx);
        (list, changed)
    }

    #[test]
    fn the_parts_follow_the_design_insets() {
        let p = SearchField::parts(RECT, 1.0);
        assert_eq!(p.glyph, Rect::new(11.0, 9.0, 10.0, 10.0));
        assert_eq!(p.text_inset, 27.0);
        assert_eq!(p.clear, Rect::new(179.0, 5.0, 16.0, 17.0));
        assert_eq!(p.input, Rect::new(0.0, 0.0, 173.0, SearchField::HEIGHT));
    }

    #[test]
    fn an_empty_field_shows_the_magnifier_and_no_clear_key() {
        let mut field = TextInput::default().with_placeholder("Filter");
        let (list, changed) = frame(
            &mut field,
            &mut FocusState::new(),
            &input_at(500.0, 500.0, false),
        );
        assert!(!changed);
        assert_eq!(list.icons_msdf.len(), 1, "only the magnifier");
        assert!(!field.well, "the round well replaces the input's own");
        assert_eq!(field.insets, Some([27.0, 0.0]));
    }

    #[test]
    fn typing_reports_a_change_and_brings_the_clear_key() {
        let mut field = TextInput::default();
        let mut focus = FocusState::new();
        focus.request(ID);
        let typing = InputState {
            text_input: "rs".into(),
            ..input_at(500.0, 500.0, false)
        };
        let (_, changed) = frame(&mut field, &mut focus, &typing);
        assert!(changed);
        assert_eq!(field.value, "rs");
        let (list, changed) = frame(&mut field, &mut focus, &input_at(500.0, 500.0, false));
        assert!(!changed);
        assert_eq!(list.icons_msdf.len(), 2, "magnifier and clear key");
    }

    #[test]
    fn the_clear_key_empties_the_field() {
        let mut field = TextInput::default().with_value("agent");
        let c = SearchField::parts(RECT, 1.0).clear;
        let (_, changed) = frame(
            &mut field,
            &mut FocusState::new(),
            &input_at(c.x + c.width * 0.5, c.y + c.height * 0.5, true),
        );
        assert!(changed);
        assert_eq!(field.value, "");
        assert_eq!(field.cursor_pos, 0);
    }

    #[test]
    fn clicking_the_text_focuses_the_field() {
        let mut field = TextInput::default();
        let mut focus = FocusState::new();
        frame(&mut field, &mut focus, &input_at(60.0, 14.0, true));
        assert!(focus.is_focused(ID));
    }
}
