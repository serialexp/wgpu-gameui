# GPU Chrome and Shadows — Design

Status: implemented; verification complete
Owner: Bart
Last updated: 2026-09-21

## Implementation status

Tracking the gap between this design and the main branch. A visible phase is not
done until its focused GPU tests pass, the affected widget-gallery rows have
been regenerated and inspected, and the old approximation is no longer used by
the migrated widgets.

### Done

- [x] **Phase 0 — reference fixtures and budgets.** Checked-in Chromium 153 captures cover 24 cases over transparent/black/white backdrops at DPR 1/1.5/2. Shadow build/render benchmarks cover contiguous and alternating 100/1,000/10,000-instance cases; measured results are recorded below.
- [x] **Phase 1 — instanced analytic shadows.** `BoxShadow` and `CornerRadii`, retained/coalesced draw-list payloads, full-affine instances, reusable GPU arenas, analytic Gaussian and zero-sigma shader paths, clipping, inset/outset spread, diagnostics, shared color passes, and CPU/GPU tests are implemented.
- [x] **Phase 2 — composable quad primitives.** `Background`, `EdgeWidths`, `EdgeStyle`, and `QuadStyle` are fixed-size values; the full-affine retained chrome path supports axis gradients, asymmetric radii, unequal rounded borders, tint/clip, staged background/border painting, and structural edge lines.
- [x] **Phase 3 — typed component chrome and surface painter.** `Theme::chrome` contains finite `Copy` component materials, `StyleOverlay` and `StyleResolver` provide O(1) typed overrides, and `SurfacePainter` owns reverse-stacked outset/background/inset and post-content border/line ordering with explicit padding geometry.
- [x] **Phase 4 — widget migration.** Splitter/status, toolbar/dock, menubar/context menus, dropdown/popover/tooltip/toast, and curve-editor key surfaces resolve typed chrome and use composable quads/edges plus analytic shadows. No production `drop_shadow` or `rounded_rect_glow` call remains.
- [x] **Phase 5 — visual and performance acceptance.** All 72 Chromium case/DPR comparisons pass their predeclared gates, deterministic GPU/structural performance suites pass, and the regenerated widget/menu galleries were inspected. Chrome and shadows share one ordered tagged instance stream, so even strict alternation uploads once and renders in one instanced draw while preserving source-over order; warmed draw-list rebuilds allocate nothing.
- [x] **Phase 6 — documentation and status.** Fixture formats and tolerances, browser-model differences, representative benchmark results, public APIs, and verification are recorded here and in README/TODO.

### Deferred

- [ ] **Text shadows.** Glyph shadows require text-renderer coverage rather than rounded-rectangle coverage; omit them instead of approximating them with this pipeline.

## Why this exists

The library's default chrome is being brought in line with the Forge design
system (claude.ai/design project `1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`, the
authoritative source since 2026-09-24; the old `design_handoff_forge_chrome/`
folder was removed). The existing renderer can already draw good
solid and gradient rounded rectangles, but ordinary widget code assembles
surfaces from low-level calls and scatters authored colors across modules. A
single palette change therefore requires editing several widgets, and visually
similar values are sometimes coupled accidentally while intentional component
variants are hard-coded.

Shadows are a more fundamental mismatch. `DrawList::drop_shadow` currently
builds a shadow from an opaque core, four linear-gradient skirts, and bilinear
corner patches. It fixes the skirt's edge opacity at 35%, treats vertical offset
as unequal skirt widths rather than an offset blurred source, has no spread or
inset mode, and cannot reproduce a Gaussian rounded-rectangle convolution.
`rounded_rect_glow` improves symmetry with twelve concentric contours, but its
peak is manually attenuated and the layers remain visible on demanding shapes.
Those approximations cannot faithfully represent authored values such as:

```css
box-shadow:
    0 16px 40px rgba(0, 0, 0, 0.7),
    0 2px 6px rgba(0, 0, 0, 0.5),
    inset 0 1px 0 rgba(255, 255, 255, 0.12),
    inset 0 -1px 0 rgba(0, 0, 0, 0.5);
```

The problem is made worse by inconsistent call-site interpretation. Some
widgets pass the HTML blur number to `drop_shadow`; others pass half of it as a
hand-estimated falloff margin. The resulting API does not have one physical
meaning.

This document defines two related contracts:

1. a small GPUI-inspired set of composable surface primitives backed by typed,
   centralized theme values; and
2. a first-class instanced GPU shadow primitive whose input maps mechanically
   from authored box shadows and whose output has a smooth analytic falloff.

## Goals

- Map an authored shadow's x/y offset, blur, spread, color, inset flag, and
  source corner radii into one fixed-size value without widget-side conversion.
- Reproduce smooth rounded-rectangle outset shadows, inset shadows, contact
  shadows, and colored glows with consistent all-sides falloff.
- Preserve CSS multi-shadow stacking semantics (first declared is topmost) and
  the required ordering relative to surface, content, and border paint.
