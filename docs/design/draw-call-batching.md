# Draw-call batching

**Status:** Done
**Last updated:** 2026-10-21

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

- [x] Phase 0: the `batching` bench group (CPU-only and GPU-wait timings) and the stats line in `frame_render`, in `crates/core/benches/ui_stress.rs` (`feat(core)!: draw-call batching`, 2026-10-16)
- [x] Before Phase 1: text and MSDF icons project with the exact logical size, as the other pipelines do (an existing bug), with a GPU test at a target size that doesn't divide by the scale (`feat(core)!: draw-call batching`, 2026-10-16; `text.rs`, `tests/text_projection.rs`)
- [x] Before Phase 1: a clip of zero size draws nothing (an existing bug; open question 3) (`feat(core)!: draw-call batching`, 2026-10-16; in the shaders, see the 2026-10-16 note under Phasing; `tests/zero_size_clips.rs`)
- [x] Before Phase 1: the `--no-default-features` library build is repaired (an existing bug), and `tests/external_sprite.rs` and `tests/text_contrast.rs` get `required-features = ["headless"]` (`feat(core)!: draw-call batching`, 2026-10-16; `file_dialog` gated behind `phosphor-icons`; `tests/widget_gallery.rs` also needed `required-features = ["headless", "phosphor-icons"]`; every target now checks with `--no-default-features`)
- [x] Phase 1: draw boxes for every kind of primitive except text and MSDF icons, mirroring the shaders, with unit tests (`feat(core)!: draw-call batching`, 2026-10-16; `render/draw_box.rs`, 35 CPU tests in `render/draw_box_tests.rs`, including one that copies the shader's edge maths to check the 0.61 px reach; reviewed, and the review's fixes are in, see the 2026-10-16 notes under "Per kind"; the module's `expect(dead_code)` went when Phase 3 used it)
- [x] Phase 1: a GPU test that no primitive changes a pixel outside its draw box (`feat(core)!: draw-call batching`, 2026-10-16; three `#[ignore]`d tests in `render/draw_box_tests.rs`, 36 scenes without text and icons, at scales 0.75 to 2, including flipped transforms, round joins and round-capped dashes; each was checked to fail with the chrome reach cut to 0.45 or 0.55 px, the circle quad's 2 px cut to 0.2, or the segment pad cut to 0.2)
- [x] Phase 2: the CPU side of text and MSDF icon preparation is split from the GPU upload and keeps each unit's vertices and a box per block, building vertices only once the atlas is final (with a test where a later unit grows the atlas); text and MSDF icons join the GPU box test (`feat(core)!: draw-call batching`, 2026-10-16; see the 2026-10-16 note under Phasing)
- [x] Phase 3: `DrawOrder`, order from overlap, with unit tests against a brute-force oracle (including units with several boxes, some of them empty) and a count of its own work (`feat(core)!: draw-call batching`, 2026-10-16; `render/draw_order.rs`, 14 tests in `render/draw_order_tests.rs`; see the 2026-10-16 note under Phasing)
- [x] Phase 4: `DrawPlan`, the batches for a draw list, with CPU tests (`feat(core)!: draw-call batching`, 2026-10-16; `render/draw_plan.rs`, `render/draw_plan_tests.rs`; the button grid plans as 2 batches and the scroll view as 3; see the 2026-10-16 note under Phasing)
- [x] Phase 5: the renderer uploads and draws from the plan; the paint-order path is the identity plan; `RenderStats` counts real draws, with the new counters (`batches`, `units_ordered`, `order_overlap_tests`, `order_cell_visits`) and their debug report lines; the `paint-order-oracle` feature and its `publish.yml` checks; `render_widget_gallery` turns on `set_check_plan`; the `viewport` contract stated on `render` and `render_layers`; `RenderStats` `#[non_exhaustive]`, committed as breaking; existing tests moved as described (`feat(core)!: draw-call batching`, 2026-10-16; `render/batch_draws.rs`, the renderer in `render/ui_renderer.rs`, `MsdfAtlas` and `upload_vertices` in `text.rs`; see the 2026-10-16 note under Phasing)
- [x] Phase 6: GPU pixel tests (batch order matches paint order), an allocation test, bench cases reporting batch counts (list rows at scale 1.0 and 1.5, a dense chart, a 10k-point polyline), and bench numbers recorded here; docs promising strict paint order updated; TODO entries closed (`feat(core)!: draw-call batching`, 2026-10-16; `tests/batch_order.rs`, the allocation test in `render/draw_plan_tests.rs`, the `batching` bench group; numbers taken on a busy machine, see the 2026-10-16 notes under Phasing)

- [x] Follow-up: a list of one key isn't ordered through the grid (2026-10-21; `DrawOrder`'s `Keys` and `pending`, see the 2026-10-21 note under Phasing)

### Outstanding

- [ ] Rerun the `batching` bench times on a quiet machine (deferred: until one is free; the 2026-10-16 times were taken at load average 10 to 25, see TODO.md "Chart benches on a quiet machine")
- [ ] Follow-up in TODO.md: the cost of boxing soup per triangle (deferred: Bart to decide)

## Why this exists

gameui draws a `DrawList` in strict paint order. A screen of buttons alternates
"box, text, box, text", and every change of kind is a new draw call, so 1,000
buttons cost 2,000 draw calls. gpui (Zed's UI library) gives each primitive an
order number from what it overlaps, then draws every box that overlaps nothing
in one batch and every glyph in another. The result looks the same, because
draws that don't overlap can go in any order.

This doc is the contract for doing the same in gameui: draws that can't change
the same pixel may be regrouped, and draws that can keep their paint order.

### Measurement (2026-10-16, Apple GPU, Metal)

`cargo bench -p wgpu-gameui --bench ui_stress -- batching` draws N boxes
(`chrome_rect`), each with an "OK" label, none overlapping. "Interleaved"
records them box, text, box, text. "Grouped" records every box and then every
label: the same pixels in two runs.

- "Encode" is the CPU's work: building, uploading, encoding and submitting.
  Its loop doesn't wait for the GPU, so some GPU backlog may leak into it.
- "Wait" adds waiting for the GPU.
- The 10k grid is about 6400×2800 on a 1920×1080 target, so most of it is off
  screen.

| N | encode, interleaved | encode, grouped | wait, interleaved | wait, grouped | draw calls | buffer writes |
|---|---|---|---|---|---|---|
| 100 | 0.63 ms | 0.82 ms | 6.3 ms | 5.0 ms | 200 → 2 | 102 → 3 |
| 1,000 | 6.6 ms | 0.80 ms | 10.9 ms | 6.2 ms | 2,000 → 2 | 1,002 → 3 |
| 10,000 | 60.0 ms | 16.7 ms | 73.3 ms | 21.1 ms | 20,000 → 2 | 10,002 → 3 |

At 100, grouped encoding comes out slower than interleaved. I haven't
explained that yet; Phase 6 re-measures with on-target unit counts.

The interleaved frame's extra cost isn't only the draw calls. Each text run
also builds its vertices separately (taking the font lock and allocating) and
writes its own buffer. Batching removes all three, which is why the plan also
changes how text is prepared.

Batching also skips units that draw nothing (off target or clipped away), and
that alone cuts the 10k case. Phase 6 records on-target unit counts next to
the times, so the two gains can be told apart.

These numbers are from Apple Metal. The older "`frame_render` is ~140× slower"
TODO entry was measured on an RX 7900 XTX.

Real `Button`s (`frame_render`) record 4 primitives per button in 2 runs:
3 analytic records (plinth, face, inset highlight), then the label. 1,000
buttons give 2,000 runs and 2,000 draw calls. So the TODO's "P1 — Measure
draw-call batching before building anything" is answered: it's worth building.
Bisecting the June slowdown is a separate job.

## Goals

- Units that can't change the same pixel as anything recorded between them
  can share a draw call. How many do is what the order rule below gives, as
  with gpui; it isn't a guarantee for every pair. (For example, with key X
  sorting before Y: W of key Y, then A of key X over W, then C of key X
  touching nothing. A gets order 1 and C order 0, with W at order 0 between
  them in the sort, so A and C land in different batches.)
- A frame drawn in batch order matches the same frame drawn in paint order, to
  the pixel. This covers clips, transforms, fractional scale factors, a view
  origin, shadow blur, strokes, text effects, sprites in textures of their own,
  and YUV video sprites.
- The extra CPU work grows about linearly with the number of units (boxes are
  computed for every unit, on the target or not), and is far smaller than the
  work it saves. This is checked with counts, not timing asserts:
  - `DrawOrder` counts its own work (overlap tests and cell visits), and a
    Phase 3 test asserts it stays within a fixed amount per unit for
    list-like layouts of 1k and 10k units, all on the target (a large target,
    with the number of units ordered asserted too, so culling can't make the
    test pass);
  - the bench reports that count for the dense chart, along with batches,
    draw calls and buffer writes.
- Building the boxes, the order and the plan allocates nothing per frame once
  warmed up, and nor do the uploads that batching replaces. wgpu's own encoding
  is out of scope.
- Batching is always on. A paint-order path exists only for tests.

## Non-goals

- Changing the `DrawList` recording API or what widgets record.
- Reordering across `LayerStack` layers: each list keeps its own pass, in order.
- The two compatibility paths: a list filled through the public arrays with no
  paint stream, and soup appended after the last command. Both keep their own
  passes, drawn in paint order.
- Merging different families into one pipeline (glyphs and boxes in one
  shader).
- Splitting one soup command per triangle (see open questions).
- Snapping layout to whole device pixels (TODO "P3 — Snap layout to whole
  device pixels"), though it would help batching at fractional scales.
- Bisecting the June `frame_render` slowdown.

## Domain model

### What exists today

- **`DrawList`** (`crates/core/src/widgets/draw_list.rs`) keeps one array per
  family:
  - `vertices`/`indices`: soup, with clips interpolated across each triangle;
  - `analytic_instances`: chrome, shadows, stripes and stroke segments, tagged
    by `kind`;
  - `circle_instances`, `nine_slices`, `icons` (sprites, images, YUV) and
    `texts`;
  - `icons_msdf`, behind the `phosphor-icons` feature.
- **`paint_cmds: Vec<PaintCmd>`** is the paint order. Each command is a range
  into one family's array:
  - `push_paint_cmd` merges a command into the previous one when they're the
    same family and their ranges touch;
  - circles merge in their own code (`draw_list.rs` ~2147);
  - one `Soup` command is all the soup recorded between two other commands.
- **`UiRenderer::render_one`** (`crates/core/src/render/ui_renderer.rs`):
  1. `prepare_texts` builds every text vertex to settle the glyph atlas, then
     discards them (`text.rs` ~933-944). `prepare_icons` only generates
     glyphs; icon vertices are built per run in `upload_icons` (`text.rs`
     ~994-1045).
  2. It uploads each colour family's array as it is (`upload_ordered_color`).
  3. `upload_ordered_runs` uploads each textured or text command as its own
     run, and each text run builds its vertices again:
     - sprite and nine-slice runs split where the texture changes;
     - sprites that can't be resolved, and nine-slices backed by YUV, are
       skipped.
  4. It draws the runs in order in one render pass. That covers only the main
     paint stream: trailing soup and the no-stream path open passes of their
     own.
- **Text draws in three sweeps** over the blocks it is given: every shadow,
  then every glow, then every fill and outline (`text.rs` ~1129-1197). So
  within one `Text` command, block B's shadow sits under block A's fill. Text
  and MSDF icons share a pipeline but use different atlases.
- **`debug.rs` computes a box for every primitive** (~804, 845, 1333, 1360,
  1388, 1429, 1520, 1537). Those boxes measure layout and ink, not what the
  shaders draw, so they must not be used for ordering. Only their decode
  helpers can be shared (the affine from a payload, `clip_from_parts`).
- **What the pass relies on, all checked:**
  - Each draw path binds everything it needs (`ui_renderer.rs` ~2149-2189,
    ~2277-2289; `text.rs` ~1074-1080).
  - No scissor, stencil or depth is used, and multisampling is off.
  - There's no `set_viewport`, so the projection maps to the whole attachment,
    and the size passed in must equal the attachment's.
  - Every pipeline projects the same logical window. Today that isn't quite
    true, and batching depends on it: `TextRenderer::resize` rounds the
    logical size down to whole pixels (`text.rs` ~704-710), while the other
    pipelines use the exact value (`ui_renderer.rs` ~1644-1647). A 1001 px
    target at scale 1.25 gives text 800 logical px against 800.8, which puts
    text up to about 1 physical px further right or down than everything
    else. Glyph snapping (`text.rs` ~1419-1422) uses the exact scale, so
    snapped glyphs also land off the pixel grid at such sizes. This is fixed
    first, as a change of its own: the text renderer keeps the logical size
    as f32.
  - Blending is straight-alpha source-over (`ALPHA_BLENDING`), so a fragment
    with alpha exactly 0 leaves the target unchanged, whatever its format.
  - Transforms are `Affine2`, with no perspective.
  - The offscreen layer and backdrop blur run outside `render_one`.

### New: draw boxes

A draw box is the range of physical pixels a unit could change. It's computed
by a Rust function per kind that mirrors that kind's shaders. Every box is
taken from the min and max of its corners, so negative sizes and flips are
handled.

**Two kinds of edge.** Every bound is widened by a float margin. Positions go
through f32 world coordinates and the view-origin projection
(`ui_renderer.rs` ~1646-1648), so at a world y of about 5e4 a quad can move by
about 0.004 logical px. The margin, in physical px, is
`8 · f32::EPSILON · max(|x|, |y|, |origin x|, |origin y|, target width,
target height) · scale`, which is several times the error of the steps a
position goes through. Phase 1's GPU box test checks it at coordinates around
1e5.
- A hard edge is all-or-nothing: clip tests, crisp stripes and triangle edges.
  - Clip tests keep a pixel that lies exactly on the edge: they discard only on
    strict `<`/`>` (`ui.wgsl` ~61-63, 241-242, 316-317, 426-427, 495-496,
    643-644, 814-816, 976-978; `ui_msdf.wgsl` ~104).
  - Crisp stripes use `<=` (~510).
  - The rasterizer snaps triangle edges to a sub-pixel grid. Vulkan only
    promises 4 bits of sub-pixel precision, so an edge can move by about
    1/32 px.
  - So a hard edge's bound is inclusive, widened by 1/16 px plus the float
    margin.
- A soft edge fades out through anti-aliasing. Its bound is where the
  shader's formula gives coverage of exactly 0 (for example
  `clamp(0.5 - d / footprint, 0, 1)` is 0 once `d >= footprint / 2`), plus the
  float margin. Alpha that is exactly 0 changes nothing on any target format,
  so the bound doesn't depend on the target's precision.

**Per kind:**
- **Chrome and stripes:**
  - "No transform" uses the shader's own exact test, `p0 != (1, 0, 0, 1)`,
    with no tolerance.
  - With no transform, the box is the rect widened by 0.61 physical px plus
    the float margin, on every side.
    - Along a straight edge, `edge_coverage` (`ui.wgsl` ~181-184) reaches
      half a physical pixel.
    - Near a corner, `box_sdf`'s `length()` branch (~189-196) has a diagonal
      gradient, so it reaches further. The maximum, evaluated from the
      shader's formulas, is 0.604 px, at radius 0. It falls as the radius (in
      physical px) grows: 0.587 at 0.25, 0.559 at 1, 0.525 at 4.
    - A fixed 0.61 px covers every radius, needs no evaluation per corner, and
      still keeps shapes 2 px apart from chaining.
  - With any transform, it's the shader's padded quad (2 screen px, ~142-148).

  > **2026-10-16 update:** the 0.61 px reach holds only for radii of 0 or more
  > and finite fields. A negative radius reaches further (0.68 px at -0.6),
  > and a NaN colour gives a NaN alpha, which `alpha <= 0` doesn't discard.
  > `DrawList` never records either, but `analytic_instances` is public, so
  > such a record gets the padded quad instead (`reach_holds` in
  > `draw_box.rs`). A CPU test copies `edge_coverage` and `box_sdf` and
  > checks the reach on a 1/1024 px grid, since the GPU test reads 8-bit
  > pixels and can't see coverage below 1/510. `ui.wgsl` now points at
  > `CHROME_REACH`, and its old "~0.7px" comment is corrected.
