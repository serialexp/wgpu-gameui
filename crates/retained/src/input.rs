//! Pointer input: hit testing against the last frame's layout, hover and
//! press written to signals the app made, clicks, and the wheel.
//!
//! Input is handled when it comes, against what the user saw: the layout of
//! the last frame. Handlers write signals; the next frame draws what follows
//! from them. After each frame's layout the hovered node is found again from
//! the last pointer position, so a scroll, a move or a removal under a still
//! pointer moves the hover without a pointer event.

use std::ops::Range;

use crate::node::{Insets, Node};
use crate::paint::offset;
use crate::signal::Signal;
use crate::ui::{NodeData, Ui};

/// A pointer button.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Button {
    Primary,
    Secondary,
    Middle,
}

impl Button {
    const ALL: [Button; 3] = [Button::Primary, Button::Secondary, Button::Middle];

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

pub(crate) type ClickFn = Box<dyn FnMut(&mut Ui, Button)>;

/// How a node takes the pointer.
#[derive(Default)]
pub(crate) struct Input {
    pub(crate) hover: Option<Signal<bool>>,
    pub(crate) press: Option<Signal<bool>>,
    pub(crate) click: Option<ClickFn>,
    /// The buttons whose press it holds, one bit each.
    pub(crate) held: u8,
}

/// Pointer work, counted since the tree was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputStats {
    /// Hit tests run (pointer events, and one per layout round of a frame
    /// while the pointer is in the window).
    pub hit_tests: u64,
    /// Nodes those hit tests entered.
    pub entered: u64,
}

/// The pointer's state.
#[derive(Default)]
pub(crate) struct Pointer {
    /// Where it is, in the root's coordinates, while it is in the window.
    at: Option<[f32; 2]>,
    /// The node it is over, if that node takes the pointer.
    hovered: Option<u32>,
    /// The node holding each button's press.
    presses: [Option<u32>; 3],
    stats: InputStats,
}

impl Pointer {
    pub(crate) fn at(&self) -> Option<[f32; 2]> {
        self.at
    }
}

/// Takes hover, presses and clicks.
pub(crate) fn takes_pointer(d: &NodeData) -> bool {
    d.input.is_some()
}

/// Stops the wheel: takes the pointer, or scrolls.
fn takes_wheel(d: &NodeData) -> bool {
    d.input.is_some() || d.scroll.is_some()
}

fn inside(local: [f32; 2], size: [f32; 2]) -> bool {
    local[0] >= 0.0 && local[1] >= 0.0 && local[0] < size[0] && local[1] < size[1]
}

fn inside_grown(local: [f32; 2], size: [f32; 2], g: Insets) -> bool {
    local[0] >= -g.left
        && local[1] >= -g.top
        && local[0] < size[0] + g.right
        && local[1] < size[1] + g.bottom
}