- Batch contiguous shadows as instanced GPU work and reuse all CPU/GPU buffer
  capacity across frames.
- Keep structural opaque lines distinct from genuinely translucent blur.
- Centralize the default component materials in `Theme`; changing a material
  should not require editing widget paint code.
- Preserve scoped style overrides without cloning a theme or allocating while
  resolving a widget's material.
- Provide focused headless tests, browser-reference captures, gallery coverage,
  and a representative-scale performance benchmark.
- Keep the public API useful to custom widgets rather than making it specific to
  the current design handoff.

## Non-goals (v1)

- A complete CSS parser, CSS cascade, retained style tree, or DOM compositor.
- Arbitrary gradient stop counts. The existing design needs solid fills and
  two-stop linear gradients.
- Backdrop blur. The handoff explicitly permits removing it because the sheet
  surfaces are effectively opaque.
- Text shadows. Their mask is shaped glyph coverage, not a rounded rectangle.
- Automatically turning every 1px highlight into a blurred inset shadow.
  Resolved structural highlights and counter-edges remain explicit opaque lines.
- Unbounded shadow lists owned by each widget or allocated each frame.
- Pixel-identical output across all browser engines. Acceptance is against
  checked-in reference fixtures and documented tolerances.
- Replacing nine-slice art. Image-authored frames remain image-authored frames.

## Lessons adopted from GPUI

GPUI separates the style vocabulary from the renderer's primitive vocabulary:

- a quad carries bounds, a solid or two-stop gradient background, border widths,
  border color, and corner radii;
- a box shadow carries offset, blur, spread, color, and an inset flag;
- the scene stores shadows as their own instanced primitive;
- style painting orders drop shadows before the background, inset shadows after
  it, contents next, and borders last;
- component-specific carved lines are composed from ordinary paint primitives
  rather than encoded in an all-purpose surface recipe.

The relevant reference implementation is in the local Zed checkout:

- `crates/gpui/src/style.rs` — `BoxShadow` value shape and style paint staging;
- `crates/gpui/src/scene.rs` — fixed-size `Shadow` scene primitive;
- `crates/gpui/src/window.rs` — inset/outset geometry construction;
- `crates/gpui_wgpu/src/shaders.wgsl` — analytic Gaussian rounded-rectangle
  shadow using an error-function approximation and four samples on the second
  axis.

GPUI is an algorithm and packing reference only. Its current implementation does
not reverse CSS shadow declarations, does not implement CSS outset-radius spread
correction, uses only linear inset radius subtraction, and rasterizes shadows
axis-aligned. This project adapts the analytic method to its full-affine
`DrawList`, CSS ordering/geometry contract, and sRGB-space RGBA renderer; it does not
copy GPUI's retained style/layout system or treat GPUI output as CSS parity.

## Primitive model

### Colors

All draw-list colors are straight **sRGB-encoded** RGBA — exactly what a CSS
hex value means — and the renderer blends and interpolates them in sRGB space
like a browser (2026-09-24 change; see `crate::color` and
`forge-token-audit.md`). Design hex / `oklch()` values are used as-is, with no
decode. Alpha remains meaningful for true
shadows and glows; ordinary surfaces and structural edge lines use opaque
values resolved from the DesignSync `tokens/colors.css`.

A public `Color` alias/newtype is outside this design's required scope. The
initial values may continue using `[f32; 4]`, provided conversions are no longer
repeated in widget modules.

### Backgrounds

The known surface combinations require only a solid fill and a two-stop linear
gradient:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    Solid([f32; 4]),
    LinearGradient {
        start: [f32; 4],
        end: [f32; 4],
        axis: GradientAxis,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientAxis {
    Horizontal,
    Vertical,
}
```

Arbitrary angles are already available through lower-level draw-list methods,
but are not required in every themed quad value.

### Edges and corners

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeWidths {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeStyle {
    pub thickness: f32,
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadStyle {
    pub background: Background,
    pub border_widths: EdgeWidths,
    pub border_color: [f32; 4],
    pub corner_radii: CornerRadii,
}
```

`DrawList::paint_quad(rect, style)` chooses the existing single-instance chrome
path when border widths and radii fit it. A bounded set of instanced rectangles
is a correct fallback only when the radii are zero, or when the border is
uniform and can use existing rounded chrome. Unequal edge widths meeting rounded
corners cannot be reproduced by rectangular bands; those combinations require
an extended quad shader or an explicit unsupported-combination diagnostic. The
API must never silently flatten or overlap the corner arcs. All supported paths
remain allocation-free; profiling can later justify a wider chrome instance
layout.

A narrow `edge_line(rect, edge, thickness, color)` helper expresses highlights,
separator counter-edges, and dock rules without repeating coordinate arithmetic.
It is composition sugar over `quad`, not a new renderer pipeline.

### Shadows

The public value is authored in box-shadow terms:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset: [f32; 2],
    /// Authored CSS-compatible blur radius. Callers do not convert this to sigma.
    pub blur: f32,
    pub spread: f32,
    pub color: [f32; 4],
    pub inset: bool,
}
```

The draw entry points are rect-native. Inset geometry is explicit because CSS
clips inner shadows to the padding edge, not generically to the border box:

```rust
pub fn box_shadow_outset(
    &mut self,
    border_box: Rect,
    border_radii: CornerRadii,
    shadow: BoxShadow,
);

