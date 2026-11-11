# UI library comparison

How wgpu-gameui, egui, iced, gpui, BUI and Rune compare on the same scene. Separate
from the published crates on purpose: gpui's dependency tree and BUI's GPL
licence must not reach them or CI. There are four Cargo workspaces here, each
with its own lockfile and `target/`:

- `.` : `common`, `gameui`, `egui`, `iced`;
- `gpui/` : gpui pins crates (`unicode-properties =0.1.3`) that egui cannot
  share a lockfile with. It builds from the zed checkout at `~/Projects/zed`
  (a sibling of this repo; the aeolun fork's `main`, `e231594b6b` of 6 Nov 2026)
  and carries zed's `[patch.crates-io]` entries, which a path dependency does
  not inherit;
- `bui/` : the only place BUI (`blenderui` 0.4.1, GPL-3.0-or-later, a C core
  built with CMake and a C compiler) is linked;
- `rune/` : Rune (`anomalyco/rune`, MIT, pre-release, git only, no crates.io
  release) from a local clone at `~/Projects/rune-src/rune` (the `dev` branch,
  shallow). It is a workspace of its own so its toolchain pin (1.99.1; the
  harness builds fine on 1.97.1) and dependency tree stay out of the others.

```
cd bench/ui-compare
cargo build --release                          # gameui, egui, iced
(cd gpui && cargo build --release)
(cd bui && cargo build --release)
(cd rune && cargo build --release)
DISPLAY=:0 ./target/release/ui-compare-gameui   # or -egui, -iced
DISPLAY=:0 ./target/release/ui-compare-egui 1000  # one cell count only
DISPLAY=:0 ./target/release/ui-compare-gameui-retained      # gameui's retained tree
DISPLAY=:0 ./target/release/ui-compare-gameui-retained rows # one scene group: rows, deep, text
UI_COMPARE_REUSE=1 DISPLAY=:0 ./target/release/ui-compare-gameui  # renderer frame comparison on
DISPLAY=:0 gpui/target/release/ui-compare-gpui
DISPLAY=:0 rune/target/release/ui-compare-rune
DISPLAY=:0 bui/target/release/ui-compare-bui           # headless, CPU raster
DISPLAY=:0 bui/target/release/ui-compare-bui --window  # real X11 window, OpenGL
```

Each harness needs a GPU adapter (it picks the high-performance one and prints
its name) and writes `shots/<library>-<n>.png`, which you should look at.

## The scene

`n` cells (100, 1,000, 3,000, 10,000) tile a 1920x1080 logical window, so
**every cell is on screen**: egui culls what is off screen and gameui does not,
so a scene bigger than the window would compare different amounts of work. A
cell is a filled rounded rectangle with a label of at most two characters
(`i % 100`), in IBM Plex Sans, the font gameui bundles, loaded by each harness
from the same file. Cells shrink as `n` grows (about 64x28 px at 1,000 and
14x14 at 10,000).

- `static`: nothing changes between frames.
- `animated`: every cell's colour changes every frame.

All drawing is by absolute position: rectangles and text through each
library's draw API (gameui `DrawList`, egui `Painter`), or, for iced, which has
no such thing, a column of rows of fixed-size `container(text)` widgets. iced's
number therefore includes its layout pass and widget diff, which the other two
do not have.

## What is timed

CPU frame cost on the calling thread, no waiting for the GPU:

| library | one timed frame |
| --- | --- |
| gameui | fresh `DrawList` (rect + `TextBlock` per cell), `UiRenderer::render` (shaping, instance encode, upload, encode), `submit` |
| egui | `run_pass` + `tessellate` (`egui`); plus texture delta, `update_buffers`, render pass, `submit` (`egui+wgpu`) |
| iced | `view`, `UserInterface::build` (diff + layout), `draw`, `Renderer::present` (prepare, upload, pass, `submit`) |
| gpui | gpui's own bench machinery, headless (`BenchAppContext`, a real text system, `WgpuHeadlessRenderer`): the view's `render`, taffy layout, prepaint, paint into a `Scene`, then the scene is handed to the wgpu renderer (atlas and instance upload, pass, `submit`). One iteration is the gap between successive calls of the per-iteration callback; criterion only drives the loop and its statistics are unused |
| rune | Rune headless against an offscreen wgpu target, as its own `pinch_bench` example does: change the state (animated) or only `invalidate` it (static, so the view is rebuilt either way), `Runtime::frame` (view, flex layout in `rune-taffy`, instances), then `Renderer::upload_images`, `prepare` (atlas and instance upload), the pass and `submit`. Not waited on. Scale factor 1 |
| rune(cpu) | the same frames; the thread's CPU time (`CLOCK_THREAD_CPUTIME_ID`) instead of wall time. At 100 cells wall time is 1.7 ms against 0.2 to 0.4 ms of CPU, so about 1.4 ms of each frame is spent blocked (probably on the previous frame's GPU work); from 1,000 cells on the two are equal. The comparison table uses the CPU row |
| bui(cpu) | BUI headless (`Application::headless()`, as its own `raster_probe` example): `render`, layout, **CPU software rasterization** of the whole frame, and a copy of the 1920x1080 pixels out (about 8 MB, which `App::draw` always makes) |
| bui(gpu,cpu) | `--window`: a real, visible X11 window drawn with BUI's OpenGL 3.3 path (`Application::gpu(true)`, vsync off, a new frame requested every `render`). The window's own loop does the work, so the view times itself: **the thread's CPU time** (`CLOCK_THREAD_CPUTIME_ID`) between two successive `render` calls. That is the rebuild, layout, paint and the GL calls and driver work on the UI thread, with the waiting left out. Any BUI worker threads are not counted |
| bui(gpu,wall) | the same runs, but wall-clock time between two `render` calls: the whole frame as the window sees it, including the buffer swap and the compositor. Pinned near the display's refresh interval (about 17 ms) at small sizes, so it says when BUI stops keeping up, not what a frame costs |

