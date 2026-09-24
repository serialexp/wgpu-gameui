# Forge design-token audit (2026-09-24)

Comparison of the Forge design system — claude.ai/design project
`1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`, **the authoritative design source** —
against the library as of 2026-09-24. Three read-only audits (colours; sizes &
type; depth & motion) plus a component inventory. File:line references are to
that date's working tree and will drift; re-verify before acting on one.

Token files live in the design project under `tokens/` (`colors.css`,
`geometry.css`, `typography.css`, `depth.css`, `motion.css`).

## Decisions taken (Bart, 2026-09-24)

1. **`TextBlock::with_color` takes ordinary sRGB hex and must render it
   correctly.** No caller should convert to linear by hand.
2. **The DesignSync tokens are authoritative.** The old
   `design_handoff_forge_chrome/` folder (incl. `opaque-colors.md`, whose
   oklch→hex conversions were wrong) and `UI System Default Look/` were removed.
3. **Motion:** no hover/press/toast fades. Smooth scrolling **stays** (the
   design will be updated to allow it as the one exception).
4. **Blend like the browser (sRGB space)** so what the design agent shows is
   what the library renders.

## Fixed since the audit

- **Colour space and text colours** — `6890f05` (`feat!: blend UI colours in
  sRGB space like the browser`): colours are sRGB-encoded everywhere and blend
  like CSS; `TextBlock` colours are plain hex.
- **Two accents and palette drifts** — `fix: use DesignSync accent and palette
  values`: the rows marked *(fixed)* below.

## Cross-cutting findings

### Colour space (drives decisions 1 and 4)

- Render targets are `*Srgb` with `ALPHA_BLENDING`, so blending and gradient
  interpolation happen in **linear** light; CSS does both in **sRGB**. Every
  translucent token (key-face sheens, washes, edges) therefore renders lighter
  than the design even when the alpha matches (white-sheen key faces ≈45 steps
  lighter). `chrome.rs` families dodge this with pre-blended opaque colours.
- Theme / DrawList colours are stored linear (`srgb_to_linear`,
  `opaque_srgb8`, ~140 call sites in 11 files).
- **Bug:** `TextBlock::with_color(u8…)` → `color_to_rgba` (`text.rs`) divides
  by 255 and passes straight through `ui_msdf.wgsl`, so the u8s are treated as
  linear. Theme-driven callers pass `linear*255` (looks right); literal sRGB
  hex in `menubar/mod.rs`, `menubar/paint.rs`, `context_menu.rs` renders 19–73
  steps too bright (disabled `#5d656c` → `#a3a9ae`; ink-on-accent `#041418` →
  `#224f56`). Same class: `AXIS_TINTS` (`vector_field.rs`), slider knob,
  asset-grid plate, toolbar popup text, syntax palette.
- **Two accents:** menus, context menu, splitter grip/glow and the latched
  toolbar key use `#79c6d8` & friends (bad conversions). Forge `--accent` =
  oklch(0.74 0.11 200) = `#3ebfc6`, which `theme.accent` already has.

### Motion (decision 3)

- `Theme::animation_duration` defaults to 0.12 s EaseOut (checkbox fill,
  hover overlays, button label colour). Toasts fade out over 0.4 s. → remove.
- ScrollView smooth-scroll 0.22 s → keep.
- The three loops don't match: spinner caller-driven (~0.785 s/turn vs 0.75);
  shimmer is a hard 0.10 white band scaled to width (design: 1.1 s, ±180 px,
  0.05→0.12→0.05 gradient); pulse opacity 0.8–1.0 with radius scaling and a
  wrap discontinuity (design: 0.25↔1, 1.1 s, 0.15 s stagger). No
  indeterminate `ProgressBar` (design: 120 ms tick, `--accent-sweep`).

### Other bugs found

- `bundled_mono_font()` re-inserts all four Plex Mono faces (~700 KB) into
  fontdb on every call; the menubar calls it every frame a menu is open.

## Colours

Legend: ✅ match (≤2/255, ≤0.01 α) · ≠ different · ✗ missing.

