# SDF lines

**Status:** Draft
**Last updated:** 2026-10-08

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

(nothing yet)

### Outstanding

- [ ] Phase 0: Chromium reference captures of SVG strokes (caps, joins, dashes, translucent, hairlines) at DPR 1 / 1.5 / 2, and a benchmark of today's soup `line` as the baseline
- [ ] Phase 1: segment instance kind in the analytic stream (`ANALYTIC_SEGMENT`), pixel-area coverage, full affine, clip
- [ ] Phase 1: `DrawList::line` records a segment instance (butt caps, as today) instead of a soup quad
- [ ] Phase 1: `Stroke { width, cap }` with butt / round / square caps
- [ ] Phase 1: axis-aligned hairlines snap to the pixel grid and come out crisp (one pixel, full coverage)
- [ ] Phase 2: `DrawList::stroke_polyline(points, &Stroke, color)` with round / miter / bevel joins and a miter limit
- [ ] Phase 2: joins split by the angle bisector, so each pixel is drawn once and translucent strokes don't darken at corners
- [ ] Phase 2: `polyline` (no joins) removed; its callers move to `stroke_polyline`
- [ ] Phase 3: dashes (`Stroke.dash`: on, off, offset) evaluated in the shader, continuing across a polyline's segments
- [ ] Phase 3: `dashed_rect_outline` re-checked against the dashed stroke (kept or rebuilt on it; see open question 5)
- [ ] Phase 4: arcs: the ring instance gains a start and end angle; `stroked_arc` and transformed `circle_outline` stop tessellating
- [ ] Phase 5: migrate gameui callers (`busy.rs`, `curve_editor.rs`, `placeholder.rs`, `waffle.rs`, `menubar/paint.rs`, `dropdown.rs`, `combo_box.rs`, `breadcrumb.rs`) and agent-ui's desktop (`transcript_cards.rs`, `main_view.rs`)
- [ ] Phase 5: remove the soup paths for lines and arcs; the gallery and the Chromium comparisons pass
- [ ] Phase 6: README, benchmarks and this doc's status updated
- [ ] Triangles (carets, popover arrows) (deferred: open question 2)
- [ ] Curves drawn exactly in the shader (deferred: open question 3; curves are flattened into polylines until then)

## Why this exists

Everything in gameui that should look smooth is a signed distance field (SDF). The shader works out
how far each pixel is from the shape's edge and turns that into coverage. Rounded rectangles, borders
and shadows are instances in one ordered "analytic" stream (`vs_analytic`/`fs_analytic` in
`src/render/ui.wgsl`). Circles and rings are SDF instances (`vs_circle`/`fs_circle`). Text and icons
are MSDF (multi-channel SDF) glyphs (`ui_msdf.wgsl`; Phosphor and other icon fonts through
`render/icon_font.rs`).

Lines are the exception. `DrawList::line` (`src/widgets/draw_list.rs`) pushes a quad of two plain
triangles into the "vertex soup" (`vs_color`/`fs_color`), and so do `triangle`, `filled_polygon`,
`stroked_arc` and `quad_gradient`. The soup has no anti-aliasing, so a diagonal line has stair-stepped
edges. `polyline` just puts separate `line`s end to end. There are no joins, so corners show notches,
and where segments overlap a translucent colour is drawn twice and comes out darker. There are no caps
or dashes either. Callers work around this: `placeholder.rs` draws its own dashes, `dashed_rect_outline`
builds dashes from axis-aligned quads, and agent-ui's transcript once drew icons as lines with a disc at
every corner. Circles also fall back to soup triangle fans whenever the transform rotates or scales.

This doc is the contract for drawing lines the way the rest of gameui draws shapes: SDF instances with
smooth edges, real caps, joins and dashes, in the same ordered stream as the chrome around them.

## Goals

- Smooth, anti-aliased lines at any angle, scale and DPR, matching how Chromium draws an SVG stroke
  of the same width.
- Caps (butt, round, square), joins (round, miter with a limit, bevel) and dashes, as SVG and CSS
  define them.
- Translucent strokes cover each pixel once: no darker corners where segments meet.
- Axis-aligned 1px lines stay crisp, as the soup quads are today (borders, rules, hairlines).
- Full affine transforms and the draw list's clip, as analytic chrome already supports.
- No allocation per call once warmed up, and no extra draw calls or render passes: segments join the
  existing analytic instance stream.

