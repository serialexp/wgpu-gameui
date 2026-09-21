# Handoff: Forge — application chrome (menu bar, toolbar, docks, status bar)

## Overview
The chrome layer of a desktop level-editor UI: a menu bar with tracking dropdowns and
submenus, a dockable tool strip, Zed-style edge dock panels with resizable splitters,
a viewport context menu, and a status bar. This is the dark "workstation" theme — a
skeuomorphic, measured aesthetic with real depth and tangible press feedback, deliberately
opposed to flat gray-on-gray tool UI.

## About the design files
Every file in this bundle is a **design reference written as plain static HTML**. They are
not production code and not a component library. Each one renders a single section in
isolation, with every state laid out side by side so nothing has to be triggered to be seen.
All styling is literal inline `style` attributes — no CSS classes, no variables, no build
step, no JavaScript. Open any file in a browser; read it top to bottom.

The task is to **recreate these sections in the target codebase's own environment** (React,
Vue, SwiftUI, Qt, ImGui, whatever the app already uses), following its established patterns.
If there is no environment yet, pick one and implement there.

## Fidelity
**High fidelity.** Colors, type, spacing, shadow stacks and interaction states are final and
should be matched exactly. Every numeric value in these files is intentional.

## Files in this bundle
| File | Section |
|---|---|
| `index.html` | Contact sheet — links to every section |
| `01-menu-bar.html` | The bar itself; idle / hover / open states |
| `02-menu-sheet.html` | Dropdown sheet; every item variant and the submenu |
| `03-toolbar.html` | Tool strip; all four dock orientations, key states, tooltip, dock popover |
| `04-dock-panel.html` | Edge dock: tab header, list body, splitter, all three panel types |
| `05-context-menu.html` | Viewport right-click menu |
| `06-status-bar.html` | Status bar with dock toggles |
| `tokens.md` | Every shared value in one table (as authored, with alpha) |
| `opaque-colors.md` | **Every colour resolved to flat sRGB — use this when implementing outside a browser** |
| `splitter.md` | Standalone splitter spec |
| `4a Menu Bar.dc.html` | The original working prototype (interactive, framework-bound) |

Read the section files for structure and exact values. Read the prototype only if you need
to see the interaction actually run.

---

## Section specs

### 1. Menu bar
Fixed **26px** tall, full width, `z-index` above everything except open sheets. Contents laid
out with flex, `gap: 1px`, `padding: 0 4px`.

- **Brand mark** — `FORGE`, IBM Plex Mono 9px, letter-spacing `0.18em`, uppercase, `#6f7982`,
  padding `0 9px 0 6px`.
- **Menu titles** — IBM Plex Sans 11.5px, `#d5dce2`, padding `0 9px`, full-height. Hover is
  `rgba(255,255,255,0.09)`. Open is the accent `oklch(0.74 0.11 200)` with **dark** ink
  `#041418` — accent means *modal state*, never decoration.
- **Document tag** — right-aligned, mono 10px `#78818a`, followed by a 5px dirty-dot in
  accent with a `0 0 6px` glow.

### 2. Menu sheet (dropdown)
Absolutely positioned at `top: 26px; left: 0` of its title. `min-width: 218px`, `padding: 3px`,
`border-radius: 1px`. Background `linear-gradient(180deg, rgba(30,35,40,0.96), rgba(20,24,28,0.97))`
with `backdrop-filter: blur(22px)`.

Item rows are **22px** tall, `padding: 0 8px`, `gap: 7px`, four columns in fixed order:
tick (10px) · label (flex) · shortcut (mono 10px, right) · submenu arrow (8px).

Item variants: default, hover, disabled (`#5d656c`, no hover), checkmark `✓`, radio dot `•`,
submenu parent `▸`, and danger (`oklch(0.72 0.15 25)`). Separators are a 1px rule inset
`0 6px` with a highlight line beneath.

**Submenus** open on parent hover at `top: -4px; left: 100%; margin-left: 2px`, using the
identical sheet style. The parent row stays lit while its submenu is open.

### 3. Toolbar
A strip of **press keys**. This is the core interaction metaphor of the whole system: each key
is a *face plate on a plinth*. The plinth is a transparent box with `padding-bottom: 2px`;
pressing moves that 2px to `padding-top`, so the face physically drops into its own shadow.
Do not fake this with `transform: translateY` — the travel must come from the plinth so the
key's footprint never moves.

Face: `24×24px`, `border-radius: 1px`, `border: 1px solid rgba(0,0,0,0.45)`.
- idle — `linear-gradient(180deg, rgba(255,255,255,0.16), rgba(255,255,255,0.06))`, `inset 0 1px 0 rgba(255,255,255,0.18)`
- hover — `0.24 → 0.11`, top highlight to `0.28`
- pressed — `0.09 → 0.035`, highlight replaced by `inset 0 2px 3px rgba(0,0,0,0.4)`
- **latched** (selected tool) — accent fill `oklch(0.62 0.1 200) → oklch(0.7 0.11 200)`,
  `inset 0 2px 4px oklch(0.32 0.07 200)`, icon `#eafaff`. Latched keys render pressed *and*
  accented; they never return to idle on mouse-out.

