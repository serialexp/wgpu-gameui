# Contextual Widget Lifecycle — Design

Status: partial — first text/button and measured-stack slice implemented; broader widgets outstanding
Owner: Bart
Last updated: 2026-09-02

## Implementation status

Tracking the gap between this design and the main branch.

### Done

- [x] **Preserve immediate responses.** Widget calls continue to return clicks and edited values at the call site; no deferred frame plan is introduced.
- [x] **Keep application and script callbacks one-shot.** Arrangement consumes plain measurements and rectangles, never retained application/Lua callbacks.
- [x] **Choose single-shot widget measurement.** A public widget measurement operation runs once and returns immutable prepared data; containers do not call it again.
- [x] **Keep logical pixels in the first slice.** Scale is carried as measurement context, while physical-pixel snapping is deferred.
- [x] **Phase 0 — measurement foundation.** Added normalized constraints, intrinsic results, structured text metrics, a narrow measurement context, and contract tests.
- [x] **Phase 1 — first measurable leaves.** Text and button sizing use contextual, font-aware measurement; prepared text transfers its configured block into paint.
- [x] **Phase 2 — measured stacks.** Plain measured children arrange through reusable H/V stack scratch with baseline alignment and explicit width-mismatch errors.
- [x] **Phase 3 — one-shot lifecycle integration.** Tests prove measurement emits no paint and each arranged application draw body runs once.

### Outstanding

- [ ] **Phase 4 — broader widgets and scrolling.** Migrate remaining leaves, derive same-frame scroll extents, and clamp caller-owned scrolling after arrangement.
- [ ] **Phase 5 — diagnostics and semantics.** Report intrinsic/measured/allocated/painted geometry and later emit optional semantic/accessibility data from arranged geometry.

## Why this exists

Gameui already has a geometry-only layout engine, retained topmost interaction geometry, ordered paint commands, and a one-shot `UiContext::draw_in_rect` bridge. It lacks one contract joining these pieces. Widget sizes are currently a mixture of caller guesses, theme constants, tuple-returning helpers, and default-font text measurements.

GPUI's useful lesson is its explicit `request_layout → prepaint → paint` lifecycle, not its retained entity/application runtime. Gameui adapts that lesson as `measure → arrange → interaction/paint`, while preserving caller-owned state, immediate responses, direct wgpu rendering, and binding-neutral data.

## Goals

- Measure widgets under resolved font/style, available logical space, scale, and wrapping policy.
- Return minimum, preferred, maximum, baseline, and width-dependency information.
- Keep arranged rectangles authoritative for debug declarations, hit geometry, and paint.
- Permit each widget's public measurement operation once and each draw callback once.
- Keep steady-state measure/arrange loops allocation-free through caller-owned flat scratch.
- Make unsupported width-dependent arrangements explicit rather than silently using stale measurements.

## Non-goals for the first slice

- A retained or type-erased element tree.
- Deferred keyed interaction responses.
- Replacing the existing raw widgets or geometry-only layout APIs.
- Automatic arbitrary nested layout.
- Replaying Rust, application, or Lua callbacks for measurement.
- Physical-pixel edge snapping.
- Repaint scheduling, keyed layers, or accessibility platform integration.

## Lifecycle contract

### Measurement

Measurement is pure with respect to input, focus, animation, interaction registration, caller model state, and `DrawList` paint buffers. The caller supplies final constraints for the public measurement operation. A returned prepared result records the width it was measured for when its height depends on width.

Containers may freely inspect and copy the resulting plain metrics, but must not invoke widget measurement again. If arrangement assigns a width incompatible with a width-sensitive prepared result, it returns a diagnostic error. A caller then measures once with the width that its parent already knows—for example, a stretching child in a vertical stack is measured against that stack's inner width.

### Arrangement

Arrangement consumes plain `MeasuredChild` values and writes `LayoutItem`s into caller-owned `LayoutResult` scratch. It does not own widgets, strings, callbacks, input, or rendering resources. Existing fixed/fit/fill/percent, constraints, weights, justification, alignment, and stable IDs remain the placement vocabulary.

A baseline is an offset down from the measurement box's top edge. Horizontal baseline alignment selects the greatest baseline among participating children and translates each baseline child to that shared line. Children without a baseline use their requested ordinary cross-axis alignment.

### Interaction and paint

The arranged `Rect` is passed into `draw_in_rect` or a raw rect-native widget call. That same allocation drives the debug declaration and `DrawContext::interact`, including active transform, clip, and layer. Interaction continues to resolve against the previous completed frame so later paint submissions can win without a second application pass.

## Measurement model

Constraints use nonnegative logical pixels and optional upper bounds; absent means unbounded. Invalid/negative values normalize rather than reaching layout arithmetic. Results satisfy `min ≤ preferred ≤ max` on every bounded axis.

Structured text metrics include:

- advance/line-box size reserved by layout;
- optional ink bounds;
- first baseline and visual-center offset from the block top;
- line count;
- whether unwrapped content overflowed the supplied width;
- the width used for a width-sensitive prepared layout.

Intrinsic/unwrapped and constrained/truncated measurements have distinct cache identities. A truncated probe must never answer a later intrinsic query.

## Performance model

- Measurement contexts borrow the existing `TextMeasurer`; no renderer, device, queue, or GPU state enters layout.
- `MeasureBuffer` and `LayoutResult` retain flat `Vec` capacity across frames.
- Container loops allocate no strings, boxes, nested vectors, or trait objects.
- Text cache hits do not allocate; bounded cache misses may own cache entries.
- Prepared text owns its already-configured block and is consumed into paint rather than cloned per item.

## Phasing

### Phase 0: foundation

Introduce the cohesive measurement module, structured text metrics, context construction, normalization tests, and allocation-reuse tests.

### Phase 1: text and button

Measure configured text once into a prepared result. Add contextual button measurement using the exact style/font path used by paint. Existing convenience sizing delegates where that does not duplicate policy.

### Phase 2: measured stacks

Add reusable measured child storage and H/V arrangement. A vertical stack supports constrained wrapping when children were prepared for its known inner width. A horizontal stack supports baseline alignment and rejects width-sensitive children whose allocated width differs from their prepared width.

### Phase 3 and later

Wire one-shot end-to-end examples, migrate the rest of the widget set, then tackle measured scrolling, text-layout reuse inside the renderer, richer diagnostics, and optional semantic output.

## References

- `TODO.md` contextual measurement and font-aware layout P1 items.
- `src/layout.rs`, `src/ui_context.rs`, `src/interaction.rs`, `src/text.rs`.
- GPUI-CE `Element`, Taffy measured leaves, text layout, and `Div` prepaint at commit `74b3a785`.