- **Shadows:** the stored raster rect (`draw_list.rs` ~2314-2325), transformed.
- **Stroke segments:** the end points plus end reach (`p5.zw`), half width
  (`p1.w`) and the shader's `pad`: 1 logical px with no transform, or
  `2·|M| / |det M|` with one (`ui.wgsl` ~121-133).
- **Circles:** centre ± (radius + max(thickness, 0)/2 + 2), the shader's own
  quad (`ui.wgsl` ~300).
- **Soup:** the union of each triangle's box. Clips are interpolated across a
  triangle, so each triangle is cut by the bounding box of its three
  vertices' clips, or left uncut when only some of them clip. (An
  interpolated clip can keep a pixel that lies in that bounding box but in
  none of the three clips.) `radial_gradient`'s fan
  reaches past its rect and relies on that clip (`draw_list.rs` ~1556).

  > **2026-10-16 update:** when all three clips have a width of zero or less
  > (or all three a height), every interpolated clip does too, so the
  > triangle draws nothing and its box is empty.
- **Sprites (icons, images, YUV) and nine-slices:** the box around the
  transformed corners.
- **Text:** its glyph quads (including outline, glow and the shadow offset),
  each cut by its own block's clip. They're taken from the prepared vertices
  (see Phase 2), never estimated from the `TextBlock`.
  - A unit holding several blocks keeps one box per block for the overlap
    test, while still having one sort entry. So far-apart shadowed labels in
    one command don't cover the screen between them.
