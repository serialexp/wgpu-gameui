//! Drawing: the root, and each node that clips (or scrolls), keeps a
//! list of its subtree at its own origin, recorded without its ancestors'
//! clips; such a list is placed into the list that holds it with
//! `append_translated` at its position under that list's clip. Every other
//! node is drawn into its holder's list (the nearest list above it): its own
//! drawing is recorded at its origin into a scratch list and placed there at
//! its offset, then its children are drawn the same way. A list is recorded
//! again only when something in it moved or changed in number; otherwise it
//! is placed as it is.
//!
//! A node that clips draws only the children whose paint bounds meet its
//! box, found by binary search along its stack, so its list depends on what
//! is in view and is recorded again when that changes.
//!
//! What a list placed is logged with its shape (`PlacedLog`), so that a
//! piece drawn again with the shape it had (the same paint runs over the same
//! number of records) is not placed again: its records are overwritten where
//! the holder's list holds them (`DrawList::patch_translated`), and the runs
//! overwritten are handed up for the holder's holder to overwrite in its
//! list, and so on to the root. A change deep in the tree costs the records
//! it changed at each list, not every list above it. Whatever the patch
//! cannot vouch for (another shape, a record its placement culled, a node
//! moved or out of view) records the holder again instead.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Background, CornerRadii, DrawList, EdgeWidths, PlacedLog, QuadStyle, RecordSpan,
};

use crate::node::Insets;
use crate::own_drawing::paint_node;
use crate::ui::{Kept, NodeData, Ui};

/// A list being recorded: the list and the log of what it placed, its
/// `list_gen`, and the earliest repaint asked for by what it holds.
struct Recording {
    kept: Box<Kept>,
    generation: u32,
    deadline: f64,
}

impl Ui {
    /// Whether `n`'s list must be recorded again: something in it is out of
    /// date, a volatile painter is in it, a repaint it asked for has come, or
    /// it has none yet.
    pub(crate) fn needs_record(&self, n: u32) -> bool {
        let d = &self.nodes[n as usize];
        d.stale || d.volatile_below > 0 || d.deadline <= self.now || d.kept.is_none()
    }

    /// Bring `n`'s list up to date, `n` being a node that keeps one: record
    /// it again if it must be, else patch in it the nodes queued as changed,
    /// falling back to recording it when a patch cannot be made. The runs of
    /// records patched are pushed onto `spans`.
    pub(crate) fn update(&mut self, n: u32) -> Update {
        if self.needs_record(n) {
            self.record(n);
            return Update::Recorded;
        }
        if self.nodes[n as usize].dirty_kids.is_empty() {
            return Update::Clean;
        }
        let start = self.spans.len();
        let mut kids = std::mem::take(&mut self.nodes[n as usize].dirty_kids);
        self.unqueue(&kids);
        let patched = kids
            .iter()
            .all(|&(c, generation)| self.patch_in(n, c, generation));
        kids.clear();
        self.nodes[n as usize].dirty_kids = kids;
        if patched {
            return Update::Patched;
        }
        // The nodes not yet patched are drawn by the recording, which brings
        // each up to date first.
        self.spans.truncate(start);
        self.record(n);
        Update::Recorded
    }

