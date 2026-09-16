# wgpu-gameui TODO

Consolidated from two independent audits (Claude + Codex) on 2026-04-26 of the
~2,630 LOC source tree just extracted from citybuilder. Both audits agreed on
the same major gaps. Items grouped by category and tagged with priority:

- **P0** — blocks 1.0 / blocks Teardown-API port
- **P1** — important, but the lib is usable without it
- **P2** — nice-to-have

Use this as the working backlog for the package. Cross items off as PRs land.

---

## Cross-notifier follow-up audit (2026-08-19)

These items came from regressions found while migrating cross-notifier to gameui.
The crate already has `SizeSpec::Fit`, constraints, reusable `LayoutResult`,
font-aware `TextBlock` measurement, `LayerStack`, animation clock plumbing,
`ScrollView`, and extensive debug reports. The work below should integrate and
harden those foundations rather than create parallel replacements.

### P0 — Start here

- [x] **P0 — Make paint and input z-order first-class and shared.** A `DrawList`
      is rendered in primitive pass order (nine-slices → colour → icons → MSDF
      icons → text), not arbitrary painter's order. `LayerStack` correctly orders
      whole lists, but widgets within one list cannot express cross-primitive
      overlap safely. Introduce an explicit ordered command/node model with a
      stable layer ID, z-index/pass group, and insertion sequence. Render it
      back-to-front and dispatch input through the same ordering topmost-first.
      Give popups/tooltips/menus explicit priorities rather than relying only on
      layer push order. Add regressions for overlapping quad/text/icon widgets,
      multiple popups, and input targeting the visually topmost widget.

- [x] **P0 — Unify painted geometry and hit geometry.** Widgets currently paint
      a `Rect` and independently perform `rect.contains`, while more complex
      widgets separately derive popup, row, and child hit regions. Emit a stable
      `InteractionNode`/`HitRegion` alongside paint commands, carrying widget ID,
      final bounds or shape, inherited clip/transform, z/layer, enabled/capture
      policy, and cursor. Centralize topmost-first hit traversal and return a
      common `Response { rect, hovered, pressed, clicked, focused, ... }` from
      interactive widgets. The exact arranged geometry must drive both painting
      and interaction. Test clipped, transformed, scrolled, animated, and
      overlapping controls, and expose the candidate/winner chain in diagnostics.

- [x] **P0 — Integrate interactive `UiContext` widgets with the existing layout
      system.** Added binding-neutral `StackChild` declarations for existing
      `HStack`/`VStack` fixed/fit/fill/percent sizing, constraints, weights,
      cross-axis alignment, and stable `NodeId`; `LayoutResult::child_items()`
      exposes identity with geometry. `UiContext::rect_begin`/`rect_end` and
      `draw_in_rect` draw each child exactly once in its resolved local rect with
      no callback replay. Cross-notifier's server settings row is the first real
      migration fixture and no longer maintains manual x coordinates or guessed
      button padding. Baseline measurement remains part of contextual measurement
      work below rather than this replay-free P0.

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

- [ ] **P1 — Return repaint requirements and deadlines from a UI frame.** Build
      on the existing animation clock with a host-facing
      `UiFrameResult { changed, needs_repaint, next_deadline }` that aggregates
      widget transitions, toast/tooltip timing, caret blinking, and externally
      registered animation. Clamp invalid or very large delta times. Event-driven
      hosts must be able to schedule another frame without relying on incidental
      mouse movement.

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

## Architecture / Core Plumbing

- [x] **P0 — Public `UiRenderer`/`Backend`** that owns the wgpu pipeline, sampler,
      atlas, and consumes a `DrawList` per frame. (`src/render/ui_renderer.rs`)
- [x] **P0 — Texture atlas / sprite registry.** Dynamic shelf packer with grow
      (`src/render/atlas.rs`), `load_sprite_rgba8` + `register_nine_slice` on
      `UiRenderer`. `IconDraw`/`NineSliceDraw` now carry pre-resolved
      `SpriteId`/`NineSliceId` with name fallback.
- [x] **P0 — Matrix / transform stack** (`UiPush`/`UiPop`/`UiTranslate`/`UiAlign`/
      `UiCenter`/`UiRotate`/`UiScale`). 2x3 affine stack lives on `DrawList`
      so existing widgets that take absolute `Rect`s pick it up transparently;
      `UiContext` (`src/ui_context.rs`) is the Teardown-verb façade.
- [x] **P0 — Color / tint stack** (`UiColor`/`UiColorFilter`) with sub-tree alpha
      multiplier. Lives alongside the transform stack on `DrawList`; primitive
      methods multiply input color by current tint at push time.
- [x] **P0 — Clip / scissor stack.** `push_clip(rect)`/`pop_clip()` with draw
      commands grouped per clip stack. `Table::draw_cell` text is currently
      *not actually clipped* by `content_rect`.
- [x] **P1 — Unify widget API around `DrawContext` + `Rect`.** `Dropdown::draw`
      and `Button::draw`/`draw_at`/`draw_nine_slice` now take `&mut DrawContext`
      instead of individual `(&mut DrawList, &Theme, &InputState)` params.
      `DrawContext::register_focus(id)` auto-scopes to the active layer.
      Checkbox, Slider, and TextInput now take `&mut DrawContext` too. Remaining
      non-interactive widgets (Tabs, ProgressBar, ScrollView, Table, Tooltip,
      ImageButton) still take individual params — deferred to follow-up passes.
- [x] **P1 — Replace `String` keys in draw commands with interned `IconId`/
      `SpriteId`/`u32` handles** produced by the atlas. (Both still accept
      string-keyed helpers for ergonomics; `icon_sprite`/`nine_slice_id` are the
      allocation-free path.)
- [x] **P1 — Don't `unwrap()` glyphon errors in `TextRenderer::prepare`/
      `render`** (`src/text.rs:103,130`). ~~Bubble as a typed `UiError`.~~
      Obsoleted by the MSDF rewrite (Phase 1-3): glyphon's fallible GPU
      `prepare`/`render` stage was replaced by the custom `MsdfGlyphAtlas` path.
      There is no `fn prepare` anymore and `render` is infallible — a crate-wide
      grep finds zero uses of glyphon's GPU error API. The only remaining
      `expect`s in `text.rs` are `FontSystem` lock-poison guards (another thread
      panicked holding the lock = unrecoverable; propagating the panic is the
      correct idiom, not a recoverable `UiError`).
- [x] **P1 — Cache glyphon `Buffer`s by content+size hash or pool them.**
      Done better than pooling buffers: `TextRenderer::build_vertices` caches the
      *shaped glyph layout* (relative positions) keyed by
      `(content, font_size, line_height, max_width, family, align, ellipsize,
      weight, style)`. A hit skips `Buffer::new`/`set_text`/`shape_until_scroll`
      and takes no `FontSystem` lock (the MSDF atlas never evicts, so cached
      glyphs are always present). Working-set eviction past `SHAPE_CACHE_MAX`
      (8192); `clear_shape_cache()` for font hot-loads. (`src/text.rs`)

---

## Draw Primitives

- [x] **P0 — Rounded rectangles.** `theme.border_radius` exists but is never
      used. Teardown's `UiRoundedRect` is widely used. Tessellate or SDF.
- [x] **P0 — Lines / strokes** (`line(p0, p1, thickness, color)`) with
      thickness/joins/caps/AA. Teardown's `DrawLine` is top-30. Also needed
      for slider tick marks, debug overlays.
- [x] **P1 — Circles / arcs / ellipses** (`DrawList::circle`, `circle_outline`, `stroked_arc`; `UiContext::circle`, `circle_outline`).
- [x] **P1 — Textured quad with explicit UV rect.** Atlas `AtlasRegion::uv()`
      drives icon UVs, with optional tint per draw.
- [x] **P1 — Nine-slice border metadata.** `register_nine_slice(name, sprite,
      border)` on `UiRenderer` records source rect (via SpriteId) + per-side
      borders, with tint per draw.
- [x] **P1 — Vector icon library (Phosphor, MSDF).** Curated `PhosphorIcon`
      enum (regular/line weight, MIT TTF vendored under `assets/fonts/phosphor/`)
      rendered through a dedicated MSDF icon atlas that reuses the text MSDF
      generator/pipeline (extracted `MsdfTextureGpu` GPU mirror; icon atlas keyed
      `PHOSPHOR_FONT_ID`, `ref_px = 64`). API: `DrawList::icon_msdf(rect, icon,
      tint)` (fit-centred into the rect via `fit_centered`, transform/tint/clip
      aware, rotation-capable) and the stateless `Icon::new(PhosphorIcon).tint(..)
      .draw(rect, list)` widget. Adopted by the `NumberInput` `+`/`−` steppers.
      Behind the default-on `phosphor-icons` feature; widgets fall back to text
      glyphs when it's off.
- [x] **P2 — Gradient helpers** (linear/radial). `DrawList` constructors over
      the per-vertex color path: `linear_gradient(rect, start, end, angle)`
      (exact at any angle — a linear ramp is affine, so the four corner colors
      are projected onto the gradient axis and bilinear interpolation reproduces
      it), plus cheaper `horizontal_gradient(rect, left, right)` /
      `vertical_gradient(rect, top, bottom)`. `radial_gradient(rect, inner,
      outer, segments)` draws a triangle fan (≥3 wedges) whose radius reaches the
      farthest corner and is clipped to the rect, so the whole rect fills;
      transform/tint/clip-aware like the other soup primitives.
- [x] **P2 — Text outline / shadow** (`UiTextOutline`, `UiTextShadow`). Already
      shipped via the MSDF text path: `TextBlock::with_outline(r,g,b,a,width_px)`,
      `with_shadow(r,g,b,a,dx,dy,softness)`, and a bonus `with_glow(r,g,b,a,
      radius_px)`; backed by `TextOutline`/`TextShadow`/`TextGlow` (all exported).
      Rendered by the back-to-front glyph sweep in `src/text.rs` (not feature-
      gated); gallery has Outline/Shadow/Glow demo rows.

---

## Widgets

- [x] **P0 — Dropdown / combo / select.** `Dropdown<'a>` + caller-owned
      single-owner `DropdownState`/`DropdownId` (one open at a time, like
      `FocusState`). Button drawn inline; the open list floats in a `Popup`
      layer pushed at frame-top (blocks clicks underneath). Click/Esc/
      click-outside close, hover highlight, selected highlight, scroll-clipped
      past `max_visible`. `src/widgets/dropdown.rs`. Keyboard nav + Tab-focus
      deferred (P1 below).
- [x] **P1 — Dropdown keyboard nav.** Arrow Up/Down + Enter to select, and
      register the dropdown in `FocusState` so Tab reaches it (open with
      Space/Enter). Same-frame open on keyboard activation (geom set immediately
      so the list appears without a one-frame delay). `Dropdown::draw` takes
      `&mut DrawContext` (which bundles `DrawList` + `FocusState` + `Theme` +
      `InputState` + screen dimensions). `key_up`/`key_down`/`key_space` added
      to `InputState`.
- [x] **P0 — ScrollView / scroll container** (general — `ScrollView` widget
      with caller-owned `ScrollState`, vertical+horizontal scroll, wheel
      input, draggable thumb, lives in `src/widgets/scroll_view.rs`. `Table`
      now uses it).
- [x] **P0 — Modal / dialog / popup layer** with z-order stacking and input
      gobbling. `LayerStack` in `src/layer.rs` plus
      `UiContext::modal_begin`/`modal_end`/`popup_begin`/`popup_end`.
      `InputState::mouse_consumed` tracks layer-dispatch capture.
