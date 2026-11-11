//! Signals and memos: the reactive graph a retained tree's properties live in.
//!
//! A [`Signal`] holds a value that is set from outside. A [`Memo`] computes
//! one from signals and other memos, through a [`Reader`] that records what it
//! read, again on every run, so a memo that stops reading something (a branch
//! not taken) stops depending on it.
//!
//! Propagation is mark, then pull. A write that changes a signal's value marks
//! the memos that read it dirty and everything downstream of them "check", and
//! runs nothing. A memo is brought up to date when it is read: a dirty one
//! runs; a check one first brings its sources up to date, in the order it last
//! read them, and runs only if one of them changed, else it is clean without
//! running. So every memo runs at most once per batch of writes, never sees a
//! stale input, and stops propagation where its value did not change.
//!
//! Observed memos (the properties a tree draws from) report, by a key of the
//! caller's choosing, when they may have changed; the caller reads them and
//! compares their [`version`](Graph::version).
//!
//! Every signal and memo belongs to an [`Owner`]. Disposing of an owner
//! disposes of everything it and the owners below it own: their edges both
//! ways, their closures and values, and any notification still queued for
//! them. A handle to something disposed panics when it is used.
//!
//! Edges are kept on both ends with each end's index into the other's list,
//! so adding or removing one costs O(1), and disposing of n nodes costs O(n
//! plus their edges), however many other readers a shared signal has.

use std::any::Any;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_GRAPH: AtomicU32 = AtomicU32::new(1);

/// A run that has read more distinct sources than this keeps a set of them,
/// rather than scanning its list, to read each only once.
const SCAN_SOURCES: usize = 32;

/// Which node of which graph a handle names, and which use of its slot.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId {
    graph: u32,
    index: u32,
    generation: u32,
}

/// A value set from outside the graph. A `Copy` handle; the value lives in
/// the [`Graph`].
pub struct Signal<T> {
    id: NodeId,
    _type: PhantomData<fn() -> T>,
}

/// A value computed from signals and other memos. A `Copy` handle; the value
/// and the closure live in the [`Graph`].
pub struct Memo<T> {
    id: NodeId,
    _type: PhantomData<fn() -> T>,
}

macro_rules! handle_impls {
    ($name:ident) => {
        impl<T> $name<T> {
            /// The handle naming `id`, which names one of type `T`: for a
            /// handle kept untyped (reading it as another type panics).
            pub(crate) fn from_id(id: NodeId) -> Self {
                Self {
                    id,
                    _type: PhantomData,
                }
            }
        }
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T> Copy for $name<T> {}
        impl<T> PartialEq for $name<T> {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        impl<T> Eq for $name<T> {}
        impl<T> std::fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.id)
            }
        }
        impl<T> Readable<T> for $name<T> {
            fn id(&self) -> NodeId {
                self.id
            }
        }
    };
}

/// A signal or a memo: something a [`Reader`] or the [`Graph`] can read.
pub trait Readable<T>: Copy {
    /// The node it names.
    fn id(&self) -> NodeId;
}

handle_impls!(Signal);
handle_impls!(Memo);

/// Something signals and memos belong to; disposing of it disposes of them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Owner {
    graph: u32,
    index: u32,
    generation: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum State {
    Clean,
    Check,
    Dirty,
}

/// One end of an edge: the node at the other end, and where this edge sits
/// in that node's list.
#[derive(Clone, Copy, Debug)]
struct Edge {
    node: u32,
    back: u32,
}

/// A memo's closure, typed away: runs it, stores the value if it differs,
/// and says whether it did.
type Compute = Box<dyn FnMut(&mut Reader<'_>, &mut Option<Box<dyn Any>>) -> bool>;

struct Slot {
    generation: u32,
    live: bool,
    memo: bool,
    type_name: &'static str,
    value: Option<Box<dyn Any>>,
    compute: Option<Compute>,
    state: State,
    running: bool,
    disposing: bool,
    /// Moves on each time the value changes.
    version: u64,
    /// What this memo read on its last run, in the order it first read them.
    sources: Vec<Edge>,
    /// The memos that read this.
    readers: Vec<Edge>,
    observer: Option<u64>,
}

struct OwnerSlot {
    generation: u32,
    live: bool,
    parent: Option<u32>,
    /// Where this sits in its parent's list of children.
    parent_back: u32,
    children: Vec<u32>,
    nodes: Vec<u32>,
}

/// Counts of what the graph holds and what it has done, for tests and
/// measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphStats {
    /// Live signals and memos.
    pub nodes: usize,
    /// Live owners, the root included.
    pub owners: usize,
    /// Edges from a memo to what it read.
    pub edges: usize,
    /// Memo closures held.
    pub closures: usize,
    /// Memo runs since the graph was made.
    pub runs: u64,
    /// Notifications queued and not yet taken.
    pub notified: usize,
}

