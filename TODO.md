# wgpu-gameui TODO

Consolidated from two independent audits (Claude + Codex) on 2026-04-26 of the
~2,630 LOC source tree just extracted from citybuilder. Both audits agreed on
the same major gaps. Items grouped by category and tagged with priority:

- **P0** — blocks 1.0 / blocks Teardown-API port
- **P1** — important, but the lib is usable without it
- **P2** — nice-to-have

Use this as the working backlog for the package. Cross items off as PRs land:
a finished item moves to [TODO_DONE.md](TODO_DONE.md) with a short note
describing the API that closed it, so this file stays the list of what is
still open.

---

## Code editor follow-ups (2026-10-16)

- [ ] **Show IME composition** (preedit) at the caret in `CodeEditor`.
- [ ] **Word delete and line moves**: Alt/Ctrl-Backspace, Alt-Up/Down.
- [ ] **Long non-ASCII lines** are shaped whole when in view; window them as ASCII ones are.
- [ ] **Sideways scroll width never shrinks** (widest line seen so far).
- [ ] **Injections** (Markdown fences, script in HTML) and **WGSL** (needs a grammar on
      Tree-sitter 0.27).
- [ ] **Diagnostics and completion** from a language server.

## Video phase 1 follow-ups (review, 2026-10-14)

- [ ] **Change a YUV sprite's colour settings** without making a new sprite
      (`set_yuv_colour`: rewrite its uniform at the next flush). A stream whose
      settings arrive after the first frame needs it; until then the media
      crate recreates the sprite, which gives it a new id.
- [ ] **Chroma siting.** `fs_yuv` takes colour samples as centred between the
      brightness samples; H.264's default (and its VUI `chroma_sample_loc`)
      puts them in line with the left column, a quarter pixel sideways. Carry
      the siting in `YuvColour` and offset the colour planes' UV.
- [ ] **Refuse a nine-slice of a YUV sprite when it is registered.** Today
      `register_nine_slice` accepts it and the draw is skipped with a warning.
- [ ] **Settings key capture reads the raw Escape key** (`widgets/settings.rs`,
      `key_escape`), so one Escape both cancels a capture and reaches anything
      else that takes `nav.cancel` (a full-screen owner, focus).

## Video phase 4 follow-ups (2026-10-16)

- [ ] **"Can't play this video" is in `DangerText`.** Forge colours it
      `--danger-rule`, which has no `StyleKey`; add one if the two differ
      enough to matter.
- [ ] **The placeholder's time has no letter spacing.** Forge spaces it
      0.12em; `mono_block` has no tracking.
- [ ] **The player's frame isn't clipped to its border's rounded corners.**
      A picture's corner pixels show just outside the curve; `DrawList` has
      no rounded clip.
- [ ] **Full screen can wait forever for a window that never changes.**
      `FullscreenState` ignores a report contradicting the change it is
      `awaiting`, and only a report of that state clears it. If the app's
      window change brings no resize (a window already the screen's size),
      no report comes, and a later real "not full screen" (the OS's own
      key) is dropped as stale: the owner keeps the screen over a windowed
      window until Escape. Needs some way to end the wait that isn't a
      timer: the app reporting the window's state after applying the
      change, say, if that can be told apart from macOS's stale reports.
- [ ] **`out_of_pictures_the_clock_waits_for_them` fails under a full
      workspace run.** Found 2026-10-21: it passes alone, but in
      `cargo test --workspace --all-features` it failed in most runs, the
      clock at 133 ms where it expects picture 3's 66 ms
      (`crates/media/src/player_tests.rs:367`). Under load the worker
      seems to decode past the two pictures `Ahead::frames` allows before
      the test checks, or the clock reads a later picture; a race between
      the test and the decoding worker, not a timing assert, so it wants
      the test (or the player) to wait on what the worker did.

## Video phase 3 follow-ups (review, 2026-10-16)

- [ ] **Pictures dropped around a late device start.** On Bart's Mac
      (a 44.1 kHz output with about 320 ms of latency, opening in 1.3 to
      2.4 s), the real-device test shows 8 of 60 pictures dropped, likely
      around the restart after the device opens late. Look at what the clock
      does there.
- [ ] **Open the device before it's needed.** The device opens when the
      player does, and a Bluetooth one took up to 2.4 s: an app that plays
      right after opening waits up to a second, then plays without sound
      until it's ready. Sharing one open device between players would also
      help.
- [x] **agent-ui needs the media profile lines.** `[profile.dev.package]`
      only applies at the workspace root: an app using `wgpu-gameui-media`
      should copy gameui's `opt-level = 3` for `wsola`, `rubato`, `realfft`
      and `rustfft`, or a debug build may not stretch sound in real time.
      (Done in agent-ui's root `Cargo.toml`, with `rust_h264` and
      `symphonia-codec-aac`.)
- [ ] **Reopen the device.** Any `cpal` stream error ends the sound for good
      (the picture goes on by the wall clock), including errors that don't
      stop the stream, and a new default output (headphones plugged in) isn't
      followed. Reopening on failure and on a default-device change would fix
      both.
- [ ] **Pause, seek and a new speed cut the sound.** What follows fades in,
      but what stops doesn't fade out: the stream pauses (or the ring is
      skipped) mid-wave, which can click. A fade-out needs the callback to
      play a short ramp before the pause takes effect.
