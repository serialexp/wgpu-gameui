//! Toast notifications: floating notices stacked in a screen corner, after
//! Forge's `feedback/Toast` (a tone pip, a title over a body, a ghost close
//! key).
//!
//! [`ToastStack`] is caller-owned and persists across frames:
//! [`push`](ToastStack::push) a [`Toast`] when something happens,
//! [`tick`](ToastStack::tick) once per frame with the frame `dt` (which ages
//! timed toasts and drops expired ones; [`UiContext`](crate::UiContext)'s
//! state does this in `begin_frame`), and [`draw`](ToastStack::draw) last so
//! the stack sits above the rest of the UI.
//!
//! A toast lasts [`DEFAULT_TTL`] seconds unless given another time, or stays
//! until its close key is clicked ([`Toast::until_dismissed`], meant for
//! errors). A toast with a [`key`](Toast::with_key) replaces the shown toast
//! with the same key instead of stacking a copy, so a failure that repeats
//! shows once.
//!
//! Clicks on a toast should not reach the widgets under it: push its layer
//! with [`push_layer`](ToastStack::push_layer) before resolving the base
//! input, then draw into that layer with the layer's input.
//!
//! ```ignore
//! // Persist across frames.
//! let mut toasts = ToastStack::new().with_corner(Corner::TopRight);
//!
//! // On some event:
//! toasts.push(Toast::error("Connection refused").with_title("Sessions")
//!     .with_key("sessions").until_dismissed());
//!
//! // Each frame, before resolving the base layer's input (`area`: where
//! // toasts may go, e.g. the screen below the menu bar):
//! let layer = toasts.push_layer(&mut layers, area, &styles);
//! // ... base UI ...
//! if let Some(index) = layer {
//!     let input = layers.input_for_layer(index, &input);
//!     let list = &mut layers.layers_mut()[index].list;
//!     let mut ctx = DrawContext::new(list, &mut focus, &theme, &input, screen_w, screen_h);
//!     toasts.draw(area, &mut ctx); // returns the toast whose close key was clicked
//! }
//! ```

use crate::chrome::SurfacePainter;
use crate::layer::LayerStack;
use crate::layout::Rect;
use crate::style::{Ink, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::material::Tone;
use super::{DrawContext, DrawList, Severity};

/// Screen corner a [`ToastStack`] anchors to. The newest toast sits nearest the
/// corner; older toasts stack away from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    /// Top-right (default): newest at the top, stack downward.
    TopRight,
    /// Top-left: newest at the top, stack downward.
    TopLeft,
    /// Bottom-right: newest at the bottom, stack upward.
    BottomRight,
    /// Bottom-left: newest at the bottom, stack upward.
    BottomLeft,
}

impl Corner {
    fn is_right(self) -> bool {
        matches!(self, Corner::TopRight | Corner::BottomRight)
    }
    fn is_top(self) -> bool {
        matches!(self, Corner::TopRight | Corner::TopLeft)
    }
}

/// Default seconds a timed toast is shown.
pub const DEFAULT_TTL: f32 = 4.0;

/// Padding inside a toast (Forge: `9px 10px`).
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 9.0;
/// The tone pip's width.
const PIP_W: f32 = 3.0;
/// Space between the pip, the text and the close key.
const GAP: f32 = 9.0;
/// Space between the title and the body.
const LINE_GAP: f32 = 2.0;
/// The body's line height, as a multiple of its size.
const BODY_LEADING: f32 = 1.45;
/// The close key's side (a header-size key).
const CLOSE: f32 = 17.0;

/// A notification to push onto a [`ToastStack`].
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    severity: Severity,
    title: Option<String>,
    message: String,
    /// Seconds before it goes by itself; `None` stays until dismissed.
    ttl: Option<f32>,
    key: Option<String>,
}

