//! The timeline's data and its pure logic: tracks, keys and markers, the rows
//! a frame shows, the edits it makes, and the ruler's arithmetic. No drawing,
//! so all of it tests without a [`DrawList`](crate::DrawList).

use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// How a key eases into the next one. Drawn as its shape: a diamond, a
/// circle, a square.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interp {
    /// Eased in and out (a diamond).
    #[default]
    Ease,
    /// Straight to the next key (a circle).
    Linear,
    /// Holds its value until the next key (a square, with a bar to the next
    /// key).
    Hold,
}

/// One key on a track: a frame and how it eases on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keyframe {
    /// The frame.
    pub t: u32,
    /// How it eases into the next key.
    pub interp: Interp,
}

impl Keyframe {
    /// An eased key at frame `t`.
    pub const fn new(t: u32) -> Self {
        Self {
            t,
            interp: Interp::Ease,
        }
    }

    /// Set how it eases.
    pub const fn interp(mut self, interp: Interp) -> Self {
        self.interp = interp;
        self
    }
}

/// Identity of a track: stable across frames and edits.
pub type TrackId = u64;

/// One row of the dope sheet: a property's keys, or a group heading that
/// sums up the tracks nested under it.
///
/// Tracks are a flat list in display order; nesting comes from
/// [`depth`](Self::depth): a group owns the tracks after it that are deeper
/// than it. A group with no keys of its own is a **summary**: it shows the
/// union of its tracks' keys, and selecting or dragging one of those moves
/// the real keys beneath.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Stable identity.
    pub id: TrackId,
    /// The name in the label column.
    pub label: String,
    /// Nesting depth; 0 at the top.
    pub depth: u32,
    /// A group heading (collapsible; sums up its tracks when it has no keys
    /// of its own).
    pub group: bool,
    /// A colour swatch beside the label.
    pub color: Option<[f32; 4]>,
    /// Muted: drawn dim (the host skips it on playback).
    pub muted: bool,
    /// Locked: its keys can't be moved, added or deleted.
    pub locked: bool,
    /// Its keys, sorted by frame, at most one per frame. The timeline keeps
    /// them so.
    pub keys: Vec<Keyframe>,
}

impl Track {
    /// A track `id` named `label`, with no keys.
    pub fn new(id: TrackId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            depth: 0,
            group: false,
            color: None,
            muted: false,
            locked: false,
            keys: Vec::new(),
        }
    }

    /// A group heading `id` named `label`.
    pub fn group(id: TrackId, label: impl Into<String>) -> Self {
        Self {
            group: true,
            ..Self::new(id, label)
        }
    }

    /// Nest at `depth`.
    pub fn depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// A colour swatch beside the label.
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.color = Some(color);
        self
    }

    /// Mute the track.
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Lock the track.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// Set the keys; they are sorted and de-duplicated by frame (the last of
    /// a frame wins).
    pub fn keys(mut self, keys: impl IntoIterator<Item = Keyframe>) -> Self {
        self.keys = keys.into_iter().collect();
        normalize_keys(&mut self.keys);
        self
    }

    /// The key at frame `t`.
    pub fn key_at(&self, t: u32) -> Option<&Keyframe> {
        self.keys
            .binary_search_by_key(&t, |k| k.t)
            .ok()
            .map(|i| &self.keys[i])
    }
}

/// Sort keys by frame and keep the last of each frame.
pub(crate) fn normalize_keys(keys: &mut Vec<Keyframe>) {
    keys.reverse();
    keys.sort_by_key(|k| k.t);
    keys.dedup_by_key(|k| k.t);
}

/// A named point in time on the ruler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    /// The frame.
    pub t: u32,
    /// The name on its flag.
    pub label: String,
}

impl Marker {
    /// A marker `label` at frame `t`.
    pub fn new(t: u32, label: impl Into<String>) -> Self {
        Self {
            t,
            label: label.into(),
        }
    }
}

/// A key by where it is: its track and frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyRef {
    /// The track.
    pub track: TrackId,
    /// The frame.
    pub t: u32,
}