    /// Bring `c`, drawn into `n`'s list, up to date and patch its records
    /// where `n`'s list placed them, pushing the runs patched (in `n`'s list)
    /// onto `spans`: `c`'s own list's, or its own drawing's. `false` when that
    /// cannot be done and `n` must be recorded again: `c` is not the node
    /// queued (`generation`) or is not drawn into `n`'s list, its piece is
    /// not in `n`'s list as it is (it was out of view, or `n` was recorded
    /// since) or is at another offset, it is out of `n`'s view now, it was
    /// drawn with another shape, or the patch is declined.
    fn patch_in(&mut self, n: u32, c: u32, generation: u32) -> bool {
        let d = &self.nodes[c as usize];
        if !d.live || d.generation != generation || d.deferred || self.holder(c) != Some(n) {
            return false;
        }
        let p = &self.nodes[n as usize];
        if d.placed_gen != p.list_gen {
            return false;
        }
        let piece = d.piece;
        let Some(placed) = p.kept.as_ref().and_then(|k| k.log.placed(piece)).copied() else {
            return false;
        };
        if self.offset_in(c, n) != placed.offset {
            return false;
        }
        // A child of a node that clips is drawn only while in view.
        if d.parent == n && p.clip {
            self.grow(c);
            if !in_view(
                &self.nodes[n as usize],
                &self.nodes[c as usize],
                placed.offset,
            ) {
                return false;
            }
        }
        let mark = self.spans.len();
        let keeps = self.keeps_list(c);
        let mut own = None;
        if keeps {
            match self.update(c) {
                Update::Clean => return true,
                Update::Patched => {}
                Update::Recorded => self.spans.truncate(mark),
            }
        } else {
            if !self.nodes[c as usize].stale {
                return true;
            }
            own = Some(self.paint_own(c));
        }
        let mut kept = self.nodes[n as usize]
            .kept
            .take()
            .expect("a list placed it");
        let child = match &own {
            Some(list) => &**list,
            None => {
                &self.nodes[c as usize]
                    .kept
                    .as_deref()
                    .expect("recorded")
                    .list
            }
        };
        // Drawn again whole, it must have the shape it was placed with.
        let whole = self.spans.len() == mark;
        let mut ok = !whole || kept.log.fits(piece, child);
        if ok && whole {
            self.spans.push(RecordSpan::all_of(child));
        }
        if ok {
            for span in &mut self.spans[mark..] {
                if !kept.list.patch_translated(&placed, child, span) {
                    ok = false;
                    break;
                }
                self.stats.patched += span.len();
                *span = span.offset_by(placed.at);
            }
        }
        let deadline = if keeps {
            self.nodes[c as usize].deadline
        } else {
            f64::INFINITY
        };
        if own.is_some() {
            self.piece_list = own;
        }
        let d = &mut self.nodes[n as usize];
        d.kept = Some(kept);
        // A deadline that moved later leaves this one early, which records
        // this list again when it comes and puts it right.
        d.deadline = d.deadline.min(deadline);
        ok
    }

    /// Draw `c`'s own drawing, `c` being a node that keeps no list, into the
    /// scratch piece list, which is returned (the caller puts it back);
    /// its repaint request goes to its holder.
    fn paint_own(&mut self, c: u32) -> Box<DrawList> {
        let mut own = self
            .piece_list
            .take()
            .unwrap_or_else(|| Box::new(DrawList::with_font_system(self.fonts.clone())));
        own.clear();
        let mut deadline = f64::INFINITY;
        self.stats.painted += 1;
        paint_node(
            &self.nodes[c as usize],
            &mut self.graph,
            self.now,
            &mut own,
            &mut deadline,
        );
        self.nodes[c as usize].stale = false;
        if let Some(h) = self.holder(c) {
            let d = &mut self.nodes[h as usize];
            d.deadline = d.deadline.min(deadline);
        }
        own
    }

    /// Where `c` is drawn in `n`'s list, `n` being its holder: the offsets
    /// down from `n` added in the order drawing adds them.
    fn offset_in(&self, c: u32, n: u32) -> [f32; 2] {
        let d = &self.nodes[c as usize];
        let base = if d.parent == n {
            [0.0; 2]
        } else {
            self.offset_in(d.parent, n)
        };
        let o = offset(d, self.nodes[d.parent as usize].scrolled());
        [base[0] + o[0], base[1] + o[1]]
    }

