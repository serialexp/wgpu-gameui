# Opaque colour table — Forge dark chrome

## Why this file exists
The original spec expresses almost every value as an alpha wash — `rgba(255,255,255,0.16)`
for a key face, `rgba(0,0,0,0.6)` for a border. In a browser those composite against
whatever is behind them at paint time. **Outside a browser compositor they do not.** Handing
`rgba(255,255,255,0.16)` to a wgpu renderer, a native toolkit, or any immediate-mode UI
produces a different colour than the design shows, because the blend never happens or happens
against a different backdrop.

Every value below is **flat, opaque sRGB** — the wash already resolved against the surface it
actually sits on in this design. Use these. Do not re-apply the alpha.

## The one rule
If you are implementing this UI, set colours from this table and blend nothing. Alpha in the
original spec was a *convenience for authoring in CSS*, never part of the design intent. A
0.16 white wash over the toolbar is not "16% white" as a concept — it is exactly
`#414449`, and that is the number the renderer wants.

## How these were resolved
Composited in straight sRGB (not linear), `out = dst × (1 − a) + src × a`, matching what a
browser does for non-linear blending. Each value is resolved against its **real parent**, not
against the app background: a key face resolves over the toolbar, a tab over the dock header,
a zebra row over the panel body.

The app background is itself a gradient (`#10171c → #060809 → #0a0d0f`). Everything here is
resolved against **`#0b1014`**, its optical midpoint. Because the chrome panels are 0.94–0.97
opaque, the choice of midpoint shifts any resolved panel value by at most ±1 per channel — below
the threshold of visibility. Where a value sits directly on the app background and is *thin*
(the splitter track, at 0.05 and 0.015) the deviation is larger in relative terms but still
under 1 sRGB step; if your app background is a flat colour rather than a gradient, re-resolve
those two.

## Gradients
Kept as gradients, with both endpoints given as opaque hex. A vertical two-stop linear gradient
is cheap in any renderer and carries the top-lit read that the whole aesthetic depends on. Do
not flatten a gradient to its midpoint — the surfaces stop looking lit.

## Inset highlights and borders
These are the values most often got wrong. An `inset 0 1px 0 rgba(255,255,255,0.18)` on a key
face is a 1px line whose colour resolves **over the face**, not over the toolbar underneath.
The table gives each highlight already resolved against its own element. Draw them as 1px
lines in the stated colour.

Where an element carries both a dark edge and a light line — every horizontal boundary in this
system does — both are listed. That pairing is what produces the carved look; dropping either
one flattens the surface.

## Text
Text colours in the original spec are already opaque hex and need no conversion:
`#e6e9ec #dbe1e7 #d5dce2 #cfd6dd #f1f5f9 #98a0a8 #8b939b #78818a #7d858e #6f7880 #6a737b
#5d656c #464e55 #b6bec5 #eef2f6 #eafaff #041418`.

Text shadows are the exception: `0 -1px 0 rgba(0,0,0,0.5–0.7)` must resolve against whatever
the text sits on. If your renderer cannot draw a text shadow, **drop it rather than approximate
it** — a mis-coloured carve line is worse than none.

---

### Surfaces

| Element | sRGB | Note |
|---|---|---|
| Menu bar (top) | `#252b31` | linear-gradient(180deg, top, bottom) |
| Menu bar (bottom) | `#171b1f` |  |
| Toolbar (top) | `#23282e` |  |
| Toolbar (bottom) | `#161a1e` |  |
| Sheet (menu / context / popover) (top) | `#1d2227` | blur removed; resolve over app bg |
| Sheet (menu / context / popover) (bottom) | `#14181c` |  |
| Dock panel (top) | `#15191d` |  |
| Dock panel (bottom) | `#0f1215` |  |
| Status bar (top) | `#1e2328` |  |
| Status bar (bottom) | `#13171b` |  |
| Tooltip | `#1b2025` | flat |
| Viewport (centre → edge) | `#2c5a66` | radial-gradient, already opaque |
| Viewport edge | `#0c1c23` |  |
| Dock header (top) | `#23272b` | white wash over dock panel |
| Dock header (bottom) | `#181c20` |  |
| Panel row, zebra (odd) | `#16191d` | over dock panel body |
| Panel row, plain (even) | `#121619` | the panel itself |

### Toolbar key

| Element | sRGB | Note |
|---|---|---|
| Idle face (top) | `#414449` |  |
| Idle face (bottom) | `#2a2e33` |  |
| Hover face (top) | `#53565a` |  |
| Hover face (bottom) | `#35393e` |  |
| Pressed face (top) | `#313539` |  |
| Pressed face (bottom) | `#24292d` |  |
| Latched face (top) | `#4a8a9c` | oklch(0.62 0.1 200) — already opaque |
| Latched face (bottom) | `#5fa3b6` | oklch(0.7 0.11 200) |
| Border, idle | `#101215` | 1px |
| Border, latched / pressed | `#0b0d0f` | 1px |
| Top highlight, idle | `#636669` | inset 0 1px 0 — resolved over the face, not the toolbar |
| Top highlight, hover | `#838588` |  |
| Top highlight, pressed | `#3d4145` |  |

### Menu bar

