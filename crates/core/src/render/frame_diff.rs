//! What changed since the last frame (`docs/design/frame-reuse.md`, Phase 1).
//!
//! A [`FrameSnapshot`] keeps the records of the last list drawn, and
//! [`compare`](FrameSnapshot::compare) says, per record, how the next list
//! differs: not "same or changed", because a colour animation changes bytes
//! and nothing else, and what it leaves alone (boxes, keys, orders, batches)
//! is most of a frame's cost.
//!
//! Nothing here reuses anything yet. The answers are checked against the
//! frame oracle (`frame_oracle.rs`): a record called [`Change::Same`] or
//! [`Change::Look`] must really leave the plan it feeds as it was.

use super::draw_box::{same_analytic_box_inputs, same_circle_box_inputs, same_vertex_box_inputs};
use crate::layout::Rect;
use crate::text::{PixelGrid, TextBlock, TextOutline};
use crate::widgets::{
    AnalyticInstance, CircleInstance, DrawList, IconDraw, ListVersion, NineSliceDraw, PaintCmd,
    RecordCounts, Vertex,
};

/// How a record differs from the same record last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Change {
    /// Bit for bit as it was.
    #[default]
    Same,
    /// It draws differently but takes the same place: the same batch key, the
    /// same box, the same number of instances. Its order among the others
    /// cannot move, so a plan made without it changing is still the plan; only
    /// the bytes uploaded for it differ. A colour, a tint.
    Look,
    /// Anything else: it may move, resize, change family or texture, or draw
    /// a different number of instances.
    Place,
}

/// Everything outside the list that its plan and instances depend on
/// (`docs/design/frame-reuse.md`, "The kept frame, and the context key").
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContextKey {
    /// Scale, view origin, glyph snap and hinting, and the logical size glyphs
    /// are culled against.
    pub(crate) grid: PixelGrid,
    /// The target's size in physical px.
    pub(crate) viewport: (u32, u32),
    /// The font system's generation: shaped text is stale once it moves.
    pub(crate) fonts: u64,
    /// What sprites, images and nine-slices resolve to.
    pub(crate) resolution: u64,
}

/// What [`compare`](FrameSnapshot::compare) found, in all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameDiff {
    /// There was a frame to compare with.
    pub(crate) compared: bool,
    pub(crate) context_same: bool,
    /// The paint stream, the soup and every array have the shape they had, so
    /// records can be compared one for one. Without it the counts below are 0.
    pub(crate) topology_same: bool,
    /// The comparison gave up part way, because most of what it had looked at
    /// had changed place: the counts and the per-family answers are partial.
    pub(crate) abandoned: bool,
    /// Records by how they changed.
    pub(crate) same: u32,
    pub(crate) look: u32,
    pub(crate) place: u32,
}

impl FrameDiff {
    /// The frame is the last one again, to the byte: the plan, the instances
    /// and the draws would come out as they did.
    pub(crate) fn identical(&self) -> bool {
        self.keeps_places() && self.look == 0
    }

    /// Every record is where it was: the plan (orders, keys, boxes, batches,
    /// instance counts) would come out as it did, though instance bytes may
    /// differ.
    pub(crate) fn keeps_places(&self) -> bool {
        self.answered() && self.place == 0
    }

    /// Every record was compared one for one on the same target, so
    /// [`Changes`] says for each whether it moved, and what the last frame
    /// derived from the ones that did not (their boxes) still holds.
    pub(crate) fn answered(&self) -> bool {
        self.compared && self.context_same && self.topology_same && !self.abandoned
    }
}

/// The per-family answers behind a [`FrameDiff`], valid until the next compare,
/// and not at all after an abandoned one.
#[derive(Default)]
pub(crate) struct Changes {
    pub(crate) analytic: Vec<Change>,
    pub(crate) circles: Vec<Change>,
    pub(crate) icons: Vec<Change>,
    pub(crate) nine_slices: Vec<Change>,
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icons_msdf: Vec<Change>,
    pub(crate) texts: Vec<Change>,
    /// One per `Soup` paint command, in order.
    pub(crate) soup: Vec<Change>,
    /// The soup after the last command.
    pub(crate) tail: Change,
}

