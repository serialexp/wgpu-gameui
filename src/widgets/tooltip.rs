//! Tooltip system - hover regions with rich content display.
//!
//! Tooltips live on the popup/tooltip layer of `LayerStack` so they always
//! draw above the rest of the UI without callers having to remember to
//! "draw last". They support a configurable hover delay (`with_delay_ms`) —
//! the tooltip only appears once the cursor has rested over the same hover
//! region for that many milliseconds.

use crate::chrome::SurfacePainter;
use crate::layer::LayerStack;
use crate::layout::Rect;
use crate::style::{Ink, TextSize};
use crate::text::TextBlock;
use crate::{InputState, StyleKey, StyleResolver};

use super::DrawList;

/// Content that can be displayed in a tooltip.
/// Designed to be extensible for future content types (images, icons, etc.)
#[derive(Clone)]
pub enum TooltipContent {
    /// Simple text tooltip with optional title.
    Text {
        /// Optional bold title shown above the body.
        title: Option<String>,
        /// Wrapped body text.
        body: String,
    },
    /// Multi-line text with optional title.
    Lines {
        /// Optional bold title shown above the lines.
        title: Option<String>,
        /// One entry per rendered line.
        lines: Vec<String>,
    },
    /// Rich content with title, description, and key-value pairs.
    Rich {
        /// Bold title.
        title: String,
        /// Wrapped description paragraph.
        description: String,
        /// Key/value detail rows shown below the description.
        details: Vec<(String, String)>,
    },
}

impl TooltipContent {
    /// Build a plain text tooltip with no title.
    pub fn text(body: impl Into<String>) -> Self {
        Self::Text {
            title: None,
            body: body.into(),
        }
    }

    /// Build a text tooltip with a title above the body.
    pub fn text_with_title(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Text {
            title: Some(title.into()),
            body: body.into(),
        }
    }

    /// Build a multi-line tooltip with no title.
    pub fn lines(lines: Vec<String>) -> Self {
        Self::Lines { title: None, lines }
    }

    /// Build a multi-line tooltip with a title above the lines.
    pub fn lines_with_title(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self::Lines {
            title: Some(title.into()),
            lines,
        }
    }

    /// Build a rich tooltip with a title and description; add detail rows with
    /// [`TooltipContent::with_detail`].
    pub fn rich(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::Rich {
            title: title.into(),
            description: description.into(),
            details: Vec::new(),
        }
    }

    /// Append a key/value detail row. No-op unless this is a [`TooltipContent::Rich`].
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if let Self::Rich {
            ref mut details, ..
        } = self
        {
            details.push((key.into(), value.into()));
        }
        self
    }
}

// ---------------------------------------------------------------------------
// Anchored hint (Forge `Tooltip`)
// ---------------------------------------------------------------------------

/// Gap between an anchored hint and the widget it describes (`calc(100% + 7px)`).
const HINT_OFFSET: f32 = 7.0;
/// Horizontal padding inside an anchored hint.
const HINT_PAD_X: f32 = 7.0;
/// Vertical padding inside an anchored hint.
const HINT_PAD_Y: f32 = 3.0;
/// Gap between an anchored hint's label and its shortcut.
const HINT_SHORTCUT_GAP: f32 = 7.0;

/// Which side of its anchor an anchored [`TooltipHint`] prefers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TooltipSide {
    /// Above the anchor, centred horizontally.
    #[default]
    Top,
    /// Below the anchor, centred horizontally.
    Bottom,
    /// Left of the anchor, centred vertically.
    Left,
    /// Right of the anchor, centred vertically.
    Right,
}

