//! A list edited in place, saying what changed: an owner that keeps one
//! list and patches it ([`DrawList::patch_translated`]) knows which records
//! it overwrote, so the renderer need neither compare the list with the last
//! one it drew nor keep a copy of it to compare with
//! (`docs/design/frame-reuse.md`, "caller-supplied dirty flags").
//!
//! The list has a version: a stamp, new each time it is cleared, and the
//! patches made since. [`DrawList::begin_edits`] says the list as it stands
//! may be what the renderer drew last; each patch after it logs, for every
//! record it overwrote, how the record changed ([`Change`], judged as the
//! renderer's comparison judges it). The renderer, which remembers the version
//! it drew in each place, takes the log when it is relative to that version,
//! and plans the list from nothing otherwise.

use super::*;
#[cfg(feature = "phosphor-icons")]
pub(crate) use crate::render::icon_msdf_change;
pub(crate) use crate::render::{
    Change, analytic_change, circle_change, icon_change, nine_slice_change, same_vertex_box_inputs,
    text_change,
};

/// A list's contents at one moment: the stamp it was given when last cleared,
/// and the patches made since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ListVersion {
    pub(crate) stamp: u64,
    pub(crate) patches: u64,
}

/// What has been patched in a list since [`DrawList::begin_edits`]: each
/// record overwritten, by kind, with how it changed (a record overwritten
/// twice is in twice).
#[derive(Clone, Debug, Default)]
pub(crate) struct EditLog {
    /// The owner reports its edits ([`DrawList::report_edits`]).
    pub(crate) on: bool,
    /// Patches made since the list was last cleared.
    pub(crate) patches: u64,
    /// The version the log is relative to; `None` until
    /// [`begin_edits`](DrawList::begin_edits), and again after a clear.
    pub(crate) base: Option<ListVersion>,
    pub(crate) analytic: Vec<(u32, Change)>,
    pub(crate) circles: Vec<(u32, Change)>,
    pub(crate) icons: Vec<(u32, Change)>,
    pub(crate) nine_slices: Vec<(u32, Change)>,
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icons_msdf: Vec<(u32, Change)>,
    pub(crate) texts: Vec<(u32, Change)>,
    /// The most a soup vertex or index overwritten changed by: an index, or
    /// a vertex's place or clip, is a [`Change::Place`]; its colour alone a
    /// [`Change::Look`]. The soup is reported whole at this.
    pub(crate) soup: Change,
}

impl EditLog {
    /// Forget what was logged, and what it was relative to.
    pub(crate) fn forget(&mut self) {
        self.base = None;
        self.analytic.clear();
        self.circles.clear();
        self.icons.clear();
        self.nine_slices.clear();
        #[cfg(feature = "phosphor-icons")]
        self.icons_msdf.clear();
        self.texts.clear();
        self.soup = Change::Same;
    }

    /// Whether patches are logged now.
    pub(crate) fn logging(&self) -> bool {
        self.on && self.base.is_some()
    }
}

/// Log record `at`'s change from `old` to `new` in `log`, when it changed.
pub(crate) fn log_change<T>(
    log: &mut Vec<(u32, Change)>,
    at: usize,
    old: &T,
    new: &T,
    change: impl Fn(&T, &T) -> Change,
) {
    let c = change(old, new);
    if c != Change::Same {
        log.push((at as u32, c));
    }
}

impl DrawList {
    /// Say whether this list's owner reports what it changes between renders
    /// ([`begin_edits`](Self::begin_edits)). A list that reports is never
    /// compared with the last one drawn or copied to compare the next with:
    /// the renderer takes what it was told instead, which costs it the
    /// records that changed rather than every record, and no copy of the
    /// frame.
    ///
    /// Between [`begin_edits`](Self::begin_edits) and the render, the owner
    /// changes the list only with [`patch_translated`](Self::patch_translated)
    /// or by clearing it; what is recorded or appended without a clear shows
    /// in the record counts, and makes the renderer plan the list from
    /// nothing. Writing the public record arrays directly is not seen, and
    /// draws a wrong frame. The setting survives [`clear`](Self::clear).
    pub fn report_edits(&mut self, on: bool) {
        self.edits.on = on;
        self.edits.forget();
    }

    /// Whether the owner reports its edits ([`report_edits`](Self::report_edits)).
    pub fn reports_edits(&self) -> bool {
        self.edits.on
    }

    /// The list as it stands may be what the renderer drew last: log against
    /// it what is patched from now on, forgetting what was logged before.
    /// Call it after a render and before the next patches; a list that
    /// does not report its edits ignores it.
    pub fn begin_edits(&mut self) {
        if !self.edits.on {
            return;
        }
        self.edits.forget();
        self.edits.base = Some(self.version());
    }

    /// The list's version: a stamp new at each clear, and the patches since.
    pub(crate) fn version(&self) -> ListVersion {
        ListVersion {
            stamp: self.stamp,
            patches: self.edits.patches,
        }
    }

    /// What was logged since [`begin_edits`](Self::begin_edits).
    pub(crate) fn edit_log(&self) -> &EditLog {
        &self.edits
    }
}
