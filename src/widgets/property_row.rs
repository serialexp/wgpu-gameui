//! Property row — label left, value right (Forge `PropertyRow`).

use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize, Tracking};
use crate::text::{TextAlign, vcentered_line_y};

use super::material;
use super::{DragCapture, DragId, DrawContext, DrawList, PressState, Pressable};

/// A property row's height (`--h-panel-row`).
pub const PROPERTY_ROW_HEIGHT: f32 = 21.0;
/// The label column's default width (`--inspector-label`).
pub const PROPERTY_LABEL_WIDTH: f32 = 52.0;
/// Between the label column and the value column.
pub(crate) const LABEL_GAP: f32 = 7.0;
/// Padding either side of the number in its well.
const VALUE_PAD: f32 = 6.0;
/// Padding right of the unit.
const UNIT_PAD: f32 = 5.0;
/// The ▴▾ stepper column's width, its left rule included.
const STEPPER_W: f32 = 14.0;
/// Half the width of a stepper caret (`▴` at 6 px).
const CARET_HALF: f32 = 2.0;
/// Values are rounded to this many decimals after a scrub or a step, so a
/// run of steps doesn't drift (Forge's `toFixed(4)`).
const ROUND: f64 = 1e4;

/// Where a label scrub started. One is shared by every row on a surface:
/// the [`DragCapture`] says which row owns the drag, this says where it
/// began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropertyScrub {
    start_x: f32,
    base: f64,
}

impl PropertyScrub {
    /// No scrub has started.
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a [`PropertyRow`] frame reported.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropertyRowOutput {
    /// The value after this frame's scrub or step (the value passed in when
    /// nothing changed).
    pub value: f64,
    /// A scrub or a step changed the value this frame.
    pub changed: bool,
    /// The label is being scrubbed.
    pub scrubbing: bool,
}

/// One inspector property: a mono-caps label on the left, the value on the
/// right (Forge `PropertyRow`).
///
/// The label is the scrub handle: press it and drag sideways to change the
/// number by [`step`](Self::step) per pixel. The ▴▾ keys at the well's end
/// step it once. The number can't be typed into. A [`mixed`](Self::mixed)
/// row (a multi-selection whose values differ) shows "—" and can't be
/// changed. A [`read_only`](Self::read_only) row has neither scrub nor keys.
///
/// For a control other than a number, [`draw_slot`](Self::draw_slot) draws
/// the label and hands back the value column to draw into.
///
/// ```ignore
/// let out = PropertyRow::new("Mass").unit("kg").step(1.0).precision(1)
///     .draw(MASS_ID, rect, mass, &mut scrub, &mut capture, &mut ctx);
/// if out.changed { mass = out.value; }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertyRow<'a> {
    label: &'a str,
    step: f64,
    precision: usize,
    unit: Option<&'a str>,
    mixed: bool,
    editable: bool,
    label_width: f32,
}

