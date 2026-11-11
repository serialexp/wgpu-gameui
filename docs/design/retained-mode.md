# Retained mode — Design

Status: partial — Phases 0 to 5 landed (`DrawList::append`, `append_translated`, signals, the tree with layout and cached drawing, pointer input, the comparison; 2026-11-10 to 11), and lists patched in place in their ancestors', kept only at the root and at nodes that clip, the root's list reporting its edits to the renderer (2026-11-11), a scroll's rows laid out only near its view and the node table at 384 bytes a node (2026-11-12); Phase 6 and the rest of the comparison's findings outstanding. A prototype.
Owner: Bart
Last updated: 2026-11-12

## Implementation status

Tracking the gap between this design and what's on the main branch.
Tick items as they land.

### Done

- [x] **Phase 0 — `DrawList::append`.** A list recorded in pieces, each into its
      own list and appended in order, is the list the same calls record into one:
      arrays extended, soup indices and run ranges offset, runs of one family that
      meet merged as recording merges them. Oracle in `render/append_oracle.rs`:
      every cut of one sequence, and 200 random sequences of every record kind,
      compared array by array and by the renderer's `FrameSnapshot::compare`
      (`identical()`). Checked to fail when the leading soup flush is removed
      (`db9af2f`).
- [x] **Phase 1a — translation-invariant snapping in core.** The underline's
      record-time `round()` became `floor(x + 0.5)`; stroke snapping was already
      invariant (`(e - 0.5).ceil()`). The test moves an underline on a half
      pixel by whole pixels across zero, and fails with `round` (`efb2a40`).
- [x] **Phase 1b — `DrawList::append_translated`.** Per record kind as in the
      table below, shadows recording would have culled dropped, soup committed
      only before a record kept (a culled shadow commits none when recorded),
      `viewport_clips` moved and clipped. Oracle in `render/append_oracle.rs`:
      300 random placements (quarter-pixel coordinates either side of zero,
      stripes, joined and snapped polylines, underlines, tint, a dyadic scale
      over every transform-keeping kind, viewport clips) against the same calls
      recorded inside `push_clip` and `translate`; nested placements against
      nested recording; a culled shadow between soup; placing with no move is
      appending; and the empty nested clip exception pinned. Each of six
      mutations (no culling, eager soup commits, neighbours unmoved, stale chrome
      clip flag, translate-only chrome moved by its translation, viewport and
      text clips unmoved) fails at least one test.
- [x] **Phase 1c — `append`'s repaint request documented as relative** to when
      `other` was recorded, with what a keeper of lists should do instead.

- [x] **Phase 2 — signals.** `crates/retained/src/signal.rs`: a `Graph` of
      signals and memos with mark-then-pull propagation, equality guards,
      observers reported by key with a version per node, owners and disposal
      with edges removed both ways in O(1) each, generations on handles, cycle
      detection. 13 tests (laziness, equal writes, a diamond, propagation
      stopped by an equal value, a branch switched in the same batch, sources
      dropped, deduplicated reads past the scan limit, a cycle, disposal leaving
      nothing, stale handles, disposing of what a live memo reads, 10,000 readers
      of one signal removed one by one, slot reuse). Checked to fail with the
      back pointers left stale, with "check" run as "dirty", and with the check
      walk skipped.

- [x] **Core: canonical empty clips** (Bart, 2026-11-10; follow-up under
      `append_translated`). `DrawList::difference` names the first record two
      lists differ in, for the oracles (`656d39f`, `f8613e8`).
- [x] **Phase 3 — the tree, layout and drawing.** `crates/retained`: `ui.rs`
      (the tree, props bound or set, the frame, counters), `node.rs` (`Spec`,
      `Size`, `Stack`, `Look`, `Text`, `Painter`), `layout.rs` (stack layout with
      snapped edges, fill shares from fractional cursors, absolute children,
      scroll containers with clamped offsets, layout dirt stopping at a node
      whose size does not follow its content), `paint.rs` (a list per node with
      children in local coordinates, placed with `append_translated`; culling by
      binary search over the stack, widened by the children's paint reach;
      paint marks stopping at a node marked this frame), `own_drawing.rs`.
      External context (fonts generation, scale, viewport), clocked repaint
      deadlines, volatile painters, work counters (`FrameStats`, `TreeStats`).
      33 tests: each layout rule, dirt, counted frames (an unchanged frame does
      nothing; one cell of a 10×10 grid records 4 lists; scrolling examines the
      rows in view), shadows reaching into a clip from either side, culled nodes
      already out of date, painters by value, clock and volatility, removal
      disposal, a font change, an emptied scroll container, and the oracles:
      40 random trees × 30 random changes (sets, adds, removes, scrolls, offsets,
      viewports, a moving clock, clocked and volatile painters), each frame
      against the tree drawn from nothing and laid out from nothing. At 10,000
      rows a change in view costs 4 nodes visited, 3 lists recorded and the 20
      rows in view examined (counted); timings print from an ignored test
      (release, this machine: 57 µs per frame with a row changed, 3.4 µs per
      scrolled frame, 11 ns unchanged, 45 ms to build). Checked to fail under
      ten mutations (paint marks stopping at stale nodes or never resetting,
      either side of the search unwidened, a clip's children's reach not
      refreshed, layout dirt not walking up, the cached layout entering clean
      children, deadlines or volatility ignored, absolute children left out of
      the paint reach).