/// The reactive graph: every signal and memo, their edges and owners.
pub struct Graph {
    id: u32,
    slots: Vec<Slot>,
    free: Vec<u32>,
    owners: Vec<OwnerSlot>,
    free_owners: Vec<u32>,
    notified: Vec<u64>,
    live_nodes: usize,
    live_owners: usize,
    edges: usize,
    closures: usize,
    runs: u64,
    stale_stack: Vec<(u32, State)>,
    source_pool: Vec<Vec<u32>>,
    dispose_owners: Vec<u32>,
    dispose_nodes: Vec<u32>,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    /// An empty graph with its root owner.
    pub fn new() -> Self {
        Self {
            id: NEXT_GRAPH.fetch_add(1, Ordering::Relaxed),
            slots: Vec::new(),
            free: Vec::new(),
            owners: vec![OwnerSlot {
                generation: 0,
                live: true,
                parent: None,
                parent_back: 0,
                children: Vec::new(),
                nodes: Vec::new(),
            }],
            free_owners: Vec::new(),
            notified: Vec::new(),
            live_nodes: 0,
            live_owners: 1,
            edges: 0,
            closures: 0,
            runs: 0,
            stale_stack: Vec::new(),
            source_pool: Vec::new(),
            dispose_owners: Vec::new(),
            dispose_nodes: Vec::new(),
        }
    }

    /// The owner that lives as long as the graph.
    pub fn root(&self) -> Owner {
        Owner {
            graph: self.id,
            index: 0,
            generation: 0,
        }
    }

    /// A new owner below `parent`.
    pub fn owner(&mut self, parent: Owner) -> Owner {
        let parent = self.owner_index(parent);
        let parent_back = self.owners[parent as usize].children.len() as u32;
        let index = match self.free_owners.pop() {
            Some(index) => {
                let slot = &mut self.owners[index as usize];
                slot.live = true;
                slot.parent = Some(parent);
                slot.parent_back = parent_back;
                index
            }
            None => {
                self.owners.push(OwnerSlot {
                    generation: 0,
                    live: true,
                    parent: Some(parent),
                    parent_back,
                    children: Vec::new(),
                    nodes: Vec::new(),
                });
                self.owners.len() as u32 - 1
            }
        };
        self.owners[parent as usize].children.push(index);
        self.live_owners += 1;
        Owner {
            graph: self.id,
            index,
            generation: self.owners[index as usize].generation,
        }
    }