impl<'a> PropertyRow<'a> {
    /// A row labelled `label`, stepping by 0.1 and showing two decimals.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            step: 0.1,
            precision: 2,
            unit: None,
            mixed: false,
            editable: true,
            label_width: PROPERTY_LABEL_WIDTH,
        }
    }

    /// The change per ▴▾ press and per pixel of scrub (default 0.1).
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Decimals shown (default 2).
    #[must_use]
    pub fn precision(mut self, precision: usize) -> Self {
        self.precision = precision;
        self
    }

    /// A unit after the number, such as "m" or "kg".
    #[must_use]
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = Some(unit);
        self
    }

    /// The selection's values differ: show "—" and don't allow changes.
    #[must_use]
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// Show the value only: no scrub, no ▴▾ keys.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.editable = false;
        self
    }

    /// The label column's width (default [`PROPERTY_LABEL_WIDTH`]; Forge
    /// narrows it to 11 for single-letter axis rows).
    #[must_use]
    pub fn label_width(mut self, width: f32) -> Self {
        self.label_width = width;
        self
    }

    /// Whether the value can be scrubbed or stepped.
    fn changeable(&self) -> bool {
        self.editable && !self.mixed
    }

    /// The label column and the value column of a row at `rect`.
    fn columns(&self, rect: Rect) -> (Rect, Rect) {
        let label_w = self.label_width.min(rect.width).max(0.0);
        let label = Rect::new(rect.x, rect.y, label_w, rect.height);
        let value_x = rect.x + label_w + LABEL_GAP;
        let value = Rect::new(
            value_x,
            rect.y,
            (rect.right() - value_x).max(0.0),
            rect.height,
        );
        (label, value)
    }

    /// Draw the label in `label`, accent while `active`.
    fn draw_label(&self, list: &mut DrawList, s: &StyleResolver, label: Rect, active: bool) {
        let size = s.text_size(TextSize::Caption);
        let mut block = s
            .caption_block(
                self.label,
                label.x,
                vcentered_line_y(label.y, label.height, size),
                Tracking::Prop,
                Ink::Label,
            )
            .with_max_width(label.width)
            .with_ellipsis();
        if active {
            block = block.with_color_f32(s.color(StyleKey::AccentGrip));
        }
        list.text(block);
    }

    /// Draw the label in `rect`'s label column and return the value column,
    /// for a control other than a number (a slider, a checkbox, a dropdown).
    pub fn draw_slot(&self, rect: Rect, ctx: &mut DrawContext) -> Rect {
        let (label, value) = self.columns(rect);
        let s = ctx.styles();
        ctx.push_debug_scope_rect(super::scope_name("PropertyRow", self.label), rect);
        self.draw_label(ctx.draw_list, &s, label, false);
        ctx.pop_debug_scope();
        value
    }

    /// Draw a numeric row showing `value` in `rect` and run its scrub and
    /// steps. `id` is the row's [`DragId`] in the surface's `capture`;
    /// `scrub` is the surface's shared [`PropertyScrub`].
    pub fn draw(
        &self,
        id: DragId,
        rect: Rect,
        value: f64,
        scrub: &mut PropertyScrub,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> PropertyRowOutput {
        let s = ctx.styles();
        let input = ctx.input;
        let (label, column) = self.columns(rect);
        let mut out = PropertyRowOutput {
            value,
            ..PropertyRowOutput::default()
        };

        // The scrub: press the label, drag sideways, let go.
        if capture.is_active(id) {
            if !input.mouse_down || !self.changeable() {
                capture.release(id);
            } else {
                let next = round(scrub.base + f64::from(input.mouse_x - scrub.start_x) * self.step);
                if next != value {
                    out.value = next;
                    out.changed = true;
                }
                out.scrubbing = true;
            }
        } else if self.changeable()
            && input.mouse_clicked
            && !input.mouse_consumed
            && capture.is_free()
            && label.contains(input.mouse_x, input.mouse_y)
        {
            capture.try_begin(id);
            *scrub = PropertyScrub {
                start_x: input.mouse_x,
                base: value,
            };
            out.scrubbing = true;
        }
        let hover_label = !input.mouse_consumed && label.contains(input.mouse_x, input.mouse_y);
        if out.scrubbing || (self.changeable() && hover_label) {
            ctx.request_cursor(crate::CursorIcon::ResizeHorizontal);
        }

        ctx.push_debug_scope_rect(
            super::scope_name("PropertyRow", self.label),
            rect.union(material::row_well_ink(column, out.scrubbing)),
        );
        self.draw_label(ctx.draw_list, &s, label, out.scrubbing);
        let inner = material::draw_row_well(ctx.draw_list, &s, column, out.scrubbing);

        // From the right: the stepper, the unit, then the number.
        let mut right = inner.right();
        if self.editable {
            let stepper = Rect::new(right - STEPPER_W, inner.y, STEPPER_W, inner.height);
            right = stepper.x;
            if let Some(next) = self.draw_stepper(stepper, value, ctx) {
                out.value = next;
                out.changed = true;
            }
        }
        let list = &mut *ctx.draw_list;
        if let Some(unit) = self.unit {
            let size = s.text_size(TextSize::Caption);
            let w = s.mono_width(list, unit, TextSize::Caption);
            right -= UNIT_PAD;
            list.text(s.mono_block(
                unit,
                right - w,
                vcentered_line_y(inner.y, inner.height, size),
                TextSize::Caption,
                Ink::Dim,
            ));
            right -= w;
        }
        let size = s.text_size(TextSize::Dense);
        let (text, ink) = if self.mixed {
            ("—".to_string(), Ink::Disabled)
        } else {
            (format!("{:.*}", self.precision, out.value), Ink::Value)
        };
        let left = inner.x + VALUE_PAD;
        list.text(
            s.mono_block(
                text,
                left,
                vcentered_line_y(inner.y, inner.height, size),
                TextSize::Dense,
                ink,
            )
            .with_max_width((right - VALUE_PAD - left).max(0.0))
            .with_align(TextAlign::Right)
            .with_ellipsis(),
        );
        ctx.pop_debug_scope();
        out
    }

    /// The ▴▾ column at `column`, its left rule included. Returns the
    /// stepped value when a key was pressed.
    fn draw_stepper(&self, column: Rect, value: f64, ctx: &mut DrawContext) -> Option<f64> {
        let s = ctx.styles();
        let rule = s.scalar(StyleKey::BorderWidth);
        ctx.draw_list.quad(
            column.x,
            column.y,
            rule,
            column.height,
            s.color(StyleKey::InputBorder),
        );
        let keys = Rect::new(
            column.x + rule,
            column.y,
            (column.width - rule).max(0.0),
            column.height,
        );
        let half = keys.height * 0.5;
        let mut stepped = None;
        for (i, up) in [(0, true), (1, false)] {
            let rect = Rect::new(keys.x, keys.y + i as f32 * half, keys.width, half);
            let key = Pressable::new()
                .hollow(true)
                .travel(1.0)
                .radius(0.0)
                .enabled(!self.mixed)
                .name(if up { "step up" } else { "step down" });
            let clicked = key
                .draw(rect, ctx, |press, ctx| draw_caret(press, up, ctx))
                .clicked;
            if clicked {
                let sign = if up { 1.0 } else { -1.0 };
                stepped = Some(round(value + sign * self.step));
            }
        }
        stepped
    }
}

