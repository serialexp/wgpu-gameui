//! The tree: nodes, their properties, and the frame that lays them out and
//! draws them.

use wgpu_gameui::{
    DrawList, FontSystemHandle, PlacedLog, PreparedLayout, PreparedText, RecordSpan, TextMeasurer,
};

use crate::input::{Input, Pointer};
use crate::layout::Avail;
use crate::node::{CustomPaint, Insets, Look, Node, Prop, Size, Spec, SpecContent, Stack, Text};
use crate::signal::{Graph, Memo, NodeId, Owner, Readable, Reader, Signal};
use crate::texts::{Link, Texts};

/// What the frame is drawn in: what, outside the tree, its layout and
/// drawing depend on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameContext {
    /// The frame's time, in seconds of the host's clock. Repaint deadlines
    /// are in this clock; nothing in the tree reads a clock of its own.
    pub now: f64,
    /// The window's logical size; the root is laid out to it.
    pub viewport: [f32; 2],
    /// The font system's generation: when it moves (a font loaded), every
    /// text is measured and drawn again.
    pub fonts: u64,
    /// The scale factor; when it moves, every text is measured and drawn
    /// again.
    pub scale: f32,
}

/// What the last frame did, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Property notifications taken from the graph.
    pub notified: u32,
    /// Nodes the layout walk entered, laid out or not.
    pub visited: u32,
    /// Nodes laid out (not taken from their cached layout).
    pub laid_out: u32,
    /// Texts measured.
    pub measured: u32,
    /// Children of a scroll sized without being laid out, their layout put
    /// off until they are near its view: those near it are laid out next,
    /// the rest when the scroll brings them near.
    pub deferred: u32,
    /// Nodes whose own drawing (box, text, painter) was recorded.
    pub painted: u32,
    /// Lists recorded again (each kept by the root or a node that clips).
    pub recorded: u32,
    /// Lists placed into the list that holds them.
    pub placed: u32,
    /// Lists dropped because the tree as drawn no longer places them (a
    /// node that clips scrolled out of view, or one whose holder was): each
    /// is recorded again when it is drawn again.
    pub dropped: u32,
    /// Records copied by those placements.
    pub copied: u64,
    /// Records overwritten where they were in an ancestor's list, carrying a
    /// node's drawing, or a list recorded again, with its shape up the tree
    /// instead of recording the ancestors again.
    pub patched: u64,
    /// Children looked at to decide whether they are in view.
    pub examined: u32,
    /// Memos run.
    pub memo_runs: u64,
}

/// What the tree holds, counted: for measuring what retaining costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeStats {
    /// Live nodes.
    pub nodes: usize,
    /// Lists kept: the root's, and one per node that clips while the tree as
    /// last drawn places it.
    pub lists: usize,
    /// Records those lists hold.
    pub records: u64,
    /// Bytes those lists, and the logs of what they placed, have reserved
    /// (see `DrawList::heap_bytes`).
    pub list_bytes: u64,
    /// Bytes the node table and the table of bound properties have
    /// reserved, live nodes or not (each node's own allocations, such as its
    /// input handlers, are not counted).
    pub node_bytes: u64,
    /// Texts keeping their shaping, to be measured and laid out again
    /// without shaping them (see [`Ui::set_text_budget`]).
    pub texts_prepared: usize,
    /// Roughly the bytes their shaping holds (see
    /// `PreparedText::heap_bytes`), kept within the budget.
    pub text_bytes: u64,
    /// Shaping dropped to keep to the budget, since the tree was made.
    pub texts_dropped: u64,
}

pub(crate) const WIDTH: u8 = 0;
pub(crate) const HEIGHT: u8 = 1;
pub(crate) const STACK: u8 = 2;
pub(crate) const FILL: u8 = 3;
pub(crate) const LOOK: u8 = 4;
pub(crate) const OFFSET: u8 = 5;
pub(crate) const TEXT: u8 = 6;
pub(crate) const TEXT_COLOR: u8 = 7;
pub(crate) const CUSTOM: u8 = 8;
pub(crate) const SCROLL: u8 = 9;

pub(crate) fn key(index: u32, prop: u8) -> u64 {
    (u64::from(index) << 4) | u64::from(prop)
}

slotmap::new_key_type! {
    /// A bound property's entry in [`Ui::bindings`].
    pub(crate) struct BindingKey;
}

/// A property of a node that follows a memo, the node holding only its
/// current value: most properties are constants, and keep nothing else.
/// The memo is the node's own, reading the memo the property was bound to
/// (so that any number of nodes can follow one memo) or the signal its
/// setter writes. A node's bindings are chained from
/// [`NodeData::bindings`], one per property at most.
pub(crate) struct Binding {
    /// Which property (`WIDTH`, `TEXT`, ...).
    pub(crate) prop: u8,
    /// The memo it follows, of the property's type.
    pub(crate) memo: NodeId,
    /// The signal a setter writes, once a setter has been used.
    pub(crate) setter: Option<NodeId>,
    /// The memo's version when the value was last taken.
    pub(crate) version: u64,
    /// The node's next binding.
    pub(crate) next: Option<BindingKey>,
}