/// The records of the last list drawn, and the comparison with the next.
#[derive(Default)]
pub(crate) struct FrameSnapshot {
    kept: bool,
    context: Option<ContextKey>,
    paint_cmds: Vec<PaintCmd>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    soup_committed: u32,
    /// The last [`compare`](Self::compare) answered for every record, so
    /// [`keep`](Self::keep) need copy only the ones that changed.
    complete: bool,
    /// How many frames to rest for the next time a compare gives up, and how
    /// many are left to rest (see [`resting`](Self::resting)).
    backoff: u8,
    rest: u8,
    analytic: Vec<AnalyticInstance>,
    circles: Vec<CircleInstance>,
    icons: Vec<IconDraw>,
    nine_slices: Vec<NineSliceDraw>,
    #[cfg(feature = "phosphor-icons")]
    icons_msdf: Vec<crate::widgets::IconMsdf>,
    texts: Vec<TextBlock>,
    pub(crate) changes: Changes,
    /// The list last drawn in this place, when its owner reported its edits
    /// (see [`reported`](Self::reported)): no records are kept for it.
    reported: Option<Reported>,
}

/// What is remembered of a list whose owner reports its edits, drawn in a
/// place: enough to tell whether the next list's log is relative to it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reported {
    id: u64,
    version: ListVersion,
    counts: RecordCounts,
    context: ContextKey,
}

/// Counts what has been compared and gives up when most of it moved.
#[derive(Default)]
struct Tally {
    same: u32,
    look: u32,
    place: u32,
    abandoned: bool,
}

/// Records looked at before the comparison may give up, and the share that
/// must have changed place by then (one in two): a list whose body is all
/// moving is not worth reading to the end, and one with a still header and a
/// moving body is given up inside the body.
const ABANDON_AFTER: u32 = 64;

/// The most frames a list rests for after a compare gave up. A list that
/// settles is compared again within this many frames.
const MAX_REST: u8 = 16;

impl Tally {
    /// Count a record. `false` once the comparison should stop.
    fn record(&mut self, change: Change) -> bool {
        match change {
            Change::Same => self.same += 1,
            Change::Look => self.look += 1,
            Change::Place => self.place += 1,
        }
        let seen = self.same + self.look + self.place;
        if self.place >= ABANDON_AFTER && self.place * 2 > seen {
            self.abandoned = true;
        }
        !self.abandoned
    }
}

/// `n` records as they were but those `entries` name, each as the most it
/// changed by, into `out`, counted in `tally` (which never gives up here: what
/// the owner said is read whole).
fn spread(out: &mut Vec<Change>, n: u32, entries: &[(u32, Change)], tally: &mut Tally) {
    out.clear();
    out.resize(n as usize, Change::Same);
    let (mut look, mut place) = (0u32, 0u32);
    for &(i, change) in entries {
        let Some(slot) = out.get_mut(i as usize) else {
            continue;
        };
        if change > *slot {
            match *slot {
                Change::Look => look -= 1,
                Change::Place => place -= 1,
                Change::Same => {}
            }
            match change {
                Change::Look => look += 1,
                Change::Place => place += 1,
                Change::Same => {}
            }
            *slot = change;
        }
    }
    tally.same += n - look - place;
    tally.look += look;
    tally.place += place;
}

/// Compare `old` with `new` one for one, recording each answer in `out`.
fn family<T>(
    old: &[T],
    new: &[T],
    out: &mut Vec<Change>,
    tally: &mut Tally,
    change: impl Fn(&T, &T) -> Change,
) {
    out.clear();
    if tally.abandoned {
        return;
    }
    for (a, b) in old.iter().zip(new) {
        let c = change(a, b);
        out.push(c);
        if !tally.record(c) {
            return;
        }
    }
}