/// A stepper key's caret, pointing up or down, centred on its face.
fn draw_caret(press: &PressState, up: bool, ctx: &mut DrawContext) {
    let s = ctx.styles();
    let ink = if press.pressed {
        s.ink(Ink::Second)
    } else if press.hovered {
        s.ink(Ink::Max)
    } else {
        s.ink(Ink::Emph)
    };
    let face = press.face;
    let cx = face.x + face.width * 0.5;
    let cy = face.y + face.height * 0.5;
    let (tip, base) = if up {
        (cy - CARET_HALF * 0.7, cy + CARET_HALF * 0.6)
    } else {
        (cy + CARET_HALF * 0.7, cy - CARET_HALF * 0.6)
    };
    ctx.draw_list.triangle(
        (cx - CARET_HALF, base),
        (cx + CARET_HALF, base),
        (cx, tip),
        ink,
    );
}

/// `value` rounded to four decimals.
fn round(value: f64) -> f64 {
    (value * ROUND).round() / ROUND
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::DebugReport;
    use crate::{FocusState, InputState, Theme};

    const ID: DragId = 9;
    const ROW: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: PROPERTY_ROW_HEIGHT,
    };

    fn at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn press(x: f32, y: f32) -> InputState {
        InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..at(x, y)
        }
    }

    fn hold(x: f32, y: f32) -> InputState {
        InputState {
            mouse_down: true,
            ..at(x, y)
        }
    }

    fn frame(
        row: &PropertyRow,
        value: f64,
        scrub: &mut PropertyScrub,
        capture: &mut DragCapture,
        input: &InputState,
    ) -> (PropertyRowOutput, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            row.draw(ID, ROW, value, scrub, capture, &mut ctx)
        };
        (out, list)
    }

    fn text<'l>(list: &'l DrawList, content: &str) -> &'l crate::TextBlock {
        list.texts
            .iter()
            .find(|t| t.content == content)
            .unwrap_or_else(|| panic!("no text {content:?}"))
    }

    /// The stepper keys of [`ROW`]: the top key's centre, then the bottom's.
    fn stepper_keys() -> ([f32; 2], [f32; 2]) {
        let x = ROW.right() - 1.0 - STEPPER_W * 0.5;
        let top = 1.0 + (ROW.height - 2.0) * 0.25;
        let bottom = 1.0 + (ROW.height - 2.0) * 0.75;
        ([x, top], [x, bottom])
    }

    #[test]
    fn dragging_the_label_scrubs_by_step_per_pixel() {
        let row = PropertyRow::new("Mass").step(0.5);
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let (out, _) = frame(&row, 10.0, &mut scrub, &mut capture, &press(20.0, 10.0));
        assert!(out.scrubbing && !out.changed);
        assert!(capture.is_active(ID));
        // The caller keeps feeding the value back; the scrub counts from
        // where it started, not from the last frame.
        let (out, _) = frame(&row, 10.0, &mut scrub, &mut capture, &hold(26.0, 10.0));
        assert_eq!((out.value, out.changed), (13.0, true));
        let (out, list) = frame(&row, 13.0, &mut scrub, &mut capture, &hold(14.0, 12.0));
        assert_eq!(out.value, 7.0);
        let theme = Theme::default();
        let grip = StyleResolver::new(&theme).color(StyleKey::AccentGrip);
        let label = text(&list, "MASS");
        assert_eq!(
            label.color,
            crate::TextBlock::new("", 0.0, 0.0)
                .with_color_f32(grip)
                .color,
            "the label lights while scrubbed"
        );
        let (out, _) = frame(&row, 7.0, &mut scrub, &mut capture, &at(14.0, 12.0));
        assert!(!out.scrubbing && !out.changed);
        assert!(capture.is_free());
    }

    #[test]
    fn scrubbed_values_are_rounded_to_four_decimals() {
        let row = PropertyRow::new("X").step(0.1);
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        frame(&row, 0.2, &mut scrub, &mut capture, &press(20.0, 10.0));
        let (out, _) = frame(&row, 0.2, &mut scrub, &mut capture, &hold(21.0, 10.0));
        assert_eq!(out.value, 0.3, "not 0.30000000000000004");
    }

    #[test]
    fn the_stepper_keys_step_up_and_down() {
        let row = PropertyRow::new("Mass").step(0.25);
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let (up, down) = stepper_keys();
        let (out, _) = frame(&row, 1.0, &mut scrub, &mut capture, &press(up[0], up[1]));
        assert_eq!((out.value, out.changed), (1.25, true));
        let (out, _) = frame(
            &row,
            1.0,
            &mut scrub,
            &mut capture,
            &press(down[0], down[1]),
        );
        assert_eq!((out.value, out.changed), (0.75, true));
        assert!(capture.is_free(), "a step is not a scrub");
    }

    #[test]
    fn a_mixed_row_shows_a_dash_and_cannot_change() {
        let row = PropertyRow::new("Mass").mixed(true).unit("kg");
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let (out, list) = frame(&row, 72.0, &mut scrub, &mut capture, &press(20.0, 10.0));
        assert!(!out.scrubbing);
        assert!(capture.is_free());
        assert!(list.texts.iter().any(|t| t.content == "—"));
        assert!(list.texts.iter().all(|t| t.content != "72.00"));
        let (up, _) = stepper_keys();
        let (out, _) = frame(&row, 72.0, &mut scrub, &mut capture, &press(up[0], up[1]));
        assert!(!out.changed, "the keys are disabled");
    }

    #[test]
    fn a_read_only_row_has_no_keys_and_no_scrub() {
        let row = PropertyRow::new("Mass").read_only();
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let (out, _) = frame(&row, 3.0, &mut scrub, &mut capture, &press(20.0, 10.0));
        assert!(!out.scrubbing);
        let (up, _) = stepper_keys();
        let (out, list) = frame(&row, 3.0, &mut scrub, &mut capture, &press(up[0], up[1]));
        assert!(!out.changed);
        // The number runs to the well's end instead of stopping at the keys.
        let value = text(&list, "3.00");
        let right = value.x + value.max_width;
        assert!((right - (ROW.right() - 1.0 - VALUE_PAD)).abs() < 0.01);
    }

    #[test]
    fn a_consumed_press_or_a_busy_capture_starts_no_scrub() {
        let row = PropertyRow::new("Mass");
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let consumed = InputState {
            mouse_consumed: true,
            ..press(20.0, 10.0)
        };
        let (out, _) = frame(&row, 1.0, &mut scrub, &mut capture, &consumed);
        assert!(!out.scrubbing);
        assert!(capture.try_begin(77));
        let (out, _) = frame(&row, 1.0, &mut scrub, &mut capture, &press(20.0, 10.0));
        assert!(!out.scrubbing);
        assert!(capture.is_active(77));
    }

    #[test]
    fn pressing_the_value_does_not_scrub() {
        let row = PropertyRow::new("Mass");
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let (out, _) = frame(&row, 1.0, &mut scrub, &mut capture, &press(100.0, 10.0));
        assert!(!out.scrubbing);
    }

    #[test]
    fn the_number_sits_right_before_the_unit_and_the_keys() {
        let row = PropertyRow::new("Mass").unit("kg").precision(1);
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let (_, mut list) = frame(&row, 72.0, &mut scrub, &mut capture, &at(-5.0, -5.0));
        let unit = text(&list, "kg").clone();
        let value = text(&list, "72.0").clone();
        let (unit_w, _) = list.measure_block(&unit);
        let keys_left = ROW.right() - 1.0 - STEPPER_W;
        assert!((unit.x + unit_w - (keys_left - UNIT_PAD)).abs() < 0.01);
        assert!((value.x + value.max_width - (unit.x - VALUE_PAD)).abs() < 0.01);
        assert_eq!(value.align, TextAlign::Right);
        let label = text(&list, "MASS");
        assert_eq!(label.x, 0.0);
    }

    #[test]
    fn a_slot_row_draws_the_label_and_returns_the_value_column() {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = at(-5.0, -5.0);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        let slot = PropertyRow::new("Rough").draw_slot(ROW, &mut ctx);
        let x = PROPERTY_LABEL_WIDTH + LABEL_GAP;
        assert_eq!(slot, Rect::new(x, 0.0, 200.0 - x, ROW.height));
        assert_eq!(list.texts.len(), 1);
        assert_eq!(list.texts[0].content, "ROUGH");
    }

    #[test]
    fn a_narrow_label_column_moves_the_well_left() {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = at(-5.0, -5.0);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        let slot = PropertyRow::new("X")
            .label_width(11.0)
            .draw_slot(ROW, &mut ctx);
        assert_eq!(slot.x, 11.0 + LABEL_GAP);
    }

    #[test]
    fn the_declared_area_covers_the_scrub_ring() {
        let row = PropertyRow::new("Mass");
        let mut scrub = PropertyScrub::new();
        let mut capture = DragCapture::new();
        let screen = Rect::new(-10.0, -10.0, 400.0, 100.0);
        for input in [at(-5.0, -5.0), press(20.0, 10.0)] {
            let (_, list) = frame(&row, 1.0, &mut scrub, &mut capture, &input);
            let report = DebugReport::from_draw_list(&list, screen);
            assert!(
                report
                    .problems
                    .iter()
                    .all(|p| p.code() != "overflows_declared"),
                "{}",
                report.to_text()
            );
        }
    }
}