pub fn box_shadow_inset(
    &mut self,
    padding_box: Rect,
    padding_radii: CornerRadii,
    shadow: BoxShadow,
);

pub fn box_shadows_outset(
    &mut self,
    border_box: Rect,
    border_radii: CornerRadii,
    declarations: &[BoxShadow],
);

pub fn box_shadows_inset(
    &mut self,
    padding_box: Rect,
    padding_radii: CornerRadii,
    declarations: &[BoxShadow],
);
```

The group helpers borrow storage and do not retain or clone the slice. Fixed
arrays in component themes coerce directly to `&[BoxShadow]`, so no custom
small-vector container is needed. The low-level single-shadow call paints
immediately; the group helpers filter by kind and reverse declaration order as
defined under Paint ordering.

Invalid inputs are normalized at this boundary:

- non-finite values reject the primitive and increment the draw list's
  degenerate/diagnostic count;
- blur is clamped to nonnegative;
- radii are clamped to the source rectangle;
- outset spread dilates by `spread`; when negative spread collapses its adjusted
  source, the primitive is skipped and counted as degenerate;
- inset spread dilates the hole by `-spread`; when positive spread collapses the
  hole, it is retained as a centered zero-area hole with an explicit collapsed
  flag so both blurred and zero-blur branches cover the whole padding box rather
  than leaving a pinhole or making the shadow disappear;
- zero-alpha and empty element bounds are intentional no-ops and do not increment
  the degenerate count; malformed or geometrically collapsed outset inputs do;
- zero blur is a crisp offset/spread shadow with derivative-based antialiased
  shape edges, not a dropped command and not an `erf` evaluation at sigma zero.

## Authored blur semantics

The public `blur` field means the authored CSS `box-shadow` blur length. Widget
code copies the design value exactly. It must never divide by two, choose a
falloff margin, or supply shader sigma.

The authoritative model is the ideal Gaussian whose standard deviation is half
of the authored blur radius. The renderer therefore uses `sigma = blur * 0.5`;
GPUI's shader is an algorithm reference, not a semantic reference, because its
field named `blur_radius` is consumed directly as sigma. Chromium 153's blur=2
capture at DPR 1 has a measurably more compact, discretized straight-edge profile.
The parity harness still reports that fixture but classifies its mass mismatch as
`browser-model-difference:compact-blur2`; an independent 65,536-sample Gaussian
oracle and direct exterior-edge fixture probes gate that classification. It is
not a per-case tolerance increase and does not change renderer sigma or introduce
a discrete-kernel branch. Phase 0 verifies conversion and edge behavior rather
than fitting an unknown kernel. It renders a browser matrix containing:

- blur 0, 2, 6, 8, 18, 26, 40, and 44px;
- offsets on both axes, including negative offsets;
- spread -2, 0, and +4px, including inset-hole collapse;
- square, 1px-radius, 12px-radius, and asymmetric-corner sources;
- thin sources such as the splitter's 2×26px grip at both small and 40/44px blur;
- outset, inset, black shadow, cyan glow, and mixed-color multi-shadow variants;
- device scale factors 1.0, 1.5, and 2.0.

Kernel calibration compares **alpha coverage**, not only composited RGB. Browser
fixtures are rendered over known black and white backdrops (or an equivalent
transparent-mask fixture) so coverage can be recovered independently of color
space. The same matrix is rendered by the candidate shader. A single
renderer-owned, unit-tested conversion supplies sigma; the public API and themes
retain authored blur. Widget code cannot override it.

The renderer composites in sRGB space, as Chromium does, so final RGB matches
the browser as well as the alpha mask. On a non-sRGB target it draws directly;
on an `*Srgb` (or float) target it draws into an offscreen layer and composites
it, so UI-over-UI still blends in sRGB and only translucent UI over the host's
own scene mixes in linear light (documented on `UiRenderer`). The fixtures'
black/white captures check the colour composite separately from the shape
(`shadow_colour_composites_in_srgb_like_chromium`). Sigma is never tuned to
compensate for colour space.

The shadow raster bounds extend to a documented Gaussian-tail cutoff (the GPUI
reference uses three sigma, which is 1.5 authored blur radii under the default
conversion). For an affine transform this screen-space disk is pulled back
through the inverse linear map; CPU geometry uses the resulting local ellipse's
axis-aligned extents. Bounds additionally receive the inverse image of a
one-screen-pixel square, so fractional-DPR boundary fragments exist even for
insets. That inflation is conservative only: centered SDF coverage remains the
authority. The cutoff is renderer policy and cannot be configured per widget.

## GPU representation and ordered paint

### Draw-list payload

`DrawList` gains a reusable `Vec<ShadowInstance>` and `PaintCmd::Shadow`:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowInstance {
    /// Offset/spread-adjusted shadow or inset-hole rect.
    pub shadow_rect: [f32; 4],
    /// Original element rect, required for inset clipping.
    pub element_rect: [f32; 4],
    pub color: [f32; 4],
    pub shadow_radii: [f32; 4],
    pub element_radii: [f32; 4],
    pub clip: [f32; 4],
    /// sigma, inset flag, clip-enabled flag, reserved.
    pub params: [f32; 4],
}
```