impl Toast {
    /// A toast with an explicit severity, shown for [`DEFAULT_TTL`] seconds.
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            title: None,
            message: message.into(),
            ttl: Some(DEFAULT_TTL),
            key: None,
        }
    }

    /// Info toast.
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(Severity::Info, message)
    }
    /// Success toast.
    pub fn success(message: impl Into<String>) -> Self {
        Self::new(Severity::Success, message)
    }
    /// Warning toast.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }
    /// Error toast.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// A title line above the message. Without one, the message is the
    /// title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Show for `ttl` seconds.
    pub fn with_ttl(mut self, ttl: f32) -> Self {
        self.ttl = Some(ttl.max(0.0));
        self
    }

    /// Stay until the close key is clicked.
    pub fn until_dismissed(mut self) -> Self {
        self.ttl = None;
        self
    }

    /// Pushing a toast with this key replaces a shown toast with the same
    /// key (the new one goes to the front, its time starts over) instead of
    /// adding another.
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The severity.
    pub fn severity(&self) -> Severity {
        self.severity
    }
    /// The title, if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    /// The message.
    pub fn message(&self) -> &str {
        &self.message
    }
    /// The key, if any.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// The emphasised first line and the optional body under it.
    fn lines(&self) -> (&str, Option<&str>) {
        match &self.title {
            Some(title) => (title, Some(&self.message)),
            None => (&self.message, None),
        }
    }
}

/// A toast plus its elapsed lifetime.
#[derive(Debug)]
struct Active {
    toast: Toast,
    elapsed: f32,
}

impl Active {
    fn expired(&self) -> bool {
        self.toast.ttl.is_some_and(|ttl| self.elapsed >= ttl)
    }
}

/// Alpha for a toast given its elapsed time, ttl and fade duration: full opacity
/// until the last `fade` seconds, then a linear ramp to 0 at expiry.
fn fade_alpha(elapsed: f32, ttl: Option<f32>, fade: f32) -> f32 {
    let Some(ttl) = ttl else {
        return 1.0;
    };
    if fade <= 0.0 {
        return 1.0;
    }
    ((ttl - elapsed) / fade).clamp(0.0, 1.0)
}

/// Width available to a toast's text.
fn text_width(width: f32) -> f32 {
    (width - 2.0 * PAD_X - PIP_W - 2.0 * GAP - CLOSE).max(0.0)
}

fn lead_block(s: &StyleResolver, text: &str, x: f32, y: f32, width: f32) -> TextBlock {
    s.sans_block(text, x, y, TextSize::Menu, Ink::Emph)
        .with_max_width(width)
}

fn body_block(s: &StyleResolver, text: &str, x: f32, y: f32, width: f32) -> TextBlock {
    let size = s.text_size(TextSize::Row);
    s.sans_block(text, x, y, TextSize::Row, Ink::Glyph)
        .with_line_height(size * BODY_LEADING)
        .with_max_width(width)
}

/// The height of the text column and of the first line in it.
fn text_heights(toast: &Toast, list: &mut DrawList, s: &StyleResolver, width: f32) -> (f32, f32) {
    let text_w = text_width(width);
    let (lead, body) = toast.lines();
    let lead_h = list.measure_block(&lead_block(s, lead, 0.0, 0.0, text_w)).1;
    let body_h = body.map_or(0.0, |body| {
        LINE_GAP + list.measure_block(&body_block(s, body, 0.0, 0.0, text_w)).1
    });
    (lead_h + body_h, lead_h)
}

