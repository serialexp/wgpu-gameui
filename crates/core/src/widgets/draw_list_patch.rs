//! Patching a list placed into another in place: when the list placed is
//! recorded again with the same shape (the same paint runs over the same
//! number of records), the records [`DrawList::append_translated`] put in the
//! other list are overwritten where they are, rather than the other list being
//! recorded again. A retained tree uses this to carry a change to a deep node
//! up through its ancestors' lists in the work of the records that changed.

#[cfg(feature = "phosphor-icons")]
use super::edits::icon_msdf_change;
use super::edits::{
    Change, analytic_change, circle_change, icon_change, log_change, nine_slice_change,
    same_vertex_box_inputs, text_change,
};
use super::*;

/// A position in each of a list's record arrays: how many records of each
/// kind a list holds, or where a run of them starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordCounts {
    /// Soup vertices.
    pub vertices: u32,
    /// Soup indices.
    pub indices: u32,
    /// Analytic records (chrome, shadows, stripes, segments).
    pub analytic: u32,
    /// Circles.
    pub circles: u32,
    /// Nine-slice panels.
    pub nine_slices: u32,
    /// Sprite icons.
    pub icons: u32,
    /// MSDF icons; always 0 without the `phosphor-icons` feature.
    pub icons_msdf: u32,
    /// Text blocks.
    pub texts: u32,
    /// Selectable texts' keys.
    pub selectable: u32,
    /// Viewport clips (see [`DrawList::push_clip_viewport`]).
    pub viewport_clips: u32,
}

impl RecordCounts {
    /// Every position moved on by `by`'s.
    pub fn offset_by(self, by: RecordCounts) -> RecordCounts {
        RecordCounts {
            vertices: self.vertices + by.vertices,
            indices: self.indices + by.indices,
            analytic: self.analytic + by.analytic,
            circles: self.circles + by.circles,
            nine_slices: self.nine_slices + by.nine_slices,
            icons: self.icons + by.icons,
            icons_msdf: self.icons_msdf + by.icons_msdf,
            texts: self.texts + by.texts,
            selectable: self.selectable + by.selectable,
            viewport_clips: self.viewport_clips + by.viewport_clips,
        }
    }

    /// Its positions, in a fixed order (see [`Self::from_fields`]).
    fn fields(self) -> [u32; COUNT_KINDS] {
        [
            self.vertices,
            self.indices,
            self.analytic,
            self.circles,
            self.nine_slices,
            self.icons,
            self.icons_msdf,
            self.texts,
            self.selectable,
            self.viewport_clips,
        ]
    }

    fn from_fields(f: [u32; COUNT_KINDS]) -> Self {
        let [
            vertices,
            indices,
            analytic,
            circles,
            nine_slices,
            icons,
            icons_msdf,
            texts,
            selectable,
            viewport_clips,
        ] = f;
        RecordCounts {
            vertices,
            indices,
            analytic,
            circles,
            nine_slices,
            icons,
            icons_msdf,
            texts,
            selectable,
            viewport_clips,
        }
    }

    /// Whether no position is past `limit`'s.
    fn within(self, limit: RecordCounts) -> bool {
        self.vertices <= limit.vertices
            && self.indices <= limit.indices
            && self.analytic <= limit.analytic
            && self.circles <= limit.circles
            && self.nine_slices <= limit.nine_slices
            && self.icons <= limit.icons
            && self.icons_msdf <= limit.icons_msdf
            && self.texts <= limit.texts
            && self.selectable <= limit.selectable
            && self.viewport_clips <= limit.viewport_clips
    }
}

/// A run of a list's records: from `start` up to `end` in each of its arrays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordSpan {
    /// The first record of each kind.
    pub start: RecordCounts,
    /// One past the last record of each kind.
    pub end: RecordCounts,
}

impl RecordSpan {
    /// Every record `list` holds.
    pub fn all_of(list: &DrawList) -> RecordSpan {
        RecordSpan {
            start: RecordCounts::default(),
            end: list.record_counts(),
        }
    }

