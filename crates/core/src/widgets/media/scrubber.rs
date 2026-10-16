//! Scrubber — a media timeline (Forge `Scrubber`): a sunken track with the
//! buffered part dim under the played part in the accent, and a knob that
//! is always shown (Forge never hides controls; [`knob(false)`](Scrubber::knob)
//! makes a bare meter).
//!
//! Hovering shows the time under the pointer in a tip above it. A press
//! seeks there and a drag seeks live, the output saying when the drag
//! [`started`](ScrubberOutput::started) and [`ended`](ScrubberOutput::ended)
//! so a player can hold its picture meanwhile. While focused, ←/→ step (5 s
//! over a timeline longer than 30 s, a fiftieth of a shorter one; Shift
//! steps six times as far) and Home and End go to the ends.
//!
//! Values are in whatever the caller counts in (seconds for a timeline,
//! `0..=1` for a volume); [`format`](Scrubber::format) turns one into the
//! tip's text.
//!
//! # Example
//! ```ignore
//! let output = Scrubber::new(length)
//!     .buffered(buffered)
//!     .draw(time, SEEK_DRAG, &mut capture, rect, &mut ctx);
//! if let Some(to) = output.seek {
//!     player.seek(to);
//! }
//! ```

use std::time::Duration;

use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::super::chart;
use super::super::material::draw_inset_shadow;
use super::super::{DragCapture, DragId, DrawContext, DrawList, FocusId};
use super::format_time;

/// `--slider-track`.
const TRACK: f32 = 5.0;
/// The knob's diameter: `calc(var(--slider-track) + 5px)`.
const KNOB: f32 = TRACK + 5.0;
/// The well: `rgba(0,0,0,0.55)`.
const WELL: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// The buffered layer: `rgba(255,255,255,0.2)`.
pub(super) const BUFFERED: [f32; 4] = [1.0, 1.0, 1.0, 0.2];
/// The layer up to the pointer: `rgba(255,255,255,0.08)`.
const HOVER: [f32; 4] = [1.0, 1.0, 1.0, 0.08];
/// The knob's face, top to bottom: `#f1f5f9` to `#c6ced5`; while dragged,
/// `#cfd6dd` to `#f1f5f9`.
pub(super) const KNOB_TOP: [f32; 4] = [0.945, 0.961, 0.976, 1.0];
const KNOB_BOTTOM: [f32; 4] = [0.776, 0.808, 0.835, 1.0];
const KNOB_DRAGGED_TOP: [f32; 4] = [0.812, 0.839, 0.867, 1.0];
/// The knob's edge: `rgba(0,0,0,0.6)`.
const KNOB_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// The tip's bottom sits this far above the track's middle.
const TIP_LIFT: f32 = 10.0;
/// A disabled scrubber's opacity.
const DISABLED: f32 = 0.5;

/// What a [`Scrubber`] saw this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrubberOutput {
    /// Seek here: pressed, dragged or keyed to, and not where it was.
    pub seek: Option<f32>,
    /// The knob is being dragged.
    pub dragging: bool,
    /// A drag started this frame.
    pub started: bool,
    /// A drag ended this frame.
    pub ended: bool,
    /// The pointer is over it.
    pub hovered: bool,
}

/// A media timeline. See the [module docs](self).
pub struct Scrubber<'a> {
    max: f32,
    buffered: Option<f32>,
    step: Option<f32>,
    knob: bool,
    tip: bool,
    enabled: bool,
    format: Option<&'a dyn Fn(f32) -> String>,
    focus_id: Option<FocusId>,
    name: &'static str,
}

impl<'a> Scrubber<'a> {
    /// A timeline from 0 to `max` (a length of 0 seeks nowhere but the start).
    pub fn new(max: f32) -> Self {
        Self {
            max: max.max(0.0),
            buffered: None,
            step: None,
            knob: true,
            tip: true,
            enabled: true,
            format: None,
            focus_id: None,
            name: "Scrubber",
        }
    }

