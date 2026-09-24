//! Window — a movable in-app window with a title strip, a close key, and an
//! optional bottom-right resize grip.
//!
//! Unlike [`Group`](super::Group) (a stateless titled box), a `Window` is
//! interactive: dragging its title strip moves it, the close key reports
//! [`WindowOutput::close_clicked`], and — when [`resizable`](Window::resizable)
//! — dragging the corner grip resizes it. It paints the theme's
//! [`WindowChrome`](crate::WindowChrome) and returns the **body rect** for the
//! caller to lay content into.
//!
//! # State ownership
//!
//! [`WindowState`] (the outer rect) is caller-owned and persisted across
//! frames, like [`DockPanelState`](super::DockPanelState). Store it however you
//! like — e.g. serialize `state.rect` when [`WindowOutput::interaction_ended`]
//! fires to remember the window's placement between launches.
//!
//! Drag ownership goes through the shared [`DragCapture`] (usually
//! [`UiState::drag`](crate::UiState)), so a window never fights a slider or
//! splitter for the same gesture. The per-frame movement comes from a
//! caller-owned [`DragTracker`](crate::DragTracker): run
//! `tracker.update(&mut input)` once per frame **before** drawing, exactly as
//! for [`DragHandle`](super::DragHandle). A window claims two ids: `id` for
//! moving and `id + 1` for resizing.
//!
//! # Layers
//!
//! The window draws into `ctx.draw_list` and reads `ctx.input`; the caller
//! picks the layer. Push the layer with the window's *current* rect and draw
//! with that layer's input:
//!
//! * **Blocking** — [`LayerStack::push_modal`](crate::LayerStack::push_modal):
//!   everything below ignores the pointer while the window is open.
//! * **Floating** — [`LayerStack::push_popup`](crate::LayerStack::push_popup):
//!   only the window's own rect captures the pointer; the rest of the UI keeps
//!   working.
//!
//! ```ignore
//! let layer = layers.push_popup(state.rect);
//! layers.pop_layer();
//! // ... draw the base UI with layers.input_for_base(&input) ...
//! let input = layers.input_for_layer(layer, &input);
//! let mut ctx = DrawContext::new(&mut layers.layers_mut()[layer].list, focus, theme, &input, w, h)
//!     .with_cursor(&mut cursor);
//! let out = Window::new(WINDOW_ID, "Inspector")
//!     .resizable(true)
//!     .bounds(workspace)
//!     .draw(&mut state, &mut ui_state.drag, &mut ctx);
//! if out.close_clicked { open = false; }
//! draw_contents(out.body);
//! if out.interaction_ended { save_layout(state.rect); }
//! ```
//!
//! # Geometry
//!
//! The title strip is [`StyleKey::WindowTitleHeight`] tall, measured from the
//! outer top edge. The close key is a square inset [`WINDOW_CLOSE_KEY_INSET`] from the
//! strip on every side, so it always fits the strip. A resizable window
//! reserves a [`WINDOW_RESIZE_GRIP_SIZE`] band along its bottom for the grip, keeping
//! the grip out of the body so body widgets (e.g. a scroll bar in the corner)
//! never share a press with it.

use crate::chrome::SurfacePainter;
use crate::layout::Rect;
use crate::style::StyleKey;
use crate::text::TextBlock;

use super::{DragCapture, DragHandle, DragId, DrawContext};

/// Gap between the close key and the title strip's edges, in pixels.
pub const WINDOW_CLOSE_KEY_INSET: f32 = 3.0;
/// Side length of the bottom-right resize grip, in pixels.
pub const WINDOW_RESIZE_GRIP_SIZE: f32 = 14.0;
/// Default minimum outer size (`[w, h]`) a resizable window can shrink to.
pub const DEFAULT_WINDOW_MIN_SIZE: [f32; 2] = [160.0, 96.0];

/// Caller-owned window placement, persisted across frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowState {
    /// Outer rect, including the border and the title strip.
    pub rect: Rect,
}

impl WindowState {
    /// A window occupying `rect`.
    pub fn new(rect: Rect) -> Self {
        Self { rect }
    }

