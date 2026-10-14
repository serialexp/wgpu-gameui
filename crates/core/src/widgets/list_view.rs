//! List view — the dense, selectable list for sidebars (Forge `ListView`):
//! projects, sessions, files, recent items. Use [`Tree`](super::Tree) for
//! hierarchy and [`Table`](super::Table) for columns.
//!
//! A [`List`] underneath does the scrolling, virtualisation, keyboard and
//! row backgrounds (zebra, hover, the focus-aware selection); this widget
//! paints each row's content from a [`ListRow`]:
//!
//! - an optional glyph (a Phosphor icon), the label (sans or mono, cut with
//!   `…` when it doesn't fit), and an optional right-aligned mono `meta`;
//! - with [`two_line`](ListView::two_line), a mono subtitle under the label
//!   (34 px rows instead of 22) — put a short id there rather than showing a
//!   raw id as the label;
//! - the first case-insensitive match of [`highlight`](ListView::highlight)
//!   in the label, tinted `AccentMatch`, or underlined on an accent row;
//! - disabled rows dimmed, never hovered, selected or opened, and skipped by
//!   the arrow keys.
//!
//! With no rows it shows the [`empty`](ListView::empty) message under a
//! magnifier.
//!
//! Rows come from a per-index closure, so only the visible ones are built.
//! The caller owns which row is selected (by index, mapped from its own ids
//! each frame); [`ListViewOutput::select`] reports the row the user picked
//! and [`ListViewOutput::open`] the one opened with `Enter` or a
//! double-click.
//!
//! Gated behind the `phosphor-icons` feature.
//!
//! # Example
//! ```ignore
//! let out = ListView::new()
//!     .highlight(&query)
//!     .empty("No projects match")
//!     .focused(focus.is_focused(PROJECTS_ID))
//!     .draw(rect, matches.len(), selected, &mut state, list, &style, &mut input,
//!         |i| ListRow::new(name_of(matches[i])));
//! if let Some(i) = out.select { selected = Some(i); }
//! ```

use std::ops::Range;

use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::style::{Ink, TextSize};
use crate::text::{TextAlign, TextBlock, TextStyleRange, Underline};
use crate::{InputState, StyleKey, StyleResolver};

use super::{DrawList, Icon, List, ListItem, ListState};

/// Space before the glyph (or label) and after the meta (`padding: 0 14px 0 9px`).
const PAD_LEFT: f32 = 9.0;
const PAD_RIGHT: f32 = 14.0;
/// Space between the glyph, the label and the meta.
const GAP: f32 = 7.0;
/// Width of the glyph's column, and the icon's size.
const GLYPH: f32 = 10.0;
/// How much taller a two-line row is than a one-line row (34 px against 22).
const TWO_LINE_EXTRA: f32 = 12.0;
/// In a two-line row, the centres of the label and the sub line below the
/// row's top (the label, a 2 px gap and the sub, centred in 34 px).
const TWO_LINE_LABEL_CY: f32 = 10.0;
const TWO_LINE_SUB_CY: f32 = 25.0;
/// The empty message: padding above it, the magnifier's size and the gap
/// under it, and the side padding the message wraps within.
const EMPTY_PAD_TOP: f32 = 18.0;
const EMPTY_GLYPH: f32 = 18.0;
const EMPTY_GAP: f32 = 6.0;
const EMPTY_PAD_SIDE: f32 = 12.0;

/// One row of a [`ListView`]. Borrowed from the caller's data; build it in
/// the per-row closure.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListRow<'a> {
    /// The label.
    pub label: &'a str,
    /// The mono line under the label (two-line views only).
    pub subtitle: Option<&'a str>,
    /// Right-aligned mono metadata (an age, a count).
    pub meta: Option<&'a str>,
    /// A glyph before the label.
    pub glyph: Option<PhosphorIcon>,
    /// Set the label in the mono font (paths, ids).
    pub mono: bool,
    /// Dim the row; it never hovers, selects or opens.
    pub disabled: bool,
}

