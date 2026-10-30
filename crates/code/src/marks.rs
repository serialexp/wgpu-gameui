//! How problems are drawn: the wavy underline under a range, and the
//! message shown for the one under the pointer, after Forge's `CodeEditor`
//! (`MARKS` and `squiggle` in `CodeView.jsx`).

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Ink, LayerStack, StyleResolver, SurfacePainter, TextSize, WrapMode};

use crate::diagnostics::Diagnostic;

/// One rise and fall of the wave, and how far it swings either side.
const WAVE_PERIOD: f32 = 4.0;
const WAVE_AMPLITUDE: f32 = 1.0;
/// The message's surface: its widest, its padding, the gap between the
/// severity and the message, and its distance from the line.
const MESSAGE_MAX_WIDTH: f32 = 380.0;
const MESSAGE_PAD_X: f32 = 8.0;
const MESSAGE_PAD_Y: f32 = 5.0;
const MESSAGE_LABEL_GAP: f32 = 7.0;
const MESSAGE_OFFSET: f32 = 2.0;

/// The points of a wave from `x0` to `x1` about `y`, its peaks every half
/// period from `phase`: a wave starting at the text's left edge, so
/// scrolling sideways slides it rather than redrawing it differently.
pub(crate) fn wave(x0: f32, x1: f32, phase: f32, y: f32) -> Vec<[f32; 2]> {
    let half = WAVE_PERIOD / 2.0;
    let peak = |k: i64| {
        if k.rem_euclid(2) == 0 {
            y - WAVE_AMPLITUDE
        } else {
            y + WAVE_AMPLITUDE
        }
    };
    let at = |x: f32| {
        let t = (x - phase) / half;
        let k = t.floor();
        let (a, b) = (peak(k as i64), peak(k as i64 + 1));
        [x, a + (b - a) * (t - k)]
    };
    if x1 <= x0 {
        return Vec::new();
    }
    let mut points = vec![at(x0)];
    let first = ((x0 - phase) / half).floor() as i64 + 1;
    let last = ((x1 - phase) / half).ceil() as i64 - 1;
    for k in first..=last {
        points.push([phase + k as f32 * half, peak(k)]);
    }
    points.push(at(x1));
    points
}

/// Draw `diagnostic`'s message on a tooltip layer of `layers`, under
/// `anchor` (above it when there is no room below in `viewport`): its
/// severity in `ink` and capitals, and its code, before the message.
pub(crate) fn draw_message(
    layers: &mut LayerStack,
    styles: &StyleResolver,
    diagnostic: &Diagnostic,
    ink: [f32; 4],
    anchor: Rect,
    viewport: Rect,
) -> Rect {
    let index = layers.push_tooltip(viewport);
    let list = layers.current_mut();
    let chrome = styles.tooltip();
    let border = chrome.surface.border_widths;
    let theme = styles.theme();

    let mut label = diagnostic.severity.label().to_uppercase();
    if let Some(code) = &diagnostic.code {
        label.push(' ');
        label.push_str(code);
    }
    let mut label = styles
        .mono_block(label, 0.0, 0.0, TextSize::Caption, Ink::Value)
        .with_color_f32(ink);
    let (label_width, _) = list.measure_block(&label);
    let room = MESSAGE_MAX_WIDTH
        - border.left
        - border.right
        - 2.0 * MESSAGE_PAD_X
        - label_width
        - MESSAGE_LABEL_GAP;
    let mut message = styles
        .sans_block(
            diagnostic.message.clone(),
            0.0,
            0.0,
            TextSize::Row,
            Ink::Value,
        )
        .with_max_width(room.max(40.0))
        .with_wrap(WrapMode::Word);
    let (message_width, message_height) = list.measure_block(&message);
    let width = border.left
        + border.right
        + 2.0 * MESSAGE_PAD_X
        + label_width
        + MESSAGE_LABEL_GAP
        + message_width;
    let height = border.top + border.bottom + 2.0 * MESSAGE_PAD_Y + message_height;
    let rect = place(anchor, width, height, viewport);

    // The severity sits on the message's first baseline.
    message.x = rect.x + border.left + MESSAGE_PAD_X + label_width + MESSAGE_LABEL_GAP;
    message.y = rect.y + border.top + MESSAGE_PAD_Y;
    let baseline = message.y
        + styles.text_size(TextSize::Row) * list.font_vmetrics(theme.font.as_ref()).baseline_ratio;
    label.x = rect.x + border.left + MESSAGE_PAD_X;
    label.y = baseline
        - styles.text_size(TextSize::Caption)
            * list.font_vmetrics(theme.mono_font.as_ref()).baseline_ratio;

    let padding_box = rect.inset(border.left);
    let mut surface = SurfacePainter::new(
        list,
        rect,
        padding_box,
        chrome.surface.corner_radii,
        chrome.surface,
        std::slice::from_ref(&chrome.shadow),
        &chrome.lines,
    );
    surface.paint_pre_content();
    surface.draw_list().text(label);
    surface.draw_list().text(message);
    surface.paint_post_content();
    layers.layers_mut()[index].rect = rect;
    layers.pop_layer();
    rect
}

/// A `width` × `height` surface under `anchor`, or above it when only that
/// fits in `viewport`, kept inside it.
pub(crate) fn place(anchor: Rect, width: f32, height: f32, viewport: Rect) -> Rect {
    let below = anchor.bottom() + MESSAGE_OFFSET;
    let y =
        if below + height > viewport.bottom() && anchor.y - MESSAGE_OFFSET - height >= viewport.y {
            anchor.y - MESSAGE_OFFSET - height
        } else {
            below
        };
    let x = anchor.x.min(viewport.right() - width).max(viewport.x);
    Rect::new(
        x,
        y.min(viewport.bottom() - height).max(viewport.y),
        width,
        height,
    )
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