- [ ] **Sound starved mid-play shows as playing.** If the worker falls behind
      on sound, the clock waits with the device, but the state stays
      `Playing`, not `Loading`.
- [ ] **The clock can step back a few milliseconds** when a callback runs
      late (the frame heard is set as is). Harmless for picture choice.

## Video phase 2 follow-ups (2026-10-15)

- [ ] **Carry the `rust_h264` chroma DC fix.** `dequant_chroma_dc` rounds
      where the standard doesn't (eq. 8-330), so chroma is ±1 off at low QP
      and `media`'s ffmpeg MD5 test fails. Details and the one-line fix are in
      `docs/design/video-playback.md`, "Phase 2 leftover".
- [ ] **A damaged index can still exhaust memory inside `re_mp4`.** Its
      `Mp4::read` sizes tables from counts in the file before checking them
      against the file's length; an allocation that fails aborts the
      process, which `catch_unwind` can't catch. Needs a check in `re_mp4`
      (offer upstream), or a sanity pass over the box sizes before handing
      it the file.
- [ ] **A video sample without a picture stops playback.** One holding only
      SEI or an access unit delimiter is `Damaged` today. Some muxers write
      them; skipping such a sample (the reorder already takes a `None`)
      would be more forgiving. A policy call for Bart.
- [ ] **Caught `re_mp4` / `rust_h264` panics still print.** `Container::open`
      and the decoders catch them, but the default hook has already printed
      the message and backtrace to stderr by then. An app that cares installs its own hook;
      the crate shouldn't swap the process-wide one.

## Chart benches on a quiet machine (noted 2026-10-09)

