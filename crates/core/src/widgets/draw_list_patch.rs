//! Patching a list placed into another in place: when the list placed is
//! recorded again with the same shape (the same paint runs over the same
//! number of records), the records [`DrawList::append_translated`] put in the
//! other list are overwritten where they are, rather than the other list being
//! recorded again. A retained tree uses this to carry a change to a deep node
//! up through its ancestors' lists in the work of the records that changed.

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

/// The shape of a list: how many records of each kind it holds and the paint
/// runs over them, and what else placing it adds to another list (its repaint
/// request, its count of degenerate calls dropped), but not what the records
/// hold. Two lists of one shape placed the same way leave the other list's
/// paint runs as they were, so one can be patched over the other
/// ([`DrawList::patch_translated`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListShape {
    counts: RecordCounts,
    paint_cmds: Vec<PaintCmd>,
    soup_committed: u32,
    next_repaint: Option<u32>,
    dropped_degenerate: u32,
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

    /// Write the list's shape into `out`, reusing its buffer.
    pub fn shape_into(&self, out: &mut ListShape) {
        out.counts = self.record_counts();
        out.paint_cmds.clone_from(&self.paint_cmds);
        out.soup_committed = self.soup_committed_indices;
        out.next_repaint = self.next_repaint.map(f32::to_bits);
        out.dropped_degenerate = self.dropped_degenerate;
    }

    /// Whether the list has the shape `shape` (see [`ListShape`]).
    pub fn has_shape(&self, shape: &ListShape) -> bool {
        self.record_counts() == shape.counts
            && self.soup_committed_indices == shape.soup_committed
            && self.next_repaint.map(f32::to_bits) == shape.next_repaint
            && self.dropped_degenerate == shape.dropped_degenerate
            && self.paint_cmds == shape.paint_cmds
    }

    /// Overwrite the records of `span` of `other`, which was placed into this
    /// list as `placed` says, with `other`'s records there now, placed the
    /// same way: the list [`append_translated`](Self::append_translated)
    /// would have made had `other` held them then.
    ///
    /// `other` must have the shape it had when placed ([`ListShape`]; the
    /// caller checks that, since it alone saw the list before), and this list
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
        let (s, e, at) = (span.start, span.end, placed.at);
        let to = |base: u32, i: u32| (base + i) as usize;
        for i in s.vertices..e.vertices {
            self.vertices[to(at.vertices, i)] = place.vertex(other.vertices[i as usize]);
        }
        for i in s.indices..e.indices {
            self.indices[to(at.indices, i)] = other.indices[i as usize] + at.vertices;
        }
        for i in s.analytic..e.analytic {
            let Some(record) = place.analytic(&other.analytic_instances[i as usize]) else {
                return false;
            };
            let slot = &mut self.analytic_instances[to(at.analytic, i)];
            if slot.kind != record.kind {
                return false;
            }
            *slot = record;
        }
        for i in s.circles..e.circles {
            self.circle_instances[to(at.circles, i)] =
                place.circle(other.circle_instances[i as usize]);
        }
        for i in s.nine_slices..e.nine_slices {
            self.nine_slices[to(at.nine_slices, i)] =
                place.nine_slice(&other.nine_slices[i as usize]);
        }
        for i in s.icons..e.icons {
            self.icons[to(at.icons, i)] = place.icon(&other.icons[i as usize]);
        }
        #[cfg(feature = "phosphor-icons")]
        for i in s.icons_msdf..e.icons_msdf {
            self.icons_msdf[to(at.icons_msdf, i)] = place.icon_msdf(other.icons_msdf[i as usize]);
        }
        for i in s.texts..e.texts {
            self.texts[to(at.texts, i)] = place.text(&other.texts[i as usize]);
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