impl TooltipSide {
    /// The side across the anchor from this one.
    pub fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// A compact label pinned beside the widget it describes, with an optional
/// mono shortcut — the Forge `Tooltip`.
///
/// Unlike [`TooltipLayer`] (cursor-following, delayed, rich content), a hint
/// has no delay and no fade and sits [`HINT_OFFSET`] pixels off one side of
/// its anchor. The widget that owns the anchor decides *when* to show it and
/// *which side* (a toolbar puts it on the strip's outward side); this type only
/// measures, places, and paints. If the preferred side would leave the
/// viewport and the opposite side fits, it flips; it is then clamped into the
/// viewport along both axes.
///
/// ```ignore
/// TooltipHint::new("Move")
///     .shortcut("W")
///     .side(TooltipSide::Right)
///     .draw_into_layers(&mut layers, &styles, key_rect, viewport);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipHint<'a> {
    label: &'a str,
    shortcut: Option<&'a str>,
    side: TooltipSide,
}

/// The measured blocks and border box of one hint, so layout and paint share a
/// single measurement pass.
struct HintLayout {
    rect: Rect,
    label: TextBlock,
    shortcut: Option<TextBlock>,
}

impl<'a> TooltipHint<'a> {
    /// A hint showing `label` above its anchor, with no shortcut.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            shortcut: None,
            side: TooltipSide::Top,
        }
    }

    /// Show `shortcut` after the label in the mono tip-hint ink. An empty
    /// string means no shortcut.
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = (!shortcut.is_empty()).then_some(shortcut);
        self
    }

    /// Prefer `side` of the anchor.
    pub fn side(mut self, side: TooltipSide) -> Self {
        self.side = side;
        self
    }

    /// The border box this hint would occupy beside `anchor` within `viewport`.
    pub fn layout(
        &self,
        list: &mut DrawList,
        style: &StyleResolver,
        anchor: Rect,
        viewport: Rect,
    ) -> Rect {
        self.measure(list, style, anchor, viewport).rect
    }

    /// Paint the hint beside `anchor` into `list` and return its border box.
    /// `list` should be one drawn above everything the hint may overlap —
    /// prefer [`draw_into_layers`](Self::draw_into_layers).
    pub fn draw(
        &self,
        list: &mut DrawList,
        style: &StyleResolver,
        anchor: Rect,
        viewport: Rect,
    ) -> Rect {
        let layout = self.measure(list, style, anchor, viewport);
        let rect = layout.rect;
        let chrome = style.tooltip();
        let shadow_margin = chrome.shadow.blur * 1.5;
        list.push_debug_scope_rect(
            "Tooltip hint",
            Rect::new(
                rect.x - shadow_margin,
                rect.y - shadow_margin + chrome.shadow.offset[1],
                rect.width + shadow_margin * 2.0,
                rect.height + shadow_margin * 2.0,
            ),
        );
        let padding_box = rect.inset(chrome.surface.border_widths.left);
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
        {
            let list = surface.draw_list();
            list.text(layout.label);
            if let Some(shortcut) = layout.shortcut {
                list.text(shortcut);
            }
        }
        surface.paint_post_content();
        list.pop_debug_scope();
        rect
    }

    /// Paint the hint onto a fresh tooltip layer of `layers` (purely visual; it
    /// never blocks input) and return its border box.
    pub fn draw_into_layers(
        &self,
        layers: &mut LayerStack,
        style: &StyleResolver,
        anchor: Rect,
        viewport: Rect,
    ) -> Rect {
        let index = layers.push_tooltip(viewport);
        let rect = self.draw(layers.current_mut(), style, anchor, viewport);
        layers.layers_mut()[index].rect = rect;
        layers.pop_layer();
        rect
    }

    fn measure(
        &self,
        list: &mut DrawList,
        style: &StyleResolver,
        anchor: Rect,
        viewport: Rect,
    ) -> HintLayout {
        let border = style.tooltip().surface.border_widths;
        let label_size = style.text_size(TextSize::Row);
        let meta_size = style.text_size(TextSize::Meta);
        let theme = style.theme();

        let mut label = style.sans_block(self.label, 0.0, 0.0, TextSize::Row, Ink::Emph);
        let (label_w, label_h) = list.measure_block(&label);
        let mut shortcut = self
            .shortcut
            .map(|s| style.mono_block(s, 0.0, 0.0, TextSize::Meta, Ink::TipHint));
        let (short_w, short_h) = match &shortcut {
            Some(block) => list.measure_block(block),
            None => (0.0, 0.0),
        };

        let content_w = label_w
            + if shortcut.is_some() {
                HINT_SHORTCUT_GAP + short_w
            } else {
                0.0
            };
        let width = border.left + border.right + HINT_PAD_X * 2.0 + content_w;
        let height = border.top + border.bottom + HINT_PAD_Y * 2.0 + label_h.max(short_h);
        let rect = place_hint(anchor, self.side, width, height, viewport);

        // Label and shortcut share one baseline, set by the label's x-height
        // band centred in the hint (see `ListView` rows for the same recipe).
        let cy = rect.y + border.top + (rect.height - border.top - border.bottom) * 0.5;
        let label_font = theme.font.as_ref();
        label.x = rect.x + border.left + HINT_PAD_X;
        label.y = list.x_centered_text_y(cy, label_size, label_font);
        if let Some(block) = &mut shortcut {
            let baseline = cy + label_size * list.font_vmetrics(label_font).x_ratio * 0.5;
            let mono_baseline =
                meta_size * list.font_vmetrics(theme.mono_font.as_ref()).baseline_ratio;
            block.x = label.x + label_w + HINT_SHORTCUT_GAP;
            block.y = baseline - mono_baseline;
        }
        HintLayout {
            rect,
            label,
            shortcut,
        }
    }
}

