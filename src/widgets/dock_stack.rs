//! Dock stack — collapsible sections stacked inside a dock (Forge `DockStack`
//! and `DockSection`).
//!
//! Use it when a dock holds more than one list or block (Projects + Sessions,
//! Outliner + Layers). Each [`DockSection`] gets a raised 22 px header — a
//! disclosure caret, a mono-capitals title, an optional count and ghost
//! [`IconKey`] actions on the right — an optional recessed toolbar strip under
//! it (search, scope, filters), and a body the caller fills.
//!
//! Sizing follows the design's flex rules:
//! - expanded sections share the height by [`weight`](DockSection::weight),
//!   never below their minimum body ([`min_body`](DockSection::min_body),
//!   66 px by default);
//! - [`fixed`](DockSection::fixed) sections take their content height;
//! - collapsed sections shrink to their header;
//! - a 6 px [`Splitter`] sits between two neighbouring expanded sections.
//!   Dragging one turns every expanded section's weight into its current
//!   height, so the sections that aren't being resized hold still.
//!
//! Clicking a header anywhere but its keys collapses or expands the section.
//!
//! # State ownership
//!
//! Weights, collapsed flags and the splitter grab live in a caller-owned
//! [`DockStackState`], one per stack. It takes the sections' initial weights
//! and collapsed flags the first time it sees them, and again whenever the
//! number of sections changes.
//!
//! The widget draws only the chrome; the caller draws each section's toolbar
//! and body into the rects in [`DockStackOutput::sections`].
//!
//! ```ignore
//! let sections = [
//!     DockSection::new("Projects").count(24).toolbar(SearchField::HEIGHT)
//!         .actions(&[PhosphorIcon::ArrowsClockwise]),
//!     DockSection::new("Sessions").count(12).weight(1.4)
//!         .actions(&[PhosphorIcon::Plus]),
//! ];
//! let out = DockStack::new(&sections).draw(STACK_ID, body, &mut stack, &mut capture, &mut ctx);
//! if out.action == Some((0, 0)) { rescan(); }
//! if let Some(bar) = out.sections[0].toolbar { draw_search(bar); }
//! if let Some(body) = out.sections[0].body { draw_projects(body); }
//! ```
//!
//! Gated behind the `phosphor-icons` feature (the header keys are icons).

use crate::chrome::Edge;
use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::shadow::CornerRadii;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize, Tracking};

use super::drag::{DragCapture, DragId};
use super::material::Tone;
use super::splitter::Splitter;
use super::tree::{CARET_HALF, draw_disclosure};
use super::{DrawContext, IconKey};

/// Header padding before the caret.
const PAD_LEFT: f32 = 7.0;
/// Header padding after the last key.
const PAD_RIGHT: f32 = 4.0;
/// Gap between the caret, title and count.
const GAP: f32 = 6.0;
/// Width of the caret's column.
const CARET_W: f32 = 8.0;
/// Gap between header keys.
const ACTION_GAP: f32 = 2.0;
/// Padding around the toolbar strip's content.
const TOOLBAR_PAD: f32 = 6.0;
/// The toolbar strip's bottom rule.
const TOOLBAR_RULE: f32 = 1.0;
/// Smallest weight a section can carry, so a zero weight still divides.
const MIN_WEIGHT: f32 = 1e-3;

/// One section's per-frame description. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct DockSection<'a> {
    title: &'a str,
    count: Option<usize>,
    actions: &'a [PhosphorIcon],
    toolbar: Option<f32>,
    weight: f32,
    fixed: Option<f32>,
    min_body: f32,
    collapsed: bool,
}

impl<'a> DockSection<'a> {
    /// The body's default minimum height while expanded, in pixels.
    pub const MIN_BODY: f32 = 66.0;

