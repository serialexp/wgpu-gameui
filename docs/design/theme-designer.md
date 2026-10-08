# Theme designer and presets — Design

Status: phase 1 in progress
Owner: Bart
Last updated: 2026-10-05

## Why

Forge suits many of Bart's apps but not all of them. The Forge design project
has one page in a completely different style ("Aldermoor — Theme Proof",
`explorations/Fantasy Theme.dc.html`): slate chrome, bronze trim, parchment
surfaces, green accent, serif type. We want a tool that lets us restyle gameui
like that, with a live preview, and export the result as a preset that an app
loads instead of Forge.

## Decisions (Bart, 2026-10-05)

- **Seeds + overrides.** A small set of seeds derives every colour and size;
  any single token can then be overridden.
- **Re-skin Forge, for colour.** Forge's values are the template; seeds move
  them. Later we want "three primary colours" rather than Forge's single
  accent — that comes after this.
- **TOML preset files**, loaded at runtime by apps (feature `preset`).
- **Designer first**, then close the gaps (below) it makes visible.
- **A separate binary crate** (`designer/`, a workspace member), built only
  from what gameui already offers — no file dialogs or other UI toolkits.
- **Preview:** a curated proof page first; the full widget gallery inside
  the designer later. Meanwhile the gallery test renders in any preset via
  `GAMEUI_PRESET=<file>`.
- **Chrome overrides later.** `ChromeTheme` follows the seeds; overrides
  cover the keyed `Theme` tokens (`StyleKey`) only, for now.

## The colour transform

Forge assigns one hue per meaning and lets lightness/chroma name the role
(`tokens/colors.css`). Measured in OKLCH, every Forge colour falls into one
of these families:

| Family   | How it is recognised                                | Forge anchor hue |
|----------|-----------------------------------------------------|------------------|
| Surface  | chroma < 0.02, not text                             | 248              |
| Ink      | chroma < 0.02, a text role (`Ink(_)`, `Text*`)      | 248              |
| Accent   | chroma ≥ 0.02, nearest anchor                       | 200              |
| Danger   | 〃                                                   | 25               |
| Warning  | 〃                                                   | 75               |
| Success  | 〃                                                   | 145              |
| Info     | 〃                                                   | 245              |

Forge's neutrals measure hue 237–256 at chroma 0.005–0.018; every chromatic
token is at chroma ≥ 0.02. Pure white/black overlays (chroma 0) stay as they
are — they are light and shade, not colour.

Each family has a `ColorSeed { hue, chroma, lightness }`:

- `hue` — absolute, in degrees. The transform rotates by
  `seed.hue - anchor`, so the spread within a family (warn 75 vs warn-ink 85)
  survives.
- `chroma` — a multiplier on the template's chroma (1 = Forge). For the
  neutral families this sets the strength of the tint.
- `lightness` — an offset on OKLCH L (0 = Forge).

Alpha is kept. When a family's seed equals its Forge identity the colour is
returned untouched, so default seeds reproduce `Theme::default()` exactly
(tested), not merely within rounding.

The transform runs over every keyed colour (`StyleKey`, including the ink
ladder) and every colour in `ChromeTheme`. Chrome colours are always surface
role.

### Scalars

- `density` multiplies the geometry: paddings, spacing, control and row
  heights, toolbar and dock metrics.
- `text_scale` multiplies the type sizes (`font_size`, `font_size_title`, the
  `TextSize` scale).
- `radius_scale` multiplies `border_radius` and every `ChromeTheme` corner
  radius.
- `sans_font` / `mono_font` name font families. The app must have loaded them.

## Preset file

```toml
format = 1
name = "Aldermoor"

[seeds]
density = 1.0
text_scale = 1.0
radius_scale = 3.0
sans_font = "Alegreya Sans"

[seeds.surface]
hue = 60.0
chroma = 1.5
lightness = 0.04

[seeds.accent]
hue = 135.0
chroma = 1.0
lightness = -0.05

[overrides]
accent_face_top = "#93c264"
"ink.caption" = "#8b7a55"
padding = 6.0
```

Missing seed fields take Forge's value; unknown keys are an error, so a typo
is never silently ignored. Colours are written `#rrggbb` or `#rrggbbaa`;
`oklch(L C H)` and `oklch(L C H / a)` are also accepted for hand editing.
Override keys are `StyleKey::name()`: the field name for plain keys,
`ink.<role>`, `text_size.<step>` and `tracking.<role>` for the indexed ones.

Resolving a preset costs one pass over ~300 colours; the designer re-resolves
on every edit.

## Gaps the designer will expose (later work)

1. About 135 colour literals in widget code (`meter.rs`, `thumb.rs`, the
   slider knob, toolbar inks, badges…) bypass the theme entirely.
2. Most of them, and the ink ladder, assume a dark surface. Aldermoor puts
   dark ink on light parchment inside dark chrome: ink has to be chosen per
   surface, not from one global ladder.
3. `Background::LinearGradient` has two stops; Aldermoor's faces have three.
4. No ornaments (corner brackets, trim rules).
5. No uppercase text transform.
6. "Three primary colours" instead of one accent.
7. `ChromeTheme` overrides by dotted path.
8. The full gallery as a designer preview page.

## Phases

1. **Library.** `StyleKey` names and a complete key list; OKLCH inverse;
   `ChromeTheme` colour/radius visitors; `ThemeSeeds`, `ThemePreset`,
   `resolve()`; TOML behind `preset`.
2. **Gallery** honours `GAMEUI_PRESET`.
3. **Designer crate:** window, proof page, seed editor, token inspector with
   override toggles, open/save through `PromptDialog`, status bar.
4. The gaps above, one at a time, each checked in the designer.