## Non-goals

- Filling arbitrary paths. Icons go through icon fonts (MSDF); filled shapes are rects, circles and
  rounded rects.
- Reading SVG. Callers give points.
- Variable width along a stroke, gradients along a stroke, and text outlines.
- Changing how rectangles, borders, shadows or text are drawn.

## How it works

### Today

| Call | Path | Edges |
|---|---|---|
| `rounded_rect`, `chrome_rect`, `box_shadow_*` | analytic instance stream (`PaintCmd::Analytic`) | SDF, 4×4 pixel-area coverage, full affine |
| `circle`, `circle_outline` | circle instances (`PaintCmd::Circle`) | SDF (`smoothstep` over `fwidth`); soup fan under rotation or scale |
| text, `icon_msdf` | MSDF glyph atlas | MSDF |
| `line`, `polyline`, `triangle`, `filled_polygon`, `stroked_arc`, `quad_gradient` | vertex soup (`PaintCmd::Soup`) | none |

Call sites today: `line` about 26 in gameui and 23 in agent-ui's desktop, `polyline` 1, `triangle` 23
(mostly carets and arrows: `glyphs.rs`, `popover.rs`, `tree.rs`, `context_menu.rs`), `stroked_arc` 1.

### Segment instances

A new kind in the analytic stream, next to chrome (kind 0) and shadows. One instance is one segment of a
stroke:

- the two end points, in local space, plus the forward affine transform (as chrome instances carry it)
- half the stroke width, the cap at each end, the colour, the clip
- for a polyline segment, the neighbouring points (or the join direction) at each end, and the join
  kind and miter limit
- for a dashed stroke, the dash on / off lengths and how far along the whole stroke this segment
  starts, so the pattern carries on across corners

The vertex shader expands a quad around the segment: its length plus the caps and joins, its width plus
an anti-aliasing margin. The fragment shader:

1. applies the clip first, as `fs_analytic` does;
2. finds the pixel's position along the segment (`t`) and across it (`d`);
3. decides whether this segment owns the pixel at a join (below), and discards it if not;
4. applies the cap past either end (butt: none; square: a box of half the width; round: a disc);
5. applies the dash pattern along `t`, each dash taking the stroke's cap;
6. turns the distance into coverage with a box filter over the pixel, in screen pixels (through
   `dpdx`/`dpdy`, as chrome does). An axis-aligned edge then gets exactly the coverage of the pixel area
   it covers, which keeps aligned hairlines crisp, rather than the soft ramp `smoothstep` gives;
7. returns the colour with `alpha × coverage`, straight alpha, as the other analytic kinds do.

### Joins without overlap

Two segments that meet at a corner both reach into the same pixels around it. Drawing both is what
darkens translucent strokes today. The fix is to split the corner along the bisector of its angle: each
segment only draws pixels on its own side. The distance to the whole stroke is the same on both sides of
that line, so the two halves meet without a seam and each pixel is drawn once:

- **round join:** each side draws its own part of the disc at the corner point;
- **miter join:** each side extends its edge to the bisector, making the pointed corner; past the miter
  limit it becomes a bevel (as in SVG);
- **bevel join:** each side is cut off at the line across the corner's outer points.

So each segment needs to know its neighbours' directions, not just its own two points. That is the main
size cost of an instance (open question 4).

### Hairlines

Today a 1px horizontal soup quad at a whole-pixel `y` fills exactly one row of pixels. An SDF line
centred on a pixel boundary would spread over two rows at half strength and look blurred. So a stroke
whose width is a whole number of device pixels, and that is axis-aligned under a translate-only
transform, has its centre snapped to the pixel grid (onto pixel centres for odd widths, pixel edges for
even widths), as browsers do for borders. Diagonal lines are never snapped.

### API

- `line(p0, p1, width, color)` keeps its signature and its butt caps, and now records a segment.
- `Stroke { width, cap: Cap, join: Join, miter_limit, dash: Option<Dash> }`, a plain `Copy` value.
  `Dash { on, off, offset }`.
- `stroke_line(p0, p1, &Stroke, color)` and `stroke_polyline(points, &Stroke, color)`, plus a `closed`
  form for outlines.