- **MSDF icons:** the quad the vertices are built from, which is
  `fit_centered(icon.local, w_em, h_em)` (`text.rs` ~1685), not `icon.local`.
  The em metrics include the tile padding (`glyph_msdf.rs` ~158, ~205-212),
  about 0.2 em per side at `ICON_REF_PX` 64, so the quad reaches well past
  `icon.local`. Like text, the box comes from the prepared vertices (Phase 2).

  > **2026-10-16 update:** measured, the quad reaches past `icon.local` along
  > one axis only. `fit_centered` keeps the tile's aspect, so the quad
  > matches the rect along one axis and overhangs it along the other; for a
  > phosphor icon in a 16 px square at (20, 20), the box's rows are exactly
  > 20 to 36 and its columns reach past both sides. The CPU test
  > `msdf_boxes` in `render/draw_box_tests.rs` asserts that, and that a clip
  > cuts the overhang.

Every kind is cut by its own clip, with an inclusive intersection: a clip of
zero width or height still keeps the pixels exactly on its edge, since the
shaders keep them. `Rect::intersection` returns `None` for a zero area
(`layout.rs` ~73), so it isn't used for this. `DrawList` makes such clips
whenever nested clips don't overlap (`draw_list.rs` ~1210, ~2829); see the open
questions.

> **2026-10-16 update:** with open question 3 answered, the shaders now
> discard everything inside a clip of zero width or height (`outside_clip` in
> `ui.wgsl` and `ui_msdf.wgsl`). So a box cut by such a clip is empty, and its
> unit is culled. A clip of positive size still keeps the pixels exactly on its
> edge, so the intersection stays inclusive for those.

The clip flags live in different fields:
- `p1.z`, with the clip in `p8`, for analytic records;
- `params.x` for circles;
- `flags.x` for icons;
- `tp.z` for nine-slices;
- per vertex for soup and text.

Every box is then cut to the target as the renderer sees it (the scale factor
and `set_view_origin`), and turned into the range of physical pixel centres it
covers.
- An empty box draws nothing. That covers content off the target or clipped
  away, which is most of a long list.
  - For a unit with several boxes (see units), empty boxes are dropped from
    its list, and lookup and storing use the boxes that remain.
  - A unit is culled (not ordered, not drawn, not counted in `units_ordered`)
    only when every one of its boxes is empty. So a shadowed text command
    with one block scrolled out of view still draws its other blocks.
- A NaN or infinite box counts as touching the whole target, for that box
  alone. It costs batching, never correctness.

Two units overlap when their pixel ranges share a pixel.

Phase 1 tests each box function against hand-worked boxes. A GPU test then
draws each kind alone and checks that no pixel outside its box changes. It
covers:
- scale factors 1, 1.25, 1.5 and 2, and a non-zero view origin;
- fractional positions, swept across a pixel in small steps, especially at
  corners;
- rotation and uneven scale;
- clip edges on pixel centres, and a zero-size clip;
- a target whose size doesn't divide by the scale (for example 1001×601 at
  1.25), with content near the far edges;
- positions and a view origin around 1e5;
- from Phase 2, text with every effect, and MSDF icons.

**Glyph tiles carry generous padding.** It's `px_range.ceil() + 1` texels
(`render/glyph_msdf.rs` ~158). Most UI text uses hinted tiles, oversampled 3×
(`HINTED_OVERSAMPLE`), so that's about 4.3 device px at any size, or about
2.9 logical px at scale 1.5. Text boxes use the full tiles, so they will chain
in tight layouts:
- in rows under about 22 px, descender tiles reach into the next row's
  background, giving about 2 batches per row;
- a button whose label fills its face behaves the same way, so the button
  grid's 2 draw calls hold only for short labels like the bench's "OK".

A tighter text box is a deferred follow-up, not part of this design. Glyph ink
estimated from the outline isn't safe, because MSDF fields have artefacts on
particular glyphs. Proving a box from the field itself needs its own design.
Phase 6's list-row batch counts show whether it's worth doing.

### New: units

A unit is what gets one order and one box:
- each instance of an instanced family: one analytic record, one circle, one
  sprite, one nine-slice, one MSDF icon;
- each `Soup` command;
- for text, each `PaintCmd::Text` as stored, whole: back-to-back `text()`
  calls are already merged into one command (`draw_list.rs` ~2488-2492), and
  the sweeps run over that merged command today. Its three sweeps run over all
  its blocks, so splitting it (or merging text from different commands into
  one sweep) would change pixels. There's one exception: when no block in a
  command has a shadow or glow, each block is its own unit, since the sweeps
  then reduce to one fill pass in block order.

When a batch holds several text units, their vertices go one unit after
another, each unit with its own sweeps, all in one draw call.

### New: draw order from overlap (`DrawOrder`)

This adapts gpui's rule (`zed/crates/gpui/src/bounds_tree.rs`), using exact
box tests. Units are taken in paint order, and each one gets an order from the
earlier units it overlaps:

    order(B) = max over earlier units U that overlap B of
               order(U) + (1 if U's batch key differs from B's, else 0)

It is 0 if B overlaps none. Units are later sorted by (order, batch key, paint
index).

Why this keeps paint order wherever two units overlap. Let U come before B and
overlap it:
- If their keys differ, `order(B) > order(U)`, so B sorts after U.
- If their keys are the same, `order(B) >= order(U)`. A higher order sorts
  after. An equal order puts both in the same (order, key) group, and within a
  group the paint index decides.