impl FrameSnapshot {
    /// Bytes the kept records hold, about: what keeping a frame costs.
    pub(crate) fn bytes(&self) -> usize {
        use std::mem::size_of;
        size_of::<Vertex>() * self.vertices.capacity()
            + size_of::<u32>() * self.indices.capacity()
            + size_of::<AnalyticInstance>() * self.analytic.capacity()
            + size_of::<CircleInstance>() * self.circles.capacity()
            + size_of::<IconDraw>() * self.icons.capacity()
            + size_of::<NineSliceDraw>() * self.nine_slices.capacity()
            + size_of::<TextBlock>() * self.texts.capacity()
            + size_of::<PaintCmd>() * self.paint_cmds.capacity()
    }

    /// How `list`, drawn in `context`, differs from the frame kept.
    pub(crate) fn compare(&mut self, list: &DrawList, context: &ContextKey) -> FrameDiff {
        let mut diff = FrameDiff::default();
        self.complete = false;
        self.reported = None;
        if !self.kept {
            return diff;
        }
        diff.compared = true;
        diff.context_same = self.context.as_ref() == Some(context);
        diff.topology_same = self.same_topology(list);
        if !diff.topology_same {
            return diff;
        }
        let mut tally = Tally::default();
        let c = &mut self.changes;
        family(
            &self.analytic,
            &list.analytic_instances,
            &mut c.analytic,
            &mut tally,
            analytic_change,
        );
        family(
            &self.circles,
            &list.circle_instances,
            &mut c.circles,
            &mut tally,
            circle_change,
        );
        family(
            &self.icons,
            &list.icons,
            &mut c.icons,
            &mut tally,
            icon_change,
        );
        family(
            &self.nine_slices,
            &list.nine_slices,
            &mut c.nine_slices,
            &mut tally,
            nine_slice_change,
        );
        #[cfg(feature = "phosphor-icons")]
        family(
            &self.icons_msdf,
            &list.icons_msdf,
            &mut c.icons_msdf,
            &mut tally,
            icon_msdf_change,
        );
        family(
            &self.texts,
            &list.texts,
            &mut c.texts,
            &mut tally,
            text_change,
        );
        // Soup is compared a command at a time: a command's box reads only
        // the vertices its indices name.
        c.soup.clear();
        if !tally.abandoned {
            for cmd in &list.paint_cmds {
                if let PaintCmd::Soup { indices } = cmd {
                    let change = soup_change(
                        (&self.vertices, &self.indices),
                        (&list.vertices, &list.indices),
                        indices.start as usize..indices.end as usize,
                    );
                    c.soup.push(change);
                    if !tally.record(change) {
                        break;
                    }
                }
            }
        }
        if !tally.abandoned {
            let committed = list.soup_committed_indices as usize;
            c.tail = soup_change(
                (&self.vertices, &self.indices),
                (&list.vertices, &list.indices),
                committed..list.indices.len(),
            );
            tally.record(c.tail);
        }
        diff.same = tally.same;
        diff.look = tally.look;
        diff.place = tally.place;
        diff.abandoned = tally.abandoned;
        self.complete = !tally.abandoned;
        // A list that keeps changing place is not worth keeping and comparing
        // every frame: rest for longer each time, and start again once one is
        // not given up on.
        self.backoff = if tally.abandoned {
            (self.backoff * 2).clamp(1, MAX_REST)
        } else {
            0
        };
        if tally.abandoned {
            self.rest = self.backoff;
        }
        diff
    }

    /// Whether to skip this frame, neither comparing nor keeping it. Rests
    /// after a compare gave up, so that a list in which everything moves pays
    /// for the comparison on one frame in a few, not on every one. What was
    /// kept is dropped, since the frames skipped make it stale.
    pub(crate) fn resting(&mut self) -> bool {
        if self.rest == 0 {
            return false;
        }
        self.rest -= 1;
        self.forget();
        true
    }

    /// Drop the frame kept: the next list is kept without being compared.
    pub(crate) fn forget(&mut self) {
        self.kept = false;
        self.complete = false;
        self.reported = None;
    }

