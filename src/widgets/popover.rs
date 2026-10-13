//! Popover — the design's anchored arrow sheet (Gallery III "popover").
//!
//! A small raised panel with a 7px 45° arrow pointing at its anchor, plus a
//! close ghost key. Geometry helpers are pure and unit-tested; drawing goes
//! through a caller-provided `DrawList` (usually a popup layer's) because the
//! popover floats above the base layer.

use crate::chrome::SurfacePainter;
use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::{DrawContext, DrawList};

/// Where the popover sits relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverSide {
    /// Above the anchor, arrow pointing down.
    Above,
    /// Below the anchor, arrow pointing up.
    Below,
}

impl PopoverSide {
    /// The other side.
    pub fn opposite(self) -> Self {
        match self {
            Self::Above => Self::Below,
            Self::Below => Self::Above,
        }
    }
}

/// Space between the anchor and the popover body (the arrow's length).
const ANCHOR_GAP: f32 = 6.0;
/// Space the body keeps from the edges of its bounds.
const EDGE: f32 = 4.0;

/// Which side of `anchor` a `height`-tall box `gap` away from it goes on:
/// `preferred` when it fits inside `bounds`, the other side when only that
/// one fits, and whichever has more room when neither does.
pub(crate) fn choose_side(
    anchor: Rect,
    height: f32,
    gap: f32,
    bounds: Rect,
    preferred: PopoverSide,
) -> PopoverSide {
    let room = |side| room(anchor, gap, bounds, side);
    let other = preferred.opposite();
    if height <= room(preferred) {
        preferred
    } else if height <= room(other) || room(other) > room(preferred) {
        other
    } else {
        preferred
    }
}

/// The height free on `side` of `anchor` for a box `gap` away from it,
/// inside `bounds` less their edge margin.
pub(crate) fn room(anchor: Rect, gap: f32, bounds: Rect, side: PopoverSide) -> f32 {
    match side {
        PopoverSide::Above => anchor.y - gap - EDGE - bounds.y,
        PopoverSide::Below => bounds.bottom() - EDGE - anchor.bottom() - gap,
    }
}

/// Where a popover went: its body, and the side of the anchor it ended up
/// on (draw its arrow with that side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverPlacement {
    /// The popover body's rect.
    pub body: Rect,
    /// The side it is on, which is not the one asked for when that had no
    /// room.
    pub side: PopoverSide,
}

/// Compute where a popover goes for its anchor (pure).
///
/// * `anchor` — the trigger's rect (in the same space as `bounds`); the arrow
///   points at the middle of its top or bottom edge.
/// * `size` — the popover body's `[w, h]`.
/// * `bounds` — the screen/viewport the popover must stay inside. The body
///   is nudged horizontally to fit, and goes on the other side of the anchor
///   when `side` has no room for it but the other has (as tooltips and
///   submenus do). With room on neither, it takes the roomier side and is
///   kept inside `bounds`, over the anchor if need be.
pub fn place_popover(
    anchor: Rect,
    size: [f32; 2],
    bounds: Rect,
    side: PopoverSide,
) -> PopoverPlacement {
    let [w, h] = size;
    let x = (anchor.x + anchor.width * 0.5 - w * 0.5)
        .max(bounds.x + EDGE)
        .min(bounds.right() - w - EDGE);
    let side = choose_side(anchor, h, ANCHOR_GAP, bounds, side);
    let y = match side {
        PopoverSide::Above => anchor.y - h - ANCHOR_GAP,
        PopoverSide::Below => anchor.bottom() + ANCHOR_GAP,
    };
    // Kept inside: when taller than the bounds, the top edge wins.
    let y = y.min(bounds.bottom() - h - EDGE).max(bounds.y + EDGE);
    PopoverPlacement {
        body: Rect::new(x, y, w, h),
        side,
    }
}

/// Measure the minimum body height that keeps `title` and wrapped `lines` inside
/// a popover with `body_width`. Pass the result to [`place_popover`] before
/// drawing; [`draw_sheet`] deliberately stays rect-native.
pub fn measure_sheet_height(
    body_width: f32,
    title: &str,
    lines: &[&str],
    list: &mut DrawList,
    s: &StyleResolver,
) -> f32 {
    let font_size = s.scalar(StyleKey::FontSize);
    let _ = title;
    let content_width = (body_width - POPOVER_PAD * 2.0).max(0.0);
    let line_stack: f32 = lines
        .iter()
        .map(|line| list.measure_text(line, font_size, Some(content_width)).1 + 4.0)
        .sum();
    // The content starts under the title bar and its rule, inside the pad.
    // Text shaping's visual bounds can sit below the measured line box, so
    // the bottom pad follows the final row gap rather than just enough
    // height to avoid clipping descenders.
    POPOVER_CONTENT_TOP + POPOVER_PAD + line_stack
}

