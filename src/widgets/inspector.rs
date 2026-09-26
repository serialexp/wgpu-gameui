//! Inspector — the property panel assembly (Forge `Inspector`).

use crate::chrome::{Edge, SurfacePainter};
use crate::layout::Rect;
use crate::shadow::CornerRadii;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::{LINE_HEIGHT_RATIO, TextAlign, TextBlock, vcentered_line_y};

use super::material::{self, Tone};
use super::{
    BADGE_HEIGHT, Badge, BadgeTone, Button, DrawContext, DrawList, FocusId, IconKey, Pressable,
    PropertyStack, ScrollState, ScrollView,
};

/// The panel's width (`--inspector-w`).
pub const INSPECTOR_WIDTH: f32 = 272.0;
/// The tallest the body's content gets before it scrolls (Forge
/// `maxBodyHeight`). The body's padding comes on top.
pub const INSPECTOR_MAX_BODY_HEIGHT: f32 = 452.0;

/// Padding around the body's content.
const BODY_PAD: f32 = 11.0;
/// Between the identity block and each top-level body item.
const BODY_GAP: f32 = 12.0;
/// Between the badges, the name well and the path.
const IDENTITY_GAP: f32 = 5.0;
/// Between the badges.
const BADGE_GAP: f32 = 6.0;
/// The name well's height, and its text's padding.
const NAME_H: f32 = 23.0;
const NAME_PAD: f32 = 7.0;
/// How much a multi-selection's name well fades (Forge `opacity: .5`).
const MULTI_FADE: f32 = 0.5;
/// Padding at the header's sides, and between its keys.
const HEADER_PAD: f32 = 3.0;
const HEADER_KEY_GAP: f32 = 1.0;
/// The tab key's label padding (Forge `pad="2px 7px"`) and line height (a
/// Forge `Key`'s `line-height: 1.2`).
const TAB_PAD: (f32, f32) = (7.0, 2.0);
const KEY_LINE: f32 = 1.2;
/// Footer padding and the gap between its keys.
const FOOT_PAD: (f32, f32) = (10.0, 8.0);
const FOOT_GAP: f32 = 5.0;
/// The empty state's padding, gap, glyph size and hint line height.
const EMPTY_PAD: (f32, f32) = (20.0, 46.0);
const EMPTY_GAP: f32 = 7.0;
const EMPTY_GLYPH: f32 = 22.0;
const HINT_LINE: f32 = 1.5;
/// The carved-in shadow under the tab's label (`--carve`).
const CARVE_ALPHA: u8 = 128;

/// What an [`Inspector`] is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorSelection<'a> {
    /// Nothing: the empty state instead of a body, and only the footer's
    /// keys disabled.
    None,
    /// One object: its name, and the path it lives at when known.
    Single {
        /// The object's name, shown in the name well.
        name: &'a str,
        /// Where the object lives ("World / Props / Crate_01").
        path: Option<&'a str>,
    },
    /// Several objects, edited together. The name well reads "—"; mixed
    /// values are the rows' business (see
    /// [`PropertyRow::mixed`](crate::PropertyRow::mixed)).
    Multi {
        /// How many objects are selected.
        count: usize,
    },
}

/// What an [`Inspector`] keeps between frames: the body's scroll, and how
/// tall its content was last frame (content is measured while it is drawn,
/// so sizing and scroll limits run one frame behind).
#[derive(Clone, Debug, Default)]
pub struct InspectorState {
    /// The body's scroll position.
    pub scroll: ScrollState,
    /// The body's content height, padding included, as drawn last frame.
    content_height: f32,
}

impl InspectorState {
    /// A fresh state: scrolled to the top, nothing measured yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The body's content height (padding included) as last drawn.
    pub fn content_height(&self) -> f32 {
        self.content_height
    }
}

/// What happened in an [`Inspector`] this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InspectorOutput {
    /// The body's viewport (empty while nothing is selected).
    pub body: Rect,
    /// The header's pin key was clicked.
    pub pin: bool,
    /// The header's close key was clicked.
    pub close: bool,
    /// Apply was clicked (only possible while dirty).
    pub apply: bool,
    /// Revert was clicked (only possible while dirty).
    pub revert: bool,
    /// Delete was clicked (only possible with a selection).
    pub delete: bool,
}