    /// A signal owned by the root.
    pub fn signal<T: PartialEq + 'static>(&mut self, value: T) -> Signal<T> {
        self.signal_in(self.root(), value)
    }

    /// A signal owned by `owner`.
    pub fn signal_in<T: PartialEq + 'static>(&mut self, owner: Owner, value: T) -> Signal<T> {
        let id = self.alloc(
            owner,
            false,
            std::any::type_name::<T>(),
            Some(Box::new(value)),
            None,
        );
        Signal {
            id,
            _type: PhantomData,
        }
    }

    /// A memo owned by the root.
    pub fn memo<T, F>(&mut self, compute: F) -> Memo<T>
    where
        T: PartialEq + 'static,
        F: FnMut(&mut Reader<'_>) -> T + 'static,
    {
        self.memo_in(self.root(), compute)
    }

    /// A memo owned by `owner`. It first runs when it is first read.
    pub fn memo_in<T, F>(&mut self, owner: Owner, mut compute: F) -> Memo<T>
    where
        T: PartialEq + 'static,
        F: FnMut(&mut Reader<'_>) -> T + 'static,
    {
        let compute: Compute = Box::new(move |reader, stored| {
            let new = compute(reader);
            match stored {
                Some(old) => {
                    let old = old
                        .downcast_mut::<T>()
                        .expect("a memo's value keeps its type");
                    if *old == new {
                        false
                    } else {
                        *old = new;
                        true
                    }
                }
                None => {
                    *stored = Some(Box::new(new));
                    true
                }
            }
        });
        let id = self.alloc(owner, true, std::any::type_name::<T>(), None, Some(compute));
        Memo {
            id,
            _type: PhantomData,
        }
    }

    /// The value of `source`, brought up to date. Not tracked: this is a
    /// read from outside the graph.
    pub fn get<T: Clone + 'static>(&mut self, source: impl Readable<T>) -> T {
        self.with(source, T::clone)
    }

    /// `f` of the value of `source`, brought up to date, without cloning it.
    pub fn with<T: 'static, R>(&mut self, source: impl Readable<T>, f: impl FnOnce(&T) -> R) -> R {
        let index = self.index(source.id());
        self.fresh(index);
        f(self.value::<T>(index))
    }

    /// How many times the value of `source` has changed, brought up to date:
    /// a reader that keeps the last version it saw knows whether to look again.
    pub fn version<T>(&mut self, source: impl Readable<T>) -> u64 {
        let index = self.index(source.id());
        self.fresh(index);
        self.slots[index as usize].version
    }

    /// Set `signal`'s value. Nothing happens if it equals the current one;
    /// otherwise what reads it is marked, and nothing runs. Says whether the
    /// value changed.
    pub fn set<T: PartialEq + 'static>(&mut self, signal: Signal<T>, value: T) -> bool {
        let index = self.index(signal.id);
        let slot = &mut self.slots[index as usize];
        let current = slot
            .value
            .as_mut()
            .and_then(|v| v.downcast_mut::<T>())
            .expect("a signal holds a value of its type");
        if *current == value {
            return false;
        }
        *current = value;
        slot.version += 1;
        for k in 0..self.slots[index as usize].readers.len() {
            let reader = self.slots[index as usize].readers[k].node;
            self.stale(reader, State::Dirty);
        }
        true
    }

    /// Change `signal`'s value in place, as [`set`](Self::set) with the
    /// changed copy.
    pub fn update<T: PartialEq + Clone + 'static>(
        &mut self,
        signal: Signal<T>,
        change: impl FnOnce(&mut T),
    ) -> bool {
        let mut value = self.get(signal);
        change(&mut value);
        self.set(signal, value)
    }

    /// Report `memo` under `key` whenever it may have changed (and now, if
    /// it has never run): see [`take_notified`](Self::take_notified). A
    /// report means "read it again"; its [`version`](Self::version) says
    /// whether it did change.
    pub fn observe<T>(&mut self, memo: Memo<T>, key: u64) {
        let index = self.index(memo.id);
        let slot = &mut self.slots[index as usize];
        slot.observer = Some(key);
        if slot.state != State::Clean {
            self.notified.push(key);
        }
    }

    /// Move the keys of the observed memos reported since the last call into
    /// `into`: each once per time it went from up to date to marked.
    pub fn take_notified(&mut self, into: &mut Vec<u64>) {
        into.append(&mut self.notified);
    }

    /// Dispose of `owner`, the owners below it and every signal and memo they
    /// own. Panics if a memo that is not being disposed of still reads one
    /// that is.
    pub fn dispose(&mut self, owner: Owner) {
        let root = self.owner_index(owner);
        assert!(root != 0, "the root owner lives as long as the graph");
        let mut owners = std::mem::take(&mut self.dispose_owners);
        let mut nodes = std::mem::take(&mut self.dispose_nodes);
        owners.push(root);
        let mut k = 0;
        while k < owners.len() {
            let o = owners[k] as usize;
            owners.extend_from_slice(&self.owners[o].children);
            nodes.extend_from_slice(&self.owners[o].nodes);
            k += 1;
        }
        for &n in &nodes {
            self.slots[n as usize].disposing = true;
        }
        for &n in &nodes {
            let slot = &self.slots[n as usize];
            if let Some(reader) = slot
                .readers
                .iter()
                .find(|e| !self.slots[e.node as usize].disposing)
            {
                panic!(
                    "disposing of a {} that a memo of {} outside the owner still reads",
                    slot.type_name, self.slots[reader.node as usize].type_name
                );
            }
        }
        for &n in &nodes {
            while !self.slots[n as usize].sources.is_empty() {
                self.pop_source(n);
            }
        }
        let mut keys = HashSet::new();
        for &n in &nodes {
            let slot = &mut self.slots[n as usize];
            debug_assert!(slot.readers.is_empty(), "every reader was disposed of too");
            if let Some(key) = slot.observer.take() {
                keys.insert(key);
            }
            if slot.compute.take().is_some() {
                self.closures -= 1;
            }
            slot.value = None;
            slot.live = false;
            slot.disposing = false;
            slot.generation = slot.generation.wrapping_add(1);
            self.free.push(n);
        }
        self.live_nodes -= nodes.len();
        if !keys.is_empty() {
            self.notified.retain(|key| !keys.contains(key));
        }

        // Detach the top owner from its parent, then free them all.
        let top = &self.owners[root as usize];
        let parent = top.parent.expect("only the root has no parent");
        let back = top.parent_back;
        let siblings = &mut self.owners[parent as usize].children;
        siblings.swap_remove(back as usize);
        if let Some(&moved) = siblings.get(back as usize) {
            self.owners[moved as usize].parent_back = back;
        }
        for &o in &owners {
            let slot = &mut self.owners[o as usize];
            slot.live = false;
            slot.parent = None;
            slot.children.clear();
            slot.nodes.clear();
            slot.generation = slot.generation.wrapping_add(1);
            self.free_owners.push(o);
        }
        self.live_owners -= owners.len();
        owners.clear();
        nodes.clear();
        self.dispose_owners = owners;
        self.dispose_nodes = nodes;
    }

    /// What the graph holds and has done.
    pub fn stats(&self) -> GraphStats {
        GraphStats {
            nodes: self.live_nodes,
            owners: self.live_owners,
            edges: self.edges,
            closures: self.closures,
            runs: self.runs,
            notified: self.notified.len(),
        }
    }

    fn owner_index(&self, owner: Owner) -> u32 {
        assert_eq!(owner.graph, self.id, "an owner from another graph");
        let slot = &self.owners[owner.index as usize];
        assert!(
            slot.live && slot.generation == owner.generation,
            "an owner that was disposed of"
        );
        owner.index
    }

    fn index(&self, id: NodeId) -> u32 {
        assert_eq!(id.graph, self.id, "a signal or memo from another graph");
        let slot = &self.slots[id.index as usize];
        assert!(
            slot.live && slot.generation == id.generation,
            "a {} that was disposed of",
            slot.type_name
        );
        id.index
    }

    fn alloc(
        &mut self,
        owner: Owner,
        memo: bool,
        type_name: &'static str,
        value: Option<Box<dyn Any>>,
        compute: Option<Compute>,
    ) -> NodeId {
        let owner = self.owner_index(owner);
        if compute.is_some() {
            self.closures += 1;
        }
        let state = if memo { State::Dirty } else { State::Clean };
        let index = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.live = true;
                slot.memo = memo;
                slot.type_name = type_name;
                slot.value = value;
                slot.compute = compute;
                slot.state = state;
                slot.version = 0;
                index
            }
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    live: true,
                    memo,
                    type_name,
                    value,
                    compute,
                    state,
                    running: false,
                    disposing: false,
                    version: 0,
                    sources: Vec::new(),
                    readers: Vec::new(),
                    observer: None,
                });
                self.slots.len() as u32 - 1
            }
        };
        self.owners[owner as usize].nodes.push(index);
        self.live_nodes += 1;
        NodeId {
            graph: self.id,
            index,
            generation: self.slots[index as usize].generation,
        }
    }

    fn value<T: 'static>(&self, index: u32) -> &T {
        self.slots[index as usize]
            .value
            .as_ref()
            .and_then(|v| v.downcast_ref::<T>())
            .expect("an up-to-date node holds a value of its type")
    }

    /// Bring `index` up to date, if it is a memo.
    fn fresh(&mut self, index: u32) {
        let slot = &self.slots[index as usize];
        if slot.running {
            panic!(
                "a memo of {} read itself: a cycle through its sources",
                slot.type_name
            );
        }
        if slot.memo {
            self.pull(index);
        }
    }

    /// Bring the memo `index` up to date: a check one asks its sources in
    /// the order it read them until one changed, a dirty one runs.
    fn pull(&mut self, index: u32) {
        let i = index as usize;
        if self.slots[i].state == State::Check {
            let mut k = 0;
            while k < self.slots[i].sources.len() {
                let source = self.slots[i].sources[k].node;
                if self.slots[source as usize].memo {
                    self.pull(source);
                }
                if self.slots[i].state == State::Dirty {
                    break;
                }
                k += 1;
            }
        }
        if self.slots[i].state == State::Dirty {
            self.run(index);
        }
        self.slots[i].state = State::Clean;
    }

    fn run(&mut self, index: u32) {
        let i = index as usize;
        let mut compute = self.slots[i]
            .compute
            .take()
            .expect("a memo that is not running holds its closure");
        let mut value = self.slots[i].value.take();
        self.slots[i].running = true;
        let sources = self.source_pool.pop().unwrap_or_default();
        let mut reader = Reader {
            graph: self,
            sources,
            seen: None,
        };
        let changed = compute(&mut reader, &mut value);
        let Reader { mut sources, .. } = reader;
        self.runs += 1;

        let slot = &mut self.slots[i];
        slot.running = false;
        slot.compute = Some(compute);
        slot.value = value;
        if changed {
            slot.version += 1;
        }
        let same = slot.sources.len() == sources.len()
            && slot.sources.iter().zip(&sources).all(|(e, &s)| e.node == s);
        if !same {
            while !self.slots[i].sources.is_empty() {
                self.pop_source(index);
            }
            for &source in &sources {
                self.add_edge(index, source);
            }
        }
        sources.clear();
        self.source_pool.push(sources);
        if changed {
            for k in 0..self.slots[i].readers.len() {
                let reader = self.slots[i].readers[k].node;
                self.stale(reader, State::Dirty);
            }
        }
    }

    /// Mark `index` at least `state`, and, if it was up to date, everything
    /// downstream of it "check", reporting the observed ones.
    fn stale(&mut self, index: u32, state: State) {
        let mut stack = std::mem::take(&mut self.stale_stack);
        stack.push((index, state));
        while let Some((n, state)) = stack.pop() {
            let slot = &mut self.slots[n as usize];
            if slot.state >= state {
                continue;
            }
            let was_clean = slot.state == State::Clean;
            slot.state = state;
            if was_clean {
                if let Some(key) = slot.observer {
                    self.notified.push(key);
                }
                stack.extend(slot.readers.iter().map(|e| (e.node, State::Check)));
            }
        }
        self.stale_stack = stack;
    }

    fn add_edge(&mut self, reader: u32, source: u32) {
        let in_reader = self.slots[reader as usize].sources.len() as u32;
        let in_source = self.slots[source as usize].readers.len() as u32;
        self.slots[source as usize].readers.push(Edge {
            node: reader,
            back: in_reader,
        });
        self.slots[reader as usize].sources.push(Edge {
            node: source,
            back: in_source,
        });
        self.edges += 1;
    }

    /// Remove `reader`'s last source edge, from both ends.
    fn pop_source(&mut self, reader: u32) {
        let edge = self.slots[reader as usize]
            .sources
            .pop()
            .expect("a source to remove");
        let readers = &mut self.slots[edge.node as usize].readers;
        readers.swap_remove(edge.back as usize);
        if let Some(&moved) = readers.get(edge.back as usize) {
            self.slots[moved.node as usize].sources[moved.back as usize].back = edge.back;
        }
        self.edges -= 1;
    }
}

