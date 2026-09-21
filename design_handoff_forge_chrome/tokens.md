# Design tokens — Forge dark chrome

> **Implementing outside a browser? Read `opaque-colors.md` instead.**
> The values on this page are written as CSS alpha washes, which only resolve to the intended
> colour inside a browser's compositor. `opaque-colors.md` gives every one of them already
> composited against its real parent surface, as flat sRGB.

Every value below appears literally in the section HTML files. There are no CSS variables in
those files on purpose: each element carries its own resolved values so nothing has to be
traced through indirection.

## Accent
A single hue drives all modal state. Hue = **200**.

| Role | Value |
|---|---|
| Accent fill (menu hover, latched key) | `oklch(0.74 0.11 200)` |
| Accent fill, top of latched gradient | `oklch(0.62 0.1 200)` |
| Accent fill, bottom of latched gradient | `oklch(0.7 0.11 200)` |
| Accent inner shadow on latched key | `oklch(0.32 0.07 200)` |
| Accent tick / glyph on dark | `oklch(0.8 0.1 200)` |
| Accent glow | `oklch(0.74 0.11 200 / 0.6–0.65)` |
| Ink **on** accent | `#041418` |
| Icon on latched accent key | `#eafaff` |
| Danger text | `oklch(0.72 0.15 25)` |
| Warning meta | `oklch(0.82 0.13 75)` |

Accent is reserved for modal state only: the open menu title, the hovered menu row, the
latched tool, an open dock toggle, a live splitter drag, the dirty dot. Never for emphasis,
never for a resting surface.

## Surfaces
| Role | Value |
|---|---|
| App background | `linear-gradient(160deg, #10171c 0%, #060809 55%, #0a0d0f 100%)` |
| Menu bar | `linear-gradient(180deg, rgba(38,44,50,0.96), rgba(23,27,31,0.97))` |
| Toolbar | `linear-gradient(180deg, rgba(36,41,47,0.96), rgba(22,26,30,0.97))` |
| Sheet (menu, context, popover) | `linear-gradient(180deg, rgba(30,35,40,0.96), rgba(20,24,28,0.97))` + `backdrop-filter: blur(22px)` |
| Dock panel | `linear-gradient(180deg, rgba(22,26,30,0.94), rgba(15,18,21,0.96))` |
| Dock header | `linear-gradient(180deg, rgba(255,255,255,0.06), rgba(255,255,255,0.012))` |
| Status bar | `linear-gradient(180deg, rgba(31,36,41,0.95), rgba(19,23,27,0.96))` |
| Tooltip | `rgba(28,33,38,0.96)` + `blur(14px)` |
| Row zebra | `rgba(255,255,255,0.016)` |
| Viewport | `radial-gradient(circle at 62% 46%, #2c5a66, #0c1c23 70%)` |

## Text
| Role | Value |
|---|---|
| Primary | `#e6e9ec` |
| Menu item label | `#dbe1e7` |
| Menu title | `#d5dce2` |
| Panel row label | `#cfd6dd` |
| Active tab | `#f1f5f9` |
| Secondary / shortcut | `#78818a` |
| Tab idle | `#98a0a8` |
| Panel glyph | `#6f7880` |
| Meta | `#6a737b` |
| Brand mark | `#6f7982` |
| Status bar | `#7d858e` |
| Disabled | `#5d656c` |
| Disabled shortcut | `#464e55` |
| Key face idle icon | `#b6bec5` |
| Key face hover icon | `#eef2f6` |
| Text shadow, all chrome text | `0 -1px 0 rgba(0,0,0,0.5–0.7)` |

The `0 -1px 0` shadow sits **above** the glyph, not below — light comes from the top, so text
is carved into the surface rather than raised off it. It is applied to every label in the
chrome and is a large part of why the theme reads as physical.

## Geometry
| Token | Value |
|---|---|
| Corner radius | **1px** everywhere (tweakable 0–4; 1 is the shipped value) |
| Key travel | **2px** |
| Menu bar height | 26px |
| Status bar height | 26px |
| Menu row height | 22px |
| Panel row height | 21px |
| Dock header height | 24px |
| Tab chip height | 18px |
| Key face | 24×24px |
| Header icon key | 17×17px |
| Status toggle key | 18×18px |
| Splitter thickness | 6px |
| Splitter grip | 2×26px |
| Sheet min-width | 218px (dock popover 142px) |
| Sheet padding | 3px |
| Toolbar padding / gap | 3px / 2px |

Sharp corners are load-bearing. At 1px the radius reads as a machined edge break, not a
rounded widget; going to 4px collapses the whole aesthetic into generic soft-UI.

## Depth
| Role | Value |
|---|---|
| Raised edge (bar, toolbar) | `inset 0 1px 0 rgba(255,255,255,0.11)` |
| Sheet | `0 16px 40px rgba(0,0,0,0.7), 0 2px 6px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.12), inset 0 -1px 0 rgba(0,0,0,0.5)` |
| Tooltip | `0 6px 18px rgba(0,0,0,0.6), inset 0 1px 0 rgba(255,255,255,0.11)` |
| Key idle | `inset 0 1px 0 rgba(255,255,255,0.18)` |
| Key hover | `inset 0 1px 0 rgba(255,255,255,0.28)` |
| Key pressed | `inset 0 1px 0 rgba(255,255,255,0.06), inset 0 2px 3px rgba(0,0,0,0.4)` |
| Key latched | `inset 0 2px 4px oklch(0.32 0.07 200), inset 0 1px 0 oklch(0.55 0.09 200)` |
| Menu row hover | `inset 0 1px 0 rgba(255,255,255,0.3), inset 0 -1px 0 rgba(0,0,0,0.25)` |
| Active tab | `inset 0 1px 0 rgba(255,255,255,0.22)` |
| Separator rule | `background rgba(0,0,0,0.6)` + `box-shadow 0 1px 0 rgba(255,255,255,0.06)` |
| Hard edge between regions | `1px solid rgba(0,0,0,0.6–0.75)` |

Every horizontal edge in the system is **two** lines: a dark one and a light one directly
below it. That pairing is what produces the carved look; a single border always reads flat.

## Type scale
| Role | Font | Size | Weight | Tracking |
|---|---|---|---|---|
| Menu title / item | IBM Plex Sans | 11.5px | 400 | — |
| Tab label | IBM Plex Sans | 11px | 400 | — |
| Panel row label | IBM Plex Sans | 11px | 400 | — |
| Tooltip | IBM Plex Sans | 11px | 400 | — |
| Shortcut | IBM Plex Mono | 10px | 400 | — |
| Status bar | IBM Plex Mono | 10px | 400 | — |
| Panel glyph | IBM Plex Mono | 9px | 400 | — |
| Panel meta | IBM Plex Mono | 9px | 400 | — |
| Section title (popover) | IBM Plex Mono | 9px | 400 | 0.14em, uppercase |
| Brand mark | IBM Plex Mono | 9px | 400 | 0.18em, uppercase |
| Key glyph | inherited | 12px | 400 | — |
| Submenu arrow | inherited | 7px | 400 | — |

## Motion
None. No transitions, no easing, no delays — including tooltips. State changes are
instantaneous by design.