The `egui+wgpu` row is the one to put beside gameui and iced, which both
include their GPU hand-off. A real iced or egui app would skip a frame in which
nothing changed; here a frame is forced either way.

Two rows are not like for like with the rest, and the README should not let a
table hide it:

- `bui(cpu)` rasterizes on the CPU; everything else rasterizes on the GPU
  after the hand-off. Its number contains work the others leave to the GPU, so
  read it as BUI's cost per frame in its default mode, not as its draw-call
  cost. `bui(gpu,cpu)` is the OpenGL path in a real window and is the closer
  match, but it is measured differently (thread CPU time inside a live window
  loop, not a timed call) and the window really is presented to the desktop.
  The GPU path switches on a few frames after the window opens, and each row
  is labelled with what `is_gpu()` said when the case ended. The frame was
  checked with an `xwd` capture of the window (`shots/bui-gpu-window.png`).
- gpui's test window has a fixed scale factor of 2 and no way to change it from
  outside, so its scene is built at half logical size: the frame is the same
  1920x1080 device pixels with the same cells and 12 px (device) text. gpui's
  timing excludes the compositor, as the others do.

Each case runs 30 warm-up frames, then up to 200 timed ones (20 s budget).
GPU execution time is **not** measured; that is the second phase.

## Checks that the scene was drawn

A fast number from a harness that drew nothing is worthless, and this caught
real problems while building it: egui culled off-screen cells, an unfinished
egui font setup drew empty boxes, labels were clipped differently per library.
Each harness reads the frame back (or asks the renderer) and `check_frame`
asserts more than half of it is lit, then saves the PNG. gameui also asserts its
renderer's primitive count; egui prints its vertex count.

## Versions

egui 0.37.0, egui-wgpu 0.37.0 (wgpu 30), iced 0.14.0 (wgpu 27), wgpu-gameui
from this tree (wgpu 24), gpui from zed `e231594b6b` (wgpu from zed's pinned
git revision), blenderui 0.4.1, rune `dev` `ddf1414` (0.0.1, wgpu 30, parley 0.12, taffy 0.15; clone taken 2026-11-07). Exact locks in each workspace's `Cargo.lock`.

