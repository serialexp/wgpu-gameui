//! Sheet — the raised dialog surface (Forge `Sheet`).

use crate::chrome::{Edge, SurfacePainter};
use crate::layout::Rect;
use crate::shadow::CornerRadii;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::material::Tone;
use super::{Button, DrawContext, DrawList, FocusId, STATUS_ICON_SIZE, Severity, StatusIcon};

/// Forge's default sheet width.
pub const SHEET_WIDTH: f32 = 340.0;
/// The backdrop blur Forge puts behind a sheet, in px, for
/// [`UiRenderer::blur_backdrop`](crate::UiRenderer::blur_backdrop).
pub const SHEET_BLUR: f32 = 18.0;

/// Header padding: 12 px top and bottom, 14 px at the sides.
const HEAD_PAD_Y: f32 = 12.0;
const PAD_X: f32 = 14.0;
/// Between the tone icon and the text column.
const ICON_GAP: f32 = 11.0;
/// Between the title and the description.
const TITLE_GAP: f32 = 6.0;
/// Between the description text and the content under it.
const CONTENT_GAP: f32 = 8.0;
/// Padding around the body.
const BODY_PAD: f32 = 14.0;
/// Footer padding: 9 px top and bottom, 12 px at the sides.
const FOOT_PAD_Y: f32 = 9.0;
const FOOT_PAD_X: f32 = 12.0;
/// Between footer keys.
const KEY_GAP: f32 = 6.0;
/// Between the footer's lead slot and a leading key.
const LEAD_GAP: f32 = 8.0;
/// Description line height, in em.
const DESCRIPTION_LEADING: f32 = 1.55;
/// Room kept around a centred sheet: 24 px above and below, 12 at the sides.
const PLACE_PAD: (f32, f32) = (12.0, 24.0);

/// One key in a sheet's footer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetAction<'a> {
    label: &'a str,
    tone: Tone,
    enabled: bool,
    leading: bool,
    space_after: f32,
}

impl<'a> SheetAction<'a> {
    /// A default key reading `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            tone: Tone::Default,
            enabled: true,
            leading: false,
            space_after: 0.0,
        }
    }

    /// Extra room after the key, on top of the usual gap: sets a secondary
    /// key apart from the pair it sits beside.
    #[must_use]
    pub fn space_after(mut self, px: f32) -> Self {
        self.space_after = px.max(0.0);
        self
    }

    /// The key's face: [`Tone::Accent`] for the primary action,
    /// [`Tone::Danger`] for a destructive one, [`Tone::Ghost`] for a quiet
    /// one.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// A disabled key fades and ignores clicks.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Put the key at the footer's start instead of its end (a "Don't save"
    /// beside the lead slot).
    #[must_use]
    pub fn leading(mut self, leading: bool) -> Self {
        self.leading = leading;
        self
    }
}

/// Where [`Sheet::place`] puts the sheet in its bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SheetAlign {
    /// In the middle.
    #[default]
    Center,
    /// Near the top.
    Start,
}

/// A caller-filled part of a sheet, handed to [`Sheet::draw_with`]'s fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetSlot {
    /// Under the description, in the text column ([`Sheet::content`]).
    Content,
    /// Full width under the header ([`Sheet::body`]).
    Body,
    /// The footer's start ([`Sheet::lead`]).
    Lead,
}

/// What one [`Sheet::draw`] laid out and what was clicked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SheetOutput {
    /// The whole sheet.
    pub rect: Rect,
    /// The [`content`](Sheet::content) slot under the description, in the
    /// header's text column. Zero height without one.
    pub content: Rect,
    /// The [`body`](Sheet::body) slot, full width under the header. Zero
    /// height without one.
    pub body: Rect,
    /// The footer's [`lead`](Sheet::lead) slot, left of the keys. Zero width
    /// without one.
    pub lead: Rect,
    /// The index of the action clicked (or activated from the keyboard)
    /// this frame.
    pub clicked: Option<usize>,
}