pub(crate) struct TextContent {
    pub(crate) text: Text,
    pub(crate) color: [f32; 4],
    /// The text's measured size, whole pixels.
    pub(crate) size: [f32; 2],
    /// Its shaping, from its first measure, until the budget drops it
    /// (`texts.rs`): it is measured and laid out from this, shaped again only
    /// when what shaping depends on changed or the fonts did.
    pub(crate) prepared: Option<PreparedText>,
    /// The layout it is drawn with, made when its size was settled: drawing
    /// it, and the frame drawn from nothing, only read it.
    pub(crate) drawn: Option<PreparedLayout>,
    /// Its place in the tree's chain of texts holding their shaping.
    pub(crate) link: Link,
}

/// The list a node keeps (see `Ui::keeps_list`), and the log of what it
/// placed: where each node drawn into it went and its shape then.
pub(crate) struct Kept {
    pub(crate) list: DrawList,
    pub(crate) log: PlacedLog,
    /// Its node's place in `Ui::keepers`.
    pub(crate) slot: u32,
}

pub(crate) struct CustomNode {
    pub(crate) paint: Box<dyn CustomPaint>,
    pub(crate) version: u64,
    pub(crate) volatile: bool,
    pub(crate) overflow: Insets,
}

pub(crate) struct Scroll {
    pub(crate) signal: Signal<f32>,
    pub(crate) memo: Memo<f32>,
    pub(crate) version: u64,
    /// The offset asked for.
    pub(crate) requested: f32,
    /// The offset drawn: clamped to the content, whole pixels.
    pub(crate) applied: f32,
    /// The content's length along the stack, padding included.
    pub(crate) extent: f32,
    /// Its content's length across the stack is exact, so the children it
    /// can size without laying them out (`layout::deferrable`) are laid out
    /// only within a viewport of its view.
    pub(crate) defers: bool,
    /// Some stacked child is one it cannot defer: those are laid out
    /// wherever they are, so finding what is dirty means looking at every
    /// child, not only those near the view.
    pub(crate) mixed: bool,
}

pub(crate) const NO_PARENT: u32 = u32::MAX;

pub(crate) struct NodeData {
    pub(crate) generation: u32,
    pub(crate) live: bool,
    pub(crate) parent: u32,
    /// Stacked children, in order.
    pub(crate) children: Vec<u32>,
    /// Children placed at an offset, drawn after the stacked ones.
    pub(crate) absolute: Vec<u32>,
    pub(crate) owner: Owner,
    /// Its bound properties, chained; `None` when all are constants.
    pub(crate) bindings: Option<BindingKey>,
    pub(crate) width: Size,
    pub(crate) height: Size,
    pub(crate) stack: Stack,
    pub(crate) fill: [f32; 4],
    pub(crate) look: Look,
    pub(crate) offset: Option<[f32; 2]>,
    pub(crate) text: Option<Box<TextContent>>,
    pub(crate) custom: Option<Box<CustomNode>>,
    pub(crate) clip: bool,
    pub(crate) scroll: Option<Box<Scroll>>,
    pub(crate) input: Option<Box<Input>>,

    // Layout.
    /// The room it was last laid out in.
    pub(crate) avail: Option<[Avail; 2]>,
    pub(crate) size: [f32; 2],
    /// Its place in its parent's box, before the parent's scroll.
    pub(crate) pos: [f32; 2],
    /// It must be laid out again.
    pub(crate) layout_self: bool,
    /// Something below it must be laid out again.
    pub(crate) layout_below: bool,
    /// Its layout was put off, it being out of its scroll's view: its size
    /// is right, nothing in it is laid out, and it is neither drawn nor hit
    /// until it is.
    pub(crate) deferred: bool,

    // Paint.
    /// What it draws is out of date: for a node that keeps a list, the list
    /// must be recorded again; for one drawn into its holder's list, its own
    /// drawing must be drawn again.
    pub(crate) stale: bool,
    /// The frame it was last marked in: a mark stops where one was already
    /// made this frame, since everything above it is marked too.
    pub(crate) mark_epoch: u32,
    /// For a node that keeps a list: the nodes drawn into it whose records
    /// may have changed since, each once (see `queued`), with their
    /// generations then; the list is brought up to date by patching their
    /// records in it where it can.
    pub(crate) dirty_kids: Vec<(u32, u32)>,
    /// It is in its holder's `dirty_kids`.
    pub(crate) queued: bool,
    /// Its piece in its holder's log (its own list placed there, or, for a
    /// node that keeps none, its own drawing), and the holder's `list_gen`
    /// then: the piece holds while the holder's list is the one it was
    /// placed in.
    pub(crate) piece: u32,
    pub(crate) placed_gen: u32,
    /// Counts the times its list was recorded.
    pub(crate) list_gen: u32,
    /// Volatile painters in its subtree, itself included: its list is
    /// recorded every frame while there are any.
    pub(crate) volatile_below: u32,
    /// The list of its subtree, at its origin, and the log of what it
    /// placed, for a node that keeps one (see `Ui::keeps_list`), once
    /// recorded.
    pub(crate) kept: Option<Box<Kept>>,
    /// The earliest repaint asked for by what its list holds.
    pub(crate) deadline: f64,
    /// How far its own drawing reaches outside its box.
    pub(crate) own_grow: Insets,
    /// How far its drawing, and its children's unless it clips, reaches
    /// outside its box: its paint bounds.
    pub(crate) grow: Insets,
    pub(crate) grow_stale: bool,
    /// The most any stacked child's paint bounds reach before and after it
    /// along the stack: for culling (in a node that clips) and hit testing.
    pub(crate) child_grow: [f32; 2],
    pub(crate) child_grow_stale: bool,
}