    /// Record `n`'s list again, `n` being a node that keeps one: its own
    /// drawing, then what is in view below it.
    pub(crate) fn record(&mut self, n: u32) {
        self.stats.recorded += 1;
        let d = &mut self.nodes[n as usize];
        let mut kept = d.kept.take().unwrap_or_else(|| {
            Box::new(Kept {
                list: DrawList::with_font_system(self.fonts.clone()),
                log: PlacedLog::default(),
            })
        });
        kept.list.clear();
        // The frame's list says what each frame patched in it, so the
        // renderer need not compare it with the last (`Ui::frame`).
        kept.list.report_edits(n == self.root);
        kept.log.clear();
        d.list_gen = d.list_gen.wrapping_add(1);
        let generation = d.list_gen;
        // Everything in the list is drawn below, and so brought up to date;
        // what was queued is not wanted.
        let mut kids = std::mem::take(&mut d.dirty_kids);
        self.unqueue(&kids);
        kids.clear();
        self.nodes[n as usize].dirty_kids = kids;
        let mut rec = Recording {
            kept,
            generation,
            deadline: f64::INFINITY,
        };
        self.stats.painted += 1;
        paint_node(
            &self.nodes[n as usize],
            &mut self.graph,
            self.now,
            &mut rec.kept.list,
            &mut rec.deadline,
        );
        self.draw_children(n, [0.0; 2], &mut rec);
        let d = &mut self.nodes[n as usize];
        d.kept = Some(rec.kept);
        d.deadline = rec.deadline;
        d.stale = false;
    }

    /// Draw `n`'s children into `rec`, `n` being drawn at `base` in it.
    fn draw_children(&mut self, n: u32, base: [f32; 2], rec: &mut Recording) {
        let d = &self.nodes[n as usize];
        // Only a node that keeps a list clips. A clip encloses the children,
        // so a node without any (a scroll container whose last row went) has
        // none, as in the frame drawn from nothing.
        let clips = d.clip && d.has_children();
        let scrolled = d.scrolled();
        if clips {
            let rect = Rect::new(0.0, 0.0, d.size[0], d.size[1]);
            if d.scroll.is_some() {
                rec.kept.list.push_clip_viewport(rect);
            } else {
                rec.kept.list.push_clip(rect);
            }
        }
        let clip = rec.kept.list.current_clip();
        let range = self.flow_candidates(n);
        self.stats.examined += range.len() as u32;
        let place_of = |c: &NodeData| {
            let o = offset(c, scrolled);
            [base[0] + o[0], base[1] + o[1]]
        };
        for i in range {
            let c = self.nodes[n as usize].children[i];
            let cd = &self.nodes[c as usize];
            // One whose layout was put off is out of view, unless its paint
            // bounds reach a viewport: it is not drawn until it is laid out.
            if cd.deferred || clips && !in_view(&self.nodes[n as usize], cd, offset(cd, scrolled)) {
                continue;
            }
            let at = place_of(cd);
            self.draw_child(c, at, clip, rec);
        }
        for i in 0..self.nodes[n as usize].absolute.len() {
            let c = self.nodes[n as usize].absolute[i];
            self.stats.examined += 1;
            if clips {
                self.grow(c);
                let cd = &self.nodes[c as usize];
                if !in_view(&self.nodes[n as usize], cd, offset(cd, scrolled)) {
                    continue;
                }
            }
            let at = place_of(&self.nodes[c as usize]);
            self.draw_child(c, at, clip, rec);
        }
        if clips {
            rec.kept.list.pop_clip();
        }
    }