/// The raised dialog surface (Forge `Sheet`): a header with a title and a
/// description, a free body, and a footer of keys.
///
/// It lives in the page: no backdrop, no focus trap. [`Modal`](super::Modal)
/// is a sheet over a backdrop. The caller owns the body: give the slot a
/// height, then draw into the rect the sheet hands back.
///
/// ```ignore
/// let actions = [SheetAction::new("Cancel"), SheetAction::new("Export").tone(Tone::Accent)];
/// let sheet = Sheet::new().title("Export").description("Pick a format.").body(60.0).actions(&actions);
/// let rect = sheet.place(screen, SheetAlign::Center, list, &style);
/// let out = sheet.draw(rect, &mut ctx);
/// draw_format_picker(out.body, &mut ctx);
/// if out.clicked == Some(1) { export(); }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheet<'a> {
    title: Option<&'a str>,
    description: Option<&'a str>,
    tone: Option<Severity>,
    meta: Option<&'a str>,
    actions: &'a [SheetAction<'a>],
    content: Option<f32>,
    body: Option<f32>,
    lead: f32,
    width: f32,
    focus_base: Option<FocusId>,
}

impl Default for Sheet<'_> {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
            tone: None,
            meta: None,
            actions: &[],
            content: None,
            body: None,
            lead: 0.0,
            width: SHEET_WIDTH,
            focus_base: None,
        }
    }
}

/// A sheet's parts, positioned in a rect.
struct Layout {
    height: f32,
    icon: Option<(f32, f32)>,
    title: Option<TextBlock>,
    description: Option<TextBlock>,
    content: Rect,
    body: Rect,
    footer: Option<Rect>,
}

impl<'a> Sheet<'a> {
    /// An empty sheet, [`SHEET_WIDTH`] wide.
    pub fn new() -> Self {
        Self::default()
    }

    /// The semibold title at the top.
    #[must_use]
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// The wrapped text under the title.
    #[must_use]
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    /// A [`StatusIcon`] beside the title.
    #[must_use]
    pub fn tone(mut self, tone: Severity) -> Self {
        self.tone = Some(tone);
        self
    }

    /// Muted text at the footer's start.
    #[must_use]
    pub fn meta(mut self, meta: &'a str) -> Self {
        self.meta = Some(meta);
        self
    }