/// The property panel (Forge `Inspector`): a dock header with the tab and
/// pin/close keys, the selection's identity (badges, name, path), the
/// caller's property groups, and one commit footer — Apply and Revert, live
/// only while there are unapplied edits, and Delete, the one danger key.
///
/// The body is a vertical [`PropertyStack`] handed to a closure; it scrolls
/// once its content passes [`INSPECTOR_MAX_BODY_HEIGHT`]. Content is
/// measured as it is drawn, so [`height`](Self::height) and the scroll limit
/// use the previous frame's measurement.
///
/// ```ignore
/// let inspector = Inspector::new(InspectorSelection::Single { name: "Crate_01", path: Some("World / Props") })
///     .dirty(edited);
/// let h = inspector.height(INSPECTOR_WIDTH, &state, ctx.draw_list, &ctx.styles());
/// let out = inspector.draw_with(Rect::new(x, y, INSPECTOR_WIDTH, h), &mut state, &mut ctx, |body, ctx| {
///     PropertyGroup::new("Transform").draw_in(body, &mut open, ctx, |rows, ctx| { /* rows */ });
/// });
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inspector<'a> {
    selection: InspectorSelection<'a>,
    tab: &'a str,
    dirty: bool,
    max_body_height: f32,
    empty_title: &'a str,
    empty_hint: &'a str,
    focus: Option<FocusId>,
}

/// Where an [`Inspector`]'s parts go.
struct Layout {
    inner: Rect,
    header: Rect,
    body: Rect,
    footer: Rect,
}

impl<'a> Inspector<'a> {
    /// An inspector showing `selection`.
    pub fn new(selection: InspectorSelection<'a>) -> Self {
        Self {
            selection,
            tab: "Inspector",
            dirty: false,
            max_body_height: INSPECTOR_MAX_BODY_HEIGHT,
            empty_title: "Nothing selected",
            empty_hint: "Select an object in the viewport or outliner to edit its properties.",
            focus: None,
        }
    }

    /// The header tab's label ("Inspector" by default).
    #[must_use]
    pub fn tab(mut self, tab: &'a str) -> Self {
        self.tab = tab;
        self
    }

    /// There are unapplied edits: shows the "Edited" badge and enables Apply
    /// and Revert.
    #[must_use]
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    /// Scroll the body past `height` px of content instead of
    /// [`INSPECTOR_MAX_BODY_HEIGHT`].
    #[must_use]
    pub fn max_body_height(mut self, height: f32) -> Self {
        self.max_body_height = height.max(0.0);
        self
    }

    /// The empty state's title and hint, shown while nothing is selected.
    #[must_use]
    pub fn empty_text(mut self, title: &'a str, hint: &'a str) -> Self {
        self.empty_title = title;
        self.empty_hint = hint;
        self
    }

    /// Put the header and footer keys in the Tab ring: pin = `base`, close =
    /// `base + 1`, Apply = `base + 2`, Revert = `base + 3`, Delete =
    /// `base + 4`. The body's controls take their own ids, and come in the
    /// ring between the header and the footer.
    #[must_use]
    pub fn focusable(mut self, base: FocusId) -> Self {
        self.focus = Some(base);
        self
    }

    fn none(&self) -> bool {
        self.selection == InspectorSelection::None
    }