The exact packing may change during implementation to satisfy vertex attribute
limits or storage-buffer alignment. The selected v1 representation is full
affine: retain local element/shadow or inset-hole geometry and radii plus the
forward `Affine2`. The vertex shader expands the local raster quad, transforms
its four corners, and interpolates both local position (for convolution) and
world position (for clip testing). Non-finite or singular transforms are
rejected at record time. The semantic fields and fixed-size, plain-data property
are required.

As with chrome and circles:

- pending soup is flushed before a shadow instance is recorded;
- adjacent shadow commands coalesce into one instance range;
- interleaving a surface or text between shadows creates separate ordered runs;
- `DrawList::clear` empties lengths but retains capacities;
- renderer dynamic buffers grow geometrically and are reused;
- debug primitive counts and render statistics include shadows explicitly.

No shadow call allocates. Iterating a shadow slice pushes directly into retained
flat storage. There is no `Vec<Vec<_>>`, per-widget box, generated texture, or
per-shadow bind group.

### Renderer pipeline

`UiRenderer` gains:

- a shadow instance buffer in the existing per-frame arena/capacity model;
- a shadow render pipeline sharing the orthographic uniform;
- `vs_shadow`/`fs_shadow` entry points in `ui.wgsl`;
- ordered encoding for `PaintCmd::Shadow` ranges;
- accounting in `RenderStats`, arena sanity checks, and debug reports.

A contiguous shadow range is one instanced draw call. Two authored shadows on a
menu sheet are two instances in one range when nothing is painted between them.
Consecutive color-stage commands (`Soup`, `Chrome`, `Circle`, and `Shadow`) must
execute inside one active render pass while switching pipelines; the renderer
must not open a `LoadOp::Load` render pass for every command. Textured/text stages
may still define pass boundaries where their renderer integration requires it.
Phase 1 records render-pass count alongside draw-call count so tile-based GPU
cost is not hidden by otherwise-correct batching.

### Shader algorithm

The fragment shader adapts GPUI's analytic approximation while preserving this
renderer’s straight-alpha contract:

1. apply the active draw-list clip first, discarding before any Gaussian work;
2. for every rounded source/element clip, integrate pixel-area coverage with a
   fixed 4×4 screen-space grid pulled back through `dpdx`/`dpdy`; this remains
   stable for tiny rounded shapes and full affine transforms. For
   `sigma <= epsilon`, use that coverage directly and skip every division by
   sigma; identical crisp source and element evaluations therefore still cancel
   bit-for-bit;
3. otherwise analytically integrate the Gaussian along one axis with an
   error-function approximation;
4. use fixed four-sample quadrature on the other axis, selecting the appropriate
   corner radius for each sample's quadrant when radii are asymmetric;
5. for inset shadows, evaluate the complement of the blurred hole, explicitly
   return full coverage for a collapsed zero-blur hole, and clip the result to the
   rounded padding-box bounds;
6. for outset shadows, exclude the original element's rounded coverage so the
   shadow does not darken translucent element interiors or leak through
   antialiased corners;
7. compute `final_alpha = coverage * color.a`, discard zero coverage, and return
   `vec4(color.rgb, final_alpha)`. RGB is **not** multiplied by coverage because
   `wgpu::BlendState::ALPHA_BLENDING` performs straight-alpha source weighting.

Phase 0 compares GPUI's four-sample midpoint rule with four-point Gauss–Legendre
quadrature. The latter may replace it if the broad-blur/thin-source fixtures show
less ripple at the same fixed number of source evaluations. Sample count remains
independent of blur.

The expanded outset quad covers the adjusted shadow bounds plus the fixed
Gaussian tail and conservative boundary pad. Inset geometry is likewise a
conservatively inflated padding-box quad, while rounded padding-box fragment
coverage still clips authoritatively. Both are single base quads instanced from
fixed records. CPU recording rejects a shadow
whose expanded bounds do not intersect the active clip, while the fragment clip
remains authoritative at partial intersections.

The pipeline uses ordinary alpha blending for black shadows. The splitter glow
is authored as a translucent colored shadow and initially uses the same blend
mode, matching browser `box-shadow`. Additive blending is not silently selected
because the HTML reference uses source-over compositing. A future explicitly
additive glow would be a distinct paint value/pipeline decision.

## Inset, spread, and radii semantics

For an outset shadow, offset translates the source and spread dilates it before
blur. Positive spread expands all four sides; negative spread contracts them.
For an inset shadow, the adjusted rectangle is the blurred hole: offset moves the
hole, positive spread contracts it, and negative spread expands it. A collapsed
outset source emits nothing; a collapsed inset hole means full shadow coverage
inside the rounded padding box.

