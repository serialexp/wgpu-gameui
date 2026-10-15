# Current task: video playback (2026-10-14)

Bart asked for a video component in pure Rust, with sound. The plan is
`docs/design/video-playback.md`, and its "Implementation status" is the
checklist; read it first.

- Phase 0 (the workspace: `crates/core`, `crates/media`) is committed
  (`8cd9515`, agent-ui `1137178`).
- Phase 1 (core) is committed: in-place sprite updates, YUV sprites with
  colour handling, full screen on request, and the media icons. A review
  agent checked it; its findings were fixed, and the ones left are in TODO.md
  "Video phase 1 follow-ups". It changes `UiFrameResult` (a `fullscreen`
  field), so agent-ui's `crates/desktop/src/redraw.rs` tests changed with it.
- Phase 2 (`crates/media`: MP4, H.264 and AAC on a worker, clock, seeking,
  states, `Player`) is written and its design-doc items are checked off
  (2026-10-15); a review's findings are fixed (listed in the design doc),
  not yet committed. One test fails on a `rust_h264` 0.4.0 chroma bug (see the
  design doc's "Phase 2 leftover"); Bart is choosing how to carry the fix:
  a serialexp fork plus `[patch.crates-io]` and an upstream PR, a local path
  patch, or a tolerant test. A patched clone was in /tmp/rust_h264_git.
- Next: phase 3, sound (`cpal`, `rubato`, the device clock).
- The research clones (rust_h264, re_mp4, symphonia, cpal) were in
  /tmp/video-research; clone them again if /tmp was cleared.

# Current task: Timeline and viewport gizmos from Forge (2026-10-11)

Bart: "we recently added a timeline component and a bunch of editor gizmos to
the gameui forge design in claude, and we need that for an editor now, so do
you think you could actually build those components into gameui?" Source:
Claude Design project `1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`,
`components/timeline/*` and `components/viewport/*` (fetch with DesignSync
`get_file`).

Bart's decisions (2026-10-11):
- Gizmos take a **3×3 view rotation** (world→view), with a `from_yaw_pitch`
  helper for Forge's convention.
- Timeline shortcuts need letter keys: **add the generic key list**, i.e.
  `KeyState` / `InputState::keys` as designed in `docs/design/menubar.md`
  §4 (Phase 4).
- HUD glass: **no backdrop blur for now**; note a draw-list backdrop
  primitive in TODO.md.
- Timeline track data: **`&mut` edited in place**, the output says what
  changed (for undo).

Plan:
- [x] 1 — foundations: `KeyState` + `InputState::keys` (+
      `Accelerator::matches`); theme tokens for axis tints, accent ring /
      glow, warn soft / banner ink, HUD glass chrome; `Tone::Latch`.
      Also `ContextMenu::with_min_width` / `ContextMenuState::open_above`
      (for HudSelect) and seven Phosphor icons (transport, readout glyphs).
- [x] 2 — viewport: `src/widgets/hud.rs` (`HudBar`/`HudKey`/`HudItem::
      Separator`/`HudSelect` + `HudState`, `hud_glass`), `view_gizmo.rs`
      (`ViewRotation`, `AxisDir`, `ViewGizmo`), `transform_gizmo.rs`
      (`TransformGizmo`, events Start/Drag/End/Cancel, totals since start),
      `viewport_overlays.rs` (`TransformReadout`, `SelectionMarquee`,
      `MeasureLine`, `ViewportStats`). Unit tests in each; gallery has new
      `Viewport` and `Timeline` categories, all ten viewport sections
      rendered and looked at.
- [x] 3 — `Timeline` (`src/widgets/timeline/{mod,model,tests}.rs`, 36
      tests), `TimelineChrome` + `StyleKey::WarnRule`; gallery section with
      seven cells, rendered and looked at (also under aldermoor-hearth).
- [x] 4 — TODO.md follow-ups section (backdrop blur, rename-field font,
      overlapping marker labels, Forge graph category), TODO_DONE.md section
      "2026-10-11 — Timeline and viewport gizmos". `cargo build
      --all-targets` warning-free, `cargo test --lib` 1741 passed,
      designer 14 passed, gallery 85 of 91. Remaining fmt diffs and the
      clippy warning are in the parallel agent's `src/render/textures.rs`,
      `src/render/ui_renderer.rs`, `tests/external_sprite.rs` — left alone.
- [x] Commit — 85e741f, rebased onto upstream's focus and chart work
      (2026-10-14).

---

# Also open: SDF lines (anti-aliased strokes)

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

# Also open: theme designer and presets (2026-10-05)

Bart: "make something like a theme designer that will allow us to configure
the styling of gameui, and potentially export it to be used as a completely
different preset", with live preview. Design and Bart's decisions:
`docs/design/theme-designer.md`.

- [x] Phase 1 — library: `StyleKey` names + complete list, OKLCH inverse,
      `ChromeTheme` visitors, `ThemeSeeds`/`ThemePreset`/`resolve`, TOML
      behind feature `preset`.
- [x] Phase 2 — gallery honours `GAMEUI_PRESET`.
- [x] Phase 3 — `designer/` workspace crate (`gameui-designer`). Forge
      chrome; Seeds dock (left), Tokens dock with filter + colour/number
      override editor (right), proof page in the viewport. File menu
      New/Open/Save/Save as/Quit with discard confirmation; Preset menu
      resets seeds / clears overrides. `--capture out.png [--size WxH]
      [--select token]` renders headlessly (captures land in
      `test_output/designer/`). 14 tests incl. pointer-driven ones (slider
      drag, filter + hex override) that locate widgets by debug scope.
- [ ] Phase 4 — the gaps listed in the design doc. The designer makes them
      visible: e.g. slider knob and prompt-dialog error ink are hard-coded.
- [ ] Not yet: the full gallery inside the designer (design doc gap 8);
      three primaries (gap 6); chrome overrides (gap 7).

The Slug A/B experiment is off main (parked on `exp/slug`). Its 2026-10-05
work (the GPU path, `slug_compare`, `tests/slug_render.rs` and
`docs/design/slug-experiment.md`, awaiting Bart's verdict) is in commit
`d7dc71f`, not on main.

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

## UTF-8 retained editor crash fix (agent-ui follow-up)

External buffer replacement previously kept any cursor byte offset below the new
length, even inside a multibyte character. Subsequent typing panicked in
`String::insert_str`. Added `TextInput::set_value`, which reuses the value buffer,
clamps/snaps the caret backward to a UTF-8 boundary (at most three bytes), and
clears old selection, drag anchor/unit, and sticky navigation column. The shared
UiContext retained-field path now uses it. Replacement remains O(new text bytes),
with no added full-text scan or per-character allocation.

Regression tests cover 2/3/4-byte characters, valid boundaries, shortening, empty
replacement, stale drag state, and subsequent editing in text/password/textarea
fields. Added a Unicode replacement-and-typing gallery cell. Gameui lib tests
(1671 passed, 14 ignored), strict all-target Clippy, gallery render, desktop lib
(188 passed, 5 ignored), and touched-file formatting passed. Workspace formatting
reported concurrent unrelated view_gizmo/gallery edits; those were not reformatted.
No service actions or commits. Vulkan's secondary unwind panic remains outside
this gameui editor fix.
