# Splitter — Forge dark chrome

A draggable divider between two panes. 6px thick, cut into the surface like a groove, with a
centred grip that is the only moving part. Vertical (`col-resize`) and horizontal
(`row-resize`) variants are the same thing rotated.

## Geometry
| Token | Vertical | Horizontal |
|---|---|---|
| Thickness | `width: 6px` | `height: 6px` |
| Cross axis | `height: auto` (stretches) | `width: auto` (stretches) |
| Grip | `2px × 26px` | `26px × 2px` |
| Grip radius | `1px` | `1px` |
| Cursor | `col-resize` | `row-resize` |

`flex-shrink: 0`, `display: flex`, `align-items: center`, `justify-content: center`,
`position: relative`. The grip is `pointer-events: none` so it never interrupts the drag.

## The track
Never changes. It has no hover or active state — all feedback lives in the grip.

**Vertical**
```css
background: linear-gradient(90deg, rgba(255,255,255,0.05), rgba(255,255,255,0.015));
box-shadow:
  inset  1px 0 0 rgba(0,0,0,0.55),      /* dark edge, left  */
  inset -1px 0 0 rgba(0,0,0,0.55),      /* dark edge, right */
  inset  2px 0 0 rgba(255,255,255,0.05);/* highlight, inside the left edge */
```

**Horizontal**
```css
background: linear-gradient(180deg, rgba(255,255,255,0.05), rgba(255,255,255,0.015));
box-shadow:
  inset 0  1px 0 rgba(0,0,0,0.55),
  inset 0 -1px 0 rgba(0,0,0,0.55),
  inset 0  2px 0 rgba(255,255,255,0.05);
```

Three inset lines, not one border. A dark line on each side sinks the strip below both
panes; the highlight sits *immediately inside* the leading dark edge, so light reads as
coming from the top-left and the groove looks physically cut. A single border always reads
flat — this pairing is what makes it look milled.

## The grip — three states
| State | `background` | `box-shadow` |
|---|---|---|
| Idle | `rgba(255,255,255,0.26)` | `1px 0 0 rgba(0,0,0,0.5)` (vertical) / `0 1px 0 rgba(0,0,0,0.5)` (horizontal) |
| Hover | `rgba(255,255,255,0.55)` | same as idle |
| Dragging | `oklch(0.82 0.1 200)` | `0 0 7px oklch(0.74 0.11 200 / 0.65)` |

The accent glow while dragging is load-bearing: it is the only confirmation that the drag
was captured, since the panes themselves move continuously whether or not the pointer is
still over the 6px strip. Do not drop it, and do not substitute a cursor change.

Hover must persist while dragging — a fast drag leaves the strip behind, and the grip going
dim mid-resize reads as a dropped drag.

## Drag behaviour
1. `mousedown` on the strip — `preventDefault()`, cache the container's
   `getBoundingClientRect()`, set `resizing = <side>`.
2. `mousemove` on **`window`** — recompute the pane size from the pointer's distance to the
   container edge, clamp, write it.
3. `mouseup` on **`window`** — clear `resizing`.

Listeners go on `window`, not the strip. A 6px target cannot keep up with a fast pointer; if
the listeners live on the element the drag dies the moment the cursor outruns it.

Clamps in the shipped UI: left/right panes **120–460px**, bottom pane **120–420px**. Size is
recomputed from the pointer position each frame, never accumulated from deltas — accumulation
drifts once a clamp is hit.

No transition on any property. The resize is instant, frame by frame.

## Reference implementation
Framework-free, no classes, no build step. Copy the values, not the markup.

```html
<!-- vertical -->
<div style="display:flex;height:220px">
  <div style="width:186px;flex-shrink:0;background:rgba(16,19,22,0.85)"></div>

  <span style="position:relative;flex-shrink:0;display:flex;align-items:center;justify-content:center;
               width:6px;height:auto;cursor:col-resize;
               background:linear-gradient(90deg,rgba(255,255,255,0.05),rgba(255,255,255,0.015));
               box-shadow:inset 1px 0 0 rgba(0,0,0,0.55),
                          inset -1px 0 0 rgba(0,0,0,0.55),
                          inset 2px 0 0 rgba(255,255,255,0.05)">
    <span style="display:block;pointer-events:none;width:2px;height:26px;border-radius:1px;
                 background:rgba(255,255,255,0.26);
                 box-shadow:1px 0 0 rgba(0,0,0,0.5)"></span>
  </span>

  <div style="flex:1;min-width:0;background:radial-gradient(circle at 60% 60%,#24505c,#0d1e26)"></div>
</div>
```

Hover grip: `background: rgba(255,255,255,0.55)`.
Dragging grip: `background: oklch(0.82 0.1 200); box-shadow: 0 0 7px oklch(0.74 0.11 200 / 0.65)`.

```js
// drag, vertical, pane on the left
function beginResize(e, container, setSize) {
  e.preventDefault();
  const rect = container.getBoundingClientRect();
  const onMove = (ev) => setSize(Math.max(120, Math.min(460, ev.clientX - rect.left)));
  const onUp = () => {
    window.removeEventListener('mousemove', onMove);
    window.removeEventListener('mouseup', onUp);
  };
  window.addEventListener('mousemove', onMove);
  window.addEventListener('mouseup', onUp);
}
```

For a pane docked to the right, use `rect.right - ev.clientX`; for a bottom pane,
`rect.bottom - ev.clientY` with the 120–420px clamp.

## Opaque values
The CSS above uses alpha washes, which only resolve correctly in a browser compositor. Resolved
against this design's app background (`#0b1014`), as flat sRGB:

| Part | State | sRGB |
|---|---|---|
| Track | top of gradient | `#1a1f23` |
| Track | bottom of gradient | `#0d1216` |
| Edge lines, both sides | — | `#080b0e` |
| Inner highlight | — | `#1b2024` |
| Grip | idle | `#3d4245` |
| Grip | hover | `#8d9093` |
| Grip | dragging | `#8fd6e4` |

If your app background is a flat colour rather than this design's gradient, re-resolve the
track and grip against it — they are thin washes and will shift. Everything else is opaque
already.

## Accent
Hue **200**. `oklch(0.82 0.1 200)` for the grip, `oklch(0.74 0.11 200 / 0.65)` for its glow.
Accent in this system means *modal state* — something is currently happening or latched. A
splitter is accent only while its drag is live, never on hover and never at rest.