- The frame is drawn exactly in the sorted sequence, whether units share a
  draw call or not.

Compared with gpui's "one more than any overlap", same-key units don't climb:
stacked row backgrounds, or a polyline's segments, stay on one level. For the
button grid:
- each plinth is order 0, and the face and inset highlight on top of it are
  order 0 too, because they're the same key;
- each label overlaps them, so it's order 1;
- that makes 2 draw calls for any number of buttons with short labels. Phase
  4 asserts it. (Labels whose glyph tiles reach a neighbouring button chain;
  see the note on glyph tiles above.)

The lookup has to find the highest order among earlier overlapping units
quickly. It uses a reused uniform grid over the target in physical pixels,
64 px cells to start:
- Each cell keeps the units whose boxes touch it, plus two summaries: the
  highest (order, key) among them, and the highest order among units whose
  key differs from that one.
- For a query of key K, the most a cell could give is:
  - `max(top order, other order + 1)` when K is the top key;
  - `top order + 1` otherwise.

  A cell is skipped when that bound is no more than the best found so far.
  So a cell holding only units of the query's own key, such as a polyline's
  segments or stacked row backgrounds, costs one comparison at order 0, or
  once an earlier cell has reached its order. Otherwise it is scanned; this
  affects speed only, never correctness.
- A box reads and writes every cell it covers. In each cell it tests the unit
  holding the top order first, as gpui does (`bounds_tree.rs` ~144-150), then
  the rest, testing real overlap.
- A unit with several boxes (a shadowed text command keeps one per block)
  has one order: the maximum over all its boxes. Every box is looked up
  first, and only then is each stored in its cells, with that final order.
  Storing a box as soon as it's looked up would give its cells an order
  that is too low whenever a later box of the same unit finds a higher one,
  and a later unit overlapping only that first box would then sort too
  early. Overlap with an earlier unit means overlap with any of its boxes.
- When a unit is added and becomes a cell's top with a different key, the old
  top's order moves into the "other keys" summary. An overestimate there only
  costs speed; forgetting the old top would give a wrong order.
- Boxes are clamped to the target, so a box covers at most every cell, and
  large boxes need no separate list.

The risk is a dense cell, such as a chart with thousands of shapes in a small
area. There, the overlap tests grow with the square of the number of units in
the cell. The bench gets such a case. If it shows the grid is too slow, gpui's
R-tree replaces it behind the same type.

### New: the plan (`DrawPlan`)

The plan works on the CPU only. It takes the units (box, order, batch key,
paint index) and gives the batches:
- Units are sorted by (order, batch key, paint index) with `sort_unstable`.
  The paint index makes every key unique, so stability isn't needed.
- Each stretch of the sorted list with the same batch key is one batch. A
  stretch can cross order levels: units are drawn in the sorted sequence
  either way, and the GPU draws the instances of one call in order.
- The batch key is the pipeline plus what it samples:
  - for sprites and nine-slices, the texture (YUV sprites already have their
    own `Binding::Yuv` and pipeline);
  - for text and MSDF icons, the atlas.

The planner is a pure function, so `cargo test` checks it without a GPU:
- every overlapping pair keeps its paint order;
- orders and batch counts match a brute-force oracle on random layouts.

On real widget lists, the boxes for chrome, circles, soup, sprites and
nine-slices come from Phase 1's functions without a GPU. Text and MSDF icon
boxes do too, once Phase 2 has split their preparation from the GPU. Sprite keys come from
the texture each draw resolves to, so the tests stand in a texture per sprite
name. The tests assert the batch count for the button grid (2) and a scroll
view. The widget gallery's lists are built only inside `render_widget_gallery`
(`tests/widget_gallery.rs` ~2626-2680), which needs a real renderer for its
sprites, nine-slice and video, and draws through `LayerStack`; so for the
gallery, the `#[ignore]`d GPU test `render_widget_gallery` turns on
`UiRenderer::set_check_plan(true)` (see the paint-order wiring below) before
rendering its pages. Each list's plan is then checked against the
brute-force oracle inside `render_one`, rather than against a fixed number. If text
preparation can't be split from the
device cleanly, the tests on real lists that need text become GPU tests
(`#[ignore]`), and the planner tests use synthetic text boxes.

### New: drawing from the plan

- **Text and icon preparation keep their work.** The CPU side of preparation
  (placements, atlas packing, vertices and boxes) is split from the GPU
  upload:
  - `prepare_texts` takes the text units and keeps each unit's vertices (per
    sweep) and box in reused scratch, instead of building everything and
    discarding it;
  - MSDF icons get the same, instead of building vertices per run;
  - batches copy those ranges into one upload, which also removes today's
    second build per run;
  - every unit's glyphs and icons go into the atlas first, and only then are
    any vertices built, against the final atlas size. A UV has to use the
    final size, because generating a glyph can grow the atlas (`text.rs`
    ~1088-1094). That is why `build_vertices` places every block before it
    works out UVs (~1129), and why `prepare_icons` (~948-967) finishes the
    atlas before `upload_icons` builds any quads (~1016-1031). Preparing
    units one at a time would leave earlier units with UVs from before a
    later unit grew the atlas. A Phase 2 test has a later unit grow the
    atlas;
  - the atlas is therefore settled for the whole list before any of its
    batches. It is not settled for the whole frame: `render_layers` calls
    `render_one` once per layer (`ui_renderer.rs` ~1470-1472), and a frame
    can hold several `render` calls, so a later list can still grow it. Each
    list's text and icon batches keep the atlas bind group from their own
    upload, as `PreparedMsdf` does today (`text.rs` ~1041, ~1061-1066).
- **Each family uploads once per list, in batch order.** That's analytic
  instances, circles, sprites, nine-slices, text vertices and MSDF icon
  vertices.
  - Each batch draws a range of its family's upload, so it needs no buffer
    write of its own.
  - Soup uploads its vertices as they are, and its indices in batch order.
  - A map from each draw to its instance allows for the sprites and
    nine-slices that are skipped because they can't be resolved.
- **Scratch kept on the renderer:** boxes, orders, sort keys, grid cells and
  their unit lists, one reorder buffer per family, and the text and icon
  vertex scratch.
  - All of it is cleared, not freed, each frame. Memory is the high-water
    mark, and copied analytic records are the largest part, at 176 bytes each.
  - The per-frame allocations on the paths this replaces go too: the fresh
    `Vec`s in `build_icon_instances`, `build_nine_slice_instances`,
    `upload_ordered_runs`, `instanced_runs` and `build_vertices`; the
    `format!` for each YUV nine-slice (`ui_renderer.rs` ~1907) and each
    missing sprite in `warn_missing` (~874); `note_small_texture`'s
    `to_owned()` ahead of its `contains` check (~2263-2275); and
    `icon_font_snapshot()` collecting a new `Vec` on every call
    (`render/icon_font.rs` ~162-167), from `prepare_icons` and from each
    `upload_icons` run.
  - The allocation test calls the CPU-side pieces directly, not
    `render_one`: the box functions, `DrawOrder`, `DrawPlan`, the CPU side of
    text and icon preparation, the builders that fill each family's reorder
    buffer, and the step that turns batches into draws (the texture runs and
    the small-texture counting). That step is split out of today's
    `instanced_runs` / `note_small_texture` (`ui_renderer.rs` ~2220-2275) as
    a CPU function so the test can reach it. wgpu's own calls (encoding,
    buffer writes) are then outside the test without special handling.
  - The test counts allocations on its own thread only, so other tests
    running in parallel don't disturb it.
