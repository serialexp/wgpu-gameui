# Done, not committed: the rest of Forge's charts (2026-10-09)

Bart asked: "Lets try building the other forge charts, then we can add those
to the benchmark after too." `LineChart`, `ScatterPlot` and `BarChart`'s
overlay are built, tested, in the gallery and in `benches/ui_stress.rs`;
`DrawList::triangle_strip` came along. What changed is in TODO_DONE.md "The
rest of Forge's charts". Left: commit when Bart asks, and rerun the chart
benches on a quiet machine (timings on 2026-10-09 were taken under heavy
load).

# Current task: SDF lines (anti-aliased strokes)

Bart asked (2026-10-08) to implement `docs/design/sdf-lines.md`. That doc's
"Implementation status" is the checklist; this section is how to resume.

## Where it stands

Phases 0-3 and the caller migration (5) are done and pass. Read the design
doc's status and "Known limitation" first.

- **Code.** `src/widgets/stroke.rs` (`Stroke`, `Cap`, `Join`, `Dash`, the
  CPU builder `build_segments`), analytic kind 3 in `src/render/ui.wgsl`
  (`shade_segment` and helpers), `DrawList::{line, stroke_line,
  stroke_polyline, stroke_closed}`, `Uniforms.view` (the fragment stage needs
  the exact screen-to-world mapping, so neighbouring segments agree on every
  corner pixel). `polyline` is gone.
- **Tests.** Unit tests in `stroke.rs` and `draw_list.rs`;
  `tests/strokes.rs` (GPU); `tests/stroke_browser_parity.rs` against
  `fixtures/browser/sdf-lines/` (README there says how to re-capture; any
  Playwright works, set `NODE_PATH`). Run the GPU tests with
  `cargo test --features headless --test strokes --test stroke_browser_parity -- --ignored`.
- **Checked on 2026-10-08:** lib tests (1651), both GPU suites, clippy
  (`--all-targets`, with `--all-features` and with `--no-default-features`)
  and fmt. The widget gallery was compared with HEAD: 16 images changed, all
  line-drawn (curve editor, line sample, spinner, pager arrows, placeholder,
  combo chevron, menu tick), and they look right.
- **agent-ui** (`crates/desktop/src/transcript_cards.rs`): card, panel and
  tile outlines are one `stroke_closed` (`edge_box`). Needs this gameui.

## Next

1. **Phase 4, arcs.** Decided 2026-10-08 (recommended, Bart said OK): an
   analytic **arc kind** in the same stream (full affine, reusing the segments' caps, dashes,
   coverage and clip), not angles on the ring instance. Covers `stroked_arc`
   and transformed `circle_outline`. Under uneven scale, measure in the arc's
   space and correct to screen pixels as segments do. Add Chromium cases (SVG
   `path` arcs) and GPU tests like the segments'. Moving circles themselves
   into the stream (open question 6) is a separate step afterwards, with
   before/after checks.
2. **Benchmarks.** Run `strokes_build` / `strokes_render` (and the
   `primitives_*` groups) on a quiet machine and record them in the design
   doc; on 2026-10-08 the load was too high for render timings to mean
   anything (recording was ~80-90 ns a line on both old and new).
3. Optional: the combo box chevron's mitred tip reaches a pixel lower than
   before; round joins would match the Phosphor carets. Bart hasn't decided.
4. Follow-ups are in TODO.md "SDF lines follow-ups".

## How to compare the gallery with HEAD

Export HEAD with `git checkout-index` into /tmp/gameui_head (no worktree),
run its gallery with `CARGO_TARGET_DIR=/tmp/gameui_head/target`, then
`magick compare -metric AE` per image; build side-by-sides with `+repage`
(the PNGs carry page geometry, so `+append` otherwise shows one image).

---

# Also open: build the Forge components the gallery is still missing

The unfinished Slug text-rendering A/B experiment (2026-10-02) is parked on
the `exp/slug` branch, off main since 2026-10-08; its progress notes are
there.

Bart asked (2026-09-26): "pull the currently missing things from designsync
and build those components". Source of truth is the Forge Design System in
Claude Design (project `1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`): each
component has `components/<group>/<Name>.jsx`, `.prompt.md` and `.d.ts`.
Fetch them with the DesignSync tool (`get_file`).