    /// A section titled `title` (Title Case in source; drawn in capitals),
    /// weight 1, no count, keys or toolbar.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            count: None,
            actions: &[],
            toolbar: None,
            weight: 1.0,
            fixed: None,
            min_body: Self::MIN_BODY,
            collapsed: false,
        }
    }

    /// Show `count` after the title.
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// Ghost keys on the right of the header, left to right. A click reports
    /// `(section, index into icons)` in [`DockStackOutput::action`].
    pub fn actions(mut self, icons: &'a [PhosphorIcon]) -> Self {
        self.actions = icons;
        self
    }

    /// Reserve a recessed toolbar strip under the header whose content is
    /// `height` px tall (e.g. [`SearchField::HEIGHT`](crate::SearchField::HEIGHT)).
    pub fn toolbar(mut self, height: f32) -> Self {
        self.toolbar = Some(height.max(0.0));
        self
    }

    /// This section's share of the free height, relative to the others.
    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Size the body to `height` px instead of sharing the free height. Fixed
    /// sections get no splitters.
    pub fn fixed(mut self, height: f32) -> Self {
        self.fixed = Some(height.max(0.0));
        self
    }

    /// The smallest the body gets while expanded (default
    /// [`MIN_BODY`](Self::MIN_BODY)); splitters stop there.
    pub fn min_body(mut self, height: f32) -> Self {
        self.min_body = height.max(0.0);
        self
    }

    /// Start collapsed (read when the state first sees the section).
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Height of the toolbar strip, rules included; 0 without a toolbar.
    fn strip_height(&self) -> f32 {
        self.toolbar
            .map_or(0.0, |h| h + 2.0 * TOOLBAR_PAD + TOOLBAR_RULE)
    }
}

/// Caller-owned state of one [`DockStack`]: each section's weight and
/// collapsed flag, and the splitter being dragged.
#[derive(Clone, Debug, Default)]
pub struct DockStackState {
    weights: Vec<f32>,
    collapsed: Vec<bool>,
    /// The splitter being dragged (the index of the section below it) and
    /// where the pointer grabbed it, from the splitter's top.
    grab: Option<(usize, f32)>,
}

impl DockStackState {
    /// State seeded from `sections`' weights and collapsed flags.
    pub fn new(sections: &[DockSection]) -> Self {
        let mut state = Self::default();
        state.sync(sections);
        state
    }

    /// Whether section `i` is collapsed (false for an unknown index).
    pub fn is_collapsed(&self, i: usize) -> bool {
        self.collapsed.get(i).copied().unwrap_or(false)
    }

    /// Collapse or expand section `i`. Unknown indices are ignored.
    pub fn set_collapsed(&mut self, i: usize, collapsed: bool) {
        if let Some(c) = self.collapsed.get_mut(i) {
            *c = collapsed;
        }
    }

    /// The sections' current weights: as authored until a splitter is
    /// dragged, then the expanded sections' heights in pixels.
    pub fn weights(&self) -> &[f32] {
        &self.weights
    }

    /// Adopt `sections`' initial weights and flags when the number of
    /// sections changed (including the first time).
    fn sync(&mut self, sections: &[DockSection]) {
        if self.weights.len() != sections.len() {
            self.weights = sections.iter().map(|s| s.weight).collect();
            self.collapsed = sections.iter().map(|s| s.collapsed).collect();
            self.grab = None;
        }
    }
}

/// Where one section landed this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockSectionOutput {
    /// The whole section: header, toolbar strip and body.
    pub rect: Rect,
    /// The header bar.
    pub header: Rect,
    /// The toolbar's content rect (inside the strip's padding); `None` when
    /// the section has no toolbar or is collapsed.
    pub toolbar: Option<Rect>,
    /// The body; `None` while collapsed.
    pub body: Option<Rect>,
    /// The splitter above this section, if any.
    pub splitter_above: Option<Rect>,
    /// Whether the section is collapsed.
    pub collapsed: bool,
}

/// Outcome of drawing a [`DockStack`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockStackOutput {
    /// One entry per section, in order.
    pub sections: Vec<DockSectionOutput>,
    /// `(section, key)` of a header key clicked this frame.
    pub action: Option<(usize, usize)>,
    /// The section whose header was clicked this frame; its collapsed flag in
    /// the state has already flipped and `sections` reflects it.
    pub toggled: Option<usize>,
}