- **`RenderStats`:**
  - `draw_calls` is counted where draws are encoded. That also fixes today's
    overcount when every sprite in a command is skipped (~2380-2398).
  - New public counters go in, so benches and integration tests (separate
    crates) can read them:
    - `batches`;
    - `units_ordered`: units that were ordered, meaning those with at least
      one box on the target and not clipped away (`primitives` counts
      everything recorded);
    - `order_overlap_tests` and `order_cell_visits`: `DrawOrder`'s own work.

    All of them also go into the debug report's text and JSON output and its
    test fixture (`debug.rs` ~2188, ~2307, ~3565).
  - This breaks the public API. `RenderStats` is a public struct with only
    public fields (`ui_renderer.rs` ~291-338), so callers can build it with a
    struct literal, and `fragmentation_ratio()` changes meaning. The crate
    is at 1.0.0 and just-release picks the version bump from commit types,
    so the Phase 5 commit is marked breaking (`feat!`). In the same change,
    `RenderStats` becomes `#[non_exhaustive]`, so later counters don't break
    it again. The release turns 1.0.0 into 2.0.0, so agent-ui's
    `wgpu-gameui = "1.0.0"` (`crates/desktop/Cargo.toml` ~19) must move to
    `2` when the 2.0.0 release commit lands in the checkout, not with the
    Phase 5 commit (until then the checkout is still 1.0.0, and `"2"` would
    not resolve). Its `[patch]` only applies to a matching version. Left at
    `1.0.0`, the build would break: the desktop would get crates.io 1.0.0
    directly and the 2.0.0 checkout through `wgpu-gameui-media` (which takes
    it by path), and media's `PictureSink` uses the checkout's `SpriteId`
    (`crates/media/src/source.rs` ~36-43), so the two copies' types
    wouldn't match.
  - `paint_runs` keeps counting the recorded runs.
  - `color_runs`, `text_runs` and `icon_runs` count batches of each family.
  - The "primitive-family alternation" warning moves from `paint_runs` to
    draw calls per primitive, so it stops firing once batching has fixed the
    problem. The public `fragmentation_ratio()` moves with it (draw calls per
    primitive), and so does its line in the debug report.
  - `small_texture_batches` counts draw calls of small sprites in textures of
    their own, as its doc says, now taken from batches.
  - Growing the text vertex buffer starts counting in `buffer_reallocations`.
    Today it doesn't (`text.rs` ~1255-1274).
- **The paint-order path is the identity plan.** It's the same upload and draw
  code, with batches that reproduce today's runs.
  - There is one batch per paint command, split where the texture changes,
    after unresolved draws are dropped (as `textures::batches` does over the
    resolved bindings today, `ui_renderer.rs` ~2229, `textures.rs`
    ~908-921).
  - It never reads boxes or orders: no culling, and every command is drawn.
    If it shared the box code, a box that was too small would cull content on
    both paths, and the pixel comparison couldn't see it.
  - So there's still only one upload path to maintain. Pixel tests that check
    against fixed expected colours, rather than against this path, cover the
    shared upload code.
- **Wiring the paint-order path for tests.** A crate feature
  `paint-order-oracle` exposes two test switches:
  - `UiRenderer::set_paint_order(bool)` draws through the identity plan;
  - `UiRenderer::set_check_plan(bool)` makes `render_one` run the
    brute-force oracle on the units it just planned, and panic on any
    difference in orders or batch count. The oracle is library code compiled
    under `cfg(any(test, feature = "paint-order-oracle"))`, not in a
    `*_tests.rs` module, so both the unit tests and `render_one` can call it. Integration tests can't reach
    `DrawOrder`, `DrawPlan` or the oracle directly, so this is how the
    gallery's lists get checked.

  The crate
  turns it on for its own tests through a dev-dependency on itself, because
  integration tests build as a separate crate and `#[cfg(test)]` doesn't reach
  them. This works with resolver 3, and publishing strips path-only
  dev-dependencies.
  - The dev-dependency is
    `wgpu-gameui = { path = ".", default-features = false, features = ["paint-order-oracle"] }`.
    Without `default-features = false` it would turn the default features back
    on, so a `--no-default-features` test run would quietly test with
    `phosphor-icons` on.
  - The feature is then on in benches and in every `cargo test` build. There's
    no CI on pushes or pull requests today: only the release gate,
    `publish.yml`, builds with `cargo build --all-targets` and runs
    `cargo test --lib`. So the checks go into
    the local verification this work runs and into `publish.yml`:
    - a library-only clippy run, without the feature;
    - `cargo build -p wgpu-gameui --lib --no-default-features` (`IconMsdf`
      sits behind `phosphor-icons`). A `--no-default-features` test run would
      fail for another reason: `tests/external_sprite.rs` and
      `tests/text_contrast.rs` use `HeadlessGpu` without `required-features`
      (`crates/core/Cargo.toml` ~98-142). Those two get
      `required-features = ["headless"]`, and so do any new GPU test targets
      that use it, each when it is added (the first comes before Phase 1).
      (`markdown_gallery.rs` gates itself. Whether
      `widget_gallery` builds with `phosphor-icons` off can't be known until
      the library does.)
- **The `viewport` contract.** The size passed to `UiRenderer::render` must
  equal the attachment's. Nothing checks this, and nothing can as the API
  stands: wgpu 24's `TextureView` can't report its size, and `render` takes
  only the view. The offscreen layer is created from `viewport` itself
  (`composite.rs` ~153-156), so checking against it proves nothing. With
  batching, breaking the contract could misorder draws, not just stretch the
  frame, so the docs of `render` and `render_layers` (which takes the same
  `viewport`, `ui_renderer.rs` ~1447-1472) state it. Changing the API to take something
  that carries the size is out of scope.
- **Existing tests:**
  - `tests/ordered_paint_stress.rs` uses zero-alpha soup placed off to the side
    as an ordering barrier (~130-157), and asserts draw counts
    (`reference_stats.draw_calls > PAIRS*2` at ~157, `ROWS*2+1` at ~247-249).
    Its first test's reference list renders on the paint-order path (on the
    batch path its barriers overlap nothing, so its box draws would collapse
    into the same single draw as `unified`, plus one batch of barriers, and
    the pixel comparison would compare a draw with itself), and `unified` renders on both paths. Its count
    asserts are checked on the paint-order path. Its
    `many_alternating_runs_do_not_produce_screen_sized_triangle` (~168-197)
    guards the upload offsets this work rewrites, so it runs on both paths.
    Its third test keeps one list with count asserts and fixed-colour checks
    (~247-250); it renders on both paths, with counts checked on the
    paint-order path and colours on both.
  - `tests/render_passes.rs` keeps passing, since `paint_runs` doesn't change.
    But on the batch path its text is culled, so it no longer tests that
    alternating runs keep their order. It runs on both paths.
  - `tests/analytic_shadow_performance.rs` keeps passing: it's all analytic
    records on the target.
  - Stats asserts that keep passing, checked:
    - `tests/icon_instancing.rs` ~327 (`small_texture_batches == 1`);
    - `tests/multi_pass_render.rs` ~459-469 (reallocations; the text buffer
      starts at 4096 vertices);
    - `tests/render_stats.rs`.
  - Moving the fragmentation ratio breaks these, which are updated:
    - `debug.rs` ~3603, which expects 1.3 from `paint_runs`;
    - `ui_renderer.rs` ~2781-2794;
    - `pressure_detector_requires_sustained_bad_frames_and_warns_once`
      (`ui_renderer.rs` ~2823-2834), whose bad frame is `paint_runs: 140`
      with `draw_calls` 0, so it would never fire;
    - the `fragmented` value in the small-texture test (~2813-2819), which
      would stop being fragmented and pass for no reason.

## Phasing

1. **Phase 0, measure.** The `batching` bench group and the `frame_render`
   stats line are written; the numbers are above.