impl<'a> ListRow<'a> {
    /// A row showing `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            ..Self::default()
        }
    }

    /// Add the mono line under the label (shown in two-line views).
    #[must_use]
    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    /// Add right-aligned metadata.
    #[must_use]
    pub fn meta(mut self, meta: &'a str) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Add a glyph before the label.
    #[must_use]
    pub fn glyph(mut self, glyph: PhosphorIcon) -> Self {
        self.glyph = Some(glyph);
        self
    }

    /// Set the label in the mono font.
    #[must_use]
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// Disable the row.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// What the user did with a [`ListView`] this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListViewOutput {
    /// The row the user selected this frame (a click or an arrow key), when
    /// it differs from the `selected` passed in.
    pub select: Option<usize>,
    /// The row opened this frame: a double-click, or `Enter` on the
    /// selection while the view has focus.
    pub open: Option<usize>,
    /// The row under the pointer.
    pub hovered: Option<usize>,
    /// Whether the pointer is over the rows (for scroll routing).
    pub mouse_over_content: bool,
}

/// A dense selectable list. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct ListView<'a> {
    two_line: bool,
    highlight: &'a str,
    empty: &'a str,
    focused: bool,
}

/// Where a row's texts sit vertically: the label's centre and, in two-line
/// rows, the sub line's.
struct Lines {
    label_cy: f32,
    sub_cy: Option<f32>,
}

impl Default for ListView<'_> {
    fn default() -> Self {
        Self {
            two_line: false,
            highlight: "",
            empty: "Nothing here",
            focused: false,
        }
    }
}

impl<'a> ListView<'a> {
    /// A one-line list view with no highlight and an empty message of
    /// "Nothing here".
    pub fn new() -> Self {
        Self::default()
    }

    /// Two-line rows: 34 px, with each row's subtitle under its label.
    #[must_use]
    pub fn two_line(mut self, two_line: bool) -> Self {
        self.two_line = two_line;
        self
    }

    /// Tint the first case-insensitive match of `query` in each label.
    #[must_use]
    pub fn highlight(mut self, query: &'a str) -> Self {
        self.highlight = query;
        self
    }

    /// The message shown when there are no rows.
    #[must_use]
    pub fn empty(mut self, message: &'a str) -> Self {
        self.empty = message;
        self
    }

    /// Whether the view has keyboard focus: it then takes the arrow keys and
    /// `Enter`, and its selection wears the accent.
    #[must_use]
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// The height of one row under `style`.
    pub fn row_height(&self, style: &StyleResolver) -> f32 {
        let one = style.scalar(StyleKey::ListRowHeight);
        if self.two_line {
            one + TWO_LINE_EXTRA
        } else {
            one
        }
    }

    /// The byte range of the first case-insensitive match of `query` in
    /// `text`, or `None` (also for an empty query). The view highlights
    /// exactly this, so a caller filtering rows with it shows a highlight on
    /// every row it keeps.
    pub fn match_range(text: &str, query: &str) -> Option<Range<usize>> {
        if query.is_empty() {
            return None;
        }
        if text.is_ascii() && query.is_ascii() {
            // The common case, byte by byte: the same answer as
            // `match_range_unicode` (lowercasing ASCII gives ASCII), without
            // a Unicode lowercase per character per position. Non-ASCII text
            // takes the long way, since some of its characters lowercase to
            // ASCII (the Kelvin sign K to k).
            let (text, query) = (text.as_bytes(), query.as_bytes());
            return text
                .windows(query.len())
                .position(|window| window.eq_ignore_ascii_case(query))
                .map(|start| start..start + query.len());
        }
        Self::match_range_unicode(text, query)
    }