/// A vertical stack of [`DockSection`]s filling a dock. See the
/// [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct DockStack<'a> {
    sections: &'a [DockSection<'a>],
}

impl<'a> DockStack<'a> {
    /// A stack of `sections`, top to bottom.
    pub fn new(sections: &'a [DockSection<'a>]) -> Self {
        Self { sections }
    }

    /// Whether section `i` shares the free height (expanded and not fixed).
    fn grows(&self, state: &DockStackState, i: usize) -> bool {
        !state.is_collapsed(i) && self.sections[i].fixed.is_none()
    }

    /// The smallest a growing section gets: header, toolbar, minimum body.
    fn min_height(&self, i: usize, header_h: f32) -> f32 {
        let section = &self.sections[i];
        header_h + section.strip_height() + section.min_body
    }

    /// Place the sections in `rect` for `state`, without drawing. Pure; the
    /// same placement [`draw`](Self::draw) uses.
    pub fn layout(
        &self,
        rect: Rect,
        state: &DockStackState,
        s: &StyleResolver,
    ) -> Vec<DockSectionOutput> {
        let n = self.sections.len();
        let header_h = s.scalar(StyleKey::ListRowHeight);
        let splitter_h = s.scalar(StyleKey::DockSplitterWidth);

        let splitter_above: Vec<bool> = (0..n)
            .map(|i| i > 0 && self.grows(state, i - 1) && self.grows(state, i))
            .collect();

        // Sections that don't grow take their own height; the rest share
        // what is left by weight.
        let mut heights = vec![0.0; n];
        let mut taken = splitter_above.iter().filter(|&&s| s).count() as f32 * splitter_h;
        let mut growers = Vec::new();
        for (i, section) in self.sections.iter().enumerate() {
            if state.is_collapsed(i) {
                heights[i] = header_h;
            } else if let Some(body) = section.fixed {
                heights[i] = header_h + section.strip_height() + body;
            } else {
                growers.push(i);
                continue;
            }
            taken += heights[i];
        }
        let weights: Vec<f32> = growers
            .iter()
            .map(|&i| state.weights.get(i).copied().unwrap_or(1.0))
            .collect();
        let mins: Vec<f32> = growers
            .iter()
            .map(|&i| self.min_height(i, header_h))
            .collect();
        let shares = distribute((rect.height - taken).max(0.0), &weights, &mins);
        for (&i, share) in growers.iter().zip(shares) {
            heights[i] = share;
        }

        // Place top to bottom on whole pixels so the one-pixel rules stay
        // crisp: every boundary is the rounded running total.
        let mut y = rect.y;
        let mut out = Vec::with_capacity(n);
        for (i, section) in self.sections.iter().enumerate() {
            let splitter = splitter_above[i].then(|| {
                let top = y.round();
                y += splitter_h;
                Rect::new(rect.x, top, rect.width, y.round() - top)
            });
            let top = y.round();
            y += heights[i];
            let bottom = y.round();
            let collapsed = state.is_collapsed(i);
            let header = Rect::new(rect.x, top, rect.width, header_h);
            let strip = if collapsed {
                0.0
            } else {
                section.strip_height()
            };
            let toolbar = section.toolbar.filter(|_| !collapsed).map(|h| {
                Rect::new(
                    rect.x + TOOLBAR_PAD,
                    top + header_h + TOOLBAR_PAD,
                    (rect.width - 2.0 * TOOLBAR_PAD).max(0.0),
                    h,
                )
            });
            let body_top = top + header_h + strip;
            let body = (!collapsed)
                .then(|| Rect::new(rect.x, body_top, rect.width, (bottom - body_top).max(0.0)));
            out.push(DockSectionOutput {
                rect: Rect::new(rect.x, top, rect.width, bottom - top),
                header,
                toolbar,
                body,
                splitter_above: splitter,
                collapsed,
            });
        }
        out
    }

    /// Draw the stack's chrome into `rect` and return where each section's
    /// toolbar and body go. `id` seeds the drag ids of the stack's splitters
    /// (see [`splitter_id`](Self::splitter_id)); `capture` is the surface's
    /// shared [`DragCapture`].
    pub fn draw(
        &self,
        id: DragId,
        rect: Rect,
        state: &mut DockStackState,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> DockStackOutput {
        ctx.push_debug_scope_rect("DockStack", rect);
        state.sync(self.sections);
        let s = ctx.styles();
        let input = ctx.input;
        let header_h = s.scalar(StyleKey::ListRowHeight);
        let mut placed = self.layout(rect, state, &s);

        // A splitter drag in progress resizes the two sections around it
        // before anything is drawn, so the frame shows where the pointer is.
        if let Some((i, grab)) = state.grab {
            let live = capture.is_active(Self::splitter_id(id, i))
                && input.mouse_down
                && placed.get(i).is_some_and(|p| p.splitter_above.is_some());
            if live {
                let (a, b) = (placed[i - 1].rect, placed[i].rect);
                let total = a.height + b.height;
                let min_a = self.min_height(i - 1, header_h);
                let min_b = self.min_height(i, header_h);
                let hi = (total - min_b).max(min_a).min(total);
                let wanted = input.mouse_y - grab - a.y;
                let new_a = wanted.clamp(min_a.min(hi), hi);
                for (k, p) in placed.iter().enumerate() {
                    if self.grows(state, k) {
                        state.weights[k] = p.rect.height.max(MIN_WEIGHT);
                    }
                }
                state.weights[i - 1] = new_a.max(MIN_WEIGHT);
                state.weights[i] = (total - new_a).max(MIN_WEIGHT);
                placed = self.layout(rect, state, &s);
            } else {
                state.grab = None;
            }
        }

        // A header click (outside its keys) flips the section before drawing,
        // so the returned rects already match the new state.
        let mut toggled = None;
        if input.mouse_clicked && !input.mouse_consumed && state.grab.is_none() {
            let key_w = IconKey::new(PhosphorIcon::Plus, IconKey::HEADER).outer_size(&s)[0];
            toggled = placed.iter().enumerate().find_map(|(i, p)| {
                let over_header = p.header.contains(input.mouse_x, input.mouse_y);
                let over_keys = keys_rect(p.header, self.sections[i].actions.len(), key_w)
                    .is_some_and(|k| k.contains(input.mouse_x, input.mouse_y));
                (over_header && !over_keys).then_some(i)
            });
            if let Some(i) = toggled {
                let collapsed = state.is_collapsed(i);
                state.set_collapsed(i, !collapsed);
                placed = self.layout(rect, state, &s);
            }
        }

        let mut action = None;
        for (i, (section, p)) in self.sections.iter().zip(&placed).enumerate() {
            if let Some(bar) = p.splitter_above {
                let dragging = Splitter::horizontal(bar.height)
                    .draw(Self::splitter_id(id, i), capture, bar, ctx)
                    .dragging;
                if dragging && state.grab.map(|(g, _)| g) != Some(i) {
                    state.grab = Some((i, input.mouse_y - bar.y));
                }
            }
            if let Some(k) = self.draw_header(i, section, p, ctx) {
                action = Some((i, k));
            }
            if let Some(toolbar) = p.toolbar {
                let chrome = s.dock_section();
                let strip = Rect::new(
                    p.rect.x,
                    p.header.bottom(),
                    p.rect.width,
                    toolbar.height + 2.0 * TOOLBAR_PAD + TOOLBAR_RULE,
                );
                let list = &mut *ctx.draw_list;
                list.paint_quad_background(strip, chrome.toolbar, CornerRadii::uniform(0.0));
                list.edge_line(
                    strip,
                    Edge::Bottom,
                    chrome.toolbar_rule.thickness,
                    chrome.toolbar_rule.color,
                );
                let above_rule = Rect::new(
                    strip.x,
                    strip.y,
                    strip.width,
                    strip.height - chrome.toolbar_rule.thickness,
                );
                list.edge_line(
                    above_rule,
                    Edge::Bottom,
                    chrome.toolbar_highlight.thickness,
                    chrome.toolbar_highlight.color,
                );
            }
        }

        ctx.pop_debug_scope();
        DockStackOutput {
            sections: placed,
            action,
            toggled,
        }
    }

    /// The drag id of the splitter above section `i` in the stack drawn with
    /// `id`. Distinct per splitter and spread across the id space, so a
    /// stack's splitters don't collide with neighbouring ids.
    pub fn splitter_id(id: DragId, i: usize) -> DragId {
        id ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    /// Draw section `i`'s header; returns the index of a clicked key.
    fn draw_header(
        &self,
        i: usize,
        section: &DockSection,
        p: &DockSectionOutput,
        ctx: &mut DrawContext,
    ) -> Option<usize> {
        let s = ctx.styles();
        let chrome = s.dock_section();
        let input = ctx.input;
        let header = p.header;
        let first = i == 0;

        let hovered = header.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        let key = |icon| IconKey::new(icon, IconKey::HEADER).tone(Tone::Ghost);
        let [key_w, key_h] = key(PhosphorIcon::Plus).outer_size(&s);
        let keys = keys_rect(header, section.actions.len(), key_w);
        if hovered && !keys.is_some_and(|k| k.contains(input.mouse_x, input.mouse_y)) {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }

        // Surface: the wash under everything, then the rules (the two-line
        // top edge on all but the first, the rule under an expanded header).
        {
            let list = &mut *ctx.draw_list;
            list.paint_quad_background(
                header,
                chrome.header[usize::from(hovered)],
                CornerRadii::uniform(0.0),
            );
            let top_border = if first {
                0.0
            } else {
                chrome.top_rule.thickness
            };
            if !first {
                list.edge_line(header, Edge::Top, top_border, chrome.top_rule.color);
            }
            let inside = Rect::new(
                header.x,
                header.y + top_border,
                header.width,
                header.height - top_border,
            );
            list.edge_line(
                inside,
                Edge::Top,
                chrome.header_highlight.thickness,
                chrome.header_highlight.color,
            );
            if !p.collapsed {
                list.edge_line(
                    header,
                    Edge::Bottom,
                    chrome.header_rule.thickness,
                    chrome.header_rule.color,
                );
            }
        }

        // Content is centred between the borders, as the design's flex row is.
        let top_border = if first {
            0.0
        } else {
            chrome.top_rule.thickness
        };
        let bottom_border = if p.collapsed {
            0.0
        } else {
            chrome.header_rule.thickness
        };
        let cy = header.y + top_border + (header.height - top_border - bottom_border) * 0.5;

        let caret_cx = header.x + PAD_LEFT + CARET_W * 0.5;
        draw_disclosure(
            ctx.draw_list,
            caret_cx,
            cy,
            CARET_HALF,
            !p.collapsed,
            s.ink(Ink::Body2),
        );

        let text_x = header.x + PAD_LEFT + CARET_W + GAP;
        let text_end = keys.map_or(header.right() - PAD_RIGHT, |k| k.x - GAP);
        let size = s.text_size(TextSize::Caption);
        let caps = section.title.to_uppercase();
        let ty = ctx
            .draw_list
            .vcentered_text_y(cy, 0.0, size, s.theme().mono_font.as_ref(), &caps);
        let count = section.count.map(|n| {
            let block = s.mono_block(n.to_string(), 0.0, ty, TextSize::Caption, Ink::Dim);
            let (w, _) = ctx.draw_list.measure_block(&block);
            (block, w)
        });
        let count_room = count.as_ref().map_or(0.0, |(_, w)| w + GAP);
        let title_ink = if hovered { Ink::Second } else { Ink::Glyph };
        let title = s
            .caption_block(section.title, text_x, ty, Tracking::Section, title_ink)
            .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0)
            .with_max_width((text_end - count_room - text_x).max(0.0))
            .with_ellipsis();
        let (title_w, _) = ctx.draw_list.measure_block(&title);
        ctx.draw_list.text(title);
        if let Some((mut block, _)) = count {
            block.x = text_x + title_w + GAP;
            ctx.draw_list.text(block);
        }

        let mut clicked = None;
        if let Some(keys) = keys {
            for (k, &icon) in section.actions.iter().enumerate() {
                let r = Rect::new(
                    keys.x + k as f32 * (key_w + ACTION_GAP),
                    (cy - IconKey::HEADER * 0.5).round(),
                    key_w,
                    key_h,
                );
                if key(icon).draw(r, ctx).clicked {
                    clicked = Some(k);
                }
            }
        }
        clicked
    }
}