2. **Before Phase 1, three existing bugs.**
   - A clip of zero size draws nothing (open question 3). `push_clip` and
     `text()` mark a clip that misses its parent as empty, and every shader
     discards inside an empty clip. A GPU test at scale 1 and 1.25 checks
     that no pixel draws where the fallback point used to land on a pixel
     centre.

     > **2026-10-16 update:** done in the shaders instead. An `outside_clip`
     > helper in `ui.wgsl` and `ui_msdf.wgsl` treats a clip with zero width
     > or height as holding nothing, and all nine clip tests use it, so
     > `push_clip` and `text()` don't need to mark anything: the fallback
     > point already has zero size. The GPU test draws soup, chrome and
     > circles inside such a clip at scale 1 and 1.25, with a one-pixel clip
     > as the control; without the fix soup leaked a pixel.
   - The text renderer keeps its logical size as f32, so text and MSDF icons
     project like everything else. A GPU test at a target size that doesn't
     divide by the scale (1001×601 at 1.25) checks text lands where chrome
     does.
   - The `--no-default-features` library build is repaired. It fails today
     with 6 errors where `PhosphorIcon`, `Icon`, `IconKey::new`, `Thumb` and
     `SearchField` are used without the `phosphor-icons` feature
     (`widgets/file_dialog`, `widgets/icon_key.rs` ~87, `widgets/mod.rs`).
     Adding that check to `publish.yml` would otherwise break the next
     release. `tests/external_sprite.rs` and `tests/text_contrast.rs` get
     `required-features = ["headless"]` at the same time.
3. **Phase 1, draw boxes.** One function per kind except text and MSDF icons,
   mirroring the shaders, with unit tests. The tests include chrome with a
   large radius at fractional positions, and coordinates around 1e5. Then the
   GPU test that no unit changes a pixel outside its box.

   > **2026-10-16 update:** the GPU test is a unit test inside the crate
   > (`render/draw_box_tests.rs`, behind `headless`), not an integration
   > test, because the box functions are crate-private. It covers YUV
   > sprites too. Untransformed `DrawList::quad` records chrome, not soup, so
   > soup is drawn with `triangle` (and `quad` under a transform);
   > `tests/zero_size_clips.rs` had the same mistake and now draws soup with
   > a triangle as well, checked to leak without the shader fix. Cutting the
   > chrome reach to 0.55 px fails at a corner pixel, so the 0.604 px corner
   > reach shows up on the GPU and 0.61 is needed.
4. **Phase 2, text and icon preparation.** Split the CPU side from the GPU
   upload, keep each unit's vertices and a box per block, and draw runs from
   them. Pixels don't change, since the vertices are the same; the existing
   text tests check that, along with a look at the widget gallery (which
   writes images for inspection and asserts only that something drew).
   A new test has a later unit grow the atlas and checks that earlier units'
   UVs use the final size. Text and MSDF icons join Phase 1's GPU box test.

   > **2026-10-16 update:** the CPU side is `MsdfCpu` in `text_prep.rs`
   > (the atlases, font keys and font system), and what it prepares is
   > `MsdfPrepared`: every text unit's vertices, each block's range in each
   > of the three sweeps, each command's range, and each icon's six vertices.
   > `TextRenderer` keeps one of each and uploads a command's or an icon
   > run's range when it draws. `MsdfGlyphAtlas::starting_at` (tests only)
   > starts an atlas small, so the growth tests for text and icons need a
   > few glyphs, not hundreds; each fails when the atlas size is taken
   > before the glyphs are placed. The text tests that only build vertices
   > now run on the CPU, without a GPU. The GPU box test adds text (plain,
   > outline, shadow, glow, scaled) and icons (plain, turned); it fails when
   > a block's box leaves out its shadow and glow sweeps. Pixels are
   > unchanged: every existing test passes.

   > **2026-10-16 update, after review:** the review found no change in
   > behaviour, except one fix it confirmed: icons drawn on a path without a
   > preflight (`render_icons`, `upload_icons`) used to build their UVs
   > against the atlas size before later icons grew it, and now use the
   > final size like text. Its smaller points are in: the test-only
   > accessors are gated on the features their tests need, the text and
   > icon boxes have CPU tests (`msdf_boxes`, 4 tests), a text command after
   > an empty one gets its own vertices (a test), the upload slices its
   > range once, and `block_vertices` the field became `block_sweeps`.
   > Tests that need the bundled font are now gated on `bundled-font`, so
   > the library's tests pass with any set of features (`--no-default-features`
   > had 26 failures before); the `expect(dead_code)` conditions on the new
   > modules in `render/mod.rs` follow the same features. A checkbox test
   > that counted soup vertices for its tick, stale since strokes became
   > analytic records, now counts segment records.
5. **Phase 3, `DrawOrder`.** Unit tests:
   - overlap keeps the order, with a different key and with the same one;
   - disjoint units share an order;
   - boxes whose pixel ranges are a pixel apart don't chain (for chrome, 2 px
     apart; chrome 1 px apart with edges on whole pixels at scale 1 shares
     the pixel between them and chains);
   - a NaN box is a barrier, and a unit whose only box is empty isn't
     ordered;
   - a shadowed unit with several boxes, one empty and one on the target, is
     ordered and kept;
   - orders match a brute-force oracle on random layouts, which include
     units with several boxes, some of them empty;
   - a unit with several boxes where only a later box overlaps an earlier
     unit of a different key gets the raised order;
   - a later unit that overlaps only the first box of such a unit, whose
     order was raised by its second box, sorts after it;
   - its own work (overlap tests and cell visits) stays within a fixed amount
     per unit for list-like layouts of 1k and 10k units, all on the target,
     with the number of units ordered asserted.

   > **2026-10-16 update:** written as `DrawOrder` in `render/draw_order.rs`
   > (`begin` for a target size, then `add(key, boxes)` per unit in paint
   > order, giving `Some(order)` or `None` when culled; `stats()` for
   > `units_ordered`, `overlap_tests` and `cell_visits`), with `BatchKey` as
   > a `u32` newtype that Phase 4 fills in. The oracle,
   > `brute_force_orders`, is `cfg(test)` for now; Phase 5 widens it to
   > `cfg(any(test, feature = "paint-order-oracle"))` when it adds the
   > feature. The list layout has ten columns side by side at both lengths:
   > with one column at 1k and ten at 10k, the cells along the column edges
   > made the 10k list look costlier per unit (5.2 against 8.0 overlap
   > tests). With the same shape it takes about 8 overlap tests and 9 cell
   > visits a unit at either length, and the test allows 10 of each. A
   > background overlaps nothing in its cells, so it is tested against all
   > of them, as in gpui. Each of three mistakes the design warns about
   > fails the tests: forgetting the old top's order when a new key takes a
   > cell, storing each box as soon as it is looked up, and skipping a cell
   > by its top alone for the top's own key.
6. **Phase 4, `DrawPlan`.** CPU tests: overlapping pairs keep their order;
   batch counts match a brute-force oracle on random layouts; the batch
   counts on the button grid (2) and a scroll view are asserted; and units
   form as described: a text command with a shadow or glow gives one unit
   with one box per block, and a command without effects gives one unit per
   block.

   > **2026-10-16 update:** written as `DrawPlan` in `render/draw_plan.rs`.
   > `build(list, target, prepared, textures)` walks the paint commands,
   > makes a unit for each (`UnitRef`: a soup command, an analytic record, a
   > circle, a sprite, a nine-slice, a text unit or an MSDF icon), boxes it
   > (Phase 1 and 2), orders it (Phase 3), then sorts the units by
   > (order, key, paint index) and cuts the sorted list into batches, one per
   > run of a key. `BatchKey` became a `u64`: the family in the top byte and
   > the texture below it, from `Binding::code` in `render/textures.rs`.
   > Sprite and nine-slice textures come through a `ResolveTextures` trait,
   > so the plan is tested on the CPU with a fake; a sprite it can't resolve
   > draws nothing, as today, but still takes a paint index.
   > `PixelGrid::new` lets the tests build the renderer's grid. Tests in
   > `render/draw_plan_tests.rs`: overlaps keep paint order, and on random
   > layouts the batches cover every unit and their count matches
   > `brute_force_batch_count`; a grid of 100 buttons plans as 2 batches
   > (300 analytic units, then 100 text units); a scroll view of 200 rows,
   > 15 of them in view, plans as 3 (28 analytic, 2 soup, 15 text); a
   > shadowed command is one unit with a box per block, and a plain one a
   > unit per block; a shadowed unit with a block off the target still
   > draws, and is culled once all of it is off; sprites batch by texture.