| Element | sRGB | Note |
|---|---|---|
| Title, hover wash | `#32373b` |  |
| Title, open | `#79c6d8` | oklch(0.74 0.11 200) |
| Top highlight line | `#3d4247` | inset 0 1px 0 |
| Bottom edge | `#030506` | 1px solid |

### Sheet

| Element | sRGB | Note |
|---|---|---|
| Row, hover fill | `#79c6d8` | oklch(0.74 0.11 200) |
| Row hover, top highlight | `#a1d7e4` | inset 0 1px 0 |
| Row hover, bottom shade | `#5b95a2` | inset 0 -1px 0 |
| Border | `#030405` | 1px solid |
| Top highlight | `#383d41` | inset 0 1px 0 |
| Bottom shade | `#0a0c0e` | inset 0 -1px 0 |
| Separator rule | `#0a0c0d` | 1px |
| Separator highlight | `#262b2f` | 1px below the rule |

### Dock panel

| Element | sRGB | Note |
|---|---|---|
| Tab, active (top) | `#404346` |  |
| Tab, active (bottom) | `#2a2e31` |  |
| Tab, active highlight | `#6a6c6f` | inset 0 1px 0 |
| Tab, active border | `#0f1113` | 1px |
| Tab, hover | `#2e3135` |  |
| Header key, hover | `#34383b` |  |
| Header bottom edge | `#0a0b0d` | 1px |
| Header highlight, top | `#373b3e` | inset 0 1px 0 |
| Header highlight, bottom | `#212529` | inset 0 -1px 0 |

### Splitter

| Element | sRGB | Note |
|---|---|---|
| Track (top) | `#171c20` |  |
| Track (bottom) | `#0f1418` |  |
| Edge lines (both sides) | `#090b0c` | inset 1px |
| Inner highlight | `#1f2327` | inset 2px |
| Grip, idle | `#505457` |  |
| Grip, hover | `#959799` |  |
| Grip, dragging | `#8fd6e4` | oklch(0.82 0.1 200) |

### Status bar

| Element | sRGB | Note |
|---|---|---|
| Toggle, latched (top) | `#4685a0` | oklch(0.6 0.1 200) |
| Toggle, latched (bottom) | `#5a9db3` | oklch(0.68 0.11 200) |
| Toggle, hover | `#2f3338` |  |
| Divider | `#0a0c0d` | 1px |
| Divider highlight | `#262a2f` | 1px right of the divider |
| Top highlight | `#2f3438` | inset 0 1px 0 |
| Top edge | `#040608` | 1px |

### Badge

| Element | sRGB | Note |
|---|---|---|
| Draft (top) | `#0a0c0e` | neutral chip, over a panel row |
| Draft (bottom) | `#1e2125` | neutral chip, over a panel row |
| Baked (top) | `#1f6b45` | oklch(0.42 0.1 145) |
| Baked (bottom) | `#2b8354` | oklch(0.5 0.12 145) |
| Stale (top) | `#6b5218` | oklch(0.44 0.09 75) |
| Stale (bottom) | `#846527` | oklch(0.53 0.11 75) |
| Error (top) | `#6b1f20` | oklch(0.36 0.13 25) |
| Error (bottom) | `#8a2c2c` | oklch(0.45 0.15 25) |
| Live (top) | `#1f4f5e` | oklch(0.4 0.07 200) |
| Live (bottom) | `#296173` | oklch(0.48 0.08 200) |

---

## Accent, resolved
Hue 200 throughout. The oklch values in the original spec are opaque already; their sRGB
equivalents:

| Role | oklch | sRGB |
|---|---|---|
| Menu title open, row hover | `oklch(0.74 0.11 200)` | `#79c6d8` |
| Latched key (top) | `oklch(0.62 0.1 200)` | `#4a8a9c` |
| Latched key (bottom) | `oklch(0.7 0.11 200)` | `#5fa3b6` |
| Latched key inner shadow | `oklch(0.32 0.07 200)` | `#1c4653` |
| Latched key inner highlight | `oklch(0.55 0.09 200)` | `#3f7a8b` |
| Splitter grip, dragging | `oklch(0.82 0.1 200)` | `#8fd6e4` |
| Tick / accent glyph | `oklch(0.8 0.1 200)` | `#8ad1e0` |
| Status toggle latched (top) | `oklch(0.6 0.1 200)` | `#4685a0` |
| Status toggle latched (bottom) | `oklch(0.68 0.11 200)` | `#5a9db3` |
| Ink on accent | — | `#041418` |
| Icon on latched key | — | `#eafaff` |
| Danger text | `oklch(0.72 0.15 25)` | `#e08a7d` |
| Warning meta | `oklch(0.82 0.13 75)` | `#e0ab62` |

## Glows
`0 0 7px oklch(0.74 0.11 200 / 0.65)` and friends are the one place alpha is genuinely
intended — a glow is additive light, not a surface. Implement as an additive-blended sprite or
a radial falloff in `#79c6d8`, peaking at 65% intensity at the source and reaching zero at 7px.
If additive blending is unavailable, skip the glow; do not substitute a hard ring.

## Backdrop blur
Sheets specify `backdrop-filter: blur(22px)` at 0.96 opacity. At that opacity the blur
contributes almost nothing — the resolved opaque sheet colours in this table are within one
step of the blurred result over any backdrop in this app. **Skip the blur.** It is expensive in
wgpu and invisible here.
