//! Measure and arrange: a node's size from the room its parent gives it,
//! and its children's places, cached per node and redone only where dirt
//! says.
//!
//! Every size is whole pixels, and every place is snapped (each edge to
//! `floor(x + 0.5)`), so a subtree moves by whole pixels and its cached list
//! can be placed again exactly.

use wgpu_gameui::{TextBlock, WrapMode};

use crate::node::{Align, Insets, Size, Text};
use crate::ui::{NodeData, Ui};

/// The room a parent gives a child along one axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Avail {
    /// Exactly this: a fill child takes it.
    Exact(f32),
    /// Up to this: text wraps at it; a hugging child may still overflow it.
    AtMost(f32),
    /// No limit (along a scroll).
    Unbounded,
}

/// Round to whole pixels, ties up, the same wherever the subtree sits.
pub(crate) fn snap(x: f32) -> f32 {
    (x + 0.5).floor()
}

fn is_fill(size: Size) -> bool {
    matches!(size, Size::Fill(_))
}

fn weight(size: Size) -> f32 {
    match size {
        Size::Fill(w) if w > 0.0 => w,
        _ => 0.0,
    }
}

/// The node's own length along an axis, where it does not follow its
/// content.
fn own(size: Size, avail: Avail) -> Option<f32> {
    match size {
        Size::Fixed(px) => Some(if px.is_finite() {
            snap(px).max(0.0)
        } else {
            0.0
        }),
        Size::Fill(_) => match avail {
            Avail::Exact(v) => Some(v),
            _ => None,
        },
        Size::Hug => None,
    }
}

/// Whether a stacked child of a scroll whose content has an exact length
/// across the stack can be sized without laying it out: its length along
/// the stack is fixed, and across it is fixed, fills, or hugs at the start,
/// where its place does not depend on its length.
fn deferrable(s: [Size; 2], m: usize, cross: Align) -> bool {
    matches!(s[m], Size::Fixed(_))
        && match s[1 - m] {
            Size::Fixed(_) | Size::Fill(_) => true,
            Size::Hug => cross == Align::Start,
        }
}

/// The room a child gets from the room its parent's content has: a fill
/// child is given an exact length, any other a limit.
fn child_room(inner: Avail, fill: bool) -> Avail {
    match inner {
        Avail::Exact(v) if !fill => Avail::AtMost(v),
        other => other,
    }
}

/// Whether the node's size may change when its content does: on an axis
/// that hugs, or fills without being given an exact length.
pub(crate) fn follows_content(d: &NodeData) -> bool {
    let sizes = [d.width.value, d.height.value];
    (0..2).any(|axis| match sizes[axis] {
        Size::Fixed(_) => false,
        Size::Fill(_) => !matches!(d.avail.map(|a| a[axis]), Some(Avail::Exact(_))),
        Size::Hug => true,
    })
}

fn padding(p: Insets) -> Insets {
    let s = |v: f32| if v.is_finite() { snap(v).max(0.0) } else { 0.0 };
    Insets {
        left: s(p.left),
        top: s(p.top),
        right: s(p.right),
        bottom: s(p.bottom),
    }
}

/// The text block a node draws (and is measured as), at `(x, y)`, wrapping
/// at `width` when it wraps.
pub(crate) fn text_block(text: &Text, color: [f32; 4], x: f32, y: f32, width: f32) -> TextBlock {
    TextBlock::new(&text.content, x, y)
        .with_size(text.size)
        .with_line_height(text.line_height)
        .with_max_width(width)
        .with_font_opt(text.font.clone())
        .with_weight(text.weight)
        .with_align(text.align)
        .with_wrap(if text.wrap {
            WrapMode::WordOrGlyph
        } else {
            WrapMode::None
        })
        .with_color_f32(color)
}

/// How wide a text is laid out when nothing limits it.
const UNLIMITED: f32 = 1.0e6;