- [x] **P0 — Popup / portal layer** for dropdowns, context menus, tooltips.
      `LayerStack::push_popup`/`push_tooltip`. Tooltip refactored to render
      onto its own layer via `TooltipLayer::draw_into_layers`.
- [x] **P0 — Image / sprite widget** with sizing/aspect/tinting/UV-rect.
      `Image` (`src/widgets/image.rs`) draws a `SpriteId` or string key into a
      dest box with `ImageFit` (Stretch/Contain/Cover/ScaleDown/None),
      `ImageAlign`, tint, and automatic UV cropping for `Cover` (via
      `DrawList::image_cropped`). Natural size supplied by the caller (from
      `UiRenderer::image_size`); aspect fits fall back to Stretch without it.
- [x] **P1 — Image / icon button.** `ImageButton`
      (`src/widgets/image_button.rs`) layers the `Image` widget (full
      `ImageFit`/`ImageAlign`/tint) over `Button`-style chrome with
      hover/press/disabled feedback, returning a click bool like `Button`.
      `.bare()` drops the chrome (image is the hit target, overlay-only
      feedback); `.padding()` insets the image. Disabled dims via overlay so
      string-key sources without tint still read as disabled.
- [x] **P1 — Radio button group.** `RadioGroup<'a>`
      (`src/widgets/radio.rs`) draws a mutually-exclusive option set from vector
      primitives (dot = `input_background` fill + `input_border` ring; selected
      adds an `accent` inner dot — no atlas assets needed). Caller owns the
      selected index: `draw(selected, rect, ctx) -> Option<usize>` returns the
      new index on change (click or, while focused, arrow keys). Builders:
      `new(&[&str])`, `.focusable(FocusId)` (one Tab stop for the whole group +
      arrow nav: Up/Down vertical, Left/Right horizontal, clamped no-wrap),
      `.horizontal()` (cells sized to measured label width), `.spacing(px)`.
      Façade verb `UiContext::radio_group(options, selected) -> usize`
      auto-places/advances like `checkbox`. `measure(list, style) -> (w, h)`
      reports the extent the options need (dot diameter × gap × shaped label
      widths, per orientation) — a caller cannot derive it, and layout
      inspection caught the gallery's hand-guessed height cutting off a row.
- [x] **P1 — Tree view / collapsing header.** `TreeNode`
      (`src/widgets/tree.rs`) draws one row — a disclosure triangle + indented
      label for *branches*, a terminal *leaf* otherwise — against a `Rect`/
      `DrawContext`. **Action-icon slots** (`with_leading`/`with_trailing` taking
      `&[TreeAction]`, sprite or string-key via `TreeIcon`) give the
      scene/layer-outliner shape: a leading visibility toggle + right-aligned
      rename/delete, each its own hit target — clicking one returns its
      `TreeAction::id` via `TreeNodeOutput::action` and does *not* select/expand.
      Interaction: the disclosure triangle toggles; the label/body selects (and
      *also* toggles only with `with_toggle_on_label`); actions fire alone.
      Caller-owned `TreeState` holds the expanded set + single-owner selection
      (`select`/`is_selected`/`toggle`/`set_expanded`/`collapse_all`);
      `with_default_open` expands a node the first time its `TreeId` is seen.
      Highlight spans the full row width; honors `mouse_consumed`. Façade
      `UiContext::tree_row(id, TreeNode)` → `TreeNodeOutput` is the rich verb;
      `tree_node`/`tree_node_open` (whole-row toggle) + `tree_leaf` are the
      no-icon convenience path, all with automatic per-depth indentation +
      auto-advance and `tree_pop`, backed by `UiState::tree`. Keyboard arrow-nav
      landed with the keyboard-navigation P1 (see below): `TreeState` nav-ring +
      `begin_frame`/`register_nav`/`end_frame(focused)`.
- [x] **P1 — Number input / spin box** with validation. `NumberInput`
      (`src/widgets/number_input.rs`) wraps a `TextInput` (inheriting cursor /
      selection / clipboard) around an `f64` value: parses + clamps to
      `[min, max]`, with +/- step buttons, mouse-wheel stepping, and Up/Down
      arrow stepping (both gated on focus so they never hijack page scroll or
      collide with text editing — single-line `TextInput` ignores Up/Down).
      `decimals` sets precision (`0` = integer). Text is sanitised to numeric
      characters while editing; the value owns the text when unfocused (external
      changes win), and `Enter` canonicalises. Façade
      `UiContext::number_input(id, &mut f64, min, max, step, decimals, w)`.
- [x] **P1 — Drag handle / window-mover.** `DragHandle`
      (`src/widgets/drag_handle.rs`) is a draggable grab-zone drawn against a
      `Rect`/`DrawContext`. It claims a caller-owned `DragCapture` (keyed by a
      stable `DragId`) on press so it can't fight sliders/scroll-thumbs/adjacent
      handles, and reports the per-frame pointer movement as
      `DragHandleOutput { dragging, started, released, delta }`. The delta is
      *consumed from* `InputState::drag_delta` — i.e. it composes with a
      caller-owned `DragTracker` (the "uses both" pattern the drag modules
      describe): run `DragTracker::update` each frame, then thread one shared
      `DragCapture` into every handle. `DragHandle::new()` draws title-bar chrome
      (panel background + hover/drag highlight + a centred grip glyph or a
      left-aligned `with_label`); `bare()` is a chrome-less pure hit-zone.
      `drag_rect(id, cap, handle_rect, &mut target, ctx)` is the convenience that
      applies the delta straight to a target `Rect`. Honors `mouse_consumed`.
      (No `UiContext` flow verb: a free-moving window is absolute-positioned, so
      it doesn't fit the auto-advance layout flow — use the widget directly with
      `UiState::drag` as the shared capture.)
- [x] **P1 — List / grid / virtualized list.** `List` (`src/widgets/list.rs`)
      is a general virtualized iterator over a flat `count` of items: it sets
      `ScrollState::content_size` and drives `ScrollView` (vertical), culling to
      the visible index range so a 10k-item collection costs a screenful. `List`
      owns no data — the caller passes `count` and a content closure
      `FnMut(&mut DrawList, Rect, ListItem)`; the widget draws the row chrome
      (selection/hover/zebra) and handles interaction, the closure fills the
      cell. Single column is a list; `.columns(n)` packs an `n`-wide grid
      (left-to-right, top-to-bottom) with `.with_gap(col, row)`. Selection is
      caller-owned in `ListState` (scroll + selected set + keyboard cursor):
      `SelectionMode::{None, Single, Multi}` — plain click replaces, Ctrl-click
      toggles, Shift-click selects the inclusive range from the anchor. With
      `.focused(true)`, arrow keys move the cursor (Up/Down by a column, Left/
      Right within a grid row), Home/End jump, Enter/Space activate, and the
      cursor auto-scrolls into view; edges are debounced like `Tree`. Returns
      `ListOutput { clicked, activated, hovered, mouse_over_content }`. Honors
      `mouse_consumed`. Like `Table`/`ScrollView` it takes a raw `&mut
      InputState` (it consumes the wheel), not a `DrawContext`. No `UiContext`
      façade yet (raw widget first, as Tree shipped).
- [x] **P2 — Color picker.** `ColorPicker` (`src/widgets/color_picker.rs`) +
      `ColorPickerOutput`. SV square (white→hue→black, one per-corner gradient) +
      vertical hue spectrum bar + opt-in alpha bar (`.with_alpha(true)`, with a
      checkerboard under an opaque→transparent fade). Caller owns the color as
      `Hsva` (HSV is the source of truth so dragging S/V to an edge never loses
      hue); `draw(hsva, id, capture, rect, ctx) -> ColorPickerOutput { hsva,
      rgba, changed, dragging }`, threading like `Slider`'s value. Drag ownership
      via a shared `DragCapture` from a single `DragId` — each of the three
      sub-regions derives a collision-resistant id (private `region_id` mix), so
      SV/hue/alpha never co-claim one drag and don't clash with sibling drag ids.
      Builder: `.with_alpha`/`.with_bar_width`/`.with_gap`. Supporting additions:
      new `wgpu_gameui::color` module (`Hsva` + `hsv_to_rgb`/`rgb_to_hsv`) and
      `DrawList::quad_gradient(rect, [TL,TR,BR,BL])` per-corner gradient soup.
- [x] **P2 — Separator / divider.** `Separator` (`src/widgets/separator.rs`)
      + `Orientation { Horizontal, Vertical }`. Stateless, non-interactive: draws
      through a `&StyleResolver` like `Panel` (no `DrawContext`). Builder:
      `Separator::horizontal()`/`vertical()` then `.with_thickness(px)` /
      `.with_color([f32;4])` / `.with_inset(px)`; `.draw(rect, list, style)`. The
      rule is centered on its cross axis within the given `Rect` (hand it a taller
      cell and it sits mid-line), inset trims both ends, and length ≤ 0 draws
      nothing. Defaults are theme-relative: thickness = `StyleKey::BorderWidth`
      (min 1px), color = `StyleKey::PanelBorder`.
- [x] **P2 — Toast / notification / banner.** `Banner<'a>` is a stateless,
      non-interactive severity strip (`Severity::{Info,Success,Warning,Error}`):
      tinted background + left accent bar + optional bold title + wrapped
      message. Ctors `new/info/success/warning/error`, `.with_title(..)`,
      `measure_height(list, style, width)` for auto-sizing, `draw(rect, list,
      style)`. Accent colors are fixed RGBA (theme has no success/warning yet).
      `ToastStack` is the caller-owned transient layer (mirrors `TooltipLayer`'s
      `push`/`tick(dt)`/`draw` lifecycle): `push(Toast)`, `tick(dt)` ages and
      drops past-`ttl` toasts, `draw(screen_w, screen_h, list, style)` renders
      newest-nearest-corner banners with a last-`fade`-seconds alpha ramp via the
      draw list's tint stack (no animation state needed). Builders
      `with_corner(Corner::{Top,Bottom}{Left,Right})/with_width/with_max/
      with_fade/with_gap/with_margin`; `Toast::new(..).with_title(..).with_ttl(..)`
      (`DEFAULT_TTL` = 4s).
- [x] **P2 — Group / titled panel** (workshop equivalent of `UiWindow`).
      `Group<'a>` is a stateless titled container drawing through a
      `StyleResolver`: reuses `Panel::draw_at` for the bg+border, adds a
      lightened header strip (`header_h = FontSize + 2*pad`) with a `PanelBorder`
      separator and a `TextHighlight`-colored, vertically-centered title.
      `Group::new(title).with_padding(px)`; `draw(rect, list, style) -> Rect`
      **returns the inner content rect** (below the header, inset by padding,
      clamped ≥0) so callers lay children inside; `content_rect(rect, style)`
      computes it without drawing.
- [x] **P1 — Tooltip hover delay actually works.** `TooltipLayer::tick(dt,
      input)` accumulates hover time per region; `is_visible()` only
      returns true once the configured `with_delay_ms` has elapsed.
- [x] **P1 — Slider drag capture identity.** `DragCapture`/`DragId`
      (`src/widgets/drag.rs`) arbitrate a single drag owner across the UI.
      `Slider::draw` now takes `id: DragId` + `&mut DragCapture` instead of a
      caller `bool`, claims the drag only when the capture is free, and updates
      its value only while it owns the capture (also honors `mouse_consumed`).
- [x] **P1 — Checkbox fallback rendering** when icon textures aren't loaded.
      `Checkbox` now defaults to a theme-driven vector box + contrast checkmark
      (no atlas assets, never blank). Opt into textures via
      `Checkbox::with_icons(SpriteId, SpriteId)` (pre-resolved, guaranteed
      non-blank) or `with_icon_keys(&str, &str)` for knowingly-preloaded keys.