    /// The span moved on by `by`: where these records are in a list they were
    /// placed into at `by`.
    pub fn offset_by(self, by: RecordCounts) -> RecordSpan {
        RecordSpan {
            start: self.start.offset_by(by),
            end: self.end.offset_by(by),
        }
    }

    /// Records in the span, of every kind.
    pub fn len(&self) -> u64 {
        let (s, e) = (self.start, self.end);
        [
            e.vertices.saturating_sub(s.vertices),
            e.analytic.saturating_sub(s.analytic),
            e.circles.saturating_sub(s.circles),
            e.nine_slices.saturating_sub(s.nine_slices),
            e.icons.saturating_sub(s.icons),
            e.icons_msdf.saturating_sub(s.icons_msdf),
            e.texts.saturating_sub(s.texts),
        ]
        .iter()
        .map(|&n| u64::from(n))
        .sum()
    }

    /// Whether the span holds no record.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where [`DrawList::append_translated`] put a list's records in another:
/// what [`DrawList::patch_translated`] needs to overwrite them there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placed {
    /// The offset it was placed at.
    pub offset: [f32; 2],
    /// The clip it was placed under.
    pub clip: Option<Rect>,
    /// Where its records start in the list it was placed into.
    pub at: RecordCounts,
    /// What it held when placed.
    pub of: RecordCounts,
    /// Every record was placed: none was culled by the clip, so record `i`
    /// of each kind is at `at + i`.
    pub whole: bool,
}

/// The lists placed into one list, each logged with where it went and its
/// shape then, so that any of them can later be patched where it is
/// ([`DrawList::patch_translated`]).
///
/// A list's shape is how many records of each kind it holds and the paint
/// runs over them, and what else placing it adds to another list (its repaint
/// request, its count of degenerate calls dropped), but not what the records
/// hold. Two lists of one shape placed the same way leave the other list's
/// paint runs as they were, so one can be patched over the other.
///
/// The log is flat: the paint runs of every piece share one array, so a list
/// built from many small pieces (a retained tree drawing each node's own
/// records into the list of the container that keeps one) logs them without
/// an allocation per piece. [`clear`](Self::clear) keeps the arrays for the
/// next recording.
///
/// And small: a piece's counts are kept as the numbers that are not zero,
/// each in as few bytes as it needs, and its clip as an index into the clips
/// the log holds, the same clip of pieces placed one after another once.
/// A grid of 20,000 pieces reserves about 72 bytes a piece, paint runs
/// included, where whole `Placed`s took about 240.
#[derive(Clone, Debug, Default)]
pub struct PlacedLog {
    pieces: Vec<LoggedPiece>,
    paint_cmds: Vec<PaintCmd>,
    /// Every piece's numbers that are not zero, as LEB128 varints, in
    /// piece order: its position's (`at`), then its counts' (`of`), then
    /// what else placing it adds.
    numbers: Vec<u8>,
    /// The clips pieces were placed under.
    clips: Vec<Rect>,
}

/// Kinds of record a [`RecordCounts`] counts.
const COUNT_KINDS: usize = 10;
/// Bits of a piece's `shape` past the kinds: what else placing it adds is
/// logged after its counts, when not zero (or `None`).
const SOUP_COMMITTED: u16 = 1 << COUNT_KINDS;
const NEXT_REPAINT: u16 = 1 << (COUNT_KINDS + 1);
const DROPPED_DEGENERATE: u16 = 1 << (COUNT_KINDS + 2);
/// A piece placed under no clip.
const NO_CLIP: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct LoggedPiece {
    /// The offset it was placed at.
    offset: [f32; 2],
    /// Its clip in the log's `clips`, or [`NO_CLIP`].
    clip: u32,
    /// Where its paint runs start in the log's `paint_cmds`: they end where
    /// the next piece's start.
    paint_cmds: u32,
    /// Where its numbers start in the log's `numbers`.
    numbers: u32,
    /// Which kinds of its position are not zero.
    at: u16,
    /// Which kinds of its counts are not zero, and which of what else
    /// placing it adds (`SOUP_COMMITTED` and on).
    shape: u16,
    /// [`Placed::whole`].
    whole: bool,
}

