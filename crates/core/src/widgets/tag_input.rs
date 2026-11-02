//! Tag input — the design's chip-list text field (Forge `TagInput`): a
//! well holding pill tags, each with a × that removes it, and the text being
//! typed after the last of them.
//!
//! The tags are the caller's; the typing is a [`TextInput`] the caller keeps
//! (as [`UiContext::tag_input`](crate::UiContext::tag_input) does, by focus
//! id), drawn inside the well without its own. Enter adds what was typed,
//! Backspace in an empty field removes the last tag, and text left typed
//! when the field lets go of the keyboard is added too, so it is never lost
//! to a click elsewhere. The tags wrap onto as many lines as they need:
//! [`TagInput::height`] says how tall the well is for a width.
//!
//! # Example
//! ```ignore
//! let tags = TagInput::new(&exts).placeholder(".lua  ↵");
//! let rect = Rect::new(x, y, w, tags.height(w, list, &styles));
//! let out = tags.draw(&mut typing, FIELD_ID, rect, ctx);
//! if let Some(index) = out.removed { exts.remove(index); }
//! if let Some(tag) = out.added { exts.push(tag); }
//! ```

use crate::DrawList;
use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::material;
use super::{DrawContext, FocusId, TextInput};

/// Space inside the well around the tags.
const PAD_X: f32 = 5.0;
const PAD_Y: f32 = 3.0;
/// Space between tags, across and down.
const GAP: f32 = 4.0;
/// A tag's padding before its text, and the × after it.
const CHIP_PAD_LEFT: f32 = 7.0;
const CHIP_X: f32 = 13.0;
const CHIP_PAD_RIGHT: f32 = 4.0;
/// The least room the typing gets on the last line before it wraps.
const DRAFT_MIN: f32 = 60.0;

/// What a tag input did this frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagOutput {
    /// The tag whose × was clicked, or the last one, removed by Backspace in
    /// an empty field: the caller removes it.
    pub removed: Option<usize>,
    /// What was typed, trimmed, added by Enter or by the field letting go of
    /// the keyboard: the caller adds it. The typing is cleared.
    pub added: Option<String>,
}

/// A tag input over `tags` (see the [module docs](self)).
#[derive(Debug, Clone, Copy)]
pub struct TagInput<'a> {
    tags: &'a [String],
    placeholder: &'a str,
    invalid: bool,
}

/// Where a tag input's parts sit inside its rect.
#[derive(Debug, Clone, PartialEq)]
struct Layout {
    chips: Vec<Rect>,
    /// The typing's line, after the last chip or on a line of its own.
    draft: Rect,
    /// The well's height.
    height: f32,
}

impl<'a> TagInput<'a> {
    /// A tag input over `tags`, with the default placeholder.
    pub fn new(tags: &'a [String]) -> Self {
        Self {
            tags,
            placeholder: "Add tag\u{2026}",
            invalid: false,
        }
    }

    /// Shown while nothing is typed.
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Paint the well's error border and ring: what it holds was refused.
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// How tall the well is when `width` wide: one line per row of tags,
    /// and at least a text field's height.
    pub fn height(&self, width: f32, list: &mut DrawList, s: &StyleResolver) -> f32 {
        self.layout(Rect::new(0.0, 0.0, width, 0.0), list, s).height
    }

    fn line(s: &StyleResolver) -> f32 {
        // A chip holds its text with a pixel's border above and below.
        (chip_size(s) * 1.25 + 4.0).ceil()
    }

    fn layout(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) -> Layout {
        let line = Self::line(s);
        let size = chip_size(s);
        let left = rect.x + PAD_X;
        let right = (rect.right() - PAD_X).max(left + 1.0);
        let field = s.theme().input_height;
        // One line of chips centred in a field's height, as a text field
        // holds its text.
        let top = rect.y + ((field - line) / 2.0).max(PAD_Y);
        let (mut x, mut y) = (left, top);
        let mut chips = Vec::with_capacity(self.tags.len());
        let font = s.theme().font.clone();
        for tag in self.tags {
            // Measured in the font the chip is drawn in, and rounded up, so
            // the text fits the room the chip leaves it and isn't cut short.
            let block = TextBlock::new(tag.as_str(), 0.0, 0.0)
                .with_size(size)
                .with_font_opt(font.clone());
            let text = list.measure_block(&block).0.ceil();
            let width = (CHIP_PAD_LEFT + text + 4.0 + CHIP_X + CHIP_PAD_RIGHT).min(right - left);
            if x > left && x + width > right {
                x = left;
                y += line + GAP;
            }
            chips.push(Rect::new(x, y, width, line));
            x += width + GAP;
        }
        if x > left && x + DRAFT_MIN > right {
            x = left;
            y += line + GAP;
        }
        let draft = Rect::new(x, y, (right - x).max(1.0), line);
        let height = (draft.bottom() - rect.y + (top - rect.y)).max(field);
        Layout {
            chips,
            draft,
            height,
        }
    }