    /// [`match_range`](Self::match_range) for any text: each character
    /// lowercased the Unicode way.
    fn match_range_unicode(text: &str, query: &str) -> Option<Range<usize>> {
        text.char_indices().find_map(|(start, _)| {
            let mut rest = text[start..].char_indices();
            for q in query.chars() {
                let (_, c) = rest.next()?;
                if !c.to_lowercase().eq(q.to_lowercase()) {
                    return None;
                }
            }
            let end = rest.next().map_or(text.len(), |(i, _)| start + i);
            Some(start..end)
        })
    }

    /// Draw `count` rows into `rect`. `selected` is the caller's selected
    /// row, `state` keeps the scroll offset and cursor, and `row(i)` builds
    /// row `i` (asked only for visible rows, and while the arrow keys look
    /// past disabled ones).
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'r, F>(
        &self,
        rect: Rect,
        count: usize,
        selected: Option<usize>,
        state: &mut ListState,
        list: &mut DrawList,
        style: &StyleResolver,
        input: &mut InputState,
        row: F,
    ) -> ListViewOutput
    where
        F: Fn(usize) -> ListRow<'r>,
    {
        list.push_debug_scope_rect("ListView", rect);
        state.sync_selection(selected.filter(|&i| i < count && !row(i).disabled));
        let is_disabled = |i: usize| row(i).disabled;
        let out = List::new()
            .with_item_height(self.row_height(style))
            .with_zebra(true)
            .focused(self.focused)
            .disabled(&is_disabled)
            // The design's list scrolls under a floating thumb; the rows'
            // 14 px right padding keeps text clear of it.
            .overlay_scrollbar()
            .draw(
                rect,
                count,
                state,
                list,
                style,
                input,
                |list, cell, item| {
                    self.draw_row(list, style, cell, item, &row(item.index));
                },
            );
        if count == 0 {
            self.draw_empty(list, style, rect);
        }
        list.pop_debug_scope();

        let now = state.single_selected();
        ListViewOutput {
            select: now.filter(|_| now != selected),
            open: out.activated,
            hovered: out.hovered,
            mouse_over_content: out.mouse_over_content,
        }
    }

    fn lines(&self, cell: Rect) -> Lines {
        if self.two_line {
            Lines {
                label_cy: cell.y + TWO_LINE_LABEL_CY,
                sub_cy: Some(cell.y + TWO_LINE_SUB_CY),
            }
        } else {
            Lines {
                label_cy: cell.y + cell.height * 0.5,
                sub_cy: None,
            }
        }
    }

    fn draw_row(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        cell: Rect,
        item: ListItem,
        row: &ListRow,
    ) {
        let off = item.disabled;
        let on_accent = item.selected && item.focused;
        let label_ink = if off {
            s.ink(Ink::Disabled)
        } else if on_accent {
            s.color(StyleKey::OnAccent)
        } else if item.selected {
            s.ink(Ink::Max)
        } else {
            s.ink(Ink::Title)
        };
        let dim_ink = if off {
            s.ink(Ink::DisabledGlyph)
        } else if on_accent {
            s.ink(Ink::OnAccentSecond)
        } else {
            s.ink(Ink::Caption)
        };
        let lines = self.lines(cell);
        let theme = s.theme();
        let label_font = if row.mono {
            theme.mono_font.clone()
        } else {
            theme.font.clone()
        };
        let label_size = s.text_size(TextSize::Menu);
        let label_top = list.x_centered_text_y(lines.label_cy, label_size, label_font.as_ref());
        let baseline =
            lines.label_cy + label_size * list.font_vmetrics(label_font.as_ref()).x_ratio * 0.5;

        let mut left = cell.x + PAD_LEFT;
        if let Some(glyph) = row.glyph {
            let tint = if off {
                s.ink(Ink::DisabledGlyph)
            } else if on_accent {
                s.ink(Ink::OnAccentSecond)
            } else {
                s.ink(Ink::Muted)
            };
            Icon::new(glyph).tint(tint).draw(
                Rect::new(left, lines.label_cy - GLYPH * 0.5, GLYPH, GLYPH),
                list,
            );
            left += GLYPH + GAP;
        }

        let mut right = cell.x + cell.width - PAD_RIGHT;
        let meta_size = s.text_size(TextSize::Meta);
        let mono = theme.mono_font.clone();
        let mono_baseline = meta_size * list.font_vmetrics(mono.as_ref()).baseline_ratio;
        if let Some(meta) = row.meta {
            let block = s.mono_block(
                meta,
                0.0,
                baseline - mono_baseline,
                TextSize::Meta,
                Ink::Caption,
            );
            let (w, _) = list.measure_block(&block);
            right -= w;
            let mut block = block.with_color_f32(dim_ink);
            block.x = right;
            list.text(block);
            right -= GAP;
        }

        let max_w = (right - left).max(0.0);
        let mut label = TextBlock::new(row.label, left, label_top)
            .with_size(label_size)
            .with_color_f32(label_ink)
            .with_font_opt(label_font)
            .with_max_width(max_w)
            .with_ellipsis();
        if !on_accent {
            label = label.with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0);
        }
        if !off && let Some(range) = Self::match_range(row.label, self.highlight) {
            let style = if on_accent {
                TextStyleRange {
                    range,
                    color: None,
                    underline: Underline::Inherit,
                }
            } else {
                TextStyleRange {
                    range,
                    color: Some(s.color(StyleKey::AccentMatch)),
                    underline: Underline::None,
                }
            };
            label = label.with_style_ranges(vec![style]);
        }
        list.text(label);

        if let (Some(sub_cy), Some(sub)) = (lines.sub_cy, row.subtitle) {
            let top = list.x_centered_text_y(sub_cy, meta_size, mono.as_ref());
            list.text(
                s.mono_block(sub, left, top, TextSize::Meta, Ink::Caption)
                    .with_color_f32(dim_ink)
                    .with_max_width(max_w.max(0.0))
                    .with_ellipsis(),
            );
        }
    }

    fn draw_empty(&self, list: &mut DrawList, s: &StyleResolver, rect: Rect) {
        let top = rect.y + EMPTY_PAD_TOP;
        Icon::new(PhosphorIcon::MagnifyingGlass)
            .tint(s.ink(Ink::Empty))
            .draw(
                Rect::new(
                    rect.x + (rect.width - EMPTY_GLYPH) * 0.5,
                    top,
                    EMPTY_GLYPH,
                    EMPTY_GLYPH,
                ),
                list,
            );
        let width = (rect.width - EMPTY_PAD_SIDE * 2.0).max(0.0);
        list.text(
            s.sans_block(
                self.empty,
                rect.x + EMPTY_PAD_SIDE,
                top + EMPTY_GLYPH + EMPTY_GAP,
                TextSize::Row,
                Ink::Dim,
            )
            .with_max_width(width)
            .with_align(TextAlign::Center),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::color::text_color;

    const RECT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 240.0,
        height: 200.0,
    };

    fn at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    /// Draw `rows` in `view` for one frame.
    fn frame(
        view: ListView,
        rows: &[ListRow],
        selected: Option<usize>,
        state: &mut ListState,
        input: &mut InputState,
    ) -> (DrawList, ListViewOutput) {
        crate::map_keyboard(input);
        let theme = Theme::default();
        let mut list = DrawList::new();
        let out = view.draw(
            RECT,
            rows.len(),
            selected,
            state,
            &mut list,
            &StyleResolver::new(&theme),
            input,
            |i| rows[i],
        );
        (list, out)
    }

    fn text<'l>(list: &'l DrawList, content: &str) -> &'l TextBlock {
        list.texts
            .iter()
            .find(|t| t.content == content)
            .unwrap_or_else(|| panic!("no text {content:?}"))
    }

    #[test]
    fn a_two_line_row_shows_its_label_sub_and_meta() {
        let rows = [ListRow::new("Fix websocket reconnect")
            .subtitle("01a01043 · 38 msgs")
            .meta("2h")];
        let view = ListView::new().two_line(true);
        let (mut list, _) = frame(
            view,
            &rows,
            None,
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        assert_eq!(view.row_height(&s), 34.0);

        let meta = text(&list, "2h").clone();
        let (meta_w, _) = list.measure_block(&meta);
        assert!(
            (meta.x + meta_w - (RECT.width - PAD_RIGHT)).abs() < 0.01,
            "the meta ends at the right padding"
        );
        let label = text(&list, "Fix websocket reconnect");
        assert_eq!(label.x, PAD_LEFT);
        assert!(label.ellipsize);
        assert!(
            (label.max_width - (meta.x - GAP - PAD_LEFT)).abs() < 0.01,
            "the label stops short of the meta"
        );
        let sub = text(&list, "01a01043 · 38 msgs");
        assert_eq!(sub.x, PAD_LEFT);
        assert!(sub.y > label.y, "the sub line sits under the label");
        assert_eq!(sub.color, text_color(s.ink(Ink::Caption)));
    }

    #[test]
    fn the_meta_shares_the_labels_baseline() {
        let rows = [ListRow::new("agent-ui").meta("12")];
        let (mut list, _) = frame(
            ListView::new(),
            &rows,
            None,
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        let label = text(&list, "agent-ui").clone();
        let meta = text(&list, "12").clone();
        let theme = Theme::default();
        let sans = list.font_vmetrics(theme.font.as_ref()).baseline_ratio;
        let mono = list.font_vmetrics(theme.mono_font.as_ref()).baseline_ratio;
        let label_baseline = label.y + label.font_size * sans;
        let meta_baseline = meta.y + meta.font_size * mono;
        assert!((label_baseline - meta_baseline).abs() < 0.01);
    }

    #[test]
    fn the_highlight_tints_the_first_match() {
        let rows = [ListRow::new("my-agent-ui")];
        let view = ListView::new().highlight("AGENT");
        let (list, _) = frame(
            view,
            &rows,
            None,
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        let theme = Theme::default();
        let label = text(&list, "my-agent-ui");
        assert_eq!(
            *label.style_ranges,
            vec![TextStyleRange {
                range: 3..8,
                color: Some(theme.accent_match),
                underline: Underline::None,
            }]
        );
    }

    #[test]
    fn a_focused_selection_wears_on_accent_ink_and_underlines_the_match() {
        let rows = [ListRow::new("my-agent-ui")];
        let theme = Theme::default();
        let view = ListView::new().highlight("agent").focused(true);
        let (list, _) = frame(
            view,
            &rows,
            Some(0),
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        let label = text(&list, "my-agent-ui");
        assert_eq!(label.color, text_color(theme.on_accent));
        assert!(label.shadow.is_none(), "no carve on the accent");
        assert_eq!(label.style_ranges[0].color, None);
        assert_eq!(label.style_ranges[0].underline, Underline::Inherit);

        let (list, _) = frame(
            view.focused(false),
            &rows,
            Some(0),
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        let label = text(&list, "my-agent-ui");
        let s = StyleResolver::new(&theme);
        assert_eq!(
            label.color,
            text_color(s.ink(Ink::Max)),
            "held selection without focus"
        );
        assert_eq!(label.style_ranges[0].color, Some(theme.accent_match));
    }

    #[test]
    fn disabled_rows_are_dimmed_unhighlighted_and_never_selected() {
        let rows = [
            ListRow::new("gone-project").meta("missing").disabled(true),
            ListRow::new("agent-ui"),
        ];
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut state = ListState::new();
        let view = ListView::new().highlight("project");
        // The caller still believes row 0 is selected; the view refuses it.
        let (list, out) = frame(view, &rows, Some(0), &mut state, &mut at(-1.0, -1.0));
        let label = text(&list, "gone-project");
        assert_eq!(label.color, text_color(s.ink(Ink::Disabled)));
        assert!(
            label.style_ranges.is_empty(),
            "no highlight on a disabled row"
        );
        assert_eq!(
            text(&list, "missing").color,
            text_color(s.ink(Ink::DisabledGlyph))
        );
        assert_eq!(state.selected_count(), 0);
        assert_eq!(out.select, None);
    }

    #[test]
    fn clicks_select_and_double_clicks_open() {
        let rows = [ListRow::new("a"), ListRow::new("b"), ListRow::new("c")];
        let mut state = ListState::new();
        let mut click = at(50.0, 33.0); // row 1 of 22 px rows
        click.mouse_down = true;
        click.mouse_clicked = true;
        let (_, out) = frame(ListView::new(), &rows, None, &mut state, &mut click);
        assert_eq!((out.select, out.open), (Some(1), None));

        // Reporting the selection the caller already has is not a change.
        let (_, out) = frame(ListView::new(), &rows, Some(1), &mut state, &mut click);
        assert_eq!(out.select, None);

        click.mouse_double_clicked = true;
        let (_, out) = frame(ListView::new(), &rows, Some(1), &mut state, &mut click);
        assert_eq!(out.open, Some(1));
    }

    #[test]
    fn enter_opens_the_selection_of_a_focused_view() {
        let rows = [ListRow::new("a"), ListRow::new("b")];
        let mut state = ListState::new();
        let view = ListView::new().focused(true);
        let mut enter = at(-1.0, -1.0);
        enter.enter_pressed = true;
        let (_, out) = frame(view, &rows, Some(1), &mut state, &mut enter);
        assert_eq!(out.open, Some(1));
    }

    #[test]
    fn an_empty_view_shows_its_message_under_a_magnifier() {
        let (list, _) = frame(
            ListView::new().empty("No projects match \"zz\""),
            &[],
            None,
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        let message = text(&list, "No projects match \"zz\"");
        assert_eq!(message.align, TextAlign::Center);
        assert_eq!(list.icons_msdf.len(), 1);
    }

    #[test]
    fn glyphs_push_the_label_right() {
        let rows = [ListRow::new("src").glyph(PhosphorIcon::Folder)];
        let (list, _) = frame(
            ListView::new(),
            &rows,
            None,
            &mut ListState::new(),
            &mut at(-1.0, -1.0),
        );
        assert_eq!(list.icons_msdf.len(), 1);
        assert_eq!(text(&list, "src").x, PAD_LEFT + GLYPH + GAP);
    }

    #[test]
    fn match_range_is_case_insensitive_and_first_only() {
        assert_eq!(ListView::match_range("Agent-UI agent", "AGENT"), Some(0..5));
        assert_eq!(ListView::match_range("my-agent-ui", "ui"), Some(9..11));
        assert_eq!(ListView::match_range("café", "É"), Some(3..5));
        assert_eq!(ListView::match_range("abc", ""), None);
        assert_eq!(ListView::match_range("abc", "abcd"), None);
        assert_eq!(ListView::match_range("abc", "x"), None);
    }

    #[test]
    fn match_range_handles_non_ascii_text_and_queries() {
        // The Kelvin sign lowercases to an ASCII k.
        assert_eq!(ListView::match_range("\u{212A}ey", "key"), Some(0..5));
        assert_eq!(ListView::match_range("über-app", "APP"), Some(6..9));
        assert_eq!(ListView::match_range("app", "äpp"), None);
        assert_eq!(ListView::match_range("ÄPP", "äpp"), Some(0..4));
    }

    #[test]
    fn the_ascii_match_agrees_with_the_unicode_one() {
        let texts = [
            "",
            "a",
            "Agent-UI",
            "my-agent-ui",
            "AaAaB",
            "x_y-Z 09",
            "ababab",
        ];
        let queries = [
            "a", "A", "ui", "UI", "aab", "AB", "-z 0", "babab", "abababa", "x",
        ];
        for text in texts {
            for query in queries {
                assert_eq!(
                    ListView::match_range(text, query),
                    ListView::match_range_unicode(text, query),
                    "{text:?} / {query:?}"
                );
            }
        }
    }
}