7. **Phase 5, renderer.** Draw from the plan:
   - the identity plan as the paint-order path;
   - draws counted where they're encoded, plus `batches` and the stats
     changes above;
   - the `paint-order-oracle` feature (`set_paint_order` and
     `set_check_plan`) and its checks in `publish.yml`;
   - the new `RenderStats` counters and their debug report lines;
   - `render_widget_gallery` (an `#[ignore]`d GPU test) turns on
     `set_check_plan`, so each gallery list's plan is checked against the
     oracle;
   - the `viewport` contract in the docs of `render` and `render_layers`;
   - the commit marked breaking (`feat!`), with `RenderStats` made
     `#[non_exhaustive]`;
   - the existing tests, as described under "Existing tests":
     `ordered_paint_stress`'s count asserts on the paint-order path,
     `many_alternating_runs…` and its third test on both paths, and
     `render_passes` on both paths.

   > **2026-10-16 update:** written. `render_one` now draws a paint stream
   > through `render_batches`: the text and icons are prepared as before,
   > then `DrawPlan::build` (or `build_paint_order`, the identity plan, when
   > `set_paint_order` is on) plans the list, `BatchDraws::fill`
   > (`render/batch_draws.rs`, CPU only) copies each family's records in
   > draw order and makes a draw per batch with something to draw,
   > `upload_batches` writes each family once, and `draw_batches` encodes
   > the draws in the list's one pass, binding everything each draw needs.
   > Lists without a paint stream (`render_unordered`) and trailing soup
   > draw as before. Some differences from the text above:
   > - The plan resolves each sprite and nine-slice once, through
   >   `ResolveTextures`, which now returns the whole `Resolved` (and a
   >   nine-slice's border); the plan keeps it for the instance builders, so
   >   a sprite isn't looked up twice. The renderer's resolver warns once per
   >   missing name without allocating once warned (`Warned`), and
   >   `note_small_texture` looks a name up before copying it.
   > - The identity plan is the same walk as the batched one with no boxes:
   >   units in paint order, a batch per command and key, so unresolved
   >   draws between two of one texture merge, as `textures::batches` did.
   > - `set_check_plan` keeps every unit's key and boxes and runs both
   >   oracles after each plan. Making `order` wrong for some units in a
   >   mutation makes the widget gallery fail on it, so the check isn't
   >   vacuous.
   > - `order_overlap_tests` and `order_cell_visits` are `u64`, as
   >   `OrderStats` counts them.
   > - Two existing miscounts are fixed alongside: `draw_color` (trailing
   >   soup, lists without a paint stream) now counts its two buffer writes,
   >   and the text renderer's uniform arena growing counts in
   >   `buffer_reallocations`, as its vertex buffer growing now does.
   > - The `render/mod.rs` paragraph promising paint order is updated now,
   >   since it is no longer true; the other docs listed under Phase 6 wait
   >   for it.
   > - `icon_font_snapshot()` still collects a `Vec` per call; it goes with
   >   the allocation test in Phase 6.
   >
   > The whole crate's tests pass on the new path, GPU tests included (2175
   > with `--include-ignored`), clippy is clean in every feature set, and
   > the two release checks run clean locally.
   >
   > **2026-10-16 update (review):** a review of the written Phase 5 found a
   > bug and some loose ends, now fixed:
   > - On a list with no paint stream, its soup drew twice (once in
   >   `render_unordered` and again as trailing soup). Trailing soup now goes
   >   into the batch pass as its last draw (`BatchDraws::push_soup`), and
   >   `render_unordered` draws a list without a paint stream on its own.
   >   `render_passes::soup_outside_the_paint_stream_draws_once` checks both
   >   cases on both paths: one draw, half-transparent white over black
   >   comes out mid grey.
   > - Every dynamic buffer, the text vertex buffer included, is now a
   >   `FrameArena` (`render/frame_arena.rs`), which grows, retires the old
   >   buffer and counts its writes in one place. The text pipeline's writes
   >   (its uniform included) are added to the frame's counts, so an empty
   >   render now counts two buffer writes, not one
   >   (`analytic_shadow_performance`).
   > - Small textures are noted for nine-slices too, and counted where the
   >   draw is encoded rather than where it is planned.
   > - The compatibility path counts its colour, icon and text runs.
   > - Unit tests of `build_paint_order` and `BatchDraws::fill`.
   > - `icon_font_snapshot` fills a `Vec` the caller keeps (`MsdfCpu` keeps
   >   one), instead of collecting a new one per list.