/// The strip of `count` header keys `key_w` wide at the right of `header`, or
/// `None` without keys. Clicks inside it never collapse the section.
fn keys_rect(header: Rect, count: usize, key_w: f32) -> Option<Rect> {
    (count > 0).then(|| {
        let width = count as f32 * key_w + (count - 1) as f32 * ACTION_GAP;
        Rect::new(
            header.right() - PAD_RIGHT - width,
            header.y,
            width,
            header.height,
        )
    })
}

/// Share `avail` px among sections by `weights`, none below its entry in
/// `mins` — the flex rule: sections that would fall under their minimum are
/// frozen there and the rest share what remains.
fn distribute(avail: f32, weights: &[f32], mins: &[f32]) -> Vec<f32> {
    let n = weights.len();
    let mut sizes = vec![0.0; n];
    let mut frozen = vec![false; n];
    loop {
        let used: f32 = (0..n).filter(|&k| frozen[k]).map(|k| sizes[k]).sum();
        let free = (avail - used).max(0.0);
        let total_weight: f32 = (0..n)
            .filter(|&k| !frozen[k])
            .map(|k| weights[k].max(MIN_WEIGHT))
            .sum();
        if total_weight <= 0.0 {
            break;
        }
        let mut froze_any = false;
        for k in 0..n {
            if frozen[k] {
                continue;
            }
            sizes[k] = free * weights[k].max(MIN_WEIGHT) / total_weight;
        }
        for k in 0..n {
            if !frozen[k] && sizes[k] < mins[k] {
                sizes[k] = mins[k];
                frozen[k] = true;
                froze_any = true;
            }
        }
        if !froze_any {
            break;
        }
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, Theme};

    const RECT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 240.0,
        height: 300.0,
    };
    const ID: DragId = 0xD0C5;

    fn idle() -> InputState {
        InputState {
            mouse_x: -1.0,
            mouse_y: -1.0,
            ..InputState::default()
        }
    }

    fn press(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            ..InputState::default()
        }
    }

    fn hold(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            ..InputState::default()
        }
    }

    /// Draw one frame of `sections` and return the output.
    fn frame(
        sections: &[DockSection],
        state: &mut DockStackState,
        capture: &mut DragCapture,
        input: &InputState,
    ) -> DockStackOutput {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
        DockStack::new(sections).draw(ID, RECT, state, capture, &mut ctx)
    }

    fn layout(sections: &[DockSection], state: &DockStackState) -> Vec<DockSectionOutput> {
        let theme = Theme::default();
        DockStack::new(sections).layout(RECT, state, &StyleResolver::new(&theme))
    }

    #[test]
    fn expanded_sections_share_the_height_by_weight() {
        let sections = [
            // A small minimum body, so the weights alone decide.
            DockSection::new("Projects").toolbar(28.0).min_body(20.0),
            DockSection::new("Sessions").weight(1.5),
        ];
        let state = DockStackState::new(&sections);
        let out = layout(&sections, &state);
        // 300 - the 6 px splitter = 294, split 1 : 1.5 (117.6 : 176.4), each
        // boundary rounded to a whole pixel.
        assert_eq!(out[0].rect, Rect::new(0.0, 0.0, 240.0, 118.0));
        assert_eq!(
            out[1].splitter_above,
            Some(Rect::new(0.0, 118.0, 240.0, 6.0))
        );
        assert_eq!(out[1].rect, Rect::new(0.0, 124.0, 240.0, 176.0));
        assert_eq!(out[0].header, Rect::new(0.0, 0.0, 240.0, 22.0));
        // The toolbar content sits inside the strip's 6 px padding; the strip
        // is 6 + 28 + 6 + its 1 px rule.
        assert_eq!(out[0].toolbar, Some(Rect::new(6.0, 28.0, 228.0, 28.0)));
        assert_eq!(out[0].body, Some(Rect::new(0.0, 63.0, 240.0, 55.0)));
        assert_eq!(out[1].body, Some(Rect::new(0.0, 146.0, 240.0, 154.0)));
        assert_eq!(out[1].toolbar, None);
    }

    #[test]
    fn a_collapsed_section_is_its_header_and_the_rest_take_the_space() {
        let sections = [
            DockSection::new("Projects").collapsed(true),
            DockSection::new("Sessions"),
        ];
        let state = DockStackState::new(&sections);
        let out = layout(&sections, &state);
        assert!(out[0].collapsed);
        assert_eq!(out[0].rect.height, 22.0);
        assert_eq!((out[0].body, out[0].toolbar), (None, None));
        assert_eq!(
            out[1].splitter_above, None,
            "no splitter next to a collapsed section"
        );
        assert_eq!(out[1].rect, Rect::new(0.0, 22.0, 240.0, 278.0));
    }

    #[test]
    fn a_fixed_section_sizes_to_its_content() {
        let sections = [
            DockSection::new("Outliner"),
            DockSection::new("Details").fixed(40.0),
        ];
        let state = DockStackState::new(&sections);
        let out = layout(&sections, &state);
        assert_eq!(out[1].rect, Rect::new(0.0, 238.0, 240.0, 62.0));
        assert_eq!(out[1].splitter_above, None);
        assert_eq!(out[0].rect.height, 238.0);
    }

    #[test]
    fn no_section_shrinks_below_its_minimum() {
        let sections = [
            DockSection::new("Tiny").weight(0.01),
            DockSection::new("Big").weight(10.0),
        ];
        let state = DockStackState::new(&sections);
        let out = layout(&sections, &state);
        assert_eq!(out[0].rect.height, 22.0 + DockSection::MIN_BODY);
        assert_eq!(out[1].rect.height, 300.0 - 6.0 - 88.0);
    }

    #[test]
    fn clicking_a_header_collapses_and_expands_its_section() {
        let sections = [DockSection::new("Projects"), DockSection::new("Sessions")];
        let mut state = DockStackState::default();
        let mut capture = DragCapture::new();
        frame(&sections, &mut state, &mut capture, &idle());
        let header = frame(&sections, &mut state, &mut capture, &idle()).sections[1].header;

        let out = frame(
            &sections,
            &mut state,
            &mut capture,
            &press(60.0, header.y + 11.0),
        );
        assert_eq!(out.toggled, Some(1));
        assert!(state.is_collapsed(1));
        assert!(out.sections[1].collapsed, "the output already reflects it");
        assert_eq!(out.sections[0].rect.height, 300.0 - 22.0);

        let header = out.sections[1].header;
        frame(
            &sections,
            &mut state,
            &mut capture,
            &press(60.0, header.y + 11.0),
        );
        assert!(!state.is_collapsed(1));
    }

    #[test]
    fn header_keys_report_their_click_without_collapsing() {
        let actions = [PhosphorIcon::ArrowsClockwise, PhosphorIcon::Plus];
        let sections = [DockSection::new("Projects").actions(&actions)];
        let mut state = DockStackState::default();
        let mut capture = DragCapture::new();
        // The last key's face ends 4 px from the right edge.
        let out = frame(
            &sections,
            &mut state,
            &mut capture,
            &press(RECT.width - 12.0, 11.0),
        );
        assert_eq!(out.action, Some((0, 1)));
        assert_eq!(out.toggled, None);
        assert!(!state.is_collapsed(0));
        // The gap between the keys belongs to them too.
        let out = frame(
            &sections,
            &mut state,
            &mut capture,
            &press(RECT.width - 22.0, 11.0),
        );
        assert_eq!(out.toggled, None);
    }

    #[test]
    fn dragging_a_splitter_resizes_its_neighbours_and_holds_the_rest() {
        // Three 96 px sections; minimum bodies of 20 leave room to drag.
        let sections = [
            DockSection::new("A").min_body(20.0),
            DockSection::new("B").min_body(20.0),
            DockSection::new("C").min_body(20.0),
        ];
        let mut state = DockStackState::default();
        let mut capture = DragCapture::new();
        let before = frame(&sections, &mut state, &mut capture, &idle()).sections;
        let bar = before[1].splitter_above.unwrap();

        // Grab 2 px into the splitter, then pull it down 30 px.
        frame(
            &sections,
            &mut state,
            &mut capture,
            &press(100.0, bar.y + 2.0),
        );
        let after = frame(
            &sections,
            &mut state,
            &mut capture,
            &hold(100.0, bar.y + 32.0),
        )
        .sections;
        assert_eq!(after[0].rect.height, before[0].rect.height + 30.0);
        assert_eq!(after[1].rect.height, before[1].rect.height - 30.0);
        assert_eq!(after[2].rect, before[2].rect, "C holds still");

        // Far past B's minimum: B stops at its header + 20.
        let after = frame(&sections, &mut state, &mut capture, &hold(100.0, 290.0)).sections;
        assert_eq!(after[1].rect.height, 42.0);
        assert_eq!(after[2].rect, before[2].rect);

        // Back up past A's minimum: A stops at its header + 20.
        let after = frame(&sections, &mut state, &mut capture, &hold(100.0, 0.0)).sections;
        assert_eq!(after[0].rect.height, 42.0);
        assert_eq!(after[2].rect, before[2].rect);

        // Released: the sizes stay, and moving no longer resizes.
        frame(&sections, &mut state, &mut capture, &idle());
        let settled = frame(&sections, &mut state, &mut capture, &hold(100.0, 200.0)).sections;
        assert_eq!(settled[0].rect.height, 42.0);
    }

    #[test]
    fn splitter_ids_are_distinct_per_splitter() {
        let a = DockStack::splitter_id(ID, 1);
        let b = DockStack::splitter_id(ID, 2);
        assert_ne!(a, b);
        assert_ne!(a, ID);
        assert_ne!(DockStack::splitter_id(ID + 1, 1), a);
    }

    #[test]
    fn the_first_header_has_no_top_rule() {
        let sections = [DockSection::new("A"), DockSection::new("B")];
        let theme = Theme::default();
        let rule = theme.chrome.dock_section.top_rule.color;
        let mut state = DockStackState::default();
        let mut capture = DragCapture::new();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = idle();
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        let out = DockStack::new(&sections).draw(ID, RECT, &mut state, &mut capture, &mut ctx);
        let rules: Vec<_> = list
            .chrome_instances()
            .filter(|c| c.bg == rule && c.rect[3] == 1.0)
            .map(|c| c.rect[1])
            .collect();
        assert_eq!(rules, vec![out.sections[1].header.y]);
    }

    #[test]
    fn distribute_freezes_sections_at_their_minimum() {
        assert_eq!(
            distribute(100.0, &[1.0, 1.0], &[0.0, 0.0]),
            vec![50.0, 50.0]
        );
        assert_eq!(
            distribute(100.0, &[1.0, 9.0], &[30.0, 0.0]),
            vec![30.0, 70.0]
        );
        assert_eq!(
            distribute(10.0, &[1.0, 1.0], &[30.0, 30.0]),
            vec![30.0, 30.0]
        );
        assert_eq!(
            distribute(100.0, &[0.0, 0.0], &[0.0, 0.0]),
            vec![50.0, 50.0]
        );
    }
}
