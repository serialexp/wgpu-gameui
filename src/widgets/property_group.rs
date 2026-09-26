//! Property group — a collapsible inspector section (Forge `PropertyGroup`),
//! and the vertical cursor inspector content is laid out with.

use crate::layout::Rect;
use crate::style::{Ink, StyleKey, TextSize, Tracking};
use crate::text::{TextAlign, vcentered_line_y};

use super::tree::{CARET_HALF, draw_disclosure};
use super::{DrawContext, PROPERTY_ROW_HEIGHT};

/// A group header's height (`--h-panel-row`).
pub const PROPERTY_GROUP_HEADER_HEIGHT: f32 = 21.0;
/// Between the rows of an open group.
pub const PROPERTY_ROW_GAP: f32 = 4.0;
/// Between a group's header and its first row.
const BODY_GAP: f32 = 5.0;
/// Padding at the header's sides.
const PAD_X: f32 = 3.0;
/// Between the chevron, the title and the summary.
const GAP: f32 = 6.0;
/// The chevron's column (`▾` at 7 px).
const CARET_W: f32 = 7.0;
/// The rule under the header (`inset 0 -1px 0 rgba(0,0,0,.45)`).
const RULE: [f32; 4] = [0.0, 0.0, 0.0, 0.45];

/// A top-to-bottom cursor for inspector content: each
/// [`take`](Self::take) hands out the next full-width rect, `gap` px below
/// the previous one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertyStack {
    x: f32,
    y: f32,
    width: f32,
    gap: f32,
    empty: bool,
}

impl PropertyStack {
    /// A stack starting at `(x, y)`, `width` wide, with `gap` px between
    /// the rects it hands out.
    pub fn new(x: f32, y: f32, width: f32, gap: f32) -> Self {
        Self {
            x,
            y,
            width,
            gap,
            empty: true,
        }
    }

    /// The next `height` px tall rect.
    pub fn take(&mut self, height: f32) -> Rect {
        if !self.empty {
            self.y += self.gap;
        }
        self.empty = false;
        let rect = Rect::new(self.x, self.y, self.width, height);
        self.y += height;
        rect
    }

    /// The next [`PROPERTY_ROW_HEIGHT`] tall rect, for a
    /// [`PropertyRow`](crate::PropertyRow).
    pub fn row(&mut self) -> Rect {
        self.take(PROPERTY_ROW_HEIGHT)
    }

    /// The bottom of the last rect handed out (the top, before any).
    pub fn bottom(&self) -> f32 {
        self.y
    }

    /// The width of the rects handed out.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Whether nothing has been taken yet.
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// Continue below `bottom` (a nested stack's), with no gap of its own:
    /// the nested content hangs off the last rect taken.
    fn extend_to(&mut self, bottom: f32) {
        self.y = self.y.max(bottom);
    }
}

/// A collapsible inspector section (Forge `PropertyGroup`): a 21 px header
/// with a chevron, a mono-caps title and a summary on the right, over its
/// rows. Clicking the header opens or closes it; whether it is open is the
/// caller's `bool`.
///
/// When the group's rows are [`mixed`](crate::PropertyRow::mixed), its
/// summary must read "—" too, never the first object's value.
///
/// ```ignore
/// PropertyGroup::new("Transform").summary("3").draw_in(&mut body, &mut open, &mut ctx,
///     |rows, ctx| {
///         let out = PropertyRow::new("X").draw(X_ID, rows.row(), x, &mut scrub, &mut capture, ctx);
///     });
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertyGroup<'a> {
    title: &'a str,
    summary: Option<&'a str>,
}