/// Place a `width`×`height` hint on `side` of `anchor`, flipping to the
/// opposite side when only that one fits, then clamping into `viewport`.
fn place_hint(anchor: Rect, side: TooltipSide, width: f32, height: f32, viewport: Rect) -> Rect {
    let at = |side: TooltipSide| -> Rect {
        let cx = anchor.x + anchor.width * 0.5 - width * 0.5;
        let cy = anchor.y + anchor.height * 0.5 - height * 0.5;
        match side {
            TooltipSide::Top => Rect::new(cx, anchor.y - HINT_OFFSET - height, width, height),
            TooltipSide::Bottom => Rect::new(cx, anchor.bottom() + HINT_OFFSET, width, height),
            TooltipSide::Left => Rect::new(anchor.x - HINT_OFFSET - width, cy, width, height),
            TooltipSide::Right => Rect::new(anchor.right() + HINT_OFFSET, cy, width, height),
        }
    };
    let fits_main = |rect: Rect, side: TooltipSide| match side {
        TooltipSide::Top | TooltipSide::Bottom => {
            rect.y >= viewport.y && rect.bottom() <= viewport.bottom()
        }
        TooltipSide::Left | TooltipSide::Right => {
            rect.x >= viewport.x && rect.right() <= viewport.right()
        }
    };
    let preferred = at(side);
    let flipped = at(side.opposite());
    let mut rect = if !fits_main(preferred, side) && fits_main(flipped, side.opposite()) {
        flipped
    } else {
        preferred
    };
    // Clamp into the viewport; when the hint is larger than the viewport the
    // leading edge wins so the label's start stays visible.
    rect.x = rect.x.min(viewport.right() - width).max(viewport.x);
    rect.y = rect.y.min(viewport.bottom() - height).max(viewport.y);
    rect
}

/// A registered hover region with its tooltip content.
struct HoverRegion {
    rect: Rect,
    content: TooltipContent,
}