| Area | ✅ | ≠ | ✗ |
|---|---|---|---|
| Accent | accent (theme), accent-ring, accent-key top/bottom/pressed, hover-bottom, accent-switch, ink-on-accent-key, ink-on-latch; *(fixed)* menu/context/splitter accent, accent-tick (`Theme::accent_tick`), accent-grip, accent-glow, accent-key-hover-top, latch top/bottom/shade/hi, ink-on-accent-2 | accent-dirty (no glow); accent-wash (uses ButtonHover); slider fill reuses key gradient; progress fill; accent-chip (+ dark ink instead of light) | accent-match, accent-glyph, accent-toggle-*, fill-live-*, accent-sweep, live-* |
| Danger | ring-edge, key top/bottom/hover, key-ink; *(fixed)* key-pressed top/bottom, danger-ring α .18 | banner bg/ink | danger-text, danger-hint, chip-*, soft(-ink), bubble-* |
| Warning | *(fixed)* warn-rule = `theme.warning` | warn-meta (no own field); warn-soft α | warn-soft-ink, chip-*, banner-* |
| Success | ok | — | ok-glow, ok-chip-* |
| Axis | values in `AXIS_TINTS` correct; *(fixed)* rendered as sRGB | — | — |
| Ink ladder | ink-max (dock tab, *(fixed)* `text_highlight`), ink-value (`theme.text`), ink-icon, ink-chip (`text_dim`), ink-tab, ink-on-accent (geometry); *(fixed)* ink-menu/title/shortcut/disabled/disabled-key | status bar uses TextDim `#adb6bd` not ink-muted `#7d858e` | 15 of 26 steps: white, emph, row, cell, 2, glyph, tip-hint, body-2, label, caption, dim, disabled-glyph, empty, brand, primary |
| Surfaces | app, menubar, toolbar, sheet, dock(+opaque), dock-header(+opaque), status(+opaque), tooltip, dropdown | dialog/toast/popover use flat panel not their gradients; row-zebra (×1.12 linear); row-hover (opaque ButtonHover) | page, app-gradient, viewport, card |
| Key / well / edges | plinth, key-face idle/pressed α, key-border, well, well-focus, well-border, edge-dark/hard/sheet/light/light-2; *(fixed)* key-face-hover α .22/.10 | — | key-border-pressed, well-deep; edge tokens are hard-coded literals, not themeable |

Library colours with no Forge token: `theme.info` `#5eaceb`, button base tones
`#1f2429/#21262b/#14181c`, `theme.panel` `#16191d@.95`, `text_highlight`,
assorted black overlays (scrim .55, panel_border .55, tab_border .45 …),
material ghost/disabled constants.

## Sizes & type

1 design px = 1 logical px (renderer divides by `scale_factor`). Many text
sizes are `font_size × factor` rather than tokens.

| Token | Status |
|---|---|
| radius 1 | ✅ |
| radius-panel 2 | ≠ 1 (no separate panel radius); toast 3 |
| radius-pill 9 | ✅ chips/tags/switch; ≠ badge uses 1 |
| radius-search 999 | ✗ (no search well) |
| travel 2 | ✅ |
| h-menubar/statusbar 26, h-dock-header 24, h-menu-row 22 | ✅ (status bar height duplicated as literal in `app_shell.rs`) |
| h-list-row 22 | ✅ by derivation (`FontSize + 10`) |
| h-panel-row 21, h-drag-row 23 | ✗ |
| h-doc-tab 27 | ≠ 24 |
| h-chip 18 | ✅ dock chip; ≠ filter chip 17.2, tag 17, badge 15 |
| key 24 | ✅ |
| key-header 17 | ✅ dock; ≠ window close 18, popover close 14 |
| key-status 18 | ✗ (no status toggle key) |
| gap-bar 1 | ✅ |
| gap-keys 3 | ≠ 2 (toolbar) |
| gap-chips 5 | ≠ 4 (tags) |
| gap-fields 6 | ✅ (`Theme.spacing`) |
| gap-section 11, pad-panel 14 | ✗ (containers use `padding` 5) |
| gap-grid 18 | ≠ AssetGrid 6 |
| pad-sheet 3 | ✅ |
| pad-well 5×7 | partial: 5×5 |
| pad-key 6×10 | ≠ horizontal 5 |
| sheet-min 218 | ✅ (context menu hard-codes it) |
| inspector-w 272, inspector-label 52 | ✗ |
| z-* | N/A — `LayerStack` is push-ordered; nothing enforces tooltip > sheet or drag ghost on top |
| font-sans Plex Sans | ✅ bundled & default |
| font-mono Plex Mono | bundled but only menubar hints use it; no `Theme` mono field |
| text 9/10/10.5/11/11.5/12 | mostly ≠ — ad-hoc factors (10.2, 9.6, 10.8, 8.4 …); buttons 12 (→11.5); table cells 9 (→11.5); 10.5 unused |
| tracking (5 tokens) | ✗ in widgets; `TextBlock::letter_spacing` exists (px, not em) |
| uppercase captions | ✗ |
| weights 500/600 | ✗ — Medium/SemiBold faces not bundled |
| carve | supported via `with_shadow`, but a literal copied ~20×; missing on button, checkbox, radio, toggle, tabs, doc tabs, tree, table, group, panel, window title, badge, chip, keycap, tag input, dropdown, combo, breadcrumb, popover, slider; menubar variants differ slightly |
| carve-deep, carve-on-light | ✗ |

