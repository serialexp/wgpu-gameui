# Retained mode — Design

Status: partial — Phases 0 and 1 landed (`DrawList::append`, `append_translated`, 2026-11-10); Phases 2 to 6 outstanding. A prototype.
Owner: Bart
Last updated: 2026-11-10

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

### Outstanding

- [ ] **Phase 2 — signals.** Signals and memos with mark-then-pull propagation
      (no glitches with dynamic dependencies), equality guards, owners and
      disposal with edges removed both ways, cycle detection.
- [ ] **Phase 3 — the tree, layout and drawing.** `crates/retained`: nodes, props,
      stack layout with snapped positions, scroll containers, per-container cached
      lists in local coordinates replayed with `append_translated`, culling and hit
      search by binary search in stacks, external context (fonts, scale), clocked
      repaint deadlines, work counters. Oracle: every frame equals the same tree
      drawn from nothing with the same culling.
- [ ] **Phase 4 — pointer input.** Hit testing, hover and press as signals,
      clicks, wheel scrolling, hover kept current after layout, press
      cancellation.
- [ ] **Phase 5 — comparison.** `ui-compare-gameui-retained`: the grid scenes
      (`static`, `animated`, `one`) and a new `rows` scene (thousands of
      fixed-height rows in a scrolling column, one value changing; scrolling;
      hovering), gameui immediate against retained: time per frame, cold build,
      work and memory counters.
- [ ] **Phase 6 — an example in a window.** Scrolling list, hover styling, a click
      counter.
- [ ] **Question: memory and copy cost of whole-subtree caches** (below).
- [ ] **Defer: keyboard focus, text input, selection and highlight** in retained
      subtrees.
- [ ] **Defer: virtualised lists** (an item count and a row factory instead of a
      node per row).
- [ ] **Defer: a damage interface to the renderer** (it compares the whole list).
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

## Signals

- `ui.signal(value) -> Signal<T>` (a `Copy` handle: the graph's id, slot index,
  generation, type). `ui.set(signal, value)` stores it only if it differs
  (`T: PartialEq`); `ui.update(signal, |v| ..)` likewise.
- `ui.memo(|r: &Reader| -> T) -> Memo<T>`: every `r.get(s)` inside it records `s`
  (a signal or another memo) as a source. Sources are recorded again on every
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
- **Pointer input is in the first prototype; the comparison runs the grid scenes
  and a rows scene** (Bart, 2026-11-10).
- **Mark-then-pull propagation**, not height-ordered pushing: it stays glitch-free
  when dependencies change during a run (both reviews, 2026-11-10).
- **Caches are in local coordinates, replayed with `append_translated` always**,
  so ancestor clips apply on every replay (both reviews, 2026-11-10).

## Questions only building can answer

- **Memory and copy cost of whole-subtree caches.** A record is held once per
  boundary above it (depth times), and one changed leaf copies every visible
  record again at the root. Counted in the comparison (bytes held, records copied
  per frame) on the grid, the rows scene and a deep tree; settles whether the
  cache shape changes to per-node lists or explicit boundaries.
- **The frame floor.** What a retained frame with one change costs next to an
  immediate one, at each size: the point of the prototype.
- **Prop storage.** Bound props allocate a memo each; at 10,000 cells with a bound
  colour each, whether that allocation shows in the cold build.

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