The strip docks to any of the four edges: the flex direction, separator orientation, tooltip
side and dock-popover anchor all flip. Group separators are 1px rules across the short axis.
A drag grip sits at the leading end; a `⋯` overflow key and right-click both open the dock
popover (same sheet style as a menu, with a mono uppercase title row).

Tooltips appear on hover after no delay, offset 7px on the strip's outward side, mono keycap
hint at `#8d959d`.

### 4. Dock panel
Edge-anchored panel, default width **186px** (left) / **216px** (right) / height **148px**
(bottom), clamped 120–460px (bottom 120–420px) while dragging.

- **Header** — 24px, tab chips 18px tall with `padding: 0 7px`. Active tab is a raised chip:
  `linear-gradient(180deg, rgba(255,255,255,0.15), rgba(255,255,255,0.055))` + 1px dark border
  + `inset 0 1px 0 rgba(255,255,255,0.22)`. Header keys `⤢` and `×` are 17px, icon-only,
  lighting only on hover.
- **Body** — 21px rows, `padding: 0 9px`, `gap: 6px`. Zebra striping at
  `rgba(255,255,255,0.016)` on odd rows. Three columns: glyph (10px mono `#6f7880`), label
  (11px `#cfd6dd`, ellipsis), meta (mono 9px `#6a737b`; warnings `oklch(0.82 0.13 75)`).
  Console rows set the whole label in mono.
- **Splitter** — 6px, cursor `col-resize`/`row-resize`, dark inset edges on both sides and a
  single highlight line. The 2×26px grip goes `rgba(255,255,255,0.26)` → `0.55` on hover →
  accent with a `0 0 7px` glow while dragging. The glow is the only feedback that the drag is
  live; keep it.

### 5. Context menu
Identical sheet and row rules as the dropdown. Opens at the cursor, clamped so it cannot
overflow the viewport (`x` capped at `width - 236`, `y` at `height - 270`). Closes on
`Escape`, on any root click, and on item activation.

### 6. Status bar
26px, mono 10px, `#7d858e`. Left group: three 18px dock-toggle keys (`◧ ▤ ◨`) that latch to
the accent when their panel is open, using the same latched treatment as toolbar keys. Then a
1px divider, then a `last:` action echo in `#cfd6dd`. Right group: snap size, shading mode,
gizmo state, separated by 1px dividers.

---

## Interactions & behavior

**Menu tracking** — the defining behavior. Click a title to open; while *any* menu is open,
hovering a different title switches to it instantly with no click. Hovering an item with a
submenu opens that submenu immediately and closes any sibling submenu. `Escape` closes
everything; a click anywhere on the root closes everything; activating an item closes the
whole stack and writes the item's label to the status bar echo.

Menu titles use `mousedown` (not `click`) to open, so the menu appears under the finger
before release. Items stop propagation so their click does not reach the root close handler.

**Splitter drag** — `mousedown` on the splitter captures; `mousemove` on `window` recomputes
size from the pointer's distance to the window edge; `mouseup` on `window` releases. Listeners
live on `window`, not the splitter, so a fast drag cannot outrun the element.

**No transitions anywhere.** Every state change is instant. This is deliberate — the UI should
feel mechanical, not animated. Do not add ease curves when porting.

## State
`openMenu`, `hoveredTitle`, `hoveredItem`, `openSubmenu`, `contextMenu {x,y}`, `activeTool`,
`toolbarDock (top|right|bottom|left)`, `dockPopoverOpen`, `pressedKey`, `hoveredKey`,
per-panel `{open, size, tab}` for left/right/bottom, `resizingSide`, plus the app values the
menus mutate: `snap`, `shading`, `gizmos`, `grid`, `wireframe`, `stats`, and the `log` echo.

## Assets
None. Every icon is a Unicode glyph set in the UI font — `⌖ ✥ ⟳ ⤢ ▢ ✎ ⌫ ☀ ◈ ⟺ ⌗ ▸ ✓ • × ⋯
◧ ◨ ▤ ▣ ◍ ◉`. If the target platform has a real icon set, substitute it at the same optical
size (12px glyph inside a 24px key) rather than shipping these glyphs.

## Typography
IBM Plex Sans (400/500/600) for all UI text; IBM Plex Mono (400/500) for shortcuts, metadata,
numerics, console output and the status bar. No other families. Nothing in the chrome is
larger than 11.5px — this is a dense professional tool, and the scale is part of the identity.