    /// A window of `size` (`[w, h]`) centred in `bounds`, shrunk to fit.
    pub fn centered(bounds: Rect, size: [f32; 2]) -> Self {
        let width = size[0].min(bounds.width).max(0.0);
        let height = size[1].min(bounds.height).max(0.0);
        Self::new(Rect::new(
            bounds.x + (bounds.width - width) * 0.5,
            bounds.y + (bounds.height - height) * 0.5,
            width,
            height,
        ))
    }
}

/// Result of drawing a [`Window`] for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowOutput {
    /// Content area below the title strip (and above the resize band, if
    /// any), inside the border. Lay the window's contents out here.
    pub body: Rect,
    /// The close key was clicked this frame. The caller closes the window.
    pub close_clicked: bool,
    /// The title strip owns the drag (the window is being moved).
    pub dragging: bool,
    /// The resize grip owns the drag (the window is being resized).
    pub resizing: bool,
    /// A move or resize gesture was released this frame — the moment to
    /// persist [`WindowState::rect`]. Fires once per gesture, never per frame.
    pub interaction_ended: bool,
}

/// A movable in-app window. See the [module docs](self).
pub struct Window<'a> {
    id: DragId,
    title: &'a str,
    closable: bool,
    resizable: bool,
    min_size: [f32; 2],
    bounds: Option<Rect>,
}

impl<'a> Window<'a> {
    /// A closable, fixed-size window titled `title`. `id` must be unique among
    /// drag participants sharing the [`DragCapture`]; the window also uses
    /// `id + 1` for its resize grip.
    pub fn new(id: DragId, title: &'a str) -> Self {
        Self {
            id,
            title,
            closable: true,
            resizable: false,
            min_size: DEFAULT_WINDOW_MIN_SIZE,
            bounds: None,
        }
    }

    /// Show the close key (default `true`).
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Show the bottom-right resize grip (default `false`).
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Minimum outer size (`[w, h]`) while resizing. Never smaller than the
    /// title strip plus the resize band. Defaults to [`DEFAULT_WINDOW_MIN_SIZE`].
    pub fn min_size(mut self, min_size: [f32; 2]) -> Self {
        self.min_size = [min_size[0].max(0.0), min_size[1].max(0.0)];
        self
    }