    /// The panel's natural height at `width`: the header, the body (its
    /// content as measured last frame, up to the max), and the footer.
    pub fn height(
        &self,
        width: f32,
        state: &InspectorState,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> f32 {
        let border = s.inspector().surface.border_widths.top;
        let inner_w = (width - 2.0 * border).max(0.0);
        let body = if self.none() {
            self.empty_height(inner_w, list, s)
        } else {
            let content = state
                .content_height
                .max(2.0 * BODY_PAD + self.identity_height(list, s));
            content.min(self.max_body_height + 2.0 * BODY_PAD)
        };
        2.0 * border + Self::header_height(s) + body + Self::footer_height(s)
    }

    fn header_height(s: &StyleResolver) -> f32 {
        s.scalar(StyleKey::DockTabHeight)
    }

    fn footer_height(s: &StyleResolver) -> f32 {
        s.inspector().footer_rule.thickness + 2.0 * FOOT_PAD.1 + s.scalar(StyleKey::ButtonHeight)
    }

    /// The identity block's height: badges, name well, and the path line
    /// when there is one.
    fn identity_height(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let mut h = BADGE_HEIGHT + IDENTITY_GAP + NAME_H;
        if let InspectorSelection::Single { path: Some(p), .. } = self.selection {
            h += IDENTITY_GAP + Self::path_block(p, 0.0, 0.0, 0.0, s, list).1;
        }
        h
    }

    /// The path line's block at `(x, y)`, `width` wide, and its height.
    fn path_block(
        path: &str,
        x: f32,
        y: f32,
        width: f32,
        s: &StyleResolver,
        list: &mut DrawList,
    ) -> (TextBlock, f32) {
        let block = s
            .mono_block(path, x, y, TextSize::Caption, Ink::Dim)
            .with_max_width(width)
            .with_ellipsis();
        let h = list.measure_block(&block).1.ceil();
        (block, h)
    }

    /// The empty state's blocks in a column `width` wide at `x`, each at
    /// y = 0, with their heights.
    fn empty_blocks(
        &self,
        x: f32,
        width: f32,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> [(TextBlock, f32); 3] {
        let glyph = s
            .sans_block("◫", x, 0.0, TextSize::Menu, Ink::Empty)
            .with_size(EMPTY_GLYPH)
            .with_max_width(width)
            .with_align(TextAlign::Center);
        let title = s
            .sans_block(self.empty_title, x, 0.0, TextSize::Menu, Ink::Caption)
            .with_max_width(width)
            .with_align(TextAlign::Center);
        let hint = s
            .sans_block(self.empty_hint, x, 0.0, TextSize::Dense, Ink::Disabled)
            .with_line_height(s.text_size(TextSize::Dense) * HINT_LINE)
            .with_max_width(width)
            .with_align(TextAlign::Center);
        let glyph_h = EMPTY_GLYPH * LINE_HEIGHT_RATIO;
        let title_h = list.measure_block(&title).1;
        let hint_h = list.measure_block(&hint).1;
        [(glyph, glyph_h), (title, title_h), (hint, hint_h)]
    }

    fn empty_height(&self, inner_w: f32, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let column = (inner_w - 2.0 * EMPTY_PAD.0).max(0.0);
        let parts: f32 = self
            .empty_blocks(0.0, column, list, s)
            .iter()
            .map(|(_, h)| h)
            .sum();
        (2.0 * EMPTY_PAD.1 + parts + 2.0 * EMPTY_GAP).ceil()
    }

    fn layout(rect: Rect, s: &StyleResolver) -> Layout {
        let border = s.inspector().surface.border_widths.top;
        let inner = rect.inset(border);
        let header_h = Self::header_height(s).min(inner.height);
        let footer_h = Self::footer_height(s).min((inner.height - header_h).max(0.0));
        let header = Rect::new(inner.x, inner.y, inner.width, header_h);
        let footer = Rect::new(inner.x, inner.bottom() - footer_h, inner.width, footer_h);
        let body = Rect::new(
            inner.x,
            header.bottom(),
            inner.width,
            (footer.y - header.bottom()).max(0.0),
        );
        Layout {
            inner,
            header,
            body,
            footer,
        }
    }

    /// Draw the inspector with an empty body (just the identity block).
    pub fn draw(
        &self,
        rect: Rect,
        state: &mut InspectorState,
        ctx: &mut DrawContext,
    ) -> InspectorOutput {
        self.draw_with(rect, state, ctx, |_, _| {})
    }

    /// Draw the inspector in `rect` (its height normally from
    /// [`height`](Self::height)), calling `body` with a stack under the
    /// identity block, handing out rects 12 px apart, for the property
    /// groups.
    /// The body's context reads the pointer in content space, so its
    /// controls hit-test right however far it is scrolled. `body` is not
    /// called while nothing is selected.
    pub fn draw_with(
        &self,
        rect: Rect,
        state: &mut InspectorState,
        ctx: &mut DrawContext,
        body: impl FnOnce(&mut PropertyStack, &mut DrawContext),
    ) -> InspectorOutput {
        let s = ctx.styles();
        let chrome = s.inspector();
        let painted = chrome
            .shadows
            .iter()
            .fold(rect, |area, shadow| area.union(shadow.ink_rect(rect)));
        ctx.push_debug_scope_rect(super::scope_name("Inspector", self.tab), painted);

        let layout = Self::layout(rect, &s);
        let border = chrome.surface.border_widths.top;
        let radius = (chrome.surface.corner_radii.top_left - border).max(0.0);
        let mut painter = SurfacePainter::new(
            ctx.draw_list,
            rect,
            layout.inner,
            CornerRadii::uniform(radius),
            chrome.surface,
            &chrome.shadows,
            &[],
        );
        painter.paint_pre_content();
        drop(painter);

        let mut out = InspectorOutput::default();
        self.draw_header(layout.header, radius, ctx, &mut out);
        if self.none() {
            self.draw_empty(layout.body, ctx);
        } else {
            out.body = layout.body;
            self.draw_body(layout.body, state, ctx, body);
        }
        self.draw_footer(layout.footer, radius, ctx, &mut out);

        // The border goes over everything inside, as in CSS.
        SurfacePainter::new(
            ctx.draw_list,
            rect,
            layout.inner,
            CornerRadii::default(),
            chrome.surface,
            &[],
            &[],
        )
        .paint_post_content();
        ctx.pop_debug_scope();
        out
    }

    fn draw_header(
        &self,
        header: Rect,
        radius: f32,
        ctx: &mut DrawContext,
        out: &mut InspectorOutput,
    ) {
        let s = ctx.styles();
        let chrome = s.inspector();
        let list = &mut *ctx.draw_list;
        list.paint_quad_background(
            header,
            chrome.header,
            CornerRadii::new(radius, radius, 0.0, 0.0),
        );
        let rule = chrome.header_rule;
        list.edge_line(header, Edge::Bottom, rule.thickness, rule.color);
        let lit = chrome.header_highlight;
        list.edge_line(header, Edge::Top, lit.thickness, lit.color);
        // Keys centre in the header above its rule.
        let row_h = (header.height - rule.thickness).max(0.0);

        // The tab: a held, hollow key sized to its label.
        let size = s.text_size(TextSize::Row);
        let border = s.scalar(StyleKey::BorderWidth);
        let label = s
            .sans_block(self.tab, 0.0, 0.0, TextSize::Row, Ink::Max)
            .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0);
        let label_w = list.measure_block(&label).0;
        let tab_key = Pressable::new()
            .held(true)
            .hollow(true)
            .name(super::scope_name("Tab", self.tab));
        let face_h = (size * KEY_LINE + 2.0 * (TAB_PAD.1 + border)).round();
        let tab_h = face_h + tab_key.travel_px(&s);
        let tab_w = (label_w + 2.0 * (TAB_PAD.0 + border)).ceil();
        let tab = Rect::new(
            header.x + HEADER_PAD,
            header.y + ((row_h - tab_h) * 0.5).round(),
            tab_w,
            tab_h,
        );
        tab_key.draw(tab, ctx, |key, ctx| {
            let face = key.face;
            let mut block = label.clone();
            block.x = face.x + border + TAB_PAD.0;
            block.y = vcentered_line_y(face.y, face.height, size);
            ctx.draw_list.text(block);
        });

        // Pin and close, at the right.
        let key = |glyph| IconKey::glyph(glyph, IconKey::HEADER).tone(Tone::Ghost);
        let [key_w, key_h] = key("×").outer_size(&s);
        let key_y = header.y + ((row_h - key_h) * 0.5).round();
        let close_x = header.right() - HEADER_PAD - key_w;
        let pin_x = close_x - HEADER_KEY_GAP - key_w;
        let (mut pin, mut close) = (key("⌖"), key("×"));
        if let Some(base) = self.focus {
            pin = pin.focusable(base);
            close = close.focusable(base + 1);
        }
        out.pin = pin.draw(Rect::new(pin_x, key_y, key_w, key_h), ctx).clicked;
        out.close = close
            .draw(Rect::new(close_x, key_y, key_w, key_h), ctx)
            .clicked;
    }

    fn draw_empty(&self, body: Rect, ctx: &mut DrawContext) {
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        let column = Rect::new(
            body.x + EMPTY_PAD.0,
            body.y + EMPTY_PAD.1,
            (body.width - 2.0 * EMPTY_PAD.0).max(0.0),
            (body.height - 2.0 * EMPTY_PAD.1).max(0.0),
        );
        let blocks = self.empty_blocks(column.x, column.width, list, &s);
        let total: f32 = blocks.iter().map(|(_, h)| h).sum::<f32>() + 2.0 * EMPTY_GAP;
        let mut y = column.y + ((column.height - total) * 0.5).max(0.0);
        for (mut block, h) in blocks {
            block.y = y;
            list.text(block);
            y += h + EMPTY_GAP;
        }
    }

    fn draw_body(
        &self,
        viewport: Rect,
        state: &mut InspectorState,
        ctx: &mut DrawContext,
        body: impl FnOnce(&mut PropertyStack, &mut DrawContext),
    ) {
        let s = ctx.styles();
        let identity_h = self.identity_height(ctx.draw_list, &s);
        let known = state.content_height.max(2.0 * BODY_PAD + identity_h);
        state.scroll.content_size = [viewport.width, known];

        let view = ScrollView::new(viewport).vertical_only().overlay();
        let mut input = ctx.input.clone();
        let begun = view.begin(&mut state.scroll, ctx.draw_list, &mut input);

        // The content is drawn under a `-offset` transform; move the pointer
        // into the same space, and away from rows scrolled out from under it.
        let (screen_y, consumed) = (input.mouse_y, input.mouse_consumed);
        input.mouse_consumed |= !begun.inner.contains(input.mouse_x, input.mouse_y);
        input.mouse_y += state.scroll.offset[1];
        let content_w = (begun.inner.width - 2.0 * BODY_PAD).max(0.0);
        let mut stack = PropertyStack::new(
            begun.inner.x + BODY_PAD,
            begun.inner.y + BODY_PAD,
            content_w,
            BODY_GAP,
        );
        {
            let mut body_ctx = ctx.reborrow_with_input(&input);
            let identity = stack.take(identity_h);
            self.draw_identity(identity, &mut body_ctx);
            body(&mut stack, &mut body_ctx);
        }
        input.mouse_y = screen_y;
        input.mouse_consumed = consumed;

        state.content_height = stack.bottom() + BODY_PAD - begun.inner.y;
        state.scroll.content_size[1] = state.content_height;
        view.end(&mut state.scroll, ctx.draw_list, &s, &input, begun);
    }

    /// The badges, the name well and the path, stacked in `rect`.
    fn draw_identity(&self, rect: Rect, ctx: &mut DrawContext) {
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        let live = match self.selection {
            InspectorSelection::Multi { count } => format!("{count} objects"),
            _ => "live".to_string(),
        };
        let first = Badge::new(BadgeTone::Live).draw(list, &s, rect.x, rect.y, &live);
        if self.dirty {
            Badge::new(BadgeTone::Stale).draw(
                list,
                &s,
                first.right() + BADGE_GAP,
                rect.y,
                "Edited",
            );
        }

        let well = Rect::new(
            rect.x,
            rect.y + BADGE_HEIGHT + IDENTITY_GAP,
            rect.width,
            NAME_H,
        );
        let multi = matches!(self.selection, InspectorSelection::Multi { .. });
        if multi {
            list.push_tint();
            list.multiply_tint([1.0, 1.0, 1.0, MULTI_FADE]);
        }
        let inner = material::draw_row_well(list, &s, well, false);
        let (name, ink) = match self.selection {
            InspectorSelection::Single { name, .. } => (name, Ink::Value),
            _ => ("—", Ink::Disabled),
        };
        let size = s.text_size(TextSize::Menu);
        let x = inner.x + NAME_PAD;
        list.text(
            s.sans_block(
                name,
                x,
                vcentered_line_y(inner.y, inner.height, size),
                TextSize::Menu,
                ink,
            )
            .with_max_width((inner.right() - NAME_PAD - x).max(0.0))
            .with_ellipsis(),
        );
        if multi {
            list.pop_tint();
        }

        if let InspectorSelection::Single { path: Some(p), .. } = self.selection {
            let y = well.bottom() + IDENTITY_GAP;
            let (block, _) = Self::path_block(p, rect.x, y, rect.width, &s, list);
            list.text(block);
        }
    }

    fn draw_footer(
        &self,
        footer: Rect,
        radius: f32,
        ctx: &mut DrawContext,
        out: &mut InspectorOutput,
    ) {
        let s = ctx.styles();
        let chrome = s.inspector();
        let list = &mut *ctx.draw_list;
        list.paint_quad_background(
            footer,
            chrome.footer,
            CornerRadii::new(0.0, 0.0, radius, radius),
        );
        let rule = chrome.footer_rule;
        list.edge_line(footer, Edge::Top, rule.thickness, rule.color);

        let none = self.none();
        let live = self.dirty && !none;
        let mut keys = [
            Button::new("Apply").tone(Tone::Accent).enabled(live),
            Button::new("Revert").tone(Tone::Ghost).enabled(live),
            Button::new("Delete").tone(Tone::Danger).enabled(!none),
        ];
        if let Some(base) = self.focus {
            for (i, key) in keys.iter_mut().enumerate() {
                *key = key.clone().focusable(base + 2 + i as FocusId);
            }
        }
        let widths = keys
            .each_ref()
            .map(|key| key.intrinsic_size(ctx.draw_list, &s).0.ceil());
        let key_h = s.scalar(StyleKey::ButtonHeight);
        let y = footer.y + rule.thickness + FOOT_PAD.1;
        let apply_x = footer.x + FOOT_PAD.0;
        let revert_x = apply_x + widths[0] + FOOT_GAP;
        let delete_x =
            (footer.right() - FOOT_PAD.0 - widths[2]).max(revert_x + widths[1] + FOOT_GAP);
        let [apply, revert, delete] = keys;
        out.apply = apply.draw(Rect::new(apply_x, y, widths[0], key_h), ctx);
        out.revert = revert.draw(Rect::new(revert_x, y, widths[1], key_h), ctx);
        out.delete = delete.draw(Rect::new(delete_x, y, widths[2], key_h), ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, PROPERTY_GROUP_HEADER_HEIGHT, PropertyGroup, Theme};

    const SINGLE: InspectorSelection = InspectorSelection::Single {
        name: "Crate_01",
        path: Some("World / Props / Crate_01"),
    };
    const BARE: InspectorSelection = InspectorSelection::Single {
        name: "Crate_01",
        path: None,
    };
    /// The surface's border.
    const B: f32 = 1.0;

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

    fn away() -> InputState {
        at(-50.0, -50.0)
    }

    /// Draw `insp` at the origin, [`INSPECTOR_WIDTH`] wide and as tall as
    /// its [`height`](Inspector::height). Returns the output, the list and
    /// the rect.
    fn frame(
        insp: &Inspector,
        state: &mut InspectorState,
        input: &InputState,
        body: impl FnOnce(&mut PropertyStack, &mut DrawContext),
    ) -> (InspectorOutput, DrawList, Rect) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let (out, rect) = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 900.0);
            let s = ctx.styles();
            let h = insp.height(INSPECTOR_WIDTH, state, ctx.draw_list, &s);
            let rect = Rect::new(0.0, 0.0, INSPECTOR_WIDTH, h);
            (insp.draw_with(rect, state, &mut ctx, body), rect)
        };
        (out, list, rect)
    }

    fn has(list: &DrawList, content: &str) -> bool {
        list.texts.iter().any(|t| t.content == content)
    }

    fn styles(theme: &Theme) -> StyleResolver<'_> {
        StyleResolver::new(theme)
    }

    /// The footer keys' centres for an inspector at `rect`: Apply, Revert,
    /// Delete.
    fn footer_keys(rect: Rect, insp: &Inspector) -> [[f32; 2]; 3] {
        let theme = Theme::default();
        let s = styles(&theme);
        let mut list = DrawList::new();
        let key_h = s.scalar(StyleKey::ButtonHeight);
        let y = rect.bottom() - B - FOOT_PAD.1 - key_h * 0.5;
        let mut w = |label: &str| Button::new(label).intrinsic_size(&mut list, &s).0.ceil();
        let (apply, revert, delete) = (w("Apply"), w("Revert"), w("Delete"));
        let _ = insp;
        let apply_x = B + FOOT_PAD.0;
        let revert_x = apply_x + apply + FOOT_GAP;
        let delete_x = rect.right() - B - FOOT_PAD.0 - delete;
        [
            [apply_x + apply * 0.5, y],
            [revert_x + revert * 0.5, y],
            [delete_x + delete * 0.5, y],
        ]
    }

    #[test]
    fn a_single_selection_shows_its_identity() {
        let mut state = InspectorState::new();
        let (out, list, rect) = frame(&Inspector::new(SINGLE), &mut state, &away(), |_, _| {});
        assert!(has(&list, "Inspector"));
        // Badges set their text in capitals.
        assert!(has(&list, "LIVE"));
        assert!(has(&list, "Crate_01"));
        assert!(has(&list, "World / Props / Crate_01"));
        assert!(!has(&list, "EDITED"));
        assert!(!has(&list, "Nothing selected"));
        assert_eq!(out.body.y, B + 24.0);
        assert_eq!(out.body.width, rect.width - 2.0 * B);
        let name = list.texts.iter().find(|t| t.content == "Crate_01").unwrap();
        assert_eq!(name.x, B + BODY_PAD + B + NAME_PAD);
    }

    #[test]
    fn a_multi_selection_counts_and_blanks_the_name() {
        let mut state = InspectorState::new();
        let insp = Inspector::new(InspectorSelection::Multi { count: 3 }).dirty(true);
        let (_, list, _) = frame(&insp, &mut state, &away(), |_, _| {});
        assert!(has(&list, "3 OBJECTS"));
        assert!(has(&list, "EDITED"));
        assert!(has(&list, "—"));
        assert!(!has(&list, "LIVE"));
        // The name well fades as a whole.
        let dash = list.texts.iter().find(|t| t.content == "—").unwrap();
        let theme = Theme::default();
        let ink = styles(&theme).ink(Ink::Disabled);
        let alpha = crate::color::text_color([0.0, 0.0, 0.0, ink[3] * MULTI_FADE]).a();
        assert!((i32::from(dash.color.a()) - i32::from(alpha)).abs() <= 1);
    }

    #[test]
    fn nothing_selected_shows_the_empty_state_and_skips_the_body() {
        let mut state = InspectorState::new();
        let mut called = false;
        let (out, list, rect) = frame(
            &Inspector::new(InspectorSelection::None),
            &mut state,
            &away(),
            |_, _| called = true,
        );
        assert!(!called);
        assert!(has(&list, "◫"));
        assert!(has(&list, "Nothing selected"));
        assert!(!has(&list, "LIVE"));
        assert_eq!(out.body, Rect::default());
        // The column sits inside the design's 46 px padding.
        let glyph = list.texts.iter().find(|t| t.content == "◫").unwrap();
        assert!(glyph.y >= B + 24.0 + EMPTY_PAD.1 - 0.01);
        let hint = list
            .texts
            .iter()
            .find(|t| t.content.starts_with("Select an object"))
            .unwrap();
        assert!(hint.max_width <= rect.width - 2.0 * (B + EMPTY_PAD.0) + 0.01);
    }

    #[test]
    fn the_empty_text_can_be_replaced() {
        let mut state = InspectorState::new();
        let insp = Inspector::new(InspectorSelection::None).empty_text("Rien", "Choisissez");
        let (_, list, _) = frame(&insp, &mut state, &away(), |_, _| {});
        assert!(has(&list, "Rien") && has(&list, "Choisissez"));
    }

    #[test]
    fn the_body_stacks_under_the_identity_and_is_measured() {
        let mut state = InspectorState::new();
        let insp = Inspector::new(BARE);
        let mut rects = Vec::new();
        frame(&insp, &mut state, &away(), |body, _| {
            rects.push(body.take(30.0));
            rects.push(body.take(10.0));
        });
        let identity_h = BADGE_HEIGHT + IDENTITY_GAP + NAME_H;
        let top = B + 24.0 + BODY_PAD;
        assert_eq!(rects[0].y, top + identity_h + BODY_GAP);
        assert_eq!(rects[1].y, rects[0].bottom() + BODY_GAP);
        assert_eq!(rects[0].x, B + BODY_PAD);
        assert_eq!(rects[0].width, INSPECTOR_WIDTH - 2.0 * (B + BODY_PAD));
        assert_eq!(
            state.content_height(),
            rects[1].bottom() + BODY_PAD - (B + 24.0)
        );
    }

    #[test]
    fn height_follows_the_measured_content_up_to_the_max() {
        let theme = Theme::default();
        let s = styles(&theme);
        let mut list = DrawList::new();
        let insp = Inspector::new(BARE).max_body_height(100.0);
        let chrome = 2.0 * B + 24.0 + Inspector::footer_height(&s);
        let identity_h = BADGE_HEIGHT + IDENTITY_GAP + NAME_H;

        let mut state = InspectorState::new();
        let fresh = insp.height(INSPECTOR_WIDTH, &state, &mut list, &s);
        assert_eq!(fresh, chrome + 2.0 * BODY_PAD + identity_h);

        state.content_height = 90.0;
        assert_eq!(
            insp.height(INSPECTOR_WIDTH, &state, &mut list, &s),
            chrome + 90.0
        );
        state.content_height = 900.0;
        assert_eq!(
            insp.height(INSPECTOR_WIDTH, &state, &mut list, &s),
            chrome + 100.0 + 2.0 * BODY_PAD
        );
    }

    #[test]
    fn apply_and_revert_wait_for_edits_and_delete_for_a_selection() {
        let clean = Inspector::new(SINGLE);
        let dirty = Inspector::new(SINGLE).dirty(true);
        let none = Inspector::new(InspectorSelection::None).dirty(true);
        let press = |insp: &Inspector, key: usize| {
            let mut state = InspectorState::new();
            let (_, _, rect) = frame(insp, &mut state, &away(), |_, _| {});
            let [x, y] = footer_keys(rect, insp)[key];
            let (out, _, _) = frame(insp, &mut state, &click(x, y), |_, _| {});
            out
        };
        assert!(!press(&clean, 0).apply);
        assert!(!press(&clean, 1).revert);
        assert!(press(&dirty, 0).apply);
        assert!(press(&dirty, 1).revert);
        assert!(press(&clean, 2).delete);
        assert!(!press(&none, 0).apply);
        assert!(!press(&none, 1).revert);
        assert!(!press(&none, 2).delete);
    }

    #[test]
    fn the_header_keys_pin_and_close() {
        let theme = Theme::default();
        let s = styles(&theme);
        let [w, h] = IconKey::glyph("×", IconKey::HEADER)
            .tone(Tone::Ghost)
            .outer_size(&s);
        let close = [INSPECTOR_WIDTH - B - HEADER_PAD - w * 0.5, B + h * 0.5];
        let pin = [close[0] - w - HEADER_KEY_GAP, close[1]];
        let insp = Inspector::new(SINGLE);
        let mut state = InspectorState::new();
        let (out, _, _) = frame(&insp, &mut state, &click(close[0], close[1]), |_, _| {});
        assert!(out.close && !out.pin);
        let (out, _, _) = frame(&insp, &mut state, &click(pin[0], pin[1]), |_, _| {});
        assert!(out.pin && !out.close);
        let consumed = InputState {
            mouse_consumed: true,
            ..click(close[0], close[1])
        };
        let (out, _, _) = frame(&insp, &mut state, &consumed, |_, _| {});
        assert!(!out.close);
    }

    /// Three closed groups under a bare identity, in a body that scrolls
    /// past 60 px. Returns which groups were clicked.
    fn groups(state: &mut InspectorState, input: &InputState) -> [bool; 3] {
        let insp = Inspector::new(BARE).max_body_height(60.0);
        let mut open = [false; 3];
        let mut clicked = [false; 3];
        frame(&insp, state, input, |body, ctx| {
            for (i, title) in ["A", "B", "C"].into_iter().enumerate() {
                clicked[i] = PropertyGroup::new(title).draw_in(body, &mut open[i], ctx, |_, _| {});
            }
        });
        clicked
    }

    #[test]
    fn scrolled_rows_hit_test_where_they_are_drawn() {
        let mut state = InspectorState::new();
        groups(&mut state, &away());
        let identity_h = BADGE_HEIGHT + IDENTITY_GAP + NAME_H;
        let body_top = B + 24.0;
        let b_top =
            body_top + BODY_PAD + identity_h + 2.0 * BODY_GAP + PROPERTY_GROUP_HEADER_HEIGHT;
        assert_eq!(
            state.content_height(),
            BODY_PAD + identity_h + 3.0 * (BODY_GAP + PROPERTY_GROUP_HEADER_HEIGHT) + BODY_PAD
        );

        // Scrolled 60 px, group B is drawn 60 px higher; a click there
        // reaches it, and not whatever sits at that height unscrolled.
        state.scroll.snap_to(1, 60.0);
        let on_b = b_top - 60.0 + PROPERTY_GROUP_HEADER_HEIGHT * 0.5;
        assert_eq!(
            groups(&mut state, &click(150.0, on_b)),
            [false, true, false]
        );

        // Unscrolled, the same spot is the identity block: no group.
        state.scroll.snap_to(1, 0.0);
        assert_eq!(groups(&mut state, &click(150.0, on_b)), [false; 3]);
    }

    #[test]
    fn rows_scrolled_out_of_the_body_take_no_clicks() {
        let mut state = InspectorState::new();
        groups(&mut state, &away());
        let identity_h = BADGE_HEIGHT + IDENTITY_GAP + NAME_H;
        let body_top = B + 24.0;
        let viewport_h = 60.0 + 2.0 * BODY_PAD;
        // Group C sits past the body's bottom, under the footer.
        let c_top =
            body_top + BODY_PAD + identity_h + 3.0 * BODY_GAP + 2.0 * PROPERTY_GROUP_HEADER_HEIGHT;
        assert!(c_top > body_top + viewport_h);
        assert_eq!(groups(&mut state, &click(150.0, c_top + 5.0)), [false; 3]);
        // And a row scrolled up past the body's top takes no click in the
        // part hidden behind the header, only in the part still showing.
        state.scroll.snap_to(1, 80.0);
        let a_top = body_top + BODY_PAD + identity_h + BODY_GAP - 80.0;
        let a_bottom = a_top + PROPERTY_GROUP_HEADER_HEIGHT;
        assert!(a_top < body_top - 3.0 && a_bottom > body_top + 3.0);
        assert_eq!(
            groups(&mut state, &click(150.0, body_top - 3.0)),
            [false; 3]
        );
        assert_eq!(
            groups(&mut state, &click(150.0, body_top + 3.0)),
            [true, false, false]
        );
    }

    #[test]
    fn the_keys_join_the_tab_ring_around_the_body() {
        let theme = Theme::default();
        let insp = Inspector::new(SINGLE).dirty(true).focusable(10);
        let mut state = InspectorState::new();
        let mut focus = FocusState::new();
        let tab = InputState {
            nav: crate::NavInput {
                next: true,
                ..Default::default()
            },
            ..away()
        };
        let mut order = Vec::new();
        // The ring is learnt in the first frame; each later Tab moves on.
        for input in std::iter::once(away()).chain(std::iter::repeat_n(tab, 6)) {
            let mut list = DrawList::new();
            focus.begin_frame(&input);
            {
                let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 900.0);
                let rect = Rect::new(0.0, 0.0, INSPECTOR_WIDTH, 300.0);
                insp.draw_with(rect, &mut state, &mut ctx, |_, ctx| ctx.register_focus(99));
            }
            focus.end_frame(None);
            order.extend(focus.focused());
        }
        assert_eq!(order, [10, 11, 99, 12, 13, 14]);
    }
}
