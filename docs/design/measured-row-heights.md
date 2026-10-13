# Measured row heights for `List`

**Status:** In progress
**Last updated:** 2026-10-13

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

(nothing yet)

### Outstanding

- [ ] `RowHeights`: per-row heights, estimated or measured, with O(log n) offsets and lookups (built, uncommitted)
- [ ] `RowHeights`: index layouts compared at scale; keep the faster (built, uncommitted: the double-ended Fenwick won)
- [ ] `RowHeights`: heights remembered by row key across rebuilds, forgotten when the width or fonts change (built, uncommitted)
- [ ] `RowHeights`: rows still estimated, nearest a given row first (for an app's background measuring) (built, uncommitted)
- [ ] `ListState`: scroll position kept as (row, distance into it), with the pixel offset derived each frame (built, uncommitted: fixed heights; `scroll` is private, with `offset`, `top_row`, `scroll_to_row`, `glide_to_row`, `reveal_item`, `is_gliding`)
- [ ] `ListState`: follow the end (stay at the bottom as rows arrive, until the reader scrolls up) (built, uncommitted: `follow_end`, and `List::stick_to_end` to pick it up again at the bottom)
- [ ] `List`: draw from a `RowHeights` (one column), measuring rows in view through a callback (built, uncommitted: `List::draw_measured`)
- [ ] `List`: space kept free above the first row and below the last (padding inside the scroll) (built, uncommitted: `List::with_padding`, fixed and measured)
- [ ] `List`: keyboard cursor, click and hover on rows of different heights (built, uncommitted)
- [ ] Rows placed in `f64` all the way to the screen (deferred: `ScrollView` offsets are `f32`, so past about 4M px of content rows land on half pixels, past 16M on whole ones; fine for now, revisit if a transcript gets that long)
- [ ] agent-ui: `TranscriptLayout` keeps its heights in a `RowHeights`
- [ ] agent-ui: `TranscriptPane` scrolls by the anchored `ListState`; `Held` / `hold` / `reading` go
- [ ] gameui `GroupList` (its own running total of row heights in `GroupLayout`) onto `RowHeights` (deferred: after the transcript move)
- [ ] Gallery entry: a list of rows of different heights (deferred: after the agent-ui move shows the API works)

## Why this exists

gameui's `List` (`src/widgets/list.rs`) only handles rows that are all one height
(`with_item_height`). Where a row is is then a multiplication, and so is which row is
under the pointer. Anything with rows of different heights has to do its own scrolling.
agent-ui's transcript does exactly that. In `crates/desktop/src/transcript_layout.rs` it keeps:

- a rough height per row, replaced by the measured one once the row is measured, either on
  screen or by a background thread;
- a running total of heights, so it can find where a row starts;
- measured heights remembered by row content, so they outlive a reload.

In `transcript_pane.rs` it keeps "the row the reader is looking at stays still while heights above
it change" (`reading`, `hold`, `Held`).

The pane does that by correcting a pixel scroll offset every frame. The offset can be hundreds of
thousands of pixels, where an `f32` can only hold about 1/32 of a pixel. So each correction rounds a
little, and `Held` exists to stop those roundings from walking the view off its row.

gpui (`zed/crates/gpui/src/elements/list.rs`) keeps a list's position as "item N, this far into it"
(`ListOffset { item_ix, offset_in_item }`) instead of as a pixel offset. When rows above change
height, nothing has to move, because the position never referred to them.

This doc is the contract for bringing that to gameui, split the way Bart suggested on 2026-10-13:

- `List` stays the one list widget, and only asks where a row starts and how tall it is.
- A separate helper, `RowHeights`, answers those questions from measured content.

agent-ui's transcript then moves onto both.

## Goals

- `List` can draw rows of different heights. It keeps everything it does today: scrolling,
  drawing only the rows in view, selection, the keyboard cursor, hover, and disabled rows.
- The scroll position is (row, distance into it). Heights changing above the view never move what
  the reader sees, and there is no per-frame correction to round off.
- Follow the end: a list can stay at its bottom as rows arrive, and stops when the reader scrolls
  up. It starts again when the reader scrolls back to the bottom.
- `RowHeights` handles each change in O(log n) time, where n is the number of rows. This covers
  finding where a row starts, finding the row at a given position, and setting one row's height.
  Work per frame grows with the rows in view, not with the history.
- The app decides how rows are measured. The list measures rows on screen through a callback. Rows
  out of view are measured by whoever the app chooses (agent-ui keeps its `HeightWorker`), and the
  results are handed in.
- agent-ui's transcript loses its own height bookkeeping and scroll anchoring, with no visible
  change in behaviour.

## Non-goals

- Grids with rows of different heights. A measured list has one column. `columns(n > 1)` with a
  `RowHeights` is a programming error (a debug assertion), not a layout.
- Background measuring inside gameui. No threads in the widget: an app that wants rows measured off
  the UI thread does it, as agent-ui does.
- Sideways scrolling, and row widths that differ from the list's width.
- Making fixed-height lists slower or more complicated to use. `with_item_height` keeps working,
  with no `RowHeights` needed.

## Design

### `RowHeights` (new, `src/widgets/row_heights.rs`)

An app-owned value, one per list, like `ListState`. It holds, for each row:

- `height: f32`. Zero is allowed: agent-ui hides a tool result that is drawn inside its call's row.
- `measured: bool`: whether the height is a measurement or still an estimate.
- an optional `key: u64`: the identity of the row's content, used to remember its height.

Offsets come from a Fenwick tree over the heights. A Fenwick tree is an array that stores partial
sums, so "total height of rows 0..i" and "change row i's height" each take about log₂ n steps.
"Which row covers y" is a walk down the same tree, also O(log n). agent-ui's layout uses the same
structure today, but only for corrections. It keeps append-only base offsets plus the tree, and
finds the row at y with a binary search that calls `offset_at` (O(log² n)). One tree over the
heights is simpler and does both jobs.

API sketch (names to be settled in review):

```rust
impl RowHeights {
    pub fn new() -> Self;
    pub fn len(&self) -> usize;
    /// Add rows at the end with these estimates (or remembered heights, by key).
    pub fn push(&mut self, rows: impl IntoIterator<Item = Row>);
    /// Replace rows `range` with `rows` (prepending a page is `splice(0..0, page)`).
    pub fn splice(&mut self, range: Range<usize>, rows: impl IntoIterator<Item = Row>);
    pub fn set_measured(&mut self, row: usize, height: f32);
    /// Heights measured elsewhere, by key; rows not shown now are remembered for later.
    pub fn apply_by_key(&mut self, heights: impl IntoIterator<Item = (u64, f32)>);
    // 2026-10-13: replaced by `id_of(row) -> RowId` and
    // `apply_measured(id, key, height)`; see the note below.
    /// The width or fonts changed: every row is an estimate again and remembered heights go.
    pub fn reestimate(&mut self, estimate: impl FnMut(usize) -> f32);
    pub fn offset_of(&self, row: usize) -> f32;
    pub fn height_of(&self, row: usize) -> f32;
    pub fn is_measured(&self, row: usize) -> bool;
    pub fn total(&self) -> f32;
    /// The row covering `y`, skipping zero-height rows.
    pub fn row_at(&self, y: f32) -> Option<usize>;
    /// Rows still estimated, nearest `row` first.
    pub fn unmeasured_near(&self, row: usize) -> impl Iterator<Item = usize>;
}
```

`push` and `set_measured` are O(log n) per row. `splice` anywhere but the end rebuilds the tree in
O(n). That happens when a page is prepended or a conversation is swapped, not every frame, and
agent-ui already rebuilds its layout then (`TranscriptPane::prepend` calls `layout.clear()`).

> **2026-10-13 update:** putting rows in front is O(log n) per row too (see Open question 2).
> Heights measured elsewhere no longer go through a key → row table. Filing every row's key on load
> cost more than the tree itself (0.4 ms of 0.9 ms for 50k rows).
>
> Instead, each row has a `RowId`: a number that stays its own while rows are added at either end
> or replaced. A background job takes `id_of(row)` and `key_of(row)` with each row it measures, and
> hands them back with `apply_measured(id, key, height)`. The height applies when that id still
> holds a row with that key, and otherwise is remembered by key for when the row comes back.
> Clearing or any rebuild numbers rows from a fresh range, so an old id finds no row.

> **2026-10-13 update (review):** the id and key travel together with the layout they were
> measured for, as a `Ticket` (`heights.ticket(index)`, then `apply_measured(ticket, height)`): a
> height measured off-thread before a resize or a font change is dropped rather than taken as a
> measurement at the new width. Ids are also unique across every `RowHeights` in the process, so
> a `ListState` moved to another list finds no row by an old id.

### Anchored scroll position (`ListState`)

`ListState` gains the reader's position as `(row, into)`: the row at the top of the view, and how
many pixels of it are above the top edge. It also gains the same pair for the glide's target, and a
`follow_end` flag. `ScrollState.offset` and `target` remain, but are worked out from those pairs at
the start of each draw: `offset = heights.offset_of(row) + into`. `ScrollView` then glides, takes
the wheel and drags the bar in pixels as it does now. After `ScrollView::begin`, the list converts
the moved offset back into `(row, into)` with `row_at`.

The only place rounding can creep in is the one conversion in a frame where the reader actually
scrolled, and that conversion is what the reader asked for. Frames where only heights changed do
not convert at all.

Moving the view from code works like gpui:

- `scroll_to_row(row, into)` jumps;
- `glide_to_row(row, into)` glides;
- `follow_end(true)`.

A fixed-height list uses the same anchor, with offsets from a multiplication, so there is one code
path, not two.

When every row is estimated again (the width changed), keeping the same pixel distance into the top
row can point past its new end. gpui keeps the same fraction of the row in that case
(`ScrollAnchor::Proportional`), and so will this.

### `List` with a `RowHeights`

```rust
List::new()
    .with_heights(&mut heights, |row, width, list| measure(row, width, list))
    .draw(rect, count, &mut state, list, style, input, |list, cell, item| { … })
```

Each draw, in order:

1. Work out the offset from the anchor.
2. Measure the rows in view plus a few either side, which is what agent-ui's `measure_visible`
   does. Measuring a row above the anchor changes nothing on screen, because the anchor is the row
   itself.
3. Let `ScrollView` take input.
4. Convert back to the anchor if the reader scrolled.
5. Draw the visible rows at `offset_of(row)`.

Hit testing uses `row_at`. The keyboard cursor brings its row into view with `offset_of` /
`height_of` in place of `index × pitch`.

> **2026-10-13 update:** built as a separate entry point rather than a builder, since the
> measure callback and the `&mut RowHeights` are per draw: `List::draw_measured(rect, &mut heights,
> |row, width, list| height, &mut state, list, style, input, |list, cell, item| …)`. Rows are
> measured, and drawn, at the width beside a docked scrollbar whether or not it shows, so the bar
> appearing changes no height. In a measured list the anchor also keeps its row's `RowId` and finds
> the row by it at each draw, so a page put in front needs nothing from the app. A new width keeps
> the top row's share (`RowHeights::set_width`, called by the list), and a row of no height is not
> drawn. A fresh `ListState` starts at the very top, top padding included.

`with_padding(top, bottom)` keeps space above the first row and below the last inside the scroll.
agent-ui's `Pad` needs this; today it converts between "scroll space" and "card space" in every
call (`content_scroll`, `scroll_at`).

### agent-ui's transcript, after

- `TranscriptLayout` keeps what is the transcript's own:
  - row plans and their cache window;
  - the estimates (`transcript_cards::estimate_row_height`);
  - the row keys (`row_heights::row_key`);
  - jobs for the `HeightWorker`.

  Its `heights`, `base_offsets`, `base_total`, `corrections`, `exact`, `known`, `rows_by_key`,
  `visible`, `upper_bound`, `lower_bound`, `row_at` and `offset_at` are replaced by a `RowHeights`.
- `TranscriptPane` stores a `ListState` (or its anchor alone, if the pane keeps its own card
  drawing; see Open questions) in place of `scroll`, `follow_tail`, `anchor` and `held`. `reading`,
  `hold`, `top_row_even_following`, `content_scroll` and `scroll_at` go.
- Behaviour the existing tests pin keeps passing: `transcript_pane_tests.rs`,
  `transcript_updates_tests.rs` and the layout tests about the view staying put.

## Phasing

1. `RowHeights` with unit tests, first written as the behaviour we want:
   - offsets and `row_at`, including zero-height rows;
   - `splice` at the front and in the middle;
   - remembered heights by key, and forgetting them on `reestimate`;
   - `unmeasured_near` order;
   - a 100k-row scale check that counts tree steps, not time.
2. The anchored `ListState` and follow-the-end, with fixed heights. Every existing `List` test still
   passes.
3. `List::with_heights`, padding, and the variable-height cursor and hit testing. Tests:
   - rows above the view get taller and nothing on screen moves;
   - a page prepended keeps the reader's row;
   - follow-the-end holds while rows arrive and lets go on a scroll up;
   - a glide in flight keeps going while heights change;
   - the keyboard cursor scrolls a tall row into view.
4. agent-ui `TranscriptLayout` onto `RowHeights`, behind the pane's current scrolling.
5. agent-ui `TranscriptPane` onto the anchored scroll; delete `Held` and its helpers.

## Open questions for review

1. **Does the transcript draw through `List`, or only use its parts?** The pane draws cards with
   selectable text, raw panels, image tiles and sideways-scrolled tables, and has no use for
   `List`'s selection or row backgrounds.
   - (a) Draw through `List` with `SelectionMode::None`. One scrolling code path, and the pane
     drops its own `ScrollView` handling.
   - (b) Keep the pane's own `ScrollView`, and use `RowHeights` plus a small public "anchored
     scroll" type that `ListState` also uses. Less churn in agent-ui, but two drawing loops.

   Leaning (a) if `List`'s item callback can give the pane everything it needs (the content rect
   and row index); otherwise (b).

   > **2026-10-13, Bart:** (a), draw through `List`. If it falls short, adjust `List` rather than
   > agent-ui where possible.