impl NodeData {
    pub(crate) fn has_children(&self) -> bool {
        !self.children.is_empty() || !self.absolute.is_empty()
    }

    pub(crate) fn parent(&self) -> Option<u32> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }

    /// The scroll offset drawn, as a vector.
    pub(crate) fn scrolled(&self) -> [f32; 2] {
        match &self.scroll {
            Some(s) => {
                let mut v = [0.0; 2];
                v[self.stack.direction.main()] = s.applied;
                v
            }
            None => [0.0; 2],
        }
    }
}

/// A retained tree drawn with gameui: nodes whose properties are constants
/// or memos, laid out and drawn again only where something changed.
///
/// The host forwards a frame when [`needs_frame`](Self::needs_frame) says
/// so, and draws the list [`frame`](Self::frame) returns. Between, it
/// writes signals (or uses the setters, which write signals the nodes own).
pub struct Ui {
    pub(crate) graph: Graph,
    pub(crate) nodes: Vec<NodeData>,
    /// The nodes' bound properties (see [`Binding`]).
    pub(crate) bindings: slotmap::SlotMap<BindingKey, Binding>,
    free: Vec<u32>,
    pub(crate) root: u32,
    pub(crate) fonts: FontSystemHandle,
    pub(crate) measurer: TextMeasurer,
    /// The texts keeping their shaping, and their budget.
    pub(crate) texts: Texts,
    pub(crate) epoch: u32,
    pub(crate) pending: bool,
    keys: Vec<u64>,
    pub(crate) context: Option<FrameContext>,
    pub(crate) now: f64,
    pub(crate) stats: FrameStats,
    live: usize,
    scratch: Vec<u32>,
    pub(crate) scratch_f32: Vec<f32>,
    pub(crate) pointer: Pointer,
    /// The runs of records patched in the list being brought up to date,
    /// for its parent to patch in turn: each update's after those of the
    /// ones it called.
    pub(crate) spans: Vec<RecordSpan>,
    /// The list a node that keeps none draws its own drawing into, before
    /// it is placed in its holder's list; reused.
    pub(crate) piece_list: Option<Box<DrawList>>,
    /// The nodes keeping a list, each at the `slot` its `Kept` holds: so
    /// that the lists no longer placed are found without a pass over every
    /// node.
    pub(crate) keepers: Vec<u32>,
    /// Lay out every child of a scroll, as if none could be put off: for
    /// comparing a tree that defers with one that does not.
    #[cfg(test)]
    pub(crate) no_defer: bool,
    /// Laying out from nothing to check the incremental layout: texts are
    /// measured through the shared cache, their kept shaping untouched.
    #[cfg(test)]
    pub(crate) checking: bool,
}

impl Ui {
    /// An empty tree, drawing text with `fonts` (share the renderer's, so
    /// that what is measured is what is drawn). The root fills the
    /// viewport.
    pub fn new(fonts: FontSystemHandle) -> Self {
        Self::with_root(
            fonts,
            Spec::new().width(Size::Fill(1.0)).height(Size::Fill(1.0)),
        )
    }

    /// An empty tree whose root is `root`.
    pub fn with_root(fonts: FontSystemHandle, root: Spec) -> Self {
        let mut ui = Self {
            graph: Graph::new(),
            nodes: Vec::new(),
            bindings: slotmap::SlotMap::with_key(),
            free: Vec::new(),
            root: 0,
            measurer: TextMeasurer::with_font_system(fonts.clone()),
            fonts,
            texts: Texts::default(),
            epoch: 1,
            pending: true,
            keys: Vec::new(),
            context: None,
            now: 0.0,
            stats: FrameStats::default(),
            live: 0,
            scratch: Vec::new(),
            scratch_f32: Vec::new(),
            pointer: Pointer::default(),
            spans: Vec::new(),
            piece_list: None,
            keepers: Vec::new(),
            #[cfg(test)]
            no_defer: false,
            #[cfg(test)]
            checking: false,
        };
        let owner = ui.graph.root();
        ui.root = ui.create(NO_PARENT, owner, root);
        ui
    }

    /// The root node.
    pub fn root(&self) -> Node {
        Node {
            index: self.root,
            generation: self.nodes[self.root as usize].generation,
        }
    }

    // ---- The graph ----

    /// The reactive graph, to read and write signals and memos directly.
    pub fn graph(&mut self) -> &mut Graph {
        &mut self.graph
    }