    /// How `list`, whose owner reports its edits, drawn in `context`, differs
    /// from the list drawn last in this place, from what its owner says
    /// rather than by comparing: answered when it is that list again, of the
    /// same counts on the same target, either untouched since (every record
    /// [`Change::Same`]) or patched since [`DrawList::begin_edits`] was called
    /// on it as it was drawn (the log's changes, the rest `Same`). Anything
    /// else is not answered, and the list is planned from nothing. No record
    /// is kept, and what was kept for another list in this place is let go.
    pub(crate) fn reported(&mut self, list: &DrawList, context: &ContextKey) -> FrameDiff {
        self.release();
        let now = Reported {
            id: list.id(),
            version: list.version(),
            counts: list.record_counts(),
            context: *context,
        };
        let mut diff = FrameDiff::default();
        let Some(last) = self.reported.replace(now) else {
            return diff;
        };
        diff.compared = true;
        diff.context_same = last.context == now.context;
        let untouched = last.version == now.version;
        let log = list.edit_log();
        let logged = log.base == Some(last.version);
        diff.topology_same =
            last.id == now.id && last.counts == now.counts && (untouched || logged);
        if !diff.topology_same {
            return diff;
        }
        let mut tally = Tally::default();
        let c = &mut self.changes;
        let counts = now.counts;
        let mut fill = |out: &mut Vec<Change>, n: u32, entries: &[(u32, Change)]| {
            spread(out, n, if untouched { &[] } else { entries }, &mut tally);
        };
        fill(&mut c.analytic, counts.analytic, &log.analytic);
        fill(&mut c.circles, counts.circles, &log.circles);
        fill(&mut c.icons, counts.icons, &log.icons);
        fill(&mut c.nine_slices, counts.nine_slices, &log.nine_slices);
        #[cfg(feature = "phosphor-icons")]
        fill(&mut c.icons_msdf, counts.icons_msdf, &log.icons_msdf);
        fill(&mut c.texts, counts.texts, &log.texts);
        // The soup is told of as a whole, by the most any part of it changed:
        // each of its commands counts as changed by that.
        let soup = if untouched { Change::Same } else { log.soup };
        c.soup.clear();
        for cmd in &list.paint_cmds {
            if matches!(cmd, PaintCmd::Soup { .. }) {
                c.soup.push(soup);
                tally.record(soup);
            }
        }
        // Counted as the comparison counts it, whether or not it is empty.
        c.tail = soup;
        tally.record(soup);
        diff.same = tally.same;
        diff.look = tally.look;
        diff.place = tally.place;
        diff
    }

    /// Check what [`reported`](Self::reported) answered for `list`, `diff`
    /// with the answers in `self`, against `oracle`, which compares `list`
    /// with the last list it kept: wherever both answer, every record must
    /// be reported as changed at least as much as the comparison finds it.
    #[cfg(feature = "edit-oracle")]
    pub(crate) fn check_reported(
        &self,
        diff: &FrameDiff,
        oracle: &mut FrameSnapshot,
        list: &DrawList,
        context: &ContextKey,
    ) {
        let found = oracle.compare(list, context);
        oracle.keep(list, context);
        if !diff.answered() || !found.compared {
            return;
        }
        assert!(
            found.context_same && found.topology_same,
            "a reported list was answered, but differs in shape or target from the last \
             one drawn in its place"
        );
        if found.abandoned {
            return;
        }
        fn covers(what: &str, told: &[Change], found: &[Change]) {
            assert_eq!(told.len(), found.len(), "{what}: answers for other counts");
            if let Some(i) = (0..told.len()).find(|&i| told[i] < found[i]) {
                panic!(
                    "{what} {i}: reported {:?}, but it changed by {:?}",
                    told[i], found[i]
                );
            }
        }
        let (t, f) = (&self.changes, &oracle.changes);
        covers("analytic", &t.analytic, &f.analytic);
        covers("circle", &t.circles, &f.circles);
        covers("icon", &t.icons, &f.icons);
        covers("nine-slice", &t.nine_slices, &f.nine_slices);
        #[cfg(feature = "phosphor-icons")]
        covers("MSDF icon", &t.icons_msdf, &f.icons_msdf);
        covers("text", &t.texts, &f.texts);
        covers("soup command", &t.soup, &f.soup);
        covers("soup tail", &[t.tail], &[f.tail]);
    }