A mismatch caught while building each harness is worth knowing about:
`egui`'s `default_fonts` feature needs `epaint_default_fonts 0.37`, which is not
on crates.io, so egui is given the shared font by hand. `gpui_platform` built
without its `x11` feature falls back to a no-op text system: gpui then draws
quads and no text, and only the glyph-count check catches it.

## Results: CPU frame cost, 2026-11-07

One run on one machine (24 threads, Radeon RX 7900 XTX via RADV, Vulkan),
**under load**: other agents were building, load average 8 to 19 during the
run, so treat differences under about 20% as noise and rerun on a quiet machine
before quoting anything. `animated` scenario; mean ms per frame (median in
brackets). Not GPU time. `bui(cpu)` and gpui differ from the rest as described
above.

| cells | gameui | egui+wgpu | iced | gpui | bui(cpu) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 0.09 (0.09) | 0.08 (0.07) | 0.06 (0.06) | 0.46 (0.40) | 2.1 (2.0) |
| 1,000 | 0.63 (0.63) | 0.62 (0.56) | 0.41 (0.40) | 4.9 (4.5) | 14.7 (13.7) |
| 3,000 | 2.1 (2.1) | 1.8 (1.7) | 1.3 (1.2) | 23.9 (22.6) | 50.1 (49.1) |
| 10,000 | 9.5 (9.2) | 13.5 (13.4) | 10.5 (9.8) | 103 (103) | 170 (161) |

Reading it without overreaching: at 100 to 3,000 cells iced and egui cost less
CPU per frame than gameui in this scene, and by 10,000 gameui is the lowest of
the three that hand off to the GPU. gpui is about an order of magnitude above
the others from 1,000 cells and grows faster than linearly. The `static`
scenario matches `animated` within noise for every library except
`bui(cpu)`, which is cheaper when nothing changes (it compares tiles).
gpui's 10,000-cell case has only 10 timed frames, so its spread is thin.
### BUI in a real window (OpenGL), same run

`--window` run, `animated`, ms per frame, mean (p99 in brackets):

| cells | bui(cpu), headless | bui(gpu,cpu) | bui(gpu,wall) |
| ---: | ---: | ---: | ---: |
| 100 | 2.1 | 0.32 (0.74) | 17.1 (17.5) |
| 1,000 | 14.7 | 1.9 (3.5) | 18.6 (20.7) |
| 3,000 | 50.1 | 4.9 (8.4) | 21.7 (25.2) |
| 10,000 | 170 | 23.0 (32.7) | 39.8 (49.5) |

On the GPU path BUI's CPU cost falls to roughly a tenth of its headless
number, which puts it at about 3 to 5 times gameui's at 1,000 to 10,000 cells
(gameui 0.63 / 2.1 / 9.5 ms) rather than 15 to 25 times. The static scenario
is within noise of animated. At 10,000 cells the wall time (40 ms) shows BUI
no longer holds 60 fps in this scene; the other libraries' wall time was not
measured. Same load caveat as above, and one run only.
### Rune, run on its own at lower load (load average about 4)

Mean ms per frame (median in brackets); `rune(cpu)` is thread CPU time. Run
separately from the table above, so the machine was quieter than for the other
libraries. The frame was checked as for the others (`shots/rune-*.png`).

| cells | rune(cpu) animated | rune(cpu) static | rune wall, animated | gameui animated |
| ---: | ---: | ---: | ---: | ---: |
| 100 | 0.38 (0.36) | 0.19 (0.16) | 1.76 | 0.09 |
| 1,000 | 3.2 (3.2) | 1.5 (1.5) | 3.2 | 0.63 |
| 3,000 | 13.2 (12.9) | 5.8 (5.6) | 13.2 | 2.1 |
| 10,000 | 55.3 (54.7) | 30.8 (30.5) | 55.3 | 9.5 |

