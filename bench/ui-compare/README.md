# UI library comparison

How wgpu-gameui, egui and iced (gpui and BUI to follow) compare on the same
scene. A separate Cargo workspace on purpose: gpui's dependency tree and BUI's
GPL licence must not reach the published crates or CI.

```
cd bench/ui-compare
cargo build --release
DISPLAY=:0 ./target/release/ui-compare-gameui   # or -egui, -iced
DISPLAY=:0 ./target/release/ui-compare-egui 1000  # one cell count only
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

The `egui+wgpu` row is the one to put beside gameui and iced, which both
include their GPU hand-off. A real iced or egui app would skip a frame in which
nothing changed; here a frame is forced either way.

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
from this tree (wgpu 24). Exact lock in `Cargo.lock`.