- [ ] **Rerun the chart benches** in `benches/ui_stress.rs` (`LineChart`,
      `ScatterPlot`, `BarChart`'s overlay) on a quiet machine: the timings
      taken on 2026-10-09 were under heavy load. The same goes for the
      `batching` group's times recorded on 2026-10-16 in
      `docs/design/draw-call-batching.md` (load average 10 to 25).

## Multi-stop gradients (noted 2026-09-29)

- [ ] **Gradients with more than two colours** (`linear-gradient(red, blue 40%,
      green)`). `Background::LinearGradient` and `DrawList::linear_gradient`
      take two stops, which is all the design comps use so far; agent-ui will
      need more at some point. The chrome instance is fixed-size (ten `vec4`s,
      no spare room for stops), so this needs either a wider instance for the
      gradient kind, or stops in a small ramp texture/storage buffer indexed
      per instance. Keep positions as CSS gives them (optional per stop,
      spread evenly when missing), and the direction/span maths of
      `GradientAxis::position` unchanged. `GradientStop`/`gradient_ramp::sample`
      (the editor widget) already model stops and could be shared.

## Gallery foundations cards (2026-09-26)

- [ ] **Draw Forge's Foundations cards in the widget gallery, from `Theme`.**
      Forge has cards for Colors (accent roles, axis tints, ink ladder,
      semantic hues, host surfaces, washes), Depth (two-line edges, key
      states, raised/sunken/floating), Focus, Motion, Spacing (radius,
      heights, gaps) and Type (carved text, Plex Sans, Plex Mono). Drawing
      them from our `Theme` would show at a glance when a token drifts from
      the design. Follow-up to the Forge-style gallery reorganization (see
      CURRENT_TASK.md).
- [ ] **Clear the gallery's debug-report problems.** The gallery writes
      `test_output/widget_gallery.debug.txt`; it lists 104 problems (113
      before the reorganization, so these are older). Errors:
      `dropped_degenerate` in CurveEditor, Toast (3) and Window (primitives
      with non-positive size); `missing_paint` in DragHandle (2), List and
      ScrollView (a reserved box that drew nothing). Warnings:
      `invisible_alpha` (5), `overflows_declared` (5), `sibling_overlap`
      (5), `near_miss_alignment` (2). Each is either a real widget bug or a
      false positive in the report; sort them out one by one.

---

## Rotated quads and circles still tessellate (found 2026-09-26)

- [ ] **Decide whether rotated `quad()` and circles should go SDF too.**
      Rotated chrome, rounded rects and outlines are now one SDF instance
      (smooth edges). `DrawList::quad` still falls back to hard-edged soup
      triangles under rotation/scale, and `circle`/`circle_outline` fall back
      to a triangle fan (circles only care about scale, since rotation
      doesn't change them). Trade-off for quads: SDF edges are anti-aliased,
      so two rotated quads that share an edge would show a faint seam, while
      soup quads meet exactly. Circles would need the instance to carry a
      scale (or the full affine) instead of a post-transform centre.

---

## Widgets that measure text in another font than they draw (found 2026-09-26)

- [ ] **Check every `measure_text(.., None)` caller against the font its
      text is drawn in.** `measure_text` shapes in the default sans. Mono
      text measured that way comes out too narrow (the agent-ui status bar,
      chips and dock cards overlapped until they switched to
      `StyleResolver::mono_width`), and any widget drawing with
      `theme.font` is measured wrong once a theme sets that font. About 40
      callers remain (badge, breadcrumb, combo_box, context_menu hints,
      doc_tabs, panel, radio, status_bar, table, tabs, tag_input, tooltip,
      asset_grid, progress_bar, popover). For each: measure the block it
      draws (`measure_block`), or use `mono_width`/`sans_width`.

---

## Per-frame text allocations (found 2026-09-25, GroupList perf test)

- [ ] **P1 — A `TextBlock` costs 3 heap allocations to build.** Found while
      measuring `GroupList` (`tests/group_list_performance.rs`); it applies to
      every widget that draws text, every frame. The three are: the owned
      `content: String`; `style_ranges: Arc::new(Vec::new())`, which allocates
      even when empty; and `FontHandle(String)`, cloned from the theme by
      `StyleResolver::{sans,mono}_block`. Options to decide between (Bart):
      a shared empty `Arc` (or `Option<Arc<…>>`) for `style_ranges`;
      `FontHandle(Arc<str>)` so clones are free; and, bigger, borrowed or
      `Cow`/`Arc<str>` content. The GroupList perf test derives its bound from
      the measured per-block cost, so it tightens automatically.

---

## Hollow keys (found 2026-09-25, ghost plinth change)

- [ ] **The design's NumberField steppers and Toolbar keys are default-tone
      hollow keys; gameui draws them as ghost hollow keys.** Ghost keys now sit
      on the plinth like the Forge `Key` (`Material::hollow` leaves it out). The
      stepper (`number_input.rs`) and toolbar overflow key (`toolbar.rs`) were
      ghost before and stay ghost + hollow, so they look as they did. Matching
      the design means a raised default-tone face with no plinth; check it
      against Forge `NumberField.jsx` / `Toolbar.jsx` and the gallery first.

---

## Cross-notifier follow-up audit (2026-08-19)

These items came from regressions found while migrating cross-notifier to gameui.
The crate already has `SizeSpec::Fit`, constraints, reusable `LayoutResult`,
font-aware `TextBlock` measurement, `LayerStack`, animation clock plumbing,
`ScrollView`, and extensive debug reports. The work below should integrate and
harden those foundations rather than create parallel replacements.

### P1 — Complete the pipeline

- [ ] **P1 — Extend intrinsic measurement with context and two-pass container
      layout.** The first vertical slice is landed: `MeasureContext` carries the
      resolved style/font, logical constraints, scale, and wrap policy;
      `Measurement` exposes min/preferred/max, baseline, and prepared-width
      identity; reusable `MeasureBuffer::arrange_{h,v}stack_into` provides plain
      measured children, baseline rows, and explicit `WidthMismatch` diagnostics
      without remeasuring or replaying callbacks. `UiContext::measure` and
      `measure_text_button` bridge the active scope. Still outstanding: migrate
      text inputs, panels, image buttons, and list/table rows; add richer automatic
      nested propagation and same-frame measured scrolling. Design contract:
      `docs/design/contextual-widget-lifecycle.md`.

- [ ] **P1 — Make font-aware measurement the only widget-layout path.** Structured
      `TextMetrics` now reports line-box size, ink bounds, first baseline, line
      count, and width overflow; `MeasuredText` transfers its configured block to
      paint without cloning. Continue by deprecating
      default-font `measure_text` for layout and migrating remaining internal users
      (including panel, table, and progress calculations) to resolved
      `TextBlock` measurement. Return structured metrics (advance, line box, ink
      bounds, baseline, line count), and where practical reuse the measured text
      layout for painting so measurement, ellipsis, caret placement, and render
      cannot disagree.

- [ ] **P1 — Harden animation IDs and lifecycle.** Replace untyped `(u64,
      AnimSlot)` usage with scoped/typed widget IDs and generations; diagnose
      duplicate IDs and reuse by another widget kind, and support explicit
      cancellation/removal. Dynamic lists must not transfer animation state when
      items are inserted, removed, or reordered.

- [ ] **P1 — Add stable, reusable keyed layers and atomic compositing groups.**
      Avoid allocating a transient `DrawList` for every pushed layer each frame.
      Retain layer storage by `LayerId`, clear/reorder it at frame start, and keep
      stable debug/cache identity. Separately support compositing groups for
      rounded clipping, transforms, shadows, and group opacity so a translucent
      card can be composited atomically rather than making every child primitive
      independently translucent.

- [ ] **P1 — Improve the existing `ScrollView`, rather than adding another one.**
      `src/widgets/scroll_view.rs` already provides clipping, wheel input, thumb
      dragging, and caller-owned `ScrollState`, and `UiContext` exposes
      `scroll_begin`/`scroll_end`. Integrate it with measured interactive
      row/column layout so content extent can be known in the same frame instead
      of being supplied from the previous draw. Make transformed child hit nodes,
      automatic clamping after content changes, `scroll_to`/`ensure_visible`, and
      fixed headers/footers compose naturally. Migrate both cross-notifier
      settings and notification center to validate the improved API.

### P2 — Diagnostics and application ergonomics

- [ ] **P2 — Expose declarative stack layout in voxel's two Lua adapters.** Add
      mirrored `UiLayoutRow`/`UiLayoutColumn` and `UiRectBegin`/`UiRectEnd`
      bindings over the plain-data APIs, return ordered `children` plus `by_id`,
      domain-separate string/integer external IDs, reject duplicate/malformed IDs,
      and retain `UiVStack` as a compatibility wrapper. Layout remains local-space
      geometry and scripts are never replayed for measurement.

- [ ] **P2 — Extend `DebugReport` with z/input/animation diagnostics.** Include
      final command order and pass group, stable layer/z identity, clip/transform
      chain, interaction bounds, hit candidates and winner, widget identity,
      active animation slots/next repaint, and resolved font identity. Warn when
      overlapping scopes in one `DrawList` cannot preserve intended painter's
      order, and when painted and interactive bounds diverge.

- [ ] **P2 — Explain layout decisions and provide semantic test assertions.**
      Report intrinsic/measured/allocated/painted sizes, sizing policy, baseline,
      overflow amount, and the reason for ellipsis. Add helpers such as
      `assert_clean`, `assert_no_unexpected_ellipsis`, `assert_hit_target`, and
      `assert_visual_order`, with scoped exceptions for intentional clipping or
      ellipsis. Add a shared widget conformance suite covering measurement,
      painted bounds, hit bounds, constraints, font changes, disabled/focus
      behavior, scaling, and clipping.

- [~] **P2 — Add semantic form/layout conveniences on top of interactive rows.**
      Provide `FormRow`, `FormGrid`, `FieldLabel`, `Section`, `InlineError`, and
      trailing-action patterns, plus compact/application/game-menu density
      presets. Add standard text, icon, and icon+text button variants and generate
      the Phosphor enum/codepoint mapping from bundled-font metadata.
      *(First slice landed 2026-09-16: `SettingsSpec`/`SettingsForm` covers
      label|control rows, `Section` headers, and game-menu usage; text/color
      fields, `InlineError`, density presets, and generic `FormGrid` remain.)*

- [ ] **P2 — Add optional host integrations.** Provide a winit input adapter for
      logical coordinates, text/key/mouse/wheel routing, and per-window state;
      provide one surface-host rendering path for clear/load policy, `DrawList`
      or `LayerStack`, submit, and present.

- [ ] **P2 — Reuse interaction dispatch scratch.** `InteractionScene::begin_frame`
      currently allocates a fresh sorted hit vector via `collect()` every frame,
      and duplicate-ID detection scans the current regions linearly. Retain flat
      hit/ID scratch in the caller-owned scene and clear it without shrinking so
      large interactive surfaces do not pay per-frame/per-widget allocation work.

### Suggested implementation order

1. Stable widget/interaction IDs and shared paint/input ordering.
2. Emitted interaction geometry and standard widget `Response`.
3. Interactive rows/columns over the existing layout engine.
4. Measurement context and measured `ScrollView` integration.
5. Frame repaint/deadline output and robust animation lifecycle.
6. Keyed layers/compositing groups, diagnostics, and form/host conveniences.

---

## Theming / Styling


---

## Game / Teardown-Specific

- [ ] **P1 — UI sound hooks.** `UiSound`/`UiSoundLoop` and button
      hover/press sounds. **Deferred to the integrating app** (decision
      2026-06): this library is render-only and has no audio backend, so the
      app owns sound — it already gets the interaction edges it needs from the
      widget return values + `HitZoneOutput` (`clicked`/`pressed`/`hovered`/…)
      to trigger its own SFX. Re-open only if a built-in hook proves necessary.
- [ ] **P2 — Mod-friendly *widget* registration** (`register_widget(name,
      draw_fn)`), **gated on Lua integration** (decision 2026-06; tracked under
      "Beyond 1.0 → mod registry"). Widgets have heterogeneous signatures and
      there's no uniform draw-fn contract yet, and the right shape is hard to know
      without a concrete modding consumer driving the requirements — so design it
      against a real `register_widget` call site (the Lua binding layer) rather
      than in the abstract. Re-tiered from P1: not needed for 1.0 usability.
- [ ] **Out of scope but don't block:** depth-aware `DrawSprite`/`DrawLine`
      in 3D world space — keep UI overlay vs. world overlay passes
      separable.

---

## 2026-09-24 — Found during Forge design-token audit

- [ ] **P2 — Status-bar latched inset is one shadow.**
  `StatusBarChrome::latched_inset` holds only `--key-inset-latched`'s first
  layer (`inset 0 2px 4px latch-shade`); the `inset 0 1px 0 latch-hi` line
  needs a second slot. Nothing paints it yet (no latched status toggle).
- [ ] **P1 — Perf: `frame_render` is ~140× slower than its June baseline.**
  `cargo bench --bench ui_stress -- frame_render` on the RX 7900 XTX: 100
  buttons ≈ 1.16 ms, 1000 ≈ 30 ms, 10k ≈ 950 ms (criterion's stored June
  baseline had 1000 at ~0.2 ms). Reproduced identically on a clean copy of
  commit `719dc88`, so it predates the sRGB-pipeline change; `target_path`
  shows 1000 buttons at ~47 ms/frame *including* GPU wait on both target
  paths, so it is likely GPU-bound (button chrome/shadow fill?). Bisect
  between the June baseline and `719dc88`.

## 2026-09-25 — Sidebar pieces (Forge `components/sidebar`, for agent-ui)

- [ ] **P3 — SearchField scopes.** The design's latching scope key inside the
  well (and its sheet of scopes) is not built.
- [ ] **P3 — `draw_well` on box shadows.** The square well still paints its
  inner shadow as a gradient band and its under-line as a quad; moving it to
  `draw_well_rounded`'s box shadows would give one well recipe. Changes every
  input's pixels, so do it with a gallery review.
- [ ] **P3 — Ghost tone colours vs Forge.** The design's ghost key is a flat
  `rgba(255,255,255,.08)` on hover (.04 pressed), a `.1` highlight on hover
  and no highlight when pressed; gameui's ghost uses the raised key's face
  gradient and highlight tokens.
- [ ] **P3 — ListView disabled reason.** The design shows why a row is
  disabled in a tooltip; `ListRow` has no reason yet.
- [ ] **P3 — No Plex Medium.** Only Regular/Bold/Italic are bundled, and
  cosmic-text falls back to another family for weight 500, so the design's
  medium-weight labels are drawn Regular. Bundle Medium if it is wanted.
- [ ] **P3 — Table row colours.** Table's zebra (`Panel` × 1.1) and hover
  (`ButtonHover`) predate the row tokens; move them to `RowZebra` /
  `RowHover` with a gallery review.
- [x] **P2 — Ellipsis shapes the full text twice.** Done 2026-10-22 (Phase 3
  of `docs/design/shared-text-layout.md`): the cut is read from the cached
  unwrapped layout the measure uses, and a right-to-left label is cut to its
  start (it kept its end and overflowed). It was: on a miss, an ellipsized
  block's natural width is shaped by the measurer, and `ellipsis_cut` shapes
  the full content again to find the cut.
- [x] **P2 — Caret, cursor and visual layouts shape on their own.** Done
  2026-10-21 (Phase 1 of `docs/design/shared-text-layout.md`): carets and
  visual glyphs are read from the cached layout a block is drawn with, and
  the multi-line IME caret is on the composition's line. It was:
  `text_caret_layout`, `text_cursor_positions` and `text_visual_layout` build
  their own `Buffer` every call rather than using the shared layouts; a
  large text input reshapes its whole content for each of them. Planned in
  `docs/design/shared-text-layout.md`, with a bug found on the way: a
  multi-line field composing with the IME puts its caret on the first line.
  (The other, line starts found by `\n` only, is fixed: `text::line_starts`.)
- [ ] **P3 — No caret affinity at a direction boundary.** Where
  left-to-right and right-to-left text meet, one byte has two caret stops
  (the end of one run, the start of the other), and `visual_caret_pos`
  always draws the leftmost. A click on the other one (found by Codex,
  2026-10-21: `"abגד"`, a click near x=40 picks byte 2 and the caret is
  drawn at x=20) puts the right byte in the field but the caret away from
  the pointer, as arrowing onto it does. The field would keep which stop it
  stands at with its byte, set by the click or the arrow that put it there.
- [x] **P2 — Text places glyphs outside its clip.** Done 2026-10-21:
  `text_prep::glyphs_in_clip` finds the lines that can reach into a block's
  clip and `append_placements` skips glyphs that can't; the focused
  5,000-line field's idle frame went from 39 ms to 1.3 ms. It was:
  `text::append_placements`
  places, orders and uploads a quad for every glyph of a block and leaves
  the clip to the shader, so a focused 5,000-line field spends 40 ms a frame
  on ~250K glyphs to show a screenful (sampled 2026-10-21, see "Phase 1
  numbers" in `docs/design/shared-text-layout.md`). Skip the lines wholly
  outside the clip (lines are sorted by `top`, so a binary search finds the
  first in view).
- [ ] **P3 — A glyph uploads six fat vertices.** A sample of the focused
  5,000-line field's idle frame (2026-10-21, with the clip culling in) has
  `Queue::write_buffer` copying about a sixth of the frame's samples. It
  uploads only what the frame draws (`RenderStats::buffer_bytes_uploaded`:
  about 1 MB at 500 lines and at 5,000, for a screenful), but each glyph is
  six `MsdfVertex`es of 88 bytes, of which only position and uv differ: 528
  bytes a glyph. One instance a glyph (its rect, uv rect, fill, clip,
  outline) would be about a sixth of that, and a sixth of the vertex
  building in `append_placements`. Changes the MSDF pipeline and shader;
  for Bart to decide.
- [ ] **P3 — Variable font weights.** The atlas keys glyphs by
  `(font id, glyph id)` and rasterises from the font file, so a variable font
  draws every weight at its default instance. Carry the weight into the
  atlas key and apply the variation when one is needed.
- [ ] **P3 — Docked scrollbar gutter colours.** The docked gutter and step
  keys still use the older scrollbar keys, not the Forge `ScrollArea`
  values; move them into `ScrollbarChrome` with a gallery review.
- [ ] **P3 — Key tooltips.** The design's `IconKey`s carry a tooltip (title
  plus shortcut); gameui's keys take none, so DockStack actions and the
  search clear key show nothing on hover.
- [ ] **P3 — Tree action hover literal.** `Tree::draw_action`'s hover
  overlay is a hard-coded `[1, 1, 1, 0.12]`; it should be a token.
- [ ] **P3 — Tree disabled reason.** Like ListView, a disabled node has no
  tooltip saying why.
- [ ] **P3 — Keyboard expand of a disabled branch.** A disabled branch
  expands by pointer, but the arrow keys skip it, so the keyboard can't
  reach its children.
- [ ] **P3 — `block` 0.1.6 future-incompat warning.** Every build warns
  that `block` (pulled in through the Metal backend) will be rejected by a
  future Rust; it goes away with a wgpu upgrade.

## 2026-09-26 — Pressable (Forge `Key`) replaces ImageButton

- [ ] **P2 — Disabled keys fade per element, not as a group.** A disabled
  `Pressable` fades its material and its content separately (each by
  `DISABLED_ALPHA`). Forge fades the whole key with CSS `opacity`, which
  keeps the ink-to-face contrast. Per-element fading loses it where the ink
  is darker than the face: a disabled accent `Button` or `IconKey` label is
  nearly invisible (gallery `keys/Button.png`, "Button (accent,
  disabled)"). Kept on purpose for now (Bart, 2026-09-26). The fix is real
  group opacity: draw a disabled key into an offscreen layer and blend it at
  `DISABLED_ALPHA`, if the renderer's layer/backdrop path can carry it.

## 2026-09-26 — Missing Forge components, batch A (small pieces)

- [ ] **P3 — Outset lips count as overflow in the debug report.** Wells
  with Forge's lit lower lip (`0 1px 0 rgba(255,255,255,.07)` outside the
  box: `SearchField`, `Placeholder::image`) paint 1 px below their
  declared rect, and `overflows_declared` flags each one. Either the report
  learns that a 1 px outset lip is intended, or those widgets declare the
  lip in their scope rect.
- [ ] **P3 — `cargo doc` has 28 warnings again.** It was clean once (see
  "Rustdoc on all public types"). Unresolved links (`Frame::run`,
  `UiState::end_frame`, `TooltipLayer::hover_zone`, `UiFrameResult`, …)
  and links to private items, such as `Thumb`'s "See the module docs",
  which points into a private module whose docs users never see.
- [ ] **P3 — Our `Group` isn't Forge's `Group`.** Forge's is a *sunken*,
  collapsible card (`rgba(0,0,0,.22)`, mono-caps header with ▸ caret and a
  right-aligned summary). Ours is a raised panel with a sans title strip.

## 2026-09-26 — Missing Forge components, batch B (dialogs)

- [ ] **P3 — `TextInput` has one font.** Its caret layout assumes the
  default font, so `PromptDialog` can't offer Forge's `mono` field.
- [ ] **P3 — Dialog backdrop blur is up to the app.** Forge blurs behind the
  backdrop (3 px) and the sheet (18 px). `DrawList` can't sample the
  framebuffer, so `Modal` only exposes `MODAL_BACKDROP_BLUR` /
  `SHEET_BLUR` for the app's `UiRenderer::blur_backdrop` pass.

## 2026-09-27 — Missing Forge components, batch D (inspector)

- [ ] **P2 — Mixed Slider and Checkbox.** Forge's Slider and Checkbox have
  a `mixed` look for multi-selection (the inspector card's Rough and
  Shadow rows). Ours don't yet, so a three-object Inspector shows the
  first object's value in those rows. (Bart, 2026-09-26: a TODO, not
  batch D.)