---

## Layout

- [x] **P1 — Min/max size constraints.** `Constraint { min, max }`
      (`Constraint::min`/`max`/`between`, CSS semantics: min overrides max)
      clamps a resolved dimension orthogonally to `SizeSpec`. `Size` gained
      `min_width`/`max_width`/`min_height`/`max_height`; `VStack`/`HStack` gained
      `.constrain(Constraint)` applied to the last-added child. (Single-pass for
      `Fill` — a clamped fill child doesn't redistribute slack.)
- [x] **P1 — Per-child alignment within stack cells** (Center/Start/End on
      cross axis). `CrossAlign` enum (Start/Center/End/Stretch), `align()`
      builder on VStack and HStack. VStack respects alignment on the width
      axis, HStack on the height axis. `src/layout.rs`.
- [x] **P1 — Content-driven children.** The concrete gap (`content_size` always
      0.0 for `Fill`/`Percent`) is addressed by per-child alignment: `Fill` and
      `Percent` children now have proper `cross_size` control via alignment, and
      `Stretch` fills the cross-axis span as before. Full generic-child content
      driving (passing `impl LayoutNode` into stacks instead of raw pixel sizes)
      deferred to P2 as a larger API refactor. `src/layout.rs`.
- [x] **P1 — Z-order / layers** (required for popups/modals). `LayerStack`
      provides ordered Modal/Popup/Tooltip layers; `UiRenderer::render_layers`
      renders base → layers in order.
- [x] **P2 — Weighted children** beyond equal-share `Fill`. `StackChild` gained a
      `pub weight: f32` (default 1.0) and both stacks a `.weight(f32)` last-child
      builder: `VStack::new(8.0).child_fill(0.0).weight(2.0).child_fill(0.0).weight(1.0)`
      splits remaining space 2:1. Each `Fill` child gets
      `remaining * (weight / sum_of_fill_weights)`; default weight 1.0 reduces to
      the old `remaining / fill_count` byte-identically (back-compat). Ignored for
      non-`Fill` specs.
- [x] **P2 — Wrap / flow layout** (inventory grids, mod lists). New
      `layout::Flow` node: `Flow::new(spacing).with_run_spacing(r).with_padding(p)
      .item(w, h)…` lays items left-to-right, wrapping to a new row on width
      overflow. Implements `LayoutNode` (`rects[0]` = bounds, `rects[1..]` = items
      top-aligned in add order). `content_size()` gives unwrapped single-row
      extents; `measure_height(width)` returns the true wrapped height for sizing a
      `Fit`/scroll container around it.
- [x] **P2 — Main-axis justification (`justify-content`).** `MainAlign`
      (`Start`/`Center`/`End`/`SpaceBetween`/`SpaceAround`/`SpaceEvenly`) on both
      stacks via `HStack::new(8.0).justify(MainAlign::SpaceBetween)…`. Distributes
      leftover main-axis space; defaults to `Start` (byte-identical) and is a no-op
      when a `Fill` child already consumed the slack (mirrors `flex-grow` vs
      `justify-content`). Completes the flexbox model alongside weighted `Fill`
      (`flex-grow`), `CrossAlign` (`align-items`), and `Flow` (`flex-wrap`); the
      remaining CSS-flex gap is `flex-shrink`.
- [x] **P2 — Stable node IDs in `LayoutResult`.** New `NodeId(pub u64)` (with
      `From<u64>`); tag children via `.id(n)` on `VStack`/`HStack` or
      `Flow::item_id(n, w, h)`, then resolve order-independently with
      `LayoutResult::get_by_id(n) -> Option<Rect>`. `LayoutResult` is now
      encapsulated (private `entries`) behind accessors —
      `container()`/`get(i)`/`get_by_id(id)`/`children()`/`iter()`/`len()`/
      `child_count()` — so positional `[i]` indexing (fragile under reordering) is
      retired. Builders funnel through a private `StackChild::new`.
- [x] **P2 — Borrow/arena API for `LayoutResult`.** `LayoutNode::layout_into(
      bounds, &mut LayoutResult)` is now the trait primitive (clears + refills a
      caller-owned buffer, reusing its capacity → zero per-frame allocation);
      `layout()` is a provided convenience wrapper that allocates. `Positioned`
      gains `layout_screen_into(w, h, &mut buf)`. Frame-loop callers hold one
      `LayoutResult` as scratch — mirrors the `ScrollState`/`DragCapture`
      caller-owned convention. Bench `layout_resolve` now has `alloc` vs `reuse`
      arms.

---

## Input & Focus

- [x] **P0 — Real focus model.** Caller-owned `FocusState`/`FocusId`
      (mirrors `DragCapture`/`DragId`) is the single focus owner: at most one
      widget focused at a time. `TextInput.draw(id, &mut focus, …)` registers
      itself in the draw-order Tab ring and requests focus on click; Tab /
      Shift-Tab cycle, Esc and click-elsewhere blur, only the focused input
      consumes keys. `TextInput.focused: bool` removed. Modal-scoped Tab
      trapping deferred (see below).
- [x] **P1 — Modal/popup-scoped Tab trapping.** Tab cycling is scoped to the
      active layer via `FocusState::register_layer(id, layer_idx)` and
      `end_frame(Some(layer_idx))`. Base layer focusables are excluded from
      Tab order when a modal/popup is active. Click-to-focus already respects
      `mouse_consumed`. `DrawContext.register_focus(id)` auto-scopes based on
      `DrawContext.active_layer`. (`src/widgets/focus.rs`,
      `src/widgets/mod.rs`)
- [x] **P0 — Full key event model.** `InputState` now has `key_left`,
      `key_right`, `key_home`, `key_end`, `key_delete`, `shift_pressed`,
      `ctrl_pressed`. Arrows/Home/End/Delete on physical keys; Shift/Ctrl as
      held modifiers. Ctrl+A/X/C/V handled via ASCII control codes in
      `text_input` or `ctrl_pressed` + letter. Blocks usable text editing.
- [x] **P0 — Hit-testing respects clip stack and z-order.** Layer-aware
      input dispatch via `InputState::mouse_consumed` +
      `LayerStack::input_for_layer`/`input_for_base`. `is_hovered()` honors
      the flag automatically. (Note: existing widgets that call
      `rect.contains(input.mouse_x, ...)` directly should additionally AND
      with `!input.mouse_consumed` — `Table` and the example already do.)
- [x] **P1 — Multi-button mouse** (right click, middle click) for context
      menus. `InputState` gained `mouse_right_down/clicked/released` and
      `mouse_middle_down/clicked/released`. Edge fields (`clicked`/`released`)
      are cleared by `end_frame` and zeroed by `consumed()`; held-state (`down`)
      passes through. Example wires `MouseButton::Right` and `MouseButton::Middle`
      from the winit event. 5 tests. (`src/lib.rs`)
- [x] **P1 — Double-click and click-and-hold distinction** (timestamps). Caller-
      owned `ClickTracker` (same pattern as `DragTracker`): `update(&mut
      InputState, time_secs: f64)` writes `InputState::mouse_double_clicked`
      (second press within `double_click_threshold`, default 450 ms; window
      resets after a double so a rapid third click starts fresh) and
      `InputState::mouse_held` (latches after `hold_threshold`, default 500 ms,
      and stays true until release). Both fields cleared by `end_frame` and
      zeroed by `consumed()`. 14 tests; example gains a "dbl/hold" demo button.
      (`src/click_tracker.rs`)
- [x] **P1 — Drag detection on `InputState`** (`is_dragging`, `drag_delta`)
      so widgets stop reinventing it. `InputState` gained `is_dragging: bool`
      and `drag_delta: [f32; 2]`; a caller-owned [`DragTracker`] (mirrors
      `DragCapture`/`ScrollState`/`FocusState`) holds the cross-frame press
      origin + click-vs-drag latch and writes those fields each frame via
      `update(&mut InputState)`. Press-to-drag threshold (default 4px,
      configurable); a still press never drags; `cancel()` aborts; `consumed()`
      and `end_frame()` clear the outputs. Complementary to `DragCapture`
      (ownership) — a window-mover uses both. (`src/drag_tracker.rs`, 13 tests;
      example has a "drag me" box.)
- [x] **P1 — Scroll wheel propagation/capture** with a "consumed" flag for
      overlapping scroll regions. `InputState::scroll_consumed` is set by any
      `ScrollView` that claims the wheel (even at a clamp boundary, so the event
      can't "bubble out" to an outer scrollable). 4 tests: basic consume,
      inner/outer nesting, cursor-outside-inner passes through. (`scroll_view.rs`)
- [x] **P1 — IME / composition** for CJK/accented input. Crate-level done:
      `InputState.preedit`/`preedit_cursor` + `TextInput` renders the inline
      underlined preedit spliced at the caret (`compose_preedit` in
      `src/widgets/text_input.rs`). Game-side winit plumbing
      (`WindowEvent::Ime` → preedit, `set_ime_allowed`/`set_ime_cursor_area`)
      is a follow-up — the game feeds no keyboard/text input to the UI yet.
- [x] **P1 — Keyboard navigation** (Tab/Space-to-activate, arrows in lists).
      Opt-in `.focusable(FocusId)` on `Button` / `Checkbox` / `Slider`: when set
      the widget joins the Tab ring (`ctx.register_focus`), draws a focus ring
      (new `Theme::focus_ring` + `DrawContext::draw_focus_ring`), requests focus
      on click, and is keyboard-operable while focused — Button/Checkbox activate
      on Space/Enter, Slider adjusts on arrows (Left/Down decrement, Right/Up
      increment by `step` or 1/20 range, clamped). `Slider::focusable` takes a
      `FocusId` distinct from its `DragId`. `Tree` gets arrow navigation via a
      `TreeState` nav-ring mirroring `FocusState`: `begin_frame(&InputState)`
      (rising-edge capture), `register_nav` per row, `end_frame(focused: bool)`
      — Down/Up move selection, Right expand-then-descend, Left collapse-then-
      ascend, Enter/Space toggle; gated on `focused` so it can't fight a focused
      text field. The whole tree is one Tab-stop (`TreeState::set_focus_id`). The
      `UiContext` façade wires it all: `text_button`/`checkbox` get auto-ids,
      `slider` reuses its DragId, tree verbs register one reserved `FocusId` and
      draw the ring on the selected row.
- [x] **P2 — Controller / gamepad** input abstraction. Reframed: the lib never
      reads devices (the game fills `InputState`), so this is a device-agnostic
      **navigation-intent** layer, not a device driver. `InputState::nav`
      (`NavInput { up/down/left/right, confirm, cancel, next, prev }`) is read by
      the focus system + widgets instead of raw key names, so keyboard and gamepad
      share one vocabulary. A `NavMap` fills it and is a **required** arg to
      `UiState::begin_frame` / `Frame::new` (can't be forgotten): `KeyboardNav`
      (default binding — arrows→directional, Tab/Shift+Tab→next/prev,
      Enter/Space→confirm, Escape→cancel), `ManualNav` (no-op; you set `nav`
      yourself), or any `Fn(&mut InputState)` closure. `map_keyboard` +
      `map_gamepad(&mut input, &GamepadNav)` OR into `nav` so devices compose
      (`|i| { map_keyboard(i); map_gamepad(i, &pad); }`). `GamepadNav` is a
      game-filled button/d-pad/stick snapshot (south=confirm, east=cancel,
      shoulders=prev/next). Slider reads directional intents (d-pad adjusts a
      focused slider); text/number inputs keep raw keys for caret/submit so a
      d-pad never moves a caret. List Home/End stay keyboard-only.
- [x] **P2 — Explicit `Frame`/`Ui` builder** that consumes input and produces
      a draw list, instead of implicit `end_frame` that callers can forget.
      `Frame::new(&mut state, &mut input, &theme, &KeyboardNav).dt(dt).run(&mut list, |ui| { … })`
      (and `.run_layers(&mut layers, |ui| …)`; sugar `state.frame(&mut input,&theme,&KeyboardNav)`).
      The fourth arg is the required `NavMap` (see the gamepad item above).
      Closure-scoped: runs `UiState::begin_frame` before the build closure and
      `UiState::end_frame` after — the pair can't be forgotten or mis-ordered,
      and the closure's return value is threaded back out. `UiContext` is built
      interactive inside, dropped (firing its push/pop balance checks) before
      `end_frame`. **Per-surface scope by design**: `Frame` does *not* call
      `InputState::end_frame` (that clears per-frame edges and must run once per
      whole app frame, after all surfaces/layers/manual widgets) — that single
      call stays the caller's. `src/frame.rs`; unit tests + doctest.

---

## Text

- [x] **P0 — Real text measurement** via glyphon shaping. Centering today
      uses `len() * font_size * 0.5` in 6+ places; broken for proportional
      fonts, multi-byte UTF-8, non-ASCII. (Teardown ships `UiGetTextSize` for
      this reason.)
- [x] **P0 — Text selection.** Caret state, shift+arrow selection,
      click-to-position-cursor, selection highlight rendering, Ctrl+A select all,
      proper backspace/delete. (`src/widgets/text_input.rs`)
- [x] **P0 — Copy/paste / clipboard hooks.** Closure-based clipboard API on
      `TextInput` (`clipboard_get`/`clipboard_set`). Users wire the platform
      clipboard (e.g. `arboard`). Cut/copy/paste (Ctrl+X/C/V) when closures are
      set.
- [x] **P0 — Font system.** Runtime font loading (`load_font_file`/
      `load_font_bytes` → `FontHandle`), per-`TextBlock` font selection
      (`with_font`/`with_font_opt`), bold/italic/weight (`TextBlock::bold()`/
      `italic()`/`with_weight()`/`with_style()`, threaded through the shape
      cache + measurement), a bundled default font (Noto Sans, behind the
      default-on `bundled-font` feature → deterministic `Family::SansSerif` via
      `register_bundled_fonts`), `Theme.font` driving every widget, and the
      Teardown `UiFont(family,size)` push/pop font stack on `UiContext`
      (`font`/`font_size`/`font_family`/`bold`/`italic`/`text_line`).
      cosmic-text's script/glyph fallback is automatic. (Synthetic bold/oblique
      for absent faces is out of scope — cosmic-text selects real faces only.)
- [x] **P1 — Multi-line `TextInput` / textarea.** `TextInput::with_multiline`
      (and the `UiContext::text_area(id, buf, ph, w, rows)` façade): Enter inserts
      `\n`, the value wraps to the field width, Up/Down navigate visual lines with
      a sticky column, Home/End are line-relative, selection renders per line, and
      the field clips + autoscrolls vertically to keep the caret visible
      (`scroll_offset`). Line-aware caret/click via `text_caret_layout`/`CaretPos`.
- [x] **P1 — Text wrapping policy.** `TextBlock::with_wrap(WrapMode)` —
      `None`/`Word`/`Glyph`/`WordOrGlyph` (default `WordOrGlyph` preserves prior
      rendering). Single-line `TextInput` now uses `WrapMode::None` (no silent
      wrap-to-hidden-second-line); multiline uses `WordOrGlyph`.
- [x] **P1 — Ellipsis on overflow.** `TextBlock::with_ellipsis` lays the text
      on one line and truncates with an `…` reserved at the right edge
      (`ellipsize_to_width` in `src/text.rs`); opt-in, no-op when the text fits.
- [x] **P1 — Cursor x-position from real shaping**, not the same broken
      `len*0.5` formula.
- [x] **P2 — Password / masked input.** Done. `TextInput.mask: Option<char>`
      (builders `.password()` → `'•'`, `.with_mask(ch)`): `value` stays plaintext
      while the field *displays* and *measures* one mask glyph per char. Masking
      forces single-line semantics (Enter submits, Up/Down inert) and suppresses
      inline IME preedit so it can't leak. All single-line hit-testing/caret/
      selection geometry runs on the masked display via char-aligned byte mapping
      (`value_to_display_byte`/`display_to_value_byte`). Façade verb:
      `UiContext::password_input(id, buffer, placeholder, w)`.
- [x] **P2 — RTL / bidi exposure** (glyphon supports it; no public knob).
      cosmic-text already runs the Unicode bidi algorithm, so the work was
      exposing the missing knobs and making editing bidi-aware. API:
      `TextDirection { Auto, Ltr, Rtl }` + `TextBlock::with_direction` /
      `TextInput::with_direction` force a base direction (via a leading
      LRM/RLM mark) for direction-neutral content. `TextAlign` gained
      direction-relative `Start`/`End` (default is now `Start`, byte-identical
      to the old `Left` default; `Left`/`Right` are absolute). New shaping
      primitives: `text_visual_layout` → `Vec<VisualGlyph>` (visual-order,
      bidi-level-tagged), the pure `selection_rects` (bidi selections split
      into multiple visual rectangles), `visual_caret_neighbor` (visual
      Left/Right caret movement), and `visual_caret_pos`/`VisualCaret`
      (edge-correct caret rendering). `text_caret_layout` is now
      direction-aware. Single-line `TextInput` is fully bidi-precise (visual
      caret movement + edge-correct caret + multi-rect selection); multiline
      stays LTR-correct (multiline RTL caret edge-precision is a documented
      limitation, alongside direction-boundary caret affinity).
- [x] **Vertical (stacked) text** for JP-style labels (ad-hoc, not on the original
      backlog). API: `TextBlock::with_vertical()` stacks each grapheme cluster on
      its own row, top-to-bottom, centered within the column; row pitch is the
      block's `line_height`. `TextMeasurer::measure_vertical(text, font_size)`
      returns the `(column_width, stacked_height)` for layout. cosmic-text has no
      writing-mode API, so this lays the string out one cluster per buffer line
      (`vertical_stack_string`) and lets cosmic's existing line stacking + manual
      in-column centering do the rest — zero new rendering code, glyphs stay
      upright. Scope is casual upright stacking (correct for full-width kana/kanji;
      Latin stacks per-letter), **not** true CJK `vertical-rl` (no vertical glyph
      variants / rotated kana-punctuation / tate-chu-yoko / right-to-left columns),
      and labels only (no vertical `TextInput` editing). `with_align` +
      `with_max_width` position the whole column horizontally within the slot
      (`Start`/`Left`, `Center`, `End`/`Right`), mirroring horizontal alignment.
- [x] **Single-line text-input selection/caret vertical centring.** The
      selection highlight and caret band are centred on the field box rather than
      anchored at the text line-box top (which is tuned to baseline-place glyphs,
      so the band rode ~0.25em high over the text). Multiline keeps its per-line
      tops. Gallery now shows both an LTR and an RTL selected field for contrast.

---

## Theming / Styling

- [x] **P0 — DPI / scale factor** propagated through renderer; affects
      glyphon resolution, vertex output, layout. Teardown's `UiScale` exists
      (~1.2k mod calls). `UiRenderer::render`/`render_layers` take a
      `scale_factor`; the ortho is built from the logical size while the
      framebuffer stays physical (MSDF text self-sharpens via `fwidth`).
- [x] **P1 — Multiple fonts / sizes / weights** (see font system above) —
      per-`TextBlock` family/size/weight/style, `Theme.font`, and the
      `UiContext` font stack all land this.
- [x] **P1 — Per-widget style override** without copying the whole `Theme`.
      Added a no-clone **style resolver** (`src/style.rs`): `StyleKey` (one
      variant per theme field + `Custom(u64)` name-hash), `StyleValue`
      (`Color`/`Scalar`), `StyleOverlay` (caller-owned sparse override set), and
      `StyleResolver` (precedence: overlay → theme). Every widget now resolves
      style through the resolver — `DrawContext` carries an optional
      `&StyleOverlay` (`ctx.color(key)`/`ctx.scalar(key)`, set via
      `DrawContext::with_style(&overlay)`); bare-`&Theme` widgets/free-fns
      (`Panel`, `ProgressBar`, `Tabs`, `Table`, `Tooltip`, `List`, `ScrollView`,
      `label`/`title`/…) now take `&StyleResolver`. `UiContext` gained a scoped
      style stack (`set_style`/`set_style_color`/`set_style_scalar`/`clear_style`,
      pushed/popped with `push`/`pop` like the tint/font stacks) so a subtree
      restyles without a theme clone. No-overlay path is value-identical to the
      old `theme.<field>` reads.
- [x] **P1 — Extensible theme** (typed style map / `HashMap<StyleKey,
      StyleValue>`) so custom widgets don't need core changes. `Theme` keeps its
      flat typed fields as the source of truth and bridges them through
      `Theme::get`/`set(StyleKey, StyleValue)`; a `custom: HashMap<u64,
      StyleValue>` map holds mod-defined keys. `StyleKey::custom(name)` addresses
      them by FNV-1a name-hash (no global interner). A custom widget can carry
      its own keys via an overlay or `Theme::register_style`.
- [x] **P1 — Hover/press animation clock + transitions / easing.**
      egui-style "animate toward the resolved color": each frame a widget
      resolves its discrete target color and the clock eases the *displayed*
      color toward it, re-basing when the target changes (no muddy multi-state
      blend). `src/animation.rs`: `AnimationState` (caller-owned, dt-driven,
      keyed by `(u64 id, AnimSlot)`), `Easing{Linear,EaseIn,EaseOut,EaseInOut}`,
      and public `ease`/`lerp`/`lerp_color` (endpoint-snapping). Duration is a
      themeable scalar — `Theme::animation_duration` (default `0.12`) /
      `StyleKey::AnimationDuration`, overridable per-subtree via `StyleOverlay`;
      `0.0` snaps. Seam: `DrawContext::with_animations(&mut AnimationState)` +
      `ctx.animate_color(id, slot, target)` / `animate_scalar(...)` (resolve
      duration, ease-out; return `target` unchanged when no state is attached →
      byte-identical to the instant path). Raw widgets opt in with `.animated(id)`
      — adopted in **Button** (bg + border), **Checkbox** (box fill + hover
      overlay alpha), **Tabs** (per-tab bg + label, sub-key
      `base_id.wrapping_add(i)`). Façade auto-wires it: `UiState.anim` ticked by
      `begin_frame(input, theme, dt)`, and `text_button`/`checkbox` pass
      `.animated(auto_id)` + the shared state, so interactive-mode apps get
      hover/press easing for free. Hard invariant held: with no state (or
      `dt`/`duration == 0`) every drawn value is byte-identical, so existing
      tests + the gallery stayed green. Gallery: "Hover animation (easing)" ramp
      samples the ease-out curve at t∈{0,.25,.5,.75,1}. Other raw widgets adopt
      the same `.animated(id)` pattern as needed.
- [x] **P2 — Theme stack** (push tint/color), tied to A5/D8 above. Done — landed
      with the `UiContext` scope stack: `push()`/`pop()` save & restore the
      transform, **tint**, align, font, **style overlay**, and clip/window scopes
      together. Scoped setters: `set_style_color`/`set_style_scalar`/`set_style`
      (per-`StyleKey` overrides layered overlay→theme via `StyleResolver`) and
      `color`/`color_filter` (DrawList tint set/multiply, baked into every
      vertex/instance/text at push time). Tested
      (`style_override_recolors_button_chrome_and_pops_with_frame`,
      `color_replaces_tint`, `color_filter_multiplies_tint`, `push_pop_*`) and
      shown in the gallery "Styling / overrides" section.
- [x] **P2 — Move semantic policy out of theme.** Done. The theme now supplies
      only a palette; the *policy* mapping state→palette is caller-owned (progress)
      or a trivial themeable mapping (severity).
      - **Progress:** extracted the hardcoded `<0.25/<0.5` banding into a
        caller-owned `ProgressFill` policy: `ProgressFill::Stat { low, medium }`
        (default `{0.25, 0.5}` — preserves prior behavior, thresholds now named &
        tunable) or `ProgressFill::Solid(StyleKey)` for neutral progress where low
        isn't "bad". Set via `ProgressBar::with_fill(..)`; the value→color decision
        is no longer baked into the widget/theme.
      - **Severity:** added themeable `StyleKey::Info/Success/Warning` (+ matching
        `Theme.info/success/warning` fields; `Error` reuses the existing color), so
        `Banner`/`ToastStack` colors resolve through the style system
        (`Severity::style_key()` → `StyleResolver::color`) and a `StyleOverlay`/
        custom `Theme` can recolor severities. `Severity::accent()` kept as the
        resolver-free default, kept in sync with the default theme palette.

---

## Game / Teardown-Specific

- [x] **P0 — Lua-binding-friendly facade** (`UiContext` with state stack)
      that backs `UiText`/`UiTextButton`/`UiImageBox`/`UiSlider`/etc. as
      stateful immediate-mode calls. `UiContext` (push/pop,
      translate/rotate/scale, align/center, color/color_filter, place_rect,
      quad/rounded_rect/text_block) plus the font stack landed earlier; the
      interactive mode (`UiContext::interactive`/`interactive_layers` +
      caller-owned `UiState`) now provides the auto-advancing stateful verbs
      `text`/`text_button`/`slider`/`checkbox`/`image_box`/`text_input`.
      Remaining: UI sound hooks (tracked separately below).
- [x] **P0 — World-space UI** (`UiWorldToPixel`/`UiWorldToScreen`) for
      in-world labels, damage numbers, health bars over NPCs.
      `projection::world_to_screen`/`world_to_screen_na` project a world
      point to UI pixel space (None behind the camera).
- [ ] **P1 — UI sound hooks.** `UiSound`/`UiSoundLoop` and button
      hover/press sounds. **Deferred to the integrating app** (decision
      2026-06): this library is render-only and has no audio backend, so the
      app owns sound — it already gets the interaction edges it needs from the
      widget return values + `HitZoneOutput` (`clicked`/`pressed`/`hovered`/…)
      to trigger its own SFX. Re-open only if a built-in hook proves necessary.
- [x] **P1 — Mod-friendly *style* registration.** `register_style(name,
      value)` landed: `Theme::register_style(&mut self, name, StyleValue)` +
      `Theme::style(name) -> Option<StyleValue>` store/read custom keys by
      name-hash (and `StyleOverlay` can carry them per-subtree). The custom-widget
      half is split out to P2 below.
- [ ] **P2 — Mod-friendly *widget* registration** (`register_widget(name,
      draw_fn)`), **gated on Lua integration** (decision 2026-06; tracked under
      "Beyond 1.0 → mod registry"). Widgets have heterogeneous signatures and
      there's no uniform draw-fn contract yet, and the right shape is hard to know
      without a concrete modding consumer driving the requirements — so design it
      against a real `register_widget` call site (the Lua binding layer) rather
      than in the abstract. Re-tiered from P1: not needed for 1.0 usability.
- [x] **P1 — `UiMakeInteractive` / hit-zones independent of draw** for
      sensors over 3D things. `HitZone` (`src/widgets/hit_zone.rs`) is the
      deliberate draw-free widget: `HitZone::new().test(rect, &input) ->
      HitZoneOutput` reports `hovered`/`pressed`/`clicked`/`released`/
      `right_clicked`/`middle_clicked`/`double_clicked`/`held`/`scroll_delta`/
      `local_pos` over a screen-space `Rect` without touching any `DrawList` — so
      it lays over regions this UI didn't paint (a 3D viewport, a
      `world_to_screen` rect). Takes a plain `&InputState` (nothing for a
      `DrawContext` to carry); honors `InputState::mouse_consumed` for layer
      capture; reports only (never sets `mouse_consumed`) like the other
      per-layer widgets — gate world-picking on `!out.hovered`. `.enabled(false)`
      makes it inert. Façade verbs: `UiContext::hit_zone(w, h)` (flow-placed
      cell, auto-advances) and `UiContext::hit_zone_at(rect)` (explicit screen
      rect, no advance) for absolute sensors.
- [x] **P2 — Cursor state control** (`UiSetCursorState`, I-beam over text).
      Windowing-agnostic `CursorIcon` enum (Default/Pointer/Text/Grab/Grabbing/
      ResizeHorizontal/ResizeVertical/NotAllowed) + caller-owned `CursorState`
      accumulator (`request`/`resolve`/`take`/`begin_frame`, priority-arbitrated
      so an active `Grabbing` beats a stray `Pointer`). Widgets request via a new
      `DrawContext::with_cursor`/`request_cursor` seam (mirrors `with_animations`;
      no-op when unset): TextInput/NumberInput field → Text, Button/Checkbox/
      Dropdown → Pointer, Slider/DragHandle → Grab/Grabbing. The app reads
      `CursorState::resolve()` after the frame and maps it to its windowing API;
      `examples/hello_ui.rs` shows the winit `set_cursor` mapping. (Widgets that
      take raw `list`/`style`/`input` rather than a `DrawContext` — Tabs,
      ImageButton, the dropdown open-list — and the pure-hit-test HitZone have no
      seam and are left to the caller.)
- [x] **P2 — Backdrop blur** (`UiBlur`) for menu screens.
      `UiRenderer::blur_backdrop(device, queue, encoder, target, &Backdrop{view,size},
      region: Rect, viewport, scale_factor, &BlurParams{radius, downsample, tint})`.
      Separable two-pass Gaussian over an **app-provided** scene `TextureView`
      (the renderer never samples its own framebuffer, so blur isn't a `DrawList`
      record — the data layer stays GPU-handle-free). Pass A blurs horizontally
      scene→downsampled intermediate; pass B blurs vertically intermediate→target
      over the region's NDC rect (alpha-blended, so a darkening `tint` is a scrim).
      Pipeline built lazily on first call (`new` unchanged). Frame order:
      render scene → `blur_backdrop(region)` → `render(panels)` on top. Pure
      geometry/weight helpers unit-tested; GPU readback test (`tests/blur.rs`)
      asserts a sharp edge gets smeared and wider radius widens the band; gallery
      "Backdrop blur (UiBlur)" section shows a frosted-glass PAUSED menu.
- [ ] **Out of scope but don't block:** depth-aware `DrawSprite`/`DrawLine`
      in 3D world space — keep UI overlay vs. world overlay passes
      separable.

---

## Testing & Docs

- [x] **P1 — Widget tests.** Every widget module has `#[cfg(test)]` with
      headless `DrawList` tests; `text_input.rs` alone has 20+ tests, and
      `draw_list.rs`, `scroll_view.rs`, `button.rs`, `dropdown.rs`, etc. all
      have test suites. The TODO description was stale — tests existed at the
      time the repo was extracted from citybuilder and audits hadn't caught
      them. (Confirmed 2026-04-27: every `src/widgets/*.rs` except `mod.rs`
      has `#[cfg(test)]`.)
- [x] **P1 — `examples/` directory with at least one runnable wgpu
      example.** `examples/hello_ui.rs` opens a window and renders a panel +
      button + icon + nine-slice + text via `UiRenderer`.
- [x] **P2 — Rustdoc on all public types.** Done: enabled crate-level
      `#![warn(missing_docs)]` (lib.rs) as a permanent guard and documented all
      314 previously-undocumented public items across 26 modules (text, layout,
      theme, style, render/*, every widget, lib/ui_context/affine/layer). Also
      fixed every `cargo doc` intra-doc-link warning (renamed/stale paths
      qualified, private-item links demoted to code spans, one stale button
      example updated to the current `draw(rect, &mut ctx)` API). `cargo doc
      --no-deps` and `cargo build --all-targets` are both warning-clean.
- [x] **P2 — README quickstart, widget gallery, architecture overview.**
- [x] **P2 — Bench suite** (`benches/`) for hot paths (text shaping, draw
      list construction, layout, interactive widgets, scroll view, list, table,
      UiContext facade, animation).

---

## Suggested 1.0 Roadmap (in priority order)

1. **Renderer + atlas** — ship a working `UiRenderer` and texture atlas; the
   crate currently can't actually draw quads/sprites/nine-slices.
2. **Rounded rects + lines + clip stack** — foundational primitives that
   unlock proper-looking UI and scroll views.
3. **Matrix-stack `UiContext`** (push/pop/translate/align/color) — required
   for Teardown port, dramatically improves widget ergonomics.
4. **ScrollView + modal/popup layer** — enables dropdown, color picker,
   drag handle, etc.
5. **Real text editing**: focus model, full key events, selection,
   clipboard.
6. **Font system + DPI scaling.**
7. **Dropdown, Image, ImageButton** — closes biggest widget gaps.
8. **Real text measurement** (glyphon-backed) — fixes alignment everywhere.
9. **Layout: min/max, alignment, content-driven children.**
10. **Widget API unification + widget tests + runnable example.**

Beyond 1.0: world-space UI, sound hooks, mod registry, color picker,
collapsing header, blur backdrop, controller input.

---

## 2026-06-17 — UiContext verb coverage

Added `UiContext` façade verbs for previously-unwrapped widgets:

- **Non-interactive themed:** `separator()`, `progress_bar(value, w)`,
  `banner(severity, message, w)`, `group_begin(title, w, h) -> Rect`,
  `panel(w, h)` — draw through `StyleResolver` + auto-advance.
- **Interactive:** `tabs(labels, active) -> Option<usize>`,
  `image_button_key(key, w, h) -> bool`,
  `image_button_sprite(sprite, w, h) -> bool`,
  `color_picker(id, &mut hsva, w) -> ColorPickerOutput`,
  `drag_handle(id, w, h) -> DragHandleOutput`.
- **ScrollView:** `scroll_begin(w, h) -> Rect` / `scroll_end()` pair using
  existing `UiState::scroll`.
- **Dropdown:** `dropdown(id, options, selected, w)` using existing
  `UiState::dropdowns`. Layer lifecycle wired into
  `UiState::begin_frame`/`end_frame` + new
  `UiState::push_dropdown_layer`/`draw_dropdown_layer`.

**Convenience state:** added `toasts: ToastStack` and `tooltips: TooltipLayer`
to `UiState`.

**Not added as verbs (closure-based, keep raw widgets):** `List`, `Table`.

---

## 2026-07-03 — daemon-enabling APIs

Changes driven by the notification-daemon integration audit, each closing a gap
the daemon would otherwise work around:

- **Raw-RGBA image loader (`load_image_rgba8`).** The decode-free sibling of
  `load_image_bytes`: inserts already-decoded RGBA8 pixels under a key into both
  the atlas *and* the image cache (so `has_image`/`image_size`/`unload_image` see
  it). The daemon already holds decoded icon pixels, so this avoids a pointless
  RGBA→PNG→decode round-trip per notification. (`UiRenderer::load_image_rgba8`;
  `load_sprite_rgba8` remains the cache-bypassing out-of-band path.)
- **Atlas eviction / reclaim.** `SpriteAtlas` is now tombstone-based
  (`Vec<Option<StoredSprite>>` + a free-list): `remove(id)` frees a slot (its
  pixels reclaimed, the index recycled by the next load) without shifting or
  renumbering any other `SpriteId`. `compact()` repacks the live sprites into
  fresh shelves, reclaiming shelf fragmentation while preserving every id — safe
  because atlas regions are pixel rects re-derived into UVs every render and a
  dirty atlas triggers a full re-upload (the invariant `try_grow` already relied
  on). `UiRenderer::unload_image` now frees the slot (previously it leaked the
  pixels); `UiRenderer::compact_atlas()` defrags on the caller's schedule;
  `UiRenderer::atlas_size()` exposes the texture dims for monitoring. This is
  what keeps a long-running daemon's one-off-icon churn from climbing the atlas
  to its 4096² panic cap.
- **`NumberInput` display formatter.** `NumberInput::with_formatter(fn(f64) ->
  String)` (e.g. zero-padding `7` → `"07"` for an HH field) and
  `with_parser(fn(&str) -> Option<f64>)` (when the display text isn't directly
  `f64`-parseable; bypasses the default numeric sanitize so the parser owns
  validation). Formatter-only keeps the default sanitize + parse path. The
  positional `number_input` façade verb is unchanged (already at the
  too-many-arguments limit) — zero-padding uses the raw widget, the architecture's
  "full control" path.
- **Enabled/disabled subtree scope.** `UiContext::enabled_scope(enabled, |ui| …)`
  / `disabled_scope(|ui| …)` — egui's `add_enabled_ui` analogue. Self-balancing
  (closure-scoped `push`/`color_filter`/`pop`): when disabled, gray-tints the
  block and feeds every interactive verb an inert `InputState::consumed()` clone
  via a per-frame `input_disabled` flag consulted at the per-widget input-clone
  points (`localize`, `hit_zone`, `hit_zone_at`, `scroll_begin`/`end`). First
  closure-based scope verb in the file, chosen for RAII balance; nesting is
  absolute (an inner `enabled_scope(true)` re-enables, restored on exit).


---

## 2026-08-02 — Layout inspection ("debug mode" for agents)

Motivation: positioning bugs — a widget off-screen, overflowing its container,
clipped away, or misaligned by a fraction — were invisible. There was no way to
ask the library *what it actually drew*, and the only way to look at a frame was
the headless-PNG recipe copy-pasted across ten test files. Agents in particular
had no way to check their own work.

- [x] **Debug scopes on `DrawList`.** `push_debug_scope(name)` /
  `push_debug_scope_rect(name, rect)` / `pop_debug_scope()` /
  `debug_scopes()` / `debug_scope_depth()` / `truncate_debug_scopes(depth)`,
  plus `DebugScope` and `PrimCounts`. A scope records the *buffer lengths* at
  push and pop; because every geometry buffer is append-only, that delimits a
  contiguous span in each one. **No primitive method or widget changed**, and
  the cost is zero unless a scope is pushed. `color_cmds` is deliberately not
  spanned — `push_chrome_instance` merges runs by mutating its last element, so
  it is the one buffer that is not append-only.
- [x] **`DrawList::dropped_degenerate()`.** Counts primitives rejected by a
  non-positive size/radius/thickness guard (11 call sites). This is the only
  evidence that an element *collapsed*: `quad` and friends return early on a
  degenerate rect, so a width computed as `available - padding * 2` that went
  negative leaves nothing at all in any buffer. Spanned by scopes.
- [x] **`src/debug.rs` — `DebugReport`.** `from_draw_list` / `measured` /
  `from_layers` / `measured_layers` build a named, nested tree of world-space
  bounding boxes ordered by `RenderPass` (buffer order is *not* Z order — the
  renderer draws nine-slices → colour → icons → MSDF icons → text). Renders as
  an indented tree via `to_text()` or as JSON via `to_json()` (hand-rolled; no
  serde dependency), or straight to disk with `write_to_dir()`. Un-scoped draws
  still appear, named on a best-effort basis (a text block by its own content,
  an icon by its atlas key) and nested by geometric containment, with the report
  stating how many names were inferred.
- [x] **Lints + `assert_clean()`.** `Problem` / `Severity` / `LintConfig`, each
  finding carrying its coordinates and a `hint()` explaining the likely cause.
  Defaults are tuned to be silent on correct UIs: `partially_clipped` and
  `sibling_overlap` are off (a scroll view and a panel background respectively
  do those by design), `partially_off_screen` is off (toasts animate in from
  off-screen), and `near_miss_alignment` only compares *named* siblings — a
  panel's own border quads sit a pixel apart by construction. `assert_clean()`
  guards on `Error` only; warnings cover the merely suspicious.
- [x] **`near_miss_alignment` tightened to what it can actually claim.** It is
  the one lint asserting *intent* — that one piece of layout code placed these
  and meant their edges to match — so it now requires (a) the parent to be a
  **scope**, since an inferred parent can be a bounding box around unrelated
  shapes, (b) the elements to be **stacked along the other axis** from the edge
  compared, since the `left` edges of a row are deliberately different numbers,
  and (c) at most **one finding per element per axis**, preferring the origin
  edge, since a dropped element has a wrong top, bottom *and* centre for one
  reason. Loose soup geometry is also barred from adopting children in
  `nest_by_containment`: it is aggregated per scope, so its rect is a box drawn
  around disjoint triangles, and in the gallery that one node spanned the whole
  4482px page and adopted 309 others. Took the gallery from 15 alignment
  findings — all false — to zero, without loosening `align_max`, which would
  have blinded the lint to exactly the 1–2px errors it exists to catch.
  `tests/alignment_gallery.rs` is the tuning fixture: six labelled cases, two
  meant to fire and four meant to stay quiet, rendered to a readable PNG.
- [x] **`DrawList::push_clip_viewport(rect)` / `viewport_clips()`.** A clip that
  erases an element is a layout bug when the clip is a hard boundary (a window,
  a panel) and *the entire point* when it is a viewport — a scroll view hides
  the rows either side of its window on every frame it works correctly. Only the
  pusher knows which it meant, so `ScrollView::begin`, `Dropdown`'s option list
  and multiline `TextInput` now say so. The report tags nodes a viewport removed
  with an `off-viewport` effect (so they are still visible in the tree, with the
  reason) and skips them in the `fully_clipped` lint; what is left is content
  that missed its container, which is now an `Error`. Sticky through nesting:
  a node's clip is the whole stack intersected, so anything inside a viewport
  carries a clip contained by it however deep. Took the widget gallery from 33
  `fully_clipped` warnings to **zero problems across 863 nodes**.
- [x] **`TextMeasurer::measure_block` / `DrawList::measure_block`.** Measures a
  `TextBlock` as it will actually be laid out — font, weight, style, wrap, and
  `vertical` included. The existing `measure*` methods hard-code most of those,
  so they report the wrong size for any bold, italic, custom-font, or stacked
  block.
- [x] **`TextMeasurer::measure_block_ink` / `DrawList::measure_block_ink`.**
  `(top, bottom)` offsets of the real glyph **ink** below a block's top edge, or
  `None` when nothing inked. `measure_block` reports the *slot* a block reserves
  (advance × line box); this reports what lands on screen, and the two differ by
  design — `vcentered_text_y` slides the whole line box up so the glyphs' optical
  centre hits the row centre, so a correctly centred label's line box always
  pokes out of its row. The report measured the box and called it ink, which
  turned every centred label in the gallery into a 1.92px overflow. Per glyph the
  band is `baseline − y_max` … `baseline − y_min` of its outline box, faces
  parsed once per run rather than once per glyph, cached separately so the
  drawing path never pays for it.
- [x] **`Rect` helpers.** `right`, `bottom`, `is_empty`, `union` (empty operands
  contribute nothing, so it folds), `contains_rect(other, tolerance)`
  (edge-inclusive), `inset`.
- [x] **`UiContext` integration.** `push_debug_scope(name)` / `pop_debug_scope()`
  / `debug_scope(name, |ui| …)`, scopes closed by the enclosing `pop()`, and a
  `Drop` balance assert alongside the existing ones. `window_begin` opens a scope
  declaring its rect; `window_begin_named` gives it a real name. The app-facing
  verbs take a **name only** — see the widget-declaration item below for why.
- [x] **Widgets declare their own allocation.** Every widget entry point opens
  `push_debug_scope_rect(Type("label"), rect)` around its body, so the box layout
  handed it is checked without anyone opting in. Rect declaration is a
  widget-implementor concern: an application passing coordinates to a debug verb
  is absolute positioning in disguise, duplicating a number layout owns and free
  to go stale. Took the gallery from 861-of-863 inferred names to 143 real widget
  scopes.
- [x] **`src/render/capture.rs`.** `capture_draw_list` / `capture_layers` /
  `write_png` need no async runtime (`Device::poll(Maintain::Wait)` is
  synchronous) and are always available; `HeadlessGpu` sits behind the
  **default-on** `headless` feature. Both non-obvious steps are handled: the
  `LoadOp::Clear` pass `UiRenderer::render` does not do, and the 256-byte
  row-alignment de-pad that otherwise shears the image.

### Bugs this immediately found and fixed

Running the report over the widget gallery reported 11 elements drawn entirely
off-screen. All were one root cause: **`place_rect` returns *world* space, but
`DrawList` re-applies the active transform at push time.**

- [x] `panel`, `separator`, `progress_bar`, `banner`, `group_begin` passed the
  world rect straight to a widget, double-applying the translation and placing
  the widget at twice its intended offset. Fixed with a new private
  `place_local`; the interactive verbs already localized correctly.
- [x] `scroll_begin` had the same bug, plus its scrollbar hit-test compared a
  world-space pointer against a local-space rect. It now localizes, storing the
  begin-time inverse for `scroll_end` (which draws *after* `ScrollView::end`
  pops the transform).
- [x] `scroll_begin` advanced the layout cursor *before* the caller's content was
  drawn, pushing that content out of the viewport it was supposed to sit inside —
  and `ScrollView::end` then popped the transform, discarding the advance so
  nothing after the scroll region moved either. The advance is now in
  `scroll_end`. The gallery's scroll cell rendered completely empty before this.

---

## 2026-09-15 — Menubar (one level), and a renderer bug it surfaced

Design: `docs/design/menubar.md` (phases 1–2 landed).

- [x] **P0 — Menubar widget, one level.** `src/widgets/menubar/`
  (`mod`/`model`/`placement`/`state`/`paint`/`tests`), re-exported from the crate
  root as `MenuBar`, `MenuBarState`, `Menu`/`MenuItem`/`MenuItemId`/`MenuBarId`,
  `MenuTrigger`, `Accelerator`/`Key`/`Modifiers`/`AccelPlatform`, `SubmenuSide`,
  `ActivatedItem`, `MenuLayers`, `MenuDrawEnv`, plus `place_popup` and
  `blocker_regions` as pure, unit-testable helpers.
  Per-frame contract (all caller-owned state, no globals):
  `state.begin_frame(&mut input)` → `state.push_open_layers(&mut layers)` →
  `layers.input_for_base(&input)` → `bar.draw(rect, &mut state, &mut ctx) ->
  MenuBarOutput` → `state.draw_open_layers(&mut layers, slots, menus, &mut env)
  -> Option<ActivatedItem>` → `state.end_frame(&mut focus)` (before
  `FocusState::end_frame`).
  The bar claims the nav intents it acts on by zeroing them in the shared
  `InputState` at frame-top, so a focused widget later in the frame never sees an
  edge the menu handled; Tab is never claimed. An open chain registers a
  full-rect blocker per column and a viewport blocker that excludes the bar strip
  (so the labels stay live for hover-to-switch); both are registered as
  `InteractionScene` regions as well, because the scene dispatches among
  registered regions only. Three new style scalars: `MenuRowHeight`,
  `MenuItemMinWidth`, `MenuAccelGap`. 46 unit tests; a `widget_gallery` row
  (strip + File's open column: accelerators, separator, check mark, submenu
  chevron, disabled row), seeded through the real input path.
- [x] **P0 — Two or more `UiRenderer` passes recorded into one submit corrupt
  each other's geometry.** FIXED. `UiRenderer::begin_frame` is the explicit frame
  boundary: call it once per frame — before the first `render`/`render_layers`
  whose passes reach the GPU through one `queue.submit` — and it resets every
  per-frame arena (colour vbo/ibo, icon/chrome/circle/nine instance buffers, text
  vbo, and the new uniform arenas) plus `RenderStats`, and runs the sustained-
  pressure observation for the previous frame. `prepare_frame` became
  `prepare_pass` and resets nothing.
  Each pass also gets its **own ortho uniform slot** now, via
  `src/render/uniform_arena.rs` (`UniformArena`: dynamic-offset slots at
  `min_uniform_buffer_offset_alignment` strides over one growable
  `UNIFORM|COPY_DST` buffer, bind group rebuilt on growth, replaced buffers
  retained). This covers `UiRenderer`'s five pipelines, `TextRenderer` (whose
  public `resize` grew `&Device`/`&Queue` and now reserves the pass's slot), and
  `Blur` (whose two A/B buffers became an arena, so repeated `blur_backdrop`
  calls in one submission keep their own radius/tint — verified by a new
  `tests/blur.rs` case). A forgotten `begin_frame` is loud, not silent: arenas
  grow, reallocations trip the pressure warning, and a one-shot warning fires the
  first time one frame's arena crosses 64 MiB.
  Permanent regression tests in `tests/multi_pass_render.rs` (two renders per
  submission across every primitive family; `render_layers` + `render`; a second
  pass with a *different viewport* drawing with its own projection; a second
  identical frame reallocating nothing), the two-`blur_backdrop` case in
  `tests/blur.rs`, and three arena unit tests in `uniform_arena.rs`. All in-tree
  callers (capture helpers, `hello_ui`, `benches/ui_stress`, every GPU test, the
  gallery's two submission groups) declare the boundary. The gallery PNG now
  shows the previously-missing base-layer fills (Rounded-rect cell, scroll-view
  row stripes, the menubar's open-label Accent fill).
  (Diagnosis kept for the record: `prepare_frame` reset the cursors on every call
  while `Queue::write_buffer` only landed at `submit`, so all passes wrote the
  same byte ranges and every recorded draw read the last write. It was visible in
  the gallery as missing *base-layer* `quad`/`rounded_rect` fills while
  popup-layer chrome rendered fine, and pre-existing at HEAD; the GPU suites
  never caught it because each rendered exactly one pass.)

---

## 2026-09-15 — "4a" default theme + the design-folder widget set

Source: the `UI System Default Look/` design folder (4a sheets). Two halves:
restyle everything that existed to the design language, then implement the
designed components we didn't have. Fantasy Theme sheet intentionally ignored.

### Theme retokened to the 4a design language

- [x] **Renderer: gradient chrome.** `ChromeInstance` grew `bg2` — the SDF
      fragment now mixes `bg → bg2` vertically across the rect — and
      `DrawList::chrome_rect_gradient(rect, radius, thickness, bg, bg2, border)`
      is the normal chrome entry point (`chrome_rect` = flat special case).
      Instance stride 6×vec4; fallback (rotated transforms) composites
      `rounded_rect` + `vertical_gradient`.
- [x] **New material tokens on `Theme`/`StyleKey`** (all resolver/overlay
      addressable, round-trip tested): `plinth`, `travel` (scalar),
      `face_top/_hover/_pressed`, `face_bottom/…`, `edge_highlight(+hover/
      _pressed)`, `inner_shadow`, `edge_shadow`, accent/danger face gradient
      tops+bottoms per state, `on_accent`, `on_danger`. Existing keys retuned:
      near-black neutrals (#10171c backdrop, #16191d panels), compact sizing
      (13px text, 24px controls, radius 1, padding 4), teal oklch(200°) accent
      reserved for state, redesigned severity hues, sunken wells
      (`input_background` @ 42% black), doc-tab colors.
- [x] **`src/widgets/material.rs` — the face-over-plinth vocabulary.** `Tone`
      (Default/Accent/Danger/Ghost/Sunken) × `Material` state; `draw` paints
      plinth + gradient face + 1px top highlight (pressed: face drops `travel`
      px and a short inner shadow replaces the highlight), returning the face
      rect for label centering. `draw_well(_simple)` paints the sunken input
      field (inset shadow + under-line + accent border when focused).
      `sheen_over` composites the translucent face tokens over the state base
      (`Button`/`ButtonHover`/`ButtonPressed`) so `StyleKey::Button` overlays
      still recolor buttons — the seam tests carry over unchanged in spirit.
- [x] **Existing widgets restyled through the material:** Button (+ new
      `.tone(Tone)` builder; press is geometric, label rides the face),
      ImageButton, Checkbox (accent face + on-accent tick when checked, sunken
      trough when not), RadioGroup (accent disc + dark dot selected), Slider
      (sunken track, accent-gradient fill, light rectangular key knob),
      TextInput/NumberInput (via the well), Dropdown (trigger = well; open
      list = raised sheet), Tabs & menu sheets (held-key active tab, translucent
      accent row highlight), ScrollView bars (docked plinth track + material
      thumb), ProgressBar (sunken track + gradient fill + highlight).
- [x] **Fonts:** design leans on IBM Plex Sans/Mono from Google Fonts — *not*
      bundled. Kept Noto Sans as the default (`bundled-font`); theming to Plex
      is `theme.font = Some(load_font_file(..)?)`. Flagged for Bart.

### New widgets from the design sheets

All under `src/widgets/`, exported from the crate root, unit-tested headless,
and rendered in new `4a:*` gallery sections.

- [x] **Toggle** (`toggle.rs`) — 28×15 slide switch; accent face when on, sunken
      trough when off; `.focusable()` Space/Enter.
- [x] **Badge / keycap / chip** (`badge.rs`) — free functions: `badge` (tinted
      status pill), `keycap` (raised key cap with side line), `chip`
      (toggleable filter pill: raised at rest, held-in accent when on).
- [x] **Breadcrumb** (`breadcrumb.rs`) — clickable path trail; final segment is
      the non-clickable current location.
- [x] **Pager** (`breadcrumb.rs`) — `◀ n / total ▶` strip with clamping arrows,
      or `.numeric()` page keys for small totals.
- [x] **Status bar** (`status_bar.rs`) — 26px strip; first cell stretches,
      hairline dividers between cells, `StatusCell::text/spacer/highlight`.
- [x] **Splitter** (`splitter.rs`) — pane divider (vertical/horizontal) through
      `DragCapture`; reports `drag_delta` along the axis while owned.
- [x] **Tag input** (`tag_input.rs`) — well with removable chips + inline draft
      field; Enter commits, ✕ reports removal; all state caller-owned.
- [x] **Combo box** (`combo_box.rs`) — `draw_combo_trigger` (well + chevron key,
      editable query, Enter submits) + `draw_combo_list` (raised option sheet
      with accent match-highlighting), for popup-layer composition like
      `Dropdown`.
- [x] **Vector field** (`vector_field.rs`) — labeled XYZ rows; colored axis-tag
      scrubbing through one shared `DragCapture`; reports `(row, component,
      value)` deltas.
- [x] **Document tabs** (`doc_tabs.rs`) — editor tabs: active flush, inactive
      dropped 2px, dirty dot swaps to a ✕ key on hover.
- [x] **Asset grid** (`asset_grid.rs`) — thumbnail plate grid; selection =
      accent wash + inset ring; click reports the index.
- [x] **Busy states** (`busy.rs`) — `skeleton` (shimmer band, app-owned phase),
      `spinner` (accent arc on a ring), `dots` (pulsing trio),
      `empty_state` (glyph/title/body block returning its extent for a CTA).
- [x] **Gradient ramp** (`gradient_ramp.rs`) — 64-sample interpolated bar with
      draggable diamond stops (order-agnostic; `sample()` + `RampOutput`
      insert/drag reporting are pure).
- [x] **Curve editor** (`curve_editor.rs`) — sunken plot, 4×4 grid, filled
      curve region, draggable keys with neighbor-clamped x, empty-click insert.
- [x] **Popover** (`popover.rs`) — anchored arrow sheet; `place_popover` keeps
      the body inside bounds; `Popover::draw` reports close (✕ or outside
      click).

### Not ported (by design)

- Fantasy Theme sheet (explicitly out of scope).
- Backdrop-blur sheets: the popover/menu *looks* work today; the blurred-glass
  material itself is already available to apps via `UiRenderer::blur_backdrop`
  (render-side, not a widget concern).
- Palette 255-swatch panel: needs a texture/UV story for swatch cells that
  doesn't fit any existing widget; deferred until the editor needs it.

## 2026-09-15 (later) — shadow audit: curve-fill fix, deeper inset shadows, drop shadows

Bart asked whether the inner/outer shadows were missing and noticed the curve
editor's under-curve region was blank. Audit against the 4a design sheets found
three gaps, all closed:

- **Curve fill was triangulated wrong** (`curve_editor.rs`): the old two
  triangles per segment painted slivers beside the chord instead of the area
  under it. Now each segment emits the under-chord trapezoid
  (`a, b, b↓, a↓` split on the `b → a↓` diagonal) via the new
  `DrawList::triangle_gradient`, with the design's vertical accent fade
  (alpha 0.30 at the curve → 0.05 at the bottom). Key handles cast the
  design's `0 1px 3px` mini shadow.
- **Inset shadows were 2px stubs**; the design specifies `inset 0 2px 4px`.
  New `Theme::inner_shadow_depth` scalar (`StyleKey::InnerShadowDepth`,
  default 6) + shared `material::draw_inset_shadow(list, style, rect, depth,
  inset)` helper now used by wells, slider track, progress track, checkbox
  well, toggle off-state, chips, and the curve/ramp wells.
- **Outer drop shadows were entirely absent.** New `DrawList::drop_shadow(
  rect, offset_y, blur, radius, color)` primitive (five butt-joined gradient
  rects; SDF-instanced under translation) applied at half-CSS-blur falloffs
  from the design: tooltip `0 6px 18px`, dropdown list `0 10px 26px`,
  toast `0 12px 30px`, popover `0 14px 44px`.

Supporting changes: `DrawList::triangle_gradient` (per-corner colors),
`Vertex` now derives `PartialEq` (tests), the debug linter no longer flags
zero-alpha gradient skirts (`bg2` counts as paint) and exempts `Layer` nodes
from `overflows_declared` (a layer's rect is input-blocking bounds, not a
paint contract — popovers cast shadows past it by design). Gallery render
eyeballed: curve fill, floating sheets, and well depth all match the 4a
sheets; only the 5 pre-existing intentional `sibling_overlap` demo warnings
remain.

## 2026-09-16 — Toolbar, dock panels, and application shell

Design: `UI System Default Look/` mockups (4a Level Editor, 4a Menu Bar sheets).
Plan: `docs/design/menubar.md` covers the menubar; the toolbar/dock/shell are
new systems designed to compose with it and the existing widgets.

### New widgets

- [x] **Toolbar** (`src/widgets/toolbar.rs`) — edge-dockable strip of tool
      buttons with grip handle, group separators, and active-tool accent
      highlighting. `Toolbar::new(items).draw(rect, &mut state, &mut capture,
      grip_id, &mut ctx) -> ToolbarOutput`. Tool buttons use the plinth+face
      material (`Tone::Ghost` idle, `Tone::Accent` active). Icon via `Icon`
      widget (PhosphorIcon or custom font). Grip via `DragCapture` protocol
      (same as `Splitter`/`DragHandle`). Hovered tool reported in output for
      external `TooltipLayer`. No popup, no frame-deferred state. State is
      caller-owned `ToolbarState { edge, active_tool }`. 7 unit tests.
      API: `ToolbarEdge` (`Left`/`Right`/`Top`/`Bottom`), `ToolDef`, `ToolbarItem`
      (`Tool`/`Separator`), `ToolbarState`, `Toolbar`, `ToolbarOutput`.

- [x] **Dock panel** (`src/widgets/dock_panel.rs`) — tabbed, closable panel
      chrome for left/right/bottom docks. `DockPanel::new(side, tabs).closable()
      .draw(rect, &mut state, &mut ctx) -> DockPanelOutput`. Draws tab header
      (active = gradient bg + bright text, inactive = transparent + dim text) and
      panel background; returns the body rect for the caller to draw content into.
      Close button is a ghost-style `×`. The resize splitter is *not* internal —
      it's a layout concern drawn by `AppShell` or the caller. State is
      caller-owned `DockPanelState { visible, size, min_size, max_size,
      active_tab }`. 7 unit tests.
      API: `DockSide` (`Left`/`Right`/`Bottom`), `DockTab`, `DockPanelState`,
      `DockPanel`, `DockPanelOutput`.

- [x] **Application shell** (`src/widgets/app_shell.rs`) — layout composition
      that arranges menu bar + doc tabs + left/right/bottom docks with splitters +
      toolbar + viewport + status bar. `AppShell::new().with_menu_bar()
      .with_toolbar().layout(screen, docks, toolbar, &styles) -> ShellLayout`
      (pure geometry, no drawing). `draw_chrome(layout, docks, toolbar, &mut
      capture, &mut ctx) -> ShellChromeOutput` draws splitters, dock headers, and
      toolbar; applies splitter deltas with clamping. Well-known `DragId`
      constants (`SHELL_DRAG_LEFT_SPLITTER` etc.) at `0xA551_*`. 6 unit tests
      (all-zones-visible, hidden-dock-expands-viewport, no-overlap, toolbar-edge,
      heights, empty-shell).
      API: `AppShell`, `ShellLayout`, `ShellChromeOutput`, `SHELL_DRAG_*`.

### New `StyleKey` scalars

`ToolbarButtonSize` (24), `ToolbarPadding` (2), `DockTabHeight` (24),
`DockSplitterWidth` (6) — all with `Theme` fields, `get`/`set` arms, and
defaults. No new color keys (reuses existing palette: Panel, PanelBorder,
TabActive, TabInactive, Accent, Text, TextDim, etc.).

## 2026-09-16 — Menu-screen primitives (scrim, MenuList, arrow focus, tile fit)

From the "easy primitives for a main menu" request: thin composition over the
existing Button/Panel/Image/Anchor pieces rather than a monolithic menu
widget — a MenuScreen façade stays deferred until a real consumer wants one.

### New pieces

- [x] **Fullscreen scrim** (`draw_scrim` in `src/widgets/menu_screens.rs`) —
      one quad filled with the new `StyleKey::Scrim` color (theme default
      black @ 0.55), drawn between the game world/backdrop and the menu
      contents to push the scene back. Works on any layer; pair with
      `UiRenderer::blur_backdrop` for frosted glass. API:
      `draw_scrim(rect, &mut DrawList, &StyleResolver)`.
- [x] **MenuList** (`src/widgets/menu_screens.rs`) — a column of equal-width
      menu buttons, vertically centered as a block; row width derives from the
      widest label (≥3× label width, clamped to the rect). Caller owns
      `selected: &mut usize` (written in place, clamped); nav up/down wraps via
      `rem_euclid`, hover promotes the selection, and confirm activates it —
      but never on the same frame the selection moved, and never for a
      disabled row. Hover is resolved in a pre-pass before any row paints so
      every row's chrome reflects the same selection (a mid-loop mutation
      painted stale selection above the pointer for a frame). API:
      `MenuList::new(&[&str]).row_height(px).gap(px).enabled(&[bool])
      .focusable(base_id).draw(rect, selected, &mut DrawContext) ->
      MenuListOutput { activated: Option<usize>, hovered: Option<usize>,
      selected_changed: bool, intrinsic_width: f32 }`. 13 unit tests.
- [x] **Arrow focus nav** (`ArrowFocusNav` in `src/nav.rs`) — opt-in wrapper
      over any `NavMap` that turns the directional intents (arrows, d-pad,
      stick) into Tab-ring next/prev (up/left = prev, down/right = next) and
      consumes them, so a menu needs no extra wiring for gamepad/keyboard
      selection; `confirm`/`cancel` pass through. Off by default: sliders and
      text carets still own the arrows wherever the wrapper isn't applied.
      API: `ArrowFocusNav::over(inner_map)`. 4 unit tests.
- [x] **`ImageFit::Tile`** (`src/widgets/image.rs`, `DrawList::image_tiled`,
      shader wrap in `src/render/ui.wgsl`) — repeat an image at natural pixel
      size edge-to-edge across any box (tiled menu backdrop), partial tiles
      cropped at the right/bottom. Repetition is a fragment-shader
      region-relative `fract`, so a fullscreen backdrop costs **one** icon
      instance — no per-tile instance explosion. `IconInstance.flags` gained
      `[tile_wrap, tile_span_u, tile_span_v]` (the one-tile UV span, computed
      from the atlas region since `apply_crop_uv` already maps the full
      multi-tile span into atlas space). Requires a `SpriteId` source +
      `natural_size` (key sources fall back to Stretch); keep an opaque margin
      in the art for seamless tiling (atlas neighbors are never sampled, but
      the region's own halo edge is). 3 unit tests + `menu_gallery` eyeball.
- [x] **`tests/menu_gallery.rs`** — GPU render test (separate page, keeping
      `widget_gallery.rs` from growing) showing tiled backdrop, scrim +
      MenuList with pointer/selection, `Anchor::BottomRight` composition, and
      a centered title + menu column. Render with
      `DISPLAY=:0 cargo test --test menu_gallery -- --ignored --nocapture`.

### New `StyleKey` colors

`Scrim` — `Theme.scrim`, default `[0.0, 0.0, 0.0, 0.55]`, with `get`/`set`
arms and a `COLOR_KEYS` entry.

## 2026-09-16 — syntax-highlight fidelity: glyph byte mapping + dotted-capture resolution

Two bugs made Tree-sitter highlighting visibly wrong in multiline Lua (Bart:
"the highlighting seems to be mismatched to the text… not entirely random"):

- **Glyphs carried line-relative bytes.** cosmic-text's `LayoutGlyph.start` is
  relative to the glyph's *buffer line*, but the horizontal shaping branch of
  `build_vertices` used it raw, so `ShapedGlyph::byte_start` on every line after
  the first pointed back into earlier lines' bytes — style ranges resolved
  against the wrong text (line-1-shaped colors on every later line). Caret and
  selection looked correct because `text_caret_layout`/`text_visual_layout` and
  the vertical branch already rebased. `build_vertices` now adds the buffer
  line's start byte in both modes (direction prefix subtracted exactly once
  from the absolute byte). Regression: three GPU-gated `style_ranges_*` tests
  in `src/text.rs` (hard newlines, direction prefix, soft wrap) — the gap was
  that all prior `build_vertices` tests were `#[ignore]` and single-line.
- **Dotted captures resolved to the wrong category.** `configure` breaks
  equal-length matches by list order, so `keyword.function` matched both
  `keyword` and `function`; `function` being listed first stole the keyword.
  `CAPTURE_NAMES` is now ordered base-categories-before-their-dotted-modifiers
  (`keyword` < `function`, `variable` < `parameter`, …), pinned by
  `category_precedes_its_dotted_modifiers`. Also added `conditional`/`repeat`
  categories (defaulting to the keyword color) so Lua `if`/`then` and
  `while`/`for` keywords are no longer unstyled; the Lua test now asserts
  `function` (the keyword) colors as a keyword and `greet` as a function name.
  `examples/lua_highlight_probe.rs` (`--features syntax-lua`) renders a Lua
  snippet headless to `test_output/lua_highlight_probe.png` for eyeballing.

## 2026-09-16 — Settings form, bindings, and control mapping

"Define a bunch of settings and have that automatically transformed to a
settings configurator" — declarative form over caller-owned values, plus
control rebinding as just another field kind. Chosen by Bart: one spec with
bindings included, homogeneous values slice, and a pragmatic `Binding` enum
over what `InputState` already expresses (no input-layer changes).

### New pieces

- [x] **Declarative settings form** (`src/widgets/settings.rs`) — build a
      `SettingsSpec` (`section` / `toggle` / `slider` / `choice` / `binding` /
      `action`), keep a parallel `Vec<SettingValue>` the caller owns, and
      `SettingsForm::new(&spec).draw(&mut values, rect, &mut state, &mut ctx)`
      draws label-left/control-right rows and mutates values in place.
      `SettingsFormOutput { changed: Vec<usize>, activated, listening }`
      reports field indices. Sections draw as `Group` panels (or
      `.bare_headers()` title text); the label column auto-sizes to the widest
      label per section (`.label_fraction()` to pin); `intrinsic_height()`
      sizes a scroll viewport; every row joins the Tab focus ring. Choice rows
      open a `Dropdown` popup: drive `state.dropdowns.begin_frame` /
      `form.push_open_layer` / `layers.input_for_base` / ... /
      `form.draw_open_layer(...)` (applies the pick, returns the changed
      field) / `state.dropdowns.end_frame`. `default_values(&spec, &[(idx,
      value)])` builds the slice, dropping mistyped defaults so it can't
      desync. 13 unit tests.
      API: `SettingsSpec`, `SettingField`, `SettingValue` (+ `From`), `SettingsFormState`,
      `SettingsFormOutput`, `SettingsForm`, `default_values`.
- [x] **Bindings / control mapping** (`src/widgets/binding.rs`) — `Binding`
      enum: `Key(KeyCode)` for the named keys `InputState` carries as fields
      (Esc/Tab/Space/Enter/Bksp/arrows/Home/End/Del), `Char { c, ctrl, shift,
      alt }` for typed characters + modifier chords, `MouseLeft/Middle/Right`,
      `Pad(PadButton)` over the `GamepadNav` fields. `label()` renders
      "Ctrl+Shift+S" / "←" / "Mouse2" / "D-Up"; `is_down(input, pad)` matches
      edges and held state. The form's `binding` row shows the current
      binding as a button; clicking arms it (`state.listening`), the arm
      frame is grace-protected from capturing itself, Escape cancels, and the
      next key/char/mouse/pad press (via `state.pad`, the same snapshot the
      host feeds `map_gamepad`) becomes the new value. Conflict resolution
      (unbind the row that had it) is caller policy on `out.changed`. 7 unit
      tests.
      API: `Binding`, `KeyCode`, `PadButton`.

### Gallery

`tests/menu_gallery.rs` grew a `SettingsForm` section (checked toggle, slider
at 0.7, Medium dropdown, Space binding, Reset action) — canvas 640×980.