8. **Phase 6, verify.**
   - GPU tests (`#[ignore]`, like `ordered_paint_stress`) draw the same lists
     in paint order and in batch order, and compare pixels:
     - the button grid;
     - overlapping chrome with text on top;
     - overlapping shadowed text, with a box elsewhere recorded between the
       two blocks;
     - back-to-back shadowed and glowing blocks merged into one command,
       where block B's shadow overlaps block A's fill;
     - clipped text in a scroll view;
     - shadows under neighbours;
     - rotated chrome next to other content;
     - sprites in two textures, and YUV sprites;
     - strokes crossing chrome and text;
     - circles, nine-slices and `radial_gradient` soup next to and over
       other content;
     - MSDF icons overlapping text (a different key on the same pipeline);
     - a list that is wholly culled;
     - the widget gallery.

     Each runs at scales 1, 1.25 and 1.5, with a non-zero view origin, and
     with an origin around 1e5. For that last run, the content is shifted by
     the same amount, so it stays on the target rather than being culled on
     both paths.
   - The counting-allocator test described under "Scratch kept on the
     renderer" checks that a warmed frame's CPU-side pieces allocate nothing.
   - Bench cases that report batch counts: touching list rows with text at
     scales 1.0 and 1.5, a dense chart, and a 10k-point polyline. The dense
     chart is many small units that don't overlap, with alternating keys in
     the same cells: scatter markers as circles, with short segments in the
     gaps that touch no marker. That's the case where the grid's cost grows
     quadratically, so it decides between the grid and gpui's R-tree, and it
     reports `DrawOrder`'s work count. The list rows decide whether the
     tighter text box is worth its own design.
   - Bench numbers go here, with on-target unit counts.
   - The docs that promise strict or consecutive paint order are updated:
     `render/mod.rs` (~15), `UiRenderer::create_atlas` (~889),
     `README.md` (~213-216, "never regrouped across another primitive
     family"), `docs/design/gpu-chrome-and-shadows.md`, and the comments at
     `tests/icon_instancing.rs` ~282-285, `render/textures.rs` ~905-907 and
     `render/ui_renderer.rs` ~1800.
   - The TODO entries are closed.

   > **2026-10-16 update:** the pixel tests are written, in
   > `tests/batch_order.rs`: twelve of the scenes above (the MSDF icon one
   > needs `phosphor-icons`), each at the three scales and three origins,
   > must match paint order pixel for pixel, with no more draw calls than
   > paint order. A wholly culled list draws nothing. The widget gallery
   > runs with `set_check_plan` on instead of a pixel comparison. Making the
   > plan's sort ignore `order` makes the scenes fail.
   >
   > **2026-10-16 update:** the allocation test is written:
   > `draw_plan::tests::lists::planning_a_warmed_frame_allocates_nothing`
   > prepares the text and icons of a list with every kind of primitive,
   > plans it and fills its draws, after two warm-up frames, and counts no
   > allocations on its thread. It calls the CPU pieces directly; the texture
   > runs and small-texture counting now happen where draws are encoded, and
   > compare before they allocate. The counting allocator is now one file
   > (`tests/support/counting_alloc.rs`), shared by this test (through
   > `#[path]`) and the two integration tests that had their own copy.
   > Reverting `icon_font_snapshot` to a fresh `Vec` per list makes it fail.
   >
   > The docs that promised strict paint order are updated (`README.md`,
   > `create_atlas`, `render/textures.rs`, `PaintCmd`,
   > `tests/icon_instancing.rs`, and a dated note in
   > `gpu-chrome-and-shadows.md`), and the batching TODO entries are closed.
   >
   > **Bench numbers (2026-10-16, Apple GPU, Metal).** The `batching` group
   > now draws every case batched and in paint order (`set_paint_order`),
   > printing the frame's counts first. The machine was busy (load average
   > 10 to 25, other agents building), so the times are rough: medians, and
   > a rerun of one case varied by a factor of two. The counts are exact.
   >
   > | case | primitives | on target (ordered) | draw calls, batched / paint order | encode, batched / paint order | wait, batched / paint order |
   > |---|---|---|---|---|---|
   > | 100 labelled boxes, interleaved | 200 | 200 | 2 / 200 | 0.19 / 0.69 ms | 5.3 / 5.4 ms |
   > | 1,000, interleaved | 2,000 | 1,907 | 2 / 2,000 | 1.6 / 2.8 ms | 6.5 / 12.3 ms |
   > | 10,000, interleaved | 20,000 | 2,379 | 2 / 20,000 | 11.6 / 47.4 ms | 12.4 / 30.3 ms |
   > | 10,000, grouped | 20,000 | 2,379 | 2 / 2 | 8.9 / 5.7 ms | 15.7 / 9.9 ms |
   > | list rows, scale 1 | 360 | 360 | 2 / 360 | 0.51 / 1.24 ms | 5.5 / 6.5 ms |
   > | list rows, scale 1.5 | 240 | 240 | 2 / 240 | 0.45 / 0.97 ms | 5.4 / 6.9 ms |
   > | dense chart | 16,080 | 16,080 | 2 / 16,080 | 2.5 / 10.7 ms | 11.9 / 17.8 ms |
   > | 10k-point polyline | 9,999 | 9,999 | 1 / 1 | 1.1 / 0.47 ms | 11.0 / 10.3 ms |
   > | 10k-triangle soup polygon | 10,001 | 2 | 2 / 2 | 0.44 / 0.28 ms | 6.0 / 6.0 ms |
   >
   > What they say:
   > - Interleaved lists, the case this was built for, draw in 2 calls and
   >   encode 2 to 4 times faster. The labelled-box grid's interleaved and
   >   grouped recordings now cost about the same, as they should.
   > - A list that was already grouped pays for planning with nothing to
   >   gain: about 3 ms for 20,000 primitives, most of them off the target
   >   (each is still boxed to be culled).
   > - The list rows plan as 2 batches at both scales: the text box as it is
   >   doesn't make labels share cells with the next row's background badly
   >   enough to split batches, so a tighter text box isn't worth its own
   >   design for now.
   > - The dense chart, the grid's worst case, does about 8 overlap tests per
   >   unit (127,200 for 16,080 units) and 32,160 cell visits, and still
   >   encodes four times faster than paint order. The grid stays; gpui's
   >   R-tree isn't needed. (The first version of this case drew its
   >   segments within 2 px of the markers, which a circle's box reaches, so
   >   every row chained into one order per unit: 320 batches.)
   > - A list of one key gains nothing and pays for ordering: the polyline's
   >   10k segments cost about 0.6 ms more to encode. A timing run of the
   >   plan alone gave about 0.4 ms of box maths and 0.6 ms of ordering for
   >   them; the sort and the copy into draws are small. Skipping the order
   >   when every unit has one key is in TODO.md.
   > - Boxing soup costs about 16 ns per triangle (0.16 ms for 10,000); the
   >   TODO entry has the number.

   > **2026-10-21 update:** while every unit so far has had one key,
   > `DrawOrder` gives each order 0 and sets its boxes aside instead of
   > putting them in the grid; the first unit of a second key stores them, as
   > they would have been stored, and ordering goes on as before. `DrawPlan`
   > skips the sort for a list of one key, which is in draw order already.
   > Units are still boxed, to cull them. The polyline case now does no cell
   > visits (40,742 before) and encodes in 0.75 ms against 1.11 ms (paint
   > order 0.48 ms; the rest of the gap is the box maths). Same busy machine
   > caveat as above.

## Open questions for review

1. **Always on, or a switch?**
   > **2026-10-16 answer (Bart):** always on, with the paint-order path for tests only.
2. **Is one box per soup command good enough?** A chart drawn as soup is one
   command, and its box is the union of its triangles. So it can't batch with
   anything inside that union. Splitting soup into one unit per triangle would
   mean regrouping its indices per batch. That could come later, if a real
   screen shows the need. My recommendation is one box per command for now.

   > **2026-10-16 answer (Bart):** one box per soup command for now.
3. **Should a clip of zero size draw nothing?** This is an existing bug, not a
   batching one.
   - When nested clips don't overlap, `push_clip` falls back to
     `Rect::new(x, y, 0.0, 0.0)` at the child rect's origin (`draw_list.rs`
     ~1208-1212). That is a point, which may lie outside the parent clip.
   - `text()` does the same when a block's own clip misses the active one
     (~2827-2830), but its point is the active clip's corner, so it sits on
     the parent clip's edge, never outside it.
   - The shaders keep pixels exactly on a clip's edge, so wherever both of
     the point's coordinates land on a pixel centre, one pixel still draws.
     That happens at any scale; for example x = 2.5 at scale 1, or 2.0 at
     scale 1.25.
   - A real row or column of pixels only comes from a degenerate input rect
     with no parent clip, from `push_clip_exact`, or from a
     `TextBlock::with_clip` of zero width or height when no clip is active
     (`text()` keeps the block's own clip then, `draw_list.rs` ~2826-2832).

   The options:
   - `DrawList` marks such clips as empty, so nothing inside them draws. This
     is what the code means to do. It removes the stray pixel, and batching
     can then skip the content.
   - Leave it as it is. Batching keeps it correct with the inclusive
     intersection above, at a small cost in batching. With `push_clip`'s
     point, the pixel can still leak where the parent clip should have
     hidden it.

   My recommendation is the first, as a separate fix ahead of Phase 1.

   > **2026-10-16 answer (Bart):** such clips draw nothing, fixed first as a
   > change of its own.

## References

- `crates/core/benches/ui_stress.rs`: the `batching` and `frame_render` groups.
- `crates/core/tests/ordered_paint_stress.rs`: the existing GPU ordering tests.
- `crates/core/src/render/ui.wgsl`: each instance's quad (~122-152, ~300) and
  edge coverage (~181-196).
- `crates/core/src/text.rs`: `prepare_texts`, `build_vertices` and its three
  sweeps, `upload_icons`.
- `docs/design/gpu-chrome-and-shadows.md`: "GPU representation and ordered paint".
- gpui: `zed/crates/gpui/src/bounds_tree.rs` (order from overlap) and
  `zed/crates/gpui/src/scene.rs` (sorting and batching), in `~/Projects/gpuix`.
- `TODO.md`: "Learned from gpui", "P1 — Measure draw-call batching", and
  "P1 — Perf: `frame_render` is ~140× slower".
