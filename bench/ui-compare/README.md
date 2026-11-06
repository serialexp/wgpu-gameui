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
`contains_key`); neither was changed.

What is left, in the order the profile puts it, with what each would change:

1. **Pixel bounds for text** (about 16% to 40% depending on how inlined
   callers are counted): bound a glyph quad once instead of through two
   clipped triangles, or bound a whole block from its layout size and clip.
   Both change what `DrawOrder` is given, so the draw-order equivalence tests
   (`brute_force_orders`) are what would show whether the order stays the same.
2. **Draw-order ordering** (about 17% together): it exists so batches that
   overlap keep paint order. A list in which no unit overlaps another, as this
   grid, gets nothing from it; a cheap test for that, or a coarser grid for
   text, is the design question in `docs/design/draw-call-batching.md`.
3. **Owned text** (about 10% malloc/free/drop plus part of the list-building
   share): `TextBlock` owns its string, so every frame allocates one per label.
   An `Arc<str>` or `Cow` would remove it but changes the public type.
4. **Cache hits** (`LayoutCache::entry` three hashes, the `fontdb::ID` map).

Item 1's first half was then tried (`1e95fe9`): a glyph quad is bounded once
from its four corners, and the clip's bound once per run of quads sharing it.
Back to back against the commit before it (three alternating rounds, machine
at load average 20 to 30), the frame went from about 1.88 to 1.72 ms at 3,000
cells (8%) and from 8.77 to 8.38 ms at 10,000 (4.5%). That is well under the
16% the profile's `triangle` and `union` frames suggested: inlining spreads
the cost across callers in `perf`, so those shares overstate what one function
can give back. Axis-aligned quads give the same box as before (tested against
the old two-triangle method), and rotated ones never a smaller one. Items 2 to
4 and bounding a block from its layout size have not been tried. The measurement behind the table is one
profile; the per-phase ms were taken on a machine at load average 25 to 50,
which moves absolute numbers but not these shares much (two phase runs agreed
within 3%).