- `polyline` (no joins) is removed, not kept beside the new call.
- Curves: a caller flattens them into points first. A small helper for flattening a cubic Bézier or an
  arc into points can live next to `stroke_polyline` if callers need one.

## Phasing

1. **Phase 0, references.** Chromium captures of SVG `<line>` and `<polyline>` strokes, using the
   fixture setup from `gpu-chrome-and-shadows.md`: each cap and join, a miter past its limit, dashes
   round a corner, translucent strokes over a corner, 1px and 2px axis-aligned and diagonal lines, at
   DPR 1 / 1.5 / 2. A benchmark of soup `line` at 1k and 10k segments for the baseline.
2. **Phase 1, segments.** The instance kind, the shader, `line` and `stroke_line` with caps, hairline
   snapping. Tests: recorded instances, coverage of aligned hairlines, Chromium comparisons for caps.
3. **Phase 2, joins.** `stroke_polyline` with all three joins and the miter limit; remove `polyline`.
   Tests: a translucent polyline's corner pixels match the straight parts' alpha; Chromium comparisons.
4. **Phase 3, dashes.** Dash pattern and offset across segments; decide on `dashed_rect_outline`.
5. **Phase 4, arcs.** Start and end angles on the ring instance; `stroked_arc` and transformed
   `circle_outline` use it.
6. **Phase 5, migration.** Move every caller in gameui and agent-ui's desktop; delete the soup line and
   arc paths; regenerate and look at the widget gallery.
7. **Phase 6, wrap-up.** README, benchmarks recorded here, status.

## Open questions for review

1. **Which stream segments go in.** (a) The analytic tagged stream, as a new kind: ordered with chrome
   and shadows, full affine, one draw call; but the instance grows to fit segments' fields, or uses a
   union layout per kind. (b) Their own instance stream, like circles: a smaller instance, but another
   `PaintCmd` kind that splits draws wherever lines and chrome alternate. I'd go with (a).
2. **Triangles.** Carets and popover arrows are soup triangles with hard edges. Options: an SDF
   triangle kind; Phosphor glyphs for carets (`CaretUp`, `CaretDown`, `CaretRight` are already in the
   enum) with an SDF triangle only for the popover arrow; or leave them for now. Deferred until lines
   land.
3. **Curves.** Flattening curves into polyline points on the CPU is simple and enough for charts and
   the curve editor. Exact quadratic or cubic Bézier distance in the shader is smoother at large sizes
   but costs more per pixel. I'd flatten first.
4. **Instance size.** Carrying neighbour directions, join, dash and phase makes a segment instance about
   100 bytes, against four soup vertices and six indices today. Acceptable for UI line counts (hundreds
   to low thousands), but Phase 0's benchmark should confirm it at 10k.
5. **`dashed_rect_outline`.** It's axis-aligned quads and already crisp. Keep it as it is, or rebuild it
   on the dashed stroke so there is one dash implementation?
   > **2026-10-08 update:** `DrawList::stripes(rect, Stripes)` (a shader-drawn stripe fill, one record
   > however many bands) now exists, with `dashed_hline` on top of it, and `hatch` and
   > `dashed_rect_outline` use it. So axis-aligned dashes are already one SDF-style record. The question
   > becomes whether the dashed stroke reuses the stripe shader's band maths, and whether
   > `dashed_hline` stays as the cheap axis-aligned case beside `Stroke.dash`.
6. **Circles into the analytic stream.** Moving circles in too would remove their soup fallback under
   rotation and scale, and one `PaintCmd` kind. Out of scope here unless Phase 4's arc work makes it the
   natural step.

## References

- `src/widgets/draw_list.rs`: `line`, `polyline`, `triangle`, `stroked_arc`, `circle`,
  `circle_outline`, `dashed_rect_outline`, `PaintCmd`
- `src/render/ui.wgsl`: `vs_color`/`fs_color` (soup), `vs_analytic`/`fs_analytic`, `vs_circle`/`fs_circle`
- `docs/design/gpu-chrome-and-shadows.md`: the analytic stream, pixel-area coverage, Chromium fixtures
- `src/render/icon_font.rs`, `src/render/phosphor.rs`: icons as MSDF glyphs
- `src/widgets/draw_list.rs` `stripes`, `Stripes`, `dashed_hline`: shader-drawn stripe fills (hatching, axis-aligned dashes)
- SVG 2 stroke properties (`stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`, `stroke-dasharray`)