    /// Let go of the records kept, and the memory they hold.
    fn release(&mut self) {
        if !self.kept && self.bytes() == 0 {
            return;
        }
        self.kept = false;
        self.complete = false;
        self.paint_cmds = Vec::new();
        self.vertices = Vec::new();
        self.indices = Vec::new();
        self.analytic = Vec::new();
        self.circles = Vec::new();
        self.icons = Vec::new();
        self.nine_slices = Vec::new();
        #[cfg(feature = "phosphor-icons")]
        {
            self.icons_msdf = Vec::new();
        }
        self.texts = Vec::new();
    }

    /// Whether `list` is made of the same paint stream and arrays of the same
    /// lengths as the kept frame, so its records line up with the kept ones.
    fn same_topology(&self, list: &DrawList) -> bool {
        let same = self.paint_cmds == list.paint_cmds
            && self.soup_committed == list.soup_committed_indices
            && self.vertices.len() == list.vertices.len()
            && self.indices.len() == list.indices.len()
            && self.analytic.len() == list.analytic_instances.len()
            && self.circles.len() == list.circle_instances.len()
            && self.icons.len() == list.icons.len()
            && self.nine_slices.len() == list.nine_slices.len()
            && self.texts.len() == list.texts.len();
        #[cfg(feature = "phosphor-icons")]
        let same = same && self.icons_msdf.len() == list.icons_msdf.len();
        same
    }

    /// Keep `list`'s records, drawn in `context`, to compare the next list with.
    /// Buffers are reused, so a frame of the size of the last allocates only
    /// for the records that own heap data (a name, spans). When the last
    /// compare answered for every record, only the records that changed are
    /// copied: a list that is the last one again copies nothing.
    pub(crate) fn keep(&mut self, list: &DrawList, context: &ContextKey) {
        self.context = Some(*context);
        if self.kept && self.complete {
            let c = &self.changes;
            update(&mut self.analytic, &list.analytic_instances, &c.analytic);
            update(&mut self.circles, &list.circle_instances, &c.circles);
            update(&mut self.icons, &list.icons, &c.icons);
            update(&mut self.nine_slices, &list.nine_slices, &c.nine_slices);
            #[cfg(feature = "phosphor-icons")]
            update(&mut self.icons_msdf, &list.icons_msdf, &c.icons_msdf);
            update(&mut self.texts, &list.texts, &c.texts);
            // The paint stream and where the soup was committed are what the
            // compare found them to be. The soup is whole or not at all: its
            // commands share one array of vertices.
            if c.soup.iter().chain([&c.tail]).any(|&c| c != Change::Same) {
                self.vertices.clone_from(&list.vertices);
                self.indices.clone_from(&list.indices);
            }
            return;
        }
        self.paint_cmds.clone_from(&list.paint_cmds);
        self.vertices.clone_from(&list.vertices);
        self.indices.clone_from(&list.indices);
        self.soup_committed = list.soup_committed_indices;
        self.analytic.clone_from(&list.analytic_instances);
        self.circles.clone_from(&list.circle_instances);
        self.icons.clone_from(&list.icons);
        self.nine_slices.clone_from(&list.nine_slices);
        #[cfg(feature = "phosphor-icons")]
        self.icons_msdf.clone_from(&list.icons_msdf);
        self.texts.clone_from(&list.texts);
        self.kept = true;
    }
}