## Depth

| Token | Status |
|---|---|
| key-inset .18 | ✅ |
| key-inset-hover .26 | ✅ *(fixed)* |
| key-inset-pressed .07 + inset 0 2 3 .4 | partial — edge .07 *(fixed)*, but a flat unblurred 2 px band @.6 (`material.rs`); toolbar partial |
| key-inset-latched | partial — toolbar only (colours *(fixed)*); `Material` has no latched state; status-bar inset lacks the `latch-hi` line |
| accent-key-inset .5 / danger-key-inset .3 | ≠ both .18 |
| accent/danger pressed insets | ≠ (flat band) |
| sunken-key-inset | ≠ 6 px gradient band |
| well-inset | partial — top only, light line drawn *inside above* the bottom border instead of 1 px below |
| well-inset-row / -tall | ✗ |
| well-focus-ring (2 px outer @.16) | ≠ 1 px inward, likely invisible under the accent border |
| well-invalid-ring | ✗ in practice (`draw_well` private; everyone passes `invalid=false`) |
| row-hover-inset / row-select-inset | ✅ menubar and ContextMenu *(fixed: real translucent insets)*; ✗ List, Tree, Table, Dropdown |
| tab-active-inset .22 | ✅ dock; ≠ Tabs/DocTabs .18 |
| hi-bar .11 | ✅ menubar; ≠ dock header, status bar, toolbar |
| hi-card .055 | ✗ Panel/Group |
| shadow-sheet, shadow-tooltip | ✅ |
| shadow-popover | ≠ 14/44/.6 + hi .18 |
| shadow-dropdown | partial — α .6, no highlight line |
| shadow-ghost | ✗ (no drag ghost) |
| shadow-inspector | ≠ (Window stand-in) |
| chip-inset / chip-inset-neutral | partial / ≠ (unlatched chip drawn raised) |
| rule / rule-hi / rule-hi-v | ✅ chrome; ≠ `Separator` widget (.55/.18) |
| blur-sheet 22 / blur-tooltip 14 | ✗ — `blur_backdrop` exists but only over an app-supplied scene texture; radius means σ = r/2 (CSS σ = r); 16-tap cap |

Two-line edge rule (dark, then light beneath): followed by menu/toolbar/status
separators and edges; violated by wells/tracks/badges (inverted), dock & window
headers (light above dark), table header, group header, panel, menubar bottom.

## Component inventory

61 design components. Library status:

- **Present (47):** ContextMenu, DockPanel, MenuBar, StatusBar, AssetGrid,
  Badge, ListView (`List`), Table, Tree, ColorPicker, CurveEditor,
  GradientRamp, Banner, BusyDots (`dots`), EmptyState, Skeleton, Spinner,
  Toast, Tooltip, Checkbox, ComboBox, Dropdown, NumberField, ProgressBar,
  Radio, Slider, Switch (`Toggle`), TagInput, TextField, TextArea (multiline
  `TextInput`), VectorField, Button, FilterChip (`chip`), Keycap, Toolbar,
  Breadcrumb, DocumentTabs, Group, Pager, Panel, Popover, ScrollArea
  (`ScrollView`), SegmentedTabs (`Tabs`), Separator, Splitter,
  FloatingWindow (`Window`).
- **Partial (6):** Modal (layer + scrim, no dialog with title/action footer),
  MenuSheet (only inside MenuBar/ContextMenu), Key (no `held`/`hollow`),
  IconKey (no square glyph key at 24/18/17), SearchField (no ⌕ + clear key),
  PropertyRow (`SettingsForm` rows, no label-scrub / 21 px).
- **Missing (9):** CountBubble, DragList, DropZone, FieldLabel, FileField,
  Inspector, PropertyGroup, DockSection, DockStack.
