# UI library comparison

How wgpu-gameui, egui, iced, gpui and BUI compare on the same scene. Separate
from the published crates on purpose: gpui's dependency tree and BUI's GPL
licence must not reach them or CI. There are three Cargo workspaces here, each
with its own lockfile and `target/`:

- `.` : `common`, `gameui`, `egui`, `iced`;
- `gpui/` : gpui pins crates (`unicode-properties =0.1.3`) that egui cannot
  share a lockfile with. It builds from the zed checkout at `~/Projects/zed`
  (a sibling of this repo; the aeolun fork's `main`, `e231594b6b` of 6 Nov 2026)
  and carries zed's `[patch.crates-io]` entries, which a path dependency does
  not inherit;
- `bui/` : the only place BUI (`blenderui` 0.4.1, GPL-3.0-or-later, a C core
  built with CMake and a C compiler) is linked.

```
cd bench/ui-compare
cargo build --release                          # gameui, egui, iced
(cd gpui && cargo build --release)
(cd bui && cargo build --release)
DISPLAY=:0 ./target/release/ui-compare-gameui   # or -egui, -iced
DISPLAY=:0 ./target/release/ui-compare-egui 1000  # one cell count only
DISPLAY=:0 gpui/target/release/ui-compare-gpui
DISPLAY=:0 bui/target/release/ui-compare-bui
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
| bui(cpu) | BUI headless (`Application::headless()`, as its own `raster_probe` example): `render`, layout, **CPU software rasterization** of the whole frame, and a copy of the 1920x1080 pixels out (about 8 MB, which `App::draw` always makes) |

The `egui+wgpu` row is the one to put beside gameui and iced, which both
include their GPU hand-off. A real iced or egui app would skip a frame in which
nothing changed; here a frame is forced either way.

Two rows are not like for like with the rest, and the README should not let a
table hide it:

- `bui(cpu)` rasterizes on the CPU; everything else rasterizes on the GPU
  after the hand-off. Its number contains work the others leave to the GPU, so
  read it as BUI's cost per frame in its default mode, not as its draw-call
  cost. BUI's GPU path (OpenGL 3.3, `.gpu(true)`) needs a real window and
  belongs to the GPU-timing phase.
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
git revision), blenderui 0.4.1. Exact locks in each workspace's `Cargo.lock`.

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