    fn draw_child(&mut self, c: u32, at: [f32; 2], clip: Option<Rect>, rec: &mut Recording) {
        if self.keeps_list(c) {
            let mark = self.spans.len();
            self.update(c);
            self.spans.truncate(mark);
            let d = &self.nodes[c as usize];
            let child = &d.kept.as_deref().expect("recorded").list;
            let piece = rec.kept.log.place(&mut rec.kept.list, child, at, clip);
            self.stats.placed += 1;
            self.stats.copied += child.prim_counts().total() as u64;
            rec.deadline = rec.deadline.min(d.deadline);
            let d = &mut self.nodes[c as usize];
            d.piece = piece;
            d.placed_gen = rec.generation;
            return;
        }
        let mut own = self
            .piece_list
            .take()
            .unwrap_or_else(|| Box::new(DrawList::with_font_system(self.fonts.clone())));
        own.clear();
        self.stats.painted += 1;
        paint_node(
            &self.nodes[c as usize],
            &mut self.graph,
            self.now,
            &mut own,
            &mut rec.deadline,
        );
        let piece = rec.kept.log.place(&mut rec.kept.list, &own, at, clip);
        self.piece_list = Some(own);
        let d = &mut self.nodes[c as usize];
        d.piece = piece;
        d.placed_gen = rec.generation;
        d.stale = false;
        if d.has_children() {
            self.draw_children(c, at, rec);
        }
    }

    /// The stacked children of `n` that may be in view: all of them unless
    /// it clips, else those whose span along the stack, grown by the most
    /// any of them paints outside it, meets the box. Stacked children are in
    /// order along the stack (gaps are not negative, sizes not either), so
    /// both ends are found by binary search.
    fn flow_candidates(&mut self, n: u32) -> std::ops::Range<usize> {
        let d = &self.nodes[n as usize];
        if !d.clip {
            return 0..d.children.len();
        }
        if d.child_grow_stale {
            self.refresh_child_grow(n);
        }
        let d = &self.nodes[n as usize];
        let m = d.stack.direction.main();
        let s = d.scrolled()[m];
        let length = d.size[m];
        let [before, after] = d.child_grow;
        let nodes = &self.nodes;
        let lo = d.children.partition_point(|&c| {
            let c = &nodes[c as usize];
            c.pos[m] - s + c.size[m] + after <= 0.0
        });
        let hi = d.children.partition_point(|&c| {
            let c = &nodes[c as usize];
            c.pos[m] - s - before < length
        });
        lo..hi.max(lo)
    }

    pub(crate) fn refresh_child_grow(&mut self, n: u32) {
        let m = self.nodes[n as usize].stack.direction.main();
        let kids = std::mem::take(&mut self.nodes[n as usize].children);
        let mut reach = [0.0f32; 2];
        for &c in &kids {
            let g = self.grow(c);
            reach[0] = reach[0].max(g.start(m));
            reach[1] = reach[1].max(g.end(m));
        }
        let d = &mut self.nodes[n as usize];
        d.children = kids;
        d.child_grow = reach;
        d.child_grow_stale = false;
    }

    /// How far `n`'s paint bounds reach outside its box: its own drawing's
    /// reach, and, unless it clips, its children's.
    pub(crate) fn grow(&mut self, n: u32) -> Insets {
        let d = &self.nodes[n as usize];
        if !d.grow_stale {
            return d.grow;
        }
        let mut grow = d.own_grow;
        if !d.clip {
            let size = d.size;
            for list in [false, true] {
                let count = if list {
                    self.nodes[n as usize].absolute.len()
                } else {
                    self.nodes[n as usize].children.len()
                };
                for i in 0..count {
                    let p = &self.nodes[n as usize];
                    let c = if list { p.absolute[i] } else { p.children[i] };
                    let g = self.grow(c);
                    grow = grow.max(reach(&self.nodes[c as usize], g, size));
                }
            }
        }
        let d = &mut self.nodes[n as usize];
        d.grow = grow;
        d.grow_stale = false;
        grow
    }

    /// The frame drawn from nothing: every node recorded in place, inside
    /// its ancestors' translations and clips, with the same culling, and no
    /// list kept or placed. The cached frame must equal it. For tests.
    pub fn draw_from_nothing(&mut self) -> DrawList {
        let mut list = DrawList::with_font_system(self.fonts.clone());
        let mut deadline = f64::INFINITY;
        self.fresh(self.root, [0.0; 2], &mut list, &mut deadline);
        list
    }

