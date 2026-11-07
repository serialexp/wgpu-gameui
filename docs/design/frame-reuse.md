# Frame-to-frame reuse

Status: partial — Phase 0 counters and headless oracle landed (2026-11-08); no reuse implemented
Owner: Bart
Last updated: 2026-11-08

## Implementation status

Tracking the gap between this design and what's on the main branch.
Tick items as they land.

### Done

- [x] **Scenes to judge it on.** The `one`, `scroll`, `churn` and `para` scenes and
      the `static` / `animated` grid in `bench/ui-compare` (gameui, egui, iced),
      so each phase below has a number to move (`feat(bench)`, 2026-11-08).
- [x] **Review fix: the culling viewport was divided by the scale twice.** Found by
      the review of this doc in the glyph culling committed the same day (text on
      screen at a scale above 1 was not placed); fixed with a test at scale 2
      (`fix(text)`, 2026-11-08).

- [x] **Phase 0 — counters.** `RenderStats` reports `text_blocks_looked_up`,
      `glyphs_placed`, `text_instances_built` and `boxes_computed` (a reuse phase is
      judged by these dropping, not by the clock); the `ui-compare` gameui harness
      prints them per scene.
- [x] **Phase 0 — headless oracle.** `render/frame_oracle.rs` plans sequences of lists
      (colour-only, insert/remove/reorder, empty to visible, shadow regrouping,
      scrolling across the edge, scale/origin/size changes, moving overlaps) on kept
      state and on state that has seen nothing, and requires the same plan,
      instances and draws; every plan is also checked against the brute-force
      order oracle. Shares fixtures with the plan tests (`draw_plan_support.rs`).

### Outstanding

- [ ] **Phase 0 — remainder.** The oracle's pixel level (GPU, `--ignored`) and the
      sequences it cannot reach headless: font change, texture replacement, atlas
      growth and compaction, several differing renders before one submit; the memory
      counter once there is a kept frame; the comparison-cost measurement at 100,
      1,000 and 10,000 cells that the budgets in "Goals" depend on.
- [ ] **Phase 1 — snapshot and the stage-wise comparison.** What a frame keeps, the
      context key, and a comparison that says *which stage's inputs* changed.
- [ ] **Phase 2 — whole-plan replay.** When every unit's key and box are as last
      frame, reuse order, sorted layout and batches (colour-only changes included).
- [ ] **Phase 3 — reuse draw boxes** for units whose records are unchanged.
- [ ] **Phase 4 — a prepared-text cache.** Position-independent, so a moved block
      (`scroll`, `para`) hits too; plus pointer-identity layout hits.
- [ ] **Phase 5 — retained GPU storage**, then partial upload and replay on top.
- [ ] **Phase 6 — partial order reuse** (prefix, then propagation), only if the
      scenes show it pays.
- [ ] **Phase 7 — cheaper list building** on the caller side.
- [ ] **Open questions** (below).

## Why this exists

Callers build a `DrawList` every frame, and that is the interface they want: a
tree of elements fed in immediate-mode style, with nothing to mark dirty. The
renderer, though, treats each list as brand new. For a frame where little or
nothing changed it still looks every string up in the layout cache (hashing it),
places every glyph, bounds every unit, orders them, sorts, copies into batch
order and uploads. The layout cache only stores shaping; everything after it
depends on position, colour, clip and scale, so nothing is kept.

The `ui-compare` scenes show the cost. On a warm `scroll` frame gameui spends
0.66 ms to egui's 0.25, and on `para` 0.39 to 0.13 ms, because egui reuses a
laid-out paragraph's glyph quads. In a 10k-cell `animated` grid nothing but
colour changes, yet bounds, order, sort and batching all run again.

This doc is the contract for making all the work after "the caller built a
list" cost only what changed, **without changing the interface**.

### What each stage is a function of

The first draft of this table said a unit's order depends on "earlier units
only" and listed too few inputs. Corrected after review:

| derived | depends on |
| --- | --- |
| a block's glyph instances | the block's fields (incl. clip, effects, range tint); its layout (so the font system's generation); the pixel grid (scale, view origin, snap, hint); **the target's logical size** (glyphs outside it are not placed); the atlas tiles and their field parameters; the **command it is in** (a command with any shadow or glow emits three sweeps across all its blocks, so a block's instances are not one range) |
| a unit's pixel box | an analytic, circle, sprite or nine-slice record plus the target; a **soup** unit also reads the shared vertices and indices it points at; a **text** unit reads the *prepared instances*, not the block; an unresolvable sprite has no box at all |
| a unit's batch key | its family, and for sprites and nine-slices the texture **binding** the texture store resolves (which can change with no change to the list) |
| a unit's order | its **own** key and boxes, and the key, boxes and orders of earlier units that overlap it: `order(B) = max(order(U) + (key(U) != key(B)))` over earlier overlapping `U`. Never colours or text, once their effects on keys, boxes and grouping are resolved |
| the sorted layout and batches | every unit's (order, key, **paint index**); a changed unit count shifts paint indices and every `UnitRef` index behind it |
| the concrete draw ranges | the above and **each unit's emitted instance count**: a label that gains a glyph shifts every later range of its family |
| the uploaded bytes and bindings | the instances in sorted layout, plus soup vertices and indices, plus the per-pass bindings (buffer, atlas bind group, uniform offset), which are not a function of the list at all |

Two consequences drive the design. A colour-only change leaves boxes, keys,
orders, sorted layout and batches untouched and changes bytes only (one
exception: a finite-to-NaN colour makes `draw_box::analytic` fall back to a
wider box, so the comparison must still look at finiteness). And a moved unit
changes its own box and the orders of later units overlapping its old or new
box.

## Goals

- The caller's interface does not change: build a `DrawList`, call `render`.
- Output identical to a full recompute, always. Reuse is an optimisation, never
  a behaviour, and the oracle (Phase 0) checks it independently at every stage
  (instance bytes, boxes, orders, draw ranges, bindings, pixels) over *sequences*
  of frames.
- A frame where nothing changed costs a comparison and a replay: no layout
  lookups, no bounds, no ordering, no upload.
- A frame where only colours changed costs the comparison plus an instance
  refresh; no bounds, ordering or sorting.
- A frame where everything changed costs no more than today. Budget, to be
  confirmed in Phase 0: within 5% at 100 and at 10,000 cells, measured as
  renderer time apart from list build and drop; the comparison abandons early
  by a rule that does not depend on the first units being the changed ones.
- Memory for the kept frame is bounded and reported (a counter), at most the
  size of the live frame's records and derived data again.

## Non-goals (v1)

- A retained element tree or caller-supplied dirty flags. They could be added
  later as hints that skip the comparison; correctness does not need them.
- Cheaper shaping of new strings (`churn`): the layout layer, a separate item.
- Multiple renders of one list into different targets beyond one kept frame per
  slot (below).

## Design

### The kept frame, and the context key

A kept frame is a snapshot: the records of each paint command (actual `Arc`s
for text content and range lists, so pointer identity is sound; never a
remembered address or cache slot, which can be recycled), and the derived data
(boxes, orders, planned units, batches, instance counts).

The context key holds everything outside the list the derived data depends on:
target size, scale and view origin, glyph snap and hint; a **font-system
generation** (bumped when fonts load or default families change: today that
clears the layout cache without any exposed counter); the MSDF atlas epoch; a
**texture-resolution epoch** for the sprite and nine-slice side. These are
distinct events and the doc should treat them so:

- MSDF atlas growth moves no tile, instances name texels, so instances stay
  valid (the bind group and texture are replaced and must be re-bound);
- sprite atlas growth *does* change instances (normalised UVs);
- sprite compaction moves regions; unload and reload can reuse ids; a
  nine-slice name can resolve to another sprite and border; texture contents
  (RGBA, video, external) can change with no change to any instance;
- the MSDF atlas does not evict today (it grows, then panics when full), so
  eviction is prospective.

A global epoch that invalidates everything on any of these defeats "one element
changed" when something unrelated loads, so sprite resolution is compared per
unit (the resolved binding and UVs go into the unit's comparison), with the
epoch only as a coarse guard.

### Comparing with last frame, stage by stage

The comparison does not answer "same or changed". Per unit it answers four
questions, because colour animation must not look like geometry change:

1. **Order inputs:** same key and same boxes (so same order, given the earlier
   units)?
2. **Box inputs:** same geometry, so the box can be kept?
3. **Instance bytes:** same bytes, so no re-upload of this unit?
4. **Topology:** same unit count, same command grouping, same emitted instance
   count?

Records are compared semantically, not by raw bytes: `AnalyticInstance`,
`CircleInstance` and soup vertices are plain data and compare as bytes, but
`IconDraw` holds a `String`, `TextBlock` has no equality today (28 fields,
owned spans, shared range vectors) and a soup command is a range into shared
vertices, so the *vertices it covers* are compared. Soup recorded after the
last paint command, and anything written straight into the public arrays, is
compared too (it bypasses the paint stream).

Positional alignment fails on an insertion (everything shifts), so a common
prefix and common suffix are matched. A matched suffix reuses source data only:
its `paint` indices and every `UnitRef` index shift, so planned units behind an
insertion are rebuilt, not copied. The first implementation may simply treat
any change in unit count or grouping as "topology changed" and recompute.

Abandoning early: the comparison keeps a running ratio of changed units over a
window and stops when it is high, so a list with a static header and an
animating body (the common game UI) is not compared in full and then recomputed
in full. Whether the stage-wise answers can be had for less than recompute is
measured in Phase 0, not assumed: 100 cells costs 0.07 ms in total.

### Phase 2 — whole-plan replay

When every unit's key and box are as last frame and the topology is the same,
`DrawOrder`, the sort and the batches would come out identical, so they are not
run. This is the only order reuse in the first version. It needs no grid
rollback and covers `static` (nothing changed) and `animated` (colour-only).

### Phase 4 — a prepared-text cache that survives a move

`scroll` and `para` move every block every frame, so a cache of "this unchanged
block's instances" never hits there. Offsetting last frame's instances by the
move is not sound either: which glyphs exist depends on the viewport, so a move
reveals glyphs the old range lacks; rounding is not translation-invariant
(`round(-0.5) + 1 != round(-0.5 + 1)`); and a stored origin plus axes need not
reproduce a rebuilt corner bit for bit. The cache is therefore
**position-independent**: per layout, each glyph's tile, relative position,
size and fill, so that selecting the visible lines and glyphs and placing them
(the cheap arithmetic) happens after the lookup. This is the "store the resolved
atlas tile on `ShapedGlyph`" item in `TODO.md`, taken further. A layout lookup
that hashes the whole string each frame can also skip the hash when the
`Arc<str>` is the one last frame used.

### Phase 5 — retained GPU storage

Partial upload is **not** possible on the current `FrameArena`: it is a
per-frame bump allocator, reset in `begin_frame`, so a "changed range" has no
resident old contents to patch; a grown buffer is a new, uninitialised one;
several lists (`render_layers`, one list's passes) write sequentially so a
retained range can be overwritten; and queue writes land at submit, so two
versions of one range encoded into one submission would both read the last
write. Text replay also captures a buffer, atlas bind group, uniform bind group
and dynamic offset per pass. Retained storage therefore needs its own model
(persistent allocations, generations, protection for versions encoded in one
submission, growth, current per-pass bindings) before any "upload only what
changed" or "replay" is sound. A frame where all of one family's instances
changed gains nothing from many small writes; ranges are coalesced.

### Phase 6 — partial order reuse (only if measured)

The prefix rule is causally sound (units before the first changed one keep their
orders) but "truncate the grid" is **wrong as drafted**: cells keep running
summaries (`top`, `other`), and a single-key list's boxes sit in `pending`
until the list turns multi-key, so truncating the entries leaves stale
summaries. It needs a checkpoint/undo or a rebuild of the grid from the kept
prefix (which saves the overlap queries, not the stores). The propagation rule
is sound as a forward closure when every insertion, deletion, key change, box
change and grouping change is seeded, old boxes detect removed dependencies and
new boxes added ones, a surviving unit recomputes its maximum from all current
predecessors (orders can fall), and a key-only change propagates even if its
own order stays (A then B overlapping, both key X order 0; A becomes key Y: A
stays 0, B must become 1). It also needs a way to find *later* overlapping
units, which the grid (earlier-only) does not offer, so it is not cheaper than
a rescan until an index exists. Not started before Phases 2 to 5 are measured.

## Phasing

Each phase lands only if it is exact against the oracle, moves a named scene,
and holds the all-changed frame within the budget.

0. Oracle (independent recompute of boxes, orders, instances, ranges and, on
   the GPU, pixels; frame *sequences*: insert, remove, reorder, empty to
   visible, shadow regrouping, font change, resource replacement, growth and
   compaction, fractional DPI and origin, scrolling across the viewport edge,
   several differing renders before one submit), counters, budgets.
1. Snapshot, context key, stage-wise comparison.
2. Whole-plan replay (`static`, `animated`, `one`).
3. Boxes.
4. Position-independent prepared text and pointer-identity layout hits
   (`scroll`, `para`).
5. Retained GPU storage, then partial upload.
6. Partial order reuse, if measured.
7. Cheaper list building on the caller side.

## Open questions for review

- [ ] The cheapest sound comparison of a `TextBlock`: derived equality (safe), or
      retained `Arc` identity for content and range lists with a field compare
      (fast). Measured in Phase 0.
- [ ] **Slots.** `UiRenderer` holds one `DrawPlan` and one `BatchDraws`, and
      `render_layers` draws the base list and then each layer through them, so a
      single kept frame would be overwritten by each layer and the base would
      be compared against the last layer every frame. One kept frame per
      position in the call (base, layer 0, layer 1, ...), keyed by position
      since `DrawList` ids are fresh each frame? Layers stay ordering barriers.
- [ ] Memory: the kept frame is a second copy of records and derived data;
      measure at 10k units rather than guess.
- [ ] What decides a block was *moved* rather than rebuilt, if Phase 4 keeps any
      position-dependent shortcut (a runtime predicate, not "the oracle decides").
- [ ] Whether the comparison is worth it below some list size (at 100 cells a
      whole render is 0.07 ms).

## Review

Reviewed 2026-11-08 by Codex and Antigravity, each against the code. Both found
the same core problems; the findings and what was done:

Accepted (and changed above):

- **Partial upload does not work on `FrameArena`** (both, severity high): a
  per-frame bump allocator; growth gives an uninitialised buffer; layers
  overwrite each other's ranges; queue writes land at submit. Phase 5 now needs
  retained storage first, and partial upload comes after.
- **Grid rollback by truncation is wrong** (both): the `top` / `other` summaries
  and `pending` are not undoable. Phase 6 says so; the first order reuse is
  whole-plan replay, which needs no rollback.
- **The dependency table was incomplete** (both): added the target's logical
  size (visibility), the font-system generation, command grouping and soup
  vertices, text boxes coming from prepared instances, sprite bindings and
  resolution (an image that finishes loading changes nothing in the list), and a
  unit's own key and boxes in its order.
- **Sweeps** (both): a text command with any shadow or glow emits three sweeps
  across its blocks, so a block's instances are not contiguous and inserting a
  command can regroup them.
- **Instance counts and `UnitRef` / paint-index rebasing** (both): a unit gaining
  a glyph shifts every later range; an insertion shifts every index behind it.
- **Translation reuse is unsound** (both): visibility changes with position,
  rounding is not translation-invariant, and the benchmark's `scroll` and
  `para` move every block each frame. Replaced by a position-independent cache.
- **Binary same/changed contradicts colour animation** (Codex) and the early-
  abandon rule (Antigravity: a static header would hide an animating body):
  the comparison is stage-wise and the abandon rule uses a window ratio.
- **Slots and `render_layers`** (both): single kept frame is overwritten per
  layer; now an open question with the position-keyed proposal.
- **Raw byte comparison is too broad** (Codex): `IconDraw` holds a `String`,
  soup is a range into shared vertices, `TextBlock` has no equality; semantic
  comparison, with the soup's covered vertices and trailing soup.
- **Existing bug**: the culling viewport was divided by the scale twice (Codex);
  fixed and tested before anything else (see Done).
- **Measurable requirements** (Codex): a budget, renderer time separated from
  build and drop, counters and a memory counter; Phase 0 owns them.

Rejected, with reasons:

- *"Phase 6 (cheaper list building) violates the no-interface-change goal"*
  (Antigravity): it makes the caller's existing calls cheaper (`TextBlock::new`,
  pushes) and changes no signature. Kept, renumbered Phase 7.
- *"Whole-list replay first"* as a separate phase (Antigravity): folded into
  Phase 2, which already replays the plan when keys and boxes are unchanged, and
  covers colour-only frames that a whole-list compare would not.
- *"Ordinary layout eviction does not invalidate copied instances"* (Codex) is
  accepted as true, so layout-cache eviction is *not* in the context key; only
  the font-system generation is.

Not verified by either reviewer, so measured in Phase 0: the cost of the
comparison against recompute at 100, 1,000 and 10,000 cells, the cost of many
small `write_buffer` calls against one, and any caller that edits the public
arrays in a way the comparison does not see.

## References

- `docs/design/draw-call-batching.md` (units, boxes, `DrawOrder`, `DrawPlan`)
- `bench/ui-compare/README.md` (the scenes and the profile)
- `TODO.md` "Performance review follow-ups" (frozen/prepared list, cull text,
  atlas tile on `ShapedGlyph`)