    /// Show what can play without waiting, from the start to `buffered`.
    pub fn buffered(mut self, buffered: f32) -> Self {
        self.buffered = Some(buffered);
        self
    }

    /// The step ←/→ take (Shift takes six).
    pub fn step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }

    /// Show the knob (the default), or not, for a bare meter.
    pub fn knob(mut self, knob: bool) -> Self {
        self.knob = knob;
        self
    }

    /// Show the tip over the pointer (the default), or not.
    pub fn tip(mut self, tip: bool) -> Self {
        self.tip = tip;
        self
    }

    /// Take input (the default), or show faded and take none.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The tip's text for a value; seconds as a time (`1:30`) by default.
    pub fn format(mut self, format: &'a dyn Fn(f32) -> String) -> Self {
        self.format = Some(format);
        self
    }

    /// Join the Tab ring under `id`, and take ←/→, Home and End while focused.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    /// The debug scope's name, to tell scrubbers apart in tests.
    pub fn name(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    fn text(&self, value: f32) -> String {
        match self.format {
            Some(format) => format(value),
            None => format_time(Duration::from_secs_f32(value.max(0.0))),
        }
    }

    /// How far ←/→ go.
    fn key_step(&self) -> f32 {
        self.step.unwrap_or(if self.max > 30.0 {
            5.0
        } else {
            self.max / 50.0
        })
    }

    /// Draw it at `value` in `rect` (the area it takes the pointer in; the
    /// track runs along its middle). `id` is its [`DragId`] in `capture`,
    /// which arbitrates drags between widgets.
    pub fn draw(
        &self,
        value: f32,
        id: DragId,
        capture: &mut DragCapture,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> ScrubberOutput {
        ctx.push_debug_scope_rect(self.name, rect);
        let input = ctx.input;
        let at = |x: f32| {
            if rect.width > 0.0 {
                ((x - rect.x) / rect.width).clamp(0.0, 1.0) * self.max
            } else {
                0.0
            }
        };
        let mut output = ScrubberOutput::default();

        let was_dragging = capture.is_active(id);
        if !input.mouse_down || !self.enabled {
            capture.release(id);
        }
        output.hovered =
            self.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        if output.hovered && input.mouse_clicked && capture.is_free() {
            capture.try_begin(id);
            output.started = true;
        }
        output.dragging = capture.is_active(id);
        output.ended = was_dragging && !output.dragging;
        let pointer = (output.hovered || output.dragging).then(|| at(input.mouse_x));
        if output.dragging {
            output.seek = pointer;
        }

        let mut focused = false;
        if let Some(focus_id) = self.focus_id.filter(|_| self.enabled) {
            ctx.register_focus(focus_id);
            if output.started {
                ctx.focus.request(focus_id);
            }
            focused = ctx.focus.is_focused(focus_id);
            if focused {
                let step = self.key_step() * if input.shift_pressed { 6.0 } else { 1.0 };
                let keyed = if input.key_home {
                    Some(0.0)
                } else if input.key_end {
                    Some(self.max)
                } else if input.nav.right {
                    Some(value + step)
                } else if input.nav.left {
                    Some(value - step)
                } else {
                    None
                };
                if let Some(keyed) = keyed {
                    output.seek = Some(keyed.clamp(0.0, self.max));
                }
            }
        }
        // A seek to where it is already isn't one, unless a press asked.
        if !output.started && output.seek.is_some_and(|seek| seek == value) {
            output.seek = None;
        }

        if output.dragging {
            ctx.request_cursor(crate::CursorIcon::Grabbing);
        } else if output.hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }

        let shown = output.seek.unwrap_or(value);
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        list.push_tint();
        if !self.enabled {
            list.multiply_tint([1.0, 1.0, 1.0, DISABLED]);
        }
        let knob = self.paint(list, &s, rect, shown, pointer, output.dragging);
        list.pop_tint();
        if focused && let Some(knob) = knob {
            ctx.draw_focus_ring(knob);
        }
        if self.tip
            && let Some(pointer) = pointer
        {
            let text = self.text(pointer);
            let x = rect.x + rect.width * self.fraction(pointer);
            let s = ctx.styles();
            paint_tip(
                ctx.draw_list,
                &s,
                &text,
                x,
                rect.y + rect.height * 0.5 - TIP_LIFT,
            );
        }
        ctx.pop_debug_scope();
        output
    }

    /// `value` as a fraction of the length.
    fn fraction(&self, value: f32) -> f32 {
        if self.max > 0.0 {
            (value / self.max).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Paint the track, its layers and the knob; returns the knob's rect.
    fn paint(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        rect: Rect,
        value: f32,
        pointer: Option<f32>,
        dragging: bool,
    ) -> Option<Rect> {
        let track = Rect::new(
            rect.x,
            (rect.y + (rect.height - TRACK) * 0.5).round(),
            rect.width,
            TRACK,
        );
        list.chrome_rect(track, 3.0, 1.0, WELL, s.color(StyleKey::InputBorder));
        draw_inset_shadow(list, s, track, s.scalar(StyleKey::InnerShadowDepth), 1.0);
        let inner = track.inset(1.0);
        let layer = |list: &mut DrawList, upto: f32, color: [f32; 4]| {
            let width = inner.width * self.fraction(upto);
            if width > 0.0 {
                list.chrome_rect(
                    Rect::new(inner.x, inner.y, width, inner.height),
                    2.0,
                    0.0,
                    color,
                    [0.0; 4],
                );
            }
        };
        if let Some(buffered) = self.buffered {
            layer(list, buffered, BUFFERED);
        }
        if let Some(pointer) = pointer.filter(|_| !dragging) {
            layer(list, pointer, HOVER);
        }
        let width = inner.width * self.fraction(value);
        if width > 0.0 {
            let (top, bottom) = if dragging {
                (
                    StyleKey::AccentFaceTopHover,
                    StyleKey::AccentFaceBottomHover,
                )
            } else {
                (StyleKey::AccentFaceTop, StyleKey::AccentFaceBottom)
            };
            list.chrome_rect_gradient(
                Rect::new(inner.x, inner.y, width, inner.height),
                2.0,
                0.0,
                s.color(top),
                s.color(bottom),
                [0.0; 4],
            );
        }
        if !(self.knob && self.enabled) {
            return None;
        }
        let center = (inner.x + width, track.y + TRACK * 0.5);
        let knob = Rect::new(center.0 - KNOB * 0.5, center.1 - KNOB * 0.5, KNOB, KNOB);
        let (top, bottom) = if dragging {
            list.circle(center, KNOB * 0.5 + 2.0, s.color(StyleKey::AccentRing));
            (KNOB_DRAGGED_TOP, KNOB_TOP)
        } else {
            (KNOB_TOP, KNOB_BOTTOM)
        };
        list.chrome_rect_gradient(knob, KNOB * 0.5, 1.0, top, bottom, KNOB_EDGE);
        Some(knob)
    }
}

/// The tip: `text` in mono on the tooltip surface, centred on `x`, its
/// bottom at `bottom`.
fn paint_tip(list: &mut DrawList, s: &StyleResolver, text: &str, x: f32, bottom: f32) {
    let row = s.text_size(TextSize::Meta);
    let content = s.mono_width(list, text, TextSize::Meta);
    let (width, height) = chart::tooltip_size(s, content, row, 0.0);
    let rect = Rect::new(
        (x - width * 0.5).round(),
        (bottom - height).round(),
        width,
        height,
    );
    chart::paint_tooltip(list, s, rect, |list, inner| {
        let y = crate::text::vcentered_line_y(inner.y, inner.height, row);
        list.text(s.mono_block(text, inner.x, y, TextSize::Meta, Ink::Value));
    });
}

#[cfg(test)]
#[path = "scrubber_tests.rs"]
mod tests;