Rune costs about 5 to 6 times gameui per animated frame from 1,000 cells, and
about 4 times at 100. It is cheaper when nothing changes (it diffs the rebuilt
tree against the last, so static frames reuse layout and instances), roughly
half of its animated cost at 3,000 and 10,000 cells. The scene is built with
flex rows and columns (as for gpui, iced and BUI), not absolute positions, which
is also what `rune-kit` apps would use; Rune's own `pinch_bench` pins its cards
absolutely, and that may cost less than this layout. Not tried.
### Rune in a real window: not measured yet

`UI_COMPARE_WINDOW=1 WINIT_X11_SCALE_FACTOR=1 rune/target/release/ui-compare-rune`
opens a visible window and times it (thread CPU time, Rune's own
`update_ms + encode_ms` from `on_frame_encoded`, and wall time). The scene
draws correctly there (`shots/rune-window.png`), but the numbers from the one
run made are not usable and are not recorded: that session had the desktop
locked and the monitor off (`GetActive` true, DPMS "Monitor is Off"), and the
window then presented one frame per second, whatever the app asked for
(`wake_after`, a waking thread and a looping animation all made no
difference). Rune presents with `AutoVsync` and the other windowed run, BUI's,
presents with vsync off, so a compositor that stops sending frame callbacks
throttles only Rune. It needs a rerun with the screen unlocked. Two more
things to know when it does run: on X11 Rune has no display link, so a window
that has drawn goes quiet until something asks for a frame (hence the 2x2
looping-opacity element in the corner, which is part of the measured scene),
and `WINIT_X11_SCALE_FACTOR=1` is needed or the window opens at 1.17x scale
(2240x1260 device pixels, more than the others draw).
## Where gameui's time goes (profile at 3,000 animated cells)

`perf record` (frame pointers, line tables) on `ui-compare-gameui 3000`, on a
loaded machine, plus a per-phase timer around the harness's own calls
(removed again: it was only there for this). Percentages are of all samples.

Phases, ms per frame (3,000 cells / 10,000 cells), before the hasher change
below: building the `DrawList` 0.26 / 1.1; `UiRenderer::render` 1.69 / 7.6;
`queue.submit` and poll 0.02 / 0.08; dropping the list 0.14 / 0.5. So `render`
is about 80% of the frame and the list itself (building and dropping, one
`String` per label) about 20%.

Inside `render` (the share of the profile, as it stood before the change):

| what | share | notes |
| --- | ---: | --- |
| `DrawPlan` walk: pixel bounds of every unit | about 40% | text blocks are bounded by unioning two clipped triangles per glyph (`draw_box::msdf` / `triangle`, about 16% on its own); `cull_bounds` about 5% to 12% |
| `DrawOrder::add` (overlap ordering) | 12% to 15%, plus `store` 2% and sorting the plan about 3.5% | each cell is a rectangle and a text block, two batch keys, so the grid path runs for all 6,000 units |
| text preparation (`prepare_texts`) | 21% to 31% | the layout-cache lookup was 11%, of which SipHash was most; then `build_unit` and `append_placements` (placements per glyph) |
| SipHash, summed over the layout cache, the glyph-atlas lookup and a `fontdb::ID` map | about 12% | |
| malloc / free / memmove | about 10% | one `String` per `TextBlock` per frame, and `queue.write_buffer`'s copy (2.5%, wgpu's) |

One change was made and measured: the layout cache's outer map and the glyph
atlas's lookup map now hash with Fx instead of SipHash (committed as
`f8dc974`). `render` went from 1.69 to 1.39 ms at 3,000 cells and from 7.6 to
6.9 ms at 10,000; the whole frame is then about 1.9 ms against iced's 1.3 and
egui's 1.5 to 1.8 at 3,000 cells. The remaining hashing is the `fontdb::ID` map
(about 1.5%) and two more lookups per label in `LayoutCache::entry`, which
hashes the key three times in the hit path (`get`, then `get_mut` after
`contains_key`); both were changed later (below).

Every item that profile listed was then tried, one at a time, each
measured back to back against the commit before it (the frame at 3,000 cells
unless noted; the machine was at load average 20 to 40 throughout, so each
figure is the mean of several alternating runs and is good to a few percent):

