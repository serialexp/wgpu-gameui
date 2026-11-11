# Frame-to-frame reuse

Status: partial — Phases 0 to 2 landed (2026-11-09): with `set_frame_comparison` on (default off) a list whose places are unchanged is drawn from its kept plan; boxes, text and upload are not reused yet. Since then (2026-11-11): draw boxes and prepared text reused (Phase 3 and the text item in `TODO_DONE.md`), and a list whose owner reports its edits (the retained tree's root) is not compared or kept; and Phase 5's retained GPU storage: each place keeps its records on the GPU, and a frame drawn from its kept plan uploads only the records whose look changed
Owner: Bart
Last updated: 2026-11-11

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

- [x] **Phase 0 — pixel oracle** (`tests/frame_oracle.rs`, `--ignored`, GPU).
      Sequences played on a long-lived `HeadlessGpu` against one rebuilt from
      nothing in the same state, per frame: a scene across scales and families,
      sprites updated in place, unloaded and reloaded under a name (ids reused),
      a nine-slice re-registered, an atlas compacted, fonts loaded mid-run, the
      glyph atlas growing, and two renders recorded before one submit. Checked
      to fail: a fresh run that ignores the sprite changes is reported. Found on
      the way, and not a bug of reuse: text drawn from another place in the glyph
      atlas can come out one level off in a channel (the sampler quantises
      bilinear weights), so a fresh renderer and a long-lived one already differ
      by that before any reuse. The oracle allows exactly that for text
      (`Tolerance::AtlasSampling`) and nothing for the rest (`Exact`); the
      headless oracle, which shares the atlas, is exact on instances.
- [x] **Phase 0 — comparison cost** (`benches/frame_compare.rs`, Criterion). A
      stand-in for the per-record comparison (chrome records as bytes; each text
      block's layout and placement fields, its shared lists by `Arc` identity)
      against the renderer's CPU cost for the same list (cells with a label,
      GPU not waited for), one run under load 6, so read the ratios:

      | cells | compare, identical (full walk) | compare, text moved (stops early) | recompute |
      | --- | --- | --- | --- |
      | 100 | 0.45 µs | 0.15 µs | 51 µs |
      | 1,000 | 4.6 µs | 1.4 µs | 320 µs |
      | 10,000 | 67 µs | 28 µs | 7.4 ms |

      The full walk costs about 1% of a recompute at each size, so the budget
      (an all-changed frame within 5% of today) holds even if the comparison
      never stops early; and the "below some list size" question has the answer
      that no size is too small. It is a lower bound: the real comparison also
      names which stage's inputs changed.

- [x] **Phase 1 — snapshot, context key and stage-wise comparison**
      (`render/frame_diff.rs`; `UiRenderer::set_frame_comparison`, default off).
      Each render call keeps the records of its list (up to 8 places per frame:
      the base list and the layers over it) with the context key (pixel grid,
      viewport, font-system generation, texture resolution epoch), and compares
      the next list with it record by record: `Same` (bit-identical), `Look`
      (only a colour or tint differs) or `Place`. `FrameDiff::identical()`
      means plan, instances and draws are unchanged; `keeps_places()` means
      keys, boxes, orders, batches and instance counts are unchanged (bytes may
      differ). The comparison gives up after 64 records when over half changed
      place. `RenderStats` reports lists compared / identical / keeping places,
      records by class and `snapshot_bytes` (this closes the Phase 0 memory
      counter). The classification is checked against the real pipeline rather
      than argued: `draw_box` fuzz contracts (inputs it calls the same give an
      equal `PixelBox`, with NaN, infinity and negative zero in every float),
      the headless oracle checks every step's diff against the planned result,
      a seeded random-mutation fuzz exercises all six record families with
      clips, effects, inserts and removals, and the GPU oracle checks the
      renderer's reported counts. Found on the way: the box of a chrome record
      reads no colour, but the first mask compared slot 5 for every kind (a
      segment's reaches, a chrome record's *border colour*), which made a
      recoloured filled rect a `Place`; the mask is now per kind, and the
      contract test covers each kind.
      Costs, measured (`benches/frame_compare.rs`, comparison on, every frame
      differing from the last; one run, load not controlled):

      | cells | recompute | + comparison, colour only | + comparison, all moved |
      | --- | --- | --- | --- |
      | 100 | 42.8 µs | 50.1 µs (+17%) | 47.7 µs (+12%) |
      | 1,000 | 330 µs | 390 µs (+18%) | 367 µs (+11%) |
      | 10,000 | 3.27 ms | 4.33 ms (+33%) | 4.23 ms (+29%) |

      This is over the 5% budget for an all-changed frame, which is why the
      comparison stays opt-in until Phase 2 consumes it. Nearly all of it is
      keeping the frame, not comparing: copying 10,000 `TextBlock`s (a large
      struct with several `Arc`s and `Vec`s) is about 0.7 ms of the ~1.1 ms;
      the compare is about 0.1 ms. The records are copied because the renderer
      is handed a borrowed list it cannot keep.

- [x] **Phase 2 — whole-plan replay** (`UiRenderer`, with the comparison on). The
      plan of each place's list is kept beside its snapshot; a list for which
      `keeps_places()` holds is drawn from it again, so boxes, order, sort and
      batching do not run, and only the instance bytes are filled from the new
      list (`BatchDraws::fill`, then upload as before). `plans_replayed` counts
      them, and `boxes_computed`, `units_ordered` and the order counts leave
      them out. Checked by the GPU oracle with the comparison on in every run
      (frames from a kept plan, recoloured and moved, against a renderer that
      has seen nothing, in pixels), and by mutation: replaying whenever a plan
      exists fails five of the eight oracle tests. Sprite bindings are guarded
      by the context key's resolution epoch rather than compared per unit; the
      epoch moves on any load, unload, rename, compaction or nine-slice
      registration, which is coarser than per unit and sound.
      Measured with `benches/frame_compare.rs`: the comparison on, every frame
      differing from the last in the way named, against the same frames with it
      off (`recompute_alternating` for the all-moved column). Other agents'
      load on the machine moves a run by several per cent and sometimes much
      more, so these are the quiet runs, filtered to one size at a time:

      | cells | recompute | identical | colour only | all moved |
      | --- | --- | --- | --- | --- |
      | 100 | 42.5 µs | 30.1 µs (−29%) | 30.6 µs (−28%) | 43.8 µs (+0%) |
      | 1,000 | 322 µs | 162 µs (−50%) | 168 µs (−48%) | 318 µs (−1%) |
      | 10,000 | 3.13 ms | 1.55 ms (−50%) | 1.68 ms (−46%) | 3.41 ms (+8%) |

      A frame that keeps its places costs about half what a recompute does, and
      an all-moved frame pays nothing to 8%. What is left in a replayed frame
      is text preparation and the upload (Phases 4 and 5), so the saving grows
      with them. The +8% at 10,000 is unexplained: it does not shrink when the
      rest between comparisons is raised from 16 to 200 frames, so it is not
      the cost of keeping a frame, and no per-frame work in the resting path
      accounts for it. Repeat it on a quiet machine before relying on it.
- [x] **Phase 1 follow-up — the cost of keeping and comparing a frame**, which the
      Phase 2 saving justified lowering. Two changes, no API change:
      *incremental keep* (the compare already says which records changed, so
      only those are copied: a list that is the last one again copies nothing)
      and *rest after a give-up* (a list in which most records move is not kept
      or compared on the next 1, 2, 4 … up to 16 frames, so it pays for the
      comparison on one frame in a few; a list that settles is compared again
      within 16 frames and a kept one, 18 at most). Keeping a frame cost 29–33%
      before; the all-moved column above is what is left. Checked by the
      headless oracle asserting after every keep that the kept frame compares
      identical to the list (mutated: copying only `Place` records fails five
      tests), and a test of the rest and the resume. `RenderStats::lists_skipped`
      counts rests. `benches/frame_compare.rs` has the `recompute_alternating`
      baseline.

- [x] **Phase 3 — reuse draw boxes** (`DrawPlan::build` takes the comparison's
      `Changes`). In a frame where something moved, a unit whose record is
      `Same` or `Look` takes its boxes from the plan before this one instead of
      computing them; only the units that moved are boxed. `boxes_reused`
      counts them and `boxes_computed` now counts only boxes computed. The plan
      keeps the last plan's boxes beside its own. Two things found while
      building it, both by the oracle rather than by thought: paint indices
      cannot be the key, because a text block gaining a shadow regroups its
      command and shifts every later unit's paint index; units that are not text
      are therefore found by their place among such units (a regroup leaves
      them alone) and a text unit by its first block, and only when the same
      blocks make up the unit. Checked by the headless oracle (every step plans
      with reuse and is compared, boxes included, with a plan made from nothing;
      mutated: reusing the boxes of moved units fails five tests), a test that
      moving one unit of each kind computes at most two boxes, and the GPU
      oracle comparing pixels for frames in which a few units moved.
      Measured by cycles and instructions (`perf stat`, user mode; wall clock was
      useless with the machine at a load of 25–50), 1 text block moving in a
      list of cells with a label each, comparison on against off:

      | cells | instructions | cycles |
      | --- | --- | --- |
      | 1,000 | −12% | −18% |
      | 10,000 | −11% | −7% to −12% |

      and the same list repeated (Phase 2's replay): −60% / −61% instructions,
      −51% / −54% cycles. Boxes are a small part of a frame: of the text
      case's profile, `draw_box::msdf` and `analytic` are about a tenth, and
      what remains after them is ordering (`DrawOrder::add`, `store`, the sort:
      a quarter), text placement (`append_placements`, a sixth) and the
      comparison itself (`text_change`, `compare`, the byte compares: about a
      tenth), so Phase 3's saving on a partly changed frame is a tenth less the
      comparison's cost. The next savings are in text (Phase 4) and ordering
      (Phase 6).
- [x] **Edits reported by the list's owner, in place of the comparison**
      (extended beyond the original plan, from the retained-mode comparison;
      2026-11-11). A list whose owner opts in (`DrawList::report_edits`)
      carries a version (a stamp new at each `clear`, and its patches since)
      and a log of what each `patch_translated` overwrote since
      `begin_edits`, classed by the comparison's own change functions. The
      renderer takes that log when it is relative to the version it drew last
      in that place, treats an unchanged version as every record `Same`, and
      plans from nothing otherwise; it keeps no copy of such a list
      (`FrameSnapshot::reported`, `RenderStats::lists_reported`). The
      comparison stays as the oracle behind the `edit-oracle` feature. See
      the follow-up under Non-goals.
- [x] **Phase 5 — retained GPU storage, partial upload** (2026-11-11; the
      retained-mode comparison's item 5). Built as the follow-up under
      Phase 5 below describes, with the differences noted there ("As
      built"): `render/place_draws.rs` keeps, per place, the batch arrays,
      where each record went in them (`batch_draws::Positions`, recorded by
      `BatchDraws::fill`) and a GPU buffer per family; a replayed frame with
      its text refreshed in place rewrites the `Look` records and writes
      them in coalesced runs, anything else fills the place and uploads it
      whole. `BatchUpload` names each family's buffer, so a draw reads a
      place's buffers or the arenas alike. `RenderStats::records_rewritten`
      and `place_bytes`. Tests: the GPU oracle's
      `a_place_that_keeps_its_records_rewrites_only_what_changed` (a
      recolour of each family, of a soup command, of the tail and of both,
      text with a shadow and a glow, an MSDF icon, every family at once, a
      move, all culled and recoloured, back, a frame without the place and
      the one after it, each against a fresh renderer and with its count of
      records rewritten; an identical frame uploads what a frame drawing
      nothing does, one recoloured panel 176 bytes more in one more write)
      and `nine_lists_and_the_comparison_turned_off_and_on`; with
      `set_check_plan` on, every frame's place is compared byte for byte
      with a fill from nothing (mutated: a wrong glyph quad in a rewrite
      panics there). Measured in the retained bench (load about 12), bytes
      uploaded per frame: grid `static` 2,976,176 → 176 (the uniforms),
      `one` → 352 (one record), `animated` → 1,760,176 (the 10,000
      recoloured panels; no glyph quads); deep `static` 1,760,176 → 176 and
      `one` → 352; rows unchanged (113,360 on `rows-one`, whose plan is not
      replayed). The places hold 9.4 MB on the grid (CPU copy and GPU
      buffers). Frame means at that load: grid `static` 0.23 ms, `one`
      0.24, `animated` 1.70; deep `static` 0.09, `one` 0.11, `animated`
      1.75. Not yet compared with the build before it at the same load.

### Outstanding

- [ ] **Turn the comparison on by default.** Waits for the +8% at 10,000 cells
      above to be explained on a quiet machine, and for Phase 3 to have something
      more to reuse.
- [ ] **Phase 4 — a prepared-text cache.** Position-independent, so a moved block
      (`scroll`, `para`) hits too; plus pointer-identity layout hits.
- [ ] **Phase 5 — timings against the build before it**, at one load (the
      counts above are exact; the frame means are not yet compared).
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

  *Follow-up, 2026-11-11:* the retained tree (`retained-mode.md`) added them,
  as the edit log above rather than hints. Each `patch_translated` classes
  what it overwrites, so the owner pays for the records it changed and the
  renderer neither compares nor keeps the list. Correctness rests on the
  owner changing a reporting list only by patches and clears between
  `begin_edits` and the render; anything else that changes the counts is
  caught, and a write to the public record arrays is not. Choices made in
  building it (confirmed by Bart, 2026-11-12): a list produced but not drawn (a missed
  render) is planned from nothing next time; the soup is reported whole, at
  the most any part of it changed; a recorded root, since it is a new
  version with no log, is planned from nothing with no comparison, so the
  draw boxes of that frame are not reused; and the path runs only with
  `set_frame_comparison` on.
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

### Cost of keeping a frame

*Update, Phase 2: options 3 and the incremental form of 1 were built; see Done.
What follows is the reasoning at the time.*

Measured in Phase 1: keeping a frame is nearly the whole cost of the comparison
(see the table in Done), and it is the copy of the text blocks. A fully reused
frame saves far more than this costs, but a frame where everything changed pays
it for nothing, and the budget is 5%. Options, none chosen:

1. **Keep a compact text key, not the block.** Only the fields the comparison
   reads, as `Copy` data plus the `Arc`s it compares by identity (a clone of an
   `Arc` is an atomic add, so it is not free, but a `TextBlock` also has
   `Option`s for effects, a `Vec` of spans and a font handle). A pointer alone
   cannot stand for an `Arc`: the allocation can be freed and reused by another
   string, so the key must hold the `Arc`.
2. **Take the list's buffers instead of copying them.** The caller builds a new
   `DrawList` each frame and drops it after `render`; a `render` that took the
   list by value (or a `DrawList` that gives up its buffers when rendered) could
   swap the last frame's buffers in, with no copy and the same buffers reused
   for the next build. This changes the signature callers use, which the
   no-API-change goal rules out for the first version, but it removes the copy
   rather than shrinking it.
3. **Keep a frame only when it is likely to pay.** Skip keeping while the last
   few frames were abandoned (all moved), and keep again when the content
   settles. Costs one frame of latency to notice a still scene.
4. **Accept it on the compare-and-recompute path** and rely on Phase 2 to pay
   for it where frames are alike; the 5% budget is then judged on a mixed
   workload, not on the worst case alone.

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

*Follow-up, 2026-11-11: the model, as built for the retained-mode
comparison's item 5 (`TODO.md`).* Measured first: a replayed frame uploads
every family again, 2.98 MB on the 10,000-cell grid (1.76 MB of analytic
records, 1.2 MB of glyph quads) and 1.76 MB on the deep scene, whether one
record changed or none, and copies it twice on the way (`BatchDraws::fill`,
then `Queue::write_buffer`'s staging copy). A quarter of the retained
grid run's samples are in `memmove`.

- **Only lists with a place keep storage.** A list drawn in one of the
  frame comparison's places (`compare_with_last_frame`, at most 8 a frame)
  gets a `SlotDraws`: its `BatchDraws` (the family arrays in draw order, now
  one per place rather than one shared), where each record went in them,
  and a GPU buffer per family (soup vertices and indices, analytic, circles,
  sprites, nine-slices, glyph quads, MSDF icons) that holds what the arrays
  held when last drawn. Lists without a place (comparison off, past the
  eighth, lists with no paint stream) go through the `FrameArena`s as
  before.
- **One version per place per submission.** A place is drawn at most once
  between two `begin_frame` calls (places are numbered from 0 at each), and
  `begin_frame` is once per submission by contract, so no submission holds
  two versions of one place's buffers. A frame encoded but not submitted
  before the next `begin_frame` breaks the arenas the same way today; the
  contract does not change. Writes to a buffer an earlier submission reads
  are ordered after it by the queue.
- **Where each record went.** Filling a place from nothing (the soup tail
  included) also records, per family, each list record's position in the
  family array (none for a record the plan culled), the sprite and
  nine-slice records' texture entries, for each text unit drawn the signed
  shift from its prepared instances to the batch's (a unit's prepared
  instances, `MsdfPrepared::text_units`, are in paint order and include
  culled units, the batch's in batch order without them, so the shift can
  be negative; a block's sweeps, `block_sweeps`, lie inside its unit's
  range), and for each MSDF icon drawn its position and instance count.
  About 4 bytes a record.
- **A replayed frame rewrites only what changed.** When the plan is
  replayed (every record kept its place) and the place's storage is
  current, nothing is filled. Each record the frame's `Changes` class as
  `Look` is built again into its position: analytic and circle records
  copied, sprites and nine-slices rebuilt from their texture entry, a text
  block's three sweeps copied through its unit's shift. Text needs
  `prepare_texts` to have refreshed in place (it now says whether it did);
  when it prepared everything again, instance counts and ranges may differ,
  so the place is filled from nothing instead. MSDF icons are prepared again
  every frame and nothing says their tiles stayed put (a font registered
  since turns an icon of no instance into one), so each drawn icon's
  instances are compared byte for byte with its kept ones; a count that
  differs fills the place from nothing. The soup's vertices are written
  once, whole, when any soup command or the tail is a `Look` (its indices
  cannot change without a `Place`), and only if a soup draws.
- **Coalesced writes.** The positions rewritten in each family are sorted
  and written as runs, a run joined to the next when the gap between them
  is under 4 KiB (the bytes, not a record count: an analytic record is
  176 bytes, a glyph quad far fewer). When the runs of a family come to
  more than half its bytes, the family is written whole instead. There is
  no cap on the count of runs: scattered edits cost a write each, never an
  upload of everything between them. An identical frame writes nothing but
  its uniforms. The work is counted: `RenderStats::records_rewritten` (by
  record) and the existing `buffer_*` counters (by write and byte).
- **Any other frame in a place** (not replayed, or storage not current)
  fills the place from nothing, records the positions and uploads every
  family whole, growing a buffer to the next power of two when it is too
  small. The old buffer is dropped, not retired: wgpu keeps a buffer that
  encoded work uses alive until that work is done, and a place's buffers
  are not read by anything encoded earlier in the same frame. Growth is
  checked against the device's `max_buffer_size`; a list that would not
  fit is drawn through the arenas.
- **"Current" means** the place's arrays, positions and GPU buffers are
  exactly what the last fill and rewrites made from the list last drawn
  there, under the plan the place holds (`plan_current`). It is set at the
  end of a fill and upload, a list with nothing to draw included, and
  cleared whenever the place's plan is built anew without a fill, it is
  drawn in paint order, it is released, or the comparison is turned off.
- **Bindings per pass.** The text and icon runs are built every frame from
  the place's buffer and the pass's own uniform and atlas bind groups, so
  nothing captured last frame is reused.
- **Released when unused.** A place not drawn in a frame lets its storage
  go at the next `begin_frame` (a popup layer's place keeps nothing after
  it closes), and `set_frame_comparison(false)` drops all of it. What the
  places hold, CPU arrays and positions and GPU capacity, is reported
  (`RenderStats::place_bytes`).
- **Checked.** With `set_check_plan` on (the `paint-order-oracle` feature,
  on in the crate's tests), each place's arrays after a partial rewrite are
  compared with a fill from nothing, byte for byte (derived float equality
  would miss a sign of zero and fail on a NaN). That check shares the plan
  and the prepared text with what it checks; the plan oracle and the GPU
  oracle, which draws sequences against renderers that have seen nothing,
  check those. The sequences cover a recolour of one record of each family,
  of a soup command and of the tail, text with shadows and glows,
  everything culled and back, a place released and used again, the
  comparison turned off and on, and the ninth list. Counted per family: an
  identical frame rewrites no record and writes only its uniforms; one
  recoloured chrome record rewrites one record of 176 bytes.

*As built (2026-11-11).* Where the code differs from the model above:

- The place is `PlaceDraws` (`render/place_draws.rs`), not `SlotDraws`; its
  buffers are `PlaceBuffers`, one `Retained` buffer per family.
- **No fallback to the arenas for a list too big.** A buffer grows to the
  next power of two, capped at `max_buffer_size` but never below what it
  must hold. The arenas grow to the whole frame's bytes, more than one
  list's, so a list a place could not hold would not fit an arena either;
  the fallback would never help.
- **Every frame in a place is checked**, filled or rewritten, not only a
  rewrite: the check costs a fill from nothing, and the tests want both.
- **The `Look` records are found by scanning** the frame's `Changes`, a
  byte a record per family, not from the owner's edit log: for a compared
  list that is all there is, and for a reported list the log has already
  been spread into `Changes`. Copying stays O(changed); the scan is
  O(records) of one-byte compares.
- "Current" is set by every fill, and every frame that does not rewrite
  fills, so a place whose plan was built anew, or drawn in paint order, is
  never left current over stale arrays.

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
- [x] Whether the comparison is worth it below some list size: measured, about 1% of a recompute at 100, 1,000 and 10,000 cells (Phase 0).

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

### Review of the Phase 5 model (2026-11-11)

Codex and Antigravity reviewed the follow-up under Phase 5 against the code
(both needed a retry). Accepted, and changed in the bullets above:

- **A text prepare that did not refresh** (both): counts and ranges may
  differ, so copying every glyph quad through the old mapping is wrong; the
  place is filled from nothing, and `prepare_texts` says which it did.
- **MSDF icons compared in list order** (Antigravity): prepared icons are in
  list order and include culled ones, the batch's are not; compared through
  each icon's position, and a count change (a font registered since) fills
  the place.
- **Rewritten positions out of batch order, and negative shifts**
  (Antigravity, Codex): sorted before coalescing; shifts are signed and per
  unit, since a block's sweeps sit inside its unit's range.
- **Coalescing by record count** (both): 33 scattered edits would have
  written 1.7 MB. Now joined by gap bytes, with no run cap, and whole only
  past half the family's bytes.
- **The soup** (both): the tail is part of the fill and of the check; the
  vertices are written once a list, only when a soup draws.
- **Lists with nothing to draw** (Antigravity): the early return would have
  left storage never current; it is current with nothing in it.
- **Retiring grown buffers** (both): with places released and used again the
  renderer-lifetime retired list would grow each cycle; grown place buffers
  are dropped, which wgpu makes safe for work already encoded.
- **Memory and limits** (both): `place_bytes` reports what places hold;
  growth is checked against `max_buffer_size`.
- **"Current" undefined** (Codex): defined above.
- **The check** (Codex): byte for byte; the sequences it must cover are
  listed; counts per family and per record rather than "one record's bytes"
  for every recolour (a text block rewrites its glyphs, a soup its vertices).

Recorded in `TODO.md`, not changed here (they predate this and hold for the
arenas too): a no-op atlas compaction or the removal of an unknown sprite
moves the resolution epoch, which stops every replay (Codex); a tint or span
colour change is classed `Place`, so a container's fade rebuilds the plan
(Codex); a list with no paint stream takes no place, so an empty layer moves
every later list to another place and their storage is rebuilt (both).
Accepted as already true: a shared atlas mutated between two renders of one
submission makes the first sample the second's pixels (Codex); that is a
contract of `begin_frame`, not of this storage.

## References

- `docs/design/draw-call-batching.md` (units, boxes, `DrawOrder`, `DrawPlan`)
- `bench/ui-compare/README.md` (the scenes and the profile)
- `TODO.md` "Performance review follow-ups" (frozen/prepared list, cull text,
  atlas tile on `ShapedGlyph`)