Corner adjustment follows CSS rather than blindly adding spread. For positive
outset dilation, and the corresponding inset-hole adjustment, use the CSS
spread-radius rule: when the original radius is smaller than the spread
magnitude, apply the nonlinear correction
`r + s * (1 + (r / s - 1)^3)`; otherwise use `r + s`, with signs adapted for
hole contraction and all results clamped nonnegative.

After spread adjustment, normalize corner overlap with one shared factor:
`min(1, width/(tl+tr), width/(bl+br), height/(tl+bl), height/(tr+br))`, ignoring
zero denominators, and multiply every radius by that factor. Independent
per-corner clamping is not CSS-compatible because it changes asymmetric corner
proportions. Phase 0 fixtures pin both spread signs, small/asymmetric radii, and
collapsed holes so implementation does not rely on visual guesswork.

Per-corner radii are part of both the quad and shadow model even though the
current handoff mostly uses a uniform 1px radius. Uniform-radius constructors
remain convenient and cheap.

## Transform and clipping contract

V1 supports the draw list's full finite, non-singular affine transform. Shadow
geometry, radii, and authored offset remain local, while the authored
`sigma = blur / 2` kernel is isotropic in browser/screen space. For affine linear
part `A`, the CPU pulls that kernel into local coordinates as covariance
`Σ = sigma² A⁻¹ A⁻ᵀ`. The shader factors this correlated Gaussian into the y
marginal `N(0, Σyy)` and x conditional
`N((Σxy/Σyy)y, Σxx - Σxy²/Σyy)`, retaining fixed four-sample quadrature and the
analytic x integral. The vertex shader transforms the conservatively expanded
local quad with `A`; rotation, reflection, non-uniform scale, and shear therefore
all produce the same circular screen-space kernel without dynamic fragment loops.

The active clip is captured and tested in world coordinates. Existing
`DrawList` clips transform local rectangles to world-space AABBs, so full-affine
shadows do **not** make rotated/sheared clip masks exact; that pre-existing clip
limitation remains explicit and outside this work. Singular/non-finite affines
are diagnosed and skipped. A CPU nine-patch fallback is not acceptable because
it recreates the artifact this pipeline exists to remove.

## Paint ordering

CSS declares the topmost shadow first, so each outset or inset group is painted
in **reverse declaration order**. A component surface follows this order:

1. outset shadows, reverse declaration order;
2. background fill;
3. inset shadows, reverse declaration order;
4. content;
5. border;
6. explicit structural edge lines whose design requires them above the border.

The low-level `box_shadow` call has ordinary immediate semantics. Separate
`box_shadows_outset` and `box_shadows_inset` helpers filter a borrowed slice and
reverse each matching declaration group without allocating. They deliberately
do not try to insert a background into already-recorded commands.

Phase 3 adds a typed component surface painter that owns the sequence above. It
can paint pre-content layers (outset, background, inset), return the content
rect, and paint post-content border/structural lines where a component needs
content between those stages. This named painter is the ordering authority for
component theme styles; direct callers retain immediate ordering.

This distinction prevents a `QuadStyle` from growing into an unbounded CSS
recipe. A menu sheet knows it has two outset shadows, a surface, two inset
hairlines, and a border; a splitter knows it has a track, three structural
lines, a grip, and one dragging glow. Those finite component combinations live
in typed theme structures.

## Theme and style-resolution model

The default handoff is the library's default theme, not a separately branded
palette. `Theme` gains one typed chrome group whose substructures match known
components:

```rust
pub struct ChromeTheme {
    pub menu_bar: MenuBarChrome,
    pub menu_sheet: MenuSheetChrome,
    pub toolbar: ToolbarChrome,
    pub dock: DockChrome,
    pub splitter: SplitterChrome,
    pub status_bar: StatusBarChrome,
}
```

Representative component values:

```rust
pub struct MenuSheetChrome {
    pub surface: QuadStyle,
    pub shadows: [BoxShadow; 2],
    pub top_highlight: EdgeStyle,
    pub bottom_shade: EdgeStyle,
}

pub struct SplitterChrome {
    pub track: Background,
    pub outer_edges: EdgeStyle,
    pub inner_highlight: EdgeStyle,
    pub grip_idle: [f32; 4],
    pub grip_hover: [f32; 4],
    pub grip_dragging: [f32; 4],
    pub dragging_glow: BoxShadow,
}
```

Each type contains only combinations that the component actually uses. There is
no generic structure with eight optional lines, an arbitrary vector of shadows,
and every possible decoration.

The default values come from the DesignSync tokens (written with
`color::hex` / `color::oklch`) at construction. Similar
colors are shared only when changing them together is intentional; accidental
equality does not make two semantic fields one token.