/// What a memo reads through: each read is recorded as one of its sources.
pub struct Reader<'g> {
    graph: &'g mut Graph,
    sources: Vec<u32>,
    seen: Option<HashSet<u32>>,
}

impl Reader<'_> {
    /// The value of `source`, recorded as read.
    pub fn get<T: Clone + 'static>(&mut self, source: impl Readable<T>) -> T {
        self.with(source, T::clone)
    }

    /// `f` of the value of `source`, recorded as read.
    pub fn with<T: 'static, R>(&mut self, source: impl Readable<T>, f: impl FnOnce(&T) -> R) -> R {
        let index = self.graph.index(source.id());
        self.graph.fresh(index);
        self.track(index);
        f(self.graph.value::<T>(index))
    }

    /// The value of `source`, not recorded: a change to it does not run
    /// this memo again.
    pub fn untracked<T: Clone + 'static>(&mut self, source: impl Readable<T>) -> T {
        self.graph.get(source)
    }

    fn track(&mut self, index: u32) {
        if let Some(seen) = &mut self.seen {
            if seen.insert(index) {
                self.sources.push(index);
            }
            return;
        }
        if self.sources.contains(&index) {
            return;
        }
        self.sources.push(index);
        if self.sources.len() > SCAN_SOURCES {
            self.seen = Some(self.sources.iter().copied().collect());
        }
    }
}

#[cfg(test)]
mod tests;