/// A caller-owned stack of toast notifications.
pub struct ToastStack {
    active: Vec<Active>,
    corner: Corner,
    width: f32,
    gap: f32,
    margin: f32,
    fade: f32,
    max: usize,
    /// This frame's visible toasts: an index into `active` and its rect,
    /// newest first. Rebuilt by `place`.
    placed: Vec<(usize, Rect)>,
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastStack {
    /// A new, empty stack (top-right corner, sensible defaults).
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            corner: Corner::TopRight,
            width: 300.0,
            gap: 8.0,
            margin: 16.0,
            // Forge: no fades; `with_fade` opts in.
            fade: 0.0,
            max: 4,
            placed: Vec::new(),
        }
    }

    /// Anchor corner (default [`Corner::TopRight`]).
    pub fn with_corner(mut self, corner: Corner) -> Self {
        self.corner = corner;
        self
    }
    /// Toast width in px (default 300).
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width.max(1.0);
        self
    }
    /// Max simultaneously *visible* toasts (default 4). Extra (older) toasts stay
    /// queued and appear as visible ones go.
    pub fn with_max(mut self, max: usize) -> Self {
        self.max = max;
        self
    }
    /// Fade-out duration in seconds before a timed toast expires (default 0:
    /// it disappears at once when its time runs out).
    pub fn with_fade(mut self, fade: f32) -> Self {
        self.fade = fade.max(0.0);
        self
    }
    /// Gap between stacked toasts in px (default 8).
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }
    /// Margin from the area's edges in px (default 16).
    pub fn with_margin(mut self, margin: f32) -> Self {
        self.margin = margin.max(0.0);
        self
    }

    /// Show a toast. One with the [`key`](Toast::with_key) of a shown toast
    /// replaces it.
    pub fn push(&mut self, toast: Toast) {
        if let Some(key) = toast.key() {
            self.active.retain(|active| active.toast.key() != Some(key));
        }
        self.active.push(Active {
            toast,
            elapsed: 0.0,
        });
    }

    /// Remove the toast with `key`, as its close key would. Whether one was
    /// shown.
    pub fn dismiss(&mut self, key: &str) -> bool {
        let before = self.active.len();
        self.active.retain(|active| active.toast.key() != Some(key));
        self.active.len() != before
    }

    /// Age all toasts by `dt` seconds and drop timed ones that have run out.
    pub fn tick(&mut self, dt: f32) {
        for a in &mut self.active {
            a.elapsed += dt;
        }
        self.active.retain(|a| !a.expired());
    }

    /// After this frame's draws: seconds until the next timed change (a
    /// toast entering its fade window or expiring), or `None` when no timed
    /// toast is shown. Toasts that wait to be dismissed never need a frame
    /// of their own. The list is small, so the scan is cheap.
    pub fn pending(&self) -> Option<f32> {
        self.active
            .iter()
            .filter_map(|a| {
                let ttl = a.toast.ttl?;
                // Next visible change for this toast: entering the fade
                // window (when there is one), otherwise expiry.
                let until = if self.fade > 0.0 && ttl > self.fade {
                    ttl - self.fade
                } else {
                    ttl
                };
                Some((until - a.elapsed).max(0.0))
            })
            .reduce(f32::min)
    }

    /// Remove every toast.
    pub fn clear(&mut self) {
        self.active.clear();
    }

    /// Whether there are no toasts.
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Number of toasts (queued + visible).
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// The toasts, oldest first.
    pub fn toasts(&self) -> impl Iterator<Item = &Toast> {
        self.active.iter().map(|a| &a.toast)
    }

    /// The toasts shown by the last [`push_layer`](Self::push_layer) or
    /// [`draw`](Self::draw), newest first, with their rects.
    pub fn placed(&self) -> impl Iterator<Item = (&Toast, Rect)> {
        self.placed
            .iter()
            .filter_map(|&(index, rect)| Some((&self.active.get(index)?.toast, rect)))
    }

    /// Where the close key sits on a toast drawn in `toast`.
    pub fn close_rect(toast: Rect) -> Rect {
        Rect::new(
            toast.x + toast.width - PAD_X - CLOSE,
            toast.y + PAD_Y,
            CLOSE,
            CLOSE,
        )
    }

    /// Lay out the visible toasts (the newest `max`, newest nearest the
    /// corner) in the corner of `area`.
    fn place(&mut self, area: Rect, list: &mut DrawList, s: &StyleResolver) {
        self.placed.clear();
        if self.max == 0 {
            return;
        }
        let x = if self.corner.is_right() {
            area.right() - self.margin - self.width
        } else {
            area.x + self.margin
        };
        let mut top_edge = area.y + self.margin;
        let mut bottom_edge = area.bottom() - self.margin;
        let start = self.active.len().saturating_sub(self.max);
        for index in (start..self.active.len()).rev() {
            let (text_h, _) = text_heights(&self.active[index].toast, list, s, self.width);
            let h = 2.0 * PAD_Y + text_h.max(CLOSE);
            let y = if self.corner.is_top() {
                let y = top_edge;
                top_edge += h + self.gap;
                y
            } else {
                bottom_edge -= h;
                let y = bottom_edge;
                bottom_edge -= self.gap;
                y
            };
            self.placed.push((index, Rect::new(x, y, self.width, h)));
        }
    }

    /// Push the popup layer the toasts are drawn into, so the widgets under
    /// them don't get their clicks. Call before resolving the base layer's
    /// input; `None` when no toast is shown. `area` is where toasts may go,
    /// as for [`draw`](Self::draw).
    pub fn push_layer(
        &mut self,
        layers: &mut LayerStack,
        area: Rect,
        s: &StyleResolver,
    ) -> Option<usize> {
        self.place(area, layers.base_mut(), s);
        let bounds = self
            .placed
            .iter()
            .map(|&(_, rect)| rect)
            .reduce(|a, b| a.union(b))?;
        let index = layers.push_popup(bounds);
        layers.pop_layer();
        Some(index)
    }

    /// Draw the visible toasts in the corner of `area` (usually the screen,
    /// or the part of it below a menu bar) and handle their close keys. Call
    /// last, so they sit above the rest of the UI. Returns the toast whose
    /// close key was clicked; it is gone from the stack.
    pub fn draw(&mut self, area: Rect, ctx: &mut DrawContext) -> Option<Toast> {
        let s = ctx.styles();
        self.place(area, ctx.draw_list, &s);
        let mut dismissed = None;
        for i in 0..self.placed.len() {
            let (index, rect) = self.placed[i];
            if self.draw_one(&self.active[index], rect, ctx, &s) {
                dismissed = Some(index);
            }
        }
        dismissed.map(|index| self.active.remove(index).toast)
    }

    /// Draw one toast; whether its close key was clicked.
    fn draw_one(&self, a: &Active, rect: Rect, ctx: &mut DrawContext, s: &StyleResolver) -> bool {
        let chrome = s.toast();
        // Scoped per toast rather than per stack: the stack has no
        // allocation of its own, each toast does. The declared area takes in
        // the drop shadow.
        ctx.draw_list
            .push_debug_scope_rect("Toast", rect.union(chrome.shadow.ink_rect(rect)));
        // The tint is set before every instance so the shadow, surface,
        // content, border and key fade together.
        let alpha = fade_alpha(a.elapsed, a.toast.ttl, self.fade);
        ctx.draw_list.push_tint();
        ctx.draw_list.multiply_tint([1.0, 1.0, 1.0, alpha]);

        let padding_box = rect.inset(chrome.surface.border_widths.left);
        let mut surface = SurfacePainter::new(
            ctx.draw_list,
            rect,
            padding_box,
            chrome.surface.corner_radii,
            chrome.surface,
            std::slice::from_ref(&chrome.shadow),
            &chrome.lines,
        );
        surface.paint_pre_content();
        {
            let list = surface.draw_list();
            let (text_h, lead_h) = text_heights(&a.toast, list, s, rect.width);
            let top = rect.y + PAD_Y;
            // The pip runs beside the text, a pixel below its top.
            list.quad(
                rect.x + PAD_X,
                top + 1.0,
                PIP_W,
                (text_h - 1.0).max(1.0),
                s.color(a.toast.severity.style_key()),
            );
            let text_x = rect.x + PAD_X + PIP_W + GAP;
            let text_w = text_width(rect.width);
            let (lead, body) = a.toast.lines();
            list.text(lead_block(s, lead, text_x, top, text_w));
            if let Some(body) = body {
                list.text(body_block(s, body, text_x, top + lead_h + LINE_GAP, text_w));
            }
        }
        surface.paint_post_content();

        let closed = close_key(Self::close_rect(rect), ctx);

        ctx.draw_list.pop_tint();
        ctx.draw_list.pop_debug_scope();
        closed
    }
}

