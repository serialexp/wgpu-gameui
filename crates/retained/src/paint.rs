//! Drawing: each node with children keeps the list of its subtree at its
//! own origin, recorded without its ancestors' clips, and is placed into its
//! parent's list with `append_translated` at its position under the
//! parent's clip. A list is recorded again only when something in it is out
//! of date; otherwise it is placed as it is.
//!
//! A node that clips draws only the children whose paint bounds meet its
//! box, found by binary search along its stack, so its list depends on what
//! is in view and is recorded again when that changes.
//!
//! A list recorded again with the shape it had (the same paint runs over the
//! same number of records) is not placed again: its records are overwritten
//! where its parent's list holds them (`DrawList::patch_translated`), and the
//! runs overwritten are handed up for the grandparent to overwrite in its
//! list, and so on to the root. A change deep in the tree costs the records
//! it changed at each level, not every ancestor's list. Whatever the patch
//! cannot vouch for (another shape, a record its placement culled, a child
//! moved) records the parent again instead.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Background, CornerRadii, DrawList, EdgeWidths, QuadStyle, RecordSpan};

use crate::node::Insets;
use crate::own_drawing::paint_node;
use crate::ui::{NodeData, Ui};

impl Ui {
    /// Whether `n`'s list must be recorded again: something in it is out of
    /// date, a volatile painter is in it, a repaint it asked for has come, or
    /// it has none yet.
    pub(crate) fn needs_record(&self, n: u32) -> bool {
        let d = &self.nodes[n as usize];
        d.stale || d.volatile_below > 0 || d.deadline <= self.now || d.list.is_none()
    }

    /// Bring `n`'s list up to date, `n` being a node with children: record
    /// it again if it must be, else patch in it the children queued as
    /// changed, falling back to recording it when a patch cannot be made.
    /// The runs of records patched are pushed onto `spans`.
    pub(crate) fn update(&mut self, n: u32) -> Update {
        if self.needs_record(n) {
            return Update::Recorded {
                same_shape: self.record(n),
            };
        }
        if self.nodes[n as usize].dirty_kids.is_empty() {
            return Update::Clean;
        }
        let start = self.spans.len();
        let kids = std::mem::take(&mut self.nodes[n as usize].dirty_kids);
        for &c in &kids {
            if self.nodes[c as usize].parent == n {
                self.nodes[c as usize].queued = false;
            }
        }
        let patched = kids.iter().all(|&c| self.patch_child(n, c));
        let mut kids = kids;
        kids.clear();
        self.nodes[n as usize].dirty_kids = kids;
        if patched {
            return Update::Patched;
        }
        // The children not yet patched are drawn by the recording, which
        // brings each up to date first.
        self.spans.truncate(start);
        Update::Recorded {
            same_shape: self.record(n),
        }
    }

    /// Bring child `c`'s list up to date and patch its records where `n`'s
    /// list placed them, pushing the runs patched (in `n`'s list) onto
    /// `spans`. `false` when that cannot be done and `n` must be recorded
    /// again: `c` is not `n`'s child or draws straight into `n`'s list, its
    /// placement is not in `n`'s list as it is (it was culled, or `n` was
    /// recorded since) or is at another offset, `c` was recorded with
    /// another shape, or the patch is declined.
    fn patch_child(&mut self, n: u32, c: u32) -> bool {
        let p = &self.nodes[n as usize];
        let d = &self.nodes[c as usize];
        if !d.live || d.parent != n || !d.has_children() || d.placed_gen != p.list_gen {
            return false;
        }
        let Some(placed) = d.placed.as_deref().copied() else {
            return false;
        };
        if offset(d, p.scrolled()) != placed.offset {
            return false;
        }
        let mark = self.spans.len();
        match self.update(c) {
            Update::Clean => return true,
            Update::Recorded { same_shape: false } => return false,
            Update::Recorded { same_shape: true } => {
                self.spans.truncate(mark);
                let child = self.nodes[c as usize].list.as_deref().expect("recorded");
                self.spans.push(RecordSpan::all_of(child));
            }
            Update::Patched => {}
        }
        let mut list = self.nodes[n as usize]
            .list
            .take()
            .expect("a list placed it");
        let child = self.nodes[c as usize].list.as_deref().expect("recorded");
        let mut ok = true;
        for span in &mut self.spans[mark..] {
            if !list.patch_translated(&placed, child, span) {
                ok = false;
                break;
            }
            self.stats.patched += span.len();
            *span = span.offset_by(placed.at);
        }
        self.nodes[n as usize].list = Some(list);
        // A deadline that moved later leaves this one early, which records
        // this list again when it comes and puts it right.
        let deadline = self.nodes[c as usize].deadline;
        let d = &mut self.nodes[n as usize];
        d.deadline = d.deadline.min(deadline);
        ok
    }