impl<'a> PropertyGroup<'a> {
    /// A group titled `title`.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            summary: None,
        }
    }

    /// Dim text at the header's right: what the rows add up to ("3",
    /// "lit · 0.42"), or "—" when they are mixed.
    #[must_use]
    pub fn summary(mut self, summary: &'a str) -> Self {
        self.summary = Some(summary);
        self
    }

    /// Draw the header in `header` and flip `open` when it is clicked.
    /// Returns whether it was clicked this frame.
    pub fn draw(&self, header: Rect, open: &mut bool, ctx: &mut DrawContext) -> bool {
        let s = ctx.styles();
        let input = ctx.input;
        let hovered = !input.mouse_consumed && header.contains(input.mouse_x, input.mouse_y);
        let toggled = hovered && input.mouse_clicked;
        if toggled {
            *open = !*open;
        }
        if hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }

        ctx.push_debug_scope_rect(super::scope_name("PropertyGroup", self.title), header);
        let list = &mut *ctx.draw_list;
        if hovered {
            list.quad(
                header.x,
                header.y,
                header.width,
                header.height,
                s.color(StyleKey::RowHover),
            );
        }
        list.quad(header.x, header.bottom() - 1.0, header.width, 1.0, RULE);

        let cy = header.y + header.height * 0.5;
        let mut x = header.x + PAD_X;
        draw_disclosure(
            list,
            x + CARET_W * 0.5,
            cy,
            CARET_HALF,
            *open,
            s.ink(Ink::Body2),
        );
        x += CARET_W + GAP;

        let size = s.text_size(TextSize::Caption);
        let y = vcentered_line_y(header.y, header.height, size);
        let right = header.right() - PAD_X;
        let mut title_right = right;
        if let Some(summary) = self.summary {
            // Right-aligned in everything right of the chevron, so it's cut
            // only when it doesn't fit — not when `right - (right - w)`
            // rounds a hair under `w`.
            let w = s.mono_width(list, summary, TextSize::Caption);
            list.text(
                s.mono_block(summary, x, y, TextSize::Caption, Ink::Disabled)
                    .with_max_width((right - x).max(0.0))
                    .with_align(TextAlign::Right)
                    .with_ellipsis(),
            );
            title_right = (right - w).max(x) - GAP;
        }
        list.text(
            s.caption_block(self.title, x, y, Tracking::Section, Ink::Glyph)
                .with_max_width((title_right - x).max(0.0))
                .with_ellipsis(),
        );
        ctx.pop_debug_scope();
        toggled
    }

    /// Take the header from `stack`, draw it, and while `open` run `rows`
    /// with a stack for the group's rows ([`PROPERTY_ROW_GAP`] apart). The
    /// outer stack continues under the last row. Returns whether the header
    /// was clicked this frame.
    pub fn draw_in(
        &self,
        stack: &mut PropertyStack,
        open: &mut bool,
        ctx: &mut DrawContext,
        rows: impl FnOnce(&mut PropertyStack, &mut DrawContext),
    ) -> bool {
        let header = stack.take(PROPERTY_GROUP_HEADER_HEIGHT);
        let toggled = self.draw(header, open, ctx);
        if *open {
            let mut inner = PropertyStack::new(
                header.x,
                header.bottom() + BODY_GAP,
                header.width,
                PROPERTY_ROW_GAP,
            );
            rows(&mut inner, ctx);
            if !inner.is_empty() {
                stack.extend_to(inner.bottom());
            }
        }
        toggled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    fn at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn click(x: f32, y: f32) -> InputState {
        InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..at(x, y)
        }
    }

    /// Draw `group` in a stack at the origin, 200 wide, taking `rows` rows
    /// while open. Returns (clicked, the stack after, the list).
    fn frame(
        group: &PropertyGroup,
        open: &mut bool,
        rows: usize,
        input: &InputState,
    ) -> (bool, PropertyStack, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut stack = PropertyStack::new(0.0, 0.0, 200.0, 12.0);
        let toggled = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            group.draw_in(&mut stack, open, &mut ctx, |inner, _| {
                for _ in 0..rows {
                    inner.row();
                }
            })
        };
        (toggled, stack, list)
    }

    #[test]
    fn the_stack_hands_out_rects_gap_apart() {
        let mut stack = PropertyStack::new(5.0, 10.0, 100.0, 4.0);
        assert!(stack.is_empty());
        assert_eq!(stack.take(20.0), Rect::new(5.0, 10.0, 100.0, 20.0));
        assert_eq!(
            stack.row(),
            Rect::new(5.0, 34.0, 100.0, PROPERTY_ROW_HEIGHT)
        );
        assert_eq!(stack.bottom(), 34.0 + PROPERTY_ROW_HEIGHT);
        assert_eq!(stack.width(), 100.0);
    }

    #[test]
    fn an_open_group_lays_its_rows_under_the_header() {
        let mut open = true;
        let (_, stack, _) = frame(
            &PropertyGroup::new("Transform"),
            &mut open,
            3,
            &at(-5.0, -5.0),
        );
        let rows = 3.0 * PROPERTY_ROW_HEIGHT + 2.0 * PROPERTY_ROW_GAP;
        assert_eq!(
            stack.bottom(),
            PROPERTY_GROUP_HEADER_HEIGHT + BODY_GAP + rows
        );
    }

    #[test]
    fn a_closed_group_is_only_its_header() {
        let mut open = false;
        let (_, stack, _) = frame(
            &PropertyGroup::new("Transform"),
            &mut open,
            3,
            &at(-5.0, -5.0),
        );
        assert_eq!(stack.bottom(), PROPERTY_GROUP_HEADER_HEIGHT);
    }

    #[test]
    fn an_open_group_without_rows_adds_no_gap() {
        let mut open = true;
        let (_, stack, _) = frame(&PropertyGroup::new("Empty"), &mut open, 0, &at(-5.0, -5.0));
        assert_eq!(stack.bottom(), PROPERTY_GROUP_HEADER_HEIGHT);
    }

    #[test]
    fn clicking_the_header_flips_open() {
        let group = PropertyGroup::new("Material");
        let mut open = true;
        let (toggled, _, _) = frame(&group, &mut open, 1, &click(100.0, 10.0));
        assert!(toggled);
        assert!(!open);
        let (toggled, _, _) = frame(&group, &mut open, 1, &click(100.0, 10.0));
        assert!(toggled && open);
        let consumed = InputState {
            mouse_consumed: true,
            ..click(100.0, 10.0)
        };
        let (toggled, _, _) = frame(&group, &mut open, 1, &consumed);
        assert!(!toggled && open);
        // A click on a row is not a click on the header.
        let (toggled, _, _) = frame(&group, &mut open, 1, &click(100.0, 30.0));
        assert!(!toggled && open);
    }

    #[test]
    fn the_title_is_mono_caps_and_the_summary_sits_right() {
        let group = PropertyGroup::new("Material").summary("lit · 0.42");
        let mut open = true;
        let (_, _, list) = frame(&group, &mut open, 0, &at(-5.0, -5.0));
        let title = list.texts.iter().find(|t| t.content == "MATERIAL").unwrap();
        let summary = list
            .texts
            .iter()
            .find(|t| t.content == "lit · 0.42")
            .unwrap();
        assert_eq!(title.x, PAD_X + CARET_W + GAP);
        assert_eq!(summary.align, TextAlign::Right);
        assert!((summary.x + summary.max_width - (200.0 - PAD_X)).abs() < 0.01);
        // The title stops a gap short of where the summary's text starts.
        let theme = Theme::default();
        let w = crate::StyleResolver::new(&theme).mono_width(
            &mut DrawList::new(),
            "lit · 0.42",
            TextSize::Caption,
        );
        assert!(title.x + title.max_width <= 200.0 - PAD_X - w - GAP + 0.01);
    }

    /// The renderer cuts text whose width passes its box by any amount, so
    /// a box sized to the text as `right - (right - w)` loses the last
    /// digit of float at some positions and turns "3" into "…".
    #[test]
    fn the_summary_is_never_cut_by_rounding() {
        let theme = Theme::default();
        let s = crate::StyleResolver::new(&theme);
        let mut focus = FocusState::new();
        let input = at(-5.0, -5.0);
        // One list: a fresh one loads the fonts each time.
        let mut list = DrawList::new();
        for summary in ["3", "72 kg", "lit · 0.42", "—"] {
            let w = s.mono_width(&mut list, summary, TextSize::Caption);
            for step in 0..200 {
                list.clear();
                let header = Rect::new(step as f32 * 1.37, 0.0, 234.6, 21.0);
                let mut open = false;
                let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
                PropertyGroup::new("Transform")
                    .summary(summary)
                    .draw(header, &mut open, &mut ctx);
                let block = list.texts.iter().find(|t| t.content == summary).unwrap();
                assert!(
                    block.max_width >= w,
                    "{summary:?} at x {}: box {} < text {w}",
                    header.x,
                    block.max_width
                );
            }
        }
    }

    #[test]
    fn the_header_washes_on_hover() {
        let theme = Theme::default();
        let hover = crate::StyleResolver::new(&theme).color(StyleKey::RowHover);
        let group = PropertyGroup::new("Material");
        let mut open = true;
        let has_wash = |list: &DrawList| list.chrome_instances().any(|c| c.bg == hover);
        let (_, _, idle) = frame(&group, &mut open, 0, &at(-5.0, -5.0));
        let (_, _, hovered) = frame(&group, &mut open, 0, &at(100.0, 10.0));
        assert!(!has_wash(&idle));
        assert!(has_wash(&hovered));
    }
}