- [x] **Phase 4 — pointer input.** `crates/retained/src/input.rs`:
      `pointer_moved`, `pointer_button`, `pointer_left`, `cancel_pointer`,
      `wheel`, `hit_test`, `hovered`; `Spec::hover(signal)`,
      `Spec::press(signal)`, `Spec::on_click`. Hit testing in reverse paint
      order, pruned at clips and by paint bounds elsewhere, with binary search
      in every stack; hover found again after each layout round of a frame;
      removal ending hover and presses. 14 tests in `input_tests.rs` (a prop
      following hover, the topmost taker, pass-through, overflow hit and clip
      not, children moved out of their rows, nothing hit before the first
      frame, hover kept current by a scroll, a move and a removal, clicks,
      per-button presses, three cancellations, a handler removing its node,
      the wheel by axis and at its limit), and at 10,000 rows a hit enters 4
      nodes (counted; 127 ns per pointer move in release on this machine).
      `check` holds the hover and every hover and press signal against a hit
      test from nothing after each frame, and the random oracle moves the
      pointer (half the time onto a node), presses, cancels, leaves, scrolls
      by wheel, removes what is hovered, compares 8 random hits per step with
      the scan, and checks removed nodes' signals read false. Checked to fail
      under eleven mutations (non-clipping stacks' child reach not marked, the
      search unwidened on both sides and on the near side alone, open nodes pruned by their box, clips
      not pruning, no hover refresh in the frame, removal forgetting the
      pointer, a click on any release, stacked or absolute children searched
      in the wrong order, the wheel ignoring the axis).
- [x] **Phase 5 — comparison** (2026-11-11). `ui-compare-gameui-retained`, a
      second binary in the bench's `gameui` package (which now keeps the GPU
      setup in its `lib.rs`): the grid scenes, a `rows` scene (10,000 rows of
      20 px scrolling through the window: static, one value changing, scrolling,
      hovering) and, beyond the plan, a `deep` scene (10,000 leaves under four
      levels of ten), all added to the immediate binary too. Per frame the
      whole frame and the tree's share; builds; counters for the tree's work,
      what it holds (`TreeStats` gained `list_bytes` and `node_bytes`, from the
      new `DrawList::heap_bytes`), layouts shaped and the renderer's replays;
      `UI_COMPARE_REUSE=1` runs either with the renderer's frame comparison on.
      Results and their reading in `bench/ui-compare/README.md` (under
      "Retained mode"); the questions below are answered from them.
- [x] **Patching in place** (2026-11-11, beyond the plan; from the
      comparison's follow-ups in `TODO.md`). Core: `append_translated` returns
      a `Placed` (where each kind of record went, and whether none was culled);
      `DrawList::patch_translated` overwrites a run of them with a list of the
      same shape (`ListShape`: counts, paint runs, trailing soup, repaint
      request, degenerate count), declining a culled placement, other counts, a
      record now culled or an analytic record of another kind. The tree: a
      paint mark records the node's list (a leaf's parent's) and queues each
      list above it in its parent's `dirty_kids`; a list with only queued
      children is brought up to date by patching their records where it
      placed them, and hands the runs it patched up, so a change costs its
      records at each level. Anything the patch cannot vouch for records the
      parent instead. Oracles: 400 random patches in core against placing the
      new piece (whole and in two spans), the declines pinned; the tree's random
      oracle now makes three changes in some frames and asserts something was
      patched. One grid cell: 1 list recorded, 2 records patched at each of
      three levels, nothing placed (was 4 lists, 20 placements). Measured in
      the comparison (10k grid, reuse on): the tree's share of `one` 0.89 →
      0.001 ms, `deep-one` 0.145 → 0.001 ms, `rows-one` 0.065 → 0.027 ms;
      `animated` unchanged (every cell still records its list). The cache
      shape itself is still Bart's to choose (below).
- [x] **The root's list reports its edits to the renderer** (2026-11-11; was
      "Defer: a damage interface to the renderer"). The root's list is made
      with `DrawList::report_edits`, and `Ui::frame` calls `begin_edits` on it
      before bringing the tree up to date, so what the frame patches is logged
      as it is patched and the renderer takes the log instead of comparing the
      list or keeping a copy (`frame-reuse.md`, the follow-up under
      Non-goals). On the grid at 10,000 cells: `lists_compared` 1 → 0,
      `snapshot_bytes` 4.24 MB → 0, frame mean of `animated` about
      2.45 → 2.07 ms (load about 22), plans replayed and boxes reused as
      before.