/// What a piece was placed with, read back from the log.
struct Logged {
    placed: Placed,
    paint_cmds: std::ops::Range<usize>,
    soup_committed: u32,
    next_repaint: Option<u32>,
    dropped_degenerate: u32,
}

fn push_varint(bytes: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        bytes.push(value as u8 | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn read_varint(bytes: &[u8], at: &mut usize) -> u32 {
    let mut value = 0u32;
    let mut shift = 0;
    loop {
        let byte = bytes[*at];
        *at += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return value;
        }
        shift += 7;
    }
}

/// The fields of `counts` that are not zero, as bits, logged into `bytes`.
fn push_counts(bytes: &mut Vec<u8>, counts: RecordCounts) -> u16 {
    let mut kinds = 0;
    for (kind, value) in counts.fields().into_iter().enumerate() {
        if value != 0 {
            kinds |= 1 << kind;
            push_varint(bytes, value);
        }
    }
    kinds
}

fn read_counts(bytes: &[u8], at: &mut usize, kinds: u16) -> RecordCounts {
    let mut fields = [0; COUNT_KINDS];
    for (kind, field) in fields.iter_mut().enumerate() {
        if kinds & (1 << kind) != 0 {
            *field = read_varint(bytes, at);
        }
    }
    RecordCounts::from_fields(fields)
}

fn same_rect(a: &Rect, b: &Rect) -> bool {
    [a.x, a.y, a.width, a.height].map(f32::to_bits)
        == [b.x, b.y, b.width, b.height].map(f32::to_bits)
}

impl PlacedLog {
    /// Forget every piece, keeping the arrays.
    pub fn clear(&mut self) {
        self.pieces.clear();
        self.paint_cmds.clear();
        self.numbers.clear();
        self.clips.clear();
    }

    /// Pieces logged.
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// Whether no piece is logged.
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// Place `other` into `list` at `offset` under `clip`
    /// ([`DrawList::append_translated`]) and log it: the piece's number in
    /// the log.
    pub fn place(
        &mut self,
        list: &mut DrawList,
        other: &DrawList,
        offset: [f32; 2],
        clip: Option<Rect>,
    ) -> u32 {
        let placed = list.append_translated(other, offset, clip);
        let paint_cmds = self.paint_cmds.len() as u32;
        self.paint_cmds.extend_from_slice(&other.paint_cmds);
        let clip = match placed.clip {
            None => NO_CLIP,
            Some(rect) => match self.clips.last() {
                Some(last) if same_rect(last, &rect) => self.clips.len() as u32 - 1,
                _ => {
                    self.clips.push(rect);
                    self.clips.len() as u32 - 1
                }
            },
        };
        let numbers = self.numbers.len() as u32;
        let at = push_counts(&mut self.numbers, placed.at);
        let mut shape = push_counts(&mut self.numbers, placed.of);
        if other.soup_committed_indices != 0 {
            shape |= SOUP_COMMITTED;
            push_varint(&mut self.numbers, other.soup_committed_indices);
        }
        if let Some(repaint) = other.next_repaint {
            shape |= NEXT_REPAINT;
            push_varint(&mut self.numbers, repaint.to_bits());
        }
        if other.dropped_degenerate != 0 {
            shape |= DROPPED_DEGENERATE;
            push_varint(&mut self.numbers, other.dropped_degenerate);
        }
        self.pieces.push(LoggedPiece {
            offset: placed.offset,
            clip,
            paint_cmds,
            numbers,
            at,
            shape,
            whole: placed.whole,
        });
        self.pieces.len() as u32 - 1
    }

    /// What piece `piece` was placed with, if the log holds it.
    fn logged(&self, piece: u32) -> Option<Logged> {
        let p = self.pieces.get(piece as usize)?;
        let end = self
            .pieces
            .get(piece as usize + 1)
            .map_or(self.paint_cmds.len(), |next| next.paint_cmds as usize);
        let bytes = &self.numbers[..];
        let mut at = p.numbers as usize;
        let position = read_counts(bytes, &mut at, p.at);
        let of = read_counts(bytes, &mut at, p.shape);
        let mut extra = |bit: u16| (p.shape & bit != 0).then(|| read_varint(bytes, &mut at));
        let soup_committed = extra(SOUP_COMMITTED).unwrap_or(0);
        let next_repaint = extra(NEXT_REPAINT);
        let dropped_degenerate = extra(DROPPED_DEGENERATE).unwrap_or(0);
        Some(Logged {
            placed: Placed {
                offset: p.offset,
                clip: (p.clip != NO_CLIP).then(|| self.clips[p.clip as usize]),
                at: position,
                of,
                whole: p.whole,
            },
            paint_cmds: p.paint_cmds as usize..end,
            soup_committed,
            next_repaint,
            dropped_degenerate,
        })
    }

    /// Where piece `piece` was placed, if the log holds it.
    pub fn placed(&self, piece: u32) -> Option<Placed> {
        self.logged(piece).map(|logged| logged.placed)
    }

    /// Whether `other` has the shape piece `piece` had when placed, so that
    /// it can be patched over it.
    pub fn fits(&self, piece: u32, other: &DrawList) -> bool {
        let Some(p) = self.logged(piece) else {
            return false;
        };
        other.record_counts() == p.placed.of
            && other.soup_committed_indices == p.soup_committed
            && other.next_repaint.map(f32::to_bits) == p.next_repaint
            && other.dropped_degenerate == p.dropped_degenerate
            && other.paint_cmds[..] == self.paint_cmds[p.paint_cmds]
    }

    /// Bytes the log has reserved.
    pub fn heap_bytes(&self) -> usize {
        self.pieces.capacity() * std::mem::size_of::<LoggedPiece>()
            + self.paint_cmds.capacity() * std::mem::size_of::<PaintCmd>()
            + self.numbers.capacity()
            + self.clips.capacity() * std::mem::size_of::<Rect>()
    }
}

impl DrawList {
    /// How many records of each kind the list holds.
    pub fn record_counts(&self) -> RecordCounts {
        #[cfg(feature = "phosphor-icons")]
        let icons_msdf = self.icons_msdf.len() as u32;
        #[cfg(not(feature = "phosphor-icons"))]
        let icons_msdf = 0;
        RecordCounts {
            vertices: self.vertices.len() as u32,
            indices: self.indices.len() as u32,
            analytic: self.analytic_instances.len() as u32,
            circles: self.circle_instances.len() as u32,
            nine_slices: self.nine_slices.len() as u32,
            icons: self.icons.len() as u32,
            icons_msdf,
            texts: self.texts.len() as u32,
            selectable: self.selectable.len() as u32,
            viewport_clips: self.viewport_clips.len() as u32,
        }
    }

    /// Overwrite the records of `span` of `other`, which was placed into this
    /// list as `placed` says, with `other`'s records there now, placed the
    /// same way: the list [`append_translated`](Self::append_translated)
    /// would have made had `other` held them then.
    ///
    /// `other` must have the shape it had when placed (the caller checks
    /// that, with [`PlacedLog::fits`]), and this list
    /// must hold what it held after the placement, other records apart.
    /// `false`, when what this call can see says the patch would not give that
    /// list: the placement culled a record, `other` holds other counts, the
    /// span runs past them, a record would now be culled, or an analytic
    /// record changed kind. The list is then partly patched, and must be
    /// recorded again.
    pub fn patch_translated(
        &mut self,
        placed: &Placed,
        other: &DrawList,
        span: &RecordSpan,
    ) -> bool {
        if !placed.whole
            || other.record_counts() != placed.of
            || !span.end.within(placed.of)
            || !span.start.within(span.end)
        {
            return false;
        }
        let place = Placement {
            dx: placed.offset[0],
            dy: placed.offset[1],
            clip: placed.clip,
        };
        self.edits.patches += 1;
        // What the renderer is told changed, when the owner reports it.
        let log = self.edits.logging();
        let (s, e, at) = (span.start, span.end, placed.at);
        let to = |base: u32, i: u32| (base + i) as usize;
        for i in s.vertices..e.vertices {
            let vertex = place.vertex(other.vertices[i as usize]);
            let slot = &mut self.vertices[to(at.vertices, i)];
            if log && bytemuck::bytes_of(slot) != bytemuck::bytes_of(&vertex) {
                let change = if same_vertex_box_inputs(slot, &vertex) {
                    Change::Look
                } else {
                    Change::Place
                };
                self.edits.soup = self.edits.soup.max(change);
            }
            *slot = vertex;
        }
        for i in s.indices..e.indices {
            let index = other.indices[i as usize] + at.vertices;
            let slot = &mut self.indices[to(at.indices, i)];
            if log && *slot != index {
                self.edits.soup = Change::Place;
            }
            *slot = index;
        }
        for i in s.analytic..e.analytic {
            let Some(record) = place.analytic(&other.analytic_instances[i as usize]) else {
                return false;
            };
            let k = to(at.analytic, i);
            let slot = &mut self.analytic_instances[k];
            if slot.kind != record.kind {
                return false;
            }
            if log {
                log_change(&mut self.edits.analytic, k, slot, &record, analytic_change);
            }
            *slot = record;
        }
        for i in s.circles..e.circles {
            let record = place.circle(other.circle_instances[i as usize]);
            let k = to(at.circles, i);
            if log {
                let slot = &self.circle_instances[k];
                log_change(&mut self.edits.circles, k, slot, &record, circle_change);
            }
            self.circle_instances[k] = record;
        }
        for i in s.nine_slices..e.nine_slices {
            let record = place.nine_slice(&other.nine_slices[i as usize]);
            let k = to(at.nine_slices, i);
            if log {
                let slot = &self.nine_slices[k];
                log_change(
                    &mut self.edits.nine_slices,
                    k,
                    slot,
                    &record,
                    nine_slice_change,
                );
            }
            self.nine_slices[k] = record;
        }
        for i in s.icons..e.icons {
            let record = place.icon(&other.icons[i as usize]);
            let k = to(at.icons, i);
            if log {
                let slot = &self.icons[k];
                log_change(&mut self.edits.icons, k, slot, &record, icon_change);
            }
            self.icons[k] = record;
        }
        #[cfg(feature = "phosphor-icons")]
        for i in s.icons_msdf..e.icons_msdf {
            let record = place.icon_msdf(other.icons_msdf[i as usize]);
            let k = to(at.icons_msdf, i);
            if log {
                let slot = &self.icons_msdf[k];
                log_change(
                    &mut self.edits.icons_msdf,
                    k,
                    slot,
                    &record,
                    icon_msdf_change,
                );
            }
            self.icons_msdf[k] = record;
        }
        for i in s.texts..e.texts {
            let record = place.text(&other.texts[i as usize]);
            let k = to(at.texts, i);
            if log {
                let slot = &self.texts[k];
                log_change(&mut self.edits.texts, k, slot, &record, text_change);
            }
            self.texts[k] = record;
        }
        for i in s.selectable..e.selectable {
            let (key, index) = other.selectable[i as usize];
            self.selectable[to(at.selectable, i)] = (key, index + at.texts);
        }
        for i in s.viewport_clips..e.viewport_clips {
            let Some(clip) = place.clip(Some(other.viewport_clips[i as usize])) else {
                return false;
            };
            self.viewport_clips[to(at.viewport_clips, i)] = clip;
        }
        true
    }
}