/// Height of a popover's title bar (`padding: 7px 9px` around 9px caps).
pub const POPOVER_TITLE_H: f32 = 25.0;
/// Where a popover's content starts below the body's top: under the title
/// bar's two-line rule and the pad.
pub const POPOVER_CONTENT_TOP: f32 = POPOVER_TITLE_H + 2.0 + POPOVER_PAD;
/// Space around a popover's content.
pub const POPOVER_PAD: f32 = 9.0;
/// The title bar's close key.
const CLOSE_KEY: f32 = 15.0;
/// The title's letter spacing (`--track-caption`), in em.
const TITLE_TRACKING: f32 = 0.14;
/// The title bar's rule: dark, and a light line under it.
const TITLE_RULE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const TITLE_RULE_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.05];

/// A drawn popover frame (see [`draw_frame`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverFrame {
    /// The close key, for hit-testing.
    pub close: Rect,
    /// Where the content goes: the body under the title bar, inside the pad.
    pub content: Rect,
}

/// Draw the popover sheet (body + arrow) with a title and body text lines
/// (see [`draw_frame`]). Returns the rect of the close key so the caller
/// can hit-test it (or pass an input and let this do it — see
/// [`PopoverOutput`]).
pub fn draw_sheet(
    body: Rect,
    side: PopoverSide,
    title: &str,
    lines: &[&str],
    list: &mut DrawList,
    s: &StyleResolver,
) -> Rect {
    let frame = draw_frame(body, side, title, list, s);
    let font_size = s.scalar(StyleKey::FontSize);
    let dim = s.color(StyleKey::TextDim);
    let width = frame.content.width;
    let mut y = frame.content.y;
    for line in lines {
        let (_, th) = list.measure_text(line, font_size, Some(width));
        let ly = list.vcentered_text_y(y, th, font_size, s.theme().font.as_ref(), line);
        list.text(
            TextBlock::new(*line, frame.content.x, ly)
                .with_size(font_size)
                .with_color_f32(dim)
                .with_max_width(width)
                .with_font_opt(s.theme().font.clone()),
        );
        y += th + 4.0;
    }
    frame.close
}

/// Draw a Forge popover's frame: the raised sheet with its arrow toward the
/// anchor, a title bar (mono caps title, a ghost × key, a two-line rule),
/// and nothing inside, for the caller to fill [`PopoverFrame::content`]
/// with any controls.
pub fn draw_frame(
    body: Rect,
    side: PopoverSide,
    title: &str,
    list: &mut DrawList,
    s: &StyleResolver,
) -> PopoverFrame {
    let chrome = s.popover();
    let shadow_margin = chrome.shadow.blur * 1.5;
    // Declared scope covers body + arrow + the analytic shadow skirt.
    list.push_debug_scope_rect(
        "Popover",
        match side {
            PopoverSide::Above => Rect::new(
                body.x - shadow_margin,
                body.y - shadow_margin + chrome.shadow.offset[1],
                body.width + shadow_margin * 2.0,
                body.height + 6.0 + shadow_margin * 2.0,
            ),
            PopoverSide::Below => Rect::new(
                body.x - shadow_margin,
                body.y - 6.0 - shadow_margin + chrome.shadow.offset[1],
                body.width + shadow_margin * 2.0,
                body.height + 6.0 + shadow_margin * 2.0,
            ),
        },
    );

    // Keep the authored order: body elevation, arrow behind the sheet, sheet,
    // content, and finally the sheet border.
    list.box_shadow_outset(body, chrome.surface.corner_radii, chrome.shadow);
    let ax = body.x + body.width * 0.5;
    let (ay0, ay1) = match side {
        PopoverSide::Above => (body.bottom() - 1.0, body.bottom() + 6.0),
        PopoverSide::Below => (body.y + 1.0, body.y - 6.0),
    };
    let panel = match chrome.surface.background {
        crate::Background::Solid(color) => color,
        crate::Background::LinearGradient { start, .. } => start,
    };
    list.triangle((ax - 5.0, ay0), (ax + 5.0, ay0), (ax, ay1), panel);

    let padding_box = body.inset(chrome.surface.border_widths.left);
    let mut surface = SurfacePainter::new(
        list,
        body,
        padding_box,
        chrome.surface.corner_radii,
        chrome.surface,
        &[],
        &[],
    );
    surface.paint_pre_content();
    let close;
    {
        let list = surface.draw_list();
        // 1px inset highlight under the top edge.
        let hl = s.color(StyleKey::EdgeHighlight);
        list.quad(
            body.x + 1.0,
            body.y + 1.0,
            (body.width - 2.0).max(0.0),
            1.0,
            hl,
        );

        // Title bar: 9px mono caps in `--ink-glyph`, a ghost × key, and a
        // two-line rule under it.
        close = Rect::new(
            body.right() - POPOVER_PAD - CLOSE_KEY,
            body.y + (POPOVER_TITLE_H - CLOSE_KEY) * 0.5,
            CLOSE_KEY,
            CLOSE_KEY,
        );
        let size = s.text_size(TextSize::Caption);
        list.text(
            s.mono_block(
                title.to_uppercase(),
                body.x + POPOVER_PAD,
                crate::text::vcentered_line_y(body.y, POPOVER_TITLE_H, size),
                TextSize::Caption,
                Ink::Glyph,
            )
            .with_letter_spacing(size * TITLE_TRACKING)
            .with_max_width((close.x - body.x - POPOVER_PAD * 2.0).max(0.0))
            .with_ellipsis(),
        );
        let x_size = s.text_size(TextSize::Row);
        let (xw, _) = list.measure_text("×", x_size, None);
        list.text(s.sans_block(
            "×",
            close.x + (CLOSE_KEY - xw) * 0.5,
            crate::text::vcentered_line_y(close.y, close.height, x_size),
            TextSize::Row,
            Ink::Cell,
        ));
        list.quad(
            body.x,
            body.y + POPOVER_TITLE_H,
            body.width,
            1.0,
            TITLE_RULE,
        );
        list.quad(
            body.x,
            body.y + POPOVER_TITLE_H + 1.0,
            body.width,
            1.0,
            TITLE_RULE_HI,
        );
    }
    surface.paint_post_content();
    list.pop_debug_scope();
    PopoverFrame {
        close,
        content: Rect::new(
            body.x + POPOVER_PAD,
            body.y + POPOVER_CONTENT_TOP,
            (body.width - POPOVER_PAD * 2.0).max(0.0),
            (body.height - POPOVER_CONTENT_TOP - POPOVER_PAD).max(0.0),
        ),
    }
}