impl Ui {
    /// The pointer moved to (`x`, `y`), in the root's coordinates.
    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.pointer.at = Some([x, y]);
        self.refresh_hover();
    }

    /// The pointer left the window: nothing is hovered, and every press is
    /// cancelled.
    pub fn pointer_left(&mut self) {
        self.pointer.at = None;
        self.set_hovered(None);
        self.cancel_pointer();
    }

    /// Cancels every press without a click (a drag taken by the host, a
    /// window losing focus).
    pub fn cancel_pointer(&mut self) {
        for button in Button::ALL {
            if let Some(n) = self.pointer.presses[button as usize].take() {
                self.release(n, button);
            }
        }
    }

    /// `button` went down or up at the pointer. A press belongs to the
    /// topmost node taking the pointer where it landed; its release over
    /// the same node is a click.
    pub fn pointer_button(&mut self, button: Button, pressed: bool) {
        let slot = button as usize;
        if pressed {
            if let Some(n) = self.pointer.presses[slot].take() {
                self.release(n, button);
            }
            let Some(at) = self.pointer.at else {
                return;
            };
            let Some(n) = self.hit(at, takes_pointer) else {
                return;
            };
            self.pointer.presses[slot] = Some(n);
            let input = self.nodes[n as usize]
                .input
                .as_mut()
                .expect("takes the pointer");
            let was = input.held;
            input.held |= button.bit();
            if was == 0
                && let Some(signal) = input.press
            {
                self.graph.set(signal, true);
            }
        } else {
            let Some(n) = self.pointer.presses[slot].take() else {
                return;
            };
            self.release(n, button);
            let over = match self.pointer.at {
                Some(at) => self.hit(at, takes_pointer),
                None => None,
            };
            if over == Some(n) {
                self.click(n, button);
            }
        }
    }

    /// The wheel turned by (`dx`, `dy`) pixels: the innermost scroll
    /// container under the pointer that scrolls along an axis that moved
    /// scrolls by that much (positive toward its end). At its limit the
    /// step goes nowhere else.
    pub fn wheel(&mut self, dx: f32, dy: f32) {
        let Some(at) = self.pointer.at else {
            return;
        };
        let mut x = self.hit(at, takes_wheel);
        while let Some(n) = x {
            let d = &self.nodes[n as usize];
            if d.scroll.is_some() {
                let delta = [dx, dy][d.stack.direction.main()];
                if delta != 0.0 {
                    let node = Node {
                        index: n,
                        generation: d.generation,
                    };
                    self.scroll_by(node, delta);
                    return;
                }
            }
            x = d.parent();
        }
    }

    /// The node the pointer is over, if one takes the pointer there.
    pub fn hovered(&self) -> Option<Node> {
        self.pointer.hovered.map(|n| self.handle(n))
    }

    /// The topmost node taking the pointer at (`x`, `y`) in the last
    /// frame's layout.
    pub fn hit_test(&mut self, x: f32, y: f32) -> Option<Node> {
        self.hit([x, y], takes_pointer).map(|n| self.handle(n))
    }

    /// The pointer work done since the tree was made.
    pub fn input_stats(&self) -> InputStats {
        self.pointer.stats
    }

    /// Find the hovered node again, at the pointer, in the current layout.
    pub(crate) fn refresh_hover(&mut self) {
        let over = match self.pointer.at {
            Some(at) => self.hit(at, takes_pointer),
            None => None,
        };
        self.set_hovered(over);
    }

    fn set_hovered(&mut self, over: Option<u32>) {
        if over == self.pointer.hovered {
            return;
        }
        if let Some(old) = self.pointer.hovered {
            self.write_hover(old, false);
        }
        if let Some(new) = over {
            self.write_hover(new, true);
        }
        self.pointer.hovered = over;
    }

    fn write_hover(&mut self, n: u32, value: bool) {
        if let Some(signal) = self.nodes[n as usize].input.as_ref().and_then(|i| i.hover) {
            self.graph.set(signal, value);
        }
    }

    /// `n` lets go of `button`'s press.
    fn release(&mut self, n: u32, button: Button) {
        let input = self.nodes[n as usize]
            .input
            .as_mut()
            .expect("takes the pointer");
        input.held &= !button.bit();
        if input.held == 0
            && let Some(signal) = input.press
        {
            self.graph.set(signal, false);
        }
    }

    /// Run `n`'s click handler, which may change the tree, `n` included.
    fn click(&mut self, n: u32, button: Button) {
        let generation = self.nodes[n as usize].generation;
        let Some(mut click) = self.nodes[n as usize]
            .input
            .as_mut()
            .and_then(|i| i.click.take())
        else {
            return;
        };
        click(self, button);
        let d = &mut self.nodes[n as usize];
        if d.live
            && d.generation == generation
            && let Some(input) = d.input.as_mut()
            && input.click.is_none()
        {
            input.click = Some(click);
        }
    }

    /// `n` is being removed: what the pointer holds of it goes. Its hover
    /// signal (the app's) is written false; its presses end without a
    /// click.
    pub(crate) fn forget_pointer(&mut self, n: u32) {
        if self.pointer.hovered == Some(n) {
            self.write_hover(n, false);
            self.pointer.hovered = None;
        }
        for button in Button::ALL {
            if self.pointer.presses[button as usize] == Some(n) {
                self.pointer.presses[button as usize] = None;
                self.release(n, button);
            }
        }
    }

    fn handle(&self, n: u32) -> Node {
        Node {
            index: n,
            generation: self.nodes[n as usize].generation,
        }
    }

    /// The topmost node at `at` for which `want` holds, in reverse paint
    /// order: clipped-away parts are not hit, overflowing children are.
    pub(crate) fn hit(&mut self, at: [f32; 2], want: fn(&NodeData) -> bool) -> Option<u32> {
        self.pointer.stats.hit_tests += 1;
        self.context.as_ref()?;
        self.hit_in(self.root, [0.0; 2], at, want)
    }

    fn hit_in(
        &mut self,
        n: u32,
        origin: [f32; 2],
        at: [f32; 2],
        want: fn(&NodeData) -> bool,
    ) -> Option<u32> {
        self.pointer.stats.entered += 1;
        let local = [at[0] - origin[0], at[1] - origin[1]];
        let d = &self.nodes[n as usize];
        let size = d.size;
        let within = inside(local, size);
        // Children are hit only where they show: inside a node that clips,
        // inside the paint bounds (which hold the children's boxes) of one
        // that does not.
        let reach = d.has_children()
            && if d.clip {
                within
            } else {
                within || {
                    let g = self.grow(n);
                    inside_grown(local, size, g)
                }
            };
        if reach {
            let scrolled = self.nodes[n as usize].scrolled();
            for i in (0..self.nodes[n as usize].absolute.len()).rev() {
                let c = self.nodes[n as usize].absolute[i];
                let o = offset(&self.nodes[c as usize], scrolled);
                if let Some(hit) = self.hit_in(c, [origin[0] + o[0], origin[1] + o[1]], at, want) {
                    return Some(hit);
                }
            }
            for i in self.hit_candidates(n, local).rev() {
                let c = self.nodes[n as usize].children[i];
                if self.nodes[c as usize].deferred {
                    continue;
                }
                let o = offset(&self.nodes[c as usize], scrolled);
                if let Some(hit) = self.hit_in(c, [origin[0] + o[0], origin[1] + o[1]], at, want) {
                    return Some(hit);
                }
            }
        }
        (within && want(&self.nodes[n as usize])).then_some(n)
    }

    /// The stacked children of `n` whose span along the stack, grown by the
    /// most any of them paints outside it, holds `local` (in `n`'s box):
    /// found by binary search, as the drawing finds those in view.
    fn hit_candidates(&mut self, n: u32, local: [f32; 2]) -> Range<usize> {
        if self.nodes[n as usize].child_grow_stale {
            self.refresh_child_grow(n);
        }
        let d = &self.nodes[n as usize];
        let m = d.stack.direction.main();
        let q = local[m] + d.scrolled()[m];
        let [before, after] = d.child_grow;
        let nodes = &self.nodes;
        let lo = d.children.partition_point(|&c| {
            let c = &nodes[c as usize];
            c.pos[m] + c.size[m] + after <= q
        });
        let hi = d.children.partition_point(|&c| {
            let c = &nodes[c as usize];
            c.pos[m] - before <= q
        });
        lo..hi.max(lo)
    }

    /// What the pointer holds agrees with the layout and with the signals
    /// it writes: the hovered node is the one found from nothing at the
    /// pointer, each node's held buttons are the presses it owns, and each
    /// hover and press signal says so. For tests.
    #[cfg(test)]
    pub(crate) fn check_pointer(&mut self) {
        let expected = self
            .pointer
            .at
            .and_then(|at| self.hit_from_nothing(at, takes_pointer));
        assert_eq!(
            self.pointer.hovered, expected,
            "the hovered node is the one under the pointer"
        );
        for n in 0..self.nodes.len() as u32 {
            let d = &self.nodes[n as usize];
            let Some(input) = d.input.as_ref().filter(|_| d.live) else {
                continue;
            };
            let owned = Button::ALL
                .iter()
                .filter(|b| self.pointer.presses[**b as usize] == Some(n))
                .fold(0, |bits, b| bits | b.bit());
            assert_eq!(input.held, owned, "node {n} holds the presses it owns");
            let (hover, press, held) = (input.hover, input.press, input.held);
            if let Some(signal) = hover {
                let hovered = self.pointer.hovered == Some(n);
                assert_eq!(self.graph.get(signal), hovered, "node {n}'s hover signal");
            }
            if let Some(signal) = press {
                assert_eq!(self.graph.get(signal), held != 0, "node {n}'s press signal");
            }
        }
        for press in self.pointer.presses.into_iter().flatten() {
            assert!(self.nodes[press as usize].live, "a press on a removed node");
        }
    }

    /// The topmost node at `at` for which `want` holds, found by visiting
    /// every node in paint order with its clip. For tests.
    #[cfg(test)]
    pub(crate) fn hit_from_nothing(
        &self,
        at: [f32; 2],
        want: fn(&NodeData) -> bool,
    ) -> Option<u32> {
        let mut found = None;
        self.scan(self.root, [0.0; 2], None, at, want, &mut found);
        found
    }

    #[cfg(test)]
    fn scan(
        &self,
        n: u32,
        origin: [f32; 2],
        clip: Option<[f32; 4]>,
        at: [f32; 2],
        want: fn(&NodeData) -> bool,
        found: &mut Option<u32>,
    ) {
        let contains = |r: [f32; 4]| at[0] >= r[0] && at[1] >= r[1] && at[0] < r[2] && at[1] < r[3];
        let d = &self.nodes[n as usize];
        let rect = [
            origin[0],
            origin[1],
            origin[0] + d.size[0],
            origin[1] + d.size[1],
        ];
        if clip.is_none_or(contains) && contains(rect) && want(d) {
            *found = Some(n);
        }
        let clip = if d.clip {
            Some(match clip {
                Some(c) => [
                    c[0].max(rect[0]),
                    c[1].max(rect[1]),
                    c[2].min(rect[2]),
                    c[3].min(rect[3]),
                ],
                None => rect,
            })
        } else {
            clip
        };
        let scrolled = d.scrolled();
        for &c in d.children.iter().chain(&d.absolute) {
            if self.nodes[c as usize].deferred {
                continue;
            }
            let o = offset(&self.nodes[c as usize], scrolled);
            self.scan(
                c,
                [origin[0] + o[0], origin[1] + o[1]],
                clip,
                at,
                want,
                found,
            );
        }
    }
}