| what | commit | effect |
| --- | --- | --- |
| Fx hashers for the layout cache's outer map and the glyph atlas lookup | `f8dc974` | `render` 1.69 to 1.39 ms |
| bound a glyph quad once from its four corners, and the clip once per run | `1e95fe9` | about 8% at 3,000, 4.5% at 10,000. Well under the 16% the profile's `triangle` and `union` frames suggested: inlining spreads cost across callers in `perf`, so those shares overstate what one function gives back |
| one hash per level on a layout-cache hit (a slab behind the maps, so no borrow outlives the lookup) | `20fde6a` | about 2% at 3,000, nothing visible at 10,000 |
| draw-order grid cell tuned to the list (16, 32 or 64 px) | `311bbf0` | 1.62 to 1.45 ms at 3,000 (-10%), 7.49 to 6.07 ms at 10,000 (-19%) |
| a plain `TextBlock` shares its two empty range lists instead of allocating them | `5c6f672` | about 7% at 3,000 |
| `TextBlock::content` is a shared `TextContent` (`Arc<str>`), so a retained label costs a reference count | `1d1897b` | about 6% at 3,000 with the harness holding its labels, as the egui and iced ones borrow theirs |
| Fx hash for the per-glyph `fontdb::ID` lookup | `c1ef0ae` | about 3% at 3,000 |

Notes on the three that were not just a swap:

- **Draw order.** The "skip it for lists with no overlap" idea in the first
  version of this list does not apply to this scene: every label sits on its
  own plate, so every unit overlaps one. What helped is the grid under it. At
  64 px a cell held a dozen of these small boxes and each lookup tested them
  all. A smaller cell fixes that here but costs boxes that are big or overlap
  anyway (shadows with their blur): counted in cell visits plus overlap
  tests, 16 px is about 6 times the work of 64 px on a shadow grid and about
  half on this one. So `CellTuner` picks the cell by measuring: it tries a
  neighbour, keeps the cheapest, and measures afresh every 120 lists or when
  the unit count moves by a quarter. The grid only finds overlaps, so orders
  are identical (tested against the brute force at every cell size). Cells
  are reset only where they were written, since clearing all of 16 px's
  8,000 cells every frame cost a list that never reaches the grid about 20 us.
- **Owned text.** The type change is breaking (`TextBlock::content`,
  `DebugNode::text` and what `TextSelection::remember` keeps are `TextContent`;
  `TextBlock::new` and the theme and style helpers take `impl
  Into<TextContent>`, so every call that passed a `&str`, `String` or
  `&String` still compiles). The gain only appears for a caller that keeps its
  labels; one that formats a fresh `String` per frame copies it once, as before.
- **Not done: bounding a whole text block from its layout.** The per-glyph
  union can be replaced by one box over every corner only where nothing is
  clipped *and* every glyph is on the target; a block half off the edge, or
  inside a scroll view, would otherwise get a wider box than the per-glyph one,
  and a wider box can chain batches that a tight one keeps apart. `msdf` is now
  about 7% of the frame, so an exact version with those exceptions would give
  back a few percent at most.

After all of these, back to back with egui and iced in one session (animated,
mean ms per frame, load average 24, two runs each):

| cells | gameui | egui+wgpu | iced |
| ---: | ---: | ---: | ---: |
| 100 | 0.07 | 0.06 | 0.06 |
| 1,000 | 0.42 | 0.52 | 0.39 |
| 3,000 | 1.24 | 1.58 | 1.18 |
| 10,000 | 5.98 | 12.0 | 4.79 |

gameui went from 2.1 ms at 3,000 cells (and 9.5 at 10,000) to about 1.25 (and
about 5.4 to 6); iced is still 5% to 20% lower, egui is higher from 1,000
cells. The iced and egui figures here differ from the table above because the
load differs: only compare numbers within one table.

The shares in the profile table above are from one
profile; the per-phase ms were taken on a machine at load average 25 to 50,
which moves absolute numbers but not these shares much (two phase runs agreed
within 3%).