/// Where a node's text sits in its box: its content box's origin, moved
/// down by its vertical alignment, and the width it is set in.
pub(crate) fn text_place(d: &NodeData) -> (f32, f32, f32) {
    let t = d.text.as_ref().expect("a text node");
    let pad = padding(d.stack.value.padding);
    let content_w = (d.size[0] - pad.left - pad.right).max(0.0);
    let content_h = (d.size[1] - pad.top - pad.bottom).max(0.0);
    let y = snap(pad.top + (content_h - t.size[1]) * t.text.value.valign.share());
    (pad.left, y, content_w)
}

/// How far `n`'s own drawing reaches outside its box: its shadow, its
/// painter's declared overflow, its text where it overflows.
pub(crate) fn update_own_grow(ui: &mut Ui, n: u32) {
    let d = &ui.nodes[n as usize];
    let [w, h] = d.size;
    let mut grow = Insets::default();
    if let Some(s) = d.look.value.shadow
        && !s.inset
        && s.color[3] > 0.0
    {
        let ink = s.ink_rect(wgpu_gameui::layout::Rect::new(0.0, 0.0, w, h));
        grow = grow.max(Insets {
            left: (-ink.x).max(0.0),
            top: (-ink.y).max(0.0),
            right: (ink.right() - w).max(0.0),
            bottom: (ink.bottom() - h).max(0.0),
        });
    }
    if let Some(c) = &d.custom {
        grow = grow.max(c.overflow);
    }
    if let Some(t) = &d.text {
        let (x, y, _) = text_place(d);
        grow = grow.max(Insets {
            left: 0.0,
            top: (-y).max(0.0),
            right: (x + t.size[0] - w).max(0.0),
            bottom: (y + t.size[1] - h).max(0.0),
        });
    }
    if grow != d.own_grow {
        ui.nodes[n as usize].own_grow = grow;
        ui.mark_grow(n);
    }
}

/// Clamp `n`'s scroll to its content, snap it, and mark it if what is drawn
/// moved, which it returns. An offset past the content is written back
/// clamped.
pub(crate) fn apply_scroll(ui: &mut Ui, n: u32) -> bool {
    let d = &mut ui.nodes[n as usize];
    let viewport = d.size[d.stack.value.direction.main()];
    let s = d.scroll.as_mut().expect("a scrolling node");
    let max = (s.extent - viewport).max(0.0);
    let want = if s.requested.is_nan() {
        0.0
    } else {
        s.requested.clamp(0.0, max)
    };
    let applied = snap(want);
    let moved = applied != s.applied;
    s.applied = applied;
    if want != s.requested {
        s.requested = want;
        let signal = s.signal;
        ui.graph.set(signal, want);
    }
    if moved {
        ui.mark_records(n);
    }
    moved
}