- [ ] **P3 — Ellipsis has no slack.** `text::ellipsis_cut` cuts when the
  text is wider than its box by any amount, so a box sized to measured
  text can lose its last glyph to float rounding (the PropertyGroup bug
  above; `SpanTabs` pads its box by 0.5 px for the same reason). A small
  tolerance there would make every caller safe.

## SDF lines follow-ups (found 2026-10-08)

- [ ] **P3 — Chrome's edge ramp may match Chromium better as strokes' does.**
  Strokes ramp coverage over one screen pixel along the edge's normal
  (`stroke_coverage` in `ui.wgsl`), which roughly halved the mean difference
  from Chromium on diagonal strokes compared with chrome's `edge_coverage`
  (an L1 pixel footprint). Rotated chrome might gain the same; measure with
  `tests/analytic_shadow_browser_parity.rs` (the `gpu-chrome-shadows`
  captures) before switching.
- [ ] **P3 — Self-overlapping strokes paint their soft edge twice.** A segment
  only shares pixels out with its two neighbours, so a translucent line that
  folds back over itself (a dense chart line) is slightly darker along the
  fold. See the known limitation in `docs/design/sdf-lines.md`.
- [ ] **Arcs (sdf-lines Phase 4).** `stroked_arc` and transformed
  `circle_outline` still tessellate; waiting on that doc's open question 6
  (a ring with start and end angles, or an analytic arc kind with the full
  affine, which could take circles in too; see "Rotated quads and circles
  still tessellate" above).

## Learned from gpui (compared with ~/Projects/gpuix, 2026-10-08)

- [ ] **P2 — Cost of boxing soup per triangle.** Batching's draw boxes
      (`render/draw_box.rs`) bound a soup command by visiting every triangle
      and its three clips. A 10k-point polyline drawn as soup pays that each
      frame. Measure it with the Phase 6 bench (`docs/design/draw-call-batching.md`)
      and, if it shows, cache the bound with the command or bound the
      vertices in one pass. Measured 2026-10-16 (`batching/soup_polygon`, a
      busy machine): about 16 ns per triangle, 0.44 ms against 0.28 ms in
      paint order for one 10k-triangle command. Bart to decide whether that
      is worth the cache.
- [x] **P2 — Skip ordering a list of one key.** Done 2026-10-21: units
      of one key stay out of the grid until a second key arrives; the
      polyline encodes in 0.75 ms (was 1.11, paint order 0.48), see the
      note in `docs/design/draw-call-batching.md`. It was: When every unit of a list
      has the same batch key, ordering can't change anything: the units
      draw as one batch, in paint order. The plan still boxes
      and orders each unit, which costs a 10k-segment polyline about 0.6 ms
      to encode over paint order (`batching/polyline`, 2026-10-16). The
      boxes are still needed to cull; the grid isn't.
- [x] **P1 — `List` with measured row heights and an item-anchored scroll
      position.** Done (739f9bf; agent-ui ed19404), see
      `docs/design/measured-row-heights.md`. It was: `List` only handles fixed row heights. gpui's list keeps
      its position as "item N, this far into it" instead of a pixel offset,
      so when rows above the view change height nothing moves and nothing
      needs correcting. agent-ui's transcript already measures rows, but
      keeps a pixel offset and corrects it on every change (`hold` /
      `reading` in `crates/desktop/src/transcript_pane.rs`). Moving that
      measuring into `List`, with gpui's anchor, would serve both.
- [ ] **P3 — `GroupList` onto `RowHeights`.** `GroupLayout` keeps its own
      running total of row heights (headers and rows differ). Once `List`
      draws from a `RowHeights` (`docs/design/measured-row-heights.md`),
      `GroupList` could share it and its item-anchored scrolling.
- [x] **P2 — One text layout for drawing, caret and hit testing.** gpui
      shapes a string once and uses that layout to draw it, place the caret
      and map a click to a character. gameui's text widgets shape separately
      for each. gpui also caches per line, so typing in a long field
      reshapes only the edited line. (Related: "Ellipsis shapes the full
      text twice" and "A `TextBlock` costs 3 heap allocations" above; gpui
      keeps short strings inline and shares long ones, which is its answer
      to the second.) 2026-10-21: one layout for all of them is done (Phase 1
      of `docs/design/shared-text-layout.md`); caching per line is its
      Phase 2, for Bart to decide on the numbers there. 2026-10-21: Phase 2
      is done as paragraphs taken from the previous layout rather than kept
      one by one; a 5,000-line keystroke shapes the paragraph typed in.
- [ ] **P2 — Upload only the changed part of the glyph atlas.**
      `text.rs` `write_pixels` rebuilds the whole atlas pixel buffer
      (`build_pixel_buffer`) and writes the full texture whenever any glyph
      was added. gpui writes just the new glyph's rectangle. Track a dirty
      rectangle (or per-glyph rects) in `MsdfGlyphAtlas` instead of a flag.
- [ ] **P3 — Sideways scrolling, locked to one axis per gesture.**
      `InputState::scroll_delta_x` is filled in, but no gameui widget reads
      it: `ScrollView` and `List` scroll only vertically by wheel (agent-ui
      reads it itself, for code blocks in `transcript_pane.rs`). gpui
      scrolls both axes and locks a trackpad gesture to its dominant axis,
      so a slightly diagonal swipe doesn't drift sideways.
- [ ] **P3 — Spring animations.** When a target changes mid-flight,
      `AnimationState` starts the new tween from the current value (no
      jump), but the speed restarts along the easing curve, so a reversing
      panel visibly stops and starts again. gpui has springs
      (`gpui/src/spring.rs`, `SpringConfig { stiffness, damping, mass }`, a
      damped oscillator) that keep their velocity when retargeted.
- [ ] **P3 — Snap layout to whole device pixels.** gpui has
      `Window::pixel_snap` / `round_to_device_pixel` (and a stroke variant)
      for rounding positions and sizes to device pixels, which avoids 1 px
      seams and blurry edges between neighbours at fractional scale
      factors. gameui snaps glyphs (`text.rs`) and strokes (`stroke.rs`) to
      device pixels, but not layout boxes.
- [ ] **P3 — A shortcut table with contexts.** gpui binds keys to actions
      in a table scoped by context (a text field, a list, a dialog), and
      resolves "a shortcut wins if one matches, otherwise the key types
      text". gameui leaves shortcuts to each app, and agent-ui writes them
      out by hand (see its AltGr/Ctrl typing TODO, which this would fix).
- [ ] **P3 — Full IME support.** `FocusState` already asks for IME and
      reports a caret rect (`take_ime_request`), but agent-ui never reads
      it or handles `WindowEvent::Ime`, so the macOS candidate window and
      accent menu don't work. Needs preedit text drawn in the field and the
      committed text inserted, as gpui's input handler does.
- [ ] **P3 — Drag and drop that carries data.** gameui tracks a drag, but
      nothing carries a payload to a drop target. gpui attaches typed data
      to a drag (`on_drag`) and lets elements accept it (`on_drop::<T>`).
- [ ] **P3 — Accessibility through AccessKit.** gpui exposes its element
      tree to screen readers; gameui exposes nothing. Ties in with keyed
      layers ("stable, reusable keyed layers" above), since both need
      stable widget ids across frames.
- [ ] **P3 — Images loaded in the background, and vector paths.** gpui
      decodes images off the main thread and draws arbitrary paths with
      smooth edges. gameui's new `Stroke` covers lines; filled paths
      remain.

Already on this list, with gpui's answer: "Reuse interaction dispatch
scratch" (gpui walks regions in reverse paint order, no sort or
allocation) and "Stable, reusable keyed layers" (gpui reuses unchanged
parts of the previous frame). Where gameui is ahead of gpui and nothing
needs learning: scroll easing, backdrop blur, the debug report, MSDF text
sharp at any size, list selection and gamepad navigation.