`StyleResolver` exposes these component styles through overlay-first typed
accessors. `StyleOverlay` gains optional `Copy` component-material overrides in
parallel with its existing scalar/color entries; large component variants are
not added to `StyleValue`, because that would inflate every ordinary overlay
entry. Resolution is an O(1) option lookup falling back to `Theme::chrome`.

This must satisfy:

- no theme clone for a scoped override;
- no hashing, linear key scan, or heap allocation in the per-widget component
  paint path;
- an override can replace a whole component material; independently useful
  semantic subsets receive explicit typed fields rather than generic maps;
- custom widgets retain the existing extensible scalar/color key path.

Widgets must not bypass these resolver accessors by reaching directly into
`Theme::chrome`, because doing so would make scoped overrides ineffective.

## Migration map

| Component | Surfaces / structural lines | Genuine GPU shadows |
|---|---|---|
| Menu bar | vertical surface gradient, top highlight, bottom edge | authored outer `0 2px 8px` shadow |
| Menu sheet / context menu | gradient surface, border, top/bottom inset lines, separator pairs | broad `0 16px 40px` + contact `0 2px 6px` |
| Toolbar | rail gradient, dock edge, tool gradients/borders/highlights, separators | popup elevation; blurred inset on latched/pressed faces where authored |
| Dock panel | body/header gradients, header and tab lines | blurred inset on active face where authored |
| Splitter | track gradient, edge lines, grip counter-edge | symmetric cyan dragging glow |
| Status bar | gradient surface, top edge/highlight, divider pairs | blurred inset on latched toggles where authored |
| Dropdown / popover / tooltip / toast | component surface and border | replace all current `drop_shadow` calls with authored values |
| Curve editor | existing local surface | migrate current `drop_shadow`; verify whether it is design-semantic or merely depth feedback |

Opaque 1px CSS shadows that resolve into exact edge colors
remain edge lines. Blurred CSS shadows remain `BoxShadow`s. This is decided by
the authored effect, not by whether the CSS happened to spell both with the
`box-shadow` property.

## Performance model and budget

Shadow rendering scales linearly with the number of visible shadow instances
and with a fixed fragment cost per covered pixel. It must not scale with blur by
adding CPU geometry, draw calls, samples, or allocations. A larger blur covers
more pixels, which is inherent and must be bounded by clip/scissor and the
Gaussian tail cutoff.

Required steady-state properties:

- zero heap allocations per `box_shadow` call after retained draw-list and
  renderer buffers have sufficient capacity;
- one fixed-size tagged CPU/GPU record per analytic chrome or shadow primitive;
- one GPU buffer upload range per rendered draw list/layer, integrated with the
  existing arena rather than one `queue.write_buffer` per instance;
- one draw call per contiguous analytic paint run, including arbitrarily
  alternating chrome/shadow instances in exact painter order;
- fixed shader sample count independent of blur;
- no generated blur textures or intermediate full-screen render targets;
- consecutive color commands share a render pass rather than forcing tile
  load/store cycles at every shadow/surface boundary;
- clips reject fragments outside their effective region, with wholly disjoint
  shadows rejected before upload;
- fully transparent, empty, or collapsed **outset** instances do not reach the
  GPU; collapsed inset holes remain because their complement is full coverage.

The representative benchmark adds grids of 100, 1,000, and 10,000 shadows to
`benches/ui_stress.rs`, both as one contiguous run and interleaved with chrome.
It records draw-list build time, full render CPU time, uploaded bytes, paint
runs, and draw calls. The acceptance budget is:

- contiguous N-shadow cases produce one shadow draw call and do not produce one
  render pass per instance or per adjacent color command;
- draw-list construction remains allocation-free after warm-up;
- instance upload bytes are exactly O(N), with no soup vertices/indices for
  shadows;
- 10,000 instances complete within the existing benchmark's usable-frame
  expectations on the test machine, with the measured result recorded before
  merging. If shader fill dominates due to deliberately overlapping 40px
  shadows, a second sparse/non-overlapping case separates instance overhead from
  unavoidable overdraw.

### Recorded Phase-1 benchmark

Measured on the development machine with `DISPLAY=:0 cargo bench --bench
ui_stress -- shadow` (2026-09-21). The grid is deliberately sparse once it
extends past the 1920×1080 target, so these figures primarily measure fixed CPU
record/upload/encode overhead rather than worst-case overlapping fill:

| Case | 100 | 1,000 | 10,000 |
|---|---:|---:|---:|
| Build, contiguous | 2.21 µs | 21.95 µs | 219.11 µs |
| Build, alternating shadow/chrome | 4.15 µs | 42.99 µs | 435.71 µs |
| Encode/submit CPU, contiguous | 104.00 µs | 277.06 µs | 423.46 µs |
| Encode/submit CPU, alternating shadow/chrome | 108.65 µs | 307.83 µs | 579.08 µs |