/// Manages tooltip display for a frame.
///
/// Hover-delay state persists across frames; clear regions every frame with
/// [`TooltipLayer::clear`] (or use the same instance over the lifetime of
/// your UI and re-register hover regions each frame).
///
/// ```ignore
/// // Persist across frames so hover delay accumulates.
/// let mut tooltips = TooltipLayer::new().with_delay_ms(400);
///
/// // Each frame:
/// tooltips.clear();
/// tooltips.register(stat_rect, TooltipContent::rich("Strength", "Physical power..."));
/// tooltips.tick(dt_seconds, &input);
///
/// // Either route into a LayerStack popup layer, or draw directly onto a
/// // DrawList that's the last thing drawn this frame.
/// tooltips.draw_into_layers(&mut layers, &input, &style, screen_w, screen_h);
/// ```
pub struct TooltipLayer {
    regions: Vec<HoverRegion>,
    hover_delay_ms: u32,
    /// Index of the region currently hovered (for delay accumulation).
    hovered_idx: Option<usize>,
    /// Seconds the cursor has been over `hovered_idx`.
    hover_seconds: f32,
}

impl Default for TooltipLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl TooltipLayer {
    /// Create an empty tooltip layer with no hover delay.
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            hover_delay_ms: 0,
            hovered_idx: None,
            hover_seconds: 0.0,
        }
    }

    /// Set the hover delay before showing tooltips (in milliseconds).
    pub fn with_delay_ms(mut self, delay_ms: u32) -> Self {
        self.hover_delay_ms = delay_ms;
        self
    }

    /// Clear all registered regions (call at start of frame).
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Register a hover region with tooltip content.
    pub fn register(&mut self, rect: Rect, content: TooltipContent) {
        self.regions.push(HoverRegion { rect, content });
    }

    /// Advance the hover-delay timer based on the input cursor position
    /// and frame `dt` in seconds. Must be called once per frame *after*
    /// registering regions for that frame.
    pub fn tick(&mut self, dt_seconds: f32, input: &InputState) {
        let new_idx = self
            .regions
            .iter()
            .position(|r| !input.mouse_consumed && r.rect.contains(input.mouse_x, input.mouse_y));

        if new_idx != self.hovered_idx {
            // Hover target changed — reset, then count this frame as the
            // first frame of hover on the new target.
            self.hovered_idx = new_idx;
            self.hover_seconds = if new_idx.is_some() { dt_seconds } else { 0.0 };
        } else if new_idx.is_some() {
            self.hover_seconds += dt_seconds;
        }
    }

    /// Whether the hovered tooltip is currently visible (delay satisfied).
    pub fn is_visible(&self) -> bool {
        self.hovered_idx.is_some() && self.hover_seconds * 1000.0 >= self.hover_delay_ms as f32
    }

    /// After this frame's tick: pending tooltip timing. `Some(delay)` when
    /// the cursor rests on a region whose delay hasn't elapsed yet (`delay` =
    /// seconds until the tooltip appears); `None` when nothing is pending (no
    /// hover, or the tooltip is already visible).
    pub fn pending(&self) -> Option<f32> {
        self.hovered_idx?;
        let delay_s = self.hover_delay_ms as f32 / 1000.0;
        let remaining = delay_s - self.hover_seconds;
        (remaining > 0.0).then_some(remaining)
    }

    /// Render the active tooltip onto a fresh tooltip layer of `layers`.
    /// Does nothing if no tooltip is active or the delay has not elapsed.
    pub fn draw_into_layers(
        &self,
        layers: &mut LayerStack,
        input: &InputState,
        style: &StyleResolver,
        screen_width: f32,
        screen_height: f32,
    ) {
        if !self.is_visible() {
            return;
        }
        let region = match self.hovered_idx.and_then(|i| self.regions.get(i)) {
            Some(r) => r,
            None => return,
        };
        // Bound the layer rect by the on-screen tooltip area; we don't know
        // the exact size yet, so use a generous rect — tooltips never block
        // input, so this rect is purely informational.
        let bounds = Rect::new(0.0, 0.0, screen_width, screen_height);
        layers.push_tooltip(bounds);
        let list = layers.current_mut();
        draw_tooltip_body(list, style, input, region, screen_width, screen_height);
        layers.pop_layer();
    }

    /// Direct-to-draw-list rendering for callers that aren't using
    /// `LayerStack` yet (legacy "call this at the end of your UI" path).
    /// Prefer `draw_into_layers` in new code.
    pub fn draw(
        &self,
        input: &InputState,
        list: &mut DrawList,
        style: &StyleResolver,
        screen_width: f32,
        screen_height: f32,
    ) {
        if !self.is_visible() {
            return;
        }
        let region = match self.hovered_idx.and_then(|i| self.regions.get(i)) {
            Some(r) => r,
            None => return,
        };
        draw_tooltip_body(list, style, input, region, screen_width, screen_height);
    }
}