    /// Record `n`'s list again: its own drawing, then its children in view.
    /// Whether it has the shape it had (see `DrawList::has_shape`), so the
    /// lists that placed it can be patched rather than recorded again.
    pub(crate) fn record(&mut self, n: u32) -> bool {
        self.stats.recorded += 1;
        let (mut list, had) = match self.nodes[n as usize].list.take() {
            Some(list) => (list, true),
            None => (
                Box::new(DrawList::with_font_system(self.fonts.clone())),
                false,
            ),
        };
        let mut shape = self.shapes.pop().unwrap_or_default();
        if had {
            list.shape_into(&mut shape);
        }
        list.clear();
        // Every child is drawn below, and so brought up to date; what was
        // queued is not wanted.
        let kids = std::mem::take(&mut self.nodes[n as usize].dirty_kids);
        for &c in &kids {
            if self.nodes[c as usize].parent == n {
                self.nodes[c as usize].queued = false;
            }
        }
        let mut kids = kids;
        kids.clear();
        let d = &mut self.nodes[n as usize];
        d.dirty_kids = kids;
        d.list_gen = d.list_gen.wrapping_add(1);
        let mut deadline = f64::INFINITY;
        self.stats.painted += 1;
        paint_node(
            &self.nodes[n as usize],
            &mut self.graph,
            self.now,
            &mut list,
            &mut deadline,
        );
        self.draw_children(n, &mut list, &mut deadline);
        let same_shape = had && list.has_shape(&shape);
        self.shapes.push(shape);
        let d = &mut self.nodes[n as usize];
        d.list = Some(list);
        d.deadline = deadline;
        d.stale = false;
        same_shape
    }

    fn draw_children(&mut self, n: u32, list: &mut DrawList, deadline: &mut f64) {
        let d = &self.nodes[n as usize];
        let clips = d.clip;
        let scrolled = d.scrolled();
        if clips {
            let rect = Rect::new(0.0, 0.0, d.size[0], d.size[1]);
            if d.scroll.is_some() {
                list.push_clip_viewport(rect);
            } else {
                list.push_clip(rect);
            }
        }
        let clip = list.current_clip();
        let range = self.flow_candidates(n);
        self.stats.examined += range.len() as u32;
        for i in range {
            let c = self.nodes[n as usize].children[i];
            let at = offset(&self.nodes[c as usize], scrolled);
            if clips && !in_view(&self.nodes[n as usize], &self.nodes[c as usize], at) {
                continue;
            }
            self.draw_child(c, at, list, clip, deadline);
        }
        for i in 0..self.nodes[n as usize].absolute.len() {
            let c = self.nodes[n as usize].absolute[i];
            self.stats.examined += 1;
            let at = offset(&self.nodes[c as usize], scrolled);
            if clips {
                self.grow(c);
                if !in_view(&self.nodes[n as usize], &self.nodes[c as usize], at) {
                    continue;
                }
            }
            self.draw_child(c, at, list, clip, deadline);
        }
        if clips {
            list.pop_clip();
        }
    }

    fn draw_child(
        &mut self,
        c: u32,
        at: [f32; 2],
        list: &mut DrawList,
        clip: Option<Rect>,
        deadline: &mut f64,
    ) {
        if self.nodes[c as usize].has_children() {
            let mark = self.spans.len();
            self.update(c);
            self.spans.truncate(mark);
            let parent_gen = self.nodes[self.nodes[c as usize].parent as usize].list_gen;
            let d = &mut self.nodes[c as usize];
            let child = d.list.as_deref().expect("recorded");
            let placed = list.append_translated(child, at, clip);
            self.stats.placed += 1;
            self.stats.copied += child.prim_counts().total() as u64;
            *deadline = deadline.min(d.deadline);
            match &mut d.placed {
                Some(kept) => **kept = placed,
                None => d.placed = Some(Box::new(placed)),
            }
            d.placed_gen = parent_gen;
        } else {
            self.stats.painted += 1;
            list.push_transform();
            list.translate(at[0], at[1]);
            paint_node(
                &self.nodes[c as usize],
                &mut self.graph,
                self.now,
                list,
                deadline,
            );
            list.pop_transform();
            self.nodes[c as usize].stale = false;
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
        let m = d.stack.value.direction.main();
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
        let m = self.nodes[n as usize].stack.value.direction.main();
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

/// `child`'s place in its parent's list: its position less the parent's
/// scroll.
/// What [`Ui::update`] did to a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    /// Nothing in it changed.
    Clean,
    /// Children's records were patched in it; the runs are on `spans`.
    Patched,
    /// It was recorded again, keeping its shape or not.
    Recorded { same_shape: bool },
}

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
    let look = d.look.value;
    let fill = d.fill.value;
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