    /// The footer keys, left to right; the primary one goes last.
    #[must_use]
    pub fn actions(mut self, actions: &'a [SheetAction<'a>]) -> Self {
        self.actions = actions;
        self
    }

    /// Reserve `height` px under the description, in the text column beside
    /// the icon, for the caller to draw into ([`SheetOutput::content`]).
    #[must_use]
    pub fn content(mut self, height: f32) -> Self {
        self.content = Some(height.max(0.0));
        self
    }

    /// Reserve a `height` px body under the header, full width, for the
    /// caller to draw into ([`SheetOutput::body`]).
    #[must_use]
    pub fn body(mut self, height: f32) -> Self {
        self.body = Some(height.max(0.0));
        self
    }

    /// Reserve `width` px at the footer's start, before any leading key,
    /// for the caller to draw into ([`SheetOutput::lead`]): a "Don't ask
    /// again" checkbox.
    #[must_use]
    pub fn lead(mut self, width: f32) -> Self {
        self.lead = width.max(0.0);
        self
    }

    /// The sheet's width (default [`SHEET_WIDTH`]).
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    /// Put the footer keys in the Tab ring as `base`, `base + 1`, … in
    /// action order.
    #[must_use]
    pub fn focusable(mut self, base: FocusId) -> Self {
        self.focus_base = Some(base);
        self
    }

    /// The focus id of action `index`, when the sheet is
    /// [`focusable`](Self::focusable).
    pub fn action_focus_id(&self, index: usize) -> Option<FocusId> {
        self.focus_base.map(|base| base + index as FocusId)
    }

    /// The sheet's height at `width`.
    pub fn height(&self, width: f32, list: &mut DrawList, s: &StyleResolver) -> f32 {
        self.layout(Rect::new(0.0, 0.0, width, 0.0), list, s).height
    }

    /// A rect for the sheet in `bounds`: its width (shrunk to fit), its
    /// height at that width, centred across, and centred down or near the
    /// top per `align`.
    pub fn place(
        &self,
        bounds: Rect,
        align: SheetAlign,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> Rect {
        let pad_y = PLACE_PAD.1;
        let width = self.placed_width(bounds);
        let height = self.height(width, list, s);
        let x = bounds.x + (bounds.width - width) * 0.5;
        let y = match align {
            SheetAlign::Center => bounds.y + ((bounds.height - height) * 0.5).max(pad_y),
            SheetAlign::Start => bounds.y + pad_y,
        };
        Rect::new(x.round(), y.round(), width, height)
    }

    /// The width [`place`](Self::place) gives the sheet in `bounds`.
    pub fn placed_width(&self, bounds: Rect) -> f32 {
        self.width.min(bounds.width - 2.0 * PLACE_PAD.0).max(0.0)
    }

    /// The width of the header's text column (and so of the
    /// [`content`](Self::content) slot) when the sheet is `width` wide.
    pub fn content_width(&self, width: f32, s: &StyleResolver) -> f32 {
        let border = s.sheet().surface.border_widths.top;
        let icon = if self.tone.is_some() {
            STATUS_ICON_SIZE + ICON_GAP
        } else {
            0.0
        };
        (width - 2.0 * (border + PAD_X) - icon).max(0.0)
    }

    fn has_head(&self) -> bool {
        self.title.is_some() || self.description.is_some() || self.content.is_some()
    }

    fn has_footer(&self) -> bool {
        !self.actions.is_empty() || self.meta.is_some() || self.lead > 0.0
    }

    /// Lay the parts out from `rect`'s top-left at its width. The height of
    /// `rect` is ignored; the layout's own height is returned.
    fn layout(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) -> Layout {
        let border = s.sheet().surface.border_widths.top;
        let inner_x = rect.x + border;
        let inner_w = (rect.width - 2.0 * border).max(0.0);
        let mut y = rect.y + border;

        let mut icon = None;
        let mut title = None;
        let mut description = None;
        let mut content = Rect::new(inner_x + PAD_X, y, 0.0, 0.0);
        if self.has_head() {
            let top = y + HEAD_PAD_Y;
            let mut x = inner_x + PAD_X;
            let mut icon_h: f32 = 0.0;
            if self.tone.is_some() {
                // `padding-top: 1px` sets the icon on the title's line.
                icon = Some((x, top + 1.0));
                icon_h = STATUS_ICON_SIZE + 1.0;
                x += STATUS_ICON_SIZE + ICON_GAP;
            }
            let column_w = (inner_x + inner_w - PAD_X - x).max(0.0);
            let mut cy = top;
            if let Some(text) = self.title {
                let size = s.text_size(TextSize::Menu) + 1.0;
                let block = s
                    .sans_block(text, x, cy, TextSize::Menu, Ink::Value)
                    .with_size(size)
                    .with_weight(crate::Weight::SEMIBOLD)
                    .with_max_width(column_w);
                cy += list.measure_block(&block).1;
                title = Some(block);
            }
            let has_details = self.description.is_some() || self.content.is_some();
            if title.is_some() && has_details {
                cy += TITLE_GAP;
            }
            if let Some(text) = self.description {
                let size = s.text_size(TextSize::Menu);
                let block = s
                    .sans_block(text, x, cy, TextSize::Menu, Ink::Glyph)
                    .with_line_height(size * DESCRIPTION_LEADING)
                    .with_max_width(column_w);
                cy += list.measure_block(&block).1;
                description = Some(block);
                if self.content.is_some() {
                    cy += CONTENT_GAP;
                }
            }
            if let Some(h) = self.content {
                content = Rect::new(x, cy, column_w, h);
                cy += h;
            }
            y = top + (cy - top).max(icon_h) + HEAD_PAD_Y;
            if !has_details && title.is_none() {
                content.y = y;
            }
        }

        let mut body = Rect::new(inner_x + BODY_PAD, y, 0.0, 0.0);
        if let Some(h) = self.body {
            let top = if self.has_head() { 0.0 } else { BODY_PAD };
            body = Rect::new(
                inner_x + BODY_PAD,
                y + top,
                (inner_w - 2.0 * BODY_PAD).max(0.0),
                h,
            );
            y = body.bottom() + BODY_PAD;
        }

        let mut footer = None;
        if self.has_footer() {
            let rule = s.sheet().footer_rule.thickness;
            let h = rule + FOOT_PAD_Y * 2.0 + s.scalar(StyleKey::ButtonHeight);
            footer = Some(Rect::new(inner_x, y, inner_w, h));
            y += h;
        }

        Layout {
            height: (y + border - rect.y).ceil(),
            icon,
            title,
            description,
            content,
            body,
            footer,
        }
    }

    /// Draw the sheet in `rect` (its height from [`height`](Self::height)
    /// or [`place`](Self::place)). Returns the slots and the clicked action.
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> SheetOutput {
        self.draw_with(rect, ctx, |_, _, _| {})
    }

    /// [`draw`](Self::draw), calling `fill` for each reserved slot (content,
    /// body, lead, in that order) before the keys are drawn, so focusable
    /// controls in the slots come in the Tab ring in the order they sit on
    /// screen. The slots are also returned, as from `draw`.
    pub fn draw_with(
        &self,
        rect: Rect,
        ctx: &mut DrawContext,
        mut fill: impl FnMut(SheetSlot, Rect, &mut DrawContext),
    ) -> SheetOutput {
        let s = ctx.styles();
        let chrome = s.sheet();
        // The sheet paints its drop shadow past `rect`; declare that too.
        let painted = chrome
            .shadows
            .iter()
            .fold(rect, |area, shadow| area.union(shadow.ink_rect(rect)));
        ctx.push_debug_scope_rect(
            super::scope_name("Sheet", self.title.unwrap_or_default()),
            painted,
        );
        let layout = self.layout(rect, ctx.draw_list, &s);
        let border = chrome.surface.border_widths.top;
        let inner = rect.inset(border);
        let radius = chrome.surface.corner_radii.bottom_left;
        let mut painter = SurfacePainter::new(
            ctx.draw_list,
            rect,
            inner,
            CornerRadii::uniform((radius - border).max(0.0)),
            chrome.surface,
            &chrome.shadows,
            &[],
        );
        painter.paint_pre_content();
        drop(painter);

        if let Some((x, y)) = layout.icon {
            if let Some(tone) = self.tone {
                StatusIcon::new(tone).draw(ctx.draw_list, &s, x, y);
            }
        }
        if let Some(title) = layout.title {
            ctx.draw_list.text(title);
        }
        if let Some(description) = layout.description {
            ctx.draw_list.text(description);
        }

        if self.content.is_some() {
            fill(SheetSlot::Content, layout.content, ctx);
        }
        if self.body.is_some() {
            fill(SheetSlot::Body, layout.body, ctx);
        }
        let mut out = SheetOutput {
            rect,
            content: layout.content,
            body: layout.body,
            lead: Rect::default(),
            clicked: None,
        };
        if let Some(footer) = layout.footer {
            out.lead = self.draw_footer(footer, radius - border, ctx, &mut fill, &mut out.clicked);
        }

        // The border goes over everything inside, as in CSS.
        SurfacePainter::new(
            ctx.draw_list,
            rect,
            inner,
            CornerRadii::default(),
            chrome.surface,
            &[],
            &[],
        )
        .paint_post_content();
        ctx.pop_debug_scope();
        out
    }

    /// Draw the footer strip, its lead slot (through `fill`), its meta text
    /// and its keys; returns the lead slot.
    fn draw_footer(
        &self,
        footer: Rect,
        radius: f32,
        ctx: &mut DrawContext,
        fill: &mut impl FnMut(SheetSlot, Rect, &mut DrawContext),
        clicked: &mut Option<usize>,
    ) -> Rect {
        let s = ctx.styles();
        let chrome = s.sheet();
        let list = &mut *ctx.draw_list;
        list.paint_quad_background(
            footer,
            chrome.footer,
            CornerRadii::new(0.0, 0.0, radius.max(0.0), radius.max(0.0)),
        );
        let rule = chrome.footer_rule;
        list.edge_line(footer, Edge::Top, rule.thickness, rule.color);

        let key_h = s.scalar(StyleKey::ButtonHeight);
        let key_y = footer.y + rule.thickness + FOOT_PAD_Y;
        let key_w = |list: &mut DrawList, a: &SheetAction| {
            Button::new(a.label).intrinsic_size(list, &s).0.ceil()
        };

        let mut x = footer.x + FOOT_PAD_X;
        let lead = Rect::new(x, key_y, self.lead, key_h);
        if self.lead > 0.0 {
            x += self.lead + LEAD_GAP;
            fill(SheetSlot::Lead, lead, ctx);
        }
        let list = &mut *ctx.draw_list;
        // Each trailing key and the space after it, but none after the last.
        let mut trailing_w = 0.0;
        let mut last_space = 0.0;
        for action in self.actions.iter().filter(|a| !a.leading) {
            last_space = KEY_GAP + action.space_after;
            trailing_w += key_w(list, action) + last_space;
        }
        let trailing_w = (trailing_w - last_space).max(0.0);
        let right = footer.right() - FOOT_PAD_X;

        // Keys are drawn left to right, leading ones first, so the Tab ring
        // follows what the eye reads.
        let order = self
            .actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.leading)
            .chain(self.actions.iter().enumerate().filter(|(_, a)| !a.leading));
        let mut trailing_x = right - trailing_w;
        let mut meta_x = None;
        for (i, action) in order {
            let w = key_w(ctx.draw_list, action);
            let advance = w + KEY_GAP + action.space_after;
            let kx = if action.leading {
                let kx = x;
                x += advance;
                kx
            } else {
                meta_x.get_or_insert(x);
                let kx = trailing_x;
                trailing_x += advance;
                kx
            };
            let mut key = Button::new(action.label)
                .tone(action.tone)
                .enabled(action.enabled);
            if let Some(id) = self.action_focus_id(i) {
                key = key.focusable(id);
            }
            if key.draw(Rect::new(kx, key_y, w, key_h), ctx) && clicked.is_none() {
                *clicked = Some(i);
            }
        }

        if let Some(meta) = self.meta {
            let left = meta_x.unwrap_or(x);
            let limit = (right - trailing_w - KEY_GAP).max(left);
            let size = s.text_size(TextSize::Row);
            let block = s
                .sans_block(
                    meta,
                    left,
                    crate::text::vcentered_line_y(key_y, key_h, size),
                    TextSize::Row,
                    Ink::Muted,
                )
                .with_max_width(limit - left)
                .with_ellipsis();
            ctx.draw_list.text(block);
        }
        lead
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::text_color;
    use crate::{FocusState, InputState, Theme};

    fn draw_sheet(sheet: &Sheet, input: &InputState) -> (DrawList, SheetOutput) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let s = StyleResolver::new(&theme);
        let h = sheet.height(SHEET_WIDTH, &mut list, &s);
        let rect = Rect::new(10.0, 10.0, SHEET_WIDTH, h);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        let out = sheet.draw(rect, &mut ctx);
        (list, out)
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..InputState::default()
        }
    }

    #[test]
    fn declared_area_covers_the_drop_shadow() {
        let keys = [SheetAction::new("OK")];
        let sheet = Sheet::new().title("Export").actions(&keys);
        let (list, _) = draw_sheet(&sheet, &idle());
        assert!(list.shadow_instance_count() > 0, "the sheet casts shadows");
        let screen = Rect::new(-200.0, -200.0, 1200.0, 1000.0);
        let report = crate::debug::DebugReport::from_draw_list(&list, screen);
        assert!(
            !report
                .problems()
                .iter()
                .any(|p| p.code() == "overflows_declared"),
            "{:?}",
            report.problems()
        );
    }

    #[test]
    fn title_and_description_stack_in_the_header() {
        let sheet = Sheet::new().title("Export").description("Pick a format.");
        let (list, out) = draw_sheet(&sheet, &idle());
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let title = &list.texts[0];
        let description = &list.texts[1];
        assert_eq!(title.content, "Export");
        assert_eq!(title.color, text_color(s.ink(Ink::Value)));
        assert_eq!(description.color, text_color(s.ink(Ink::Glyph)));
        assert_eq!(title.x, out.rect.x + 1.0 + PAD_X);
        assert_eq!(title.y, out.rect.y + 1.0 + HEAD_PAD_Y);
        assert!(description.y > title.y + TITLE_GAP);
    }

    #[test]
    fn tone_icon_pushes_the_text_right() {
        let plain = Sheet::new().title("Delete");
        let toned = plain.tone(Severity::Warning);
        let (a, _) = draw_sheet(&plain, &idle());
        let (b, _) = draw_sheet(&toned, &idle());
        let title = |l: &DrawList| l.texts.iter().find(|t| t.content == "Delete").unwrap().x;
        assert_eq!(title(&b) - title(&a), STATUS_ICON_SIZE + ICON_GAP);
    }

    #[test]
    fn body_slot_sits_under_the_header_and_grows_the_sheet() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let head = Sheet::new().title("Export");
        let with_body = head.body(60.0);
        let grow =
            with_body.height(SHEET_WIDTH, &mut list, &s) - head.height(SHEET_WIDTH, &mut list, &s);
        assert_eq!(grow, 60.0 + BODY_PAD);
        let (_, out) = draw_sheet(&with_body, &idle());
        assert_eq!(out.body.height, 60.0);
        assert_eq!(out.body.x, out.rect.x + 1.0 + BODY_PAD);
        assert_eq!(out.body.width, SHEET_WIDTH - 2.0 - 2.0 * BODY_PAD);
    }

    #[test]
    fn body_without_a_header_is_padded_all_round() {
        let (_, out) = draw_sheet(&Sheet::new().body(40.0), &idle());
        assert_eq!(out.body.y, out.rect.y + 1.0 + BODY_PAD);
        assert_eq!(out.rect.height, 2.0 + 40.0 + 2.0 * BODY_PAD);
    }

    #[test]
    fn primary_key_sits_flush_right_and_reports_its_index() {
        let actions = [
            SheetAction::new("Cancel"),
            SheetAction::new("Export").tone(Tone::Accent),
        ];
        let sheet = Sheet::new().title("Export").actions(&actions);
        let (_, out) = draw_sheet(&sheet, &idle());
        // Click in the footer's last 20 px: the primary key.
        let x = out.rect.right() - 1.0 - FOOT_PAD_X - 10.0;
        let y = out.rect.bottom() - 1.0 - FOOT_PAD_Y - 10.0;
        let click = InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            ..InputState::default()
        };
        let (_, out) = draw_sheet(&sheet, &click);
        assert_eq!(out.clicked, Some(1));
    }

    #[test]
    fn consumed_input_clicks_nothing() {
        let actions = [SheetAction::new("OK")];
        let sheet = Sheet::new().title("Done").actions(&actions);
        let (_, out) = draw_sheet(&sheet, &idle());
        let click = InputState {
            mouse_x: out.rect.right() - 1.0 - FOOT_PAD_X - 5.0,
            mouse_y: out.rect.bottom() - 1.0 - FOOT_PAD_Y - 5.0,
            mouse_clicked: true,
            ..InputState::default()
        }
        .consumed();
        let (_, out) = draw_sheet(&sheet, &click);
        assert_eq!(out.clicked, None);
    }

    #[test]
    fn lead_slot_and_leading_key_start_the_footer() {
        let actions = [
            SheetAction::new("Don't save").leading(true),
            SheetAction::new("Cancel"),
            SheetAction::new("Save").tone(Tone::Accent),
        ];
        let sheet = Sheet::new().title("Unsaved").lead(90.0).actions(&actions);
        let (list, out) = draw_sheet(&sheet, &idle());
        assert_eq!(out.lead.x, out.rect.x + 1.0 + FOOT_PAD_X);
        assert_eq!(out.lead.width, 90.0);
        let label_x = |label: &str| list.texts.iter().find(|t| t.content == label).unwrap().x;
        assert!(label_x("Don't save") > out.lead.right());
        assert!(label_x("Don't save") < label_x("Cancel"));
        assert!(label_x("Cancel") < label_x("Save"));
    }

    #[test]
    fn keys_join_the_tab_ring_in_reading_order() {
        let actions = [
            SheetAction::new("Cancel"),
            SheetAction::new("Alt").leading(true),
            SheetAction::new("OK"),
        ];
        let sheet = Sheet::new().actions(&actions).focusable(100);
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = InputState {
            nav: crate::NavInput {
                next: true,
                ..Default::default()
            },
            ..idle()
        };
        focus.begin_frame(&input);
        let s = StyleResolver::new(&theme);
        let h = sheet.height(SHEET_WIDTH, &mut list, &s);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        sheet.draw(Rect::new(0.0, 0.0, SHEET_WIDTH, h), &mut ctx);
        focus.end_frame(None);
        // The leading "Alt" (index 1) is first on screen, so first in the ring.
        assert_eq!(focus.focused(), Some(101));
    }

    #[test]
    fn place_centres_and_shrinks_to_fit() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let sheet = Sheet::new().title("Export").body(40.0);
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = sheet.place(bounds, SheetAlign::Center, &mut list, &s);
        assert_eq!(r.width, SHEET_WIDTH);
        assert_eq!(r.x, 230.0);
        assert!((r.y + r.height * 0.5 - 300.0).abs() <= 1.0);
        let top = sheet.place(bounds, SheetAlign::Start, &mut list, &s);
        assert_eq!(top.y, PLACE_PAD.1);
        let narrow = sheet.place(
            Rect::new(0.0, 0.0, 200.0, 600.0),
            SheetAlign::Center,
            &mut list,
            &s,
        );
        assert_eq!(narrow.width, 200.0 - 2.0 * PLACE_PAD.0);
    }

    #[test]
    fn meta_text_is_muted_on_the_left() {
        let actions = [SheetAction::new("OK")];
        let sheet = Sheet::new().meta("3 files").actions(&actions);
        let (list, out) = draw_sheet(&sheet, &idle());
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let meta = list.texts.iter().find(|t| t.content == "3 files").unwrap();
        assert_eq!(meta.x, out.rect.x + 1.0 + FOOT_PAD_X);
        assert_eq!(meta.color, text_color(s.ink(Ink::Muted)));
    }
}