- [x] **Lists only at the root and at nodes that clip** (Bart's choice of
      cache shape, 2026-11-11; under Decisions). Every other node is drawn
      into its holder's list (the nearest list above it): its own drawing is
      recorded into a scratch list and placed at its offset, its children
      after it. Core: `PlacedLog` (replacing `ListShape`) logs each piece a
      list placed with where it went and its shape then, flat, so a list of
      20,000 pieces logs them without an allocation each; `fits` says whether
      a list has the shape a piece had. The tree: a node's own drawing that
      changes is patched where its holder's list holds it, and the runs go up
      as before; a node moved, added or removed (`mark_records`) records its
      holder. A child of a clipping holder found out of view is not patched
      (it records the holder), which closes a gap the per-container lists had.
      Tests: the log's pieces kept apart (`a_placed_log_holds_each_piece_
      shape`), the patch oracle through the log, counted grid, clip, rows and
      painter tests, and the random oracle (also run at 600 rounds once).
      Measured against the tree before it (same core, alternate runs, load
      about 15): the grid keeps 1 list, not 10,001, and 11.8 MB of lists and
      logs, not 30.1 MB (the root's list 6.9 MB, its log 4.9 MB); the tree's
      share of `animated` 5.5 → 1.3 to 1.6 ms (whole frame 9.8 → 5.5 to
      6.2 ms), `deep-animated` 1.79 → 1.50 ms; one cell is 1 record patched,
      nothing recorded. The rows keep 2 lists, not 110 (145 while scrolling),
      and 76 KB, not 305 KB; scrolling now draws the rows in view again into
      the scroll container's list instead of placing their lists (tree share
      of `rows-scrl` 0.009 → 0.014 ms). Builds: grid 24 → 18 to 20 ms.
- [x] **Builds shape every text, in view or not** (found by the comparison;
      done 2026-11-12, Bart's choices under Decisions): the rows build took
      108 ms, almost all shaping 10,000 labels in rows of fixed height;
      immediate mode shapes the 54 in view. A scroll whose content has an
      exact length across it now sizes the stacked children it can without
      laying them out (`layout::deferrable`: fixed along the stack, and
      across it fixed, filling, or hugging at the start) and lays out only
      those within a viewport of its view, before and after, or half the
      window when that is more (`near_margin`); the rest are
      laid out when a scroll brings them near (`layout_near_view`, reached
      through `mark_below` when the scroll moves). A child put off is
      flagged `deferred`, and is neither drawn nor hit until it is laid out;
      a child laid out before that changes while far from the view waits
      too, keeping its last layout. `FrameStats::deferred` counts the
      children sized without being laid out. Tests: the counted
      `a_scroll_lays_out_only_the_rows_near_its_view` (10,000 rows: 40
      measured at build, 5 more on a scroll of 100 px, a changed far row
      laid out on coming back, every row sized again and 40 laid out when
      the scroll narrows), and `a_scroll_that_puts_rows_off_draws_what_one_
      laying_them_out_draws`, the same random scroll of rows built twice
      (once with `Ui::no_defer`) and driven alike through scrolls, row and
      content changes, the scroll's stack and size and the viewport: the
      frames and hits are the same, and the random oracle passes (30,542
      rows put off, 3,026 caught up with, 972 frames with rows it cannot
      defer); `a_short_scroll_lays_out_to_half_the_window_past_its_view`. Mutations caught: no mark when the scroll moves (five tests),
      no paint mark when a row put off is resized (the twin test alone; the
      oracle from nothing reads the same stale data). The bench's rows build
      measures 108 labels in its first frame and 162 in its second (three
      viewports at row 5,000), not 10,000 (`rows-bld` counts, now printed);
      timed at a load average of 25 to 28, 36 to 80 ms against 108 ms (the
      untouched `deep-bld` took 31 ms there against 3.0), so not compared:
      that waits for the rerun on a quiet machine.
- [x] **The node table at 384 bytes a node** (2026-11-12; it was 664, grown
      from 576 with patching's fields, the log, and deferral's flags). A
      node keeps its properties' values and nothing for a constant: a bound
      property's memo, setter and version are a `Binding` in
      `Ui::bindings`, a `slotmap::SlotMap`, chained from
      `NodeData::bindings` and looked up by property when a notification or
      a setter comes (at most one per property, so a short walk). A
      holder's list and its `PlacedLog` share one `Box<Kept>`, so a node
      that keeps none pays a pointer for both. Bart chose the slotmap, the
      log beside the list, and a size bound in the tests
      (`a_node_stays_small`). Animated grid 10,000: `node_bytes` 21.8 →
      13.4 MB.

### Outstanding

- [ ] **Icon content.** Phase 3 built text and custom painters; `Icon` is not
      a content kind yet.
- [ ] **Phase 6 — an example in a window.** Scrolling list, hover styling, a click
      counter.
- [ ] **Text shaped twice** (found by the comparison): a text is measured at
      the width offered and drawn at its measured width, and the layout cache
      keys on the width, so each new or changed text is shaped twice (512
      layouts in 230 `rows-one` frames against 229 in immediate mode).
- [ ] **A list's log is about 245 bytes a piece** (found by the comparison;
      the small lists, 2.3 KB each, went with the cache shape): the grid's
      20,000 pieces log 4.9 MB beside the root's 6.9 MB list, mostly the two
      `RecordCounts` of each `Placed`.
- [ ] **Defer: keyboard focus, text input, selection and highlight** in retained
      subtrees.
- [ ] **Defer: virtualised lists** (an item count and a row factory instead of a
      node per row).
- [ ] **Defer: a Lua host** over the same tree (gpuix's model).

## Why this exists

gameui is immediate mode: the caller builds a `DrawList` every frame, and the
renderer turns it into draws. The cost of a frame follows what is drawn, not what
changed. The renderer's frame reuse (`frame-reuse.md`) makes the renderer's half
of an unchanged or recoloured frame cheap, but the caller still builds the whole
list, and a host has no precise way to know whether a frame is needed at all
(agent-ui marks a frame for every wake, changed or not).

Zortax's critique of GPUI (<https://blog.zortax.de/posts/a-critique-of-gpui>)
argues that this coarse "something changed" signal costs in proportion to the
tree, and that precision needs writes the framework can see and nodes that
persist across frames. gpuix (Bart's React/Solid and Lua host for GPUI) showed
that a retained tree driven by signals is fast in practice. This prototype builds
that model in Rust on top of gameui, so that:

- a Rust app can use it with no scripting runtime;
- immediate and retained can be compared on the same scenes, in the same
  language, with the renderer held fixed;
- a scripting host could later drive the same tree.

It is a prototype: the question it answers is how much retained mode saves, and
what it costs, on gameui. agent-ui is not being moved to it (Bart, 2026-11-10).

## Goals

- **Recording follows change.** A frame after one signal changed records only the
  nodes that read it; the rest is replayed from cached lists. What remains is
  assembling the frame's list from those caches (a copy of what is visible; see
  the cache question) and the renderer's comparison of it.
- **No frame without a change.** `needs_frame(now)` says whether anything changed
  or a repaint deadline came; when not, the last list is still the frame.
- **Same list as drawing from nothing.** After any sequence of changes, the frame's
  list equals the list the same tree, drawn from nothing with the same layout and
  culling, records: array by array and by the renderer's frame comparison. Exact
  for coordinates that add without rounding (dyadic, as the oracle uses); in
  general equal to within `f32` rounding of a sum taken in another order.
- **Signals as the one mechanism.** Every node property is a constant or a memo;
  setters are sugar that write a signal the node owns.
- **Counted, not timed, in tests.** Nodes examined, recorded and replayed, records
  copied, memos run, layout passes, bytes held: tests assert these; time goes in
  the bench.

## Non-goals (v1)

- Moving agent-ui or any existing gameui widget onto it.
- Keyboard focus, text input, text selection and highlight in retained subtrees
  (a highlight is painted when a block is recorded, so a cached block would show
  a stale one).
- Rotated or scaled subtrees: replay moves by translation only, and hit testing
  uses axis-aligned rects (a custom painter's own transforms draw, but do not
  change where the node is hit).
- `push_clip_exact`, or a caller-set `TextBlock::clip`, inside a cached subtree:
  both are screen-space by contract and do not move with the subtree. Such a node
  is volatile (below).
- Virtualised lists: the rows scene uses one node per row, built up front, and
  measures what that costs.
- A damage interface to the renderer: it compares the whole list (about 1% of a
  recompute, `frame-reuse.md`).

  *Follow-up, 2026-11-11:* once a replayed frame cost little else, the
  comparison was about 14% of the renderer's part of it, and its copy of the
  root's list 4 MB on the grid, so the root's list now reports its edits
  (Done).
- A Lua or JavaScript host. Publishing the crate.

## Crate and module layout

`crates/retained` (`wgpu-gameui-retained`, `publish = false`), a workspace member
depending on `crates/core` by path. Core gains `DrawList::append`,
`DrawList::append_translated` and translation-invariant underline snapping.

| module | holds |
| --- | --- |
| `signal.rs` | the reactive graph: signals, memos, owners, propagation |
| `node.rs` | `Node`, its props (`Prop<T>`), decoration and content kinds |
| `layout.rs` | measure and arrange, with per-node cached sizes |
| `paint.rs` | recording a node, the caches, replay, culling |
| `own_drawing.rs` | a node's own drawing: shadow, box, text, painter |
| `input.rs` | hit testing, hover and press signals, handlers, wheel |
| `ui.rs` | `Ui`: the tree, the frame, the context, counters |

## `append_translated`

`list.append_translated(&other, [dx, dy], clip)` gives this list what recording
`other`'s calls would have, made on this list inside `push_transform();
translate(dx, dy)` and, if `clip` is `Some`, `push_clip(clip)` (`clip` in this
list's world space). The offset is applied in world space, outside any transform
`other`'s calls pushed, which is what a subtree recorded at its own origin needs.
Whole-pixel offsets keep record-time snapping exact (Phase 1a). Per kind:

| record | moved by the offset | clip |
| --- | --- | --- |
| soup `Vertex` | `position` | `clip`, `clip_enabled` |
| chrome | translate-only: `rect` x, y (the position is folded in); affine: `translation` x, y | slot 8; enabled in `translation[2]` and `params[2]` |
| stripes | as chrome | slot 8; enabled in `translation[2]` |
| shadow | `translation` x, y (its rects stay local even when translate-only) | slot 8; and dropped, as recording drops it, when its raster box misses the clip |
| segment | translate-only: `ends` and the real neighbour points; affine: `translation` | slot 8 |
| circle | `center` x, y | enabled in `params[0]` |
| atlas icon | all four `corners` | `clip` |
| nine-slice, MSDF icon | `transform.tx`, `ty` (`local` stays) | `clip` |
| text | `x`, `y` | `clip` (the active clip only: see non-goals) |
| `viewport_clips` | each rect | intersected |

A clip is the record's own (from clips pushed inside `other`) intersected with
`clip`. An empty intersection is stored as recording stores one: a zero-size rect
at the record's own clip origin, which is exact when `other` pushed one level of
clip and otherwise differs only in the origin of a clip that shows nothing. Run
merging and ranges are rebuilt as recording builds them, since a dropped shadow
can split or join runs. A repaint request is copied as `append` copies it.

Follow-up (Bart, 2026-11-10): the tree's frame oracle met that difference, so
core now stores every clip that shows nothing as `Rect(0, 0, 0, 0)`, in
`push_clip` and in placement alike, and placement keeps a record's clip as it is
(not recomputed) when there is no outer clip. Placing is then exact for nested
clips too, and the core oracle generates nested clips and random outer clips.
`is_in_clip_region` is false inside a clip that shows nothing.

## Signals

- `ui.signal(value) -> Signal<T>` (a `Copy` handle: the graph's id, slot index,
  generation, type). `ui.set(signal, value)` stores it only if it differs
  (`T: PartialEq`); `ui.update(signal, |v| ..)` likewise.
- `ui.memo(|r: &mut Reader| -> T) -> Memo<T>`: every `r.get(s)` inside it
  records `s` (a signal or another memo) as a source; `r.untracked(s)` reads
  without recording. A memo first runs when first read. Sources are recorded again on every
  run, so a memo that stops reading something (a branch not taken) stops
  depending on it.
- **Propagation is mark, then pull.** A write marks the memos that read the signal
  `Dirty` and everything downstream of them `Check`; nothing runs. At the frame,
  each node prop that is marked asks its memo for its value. A `Dirty` memo runs;
  a `Check` memo first brings its sources up to date, in the order it last read
  them, and runs only if one of them changed value, else becomes clean without
  running. A source read during a run is brought up to date before its value is
  returned, so a memo whose dependencies changed in this batch never sees a stale
  input, whatever its old position in the graph. Each memo runs at most once per
  frame. Reading a memo that is running (a cycle) panics with both memos named.
- A memo whose new value equals its last does not count as changed, so nothing
  downstream runs.
- **Observers.** A memo the tree draws from is observed under a key
  (`observe(memo, key)`): it is reported once each time it goes from up to date
  to marked, and the tree takes the reports (`take_notified`) at the frame. A
  report means "read it again"; the memo's `version`, which moves only when its
  value changes, says whether it did.
- The graph is `Graph` in `signal.rs`; `Ui` holds one and forwards these.
- **Owners.** Signals and memos belong to an owner: a node, or the `Ui` root. A
  memo created while a node is built belongs to that node; `ui.memo_in(owner, ..)`
  says so explicitly, and one shared by several nodes belongs to the root or to a
  common ancestor. Removing a node disposes of what it owns: each disposed memo is
  removed from its sources' reader lists and its readers' source lists, its
  closure is dropped, and its slot's generation moves on, so a stale handle
  panics on use instead of reading another value. A memo still read by a live
  memo outside the subtree is an error on removal (named in the panic), not a
  dangling read.
- A node property reads a memo. A custom painter (`Content::Custom`, a closure
  drawing with the gameui API) reads signals through a `Reader` and is tracked as
  a memo whose value is "drawn".

## The tree

- A node: parent, children (ordered), layout props (`Size` per axis: `Fixed`,
  `Fill(weight)`, `Hug`; stack direction, gap, padding, absolute offset),
  decoration (fill, border, radius, shadow), content (`None`, `Text`, `Icon`,
  `Custom`), `clips`, and `scroll` (an offset signal, clamped to its content).
- `Prop<T>` is `Const(T)` or `Bound(Memo<T>)`, held inline. `ui.set_fill(node, c)`
  turns a constant into a signal the node owns and sets it.
- Each prop is of one or more kinds of change: **measure** (the node's size may
  change: its sizes, padding, gap, direction; text content, font, size and wrap
  unless both axes are `Fixed`), **arrange** (children move: the above, and an
  absolute offset), **paint** (decoration and colours; text content too). A text
  change is measure and paint.
- **Volatile** nodes are recorded every frame and their ancestors' caches are
  never replayed: a custom painter that says it is (time, external state), and
  any node that uses a screen-space clip (non-goals).

Follow-up (Phase 3, 2026-11-10): a setter on a prop that is bound to a memo
panics rather than replacing the binding; a bound prop is changed through what
it reads. Removing a node costs O(its siblings) (its parent's child vector).
A volatile painter is counted in each ancestor's `volatile_below`, and a list
with any below it is recorded every frame; that counter is the only mechanism
(an earlier per-frame paint mark of each volatile node did the same work twice).

## Layout

- Sizes come from constraints: a parent gives each child the space available on
  each axis. `Fixed(px)` takes it as given; `Hug` takes its content's size, text
  measured with the available width as its wrap width; `Fill(w)` takes, on the
  stack's main axis, `w / Σw` of what remains after the `Fixed` and `Hug`
  children and the gaps and padding (0 if that is negative), and on the cross
  axis the whole of it. A `Fill` child of a `Hug` parent on the parent's main axis
  is measured as `Hug` (there is nothing to share). Weights at or below 0 are 0.
  Children may overflow; overflow is not clipped unless the parent clips.
- Absolute children do not take part in the stack: they sit at their offset in
  the parent's content box, with the content box as their available space.
- A scroll container measures its content with its scroll axis unbounded
  (`Fill` on that axis is `Hug`).
- **Positions are snapped** to whole logical pixels when arranged: each edge to
  `floor(x + 0.5)`, the size the difference. Subtrees therefore move by whole
  pixels.
- **Caching and dirt.** A node keeps the constraint and size of its last measure.
  Measure dirt walks up from a changed node until a node whose size cannot depend
  on its children (`Fixed` on both axes); that node and everything below the dirt
  is arranged again, and an arrange gives every child whose rect changed a new
  rect. A node whose props, children and constraint are unchanged keeps its size
  without its children being visited. After arranging, a scroll container's
  content extent is updated and its offset clamped to it again, even with no wheel
  event.
- **External context.** The frame takes a context: the font system's generation,
  the scale and the viewport. A new font generation marks every text node measure
  and paint dirty; a new viewport marks the root measure dirty. Every list the
  tree keeps shares the renderer's font system.

Follow-up (Phase 3, 2026-11-10): as built, `Fill` in a parent whose size on that
axis is not yet known (a `Hug` parent) is `Hug`, on both axes, not only the
main one. Measure dirt stops at a node whose size does not follow its content
(`Fixed`, or `Fill` given an exact size), which is wider than `Fixed` on both
axes. A gap below 0 is 0. Scrolling is along the stack's main axis only.
Absolute children are drawn after the stacked ones. The scale is part of the
context: a new scale measures and draws every text again, as a new font
generation does.

Follow-up (2026-11-12): a scroll does not lay out every child. When its
content's length across the stack is exact, a stacked child whose length
along the stack is fixed, and across it fixed, filling, or hugging at the
start (where its place does not depend on that length), is sized without
being laid out; after the scroll is clamped, the children within one
viewport of the view, before and after (but at least half the window's length
along the scroll's axis), are laid out, and the rest when a scroll brings
them near. Extent and clamping need nothing else: they depend
on lengths along the stack, which are fixed. A child put off is neither drawn
nor hit, so one whose drawing reaches more than that margin past its box (a
shadow longer than the view and half the window) is not drawn while it is
that far away. A child
laid out before that changes while far from the view keeps its last layout
until it comes near; `Ui::rect` gives what is in it as last laid out.

## Drawing and the cache

Every node with children is a **cache boundary** (Bart chose this to be measured
before anything else is chosen, 2026-11-10). A boundary keeps the list of its whole
subtree, recorded **at its own origin and without its ancestors' clips**, plus the
size it was recorded at. Leaves are recorded into their boundary's list.

Drawing a node into its parent's list, at the node's position, under the clip of
its clipping ancestors:

1. A boundary that is clean (no paint, measure or arrange change in its subtree,
   same size, no volatile descendant, not past a repaint deadline): its list is
   replayed with `append_translated(position, clip)`, every time, moved or not, so
   the ancestors' clip always applies.
2. Otherwise: its list is cleared; its decoration and content are recorded at the
   origin; each child is drawn into it at its position relative to the boundary,
   under the boundary's own clip if it clips; then the list is replayed into the
   parent as in 1.

**Culling.** A child of a clipping node is drawn only if its **paint bounds** meet
the clip. Paint bounds are the layout rect grown by what the node declares it
paints outside it: its shadow's reach, and a custom painter's declared overflow.
Children of a stack are in order along the main axis, so the ones that meet the
clip are found by binary search, not a scan. A clipping node's list therefore
holds only the children in view, so it is **viewport-dependent**: it is recorded
again when its scroll offset, its size or which children are in view changes. The
children's own lists are whole and stay valid. "Drawn from nothing" in the oracle
applies the same culling.

The renderer is given the root's list. When nothing changed it is the last frame's
list, untouched, so the renderer's frame comparison replays its plan.

**Repaints.** A node may ask to be drawn again at a time (an animation's next
step): `repaint_at(now + seconds)` while recording, in the frame's clock (the
frame is given `now`; nothing reads a clock of its own). The deadline belongs to
the node, so a cache recorded long ago never carries a stale relative time. When
it comes the node is paint-dirty; removing the node drops it. A node culled out of
view keeps its deadline but is not drawn until it is in view. The root list's
`next_repaint` is the earliest deadline in view, relative to `now`.

Follow-up (Phase 3, 2026-11-10): a clip encloses children, so a node with
none draws none, including a scroll container whose last child went (it is
then drawn into its parent's list as a leaf). Paint marks walk up to the first
node already marked in the same frame, not to the first stale one: a culled
node stays stale, and stopping there would keep a later change from reaching
the lists above it.

Follow-up (2026-11-11, patching in place): rule 2 above no longer re-records a
boundary only because something below it changed. A paint mark makes the node's
own list stale (a leaf's, its parent's, since a leaf records into it), and every
list above that is only queued, in its parent's `dirty_kids`. A list with nothing
stale of its own is brought up to date child by child: a queued child is brought
up to date, and if its list kept its shape (`DrawList::has_shape`) the records it
changed are overwritten where this list placed them
(`DrawList::patch_translated`), at the offset and under the clip they were placed
with; the runs overwritten go up to this list's parent, which does the same. A
child that is not placed in the list as it is (culled, or the list recorded
since; a generation counter says which), moved, reshaped, or whose records the
patch declines, records this list as rule 2 does. Marks still stop at a node
marked in the same frame, and a node is queued once (`queued`), so a culled,
changing subtree adds nothing per frame.

Follow-up (2026-11-11, the cache shape): Bart chose to keep lists only at the
root and at nodes that clip (or scroll), against keeping whole-subtree caches
or marking boundaries explicitly. "Every node with children is a cache
boundary" above no longer holds: a node that keeps no list is drawn into its
holder's list (the nearest list above it) at its offset from the holder, its
own drawing first, recorded at its origin into a scratch list and placed with
`PlacedLog::place`, then its children, under the holder's clip. Which nodes
keep a list is fixed for a node's life (a clip with no children keeps one,
holding its own drawing), so a node's holder never changes; deciding it by
whether a node has children fails, because layout takes them out of the node
while it places them. Marks are of two kinds: `mark_paint` (a node's own
drawing changed: queue it in its holder's `dirty_kids`, and each holder in its
own) and `mark_records` (a node's children were added, removed or moved: the
list it is drawn in records again). A queued node that keeps no list is drawn
again into the scratch list and, if it fits the shape its piece was logged
with, patched where it was placed; one that keeps a list is brought up to date
and patched as before. A child of a clipping holder whose paint bounds no
longer meet the clip is not patched (patching would leave it drawn), and
records the holder instead.

## Pointer input

- `ui.pointer_moved(x, y)`, `ui.pointer_button(button, pressed)`,
  `ui.pointer_left()`, `ui.wheel(dx, dy)`: the host forwards these.
- **Ordering.** Input is handled at once, against the layout of the last frame,
  which is what the user saw. Handlers write signals; the next frame propagates
  them. After the next frame's layout, the hovered node is found again from the
  last pointer position, so a scroll, a removal or a move of the node under a
  still pointer updates hover without a pointer event. Hover is found once per
  frame: a hover that changes layout takes effect on the frame after, which may
  find another hovered node, and so on, once per frame.
- **Hit testing** walks from the root in reverse paint order. It prunes only at
  clipping nodes (a non-clipping parent's children can overflow it), and inside a
  stack it finds the candidates by binary search on the children's paint bounds.
  The topmost node with input wins; a node with none passes the pointer to what is
  under it.
- `ui.hovered(node)` and `ui.pressed(node)` are signals.
- **Clicks.** A press is owned per button by the node it landed on (a node handle
  with its generation); a release on the same node is a click and runs its
  `on_click(|ui| ..)`. The press is cancelled when its node is removed, when the
  pointer leaves the window, or on `ui.cancel_pointer()`.
- The wheel scrolls the innermost scroll container under the pointer by setting
  its offset signal, clamped to its content.

Follow-up (Phase 4, 2026-11-11): as built, a node's hover and press go to
signals the app makes and names in the spec (`Spec::hover(signal)`,
`Spec::press(signal)`), not to signals `ui.hovered(node)` returns: props are
given before the node exists, so a row's fill could not otherwise read the
row's own hover. `ui.hovered()` names the hovered node. A node takes the
pointer when it has either signal or a click handler; `on_click` gets the
button. `pressed` is true from the press to its release or cancellation,
wherever the pointer is. Removing a node writes its signals false, since they
outlive it. The hover is found again after each layout round of a frame, not
once after the frame's layout, so a scroll's new hover is drawn in the frame
that draws the scroll; a hover that changes layout settles within the frame's
four rounds. The wheel goes to the innermost scroll container under the
pointer whose axis the wheel moved along (a vertical wheel over a horizontal
row scroller goes to the column around it), and at its limit nowhere else.
Hit testing prunes a node that does not clip by its paint bounds, which hold
its children's boxes, and every node keeps its children's reach along the
stack (`child_grow`) for the binary search, not only nodes that clip.

## Phasing

0. `DrawList::append` and its oracle (done).
1. a) translation-invariant underline snapping; b) `append_translated` and its
   oracle: recording the same calls inside `translate` and `push_clip`, per kind,
   with deterministic counterexamples (a shadow culled by the clip, a tie at
   `-0.5`, stripes, segments under an affine, viewport clips) beside random
   sequences; c) `append`'s repaint note.
2. Signals, with tests: equality guard; a diamond runs each memo once; a branch
   switch to a memo upstream of a write in the same batch reads it fresh; a memo
   computing an equal value stops propagation; cycles panic; disposal leaves no
   edges, closures or queued work (counted), and a stale handle panics.
3. The tree, layout, drawing and the frame oracle: random trees, random sequences
   of sets, adds, removes and scrolls; after each, the frame's list against the same
   tree drawn from nothing (arrays, `viewport_clips`, and the frame comparison),
   and the counters against what changed (one changed leaf records one leaf).
   Layout tests for each sizing rule and for dirt stopping at a fixed-size node.
4. Pointer input, with tests for hover kept current by a scroll and a removal,
   overflowing children hit, clipped-away children not hit, press cancellation.
5. The comparison harness and the `rows` scene, immediate and retained: cold
   build, a frame with one change, a scrolled frame, a hovered row, an unchanged
   frame; memory held by caches.
6. The example in a window.

## Decisions

- **Rust, as a gameui usage mode, not Lua first** (Bart, 2026-11-10): a Rust app
  can use it without a script runtime; a script host can drive the tree later.
- **Its own crate** (`crates/retained`), so core gains only the append methods and
  the snapping fix (2026-11-10).
- **Signals are the core; setters are sugar over them** (Bart, 2026-11-10: "signals
  seem a lot more general than setters").
- **Moved subtrees are translated on append, not recorded again** (Bart,
  2026-11-10), by whole pixels (layout snaps positions).
- **Record-time snapping becomes translation-invariant**: `floor(x + 0.5)` instead
  of `round` for the underline (Bart, 2026-11-10). It differs from today only at
  exact negative half-pixel ties.
- **Every container caches its whole subtree**, kept despite both reviews, to be
  measured first (Bart, 2026-11-10). The alternatives on the table: per-node lists
  patched in place in the frame's list, or explicitly marked boundaries.
- **Lists only at the root and at nodes that clip** (Bart, 2026-11-11), once
  the comparison measured whole-subtree caches: the grid held 10,001 lists
  and 30 MB for records the root holds once. The rest are drawn into the
  nearest list above them, and their own drawing patched there. Supersedes the
  decision above.
- **Pointer input is in the first prototype; the comparison runs the grid scenes
  and a rows scene** (Bart, 2026-11-10).
- **Mark-then-pull propagation**, not height-ordered pushing: it stays glitch-free
  when dependencies change during a run (both reviews, 2026-11-10).
- **Caches are in local coordinates, replayed with `append_translated` always**,
  so ancestor clips apply on every replay (both reviews, 2026-11-10).

- **Pointer choices** (made 2026-11-11 while Bart was away; confirmed by
  Bart 2026-11-12): hover and press go to signals the app makes (the alternatives:
  binding a prop after the node exists, or spec closures given the node's own
  signals); only the topmost node taking the pointer is hovered, not its
  ancestors; the wheel stops at the innermost container's limit instead of
  chaining outward; `pressed` means the press is held, not held and over it.
- **A scroll puts off the layout of children away from its view** (Bart,
  2026-11-12): automatically, with no flag to opt in; laid out within one
  viewport of the view, before and after, not the view alone; for children
  of fixed length along the stack whose length across it is known (fixed,
  filling, or hugging at the start) in a scroll whose content's length
  across it is exact; and a child laid out before that changes while far
  from the view is put off too, not laid out at once.

  Follow-up (Bart, 2026-11-12): the margin is no less than half the window's
  length along the scroll's axis. The aim is to stop massive lists, not to
  restrict scrolls shorter than half a screen.

## Questions only building can answer

- **Memory and copy cost of whole-subtree caches.** A record is held once per
  boundary above it (depth times), and one changed leaf copies every visible
  record again at the root. Counted in the comparison (bytes held, records copied
  per frame) on the grid, the rows scene and a deep tree; settles whether the
  cache shape changes to per-node lists or explicit boundaries.

  Measured (2026-11-11, `bench/ui-compare/README.md`): the grid of 10,000
  cells holds 40,000 records (each twice) in 30 MB of lists against 6.9 MB
  for immediate mode's one list, most of it the cells' own small lists; the
  deep tree holds each leaf four times (40,000 records, 10.5 MB against 2.9).
  One changed cell copies all 20,000 of the grid root's records (0.88 ms of
  tree work at 10,000 cells), and one changed leaf of the deep tree copies
  11,100 (0.135 ms). The decision on the shape is Bart's (Outstanding).

  Settled (Bart, 2026-11-11): lists only at the root and at nodes that clip,
  with each node's own drawing patched in place (Done, and under "Drawing and
  the cache").
- **The frame floor.** What a retained frame with one change costs next to an
  immediate one, at each size: the point of the prototype.

  Measured: with one change the whole frame costs about what immediate mode's
  does on the flat grid (4.75 against 4.56 ms at 10,000 cells, the render
  being most of both), less in the deep tree (0.56 against 0.70), and more for
  a changed label in the rows (0.266 against 0.206, from shaping it twice).
  With nothing changed the tree does no work and a host can skip the frame.
  With everything changed it costs about twice immediate mode at 10,000
  cells.
- **Prop storage.** Bound props allocate a memo each; at 10,000 cells with a bound
  colour each, whether that allocation shows in the cold build.

  Not settled on its own: the rows scene binds a memo and a signal per row,
  but its build is almost all text shaping, so the allocation does not show
  in the profile.

## Review

Codex and Antigravity reviewed the first draft (2026-11-10). Applied:

- **The exact oracle as first stated was false** (both): underline `round` is not
  translation-invariant at negative ties, and float sums taken in another order
  differ. Fixed by the snapping change (Bart's choice), snapped layout positions,
  and stating exactness for dyadic coordinates.
- **The stationary replay skipped ancestor clips** (Codex): caches are recorded
  without them, so replay is always `append_translated`.
- **Per-kind representations** (both): the table above.
- **Shadows culled at record time** (both): `append_translated` drops them as
  recording does; ranges and run merging are rebuilt.
- **Viewport-dependent lists and paint bounds** (Codex): clipping nodes' lists are
  recorded again when what is in view changes; culling uses paint bounds.
- **Custom painters** (both): volatile when not a pure function of tracked reads;
  volatility keeps ancestors from replaying.
- **Stale repaint times** (both): deadlines are absolute, per node, in the frame's
  clock.
- **External context** (Codex): font generation, scale and viewport invalidate.
- **Glitches with dynamic dependencies** (both): mark-then-pull instead of
  heights.
- **Ownership, disposal and two-way edges** (both): owners, generations, graph id.
- **Separate measure / arrange / paint dirt; scroll clamping after layout**
  (Codex).
- **Layout rules** (both): constraints, Fill in Hug, overflow, weights, absolute
  children, the scroll axis.
- **Fractional positions would defeat translation** (Antigravity): layout snaps.
- **Bounded traversal** (Codex): binary search in stacks for culling and hits;
  nodes examined is counted.
- **Input ordering, hover after layout, press ownership and cancellation,
  overflowing children** (Codex).
- **`viewport_clips` and the oracle's gaps** (both): translated and compared; the
  generator gains stripes, transforms, tint, underlines, viewport clips, negative
  and fractional coordinates.
- **Cold build and teardown measured** (Codex, Antigravity's virtualisation
  point): in the comparison.

Not applied:

- **Change the cache shape** (both): Bart kept whole-subtree caches to measure
  them first; the alternatives are recorded under Decisions.
- **Remove record-time shadow culling from core** (Antigravity): `append_translated`
  reproduces it instead, leaving core's output as it is.
- **A closed enum of prop values in flat arrays** (Antigravity): constants are held
  inline already; whether bound props' allocations matter is a question above.
- **Virtualised scroll containers** (Antigravity): deferred; the rows scene measures
  what one node per row costs.
- **A damage interface to the renderer** (Antigravity): deferred; the renderer's
  comparison is about 1% of a recompute.
- **A pixel oracle on the GPU** (Codex): the record oracle compares the renderer's
  whole input; equal input to a deterministic renderer is equal output, and the
  frame-reuse pixel oracle already covers the renderer's own reuse.

## References

- `docs/design/frame-reuse.md`: the renderer's half, which a retained frame feeds.
- `crates/core/src/render/append_oracle.rs`: Phase 0's oracle.
- <https://blog.zortax.de/posts/a-critique-of-gpui>
- `~/Projects/gpuix` (`packages/native/src/retained_tree.rs`, `docs/lua-runtime.md`).