impl KeyRef {
    /// The key on `track` at frame `t`.
    pub const fn new(track: TrackId, t: u32) -> Self {
        Self { track, t }
    }
}

/// One change the timeline made to the data it was given, with what an undo
/// needs to reverse it.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEdit {
    /// `keys` (where they were) moved by `delta` frames; `replaced` are the
    /// keys they landed on, which were removed.
    KeysMoved {
        /// The frames moved.
        delta: i64,
        /// The moved keys, at their frames before the move.
        keys: Vec<KeyRef>,
        /// Keys overwritten where the moved ones landed.
        replaced: Vec<(TrackId, Keyframe)>,
    },
    /// These keys were deleted.
    KeysDeleted {
        /// The deleted keys, with their tracks.
        keys: Vec<(TrackId, Keyframe)>,
    },
    /// An eased key was added.
    KeyAdded {
        /// Where.
        key: KeyRef,
    },
    /// A track was muted or unmuted.
    TrackMuted {
        /// The track.
        track: TrackId,
        /// Its new state.
        muted: bool,
    },
    /// A track was locked or unlocked.
    TrackLocked {
        /// The track.
        track: TrackId,
        /// Its new state.
        locked: bool,
    },
    /// A marker was appended at `index`.
    MarkerAdded {
        /// Its index.
        index: usize,
    },
    /// Marker `index` moved.
    MarkerMoved {
        /// Its index.
        index: usize,
        /// Its frame before.
        from: u32,
        /// Its frame now.
        to: u32,
    },
    /// Marker `index` was renamed.
    MarkerRenamed {
        /// Its index.
        index: usize,
        /// Its name before.
        from: String,
        /// Its name now.
        to: String,
    },
    /// Marker `index` was removed.
    MarkerDeleted {
        /// Its index before removal.
        index: usize,
        /// The marker.
        marker: Marker,
    },
    /// The play range changed.
    RangeChanged {
        /// The range before.
        from: [u32; 2],
        /// The range now.
        to: [u32; 2],
    },
}

/// A row the frame shows: a track that isn't folded away under a collapsed
/// group.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// Index into the tracks.
    pub track: usize,
    /// A summary group: its keys are `times`, the union of its tracks'.
    pub summary: bool,
    /// A collapsed group: its tracks have no rows.
    pub collapsed: bool,
    /// The tracks nested under a group (groups among them are skipped).
    pub kids: Range<usize>,
    /// A summary's frames, as a range of [`Rows::times`].
    pub times: Range<usize>,
}

/// The rows of one frame, rebuilt each frame into reused buffers.
#[derive(Clone, Debug, Default)]
pub(crate) struct Rows {
    pub rows: Vec<Row>,
    /// Every summary's frames, sorted and unique per summary.
    pub times: Vec<u32>,
    /// Track index by id.
    pub index: HashMap<TrackId, usize>,
}

impl Rows {
    /// Rebuild from `tracks`, folding away what `collapsed` groups hide.
    pub fn build(&mut self, tracks: &[Track], collapsed: &HashSet<TrackId>) {
        self.rows.clear();
        self.times.clear();
        self.index.clear();
        for (i, track) in tracks.iter().enumerate() {
            self.index.insert(track.id, i);
        }
        let mut hide_below: Option<u32> = None;
        for (i, track) in tracks.iter().enumerate() {
            let d = track.depth;
            if let Some(h) = hide_below {
                if d > h {
                    continue;
                }
                hide_below = None;
            }
            let mut row = Row {
                track: i,
                summary: false,
                collapsed: track.group && collapsed.contains(&track.id),
                kids: i + 1..i + 1,
                times: 0..0,
            };
            if track.group {
                let end = tracks[i + 1..]
                    .iter()
                    .position(|t| t.depth <= d)
                    .map_or(tracks.len(), |p| i + 1 + p);
                row.kids = i + 1..end;
                if track.keys.is_empty() {
                    row.summary = true;
                    let start = self.times.len();
                    for kid in &tracks[row.kids.clone()] {
                        if !kid.group {
                            self.times.extend(kid.keys.iter().map(|k| k.t));
                        }
                    }
                    let tail = &mut self.times[start..];
                    tail.sort_unstable();
                    let mut w = 0;
                    for r in 0..tail.len() {
                        if w == 0 || tail[r] != tail[w - 1] {
                            tail[w] = tail[r];
                            w += 1;
                        }
                    }
                    self.times.truncate(start + w);
                    row.times = start..start + w;
                }
            }
            if row.collapsed {
                hide_below = Some(d);
            }
            self.rows.push(row);
        }
    }