2. **Fenwick tree or gpui's sum tree?** gpui keeps items in a B-tree with per-node summaries, so
   inserting in the middle is O(log n). A Fenwick tree rebuilds in O(n) on any insert that isn't at
   the end. For agent-ui, inserts other than appends happen when a page is prepended, about once a
   scroll to the top, over thousands of rows. Leaning Fenwick: far simpler, and fast enough at that
   scale.

   > **2026-10-13, Bart:** measure first; trying both costs little. Build both behind the same
   > `RowHeights` API, measure them with an ignored, print-only test at agent-ui's scale (appends,
   > prepended pages, a height changed per frame, `row_at` per frame), and keep the one that wins.
   >
   > **2026-10-13 update:** measured (`row_heights_tests.rs`, `compare_index_layouts`, release,
   > 50k rows). A chunked layout (rows in blocks of 256 with per-block totals) stood in for gpui's
   > B-tree. A single Fenwick tree took 0.42 ms to put a 500-row page in front; chunked took
   > 0.012 ms, but was slower per frame. Bart asked for the best of both, so a third layout was
   > built: two Fenwick trees back to back, one growing down for appended rows and one growing up
   > for rows put in front, plus a one-pass build for loading a whole list. The third layout won:
   >
   > - loading 50k rows: 0.2 ms;
   > - a page in front: 6 µs;
   > - a frame: 0.3 µs.
   >
   > Only a change in the middle that adds or removes rows still rebuilds. The chunked layout was
   > deleted. Through the whole `RowHeights` (keys included), loading takes 0.35 ms, a page in
   > front 7 µs, and a frame 0.3 µs.
3. **Should a measured list measure in the item callback instead of a separate one?** gpui learns a
   row's height by laying it out, and immediate-mode drawing could do the same: the item callback
   returns the height it drew. But then a row's place isn't known until the row above it has been
   drawn, and the first frame after an estimate turns out wrong draws rows at stale positions.
   Leaning a separate measure callback, as agent-ui's `measure_visible` does now.

   > **2026-10-13, Bart:** a separate measure callback.

## References

- gpui's list: `~/Projects/gpuix/zed/crates/gpui/src/elements/list.rs` (`ListState`,
  `ListOffset`, `PendingScroll`, `FollowState`, `remeasure_items`, `splice`)
- `src/widgets/list.rs`, `src/widgets/scroll_view.rs` (`ScrollState`)
- agent-ui: `crates/desktop/src/transcript_layout.rs`, `transcript_pane.rs` (`Held`, `reading`,
  `hold`), `row_heights.rs` (`HeightWorker`, `row_key`)
- `TODO.md`, "Learned from gpui": "`List` with measured row heights and an item-anchored scroll
  position"
