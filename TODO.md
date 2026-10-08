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

- [ ] **P2 — Stem darkening / coverage gamma.** Vello lists its absence beside
      hinting and subpixel AA as the reason its text looks weak on low-DPI
      displays, and DirectWrite exposes the same thing as "text contrast".
      Light-on-dark text optically thins; a gamma applied to coverage in
      `ui_msdf.wgsl` compensates. Cheap to try. Note it makes text *denser*, not
      sharper — it cannot merge a stem that spans two pixels, so it is a
      complement to grid fitting rather than a substitute.

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
- [ ] **P2 — `hello_ui` example: overlapping layout + per-frame `LayerStack`.**
  The text-input/dropdown row sits under the demo panel and the gradient
  swatches overlap the "Custom font" / "Click A/B" rows; it also trips the
  renderer's "freshly-constructed `LayerStack` for 120+ frames" warning (it
  should build one stack and `.clear()` it). Both reproduce on `719dc88`.

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
- [ ] **P2 — Ellipsis shapes the full text twice.** On a miss, an ellipsized
  block's natural width is shaped by the measurer, and `ellipsis_cut` shapes
  the full content again to find the cut. It could read the width and the
  glyph boundaries from the cached unellipsized layout.
- [ ] **P2 — Caret, cursor and visual layouts shape on their own.**
  `text_caret_layout`, `text_cursor_positions` and `text_visual_layout` build
  their own `Buffer` every call rather than using the shared layouts; a
  large text input reshapes its whole content for each of them.
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