impl Ui {
    /// Lay out `n` in `avail`: from its cache when nothing in it changed and
    /// the room is the same, else measured and its children placed.
    pub(crate) fn layout(&mut self, n: u32, avail: [Avail; 2]) {
        self.stats.visited += 1;
        #[cfg(test)]
        let may_defer = !self.no_defer;
        #[cfg(not(test))]
        let may_defer = true;
        let d = &mut self.nodes[n as usize];
        d.deferred = false;
        if !d.layout_self && d.avail == Some(avail) {
            if d.layout_below {
                d.layout_below = false;
                let defers = d.scroll.as_ref().is_some_and(|s| s.defers);
                // Nothing here changes size: what is dirty below is under a
                // node whose size does not follow its content.
                for list in [false, true] {
                    if defers && !list {
                        self.layout_near_view(n);
                        continue;
                    }
                    let kids = self.take_kids(n, list);
                    for &c in &kids {
                        let cd = &self.nodes[c as usize];
                        if cd.layout_self || cd.layout_below {
                            let room = cd.avail.expect("laid out before");
                            self.layout(c, room);
                        }
                    }
                    self.put_kids(n, list, kids);
                }
            }
            return;
        }
        d.layout_self = false;
        d.layout_below = false;
        self.stats.laid_out += 1;

        let sizes = [d.width.value, d.height.value];
        let stack = d.stack.value;
        let m = stack.direction.main();
        let pad = padding(stack.padding);
        let pad_start = [pad.left, pad.top];
        let pad_sum = [pad.left + pad.right, pad.top + pad.bottom];
        let own = [own(sizes[0], avail[0]), own(sizes[1], avail[1])];
        let mut inner = [0, 1].map(|a| match own[a] {
            Some(v) => Avail::Exact((v - pad_sum[a]).max(0.0)),
            None => match avail[a] {
                Avail::Exact(v) | Avail::AtMost(v) => Avail::AtMost((v - pad_sum[a]).max(0.0)),
                Avail::Unbounded => Avail::Unbounded,
            },
        });
        let scrolls = d.scroll.is_some();
        if scrolls {
            inner[m] = Avail::Unbounded;
        }
        let defers = may_defer && matches!(inner[1 - m], Avail::Exact(_));
        if let Some(s) = d.scroll.as_mut() {
            s.defers = defers;
        }
        let defers = scrolls && defers;

        let content = if d.text.is_some() {
            self.measure_text(n, inner[0])
        } else if d.has_children() {
            self.layout_stack(n, inner, pad_start, defers)
        } else {
            [0.0; 2]
        };
        let size = [0, 1].map(|a| own[a].unwrap_or_else(|| (content[a] + pad_sum[a]).ceil()));
        let d = &mut self.nodes[n as usize];
        d.avail = Some(avail);
        if size != d.size {
            d.size = size;
            self.mark_paint(n);
        }
        if scrolls {
            let s = self.nodes[n as usize].scroll.as_mut().expect("it scrolls");
            s.extent = content[m] + pad_sum[m];
            apply_scroll(self, n);
            if defers {
                self.layout_near_view(n);
            }
        }
        update_own_grow(self, n);
        // Its size, or its children's places, may have moved its paint
        // bounds.
        self.mark_grow(n);
    }

    fn take_kids(&mut self, n: u32, absolute: bool) -> Vec<u32> {
        let d = &mut self.nodes[n as usize];
        std::mem::take(if absolute {
            &mut d.absolute
        } else {
            &mut d.children
        })
    }

    fn put_kids(&mut self, n: u32, absolute: bool, kids: Vec<u32>) {
        let d = &mut self.nodes[n as usize];
        if absolute {
            d.absolute = kids;
        } else {
            d.children = kids;
        }
    }

    fn measure_text(&mut self, n: u32, inner_x: Avail) -> [f32; 2] {
        self.stats.measured += 1;
        let d = &self.nodes[n as usize];
        let t = d.text.as_ref().expect("a text node");
        let width = match inner_x {
            Avail::Exact(v) | Avail::AtMost(v) => v,
            Avail::Unbounded => UNLIMITED,
        };
        let block = text_block(&t.text.value, t.color.value, 0.0, 0.0, width);
        let (w, h) = self.measurer.measure_block(&block);
        let size = [w.ceil().max(0.0), h.ceil().max(0.0)];
        self.nodes[n as usize]
            .text
            .as_mut()
            .expect("a text node")
            .size = size;
        size
    }

    fn place(&mut self, parent: u32, child: u32, pos: [f32; 2]) {
        if self.nodes[child as usize].pos != pos {
            self.nodes[child as usize].pos = pos;
            self.mark_records(parent);
        }
    }

