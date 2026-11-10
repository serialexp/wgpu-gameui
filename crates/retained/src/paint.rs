//! Drawing: each node with children keeps the list of its subtree at its
//! own origin, recorded without its ancestors' clips, and is placed into its
//! parent's list with `append_translated` at its position under the
//! parent's clip. A list is recorded again only when something in it is out
//! of date; otherwise it is placed as it is.
//!
//! A node that clips draws only the children whose paint bounds meet its
//! box, found by binary search along its stack, so its list depends on what
//! is in view and is recorded again when that changes.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{Background, CornerRadii, DrawList, EdgeWidths, QuadStyle};

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

    /// Record `n`'s list again: its own drawing, then its children in view.
    pub(crate) fn record(&mut self, n: u32) {
        self.stats.recorded += 1;
        let mut list = match self.nodes[n as usize].list.take() {
            Some(list) => list,
            None => Box::new(DrawList::with_font_system(self.fonts.clone())),
        };
        list.clear();
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
        let d = &mut self.nodes[n as usize];
        d.list = Some(list);
        d.deadline = deadline;
        d.stale = false;
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
            if self.needs_record(c) {
                self.record(c);
            }
            let d = &self.nodes[c as usize];
            let child = d.list.as_deref().expect("recorded");
            list.append_translated(child, at, clip);
            self.stats.placed += 1;
            self.stats.copied += child.prim_counts().total() as u64;
            *deadline = deadline.min(d.deadline);
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