These are Criterion point estimates from the final accepted implementation. Both
cases coalesce into one ordered analytic run, one direct instance-buffer upload,
and one draw call; the alternating 10,000-pair case fell from 13.698 ms with
20,000 CPU-encoded draws to 579.08 µs with one draw (95.8% lower). The benchmark
measures main-thread upload/command encoding/submission, not GPU execution time.
Build scaling is linear and retained vectors are reused after warm-up. The
dominant identity/translation build path avoids affine inversion and covariance
roots; the arbitrary-affine path retains screen-isotropic semantics.

## Testing and visual acceptance

### CPU/unit tests

- authored values normalize into the expected shadow and element rectangles;
- offsets in both axes and positive/negative spread have pinned outset and inset
  semantics, including full coverage from a collapsed inset hole;
- corner radii use the CSS spread correction and clamp correctly;
- inset and outset flags produce the correct instance fields;
- clip/tint/translation are captured correctly and wholly clipped instances are
  culled;
- invalid and collapsed-outset inputs are diagnosed, while transparent/empty
  no-ops follow the documented counting policy;
- declaration groups reverse correctly, adjacent shadows coalesce, and
  soup/chrome/text interleaving preserves immediate paint order;
- `clear()` retains capacity and resets shadow state;
- primitive/debug/render counts include shadows.

### GPU tests

A dedicated ignored headless test renders and reads back:

- zero-blur crisp shadows through the SDF branch, including fractional scale;
- broad and contact shadows;
- large-blur thin-source shadows, comparing midpoint and Gauss–Legendre
  quadrature during Phase 0;
- offset and positive/negative spread variants, including a collapsed inset hole;
- uniform and asymmetric rounded corners;
- inset clipping at every edge and outset exclusion from the element interior;
- the 2×26px splitter glow, checking radial symmetry around the source;
- active draw-list clipping at scale factors 1.0, 1.5, and 2.0;
- affine rotation, reflection, uniform/non-uniform scale, shear, singular
  rejection, and the documented transformed-world-AABB clip limitation;
- reverse-stacked mixed-color shadows under a surface;
- many alternating shadow/chrome/soup runs, guarding against buffer-offset
  corruption.

Pixel assertions cover invariant geometry and symmetry. Browser parity uses an
image tolerance rather than exact bytes because browser rasterization and
transfer details can differ at antialiased boundaries.

### Browser-reference fixtures

Phase 0 captures reference PNGs at fixed DPRs, viewport, browser version, and
sRGB output. The fixture HTML is checked in beside the tests. At minimum it
includes the menu sheet's two-shadow stack, a mixed-color stack that exposes
ordering, tooltip/popover elevation, a rounded control inset, collapsed inset
spread, asymmetric corners, and both ordinary and broad-blur splitter grips.

The preferred fixture uses Chromium transparent-background capture and reads PNG
alpha directly. If the browser path flattens alpha, two otherwise-identical
captures over encoded black and white recover aggregate coverage per channel as
`A = 1 - (W - B)`; use their mean/median, clamp to `[0,1]`, and include the
±1/255-per-image quantization error in tolerance. The manifest prevents captures
with different foregrounds/crops from being compared. Coverage is reported
separately from final RGB composition:

- maximum and mean absolute alpha-mask error;
- maximum channel error and mean absolute channel error over the affected region;
- count/fraction of pixels exceeding the chosen tolerance;
- a diff PNG retained under `test_output` for inspection;
- the final-RGB composite, checked against the sRGB-space source-over formula
  on the black/white captures.

Alpha tolerance is selected and documented from the calibration matrix, not
loosened until a failing implementation passes. MAE, p99, mass, and centroid
remain universal. The fixed 8×8 CSS collapsed-inset ROI instead gates the
absolute count of pixels over 8/255 (at most 64: one physical-pixel boundary
band, `4 × 8 CSS px × DPR 2`), because its 36–124-pixel support makes a
universal fraction statistically unstable; the
fraction is still reported. Strict CPU/GPU 256×256-sample area-oracle checks at
DPR 1/1.5/2, three subpixel positions, and identity/affine transforms separately
guard tiny-shape coverage. Composite-RGB tolerance is separate and cannot be
used to retune sigma around a color-space mismatch.

### Widget gallery

Every migrated visible component keeps or gains a focused gallery row. Run and
inspect:

```text
DISPLAY=:0 cargo test --test widget_gallery -- --ignored --nocapture
```

The gallery pass specifically checks broad sheet corners, contact shadow density,
inset clipping, layering between adjacent popup surfaces, and splitter-glow
symmetry—not merely that pixels were emitted.

## Phasing

### Phase 0 — references, calibration, and baseline

Check in the reference fixture HTML and browser captures. Recover reference
alpha masks from controlled backdrops, verify `sigma = blur / 2`, compare
midpoint and Gauss–Legendre quadrature, and check the final RGB composite
separately from the alpha mask. Benchmark the existing `drop_shadow`/glow approximations
so the replacement has an honest CPU, allocation, upload, draw-call, and render-
pass baseline.

### Phase 1 — GPU shadow primitive