    fn fresh(&mut self, n: u32, at: [f32; 2], list: &mut DrawList, deadline: &mut f64) {
        list.push_transform();
        list.translate(at[0], at[1]);
        paint_node(
            &self.nodes[n as usize],
            &mut self.graph,
            self.now,
            list,
            deadline,
        );
        let d = &self.nodes[n as usize];
        // A clip encloses the children, so a node without any (a scroll
        // container whose last row went) has none, as when it is drawn
        // into its parent's list.
        let clips = d.clip && d.has_children();
        let scrolled = d.scrolled();
        if clips {
            let rect = Rect::new(0.0, 0.0, d.size[0], d.size[1]);
            if d.scroll.is_some() {
                list.push_clip_viewport(rect);
            } else {
                list.push_clip(rect);
            }
        }
        let kids: Vec<u32> = d.children.iter().chain(&d.absolute).copied().collect();
        for c in kids {
            let cd = &self.nodes[c as usize];
            if cd.deferred {
                continue;
            }
            let at = offset(cd, scrolled);
            if clips {
                let g = fresh_grow(self, c);
                if !meets(&self.nodes[n as usize], cd, g, at) {
                    continue;
                }
            }
            self.fresh(c, at, list, deadline);
        }
        if clips {
            list.pop_clip();
        }
        list.pop_transform();
    }
}

/// What [`Ui::update`] did to a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    /// Nothing in it changed.
    Clean,
    /// Records drawn into it were patched; the runs are on `spans`.
    Patched,
    /// It was recorded again.
    Recorded,
}

/// `child`'s place in its parent's list: its position less the parent's
/// scroll.
pub(crate) fn offset(child: &NodeData, scrolled: [f32; 2]) -> [f32; 2] {
    [child.pos[0] - scrolled[0], child.pos[1] - scrolled[1]]
}

/// How far `child`, at its position with paint bounds `g`, reaches outside
/// a parent of `size`.
fn reach(child: &NodeData, g: Insets, size: [f32; 2]) -> Insets {
    Insets {
        left: (g.left - child.pos[0]).max(0.0),
        top: (g.top - child.pos[1]).max(0.0),
        right: (child.pos[0] + child.size[0] + g.right - size[0]).max(0.0),
        bottom: (child.pos[1] + child.size[1] + g.bottom - size[1]).max(0.0),
    }
}

fn meets(parent: &NodeData, child: &NodeData, g: Insets, at: [f32; 2]) -> bool {
    let bounds = Rect::new(
        at[0] - g.left,
        at[1] - g.top,
        child.size[0] + g.left + g.right,
        child.size[1] + g.top + g.bottom,
    );
    bounds
        .intersection(Rect::new(0.0, 0.0, parent.size[0], parent.size[1]))
        .is_some()
}

fn in_view(parent: &NodeData, child: &NodeData, at: [f32; 2]) -> bool {
    meets(parent, child, child.grow, at)
}

/// Paint bounds worked out from nothing, for the oracle.
fn fresh_grow(ui: &Ui, n: u32) -> Insets {
    let d = &ui.nodes[n as usize];
    let mut grow = d.own_grow;
    if !d.clip {
        for &c in d.children.iter().chain(&d.absolute) {
            let g = fresh_grow(ui, c);
            grow = grow.max(reach(&ui.nodes[c as usize], g, d.size));
        }
    }
    grow
}

pub(crate) fn quad_style(d: &NodeData) -> Option<QuadStyle> {
    let look = d.look;
    let fill = d.fill;
    let border = look.border.filter(|b| b.width > 0.0 && b.color[3] > 0.0);
    if fill[3] <= 0.0 && border.is_none() {
        return None;
    }
    Some(QuadStyle {
        background: Background::Solid(fill),
        border_widths: EdgeWidths::uniform(border.map_or(0.0, |b| b.width)),
        border_color: border.map_or([0.0; 4], |b| b.color),
        corner_radii: CornerRadii::uniform(look.radius),
    })
}