    /// The frames row `row` shows keys at.
    pub fn row_times<'t>(&'t self, row: &Row, tracks: &'t [Track]) -> RowTimes<'t> {
        if row.summary {
            RowTimes::Summary(&self.times[row.times.clone()])
        } else {
            RowTimes::Keys(&tracks[row.track].keys)
        }
    }

    /// The real keys behind a summary row's frame `t` (or the row's own key).
    pub fn keys_at(&self, row: &Row, tracks: &[Track], t: u32, out: &mut Vec<KeyRef>) {
        out.clear();
        if row.summary {
            for kid in &tracks[row.kids.clone()] {
                if !kid.group && kid.key_at(t).is_some() {
                    out.push(KeyRef::new(kid.id, t));
                }
            }
        } else {
            out.push(KeyRef::new(tracks[row.track].id, t));
        }
    }

    /// True when `id`'s track is locked (or gone).
    pub fn is_locked(&self, tracks: &[Track], id: TrackId) -> bool {
        self.index.get(&id).is_none_or(|&i| tracks[i].locked)
    }
}

/// A row's key frames: a track's own keys or a summary's frames.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RowTimes<'t> {
    Keys(&'t [Keyframe]),
    Summary(&'t [u32]),
}

impl RowTimes<'_> {
    pub fn len(&self) -> usize {
        match self {
            RowTimes::Keys(k) => k.len(),
            RowTimes::Summary(t) => t.len(),
        }
    }

    pub fn t(&self, i: usize) -> u32 {
        match self {
            RowTimes::Keys(k) => k[i].t,
            RowTimes::Summary(t) => t[i],
        }
    }

    pub fn interp(&self, i: usize) -> Interp {
        match self {
            RowTimes::Keys(k) => k[i].interp,
            RowTimes::Summary(_) => Interp::Ease,
        }
    }

    /// The index of the first frame at or after `t`.
    pub fn lower_bound(&self, t: u32) -> usize {
        match self {
            RowTimes::Keys(k) => k.partition_point(|k| k.t < t),
            RowTimes::Summary(s) => s.partition_point(|&x| x < t),
        }
    }
}

/// Move the `selected` keys by `delta` frames, in place. Keys on locked
/// tracks stay. A moved key replaces any key it lands on. Updates `selected`
/// to the keys' new frames and returns the edit (`None` when nothing moved).
pub(crate) fn move_keys(
    tracks: &mut [Track],
    selected: &mut HashSet<KeyRef>,
    delta: i64,
) -> Option<TimelineEdit> {
    if delta == 0 {
        return None;
    }
    let mut moved: Vec<KeyRef> = selected
        .iter()
        .copied()
        .filter(|k| {
            tracks
                .iter()
                .find(|t| t.id == k.track)
                .is_some_and(|t| !t.locked && t.key_at(k.t).is_some())
        })
        .collect();
    if moved.is_empty() {
        return None;
    }
    moved.sort_unstable();
    let shift = |t: u32| (t as i64 + delta).max(0) as u32;
    let mut replaced = Vec::new();
    for track in tracks.iter_mut().filter(|t| !t.locked) {
        let id = track.id;
        let mine = |t: u32| moved.binary_search(&KeyRef::new(id, t)).is_ok();
        if !track.keys.iter().any(|k| mine(k.t)) {
            continue;
        }
        let mut lifted = Vec::new();
        track.keys.retain(|k| {
            if mine(k.t) {
                lifted.push(Keyframe {
                    t: shift(k.t),
                    ..*k
                });
                false
            } else {
                true
            }
        });
        track.keys.retain(|k| {
            let landed_on = lifted.iter().any(|m| m.t == k.t);
            if landed_on {
                replaced.push((id, *k));
            }
            !landed_on
        });
        track.keys.extend(lifted);
        track.keys.sort_by_key(|k| k.t);
    }
    for k in &moved {
        selected.remove(k);
    }
    for k in &moved {
        selected.insert(KeyRef::new(k.track, shift(k.t)));
    }
    Some(TimelineEdit::KeysMoved {
        delta,
        keys: moved,
        replaced,
    })
}