Add `BoxShadow`, corner radii, `ShadowInstance`, draw-list recording and ordered
commands, dynamic buffer plumbing, renderer pipeline, analytic shader, debug
accounting, and CPU/GPU tests. Include full forward-affine geometry, explicit
padding-box inset geometry, shared radius-overlap normalization, the zero-sigma
SDF/collapsed-hole branches, straight-alpha output, CSS spread/radius adjustment,
outset interior exclusion, early clipping, and shared color render passes.

### Phase 2 — composable surface primitives

Add `Background`, `EdgeWidths`, `CornerRadii`, `EdgeStyle`, `QuadStyle`,
`paint_quad`, and `edge_line`. Reuse the existing chrome instance whenever
possible. Bounded rectangular composition is limited to geometrically correct
cases; unequal rounded borders require shader support or a diagnostic. Do not
add component semantics here.

### Phase 3 — centralized component theme

Define the finite component chrome structures and populate the default theme
from the handoff's resolved opaque values and authored true shadows. Add typed
optional component overrides to `StyleOverlay` and O(1) resolver accessors,
without cloning or per-frame allocation. Add the component surface painter and
its reverse CSS shadow stacking, outset/background/inset partition, and border/
edge stages. Test that overrides and ordering reach every migrated paint path.

### Phase 4 — migrate widgets

Migrate one small family first (splitter and status bar) to validate the API,
then toolbar, menubar/sheets, dock panel, and remaining floating surfaces.
Regenerate focused gallery images after each family. Remove widget-local handoff
literals as they move into `Theme`. Once no production call site remains, delete
`drop_shadow` and `rounded_rect_glow` and their approximation-specific tests.

### Phase 5 — acceptance and documentation

Run all focused GPU comparisons, the full gallery, library tests, all-target
build, formatting, diff checks, and stress benchmarks. Record benchmark numbers
and any accepted visual tolerance in this document. Update `README.md` API
examples and `TODO.md`, then move completed checklist entries above.

## Risks and trade-offs

### Fragment cost and overdraw

Analytic Gaussian evaluation is more expensive per fragment than a linear color
interpolation. It replaces many CPU primitives and produces better output, but
large overlapping shadows can still consume fill rate. Fixed sampling, bounded
tails, clipping, and batching control overhead; benchmarks distinguish pipeline
overhead from unavoidable affected pixels.

### Browser parity

CSS supplies the `sigma = blur / 2` semantic starting point, but antialiasing,
finite-tail cutoff, quadrature, spread/radius handling, and browser engine details
can still differ. Alpha-mask fixtures verify those details without allowing
colour compositing to distort kernel calibration; the RGB composite is checked
on its own (both composite in sRGB space). Sigma never compensates for colour.

### Instance size

Per-corner radii, local element/hole rectangles, clip, affine, and parameters
make shadows larger than chrome instances. Keeping the record flat avoids
allocation and favors direct upload. The selected forward-affine layout is
expected to consume roughly nine instance `vec4` attributes (plus the base-quad
attribute), within ordinary WebGPU limits; construction validates the actual
device limits and records the final packing. If supported backend limits make the
layout awkward, a renderer-owned storage buffer indexed by instance is
acceptable; per-instance bind groups are not.

### Theme surface area

Typed component styles add fields. That is intentional: the values are already
part of the visual contract, only currently hidden as literals. Types should be
component-sized and finite, while shared semantic tokens remain shared only
where coupled customization is desired.

## Decisions closed for v1

- Shadows carry local geometry plus the forward affine and support all finite,
  non-singular draw-list transforms.
- Runtime compositing is straight-alpha, sRGB-space source-over, like the
  browser: directly into non-sRGB targets (the capture format is `Rgba8Unorm`),
  via an offscreen layer and one composite pass for `*Srgb`/float targets.
- Insets use explicit padding-box geometry supplied by the component surface
  painter.
- Typed component overrides live as O(1) optional fields on `StyleOverlay`.
- Shadow groups accept borrowed slices and paint matching declarations in reverse.

Additive glows are not a v1 option: the current handoff is CSS
`box-shadow`, so source-over is required. A future additive-light primitive would
need a separate design and pipeline rather than silently changing `BoxShadow`.

## References

- Forge design system tokens (`tokens/*.css`) and component sources in the
  claude.ai/design project `1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`. The former
  `design_handoff_forge_chrome/opaque-colors.md` was removed: its oklch→hex
  conversions were wrong (see `docs/design/forge-token-audit.md`).
- `src/widgets/draw_list.rs` — current patch shadows, glows, chrome instances,
  and ordered paint stream
- `src/render/ui_renderer.rs`, `src/render/ui.wgsl` — renderer pipeline and
  instanced SDF precedent
- `src/theme.rs`, `src/style.rs`, `src/widgets/material.rs` — current theme,
  scoped resolution, and control materials
- `benches/ui_stress.rs`, `tests/chrome_instancing.rs`,
  `tests/ordered_paint_stress.rs`
- GPUI/Zed `crates/gpui/{src/style.rs,src/scene.rs,src/window.rs}` and
  `crates/gpui_wgpu/src/shaders.wgsl`