The previous task (Forge-style gallery) is done; its notes are in git
history (`git log -- CURRENT_TASK.md`).

## Missing (23 of 77, from `test_output/widget_gallery/index.html`)

Batches, smallest first; one commit per batch:

- [x] A — small: CountBubble, DropZone, FieldLabel, StatusIcon, Placeholder,
      Panel (reworked to Forge), DockSection (gallery section only).
      Notes in TODO_DONE.md "Missing Forge components, batch A".
- [x] B — dialogs: Modal, AlertDialog, ConfirmDialog, PromptDialog, Sheet.
      Notes in TODO_DONE.md "Missing Forge components, batch B". Coverage is
      66 of 77.
- [x] C — DragList. Notes in TODO_DONE.md "Missing Forge components, batch C".
      Coverage 67 of 77.
- [x] D — inspector: PropertyRow, PropertyGroup, FileField, Inspector.
      Notes in TODO_DONE.md "Missing Forge components, batch D". Coverage 71
      of 77. Open question for Bart: Forge hides the Inspector body's
      scrollbar (`scrollbarWidth: none`); ours shows the thin overlay
      thumb, since `ScrollView` has no hidden-bar mode.
      Bart's decisions (2026-09-26):
      - Inspector body: a closure gets a vertical cursor
        (`body.take(h) -> Rect`); the body's height is measured while
        drawing and feeds the scroll (one frame of lag on the clamp is ok).
      - Slider/Checkbox "mixed" look: not in this batch — TODO item.
        PropertyRow and FileField have their own mixed state.
      - Ungate `IconKey`: only `IconKey::new(PhosphorIcon)` stays behind
        `phosphor-icons`; `IconKey::glyph` works everywhere.
      - PropertyRow matches Forge: scrub the label or step ▴▾, no typing.
- [ ] E — mobile: AppBar, TabBar
- [ ] F — CommandPalette
- [ ] G — Console
- [ ] H — SettingRow, SettingsPanel (overlap with the existing
      `SettingsForm` in `src/widgets/settings.rs` — ask Bart how to
      reconcile before building)

## Notes

- **Gallery pages.** The canvas is too tall for one texture, so the
  gallery renders pages of at most `PAGE_MAX` (4096) rows through
  `UiRenderer::set_view_origin`, split between sections. A section taller
  than a page fails the test with a clear message. Bart: no full-canvas
  image is wanted, only the section and cell images and the index page.
- **Debug-report check per batch.** Compare the gallery's problem list
  with the previous commit's. Export that commit's tree into /tmp with
  `git show <rev>:<path>` for every file of `git ls-tree -r --name-only
  <rev>` (no `git archive`/`worktree`: not on the git allowlist), run its
  gallery with `CARGO_TARGET_DIR=/tmp/gameui-head-target`, and diff
  `code name` pairs from `test_output/widget_gallery.debug.json`, sorted,
  with `#<digits>` normalised to `#N`. After batch B the list equals
  3cbe69e's (113 problems). After batch C it is that list minus 13 false
  `sibling_overlap`s between layers and the base (101 problems); a copy
  of the batch B list is `/tmp/probs-b.txt` while it lasts. Batch D adds
  nothing: still the same 101 (`/tmp/probs-c.txt`).
- **Scrolled content and the pointer.** `ScrollView::begin` translates
  drawing, not input. A widget that scrolls other widgets must hand them
  the pointer moved by the offset (and consumed outside the viewport):
  `DrawContext::reborrow_with_input`, as `Inspector::draw_body` does.
- **Eyeball small text closely.** The PropertyGroup "…" bug only showed at
  some x positions; zoom crops with `convert <png> -crop WxH+X+Y -scale
  600% /tmp/crop.png` catch what the full section image hides.
- Widgets whose ghost/popup floats over other content draw it on a layer
  after the base pass (the gallery does this for DragList's ghost), so the
  debug report sees it stacked, not as a sibling.
- Raised surfaces (Sheet, Toast) declare their shadow in their debug scope
  via `BoxShadow::ink_rect`; gallery cells for free-standing sheets reserve
  the shadow's room (`sheet_cell`) so it doesn't fall on neighbours.
  Bart (2026-09-26): keep that empty room — accurate warnings are worth
  the extra space. Don't pack shadowed cells tight.