## agent-ui follow-up from the layer blockers (2026-10-08)

- [ ] **P3 — Popovers could use `UiState::block_under_layer`.** agent-ui's
      `usage_popovers.rs`, `task_popover.rs` and `drop_up.rs` (the status
      bar's frame and connection details) push a screen-sized modal layer
      and register a hand-made enabled backdrop on it
      (`cx.interact(BACKDROP_ID, screen, true)`) so a click outside closes
      them. They already block clicks correctly. Moving them to
      `block_under_layer` would also mark them as the top modal, confining
      Tab inside the popover (and doing nothing when it has no focusables)
      — a behaviour change for Bart to decide on, so they were left as
      they are. The outside-click-closes behaviour would still need its own
      check, since the blocker is disabled and reports no click.

## Timeline and viewport gizmos follow-ups (found 2026-10-11)

- [ ] **P2 — HUD glass has no backdrop blur.** Forge's `hudGlass` blurs the
      scene behind every HUD bar, readout and stats panel. Bart (2026-10-11):
      no blur for now. It needs a draw-list backdrop primitive, which would
      sample the frame behind a rect at its place in the draw order. That is
      the same gap as "Dialog backdrop blur is up to the app" above, and one
      primitive would close both. Until then `hud_glass` is a darker, more
      opaque fill (`ChromeTheme::hud`).
- [ ] **P2 — The timeline's marker rename field uses the theme's field
      font.** Forge edits a marker's name in place at the flag's 8.5 px mono
      size. `TextInput` takes its font and size from the theme
      (`StyleKey::FontSize`), so the field is a normal 19 px field. Giving
      `TextInput` a font and size override would fix it; the same override
      would suit other compact in-place editors.
- [ ] **P3 — Nearby marker labels overlap.** When the timeline is zoomed out,
      marker flags a few frames apart draw their labels over each other.
      Forge does the same. They could be staggered onto a second line, or
      shortened to the flag with a tooltip.
- [ ] **P2 — Forge's `graph` components are neither built nor counted.**
      `components/graph` has GraphNode, GraphPort, GraphWire, NodeCanvas
      and NodeSearch (the node-editor kit). The gallery's
      `FORGE_COMPONENTS` doesn't list them, so the index's "85 of 91"
      doesn't count them as missing. Add a `Graph` category with the five
      names, and build them when an editor needs them.
- [ ] **P2 — A `LayerStack::new()` scans the system fonts for every layer it
      pushes.** Without a shared font system, `make_list` gives each layer a
      `DrawList::new()`, whose `TextMeasurer::default()` builds a
      `FontSystem` of its own: about 0.15 ms a layer on Bart's Mac, every
      frame (seen in the `layer_dispatch` bench before it shared one). Hosts
      with a renderer pass `with_font_system`; tests and font-less hosts pay
      it. Share one lazily made system across the stack's layers, or make
      `new()` take the process-wide `shared_font_system()`.