## More scenes (gameui, egui, iced)

The grid draws many short labels, redrawn in full. The scenes below cost
differently, so they say different things about a renderer. They run in the
gameui, egui and iced harnesses (not yet in BUI, gpui or Rune) after the grid
counts, unless one grid size is asked for on the command line. Each is checked
the same way as the grid: a frame is read back and must be lit and carry a real
amount of light ink (`check_text_frame`; the PNGs land in `shots/`).

| scene | what it draws | what it exposes |
| --- | --- | --- |
| `one` (grid) | the grid, where only cell 0 changes colour | whether unchanged work is reused; in an immediate-mode library it should cost the same as `animated` |
| `scroll` | 4 columns of 69 unique ~60-char lines, scrolling 3 px per frame; strings repeat from frame to frame | warm layout lookup, glyph preparation, many mid-size text blocks |
| `churn` | the same lines, but each line's text changes every frame | cold shaping and layout, atlas lookups, no cache can help |
| `para` | one wrapped ~24,000-character block, 2,500 px tall, scrolled through a 1,080 px window | whole-block work when most of the block is off screen (culling) |

`iced` has no absolute positioning, so its `scroll` scene pads each column's
top by the sub-line offset and draws rows 1.., and its `para` is a
`scrollable` set to the frame's offset with `scroll_to`.

### Results, 2026-11-08 (load average about 3; mean CPU ms per frame)

| scene | gameui | egui+wgpu | iced |
| --- | ---: | ---: | ---: |
| grid 3,000 `one` | 1.22 | 1.72 | 1.33 |
| `scroll` | 1.07 → 0.66 | 0.25 | 1.43 (p50 0.80) |
| `churn` | 5.24 → 4.80 | 2.40 | 3.85 |
| `para` | 1.39 → 0.39 | 0.13 | 0.40 |

The gameui arrows are before and after the text work of 2026-11-08, measured
by alternating the two binaries at load 6 to 23 (egui and iced were measured
once, before it; the 10,000-cell animated grid went from about 5.7 to 5.0 ms
in the same alternation). What changed, each its own commit: glyphs the target
cannot show are not placed (`para` 1.4 to 0.6), a quad's pixel range is found
by truncation instead of `ceil`/`floor` (library calls without SSE4.1) and a
block's glyphs reuse the colour and half-float conversions of the one before
(`scroll` 1.0 to 0.72, `para` to 0.44), and `round` goes the same way with a
remembered font key and size (about 10% more on the text scenes). `churn` is
cold shaping: about two thirds of it is cosmic-text's `shape`, which gameui
calls once per new string.

On text-heavy scenes gameui is the slowest of the three: egui's galley cache
lets a scrolled block cost almost nothing (`para` 0.13 ms), and gameui prepares
every glyph of a block whether or not it is on screen. These are the scenes
the structural items in `TODO.md` (cull text against the viewport, retained
prepared text) should be judged on.
## Retained mode: `ui-compare-gameui-retained` (gameui only)

The retained tree in `crates/retained` (`docs/design/retained-mode.md`) set
beside immediate-mode gameui on the same scenes, drawn by the same renderer
into the same target. Both binaries live in the `gameui` package, which keeps
the GPU setup in its `lib.rs`. Two scenes were added for this, to both gameui
binaries only:

| scene | what it draws | what it exposes |
| --- | --- | --- |
| `rows-*` | 10,000 rows of 20 px in a column scrolling through the whole window (54 in view), each a box and a 13 px label; starting at row 5,000. `stat`: nothing changes. `one`: row 5,010's value changes every frame. `scrl`: 3 px of scroll a frame. `hovr`: the pointer moves down 3 px a frame and the row under it takes the hover colour | what a change costs next to what is in view; culling; hover |
| `deep-*` | 10,000 filled leaves under four levels of inner nodes, ten children each, rows and columns in turn sharing their room (the grid's colours, no labels). `static`, `animated`, `one` as for the grid | what a tree holds and copies per level above a leaf |

Immediate mode draws the rows in view, found by arithmetic, and the leaves
where the tree would put them. The retained tree: grid cells are absolutely
placed boxes with their label as a child; rows are a scrolling column of
boxes, each with its fill a memo over its own hover signal and its label as a
child; the deep scene is the tree itself. Changes are written the way an app
would: `set_fill` per changed cell, `set_text`, `scroll_to`, `pointer_moved`.

What is timed, per frame: `retained` is the changes, `Ui::frame` and the same
`render` and `submit` as immediate mode; `ret-tree` is the tree's share of
those same frames (the changes and `Ui::frame`). A frame is drawn even when
`needs_frame` says nothing changed, as the immediate harness does; a host
following `needs_frame` would draw none, so for `static` its real cost is 0.
`build` is a scene made from nothing: the tree, its first frame (and, for the
rows, the scroll to row 5,000 and a second layout), and that frame's render,
timed five times with the renderer's caches warm. Immediate mode has nothing
to build: its first frame is a frame.

`UI_COMPARE_REUSE=1` turns on the renderer's frame comparison
(`UiRenderer::set_frame_comparison`, off by default), which draws a list in
which nothing moved from the plan kept for it; results are then named with
`+r`. It matters here because a retained tree hands the renderer the same
list, untouched, when nothing in it changed.

Each case prints its counters: the tree's work in the last frame
(`FrameStats`), what it holds (`TreeStats`, with `list_bytes` from the new
`DrawList::heap_bytes`, and `node_bytes`, the node table), text layouts shaped
over the run, and the renderer's last `RenderStats` (`plans_replayed`,
`boxes_computed`). A build prints what its last frame laid out (`visited`,
`laid_out`, `measured`, and `deferred`, the children a scroll sized without
laying them out).

The pictures were checked: `shots/retained-*.png` against `shots/gameui-*.png`.
The rows match line for line. The 10,000-cell grid is 80% lit against 95%
for immediate mode: its pitch is 14.33 by 14.4 px, and the tree snaps sizes
and positions to whole pixels, so cells are 13 px with gaps of 1 to 2 px. The
deep scene the other way round: immediate mode's leaves are 19.2 by 10.8 px
and show seams between them, the tree's do not.

### Results, 2026-11-11 (load average 2 to 5; mean CPU ms per frame)

One run of each binary, reuse off and then on. Immediate (`gameui`) against
retained (`retained`), with the tree's share in brackets.

| scene | gameui | retained (tree) | gameui+r | retained+r (tree) |
| --- | ---: | ---: | ---: | ---: |
| grid 100 `static` | 0.077 | 0.077 (0.000) | 0.082 | 0.075 (0.000) |
| grid 1,000 `static` | 0.373 | 0.301 (0.000) | 0.220 | 0.157 (0.000) |
| grid 3,000 `static` | 1.08 | 0.861 (0.000) | 0.610 | 0.405 (0.000) |
| grid 10,000 `static` | 4.54 | 3.74 (0.000) | 2.35 | 1.57 (0.000) |
| grid 100 `one` | 0.060 | 0.061 (0.007) | 0.045 | 0.062 (0.007) |
| grid 1,000 `one` | 0.369 | 0.374 (0.070) | 0.216 | 0.225 (0.070) |
| grid 3,000 `one` | 1.08 | 1.11 (0.214) | 0.614 | 0.662 (0.223) |
| grid 10,000 `one` | 4.56 | 4.75 (0.875) | 2.49 | 2.76 (0.948) |
| grid 100 `animated` | 0.061 | 0.075 (0.021) | 0.052 | 0.067 (0.022) |
| grid 1,000 `animated` | 0.368 | 0.503 (0.205) | 0.226 | 0.373 (0.206) |
| grid 3,000 `animated` | 1.07 | 1.51 (0.629) | 0.640 | 1.10 (0.650) |
| grid 10,000 `animated` | 4.41 | 9.21 (5.01) | 2.55 | 7.27 (5.03) |
| `rows-stat` | 0.193 | 0.188 (0.000) | 0.166 | 0.153 (0.000) |
| `rows-one` | 0.206 | 0.266 (0.062) | 0.180 | 0.236 (0.061) |
| `rows-scrl` | 0.198 | 0.201 (0.008) | 0.200 | 0.216 (0.008) |
| `rows-hovr` | 0.195 | 0.191 (0.002) | 0.159 | 0.159 (0.002) |
| `deep-static` | 0.706 | 0.420 (0.000) | 0.490 | 0.213 (0.000) |
| `deep-one` | 0.703 | 0.561 (0.135) | 0.504 | 0.325 (0.137) |
| `deep-animated` | 0.706 | 2.01 (1.52) | 0.589 | 1.70 (1.35) |