/// Copy the records of `new` that `changes` says differ into `kept`, which
/// holds the records they were compared with.
impl DrawList {
    /// The first part of what this list records that `other` records
    /// differently, named and shown, or `None` when the two would draw the
    /// same frame bit for bit: every record array compared byte for byte, in
    /// order, and text blocks, icons and nine-slices as the renderer compares
    /// frames (every field that reaches the screen). Also compared: the
    /// repaint request, the viewport clips, the selectable texts' keys and
    /// places, and the count of degenerate calls dropped. Debug scopes are
    /// not.
    ///
    /// For tests: two ways of recording a frame (drawn again from pieces,
    /// drawn from nothing) checked to agree.
    pub fn difference(&self, other: &DrawList) -> Option<String> {
        fn first<T: std::fmt::Debug>(
            what: &str,
            a: &[T],
            b: &[T],
            same: impl Fn(&T, &T) -> bool,
        ) -> Option<String> {
            if a.len() != b.len() {
                return Some(format!("{what}: {} against {}", a.len(), b.len()));
            }
            let (k, (x, y)) = a
                .iter()
                .zip(b)
                .enumerate()
                .find(|(_, (x, y))| !same(x, y))?;
            Some(format!("{what} {k}: {x:?} against {y:?}"))
        }
        fn bytes<T: bytemuck::Pod>(a: &T, b: &T) -> bool {
            bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
        }
        fn rect(r: &Rect) -> [u32; 4] {
            [r.x, r.y, r.width, r.height].map(f32::to_bits)
        }
        fn block(t: &TextBlock) -> String {
            format!(
                "{:?} at ({}, {}) clip {:?}",
                t.content.as_str(),
                t.x,
                t.y,
                t.clip
            )
        }
        #[cfg(feature = "phosphor-icons")]
        fn msdf_icons(a: &DrawList, b: &DrawList) -> Option<String> {
            first("MSDF icon", &a.icons_msdf, &b.icons_msdf, |a, b| {
                icon_msdf_change(a, b) == Change::Same
            })
        }
        #[cfg(not(feature = "phosphor-icons"))]
        fn msdf_icons(_: &DrawList, _: &DrawList) -> Option<String> {
            None
        }
        fn selectable(list: &DrawList) -> Vec<(crate::TextKey, u32, u32)> {
            list.selectable_texts()
                .map(|(key, t)| (key, t.x.to_bits(), t.y.to_bits()))
                .collect()
        }
        first(
            "paint command",
            &self.paint_cmds,
            &other.paint_cmds,
            |a, b| a == b,
        )
        .or_else(|| {
            (self.soup_committed_indices != other.soup_committed_indices).then(|| {
                format!(
                    "trailing soup from index {} against {}",
                    self.soup_committed_indices, other.soup_committed_indices
                )
            })
        })
        .or_else(|| first("vertex", &self.vertices, &other.vertices, bytes))
        .or_else(|| first("index", &self.indices, &other.indices, |a, b| a == b))
        .or_else(|| {
            first(
                "analytic instance",
                &self.analytic_instances,
                &other.analytic_instances,
                bytes,
            )
        })
        .or_else(|| {
            first(
                "circle",
                &self.circle_instances,
                &other.circle_instances,
                bytes,
            )
        })
        .or_else(|| {
            if self.texts.len() != other.texts.len() {
                return Some(format!(
                    "text: {} blocks against {}",
                    self.texts.len(),
                    other.texts.len()
                ));
            }
            let (k, (a, b)) = self
                .texts
                .iter()
                .zip(&other.texts)
                .enumerate()
                .find(|(_, (a, b))| text_change(a, b) != Change::Same)?;
            Some(format!("text {k}: {} against {}", block(a), block(b)))
        })
        .or_else(|| {
            first("icon", &self.icons, &other.icons, |a, b| {
                icon_change(a, b) == Change::Same
            })
        })
        .or_else(|| {
            first(
                "nine-slice",
                &self.nine_slices,
                &other.nine_slices,
                |a, b| nine_slice_change(a, b) == Change::Same,
            )
        })
        .or_else(|| msdf_icons(self, other))
        .or_else(|| {
            first(
                "viewport clip",
                self.viewport_clips(),
                other.viewport_clips(),
                |a, b| rect(a) == rect(b),
            )
        })
        .or_else(|| {
            first(
                "selectable text",
                &selectable(self),
                &selectable(other),
                |a, b| a == b,
            )
        })
        .or_else(|| {
            let (a, b) = (self.next_repaint(), other.next_repaint());
            (a.map(f32::to_bits) != b.map(f32::to_bits))
                .then(|| format!("repaint after {a:?} against {b:?}"))
        })
        .or_else(|| {
            let (a, b) = (self.dropped_degenerate(), other.dropped_degenerate());
            (a != b).then(|| format!("{a} degenerate calls dropped against {b}"))
        })
    }
}

fn update<T: Clone>(kept: &mut [T], new: &[T], changes: &[Change]) {
    debug_assert!(kept.len() == new.len() && changes.len() == new.len());
    for ((kept, new), change) in kept.iter_mut().zip(new).zip(changes) {
        if *change != Change::Same {
            kept.clone_from(new);
        }
    }
}