/// The ghost × key; whether it was clicked.
fn close_key(rect: Rect, ctx: &mut DrawContext) -> bool {
    #[cfg(feature = "phosphor-icons")]
    let key = super::IconKey::new(crate::render::PhosphorIcon::X, CLOSE).tone(Tone::Ghost);
    #[cfg(not(feature = "phosphor-icons"))]
    let key = super::Button::new("×").tone(Tone::Ghost);
    #[cfg(feature = "phosphor-icons")]
    let response = key.draw(rect, ctx);
    #[cfg(not(feature = "phosphor-icons"))]
    let response = key.draw_response(rect, ctx);
    response.clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    const W: f32 = 800.0;
    const H: f32 = 600.0;
    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: W,
        height: H,
    };

    fn away() -> InputState {
        InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..Default::default()
        }
    }

    fn click_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..Default::default()
        }
    }

    /// Draw `stack` into a fresh list under `input`.
    fn draw(stack: &mut ToastStack, input: &InputState) -> (DrawList, Option<Toast>) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, W, H);
        let dismissed = stack.draw(SCREEN, &mut ctx);
        (list, dismissed)
    }

    /// The toast surfaces drawn, by their width.
    fn surfaces(list: &DrawList, width: f32) -> Vec<[f32; 4]> {
        list.chrome_instances()
            .filter(|c| c.rect[2] == width && c.border[3] > 0.0)
            .map(|c| c.rect)
            .collect()
    }

    #[test]
    fn a_timed_toast_goes_and_an_error_waits_to_be_dismissed() {
        let mut s = ToastStack::new();
        s.push(Toast::info("hi").with_ttl(1.0));
        s.push(Toast::error("broken").until_dismissed());
        s.tick(0.5);
        assert_eq!(s.len(), 2, "still alive at 0.5s");
        s.tick(0.6);
        assert_eq!(s.len(), 1, "the timed one expired past 1.0s");
        s.tick(10_000.0);
        assert_eq!(s.toasts().next().unwrap().message(), "broken");
    }

    #[test]
    fn pending_counts_only_timed_toasts() {
        let mut s = ToastStack::new();
        s.push(Toast::error("broken").until_dismissed());
        assert_eq!(s.pending(), None, "nothing will change by itself");
        s.push(Toast::info("hi").with_ttl(2.0));
        s.tick(0.5);
        assert_eq!(s.pending(), Some(1.5));
    }

    #[test]
    fn a_keyed_toast_replaces_the_one_shown() {
        let mut s = ToastStack::new();
        s.push(Toast::error("first").with_key("sessions").with_ttl(1.0));
        s.push(Toast::info("other"));
        s.tick(0.9);
        s.push(Toast::error("second").with_key("sessions").with_ttl(1.0));
        let shown: Vec<_> = s.toasts().map(Toast::message).collect();
        assert_eq!(shown, ["other", "second"], "moved to the front");
        s.tick(0.5);
        assert_eq!(s.len(), 2, "its time started over");
        assert!(s.dismiss("sessions"));
        assert!(!s.dismiss("sessions"));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn fade_alpha_ramps_in_final_window() {
        assert_eq!(fade_alpha(0.0, Some(4.0), 0.4), 1.0);
        assert_eq!(
            fade_alpha(3.5, Some(4.0), 0.4),
            1.0,
            "before the fade window"
        );
        assert!((fade_alpha(3.8, Some(4.0), 0.4) - 0.5).abs() < 1e-4);
        assert_eq!(fade_alpha(4.0, Some(4.0), 0.4), 0.0);
        assert_eq!(fade_alpha(3.99, Some(4.0), 0.0), 1.0, "no fade");
        assert_eq!(fade_alpha(1e6, None, 0.4), 1.0, "never fades");
    }

    #[test]
    fn draw_caps_visible_at_max() {
        let mut s = ToastStack::new().with_max(2);
        for i in 0..3 {
            s.push(Toast::info(format!("toast {i}")));
        }
        let (list, _) = draw(&mut s, &away());
        assert_eq!(s.len(), 3, "all three stay queued");
        assert_eq!(list.shadow_instance_count(), 2, "two toasts drawn");
    }

    #[test]
    fn corners_place_at_their_edges() {
        let mut right = ToastStack::new().with_width(300.0).with_margin(16.0);
        right.push(Toast::info("x"));
        let rect = surfaces(&draw(&mut right, &away()).0, 300.0)[0];
        assert!((rect[0] - (W - 16.0 - 300.0)).abs() < 1e-3, "right");
        assert!((rect[1] - 16.0).abs() < 1e-3, "top");

        let mut left = ToastStack::new()
            .with_width(300.0)
            .with_margin(16.0)
            .with_corner(Corner::BottomLeft);
        left.push(Toast::info("x"));
        let rect = surfaces(&draw(&mut left, &away()).0, 300.0)[0];
        assert!((rect[0] - 16.0).abs() < 1e-3, "left");
        assert!((rect[1] + rect[3] - (H - 16.0)).abs() < 1e-3, "bottom");
    }

    #[test]
    fn toasts_go_in_the_corner_of_their_area() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut s = ToastStack::new().with_width(300.0).with_margin(16.0);
        s.push(Toast::info("x"));
        // Below a 24px menu bar, left of a 200px side panel.
        s.place(
            Rect::new(0.0, 24.0, 600.0, 576.0),
            &mut DrawList::new(),
            &styles,
        );
        let rect = s.placed[0].1;
        assert_eq!((rect.x, rect.y), (600.0 - 16.0 - 300.0, 24.0 + 16.0));
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn newer_toasts_sit_nearest_the_corner() {
        let mut s = ToastStack::new();
        s.push(Toast::info("old"));
        s.push(Toast::info("new"));
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        s.place(SCREEN, &mut DrawList::new(), &styles);
        let newest = s.placed[0];
        assert_eq!(newest.0, 1);
        assert!(newest.1.y < s.placed[1].1.y);
    }

    #[test]
    fn a_body_makes_the_toast_taller_and_long_text_wraps() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let height = |toast: Toast| {
            let mut s = ToastStack::new();
            s.push(toast);
            s.place(SCREEN, &mut DrawList::new(), &styles);
            s.placed[0].1.height
        };
        let short = height(Toast::error("Sessions"));
        assert_eq!(short, 2.0 * PAD_Y + CLOSE, "at least the close key");
        let titled = height(Toast::error("connection refused").with_title("Sessions"));
        assert!(titled > short);
        let long = height(Toast::error("connection refused ".repeat(12)).with_title("Sessions"));
        assert!(long > titled, "{long} > {titled}");
    }

    #[test]
    fn the_close_key_dismisses_its_toast() {
        let mut s = ToastStack::new();
        s.push(Toast::error("first").until_dismissed());
        s.push(Toast::error("second").until_dismissed());
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        s.place(SCREEN, &mut DrawList::new(), &styles);
        // The newest ("second") is on top; click beside its close key first.
        let rect = s.placed[0].1;
        let (_, none) = draw(&mut s, &click_at(rect.x + 40.0, rect.y + PAD_Y + 4.0));
        assert_eq!(none, None, "a click on the text keeps it");
        let placed: Vec<_> = s
            .placed()
            .map(|(toast, rect)| (toast.message(), rect))
            .collect();
        assert_eq!(placed, [("second", rect), ("first", s.placed[1].1)]);
        let key = ToastStack::close_rect(rect);
        assert_eq!(
            key,
            Rect::new(rect.right() - PAD_X - CLOSE, rect.y + PAD_Y, CLOSE, CLOSE)
        );
        let (_, dismissed) = draw(
            &mut s,
            &click_at(key.x + key.width / 2.0, key.y + key.height / 2.0),
        );
        assert_eq!(dismissed.as_ref().map(Toast::message), Some("second"));
        let shown: Vec<_> = s.toasts().map(Toast::message).collect();
        assert_eq!(shown, ["first"]);
    }

    #[test]
    fn the_layer_keeps_clicks_on_a_toast_from_the_ui_under_it() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut layers = LayerStack::new();
        let mut s = ToastStack::new();
        assert_eq!(
            s.push_layer(&mut layers, SCREEN, &styles),
            None,
            "no toasts"
        );
        s.push(Toast::error("broken").until_dismissed());
        let index = s.push_layer(&mut layers, SCREEN, &styles).unwrap();
        let rect = s.placed[0].1;
        let on_toast = click_at(rect.x + 20.0, rect.y + 10.0);
        assert!(layers.input_for_base(&on_toast).mouse_consumed);
        assert!(!layers.input_for_layer(index, &on_toast).mouse_consumed);
        let elsewhere = click_at(10.0, H - 10.0);
        assert!(!layers.input_for_base(&elsewhere).mouse_consumed);
    }

    #[test]
    fn typed_overlay_and_fade_tint_reach_toast_surface_and_shadow() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.toast;
        chrome.surface.background = crate::Background::Solid([0.2, 0.3, 0.4, 1.0]);
        chrome.shadow.color = [0.5, 0.1, 0.2, 0.8];
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_toast(chrome);
        let mut stack = ToastStack::new().with_fade(0.4);
        stack.push(Toast::info("toast").with_ttl(1.0));
        stack.tick(0.8);
        let alpha = fade_alpha(0.8, Some(1.0), stack.fade);
        assert!(alpha > 0.0 && alpha < 1.0, "mid-fade: {alpha}");
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = away();
        let mut ctx =
            DrawContext::new(&mut list, &mut focus, &theme, &input, W, H).with_style(&overlay);
        stack.draw(SCREEN, &mut ctx);
        let surface = list.chrome_instances().find(|i| i.bg[0] == 0.2).unwrap();
        assert_eq!(surface.bg[3], alpha);
        assert!(
            (list.shadow_instance(0).unwrap().color[3] - chrome.shadow.color[3] * alpha).abs()
                < 1e-6
        );
    }

    #[test]
    fn empty_stack_draws_nothing() {
        let mut s = ToastStack::new();
        let (list, dismissed) = draw(&mut s, &away());
        assert_eq!(list.chrome_instance_count(), 0);
        assert_eq!(dismissed, None);
    }
}