Builds (retained, median of five, ms): grid 0.15 / 0.87 / 2.8 / 14 at 100 /
1,000 / 3,000 / 10,000 cells; rows 108; deep 3.0.

What the tree holds and copies (counters, identical in every run):

| scene | immediate list | retained lists | node table | records held | records copied, one change |
| --- | ---: | ---: | ---: | ---: | ---: |
| grid 1,000 | 0.43 MB | 2.7 MB in 1,001 lists | 1.2 MB | 4,000 (2 per cell, twice) | 2,000 |
| grid 10,000 | 6.9 MB | 30.1 MB in 10,001 lists | 18.9 MB | 40,000 | 20,000 |
| rows | 27 KB | 305 KB in 110 lists | 18.9 MB | 432 | 216 |
| deep | 2.9 MB | 10.5 MB in 1,111 lists | 9.4 MB | 40,000 (each leaf 4 times) | 11,100 |

Reading it without overreaching:

- **Nothing changed**: the tree does no work (`ret-tree` 0), and a host
  following `needs_frame` would skip the frame entirely. Drawn anyway, the
  untouched list costs less to render than a rebuilt one, most with frame
  comparison on (grid 10,000: 1.57 ms against 2.35). Immediate mode has no
  way to know it could skip.
- **One change**: the tree's share is small next to the render, but it grows
  with what is in view, not with what changed: one changed cell of 10,000
  copies all 20,000 records into the root's list (0.88 ms), because the root
  holds every cell. In the deep tree the same change copies 11,100 records
  (one list per level) for 0.135 ms, and the whole frame is cheaper than
  immediate mode's (0.33 against 0.50 ms with reuse on). The render is most
  of every frame; with frame comparison on, a list in which only colours
  changed is drawn from its kept plan in both modes (`plans_replayed=1`).
- **Everything changed** (`animated`): the tree costs more than immediate
  mode, about twice the whole frame at 10,000 cells, since every cell is
  set, recorded and copied as well as drawn. A tree is the wrong shape for a
  scene that changes everywhere every frame.
- **Rows**: scrolling (8 us) and hover (2 us) cost the tree almost nothing:
  culling enters the rows in view and the hover moves by binary search. The
  one changed label costs more than in immediate mode (0.266 against 0.206
  ms), because the tree shapes every new label twice (below).

Problems the comparison found (not fixed; in the design doc's Outstanding
list):

- **Text is shaped twice.** A label is measured at the width it is offered
  (the row's content width) and drawn at its own measured width; the layout
  cache keys on the width, so each new or changed text is shaped once to
  measure and once to draw: 512 layouts shaped in 230 `rows-one` frames
  against 229 for immediate mode, and 54 extra on the first frame of every
  rows scene.
- **The rows build shapes 10,000 labels** that are not in view, to size
  rows whose height is fixed anyway: 108 ms, almost all cosmic-text shaping
  (profiled). Immediate mode shapes the 54 in view.
- **Small lists are expensive to keep.** A grid cell's list holds two
  records and takes about 2.3 KB (the list's own struct and its arrays'
  first allocations), so the 10,000 cell lists hold 23 MB for records the
  root holds again in 6.9 MB. The node table is 576 bytes a node (18.9 MB
  reserved at 20,001 nodes).
- **A change near the root's list copies all of it** (grid `one` above):
  the design's open question about whole-subtree caches, now measured.