    /// Size `c`, a child its scroll defers, for `room` without laying it
    /// out, unless it is laid out in that room already: what is in it is
    /// laid out when it is near the view (`layout_near_view`).
    fn defer(&mut self, c: u32, room: [Avail; 2], s: [Size; 2], m: usize) {
        let d = &mut self.nodes[c as usize];
        if d.avail == Some(room) && !d.layout_self && !d.layout_below {
            return;
        }
        self.stats.deferred += 1;
        d.deferred = true;
        if d.avail != Some(room) {
            d.avail = Some(room);
            d.layout_self = true;
        }
        let x = 1 - m;
        let mut size = d.size;
        size[m] = own(s[m], room[m]).expect("a fixed length");
        // Hugging at the start, it keeps its length until it is laid out:
        // nothing else depends on it.
        if let Some(v) = own(s[x], room[x]) {
            size[x] = v;
        }
        if size != d.size {
            d.size = size;
            self.mark_paint(c);
        }
    }

    /// Lay out what is dirty in the stacked children of `n`, a scroll that
    /// defers: the children it defers only within a viewport of its view,
    /// any other wherever it is. Its children are in order along the stack,
    /// so when it defers them all the first near the view is found by
    /// binary search and the walk stops past the last.
    fn layout_near_view(&mut self, n: u32) {
        let d = &self.nodes[n as usize];
        let stack = d.stack.value;
        let m = stack.direction.main();
        let view = d.size[m];
        let s = d.scroll.as_ref().expect("a scrolling node");
        let (applied, mixed) = (s.applied, s.mixed);
        let (lo, hi) = (applied - view, applied + 2.0 * view);
        let kids = self.take_kids(n, false);
        let nodes = &self.nodes;
        let first = if mixed {
            0
        } else {
            kids.partition_point(|&c| {
                let c = &nodes[c as usize];
                c.pos[m] + c.size[m] <= lo
            })
        };
        for &c in &kids[first..] {
            let cd = &self.nodes[c as usize];
            let near = cd.pos[m] + cd.size[m] > lo && cd.pos[m] < hi;
            if !near && !mixed {
                break;
            }
            if !(cd.layout_self || cd.layout_below) {
                continue;
            }
            if !near && deferrable([cd.width.value, cd.height.value], m, stack.cross) {
                continue;
            }
            let room = cd
                .avail
                .expect("laid out, or sized when its layout was put off");
            self.layout(c, room);
        }
        self.put_kids(n, false, kids);
    }