fn bits4(v: &[f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}

fn rect_bits(r: &Option<Rect>) -> Option<[u32; 4]> {
    r.as_ref()
        .map(|r| [r.x, r.y, r.width, r.height].map(f32::to_bits))
}

pub(crate) fn analytic_change(a: &AnalyticInstance, b: &AnalyticInstance) -> Change {
    if bytemuck::bytes_of(a) == bytemuck::bytes_of(b) {
        Change::Same
    } else if same_analytic_box_inputs(a, b) {
        Change::Look
    } else {
        Change::Place
    }
}

pub(crate) fn circle_change(a: &CircleInstance, b: &CircleInstance) -> Change {
    if bytemuck::bytes_of(a) == bytemuck::bytes_of(b) {
        Change::Same
    } else if same_circle_box_inputs(a, b) {
        Change::Look
    } else {
        Change::Place
    }
}

/// The triangles `range` of the indices names, compared by the vertices they
/// reach: the indices must be the same, and then each vertex as `Look` or
/// worse. An index past the vertices, which the box takes as the whole target,
/// is a place.
fn soup_change(
    old: (&[Vertex], &[u32]),
    new: (&[Vertex], &[u32]),
    range: std::ops::Range<usize>,
) -> Change {
    let (Some(old_indices), Some(new_indices)) = (old.1.get(range.clone()), new.1.get(range))
    else {
        return Change::Place;
    };
    if old_indices != new_indices {
        return Change::Place;
    }
    let mut change = Change::Same;
    for &index in new_indices {
        let (Some(a), Some(b)) = (old.0.get(index as usize), new.0.get(index as usize)) else {
            return Change::Place;
        };
        if bytemuck::bytes_of(a) != bytemuck::bytes_of(b) {
            if !same_vertex_box_inputs(a, b) {
                return Change::Place;
            }
            change = Change::Look;
        }
    }
    change
}

pub(crate) fn icon_change(a: &IconDraw, b: &IconDraw) -> Change {
    let IconDraw {
        corners,
        sprite,
        icon_key,
        tint,
        clip,
        src,
        wrap,
    } = a;
    let placed = corners.map(|c| c.map(f32::to_bits)) == b.corners.map(|c| c.map(f32::to_bits))
        && *sprite == b.sprite
        && *icon_key == b.icon_key
        && rect_bits(clip) == rect_bits(&b.clip)
        && src.as_ref().map(bits4) == b.src.as_ref().map(bits4)
        && *wrap == b.wrap;
    look_or_place(placed, bits4(tint) == bits4(&b.tint))
}

pub(crate) fn nine_slice_change(a: &NineSliceDraw, b: &NineSliceDraw) -> Change {
    let NineSliceDraw {
        local,
        transform,
        nine_slice,
        texture_key,
        tint,
        clip,
    } = a;
    let affine = |t: &crate::Affine2| [t.a, t.b, t.c, t.d, t.tx, t.ty].map(f32::to_bits);
    let placed = rect_bits(&Some(*local)) == rect_bits(&Some(b.local))
        && affine(transform) == affine(&b.transform)
        && *nine_slice == b.nine_slice
        && *texture_key == b.texture_key
        && rect_bits(clip) == rect_bits(&b.clip);
    look_or_place(placed, bits4(tint) == bits4(&b.tint))
}

#[cfg(feature = "phosphor-icons")]
pub(crate) fn icon_msdf_change(
    a: &crate::widgets::IconMsdf,
    b: &crate::widgets::IconMsdf,
) -> Change {
    let crate::widgets::IconMsdf {
        local,
        transform,
        glyph,
        tint,
        clip,
    } = a;
    let affine = |t: &crate::Affine2| [t.a, t.b, t.c, t.d, t.tx, t.ty].map(f32::to_bits);
    let placed = rect_bits(&Some(*local)) == rect_bits(&Some(b.local))
        && affine(transform) == affine(&b.transform)
        && *glyph == b.glyph
        && rect_bits(clip) == rect_bits(&b.clip);
    look_or_place(placed, bits4(tint) == bits4(&b.tint))
}

/// `Same` when nothing differs, `Look` when only what doesn't place differs,
/// `Place` otherwise.
fn look_or_place(placed: bool, looks: bool) -> Change {
    match (placed, looks) {
        (true, true) => Change::Same,
        (true, false) => Change::Look,
        (false, _) => Change::Place,
    }
}

fn colour_bits(c: &cosmic_text::Color) -> u32 {
    c.0
}

/// Text blocks. Every field of a block is named below, so a field added to
/// [`TextBlock`] does not compile until it is placed here: the colours of the
/// block and of its outline, shadow and glow only look; the rest places.
pub(crate) fn text_change(a: &TextBlock, b: &TextBlock) -> Change {
    let TextBlock {
        content,
        x,
        y,
        font_size,
        line_height,
        max_width,
        letter_spacing,
        color,
        clip,
        outline,
        shadow,
        glow,
        font,
        align,
        direction,
        ellipsize,
        weight,
        style,
        spans,
        style_ranges,
        style_range_tint,
        face_ranges,
        wrap,
        vertical,
        selectable,
        // A prepared layout is the layout the rest of the fields give, so
        // two blocks the same in those are drawn the same with or without.
        prepared: _,
    } = a;
    let f = |v: f32| v.to_bits();
    let spans_same = spans.len() == b.spans.len()
        && spans.iter().zip(&b.spans).all(|(p, q)| {
            p.text == q.text
                && p.color.as_ref().map(bits4) == q.color.as_ref().map(bits4)
                && p.underline == q.underline
        });
    let ranges_same =
        std::sync::Arc::ptr_eq(style_ranges, &b.style_ranges) || **style_ranges == *b.style_ranges;
    let faces_same =
        std::sync::Arc::ptr_eq(face_ranges, &b.face_ranges) || **face_ranges == *b.face_ranges;
    // An effect is in the same place if both are there or both are not, and
    // its numbers agree; its colour is its look.
    let outline_same = match (outline, &b.outline) {
        (None, None) => true,
        (Some(p), Some(q)) => f(p.width_px) == f(q.width_px),
        _ => false,
    };
    let shadow_same = match (shadow, &b.shadow) {
        (None, None) => true,
        (Some(p), Some(q)) => {
            p.offset.map(f32::to_bits) == q.offset.map(f32::to_bits)
                && f(p.softness) == f(q.softness)
        }
        _ => false,
    };
    let glow_same = match (glow, &b.glow) {
        (None, None) => true,
        (Some(p), Some(q)) => f(p.radius_px) == f(q.radius_px),
        _ => false,
    };
    let placed = *content == b.content
        && f(*x) == f(b.x)
        && f(*y) == f(b.y)
        && f(*font_size) == f(b.font_size)
        && f(*line_height) == f(b.line_height)
        && f(*max_width) == f(b.max_width)
        && f(*letter_spacing) == f(b.letter_spacing)
        && rect_bits(clip) == rect_bits(&b.clip)
        && outline_same
        && shadow_same
        && glow_same
        && *font == b.font
        && *align == b.align
        && *direction == b.direction
        && *ellipsize == b.ellipsize
        && *weight == b.weight
        && *style == b.style
        && spans_same
        && ranges_same
        && bits4(style_range_tint) == bits4(&b.style_range_tint)
        && faces_same
        && *wrap == b.wrap
        && *vertical == b.vertical
        && *selectable == b.selectable;
    let looks = colour_bits(color) == colour_bits(&b.color)
        && effect_colour(outline.as_ref().map(|o: &TextOutline| &o.color))
            == effect_colour(b.outline.as_ref().map(|o| &o.color))
        && effect_colour(shadow.as_ref().map(|s| &s.color))
            == effect_colour(b.shadow.as_ref().map(|s| &s.color))
        && effect_colour(glow.as_ref().map(|g| &g.color))
            == effect_colour(b.glow.as_ref().map(|g| &g.color));
    look_or_place(placed, looks)
}

fn effect_colour(c: Option<&cosmic_text::Color>) -> Option<u32> {
    c.map(colour_bits)
}
