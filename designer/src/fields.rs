//! Small pieces both side panels are built from: a scrolled column, a
//! labelled slider and a text field bound to a value the preset owns.

use std::fmt::Write as _;

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    DragCapture, DragId, DrawContext, FieldLabel, FocusId, FocusState, ScrollState, ScrollView,
    Slider, TextInput,
};

/// The gap between a field's label and the field.
pub const LABEL_GAP: f32 = 4.0;
/// A slider's row height.
pub const SLIDER_HEIGHT: f32 = 18.0;

/// Show `value` in `field` unless the user is typing in it: the preset is
/// the truth, the field only a view of it while unfocused.
pub fn sync_text(field: &mut TextInput, id: FocusId, value: &str, focus: &FocusState) {
    if focus.is_focused(id) || field.value == value {
        return;
    }
    field.value.clear();
    field.value.push_str(value);
    field.cursor_pos = field.value.len();
    field.selection_start = None;
}

/// Draw `field` at `rect` under focus id `id`.
pub fn draw_text(field: &mut TextInput, id: FocusId, rect: Rect, ctx: &mut DrawContext) {
    field.x = rect.x;
    field.y = rect.y;
    field.width = rect.width;
    field.height = rect.height;
    field.draw(id, ctx);
}

/// Lay out and draw a vertical scroll region over `viewport`.
///
/// `body` draws the content from the top of the region and returns its
/// height. It is handed the region's inner rect and the part of the content
/// in view, both in content space (so it can skip what is scrolled away),
/// and a context whose pointer is in content space too.
pub fn scrolled(
    viewport: Rect,
    state: &mut ScrollState,
    ctx: &mut DrawContext,
    body: impl FnOnce(Rect, Rect, &mut DrawContext) -> f32,
) {
    let s = ctx.styles();
    let view = ScrollView::new(viewport).vertical_only().overlay();
    let mut input = ctx.input.clone();
    let begun = view.begin(state, ctx.draw_list, &mut input);

    // The content is drawn under a `-offset` transform; move the pointer into
    // the same space, and away from rows scrolled out from under it.
    let (screen_y, consumed) = (input.mouse_y, input.mouse_consumed);
    input.mouse_consumed |= !begun.inner.contains(input.mouse_x, input.mouse_y);
    input.mouse_y += state.offset[1];
    let inner = begun.inner;
    let visible = Rect::new(
        inner.x,
        inner.y + state.offset[1],
        inner.width,
        inner.height,
    );
    let height = {
        let mut body_ctx = ctx.reborrow_with_input(&input);
        body(inner, visible, &mut body_ctx)
    };
    input.mouse_y = screen_y;
    input.mouse_consumed = consumed;

    state.content_size = [inner.width, height];
    view.end(state, ctx.draw_list, &s, &input, begun);
}

/// A slider under a caption that reads out its value.
pub struct SliderRow<'a> {
    /// The caption.
    pub label: &'a str,
    /// The slider's range.
    pub min: f32,
    pub max: f32,
    /// The step it snaps to.
    pub step: f32,
}

impl SliderRow<'_> {
    /// Draw the row across `width` from `(x, y)`, reading out `value` through
    /// `format` into `scratch`. Returns the new value when it moved, and the
    /// row's bottom edge.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        value: f32,
        format: impl Fn(&mut String, f32),
        scratch: &mut String,
        id: DragId,
        x: f32,
        y: f32,
        width: f32,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> (Option<f32>, f32) {
        let s = ctx.styles();
        scratch.clear();
        format(scratch, value);
        let label = FieldLabel::new(self.label)
            .value(scratch)
            .draw(ctx.draw_list, &s, x, y, width);
        let track = Rect::new(x, label.bottom() + LABEL_GAP, width, SLIDER_HEIGHT);
        let out = Slider::new(self.min, self.max)
            .with_step(self.step)
            .focusable(id)
            .draw(value, id, capture, track, ctx);
        (out.changed.then_some(out.value), track.bottom())
    }
}

/// `×1.25`: a multiplier.
pub fn times(out: &mut String, value: f32) {
    let _ = write!(out, "×{value:.2}");
}

/// `+0.04` / `−0.04`: a signed offset.
pub fn signed(out: &mut String, value: f32) {
    if value < 0.0 {
        let _ = write!(out, "−{:.2}", -value);
    } else {
        let _ = write!(out, "+{value:.2}");
    }
}

/// `134°`: an angle.
pub fn degrees(out: &mut String, value: f32) {
    let _ = write!(out, "{value:.0}°");
}

/// `#rrggbb` or `#rrggbbaa` when not opaque, as a preset file writes it.
pub fn hex(out: &mut String, color: [f32; 4]) {
    let [r, g, b, a] = wgpu_gameui::color::to_rgba8(color);
    let _ = write!(out, "#{r:02x}{g:02x}{b:02x}");
    if a != 255 {
        let _ = write!(out, "{a:02x}");
    }
}

/// Parse `#rrggbb` or `#rrggbbaa` (the `#` optional).
pub fn parse_hex(text: &str) -> Option<[f32; 4]> {
    let digits = text.trim().trim_start_matches('#');
    if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let alpha = if digits.len() == 8 { channel(6)? } else { 255 };
    Some([channel(0)?, channel(2)?, channel(4)?, alpha].map(|c| c as f32 / 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let mut out = String::new();
        hex(&mut out, [1.0, 0.0, 0.2, 1.0]);
        assert_eq!(out, "#ff0033");
        assert_eq!(parse_hex(&out), Some([1.0, 0.0, 0.2, 1.0]));
        out.clear();
        hex(&mut out, [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(out, "#00000080");
        assert_eq!(parse_hex("00000080").map(|c| c[3]), Some(128.0 / 255.0));
    }

    #[test]
    fn partial_hex_is_rejected() {
        assert_eq!(parse_hex("#ff00"), None);
        assert_eq!(parse_hex("#ff003g"), None);
        assert_eq!(parse_hex(""), None);
    }
}