    /// A signal owned by the root.
    pub fn signal<T: PartialEq + 'static>(&mut self, value: T) -> Signal<T> {
        self.graph.signal(value)
    }

    /// A memo owned by the root.
    pub fn memo<T, F>(&mut self, compute: F) -> Memo<T>
    where
        T: PartialEq + 'static,
        F: FnMut(&mut Reader<'_>) -> T + 'static,
    {
        self.graph.memo(compute)
    }

    /// A memo owned by `node`: removed with it.
    pub fn memo_in<T, F>(&mut self, node: Node, compute: F) -> Memo<T>
    where
        T: PartialEq + 'static,
        F: FnMut(&mut Reader<'_>) -> T + 'static,
    {
        let owner = self.nodes[self.index(node) as usize].owner;
        self.graph.memo_in(owner, compute)
    }

    /// Set `signal`; see [`Graph::set`].
    pub fn set<T: PartialEq + 'static>(&mut self, signal: Signal<T>, value: T) -> bool {
        self.graph.set(signal, value)
    }

    /// The value of a signal or memo, brought up to date.
    pub fn get<T: Clone + 'static>(&mut self, source: impl Readable<T>) -> T {
        self.graph.get(source)
    }

    // ---- Building the tree ----

    /// Add a node built from `spec` as `parent`'s last child (or, for an
    /// absolute one, its last absolute child).
    pub fn add(&mut self, parent: Node, spec: Spec) -> Node {
        let parent = self.index(parent);
        let len = self.nodes[parent as usize].children.len();
        self.insert_at(parent, len, spec)
    }

    /// Add a node built from `spec` as `parent`'s stacked child at `index`.
    pub fn insert(&mut self, parent: Node, index: usize, spec: Spec) -> Node {
        let parent = self.index(parent);
        self.insert_at(parent, index, spec)
    }

    fn insert_at(&mut self, parent: u32, at: usize, spec: Spec) -> Node {
        let p = &self.nodes[parent as usize];
        assert!(
            p.text.is_none() && p.custom.is_none(),
            "a node with text or a painter takes no children"
        );
        let absolute = spec.offset.is_some();
        let owner = self.graph.owner(p.owner);
        let index = self.create(parent, owner, spec);
        let p = &mut self.nodes[parent as usize];
        if absolute {
            p.absolute.push(index);
        } else {
            p.children.insert(at, index);
            p.child_grow_stale = true;
        }
        self.mark_layout(parent, false);
        self.mark_records(parent);
        let volatile = self.nodes[index as usize].volatile_below;
        if volatile > 0 {
            self.add_volatile_above(index, volatile, true);
        }
        Node {
            index,
            generation: self.nodes[index as usize].generation,
        }
    }

    fn create(&mut self, parent: u32, owner: Owner, spec: Spec) -> u32 {
        let index = match self.free.last() {
            Some(&i) => i,
            None => self.nodes.len() as u32,
        };
        let mut bindings = None;
        let width = self.prop(spec.width, WIDTH, (owner, index, &mut bindings));
        let height = self.prop(spec.height, HEIGHT, (owner, index, &mut bindings));
        let stack = self.prop(spec.stack, STACK, (owner, index, &mut bindings));
        let fill = self.prop(spec.fill, FILL, (owner, index, &mut bindings));
        let look = self.prop(spec.look, LOOK, (owner, index, &mut bindings));
        let offset = spec
            .offset
            .map(|o| self.prop(o, OFFSET, (owner, index, &mut bindings)));
        let mut text = None;
        let mut custom = None;
        match spec.content {
            SpecContent::None => {}
            SpecContent::Text(t, c) => {
                let t = self.prop(t, TEXT, (owner, index, &mut bindings));
                let c = self.prop(c, TEXT_COLOR, (owner, index, &mut bindings));
                text = Some(Box::new(TextContent {
                    text: t,
                    color: c,
                    size: [0.0; 2],
                    prepared: None,
                    drawn: None,
                    link: Link::default(),
                }));
            }
            SpecContent::Custom(painter) => {
                let paint = (painter.make)(&mut self.graph, owner);
                let version = paint.version(&mut self.graph);
                paint.observe(&mut self.graph, key(index, CUSTOM));
                custom = Some(Box::new(CustomNode {
                    paint,
                    version,
                    volatile: painter.volatile,
                    overflow: painter.overflow,
                }));
            }
        }
        let scroll = spec.scroll.then(|| {
            let signal = self.graph.signal_in(owner, 0.0f32);
            let memo = self.graph.memo_in(owner, move |r| r.get(signal));
            let version = self.graph.version(memo);
            self.graph.observe(memo, key(index, SCROLL));
            Box::new(Scroll {
                signal,
                memo,
                version,
                requested: 0.0,
                applied: 0.0,
                extent: 0.0,
                defers: false,
                mixed: false,
            })
        });
        let volatile = custom.as_ref().is_some_and(|c| c.volatile);
        let generation = match self.nodes.get(index as usize) {
            Some(old) => old.generation,
            None => 0,
        };
        let data = NodeData {
            generation,
            live: true,
            parent,
            children: Vec::new(),
            absolute: Vec::new(),
            owner,
            bindings,
            width,
            height,
            stack,
            fill,
            look,
            offset,
            text,
            custom,
            clip: spec.clip,
            scroll,
            input: spec.input,
            avail: None,
            size: [0.0; 2],
            pos: [0.0; 2],
            layout_self: true,
            layout_below: false,
            deferred: false,
            stale: true,
            mark_epoch: 0,
            dirty_kids: Vec::new(),
            queued: false,
            piece: 0,
            placed_gen: 0,
            list_gen: 0,
            volatile_below: u32::from(volatile),
            kept: None,
            deadline: f64::INFINITY,
            own_grow: Insets::default(),
            grow: Insets::default(),
            grow_stale: true,
            child_grow: [0.0; 2],
            child_grow_stale: true,
        };
        if index as usize == self.nodes.len() {
            self.nodes.push(data);
        } else {
            self.free.pop();
            self.nodes[index as usize] = data;
        }
        self.live += 1;
        self.pending = true;
        index
    }

    /// The value property `id` of node `index` starts with; a bound one is
    /// given a memo owned by `owner`, and a binding chained onto `chain`.
    fn prop<T: Clone + PartialEq + 'static>(
        &mut self,
        prop: Prop<T>,
        id: u8,
        (owner, index, chain): (Owner, u32, &mut Option<BindingKey>),
    ) -> T {
        match prop {
            Prop::Const(value) => value,
            Prop::Bound(source) => {
                let memo = self.graph.memo_in(owner, move |r| r.get(source));
                let value = self.graph.get(memo);
                let version = self.graph.version(memo);
                self.graph.observe(memo, key(index, id));
                *chain = Some(self.bindings.insert(Binding {
                    prop: id,
                    memo: memo.id(),
                    setter: None,
                    version,
                    next: *chain,
                }));
                value
            }
        }
    }

    /// Node `n`'s binding of property `prop`, if it is bound.
    fn binding(&self, n: u32, prop: u8) -> Option<BindingKey> {
        let mut at = self.nodes[n as usize].bindings;
        while let Some(k) = at {
            let b = &self.bindings[k];
            if b.prop == prop {
                return Some(k);
            }
            at = b.next;
        }
        None
    }

    /// Remove `node` and everything below it, with every signal and memo
    /// they own. Panics on the root, or if a memo outside the subtree still
    /// reads one inside it.
    pub fn remove(&mut self, node: Node) {
        let n = self.index(node);
        assert!(n != self.root, "the root is not removed");
        let parent = self.nodes[n as usize].parent;
        let volatile = self.nodes[n as usize].volatile_below;
        if volatile > 0 {
            self.add_volatile_above(n, volatile, false);
        }
        let p = &mut self.nodes[parent as usize];
        if let Some(at) = p.children.iter().position(|&c| c == n) {
            p.children.remove(at);
            p.child_grow_stale = true;
        } else {
            let at = p
                .absolute
                .iter()
                .position(|&c| c == n)
                .expect("a child is in its parent's lists");
            p.absolute.remove(at);
        }
        self.mark_layout(parent, false);
        self.mark_records(parent);
        self.graph.dispose(self.nodes[n as usize].owner);

        let mut stack = std::mem::take(&mut self.scratch);
        stack.push(n);
        while let Some(x) = stack.pop() {
            let d = &self.nodes[x as usize];
            stack.extend_from_slice(&d.children);
            stack.extend_from_slice(&d.absolute);
            if d.input.is_some() {
                self.forget_pointer(x);
            }
            if self.nodes[x as usize].text.is_some() {
                self.forget_text(x);
            }
            self.drop_kept(x);
            let d = &mut self.nodes[x as usize];
            d.live = false;
            d.generation = d.generation.wrapping_add(1);
            d.children = Vec::new();
            d.absolute = Vec::new();
            d.dirty_kids = Vec::new();
            let mut at = d.bindings.take();
            while let Some(k) = at {
                at = self.bindings.remove(k).and_then(|b| b.next);
            }
            let d = &mut self.nodes[x as usize];
            d.text = None;
            d.custom = None;
            d.scroll = None;
            d.offset = None;
            d.input = None;
            self.free.push(x);
            self.live -= 1;
        }
        self.scratch = stack;
    }

    fn add_volatile_above(&mut self, n: u32, count: u32, add: bool) {
        let mut p = self.nodes[n as usize].parent();
        while let Some(x) = p {
            let d = &mut self.nodes[x as usize];
            if add {
                d.volatile_below += count;
            } else {
                d.volatile_below -= count;
            }
            p = d.parent();
        }
    }

    pub(crate) fn index(&self, node: Node) -> u32 {
        let d = &self.nodes[node.index as usize];
        assert!(
            d.live && d.generation == node.generation,
            "a node that was removed"
        );
        node.index
    }

    /// The node's parent (`None` for the root).
    pub fn parent(&self, node: Node) -> Option<Node> {
        let p = self.nodes[self.index(node) as usize].parent()?;
        Some(Node {
            index: p,
            generation: self.nodes[p as usize].generation,
        })
    }

    /// The node's stacked children, in order.
    pub fn children(&self, node: Node) -> impl Iterator<Item = Node> + '_ {
        self.nodes[self.index(node) as usize]
            .children
            .iter()
            .map(|&c| Node {
                index: c,
                generation: self.nodes[c as usize].generation,
            })
    }

    /// Where the last frame placed `node`, in its parent's box (before the
    /// parent's scroll), and its size.
    pub fn rect(&self, node: Node) -> ([f32; 2], [f32; 2]) {
        let d = &self.nodes[self.index(node) as usize];
        (d.pos, d.size)
    }

    // ---- Setters ----

    /// Set property `prop` of `node` to `value`, through a signal the node
    /// owns (made the first time), so that the change is taken like any
    /// other: `has` says whether the node has the property.
    fn set_prop<T: Clone + PartialEq + 'static>(
        &mut self,
        node: Node,
        prop: u8,
        value: T,
        has: fn(&NodeData) -> bool,
    ) {
        let n = self.index(node);
        assert!(has(&self.nodes[n as usize]), "a node without the property");
        if let Some(k) = self.binding(n, prop) {
            let setter = self.bindings[k]
                .setter
                .expect("a property bound to a memo is changed through what the memo reads");
            self.graph.set(Signal::<T>::from_id(setter), value);
            return;
        }
        let owner = self.nodes[n as usize].owner;
        let signal = self.graph.signal_in(owner, value);
        let memo = self.graph.memo_in(owner, move |r| r.get(signal));
        let d = &mut self.nodes[n as usize];
        d.bindings = Some(self.bindings.insert(Binding {
            prop,
            memo: memo.id(),
            setter: Some(signal.id()),
            version: 0,
            next: d.bindings,
        }));
        // A new memo has not run, so this reports it at once.
        self.graph.observe(memo, key(n, prop));
        self.pending = true;
    }

    /// Set `node`'s width.
    pub fn set_width(&mut self, node: Node, width: Size) {
        self.set_prop(node, WIDTH, width, |_| true);
    }

    /// Set `node`'s height.
    pub fn set_height(&mut self, node: Node, height: Size) {
        self.set_prop(node, HEIGHT, height, |_| true);
    }

    /// Set how `node` stacks its children.
    pub fn set_stack(&mut self, node: Node, stack: Stack) {
        self.set_prop(node, STACK, stack, |_| true);
    }

    /// Set `node`'s fill colour.
    pub fn set_fill(&mut self, node: Node, fill: [f32; 4]) {
        self.set_prop(node, FILL, fill, |_| true);
    }

    /// Set `node`'s border, corners and shadow.
    pub fn set_look(&mut self, node: Node, look: Look) {
        self.set_prop(node, LOOK, look, |_| true);
    }

    /// Set an absolute node's offset. Panics on a stacked node.
    pub fn set_offset(&mut self, node: Node, offset: [f32; 2]) {
        self.set_prop(node, OFFSET, offset, |d| d.offset.is_some());
    }

    /// Set a text node's text. Panics on a node without text.
    pub fn set_text(&mut self, node: Node, text: Text) {
        self.set_prop(node, TEXT, text, |d| d.text.is_some());
    }

    /// Set a text node's colour. Panics on a node without text.
    pub fn set_text_color(&mut self, node: Node, color: [f32; 4]) {
        self.set_prop(node, TEXT_COLOR, color, |d| d.text.is_some());
    }

    /// The signal holding a scrolling node's offset, to read (a scroll bar)
    /// or write. Panics on a node that does not scroll.
    pub fn scroll_offset(&self, node: Node) -> Signal<f32> {
        self.nodes[self.index(node) as usize]
            .scroll
            .as_ref()
            .expect("a scrolling node")
            .signal
    }

    /// Scroll `node` to `offset`, clamped to its content as last laid out
    /// (and again after the next layout).
    pub fn scroll_to(&mut self, node: Node, offset: f32) {
        let d = &self.nodes[self.index(node) as usize];
        let s = d.scroll.as_ref().expect("a scrolling node");
        let max = (s.extent - d.size[d.stack.direction.main()]).max(0.0);
        let offset = if offset.is_nan() {
            0.0
        } else {
            offset.clamp(0.0, max)
        };
        let signal = s.signal;
        self.graph.set(signal, offset);
    }

    /// Scroll `node` by `delta`.
    pub fn scroll_by(&mut self, node: Node, delta: f32) {
        let signal = self.scroll_offset(node);
        let now = self.graph.get(signal);
        self.scroll_to(node, now + delta);
    }

    // ---- Dirt ----

    /// `n` must be laid out again, and so must each node above it whose
    /// size may follow what changed (with `own_size`, `n`'s parent at
    /// least, since `n`'s own size changed). Above those, the path from the
    /// root is marked so the layout reaches them.
    pub(crate) fn mark_layout(&mut self, n: u32, own_size: bool) {
        self.pending = true;
        let mut x = n;
        self.nodes[x as usize].layout_self = true;
        let mut up = own_size || crate::layout::follows_content(&self.nodes[x as usize]);
        while up {
            match self.nodes[x as usize].parent() {
                Some(p) => {
                    x = p;
                    self.nodes[x as usize].layout_self = true;
                    up = crate::layout::follows_content(&self.nodes[x as usize]);
                }
                None => break,
            }
        }
        if let Some(p) = self.nodes[x as usize].parent() {
            self.mark_below(p);
        }
    }

    /// Something below `n` must be laid out again: the path from the root
    /// to `n` is marked, up to the first node already marked.
    pub(crate) fn mark_below(&mut self, n: u32) {
        self.pending = true;
        let mut p = Some(n);
        while let Some(q) = p {
            let d = &mut self.nodes[q as usize];
            if d.layout_below {
                break;
            }
            d.layout_below = true;
            p = d.parent();
        }
    }

    /// Whether `n` keeps a list of what it and its subtree draw: the root
    /// does, and so does a node that clips (or scrolls). Every other node is
    /// drawn into its holder's list (see [`holder`](Self::holder)). It is
    /// fixed for a node's life, so a node's holder is too (not whether it has
    /// children: layout takes them out of the node while it places them).
    pub(crate) fn keeps_list(&self, n: u32) -> bool {
        n == self.root || self.nodes[n as usize].clip
    }

    /// The node whose list `n` is drawn into: the nearest above it that
    /// keeps one. `None` for the root.
    pub(crate) fn holder(&self, n: u32) -> Option<u32> {
        let mut p = self.nodes[n as usize].parent();
        while let Some(x) = p {
            if self.keeps_list(x) {
                return Some(x);
            }
            p = self.nodes[x as usize].parent();
        }
        None
    }

    /// What `n` draws itself is out of date (or, for a node that keeps a
    /// list, anything in it): it is queued in its holder's `dirty_kids`, and
    /// that holder in its own, and so on up, each list above having to
    /// change its records too.
    pub(crate) fn mark_paint(&mut self, n: u32) {
        self.pending = true;
        self.nodes[n as usize].stale = true;
        let mut x = n;
        loop {
            let d = &mut self.nodes[x as usize];
            if d.mark_epoch == self.epoch {
                break;
            }
            d.mark_epoch = self.epoch;
            let Some(h) = self.holder(x) else {
                break;
            };
            let d = &mut self.nodes[x as usize];
            if !d.queued {
                d.queued = true;
                let generation = d.generation;
                self.nodes[h as usize].dirty_kids.push((x, generation));
            }
            x = h;
        }
    }

    /// What is drawn below `n` changed in number or place: the list it is
    /// drawn in (its own, if it keeps one) must be recorded again.
    pub(crate) fn mark_records(&mut self, n: u32) {
        let k = if self.keeps_list(n) {
            n
        } else {
            self.holder(n).expect("the root keeps a list")
        };
        self.mark_paint(k);
    }

    /// Clear `queued` on the nodes of `kids` still the ones queued.
    pub(crate) fn unqueue(&mut self, kids: &[(u32, u32)]) {
        for &(c, generation) in kids {
            let d = &mut self.nodes[c as usize];
            if d.live && d.generation == generation {
                d.queued = false;
            }
        }
    }

    /// `n`'s paint bounds may have changed: so may those of each node above
    /// it up to the first that clips, and each of those must look at its
    /// children's again.
    pub(crate) fn mark_grow(&mut self, n: u32) {
        self.nodes[n as usize].grow_stale = true;
        let mut p = self.nodes[n as usize].parent();
        while let Some(x) = p {
            let d = &mut self.nodes[x as usize];
            d.child_grow_stale = true;
            if d.clip || d.grow_stale {
                break;
            }
            d.grow_stale = true;
            p = d.parent();
        }
    }

    /// Take property `prop` of `n` from its memo, if the memo moved on;
    /// says whether the value changed.
    fn refresh<T: Clone + PartialEq + 'static>(
        &mut self,
        n: u32,
        prop: u8,
        slot: fn(&mut NodeData) -> &mut T,
    ) -> bool {
        let Some(k) = self.binding(n, prop) else {
            return false;
        };
        let b = &mut self.bindings[k];
        let memo = Memo::<T>::from_id(b.memo);
        let version = self.graph.version(memo);
        if version == b.version {
            return false;
        }
        b.version = version;
        let value = self.graph.get(memo);
        let s = slot(&mut self.nodes[n as usize]);
        if value == *s {
            return false;
        }
        *s = value;
        true
    }

    /// Take the graph's reports and mark what the properties that changed
    /// affect.
    fn take_notifications(&mut self) {
        let mut keys = std::mem::take(&mut self.keys);
        self.graph.take_notified(&mut keys);
        self.stats.notified += keys.len() as u32;
        for &k in &keys {
            let n = (k >> 4) as u32;
            if !self.nodes[n as usize].live {
                continue;
            }
            match (k & 15) as u8 {
                WIDTH => {
                    if self.refresh(n, WIDTH, |d| &mut d.width) {
                        self.mark_layout(n, true);
                    }
                }
                HEIGHT => {
                    if self.refresh(n, HEIGHT, |d| &mut d.height) {
                        self.mark_layout(n, true);
                    }
                }
                STACK => {
                    if self.refresh(n, STACK, |d| &mut d.stack) {
                        self.mark_layout(n, false);
                        self.mark_records(n);
                        self.nodes[n as usize].child_grow_stale = true;
                    }
                }
                FILL => {
                    if self.refresh(n, FILL, |d| &mut d.fill) {
                        self.mark_paint(n);
                    }
                }
                LOOK => {
                    if self.refresh(n, LOOK, |d| &mut d.look) {
                        self.mark_paint(n);
                        crate::layout::update_own_grow(self, n);
                    }
                }
                OFFSET => {
                    if self.refresh(n, OFFSET, |d| d.offset.as_mut().expect("an absolute node")) {
                        let parent = self.nodes[n as usize].parent;
                        self.mark_layout(parent, false);
                    }
                }
                TEXT => {
                    if self.refresh(n, TEXT, |d| &mut d.text.as_mut().expect("a text node").text) {
                        self.mark_layout(n, false);
                        self.mark_paint(n);
                    }
                }
                TEXT_COLOR => {
                    if self.refresh(n, TEXT_COLOR, |d| {
                        &mut d.text.as_mut().expect("a text node").color
                    }) {
                        self.mark_paint(n);
                    }
                }
                CUSTOM => {
                    let c = self.nodes[n as usize]
                        .custom
                        .as_mut()
                        .expect("a painted node");
                    let version = c.paint.version(&mut self.graph);
                    if version != c.version {
                        c.version = version;
                        self.mark_paint(n);
                    }
                }
                SCROLL => {
                    let s = self.nodes[n as usize]
                        .scroll
                        .as_mut()
                        .expect("a scrolling node");
                    let version = self.graph.version(s.memo);
                    if version != s.version {
                        s.version = version;
                        s.requested = self.graph.get(s.memo);
                        let defers = s.defers;
                        // What comes near the view must be laid out.
                        if crate::layout::apply_scroll(self, n) && defers {
                            self.mark_below(n);
                        }
                    }
                }
                other => unreachable!("no property {other}"),
            }
        }
        keys.clear();
        self.keys = keys;
    }

    // ---- The frame ----

    /// Whether a frame at `now` would differ from the last: a property may
    /// have changed, a node was added or removed, a repaint deadline in view
    /// has come, or a volatile painter is in the tree.
    pub fn needs_frame(&self, now: f64) -> bool {
        self.pending
            || self.context.is_none()
            || self.graph.stats().notified > 0
            || self.nodes[self.root as usize].volatile_below > 0
            || self.nodes[self.root as usize].deadline <= now
    }

    /// The earliest repaint deadline among what is in view, in the frame's
    /// clock.
    pub fn next_repaint(&self) -> Option<f64> {
        let at = self.nodes[self.root as usize].deadline;
        at.is_finite().then_some(at)
    }

    /// Bring the tree up to date for `context` and return the frame's list:
    /// the root's, which is the last frame's untouched when nothing changed.
    pub fn frame(&mut self, context: FrameContext) -> &DrawList {
        let runs = self.graph.stats().runs;
        self.stats = FrameStats::default();
        self.now = context.now;
        if let Some(old) = self.context
            && (old.fonts != context.fonts || old.scale != context.scale)
        {
            for n in 0..self.nodes.len() as u32 {
                let d = &self.nodes[n as usize];
                if d.live && d.text.is_some() {
                    self.mark_layout(n, false);
                    self.mark_paint(n);
                }
            }
        }
        self.context = Some(context);
        let viewport = context
            .viewport
            .map(|v| Avail::Exact(crate::layout::snap(v).max(0.0)));
        // Laying out can write signals (a scroll offset clamped to content
        // that shrank, the hover of what is now under the pointer), which
        // can change what is laid out: settle, a few rounds at most.
        for _ in 0..4 {
            self.take_notifications();
            self.layout(self.root, viewport);
            if self.pointer.at().is_some() {
                self.refresh_hover();
            }
            if self.graph.stats().notified == 0 {
                break;
            }
        }
        // What this frame patches in the root's list is told to the renderer,
        // relative to the list as the last frame left it.
        if let Some(kept) = self.nodes[self.root as usize].kept.as_deref_mut() {
            kept.list.begin_edits();
        }
        self.update(self.root);
        self.spans.clear();
        // Only a recording changes what is placed.
        if self.stats.recorded > 0 {
            self.drop_unplaced();
        }
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.pending = self.graph.stats().notified > 0;
        self.stats.memo_runs = self.graph.stats().runs - runs;
        &self.nodes[self.root as usize]
            .kept
            .as_deref()
            .expect("the root keeps a list")
            .list
    }

    /// The last frame's list.
    pub fn list(&self) -> Option<&DrawList> {
        self.nodes[self.root as usize]
            .kept
            .as_deref()
            .map(|k| &k.list)
    }

    /// What the last frame did.
    pub fn frame_stats(&self) -> FrameStats {
        self.stats
    }

    /// What the tree holds.
    pub fn tree_stats(&self) -> TreeStats {
        let mut stats = TreeStats {
            nodes: self.live,
            node_bytes: (self.nodes.capacity() * std::mem::size_of::<NodeData>()
                + self.bindings.capacity() * std::mem::size_of::<Binding>())
                as u64,
            texts_prepared: self.texts.count,
            text_bytes: self.texts.bytes,
            texts_dropped: self.texts.dropped,
            ..TreeStats::default()
        };
        for &n in &self.keepers {
            if let Some(kept) = &self.nodes[n as usize].kept {
                stats.lists += 1;
                stats.records += kept.list.prim_counts().total() as u64;
                stats.list_bytes += (std::mem::size_of::<Kept>()
                    + kept.list.heap_bytes()
                    + kept.log.heap_bytes()) as u64;
            }
        }
        stats
    }
}