    /// Keep the whole window inside `bounds` (typically the workspace between
    /// the menu bar and status bar). Moving stops at the edges, resizing stops
    /// at the right/bottom edges, and a window larger than `bounds` is shrunk
    /// to fit — `bounds` wins over [`min_size`](Self::min_size) so a window
    /// can never be lost off-screen.
    pub fn bounds(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// The drag id of the resize grip (`id + 1`).
    pub fn resize_id(&self) -> DragId {
        self.id.wrapping_add(1)
    }

    /// The close key's rect for an outer `rect`, a `title_height` strip, and
    /// a `border` width: a square inset [`WINDOW_CLOSE_KEY_INSET`] from the strip.
    pub fn close_rect(rect: Rect, title_height: f32, border: f32) -> Rect {
        let side = (title_height - 2.0 * WINDOW_CLOSE_KEY_INSET).max(0.0);
        Rect::new(
            rect.right() - border - WINDOW_CLOSE_KEY_INSET - side,
            rect.y + (title_height - side) * 0.5,
            side,
            side,
        )
    }

    /// The resize grip's rect in the bottom-right corner, inside the border.
    pub fn grip_rect(rect: Rect, border: f32) -> Rect {
        Rect::new(
            rect.right() - border - WINDOW_RESIZE_GRIP_SIZE,
            rect.bottom() - border - WINDOW_RESIZE_GRIP_SIZE,
            WINDOW_RESIZE_GRIP_SIZE,
            WINDOW_RESIZE_GRIP_SIZE,
        )
    }

    /// The body rect for an outer `rect`: below the title strip, inside the
    /// border, and above the resize band when `resizable`.
    pub fn body_rect(rect: Rect, title_height: f32, border: f32, resizable: bool) -> Rect {
        let bottom_band = if resizable {
            WINDOW_RESIZE_GRIP_SIZE
        } else {
            0.0
        };
        Rect::new(
            rect.x + border,
            rect.y + title_height,
            (rect.width - 2.0 * border).max(0.0),
            (rect.height - title_height - border - bottom_band).max(0.0),
        )
    }

    /// Clamp `rect` to the minimum size and the bounds. When `resizing`, the
    /// right/bottom edges stop at the bounds instead of pushing the window.
    fn constrain(&self, rect: &mut Rect, title_height: f32, border: f32, resizing: bool) {
        let band = if self.resizable {
            WINDOW_RESIZE_GRIP_SIZE
        } else {
            0.0
        };
        let floor_h = title_height + border + band;
        let min_w = self.min_size[0];
        let min_h = self.min_size[1].max(floor_h);
        if self.resizable || resizing {
            rect.width = rect.width.max(min_w);
            rect.height = rect.height.max(min_h);
        }
        let Some(bounds) = self.bounds else {
            return;
        };
        if resizing {
            rect.width = rect.width.min(bounds.right() - rect.x);
            rect.height = rect.height.min(bounds.bottom() - rect.y);
        }
        rect.width = rect.width.min(bounds.width).max(0.0);
        rect.height = rect.height.min(bounds.height).max(0.0);
        rect.x = rect
            .x
            .clamp(bounds.x, (bounds.right() - rect.width).max(bounds.x));
        rect.y = rect
            .y
            .clamp(bounds.y, (bounds.bottom() - rect.height).max(bounds.y));
    }

    /// Handle input, move/resize `state`, paint the window at its new rect, and
    /// return the body rect plus this frame's events.
    pub fn draw(
        &self,
        state: &mut WindowState,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> WindowOutput {
        let s = ctx.styles();
        let chrome = s.window();
        let input = ctx.input;
        let title_height = s.scalar(StyleKey::WindowTitleHeight).max(1.0);
        let border = chrome.surface.border_widths.top.max(0.0);

        // Bounds can shrink between frames (host window resized); re-fit first
        // so hit geometry matches what was last painted as closely as possible.
        self.constrain(&mut state.rect, title_height, border, false);
        let start = state.rect;
        let pointer_free = !input.mouse_consumed;
        let (mx, my) = (input.mouse_x, input.mouse_y);

        // --- Close key: hit-tested on this frame's starting geometry, only
        // while no drag owns the pointer. It sits outside the move handle, so
        // clicking it can never start a move.
        let close = self
            .closable
            .then(|| Self::close_rect(start, title_height, border));
        let close_hovered =
            close.is_some_and(|r| r.contains(mx, my)) && pointer_free && capture.is_free();
        let close_clicked = close_hovered && input.mouse_clicked;
        if close_hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }

        // --- Move: the title strip minus the close key.
        let strip_right = close.map_or(start.right(), |r| r.x - WINDOW_CLOSE_KEY_INSET);
        let strip = Rect::new(
            start.x,
            start.y,
            (strip_right - start.x).max(0.0),
            title_height,
        );
        let moved = DragHandle::bare().draw(self.id, capture, strip, ctx);

        // --- Resize: same capture arbitration as DragHandle, under `id + 1`.
        let mut resizing = false;
        let mut resize_released = false;
        let mut grip_hot = false;
        let mut resize_delta = [0.0, 0.0];
        if self.resizable {
            let rid = self.resize_id();
            let grip = Self::grip_rect(start, border);
            let hovered = grip.contains(mx, my) && pointer_free;
            let was_resizing = capture.is_active(rid);
            if !input.mouse_down {
                capture.release(rid);
            }
            if hovered && input.mouse_clicked && capture.is_free() {
                capture.try_begin(rid);
            }
            resizing = capture.is_active(rid);
            resize_released = was_resizing && !resizing;
            grip_hot = resizing || (hovered && capture.is_free());
            if grip_hot {
                ctx.request_cursor(crate::CursorIcon::ResizeDiagonal);
            }
            if resizing {
                resize_delta = input.drag_delta;
            }
        }

        // Apply this frame's movement before painting so the window tracks the
        // pointer without a one-frame lag.
        state.rect.x += moved.delta[0];
        state.rect.y += moved.delta[1];
        state.rect.width += resize_delta[0];
        state.rect.height += resize_delta[1];
        self.constrain(&mut state.rect, title_height, border, resizing);
        let rect = state.rect;

        // --- Paint.
        ctx.push_debug_scope_rect(crate::widgets::scope_name("Window", self.title), rect);
        let widths = chrome.surface.border_widths;
        let padding_box = Rect::new(
            rect.x + widths.left,
            rect.y + widths.top,
            (rect.width - widths.left - widths.right).max(0.0),
            (rect.height - widths.top - widths.bottom).max(0.0),
        );
        let title_color = s.color(StyleKey::TextHighlight);
        let text_dim = s.color(StyleKey::TextDim);
        let text = s.color(StyleKey::Text);
        let font_size = s.scalar(StyleKey::FontSize);
        let font = s.theme().font.clone();
        {
            let mut surface = SurfacePainter::new(
                ctx.draw_list,
                rect,
                padding_box,
                chrome.surface.corner_radii,
                chrome.surface,
                core::slice::from_ref(&chrome.shadow),
                &chrome.lines,
            );
            surface.paint_pre_content();
            {
                let list = surface.draw_list();
                let header_rect = Rect::new(
                    padding_box.x,
                    padding_box.y,
                    padding_box.width,
                    (title_height - widths.top).max(0.0),
                );
                let mut header = SurfacePainter::new(
                    list,
                    header_rect,
                    header_rect,
                    chrome.header.corner_radii,
                    chrome.header,
                    &[],
                    &chrome.header_lines,
                );
                header.paint_pre_content();
                header.paint_post_content();

                // Title: highlight colour, left-padded, vertically centred,
                // clipped so a long title never runs under the close key.
                let close_now = self
                    .closable
                    .then(|| Self::close_rect(rect, title_height, border));
                let title_right = close_now.map_or(padding_box.right(), |r| r.x);
                let pad = s.scalar(StyleKey::Padding);
                let title_clip = Rect::new(
                    padding_box.x,
                    rect.y,
                    (title_right - padding_box.x).max(0.0),
                    title_height,
                );
                if title_clip.width > 0.0 && !self.title.is_empty() {
                    let ty = list.vcentered_text_y(
                        rect.y,
                        title_height,
                        font_size,
                        font.as_ref(),
                        self.title,
                    );
                    list.push_clip(title_clip);
                    list.text(
                        TextBlock::new(self.title, padding_box.x + pad, ty)
                            .with_size(font_size)
                            .with_color(
                                (title_color[0] * 255.0) as u8,
                                (title_color[1] * 255.0) as u8,
                                (title_color[2] * 255.0) as u8,
                            )
                            .with_font_opt(font.clone()),
                    );
                    list.pop_clip();
                }

                if let Some(key) = close_now {
                    // Hover follows the pre-move hit test; the key cannot be
                    // hovered while the window itself is being dragged.
                    if close_hovered {
                        list.paint_quad(key, chrome.close_hover);
                    }
                    let tint = if close_hovered { text } else { text_dim };
                    #[cfg(feature = "phosphor-icons")]
                    list.phosphor_icon(key.inset(key.width * 0.25), crate::PhosphorIcon::X, tint);
                    #[cfg(not(feature = "phosphor-icons"))]
                    {
                        let size = (key.height * 0.55).max(1.0);
                        let xty =
                            list.vcentered_text_y(key.y, key.height, size, font.as_ref(), "✕");
                        list.text(
                            TextBlock::new("✕", key.x + (key.width - size * 0.6) * 0.5, xty)
                                .with_size(size)
                                .with_color(
                                    (tint[0] * 255.0) as u8,
                                    (tint[1] * 255.0) as u8,
                                    (tint[2] * 255.0) as u8,
                                )
                                .with_font_opt(font.clone()),
                        );
                    }
                }

                if self.resizable {
                    // Classic corner grip: a triangle of 2px dots.
                    let grip = Self::grip_rect(rect, border);
                    let color = chrome.grip_colors[usize::from(grip_hot)];
                    let dot = 2.0;
                    let step = 4.0;
                    for row in 0..3 {
                        for col in 0..3 {
                            if row + col < 2 {
                                continue;
                            }
                            list.quad(
                                grip.x + 2.0 + col as f32 * step,
                                grip.y + 2.0 + row as f32 * step,
                                dot,
                                dot,
                                color,
                            );
                        }
                    }
                }
            }
            surface.paint_post_content();
        }
        ctx.pop_debug_scope();

        WindowOutput {
            body: Self::body_rect(rect, title_height, border, self.resizable),
            close_clicked,
            dragging: moved.dragging,
            resizing,
            interaction_ended: moved.released || resize_released,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, StyleOverlay, Theme};

    const ID: DragId = 40;

    fn run(
        window: &Window,
        state: &mut WindowState,
        capture: &mut DragCapture,
        input: &InputState,
    ) -> (WindowOutput, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            window.draw(state, capture, &mut ctx)
        };
        (out, list)
    }

    fn title_height() -> f32 {
        Theme::default().window_title_height
    }

    fn border() -> f32 {
        Theme::default().chrome.window.surface.border_widths.top
    }

    fn press(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    fn hold(x: f32, y: f32, delta: [f32; 2]) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            is_dragging: true,
            drag_delta: delta,
            ..Default::default()
        }
    }