    /// Draw into `rect` (as tall as [`height`](Self::height) says), typing
    /// into `typing` under focus id `id`.
    pub fn draw(
        &self,
        typing: &mut TextInput,
        id: FocusId,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> TagOutput {
        ctx.push_debug_scope_rect("TagInput", rect);
        let s = ctx.styles();
        let input = ctx.input;
        let mut out = TagOutput::default();
        let layout = self.layout(rect, ctx.draw_list, &s);

        let over_chip = |at: &Rect| input.is_hovered(at.x, at.y, at.width, at.height);
        let hovered = input.is_hovered(rect.x, rect.y, rect.width, rect.height);
        // A click anywhere in the well but a ×, focuses the typing, in time
        // for the well to light up this frame.
        let on_x = layout.chips.iter().any(|chip| over_chip(&x_rect(*chip)));
        if hovered && input.mouse_clicked && !on_x {
            ctx.focus.request(id);
        }
        if hovered && !on_x {
            ctx.request_cursor(crate::CursorIcon::Text);
        }
        let focused = ctx.focus.is_focused(id);
        material::draw_well(ctx.draw_list, &s, rect, focused, self.invalid);

        for (index, (tag, chip)) in self.tags.iter().zip(&layout.chips).enumerate() {
            if draw_chip(tag, *chip, ctx.draw_list, &s, over_chip(&x_rect(*chip)))
                && input.mouse_clicked
            {
                out.removed = Some(index);
            }
        }

        let typed_before = typing.value.is_empty();
        if focused && typed_before && input.backspace_pressed && !self.tags.is_empty() {
            out.removed = Some(self.tags.len() - 1);
        }
        typing.x = layout.draft.x;
        typing.y = layout.draft.y;
        typing.width = layout.draft.width;
        typing.height = layout.draft.height;
        typing.well = false;
        typing.insets = Some([2.0, 2.0]);
        typing.placeholder.clear();
        typing.placeholder.push_str(self.placeholder);
        typing.draw(id, ctx);

        let focused = ctx.focus.is_focused(id);
        let typed = typing.value.trim();
        if !typed.is_empty() && ((focused && input.enter_pressed) || !focused) {
            out.added = Some(typed.to_owned());
            typing.set_value("");
        }
        ctx.pop_debug_scope();
        out
    }
}

/// A tag's text size: the design's 10.5 px beside a 12 px field.
fn chip_size(s: &StyleResolver) -> f32 {
    s.scalar(StyleKey::FontSize) * 0.85
}

/// Where a chip's × is.
fn x_rect(chip: Rect) -> Rect {
    Rect::new(
        chip.right() - CHIP_PAD_RIGHT - CHIP_X,
        chip.y + (chip.height - CHIP_X) / 2.0,
        CHIP_X,
        CHIP_X,
    )
}

/// One tag: a raised pill with its text and a ×. Returns whether the
/// pointer is on the ×.
fn draw_chip(tag: &str, chip: Rect, list: &mut DrawList, s: &StyleResolver, on_x: bool) -> bool {
    let radius = chip.height * 0.5;
    let base = s.color(StyleKey::Button);
    let top = material::sheen_over(base, s.color(StyleKey::FaceTop));
    let bottom = material::sheen_over(base, s.color(StyleKey::FaceBottom));
    // The curved face gradient is the sheen: a straight band across the top
    // would cut the pill's caps as a white slash.
    list.chrome_rect_gradient(chip, radius, 1.0, top, bottom, [0.0, 0.0, 0.0, 0.55]);
    let size = chip_size(s);
    let font = s.theme().font.clone();
    let y = list.vcentered_text_y(chip.y, chip.height, size, font.as_ref(), tag);
    list.text(
        TextBlock::new(tag, chip.x + CHIP_PAD_LEFT, y)
            .with_size(size)
            .with_color_f32(s.color(StyleKey::Text))
            .with_ellipsis()
            .with_max_width((chip.width - CHIP_PAD_LEFT - CHIP_X - CHIP_PAD_RIGHT - 4.0).max(1.0))
            .with_font_opt(font.clone()),
    );
    let x = x_rect(chip);
    if on_x {
        list.rounded_rect(x, CHIP_X / 2.0, [1.0, 1.0, 1.0, 0.09]);
    }
    let glyph = "\u{00d7}";
    let glyph_size = size * 1.05;
    let width = list.measure_text(glyph, glyph_size, None).0;
    let y = list.vcentered_text_y(x.y, x.height, glyph_size, font.as_ref(), glyph);
    list.text(
        TextBlock::new(glyph, x.x + (x.width - width) / 2.0, y)
            .with_size(glyph_size)
            .with_color_f32(s.color(StyleKey::TextDim))
            .with_font_opt(font),
    );
    on_x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    const ID: FocusId = 7;

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// One frame of a tag input `width` wide at the origin.
    fn frame(
        tags: &[String],
        typing: &mut TextInput,
        focus: &mut FocusState,
        input: &InputState,
        width: f32,
    ) -> (TagOutput, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let field = TagInput::new(tags).placeholder(".lua");
        let height = field.height(width, &mut list, &StyleResolver::new(&theme));
        let mut ctx = DrawContext::new(&mut list, focus, &theme, input, 800.0, 600.0);
        let out = field.draw(typing, ID, Rect::new(0.0, 0.0, width, height), &mut ctx);
        (out, list)
    }

    fn typing() -> TextInput {
        TextInput::new(0.0, 0.0, 1.0, 1.0)
    }

    fn layout(tags: &[String], width: f32) -> Layout {
        let theme = Theme::default();
        TagInput::new(tags).layout(
            Rect::new(0.0, 0.0, width, 0.0),
            &mut DrawList::new(),
            &StyleResolver::new(&theme),
        )
    }

    #[test]
    fn an_empty_well_is_a_text_fields_height() {
        let theme = Theme::default();
        let height =
            TagInput::new(&[]).height(200.0, &mut DrawList::new(), &StyleResolver::new(&theme));
        assert_eq!(height, theme.input_height);
    }

    #[test]
    fn tags_wrap_onto_more_lines_and_the_well_grows() {
        let many = tags(&["rs", "toml", "lua", "json", "yaml", "md", "txt", "c"]);
        let wide = layout(&many, 600.0);
        let narrow = layout(&many, 120.0);
        assert!(wide.chips.iter().all(|chip| chip.y == wide.chips[0].y));
        assert!(narrow.chips.iter().any(|chip| chip.y > narrow.chips[0].y));
        assert!(narrow.height > wide.height);
        for chip in &narrow.chips {
            assert!(
                chip.right() <= 120.0 - PAD_X + 0.01,
                "{chip:?} leaves the well"
            );
        }
    }

    #[test]
    fn the_typing_gets_its_room_or_a_line_of_its_own() {
        let one = tags(&["rs"]);
        let roomy = layout(&one, 300.0);
        assert_eq!(roomy.draft.y, roomy.chips[0].y);
        assert!(roomy.draft.x > roomy.chips[0].right());
        assert!(roomy.draft.width >= DRAFT_MIN);

        let chip = roomy.chips[0].width;
        let tight = layout(&one, PAD_X * 2.0 + chip + GAP + DRAFT_MIN - 1.0);
        assert!(tight.draft.y > tight.chips[0].y);
        assert_eq!(tight.draft.x, PAD_X);
    }

    #[test]
    fn clicking_a_tags_x_removes_it_and_leaves_the_typing_alone() {
        let list = tags(&["rs", "toml"]);
        let x = x_rect(layout(&list, 300.0).chips[1]);
        let input = InputState {
            mouse_x: x.x + x.width / 2.0,
            mouse_y: x.y + x.height / 2.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut focus = FocusState::new();
        let (out, _) = frame(&list, &mut typing(), &mut focus, &input, 300.0);
        assert_eq!(out.removed, Some(1));
        assert!(!focus.is_focused(ID));
    }

    #[test]
    fn clicking_the_well_focuses_the_typing() {
        let list = tags(&["rs"]);
        let chip = layout(&list, 300.0).chips[0];
        let input = InputState {
            mouse_x: chip.x + 3.0,
            mouse_y: chip.y + chip.height / 2.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut focus = FocusState::new();
        let (out, _) = frame(&list, &mut typing(), &mut focus, &input, 300.0);
        assert_eq!(out, TagOutput::default());
        assert!(focus.is_focused(ID));
    }

    #[test]
    fn enter_adds_the_trimmed_typing() {
        let mut focus = FocusState::new();
        focus.request(ID);
        let mut field = typing();
        field.set_value("  lua ");
        let input = InputState {
            enter_pressed: true,
            ..Default::default()
        };
        let (out, _) = frame(&[], &mut field, &mut focus, &input, 300.0);
        assert_eq!(out.added.as_deref(), Some("lua"));
        assert_eq!(field.value, "");
    }

    #[test]
    fn typing_alone_adds_nothing() {
        let mut focus = FocusState::new();
        focus.request(ID);
        let mut field = typing();
        let input = InputState {
            text_input: "lu".to_string(),
            ..Default::default()
        };
        let (out, _) = frame(&[], &mut field, &mut focus, &input, 300.0);
        assert_eq!(out, TagOutput::default());
        assert_eq!(field.value, "lu");
    }

    #[test]
    fn backspace_in_empty_typing_removes_the_last_tag() {
        let list = tags(&["rs", "toml"]);
        let mut focus = FocusState::new();
        focus.request(ID);
        let input = InputState {
            backspace_pressed: true,
            ..Default::default()
        };
        let (out, _) = frame(&list, &mut typing(), &mut focus, &input, 300.0);
        assert_eq!(out.removed, Some(1));

        let mut field = typing();
        field.set_value("lu");
        field.cursor_pos = 2;
        let (out, _) = frame(&list, &mut field, &mut focus, &input, 300.0);
        assert_eq!(out.removed, None, "Backspace edits the typing first");
        assert_eq!(field.value, "l");
    }

    #[test]
    fn typing_left_behind_is_added_when_the_field_lets_go() {
        let mut focus = FocusState::new();
        let mut field = typing();
        field.set_value("lua");
        let (out, _) = frame(&[], &mut field, &mut focus, &InputState::default(), 300.0);
        assert_eq!(out.added.as_deref(), Some("lua"));
        assert_eq!(field.value, "");
    }

    #[test]
    fn an_unfocused_field_takes_no_keys_and_shows_its_placeholder() {
        let mut focus = FocusState::new();
        let mut field = typing();
        let input = InputState {
            text_input: "x".to_string(),
            ..Default::default()
        };
        let (out, list) = frame(&[], &mut field, &mut focus, &input, 300.0);
        assert_eq!(out, TagOutput::default());
        assert_eq!(field.value, "");
        assert!(list.texts.iter().any(|text| text.content == ".lua"));
    }

    #[test]
    fn tag_chips_use_gradient_sheen_without_a_straight_top_highlight() {
        let list = tags(&["occlusion"]);
        let (_, drawn) = frame(
            &list,
            &mut typing(),
            &mut FocusState::new(),
            &InputState::default(),
            200.0,
        );
        let chip = layout(&list, 200.0).chips[0];
        assert!(
            !drawn.chrome_instances().any(|instance| {
                instance.rect[1] == chip.y + 1.0
                    && instance.rect[3] == 1.0
                    && instance.rect[2] > 10.0
            }),
            "tag chip must not paint a straight top highlight over its curved pill face"
        );
    }

    #[test]
    fn the_x_is_centred_in_its_hit_area() {
        let list = tags(&["occlusion"]);
        let (_, mut drawn) = frame(
            &list,
            &mut typing(),
            &mut FocusState::new(),
            &InputState::default(),
            200.0,
        );
        let cross_y = drawn
            .texts
            .iter()
            .find(|text| text.content == "\u{00d7}")
            .expect("tag chip draws its ×")
            .y;
        let x = x_rect(layout(&list, 200.0).chips[0]);
        let size = Theme::default().font_size * 0.85 * 1.05;
        assert_eq!(
            cross_y,
            drawn.vcentered_text_y(x.y, x.height, size, None, "\u{00d7}")
        );
    }
}