/// Delete the `selected` keys not on locked tracks, in place. What stays
/// selected is what was locked.
pub(crate) fn delete_keys(
    tracks: &mut [Track],
    selected: &mut HashSet<KeyRef>,
) -> Option<TimelineEdit> {
    let mut deleted = Vec::new();
    for track in tracks.iter_mut().filter(|t| !t.locked) {
        let id = track.id;
        track.keys.retain(|k| {
            let gone = selected.contains(&KeyRef::new(id, k.t));
            if gone {
                deleted.push((id, *k));
            }
            !gone
        });
    }
    for (id, k) in &deleted {
        selected.remove(&KeyRef::new(*id, k.t));
    }
    (!deleted.is_empty()).then_some(TimelineEdit::KeysDeleted { keys: deleted })
}

/// Add an eased key on track `index` at `t` unless one is there (or the
/// track is locked or a summary).
pub(crate) fn add_key(tracks: &mut [Track], index: usize, t: u32) -> Option<TimelineEdit> {
    let track = &mut tracks[index];
    if track.locked || (track.group && track.keys.is_empty()) {
        return None;
    }
    match track.keys.binary_search_by_key(&t, |k| k.t) {
        Ok(_) => None,
        Err(at) => {
            track.keys.insert(at, Keyframe::new(t));
            Some(TimelineEdit::KeyAdded {
                key: KeyRef::new(track.id, t),
            })
        }
    }
}

/// The last key frame before `frame` on any track.
pub(crate) fn prev_key(tracks: &[Track], frame: u32) -> Option<u32> {
    tracks
        .iter()
        .filter_map(|t| {
            let i = t.keys.partition_point(|k| k.t < frame);
            i.checked_sub(1).map(|i| t.keys[i].t)
        })
        .max()
}

/// The first key frame after `frame` on any track.
pub(crate) fn next_key(tracks: &[Track], frame: u32) -> Option<u32> {
    tracks
        .iter()
        .filter_map(|t| {
            let i = t.keys.partition_point(|k| k.t <= frame);
            t.keys.get(i).map(|k| k.t)
        })
        .min()
}

/// Every key on every track.
pub(crate) fn all_keys(tracks: &[Track]) -> HashSet<KeyRef> {
    tracks
        .iter()
        .flat_map(|t| t.keys.iter().map(move |k| KeyRef::new(t.id, k.t)))
        .collect()
}

/// Ruler steps, in frames.
pub(crate) const STEPS: [u32; 12] = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000];

/// The ruler's tick steps at `ppf` px per frame: labelled majors at least
/// 56 px apart, minors (dividing the major) at least 7 px apart, and fine
/// grid lines every frame once frames are 3 px wide.
pub(crate) fn tick_steps(ppf: f32) -> [u32; 3] {
    let major = STEPS
        .into_iter()
        .find(|&s| s as f32 * ppf >= 56.0)
        .unwrap_or(STEPS[STEPS.len() - 1]);
    let minor = STEPS
        .into_iter()
        .find(|&s| major % s == 0 && s as f32 * ppf >= 7.0)
        .unwrap_or(major);
    let fine = if ppf >= 3.0 { 1 } else { minor };
    [major, minor, fine]
}

/// `frame` at `fps` as `mm:ss:ff`.
pub fn timecode(frame: u32, fps: u32) -> String {
    let fps = fps.max(1);
    let s = frame / fps;
    format!("{:02}:{:02}:{:02}", s / 60, s % 60, frame % fps)
}