    fn release(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_released: true,
            ..Default::default()
        }
    }

    #[test]
    fn centered_state_is_centred_and_shrunk_to_fit() {
        let state = WindowState::centered(Rect::new(0.0, 0.0, 800.0, 600.0), [300.0, 200.0]);
        assert_eq!(state.rect, Rect::new(250.0, 200.0, 300.0, 200.0));
        let tight = WindowState::centered(Rect::new(10.0, 10.0, 100.0, 50.0), [300.0, 200.0]);
        assert_eq!(tight.rect, Rect::new(10.0, 10.0, 100.0, 50.0));
    }

    #[test]
    fn close_key_and_body_fit_inside_the_title_strip() {
        let rect = Rect::new(100.0, 50.0, 300.0, 200.0);
        let th = title_height();
        let key = Window::close_rect(rect, th, border());
        assert!(key.y >= rect.y, "key starts inside the strip");
        assert!(key.bottom() <= rect.y + th, "key ends inside the strip");
        assert!(key.right() <= rect.right(), "key inside the right edge");
        let body = Window::body_rect(rect, th, border(), false);
        assert_eq!(body.y, rect.y + th, "body starts below the strip");
        let resizable_body = Window::body_rect(rect, th, border(), true);
        assert!(
            resizable_body.bottom() <= Window::grip_rect(rect, border()).y,
            "the resize band keeps the grip out of the body"
        );
    }

    #[test]
    fn dragging_the_title_strip_moves_the_window() {
        let window = Window::new(ID, "Tools");
        let mut state = WindowState::new(Rect::new(100.0, 100.0, 200.0, 150.0));
        let mut capture = DragCapture::new();

        let (out, _) = run(&window, &mut state, &mut capture, &press(150.0, 110.0));
        assert!(out.dragging);
        assert_eq!(
            state.rect.x, 100.0,
            "no movement before the tracker threshold"
        );

        let (out, _) = run(
            &window,
            &mut state,
            &mut capture,
            &hold(170.0, 115.0, [20.0, 5.0]),
        );
        assert!(out.dragging);
        assert_eq!(state.rect, Rect::new(120.0, 105.0, 200.0, 150.0));
        assert!(!out.interaction_ended);

        let (out, _) = run(&window, &mut state, &mut capture, &release(170.0, 115.0));
        assert!(!out.dragging);
        assert!(
            out.interaction_ended,
            "release reports the gesture end once"
        );
        let (out, _) = run(&window, &mut state, &mut capture, &release(170.0, 115.0));
        assert!(!out.interaction_ended, "only on the release frame");
        assert!(capture.is_free());
    }

    #[test]
    fn pressing_the_body_does_not_move_the_window() {
        let window = Window::new(ID, "Tools");
        let mut state = WindowState::new(Rect::new(100.0, 100.0, 200.0, 150.0));
        let mut capture = DragCapture::new();
        let (out, _) = run(&window, &mut state, &mut capture, &press(150.0, 200.0));
        assert!(!out.dragging);
        run(
            &window,
            &mut state,
            &mut capture,
            &hold(170.0, 220.0, [20.0, 20.0]),
        );
        assert_eq!(state.rect.x, 100.0);
    }

    #[test]
    fn clicking_the_close_key_closes_without_dragging() {
        let window = Window::new(ID, "Tools");
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);
        let mut state = WindowState::new(rect);
        let mut capture = DragCapture::new();
        let key = Window::close_rect(rect, title_height(), border());
        let (cx, cy) = (key.x + key.width * 0.5, key.y + key.height * 0.5);
        let (out, _) = run(&window, &mut state, &mut capture, &press(cx, cy));
        assert!(out.close_clicked);
        assert!(!out.dragging);
        assert!(capture.is_free(), "the close key never claims the drag");

        let no_close = Window::new(ID, "Tools").closable(false);
        let (out, _) = run(&no_close, &mut state, &mut capture, &press(cx, cy));
        assert!(!out.close_clicked);
        assert!(out.dragging, "without a close key the whole strip drags");
    }

    #[test]
    fn a_consumed_pointer_is_ignored() {
        let window = Window::new(ID, "Tools").resizable(true);
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);
        let mut state = WindowState::new(rect);
        let mut capture = DragCapture::new();
        let key = Window::close_rect(rect, title_height(), border());
        for (x, y) in [
            (150.0, 110.0),
            (key.x + 2.0, key.y + 2.0),
            (rect.right() - 4.0, rect.bottom() - 4.0),
        ] {
            let mut input = press(x, y);
            input.mouse_consumed = true;
            let (out, _) = run(&window, &mut state, &mut capture, &input);
            assert!(!out.close_clicked && !out.dragging && !out.resizing);
            assert!(capture.is_free());
        }
    }

    #[test]
    fn corner_grip_resizes_and_respects_the_minimum() {
        let window = Window::new(ID, "Tools")
            .resizable(true)
            .min_size([150.0, 120.0]);
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);
        let mut state = WindowState::new(rect);
        let mut capture = DragCapture::new();
        let grip = Window::grip_rect(rect, border());
        let (gx, gy) = (grip.x + 5.0, grip.y + 5.0);
        let (out, _) = run(&window, &mut state, &mut capture, &press(gx, gy));
        assert!(out.resizing && !out.dragging);
        assert!(capture.is_active(window.resize_id()));

        run(
            &window,
            &mut state,
            &mut capture,
            &hold(gx + 30.0, gy + 10.0, [30.0, 10.0]),
        );
        assert_eq!(state.rect, Rect::new(100.0, 100.0, 230.0, 160.0));

        run(
            &window,
            &mut state,
            &mut capture,
            &hold(gx - 500.0, gy - 500.0, [-530.0, -510.0]),
        );
        assert_eq!(state.rect, Rect::new(100.0, 100.0, 150.0, 120.0));
        let (out, _) = run(&window, &mut state, &mut capture, &release(gx, gy));
        assert!(out.interaction_ended);
    }

    #[test]
    fn fixed_size_windows_have_no_grip() {
        let window = Window::new(ID, "Tools");
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);
        let mut state = WindowState::new(rect);
        let mut capture = DragCapture::new();
        let grip = Window::grip_rect(rect, border());
        let (gx, gy) = (grip.x + 5.0, grip.y + 5.0);
        let (out, _) = run(&window, &mut state, &mut capture, &press(gx, gy));
        assert!(!out.resizing);
        run(
            &window,
            &mut state,
            &mut capture,
            &hold(gx + 30.0, gy + 10.0, [30.0, 10.0]),
        );
        assert_eq!(state.rect, rect);

        let mut small = WindowState::new(rect);
        let big_min = Window::new(ID, "Tools").min_size([500.0, 500.0]);
        run(
            &big_min,
            &mut small,
            &mut DragCapture::new(),
            &InputState::default(),
        );
        assert_eq!(
            small.rect, rect,
            "the minimum only applies to resizable windows"
        );
    }

    #[test]
    fn bounds_keep_the_window_inside_while_moving_and_resizing() {
        let bounds = Rect::new(0.0, 20.0, 400.0, 300.0);
        let window = Window::new(ID, "Tools").resizable(true).bounds(bounds);
        let mut state = WindowState::new(Rect::new(100.0, 100.0, 200.0, 150.0));
        let mut capture = DragCapture::new();
        run(&window, &mut state, &mut capture, &press(150.0, 110.0));
        run(
            &window,
            &mut state,
            &mut capture,
            &hold(900.0, 900.0, [800.0, 800.0]),
        );
        assert_eq!(state.rect, Rect::new(200.0, 170.0, 200.0, 150.0));
        run(&window, &mut state, &mut capture, &release(0.0, 0.0));

        let grip = Window::grip_rect(state.rect, border());
        run(
            &window,
            &mut state,
            &mut capture,
            &press(grip.x + 5.0, grip.y + 5.0),
        );
        run(
            &window,
            &mut state,
            &mut capture,
            &hold(900.0, 900.0, [300.0, 300.0]),
        );
        assert_eq!(
            state.rect,
            Rect::new(200.0, 170.0, 200.0, 150.0),
            "resizing stops at the right/bottom edge instead of pushing the window"
        );

        let mut huge = WindowState::new(Rect::new(-50.0, -50.0, 1000.0, 1000.0));
        run(
            &window,
            &mut huge,
            &mut DragCapture::new(),
            &InputState::default(),
        );
        assert_eq!(
            huge.rect, bounds,
            "an oversized window is shrunk into the bounds"
        );
    }

    #[test]
    fn paints_title_close_icon_and_grip() {
        let window = Window::new(ID, "Inspector").resizable(true);
        let mut state = WindowState::new(Rect::new(10.0, 10.0, 240.0, 160.0));
        let (_, list) = run(
            &window,
            &mut state,
            &mut DragCapture::new(),
            &InputState::default(),
        );
        assert_eq!(list.texts[0].content, "Inspector");
        #[cfg(not(feature = "phosphor-icons"))]
        assert_eq!(list.texts[1].content, "✕", "text close glyph without icons");
        #[cfg(feature = "phosphor-icons")]
        assert_eq!(list.texts.len(), 1, "only the title is text");
        assert!(
            list.chrome_instance_count() > 0,
            "surface and header painted"
        );
        #[cfg(feature = "phosphor-icons")]
        assert_eq!(list.icons_msdf.len(), 1, "one close icon");
    }

    #[test]
    fn a_typed_window_override_reaches_the_painter() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.window;
        chrome.header.background = crate::Background::Solid([0.21, 0.22, 0.23, 1.0]);
        chrome.close_hover.background = crate::Background::Solid([0.31, 0.32, 0.33, 1.0]);
        chrome.shadow.color = [0.41, 0.42, 0.43, 1.0];
        chrome.grip_colors[0] = [0.51, 0.52, 0.53, 1.0];
        let mut overlay = StyleOverlay::new();
        overlay.set_window(chrome);

        let rect = Rect::new(0.0, 0.0, 200.0, 120.0);
        let key = Window::close_rect(rect, theme.window_title_height, border());
        let input = InputState {
            mouse_x: key.x + 2.0,
            mouse_y: key.y + 2.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0)
                .with_style(&overlay);
            let mut state = WindowState::new(rect);
            Window::new(ID, "T").resizable(true).draw(
                &mut state,
                &mut DragCapture::new(),
                &mut ctx,
            );
        }
        assert!(
            list.chrome_instances()
                .any(|q| q.bg == [0.21, 0.22, 0.23, 1.0])
        );
        assert!(
            list.chrome_instances()
                .any(|q| q.bg == [0.31, 0.32, 0.33, 1.0])
        );
        assert!(
            list.shadow_instances()
                .any(|shadow| shadow.color == [0.41, 0.42, 0.43, 1.0])
        );
        assert!(
            list.chrome_instances()
                .any(|q| q.bg == [0.51, 0.52, 0.53, 1.0])
        );
    }
}