fn draw_tooltip_body(
    list: &mut DrawList,
    style: &StyleResolver,
    input: &InputState,
    region: &HoverRegion,
    screen_width: f32,
    screen_height: f32,
) {
    let padding = 8.0;
    let font_size = style.scalar(StyleKey::FontSize);
    let title_size = font_size * 0.85;
    let body_size = font_size * 0.75;
    let line_height = body_size + 3.0;
    let title_height = title_size + 4.0;

    let mut rich_desc_h: f32 = 0.0;

    let (width, height) = match &region.content {
        TooltipContent::Text { title, body } => {
            let title_h = if title.is_some() { title_height } else { 0.0 };
            let (body_width_natural, _) = list.measure_text(body, body_size, None);
            let w = 220.0f32.max(body_width_natural).min(300.0);
            let inner_w = w - padding * 2.0;
            let (_, body_h) = list.measure_text(body, body_size, Some(inner_w));
            let h = padding * 2.0 + title_h + body_h;
            (w, h)
        }
        TooltipContent::Lines { title, lines } => {
            let title_h = if title.is_some() { title_height } else { 0.0 };
            let max_line_width = lines
                .iter()
                .map(|line| list.measure_text(line, body_size, None).0)
                .fold(0.0f32, f32::max);
            let w = 180.0f32.max(max_line_width).min(300.0);
            let h = padding * 2.0 + title_h + lines.len() as f32 * line_height;
            (w, h)
        }
        TooltipContent::Rich {
            title: _,
            description,
            details,
        } => {
            let w = 240.0;
            let inner_w = w - padding * 2.0;
            let (_, desc_h) = list.measure_text(description, body_size, Some(inner_w));
            rich_desc_h = desc_h;
            let h = padding * 2.0
                + title_height
                + desc_h
                + if !details.is_empty() {
                    8.0 + details.len() as f32 * line_height
                } else {
                    0.0
                };
            (w, h)
        }
    };

    let margin = 12.0;
    let mut x = input.mouse_x + margin;
    let mut y = input.mouse_y + margin;

    if x + width > screen_width - margin {
        x = input.mouse_x - width - margin;
    }
    if y + height > screen_height - margin {
        y = input.mouse_y - height - margin;
    }
    x = x.max(margin);
    y = y.max(margin);

    // A tooltip sizes itself to its content, so its allocation is only known
    // here. Both public entry points funnel through this function, so scoping it
    // once covers them and skips their "not visible" early returns entirely —
    // an unhovered tooltip has no allocation to declare.
    let chrome = style.tooltip();
    let shadow_margin = chrome.shadow.blur * 1.5;
    list.push_debug_scope_rect(
        "Tooltip",
        Rect::new(
            x - shadow_margin,
            y - shadow_margin + chrome.shadow.offset[1],
            width + shadow_margin * 2.0,
            height + shadow_margin * 2.0,
        ),
    );

    let rect = Rect::new(x, y, width, height);
    let padding_box = rect.inset(chrome.surface.border_widths.left);
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
    {
        let list = surface.draw_list();

        let content_x = x + padding;
        let mut cursor_y = y + padding;

        let highlight = style.color(StyleKey::TextHighlight);
        let text_color = style.color(StyleKey::Text);
        let text_dim = style.color(StyleKey::TextDim);
        let font = style.theme().font.clone();

        match &region.content {
            TooltipContent::Text { title, body } => {
                if let Some(t) = title {
                    let title_block = TextBlock::new(t, content_x, cursor_y)
                        .with_size(title_size)
                        .with_color_f32(highlight)
                        .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                        .with_font_opt(font.clone());
                    list.text(title_block);
                    cursor_y += title_height;
                }
                let body_block = TextBlock::new(body, content_x, cursor_y)
                    .with_size(body_size)
                    .with_color_f32(text_color)
                    .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                    .with_max_width(width - padding * 2.0)
                    .with_font_opt(font.clone());
                list.text(body_block);
            }
            TooltipContent::Lines { title, lines } => {
                if let Some(t) = title {
                    let title_block = TextBlock::new(t, content_x, cursor_y)
                        .with_size(title_size)
                        .with_color_f32(highlight)
                        .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                        .with_font_opt(font.clone());
                    list.text(title_block);
                    cursor_y += title_height;
                }
                for line in lines {
                    let line_block = TextBlock::new(line, content_x, cursor_y)
                        .with_size(body_size)
                        .with_color_f32(text_color)
                        .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                        .with_font_opt(font.clone());
                    list.text(line_block);
                    cursor_y += line_height;
                }
            }
            TooltipContent::Rich {
                title,
                description,
                details,
            } => {
                let title_block = TextBlock::new(title, content_x, cursor_y)
                    .with_size(title_size)
                    .with_color_f32(highlight)
                    .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                    .with_font_opt(font.clone());
                list.text(title_block);
                cursor_y += title_height;

                let desc_block = TextBlock::new(description, content_x, cursor_y)
                    .with_size(body_size)
                    .with_color_f32(text_color)
                    .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                    .with_max_width(width - padding * 2.0)
                    .with_font_opt(font.clone());
                list.text(desc_block);

                cursor_y += rich_desc_h;

                if !details.is_empty() {
                    cursor_y += 8.0;
                    for (key, value) in details {
                        let detail_text = format!("{}: {}", key, value);
                        let detail_block = TextBlock::new(&detail_text, content_x, cursor_y)
                            .with_size(body_size)
                            .with_color_f32(text_dim)
                            .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
                            .with_font_opt(font.clone());
                        list.text(detail_block);
                        cursor_y += line_height;
                    }
                }
            }
        }
    }
    surface.paint_post_content();
    list.pop_debug_scope();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    #[test]
    fn no_delay_shows_immediately() {
        let mut tt = TooltipLayer::new();
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.0, &input_at(10.0, 10.0));
        assert!(tt.is_visible());
    }

    #[test]
    fn delay_blocks_immediate_display() {
        let mut tt = TooltipLayer::new().with_delay_ms(400);
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.05, &input_at(10.0, 10.0));
        assert!(!tt.is_visible(), "tooltip should still be hidden at 50ms");
    }

    #[test]
    fn delay_satisfied_after_enough_hover_time() {
        let mut tt = TooltipLayer::new().with_delay_ms(200);
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.1, &input_at(10.0, 10.0));
        assert!(!tt.is_visible());
        tt.tick(0.15, &input_at(10.0, 10.0));
        assert!(tt.is_visible(), "tooltip should be visible after 250ms");
    }

    #[test]
    fn moving_off_resets_hover_timer() {
        let mut tt = TooltipLayer::new().with_delay_ms(200);
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.5, &input_at(10.0, 10.0));
        assert!(tt.is_visible());

        // Move off — clear regions and re-register? In a real frame the
        // caller calls clear/register every frame. We simulate that.
        tt.clear();
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        tt.tick(0.05, &input_at(500.0, 500.0)); // outside
        assert!(!tt.is_visible(), "moving off should hide tooltip");
        tt.tick(0.05, &input_at(10.0, 10.0)); // re-enter
        assert!(!tt.is_visible(), "re-enter should restart delay");
    }

    #[test]
    fn typed_overlay_reaches_tooltip_surface_and_shadow() {
        let theme = crate::Theme::default();
        let mut chrome = theme.chrome.tooltip;
        chrome.surface.background = crate::Background::Solid([0.2, 0.3, 0.4, 1.0]);
        chrome.shadow.color = [0.5, 0.1, 0.2, 0.7];
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_tooltip(chrome);
        let styles = StyleResolver::with_overlay(&theme, &overlay);
        let mut layer = TooltipLayer::new();
        layer.register(Rect::new(0.0, 0.0, 40.0, 40.0), TooltipContent::text("tip"));
        let input = input_at(10.0, 10.0);
        layer.tick(0.0, &input);
        let mut list = DrawList::new();
        layer.draw(&input, &mut list, &styles, 400.0, 300.0);
        assert!(
            list.chrome_instances()
                .any(|i| i.bg == [0.2, 0.3, 0.4, 1.0])
        );
        assert_eq!(list.shadow_instance_count(), 1);
        assert_eq!(list.shadow_instance(0).unwrap().color, chrome.shadow.color);
    }

    // --- Anchored hint ---------------------------------------------------

    const ANCHOR: Rect = Rect {
        x: 100.0,
        y: 100.0,
        width: 24.0,
        height: 26.0,
    };
    const VIEWPORT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 300.0,
    };

    #[test]
    fn hint_sits_seven_px_off_its_side_centred_on_the_anchor() {
        let (w, h) = (60.0, 20.0);
        let cx = ANCHOR.x + ANCHOR.width * 0.5;
        let cy = ANCHOR.y + ANCHOR.height * 0.5;

        let r = place_hint(ANCHOR, TooltipSide::Right, w, h, VIEWPORT);
        assert_eq!(r.x, ANCHOR.right() + 7.0);
        assert_eq!(r.y + h * 0.5, cy);

        let r = place_hint(ANCHOR, TooltipSide::Left, w, h, VIEWPORT);
        assert_eq!(r.right(), ANCHOR.x - 7.0);
        assert_eq!(r.y + h * 0.5, cy);

        let r = place_hint(ANCHOR, TooltipSide::Bottom, w, h, VIEWPORT);
        assert_eq!(r.y, ANCHOR.bottom() + 7.0);
        assert_eq!(r.x + w * 0.5, cx);

        let r = place_hint(ANCHOR, TooltipSide::Top, w, h, VIEWPORT);
        assert_eq!(r.bottom(), ANCHOR.y - 7.0);
        assert_eq!(r.x + w * 0.5, cx);
    }

    #[test]
    fn hint_flips_when_only_the_opposite_side_fits() {
        // A key hugging the right edge: its Right hint would leave the screen.
        let anchor = Rect::new(VIEWPORT.right() - 30.0, 100.0, 24.0, 26.0);
        let r = place_hint(anchor, TooltipSide::Right, 60.0, 20.0, VIEWPORT);
        assert_eq!(r.right(), anchor.x - 7.0, "flipped to the left side");

        // A key at the very top: its Top hint flips below.
        let anchor = Rect::new(100.0, 2.0, 24.0, 26.0);
        let r = place_hint(anchor, TooltipSide::Top, 60.0, 20.0, VIEWPORT);
        assert_eq!(r.y, anchor.bottom() + 7.0, "flipped below");
    }

    #[test]
    fn hint_keeps_its_side_and_clamps_when_neither_side_fits() {
        // Wider than either gap beside the anchor: stays on the preferred side,
        // pulled back inside the viewport.
        let r = place_hint(ANCHOR, TooltipSide::Right, 390.0, 20.0, VIEWPORT);
        assert!(r.x >= VIEWPORT.x && r.right() <= VIEWPORT.right());
    }

    #[test]
    fn hint_is_clamped_into_the_viewport_along_the_cross_axis() {
        // A left-docked key near the bottom: centring would hang off the screen.
        let anchor = Rect::new(2.0, VIEWPORT.bottom() - 8.0, 24.0, 26.0);
        let r = place_hint(anchor, TooltipSide::Right, 60.0, 20.0, VIEWPORT);
        assert_eq!(r.bottom(), VIEWPORT.bottom());
        assert_eq!(r.x, anchor.right() + 7.0);
    }

    #[test]
    fn hint_paints_label_in_emph_and_shortcut_in_mono_tip_hint() {
        let theme = crate::Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rect = TooltipHint::new("Move")
            .shortcut("W")
            .side(TooltipSide::Right)
            .draw(&mut list, &styles, ANCHOR, VIEWPORT);

        assert_eq!(list.texts.len(), 2);
        let (label, shortcut) = (&list.texts[0], &list.texts[1]);
        assert_eq!(label.content, "Move");
        assert_eq!(label.font_size, styles.text_size(TextSize::Row));
        assert_eq!(label.font, theme.font);
        assert_eq!(
            label.color,
            TextBlock::new("", 0.0, 0.0)
                .with_color_f32(styles.ink(Ink::Emph))
                .color
        );
        assert_eq!(shortcut.content, "W");
        assert_eq!(shortcut.font_size, styles.text_size(TextSize::Meta));
        assert_eq!(shortcut.font, theme.mono_font);
        assert_eq!(
            shortcut.color,
            TextBlock::new("", 0.0, 0.0)
                .with_color_f32(styles.ink(Ink::TipHint))
                .color
        );
        // Label then shortcut, both inside the padded box.
        let border = theme.chrome.tooltip.surface.border_widths.left;
        assert_eq!(label.x, rect.x + border + 7.0);
        assert!(shortcut.x > label.x + 7.0);
        assert!(shortcut.x < rect.right() - 7.0);
        // One tooltip surface with its single authored elevation.
        assert_eq!(list.shadow_instance_count(), 1);
    }

    #[test]
    fn hint_without_shortcut_paints_only_the_label() {
        let theme = crate::Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let with = TooltipHint::new("Move")
            .shortcut("W")
            .layout(&mut list, &styles, ANCHOR, VIEWPORT);
        let rect = TooltipHint::new("Move")
            .shortcut("")
            .draw(&mut list, &styles, ANCHOR, VIEWPORT);
        assert_eq!(list.texts.len(), 1);
        assert!(rect.width < with.width, "no shortcut slot is reserved");
    }

    #[test]
    fn hint_on_a_layer_is_a_non_blocking_tooltip_layer() {
        let theme = crate::Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut layers = LayerStack::new();
        let rect = TooltipHint::new("Move")
            .side(TooltipSide::Right)
            .draw_into_layers(&mut layers, &styles, ANCHOR, VIEWPORT);
        assert!(!layers.has_active_layer(), "push/pop balanced");
        let layer = &layers.layers()[0];
        assert_eq!(layer.kind, crate::layer::LayerKind::Tooltip);
        assert_eq!(layer.rect, rect);
        assert_eq!(layer.list.texts.len(), 1);
        let over = input_at(rect.x + 2.0, rect.y + 2.0);
        assert!(!layers.input_for_base(&over).mouse_consumed);
    }

    #[test]
    fn hint_reads_the_tooltip_chrome_overlay() {
        let theme = crate::Theme::default();
        let mut chrome = theme.chrome.tooltip;
        chrome.surface.background = crate::Background::Solid([0.2, 0.3, 0.4, 1.0]);
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_tooltip(chrome);
        let styles = StyleResolver::with_overlay(&theme, &overlay);
        let mut list = DrawList::new();
        TooltipHint::new("Move").draw(&mut list, &styles, ANCHOR, VIEWPORT);
        assert!(
            list.chrome_instances()
                .any(|i| i.bg == [0.2, 0.3, 0.4, 1.0])
        );
    }

    #[test]
    fn consumed_input_blocks_tooltip() {
        let mut tt = TooltipLayer::new();
        tt.register(Rect::new(0.0, 0.0, 100.0, 50.0), TooltipContent::text("hi"));
        let mut inp = input_at(10.0, 10.0);
        inp.mouse_consumed = true;
        tt.tick(0.0, &inp);
        assert!(!tt.is_visible());
    }
}