    /// Lay out `n`'s children and return its content's size. A scroll that
    /// `defers` only sizes the children it can (`deferrable`); the caller
    /// lays out those near the view (`layout_near_view`) once the scroll is
    /// known.
    fn layout_stack(
        &mut self,
        n: u32,
        inner: [Avail; 2],
        pad_start: [f32; 2],
        defers: bool,
    ) -> [f32; 2] {
        let stack = self.nodes[n as usize].stack.value;
        let m = stack.direction.main();
        let x = 1 - m;
        let gap = if stack.gap > 0.0 { stack.gap } else { 0.0 };
        let main_exact = matches!(inner[m], Avail::Exact(_));
        let flow = self.take_kids(n, false);
        let mut mixed = false;
        let specs = |ui: &Ui, c: u32| {
            let d = &ui.nodes[c as usize];
            [d.width.value, d.height.value]
        };

        // Children that do not share the stack's length, then the share.
        let mut fixed = 0.0f32;
        let mut weights = 0.0f32;
        let mut cross = 0.0f32;
        for &c in &flow {
            let s = specs(self, c);
            if main_exact && is_fill(s[m]) {
                weights += weight(s[m]);
                continue;
            }
            let mut room = [Avail::Unbounded; 2];
            room[m] = child_room(inner[m], false);
            room[x] = child_room(inner[x], is_fill(s[x]));
            if defers && deferrable(s, m, stack.cross) {
                self.defer(c, room, s, m);
            } else {
                mixed |= defers;
                self.layout(c, room);
            }
            let size = self.nodes[c as usize].size;
            fixed += size[m];
            cross = cross.max(size[x]);
        }
        if let Some(s) = self.nodes[n as usize].scroll.as_mut() {
            s.mixed = mixed;
        }
        let gaps = gap * flow.len().saturating_sub(1) as f32;
        let remaining = match inner[m] {
            Avail::Exact(v) => (v - fixed - gaps).max(0.0),
            _ => 0.0,
        };
        let total = fixed + gaps + if weights > 0.0 { remaining } else { 0.0 };
        let free = match inner[m] {
            Avail::Exact(v) => (v - total).max(0.0),
            _ => 0.0,
        };

        let mut cursor = pad_start[m] + free * stack.main.share();
        let mut starts = std::mem::take(&mut self.scratch_f32);
        starts.clear();
        for &c in &flow {
            let s = specs(self, c);
            let filling = main_exact && is_fill(s[m]);
            let len = if !filling {
                self.nodes[c as usize].size[m]
            } else if weights > 0.0 {
                remaining * weight(s[m]) / weights
            } else {
                0.0
            };
            let start = snap(cursor);
            let end = snap(cursor + len);
            if filling {
                let mut room = [Avail::Unbounded; 2];
                room[m] = Avail::Exact(end - start);
                room[x] = child_room(inner[x], is_fill(s[x]));
                self.layout(c, room);
                cross = cross.max(self.nodes[c as usize].size[x]);
            }
            starts.push(start);
            cursor += len + gap;
        }
        let extent = match inner[x] {
            Avail::Exact(v) => v,
            _ => cross,
        };
        for (&c, &start) in flow.iter().zip(&starts) {
            let size = self.nodes[c as usize].size;
            let mut pos = [0.0; 2];
            pos[m] = start;
            pos[x] = snap(pad_start[x] + (extent - size[x]) * stack.cross.share());
            self.place(n, c, pos);
        }
        self.scratch_f32 = starts;
        self.put_kids(n, false, flow);

        let absolute = self.take_kids(n, true);
        for &c in &absolute {
            let s = specs(self, c);
            let room = [
                child_room(inner[0], is_fill(s[0])),
                child_room(inner[1], is_fill(s[1])),
            ];
            self.layout(c, room);
            let offset = self.nodes[c as usize]
                .offset
                .as_ref()
                .expect("an absolute node")
                .value;
            let at = |a: usize| {
                if offset[a].is_finite() {
                    snap(pad_start[a] + offset[a])
                } else {
                    pad_start[a]
                }
            };
            self.place(n, c, [at(0), at(1)]);
        }
        self.put_kids(n, true, absolute);

        let mut content = [0.0; 2];
        content[m] = total.ceil();
        content[x] = cross;
        content
    }

    /// Lay the whole tree out again from nothing and check it lands where
    /// the incremental layout put it. For tests.
    #[cfg(test)]
    pub(crate) fn check_layout(&mut self) {
        let before: Vec<_> = self.nodes.iter().map(|d| (d.live, d.pos, d.size)).collect();
        // What is laid out from nothing is what is near each scroll's view
        // now; what the incremental layout put off, or kept from before,
        // stays as it is. The marks are put back after, so the check
        // changes nothing the next frame does.
        let marks: Vec<_> = self
            .nodes
            .iter()
            .map(|d| (d.avail, d.layout_self, d.layout_below, d.deferred))
            .collect();
        for d in &mut self.nodes {
            if d.live {
                d.avail = None;
                d.layout_self = true;
            }
        }
        let stats = self.stats;
        let context = self.context.expect("a frame was drawn");
        let viewport = context.viewport.map(|v| Avail::Exact(snap(v).max(0.0)));
        self.layout(self.root, viewport);
        self.stats = stats;
        for (d, m) in self.nodes.iter_mut().zip(marks) {
            (d.avail, d.layout_self, d.layout_below, d.deferred) = m;
        }
        for (k, (d, b)) in self.nodes.iter().zip(&before).enumerate() {
            if d.live {
                assert_eq!(
                    (d.pos, d.size),
                    (b.1, b.2),
                    "node {k} laid out from nothing differs from the incremental layout"
                );
            }
        }
    }
}