/// Outcome of a popover frame (from [`Popover::draw`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PopoverOutput {
    /// The close key (or outside click) was activated — caller closes.
    pub close_requested: bool,
}

/// A convenience wrapper: draws the sheet and hit-tests the close key against
/// the current input. Persistent `open` stays caller-owned.
#[derive(Clone, Copy, Default)]
pub struct Popover;

impl Popover {
    /// Draw a popover at the rect from [`place_popover`]; returns close intent.
    pub fn draw(
        &self,
        body: Rect,
        side: PopoverSide,
        title: &str,
        lines: &[&str],
        ctx: &mut DrawContext,
    ) -> PopoverOutput {
        let s = ctx.styles();
        let close = {
            let list = &mut *ctx.draw_list;
            draw_sheet(body, side, title, lines, list, &s)
        };
        let input = ctx.input;
        let inside = body.contains(input.mouse_x, input.mouse_y)
            || close.contains(input.mouse_x, input.mouse_y);
        let clicked_outside = !inside && input.mouse_clicked && !input.mouse_consumed;
        let close_hit = close.contains(input.mouse_x, input.mouse_y)
            && input.mouse_clicked
            && !input.mouse_consumed;
        PopoverOutput {
            close_requested: close_hit || clicked_outside,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    #[test]
    fn place_keeps_the_body_inside_bounds_and_on_the_right_side() {
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
        let anchor = Rect::new(0.0, 100.0, 10.0, 20.0);
        // Anchor at the very left edge: the body must be nudged right to fit.
        let above = place_popover(anchor, [120.0, 60.0], bounds, PopoverSide::Above);
        assert_eq!(above.side, PopoverSide::Above);
        assert!(
            above.body.x >= bounds.x + 4.0,
            "nudged inside the left edge"
        );
        assert_eq!(above.body.bottom(), 100.0 - 6.0, "above the anchor");
        let below = place_popover(anchor, [120.0, 60.0], bounds, PopoverSide::Below);
        assert_eq!(below.side, PopoverSide::Below);
        assert_eq!(below.body.y, 120.0 + 6.0, "below the anchor's bottom");
    }

    #[test]
    fn a_popover_with_no_room_on_its_side_flips() {
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
        // Near the bottom: a popover asked for below opens above.
        let low = Rect::new(100.0, 260.0, 40.0, 20.0);
        let placed = place_popover(low, [120.0, 80.0], bounds, PopoverSide::Below);
        assert_eq!(placed.side, PopoverSide::Above);
        assert_eq!(
            placed.body.bottom(),
            260.0 - 6.0,
            "arrow on the anchor's top"
        );
        // Near the top: one asked for above opens below.
        let high = Rect::new(100.0, 10.0, 40.0, 20.0);
        let placed = place_popover(high, [120.0, 80.0], bounds, PopoverSide::Above);
        assert_eq!(placed.side, PopoverSide::Below);
        assert_eq!(placed.body.y, 30.0 + 6.0);
    }

    #[test]
    fn with_room_on_neither_side_the_roomier_wins_and_it_stays_inside() {
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
        let anchor = Rect::new(100.0, 180.0, 40.0, 20.0);
        // 200 tall: 170 above the anchor, 90 below. Above has more room.
        let placed = place_popover(anchor, [120.0, 200.0], bounds, PopoverSide::Below);
        assert_eq!(placed.side, PopoverSide::Above);
        assert!(placed.body.y >= 4.0, "kept inside: {:?}", placed.body);
        assert!(placed.body.bottom() <= 296.0);
        // Taller than the bounds: the top edge wins.
        let placed = place_popover(anchor, [120.0, 400.0], bounds, PopoverSide::Below);
        assert_eq!(placed.body.y, 4.0);
    }

    #[test]
    fn measured_height_fits_wrapped_content() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let h = measure_sheet_height(
            212.0,
            "Rename",
            &["Enter a new name for the selected entity."],
            &mut list,
            &styles,
        );
        assert!(
            h > 58.0,
            "the gallery sentence needs more than its old fixed 58px body"
        );

        let body = Rect::new(0.0, 0.0, 212.0, h);
        let line_h = list
            .measure_text(
                "Enter a new name for the selected entity.",
                theme.font_size,
                Some(196.0),
            )
            .1;
        const BOTTOM_INSET: f32 = 8.0;
        assert!(
            body.height - (POPOVER_CONTENT_TOP + line_h + 4.0) >= BOTTOM_INSET,
            "measured popover leaves a visible bottom inset after wrapped body text"
        );
    }

    #[test]
    fn a_frame_gives_its_content_the_body_under_the_title() {
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let body = Rect::new(10.0, 20.0, 380.0, 300.0);
        let frame = draw_frame(
            body,
            PopoverSide::Below,
            "context usage",
            &mut list,
            &styles,
        );
        assert_eq!(
            frame.content,
            Rect::new(
                10.0 + POPOVER_PAD,
                20.0 + POPOVER_CONTENT_TOP,
                380.0 - 2.0 * POPOVER_PAD,
                300.0 - POPOVER_CONTENT_TOP - POPOVER_PAD
            )
        );
        assert!(frame.close.right() <= body.right() - POPOVER_PAD + 0.01);
        assert!(frame.close.bottom() <= body.y + POPOVER_TITLE_H);
        assert!(
            list.texts.iter().any(|t| t.content == "CONTEXT USAGE"),
            "the title is in caps"
        );
    }

    #[test]
    fn typed_overlay_reaches_sheet_surface_and_shadow() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.popover;
        chrome.surface.background = crate::Background::Solid([0.2, 0.3, 0.4, 1.0]);
        chrome.shadow.color = [0.5, 0.1, 0.2, 0.7];
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_popover(chrome);
        let styles = StyleResolver::with_overlay(&theme, &overlay);
        let mut list = DrawList::new();
        draw_sheet(
            Rect::new(20.0, 20.0, 160.0, 80.0),
            PopoverSide::Below,
            "Title",
            &["line"],
            &mut list,
            &styles,
        );
        assert!(
            list.chrome_instances()
                .any(|i| i.bg == [0.2, 0.3, 0.4, 1.0])
        );
        assert_eq!(list.shadow_instance_count(), 1);
        assert_eq!(list.shadow_instance(0).unwrap().color, chrome.shadow.color);
    }

    #[test]
    fn close_key_click_requests_close() {
        let theme = Theme::default();
        let body = Rect::new(100.0, 100.0, 160.0, 80.0);
        // Click just outside the body (top-left corner of the screen).
        let input = InputState {
            mouse_x: 2.0,
            mouse_y: 2.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 400.0, 300.0);
        let out = Popover.draw(body, PopoverSide::Below, "Hint", &["line"], &mut ctx);
        assert!(out.close_requested, "clicking outside closes");

        // A click inside does not close.
        let input = InputState {
            mouse_x: 150.0,
            mouse_y: 130.0,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 400.0, 300.0);
        let out = Popover.draw(body, PopoverSide::Below, "Hint", &["line"], &mut ctx);
        assert!(!out.close_requested);
    }
}
