# Typography roles — Design

Status: partial — phase 0a landed, 0b onward outstanding
Owner: Bart
Last updated: 2026-11-20

## Implementation status

Tracking the gap between this design and what's on the main branch.
Tick items as they land; move whole sections to "done" once everything
inside them is checked.

### Done

- [x] **Phase 0a — the role table.** `FontFamily`, `TextStyle`, `TextRole`,
      `Typography` in `src/typography.rs`; `Theme::display_font` and
      `Theme::typography` (both built-ins derive theirs from a private
      `TypeScale`); `StyleOverlay::{set_text_style, text_style}`;
      `StyleResolver::{text_style, role_size, role_line_height, family_font,
      role_font, role_block, role_block_sized, style_block, role_width,
      style_width}`. Additive; nothing reads it yet. Extended beyond the
      plan: `style_block` / `style_width` for a style no role holds, and a
      `Value` app role (a prominent stat or name, which Slab sets in its
      display face).

### Outstanding

- [ ] **Phase 0b — tabular figures.** `TextBlock::tabular` threaded through
      shaping, every layout and prepared-text cache key, the selection
      snapshot (`text_select.rs`), frame-diff placement classification and
      natural-width measurement (`measure.rs`).
- [ ] **Phase 0c — style-aware centring.** `vcentered_text_y` /
      `x_centered_text_y` / `font_vmetrics` take the block's weight and
      style and centre the displayed (cased) content.
- [ ] **Phase 0d — the per-site role table.** The survey of every text site
      (chrome scale and body text) mapped to a role, recorded under
      "Roles" below, before Phase 1 starts. A site that fits no role gets a
      new one.
- [ ] **Phase 1 — the chrome scale moves to roles.** Every `sans_block`,
      `mono_block`, `caption_block`, `sans_width`, `mono_width`,
      `text_size(TextSize::…)` and `Tracking::…` user (51 files) names a role.
      Then `TextSize`, `Tracking`, their `StyleKey`s, `Theme::text_sizes`,
      `Theme::tracking` and those resolver methods are deleted.
- [ ] **Phase 2a — body text moves to roles.** Every `StyleKey::FontSize`,
      `StyleKey::FontSizeTitle`, `text_block`, `title_block`,
      `Theme::text`/`title` and `theme.font_size*` user (about 60 files)
      names a role, including geometry derived from text size (row heights,
      badge heights, `ListView`'s second line). Then `Theme::font_size`,
      `Theme::font_size_title`, both keys and those methods are deleted.
- [ ] **Phase 2b — editable text.** `FieldText` and its `MeasuredFor`
      cache carry the resolved `TextStyle` (family, size, weight, italic,
      tracking, tabular, line height); carets, selection and scroll use it.
- [ ] **Phase 2c — the font stack.** `UiContext`'s root font-stack entry
      comes from the `Body` role; `MeasureContext::text_block` becomes
      role-based and a push overrides only what it sets.
- [ ] **Phase 2d — Markdown.** `MarkdownStyle::forge` takes its body, code
      and heading styles from roles; emphasis composes over them.
- [ ] **Phase 3 — presets and the designer.** `ThemeSeeds::text_scale`
      scales every role's size (half-pixel rounding); a `display_font` seed;
      the designer's type-scale preview lists roles instead of `TextSize`
      steps; its token list loses the deleted keys.
- [ ] **Phase 4 — parity check.** The widget gallery in Forge and Overlay
      matches commit `272f7d0` pixel for pixel, except the differences
      listed under "Permitted differences".
- [ ] **Phase 4b — a contrast typography test theme.** A test-only theme
      whose roles differ sharply (Plex Mono display headings, italic tabs,
      bold buttons, tracked caps lists, tabular numbers), rendered in the
      gallery and used by unit tests at scale factors 1, 1.25 and 2.
- [ ] **Phase 5 — performance check.** See "Performance" below. The
      result is recorded here.
- [ ] **Phase 6 — consumers.** The commit footer spells out the breaking
      change. Citybuilder's client moves to roles (in its own repo).
      agent-ui and mesh-forge, which use gameui by path, get a heads-up in
      `CURRENT_TASK.md`.
- [ ] **Defer — per-role typography in presets and the designer.** Presets
      serialize every role's style and the designer edits it. Out of scope
      (Bart, 2026-11-20).
- [ ] **Defer — snapping the ad-hoc body factors to the Forge scale.**
      Phase 2 keeps today's computed sizes (tabs at 12 × 0.8 = 9.6px, and
      so on). Moving Forge's roles onto its 9/10/10.5/11/11.5 scale is a
      separate visual decision.

## Why this exists

A theme can recolour every pixel gameui draws, but it cannot change what
text looks like beyond two sizes. Today widgets pick their text in one of
three ways:

- **The Forge chrome scale.** `sans_block`, `mono_block` and
  `caption_block` fix the family (sans or mono) in the call. They take a
  `TextSize` step (Forge's Caption 9, Meta 10, Dense 10.5, Row 11 and Menu
  11.5) and, for captions, a `Tracking` role. A theme can resize the
  steps. It cannot make field labels a serif, or captions anything but
  upper-case.
- **Body text.** `StyleKey::FontSize` and `FontSizeTitle` are read with a
  factor at the call site: tabs are `font_size × 0.8`, document tabs
  × 0.9, table cells × 0.75, column headers × 0.7, combo boxes × 0.9. The
  factors are the only record of which text is which.
- **Raw `TextBlock`s** are built with `theme.font.clone()` and one of the
  sizes above. Some are measured with `measure_text(…, None)`, the default
  font rather than the one drawn, so a theme with another font mis-centres
  them.

Citybuilder's "Slab" design needs exactly what this blocks (Claude Design
project "Citybuilder interface design", the Flow pages). Its tab labels,
list names, values and headings are Spectral, a serif, at 14–30px, regular
or italic. Its buttons and metadata are IBM Plex Sans at 10.5–13px. Clocks
and counts use tabular figures. The same widget (a tab) is 9.6px sans in
Forge and 15px serif in Slab, so both the size and the family must come
from the theme, keyed by what the text *is*.

This doc is the contract for that: every piece of text a widget draws names
a **role**, and the theme maps each role to a complete text style.

## Goals

- One source of truth for text styling: `Theme::typography`, indexed by
  `TextRole`. No widget picks its own family, size, weight, style,
  tracking, case, figures or leading.
- A third font family, *display*, beside sans and mono. It falls back to
  sans when a theme names none.
- Tabular figures as a role property, through OpenType `tnum`.
- Forge and Overlay look as they do at `272f7d0`, apart from the listed
  bug fixes (Phase 4 is the proof).
- Measuring, centring, carets and selection always use the role's real
  face, so they are right in any theme.
- Role lookup is an array index. The non-tabular path adds no allocation,
  and neither does any cache hit.

## Non-goals (v1)

- Per-role typography in preset files or the designer (deferred, above).
- Changing Forge's or Overlay's look. Unifying the ad-hoc factors comes
  later.
- New rich-text capabilities. `TextSpan` stays colour and underline only;
  mixed faces stay `FaceRange`. Phase 2d only makes Markdown's existing
  model inherit from roles.
- The other Slab gaps: underline-rule tabs, a selection edge bar and
  gradient severity chips. They are chrome, not typography, and get their
  own work.
- Building the Slab theme. It lives in the citybuilder client (Bart,
  2026-11-20), which loads Spectral itself.

## Domain model

```rust
/// Which of the theme's three fonts a role uses.
pub enum FontFamily { Sans, Mono, Display }

/// Everything about how one role's text looks, except its colour (that is
/// the `Ink` ladder's job, chosen by the widget per state).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub family: FontFamily,
    pub size: f32,          // logical px
    pub line_height: f32,   // multiple of size; 1.25 (LINE_HEIGHT_RATIO) in Forge/Overlay
    pub weight: Weight,     // cosmic-text Weight
    pub italic: bool,
    pub tracking: f32,      // em; letter spacing = tracking × final size
    pub caps: bool,         // drawn upper-cased (see "Caps")
    pub tabular: bool,      // OpenType `tnum`
}

/// What a piece of text is. Widgets name one; the theme decides its look.
pub enum TextRole { … }     // ALL, name(), usable as an index

/// The role → style table a theme carries.
pub struct Typography([TextStyle; TextRole::ALL.len()]);
```

`Theme` gains `display_font: Option<FontHandle>` and `typography:
Typography`. `display_font` resolves like `font`: `None` falls back to
sans. `TextStyle` is `Copy`, so reading one copies it out of an array.

Sizes are logical pixels, as all text sizes are today. The draw transform
and the scale factor apply to size, leading and spacing together when text
is recorded, and measurement stays in logical coordinates.

### Roles

Roles name what the text is, not how big it is. Where two widgets show the
same kind of text at different sizes today, they get separate roles, so
parity holds and a theme can still style them apart. The rows below are
the set known so far. Phase 0d completes the list from the full survey and
records it here, with each role's callers, before Phase 1 starts.

| Role | What it is | Forge at `272f7d0` |
|------|------------|-------------|
| `Label` | Field labels (`FieldLabel`, file field) | mono caps 9, 0.14em |
| `Section` | Panel captions, section headers | mono caps 9, 0.16em |
| `Badge` | Badge text | mono caps 9, 0.08em |
| `BadgeCompact` | Compact badges (case kept) | per survey |
| `Property` | Inspector property labels | mono caps 9, 0.10em |
| `Axis` | Chart axis and tick labels | mono 9 |
| `Meta` | Shortcuts, counts, metadata, readouts | mono 10 |
| `Code` | Code popups, timecodes, Markdown code | mono 10.5 |
| `Hint` | Small secondary sans text | sans 10 |
| `Dense` | Dense row text, filter chips | sans 10.5 |
| `Row` | Panel rows, legends, tooltip text | sans 11 |
| `MenuItem` | Menu and context-menu items | sans 11.5 |
| `ListItem` | List and sheet item labels | sans 11.5 |
| `ListItemMono` | `ListRow::mono` labels | mono 11.5 |
| `Body` | Running text, inputs, the font-stack root | sans 12 |
| `Button` | Button labels | sans 12 |
| `Control` | Checkbox, radio, toggle and setting labels | sans 12 |
| `Tab` | `Tabs` labels | sans 9.6 |
| `DocTab` | `DocTabs` labels | sans 10.8 |
| `SpanTab` | `SpanTabs` labels | sans 11.5 |
| `Cell` | Table cells | sans 9 |
| `ColumnHeader` | Table column headers | sans 8.4 |
| `Title` | Panel and dialog titles | display→sans 15 |
| `Heading` | App section headings | display→sans 15 |
| `Display` | App screen titles | display→sans 24 |
| `Value` | App prominent values: a stat, a name | sans 12 |
| `Numeric` | Clocks, counters, quantities | sans 12, tabular |

"display→sans" means family `Display`, which in Forge and Overlay falls
back to sans. So the built-ins look unchanged, and setting `display_font`
visibly changes titles and headings. `Heading`, `Display`, `Value` and
`Numeric` have no gameui caller. They are app roles that games use for their own screens.

`Tracking::Brand` has no caller today and is not carried over.

Overlay's column is its current sizes (body 15, title 18, scale
11/12/13/14/14) run through the same factors.

### Reading a role

```rust
impl StyleResolver<'_> {
    fn text_style(&self, role: TextRole) -> TextStyle;   // overlay, then theme
    fn role_size(&self, role: TextRole) -> f32;
    fn role_line_height(&self, role: TextRole) -> f32;   // px, for geometry
    fn role_font(&self, role: TextRole) -> Option<FontHandle>;
    fn role_block(&self, content, x, y, role: TextRole, ink: Ink) -> TextBlock;
    fn role_block_sized(&self, content, x, y, role: TextRole, ink: Ink, size: f32) -> TextBlock;
    fn role_width(&self, list: &mut DrawList, text: &str, role: TextRole) -> f32;
}
```

`role_block` applies family, size, line height, weight, italic, letter
spacing, case and `tnum`, then the ink. A widget that needs a colour
outside the ink ladder (a severity or an accent) sets it on the returned
block.

**Computed sizes.** A few sites size text from geometry rather than the
theme: `ProgressBar`'s labeled variant uses `rect.height × 0.7`, and
`Thumb`'s monogram uses the thumbnail size. They call `role_block_sized`,
which takes everything but the size from the role and derives letter
spacing and line height from the *final* size. Such text scales with its
widget, not with `text_scale`. The survey in 0d lists every such site.

**Precedence.** The role supplies defaults. An explicit widget setting
wins over the role: `TextInput::font`, a `FaceRange`, or a font-stack push
in `UiContext`. A push overrides only the attributes it sets. The root of
the stack is `Body`. Measurement goes through the same resolution as
drawing, so the two cannot disagree.

**Measurement.** `role_width` measures borrowed `&str` with the role's
face, weight, style, spacing and features through the existing
allocation-free width path, extended with the feature flag. It builds no
temporary block. With `caps` it measures the cased text (see below).

**Centring.** Optical centring (`vcentered_text_y`, `x_centered_text_y`)
reads metrics for the block's actual weight and style, and picks its band
(x-height or cap-height) from the *displayed* content. Caps are applied
first, so a caps role centres on the cap-height band. Inputs keep their
content-independent centring.

`StyleOverlay` gains per-role overrides: a short `Vec<(TextRole,
TextStyle)>` with a linear scan, like its keyed entries. A subtree can
then restyle one role without cloning the theme.

### Caps

`caps` upper-cases the content when the block is built, as `caption_block`
does today, so plain captions allocate exactly what they do now.

Upper-casing can change byte length (`ﬃ` becomes `FFI`). Byte offsets into
the source text therefore have to be remapped:

- **Ranged text** (highlight ranges in `ListView`, `FaceRange`s, spans):
  the cased content is built a character at a time, together with an
  offset map, and every range is moved through the map. This happens only
  when ranges are present.
- **Editable text** (`TextInput`, `TextArea` and every `FieldText` user)
  ignores `caps`. Editable text shows what was typed. Carets and selection
  keep their source offsets.
- **Selectable static text** maps selection back through the same map, so
  a copy yields the displayed text.

Tests cover a ligature and a German `ß` (which upper-cases to `SS`) in a
highlighted list row and in a selection.

### Tabular figures

`TextBlock` gains `tabular: bool`. When it is set, shaping passes
cosmic-text `FontFeatures` with `tnum` (`Attrs::font_features`, as in the
patched cosmic-text revision this repo pins). The flag joins everything
that already keys or compares weight and style:

- the layout cache and the prepared-text key;
- the `DrawnBlock` snapshot and the `LayoutSpec` it rebuilds
  (`text_select.rs`);
- frame-diff classification, where a flag change means new placement
  (`render/frame_diff.rs`);
- natural-width measurement in `MeasureContext`, so overflow checks and
  constrained layout agree.

`FontFeatures` holds a `Vec`. An empty one does not allocate, so the
proportional path costs nothing. A tabular shaping *miss* allocates one
small vector, beside the allocations shaping already makes. Cache hits
allocate nothing. A test changes only the flag and checks that
measurement, prepared reuse, selection and frame reuse all see the change.

### Editable text

`FieldText` today carries the size, wrap, width, direction and font, and
`MeasuredFor` compares exactly those. It switches to holding the resolved
`TextStyle` (with the resolved font handle), and `MeasuredFor` compares
the whole style. A role change, including one arriving through an overlay
while the input keeps its state, re-measures the heights. Carets,
selection and scrolling read the same layout that is drawn. A test changes
an overlay's `Body` style under a focused multi-line input and checks the
caret position against the drawn layout.

### Markdown

`MarkdownStyle::forge(theme)` takes `font` and `size` from `Body`, `mono`
and `code_size` from `Code`, and the line height from `Body`. Heading
sizes stay multiples of the body size through `heading_scale`. The base
weight and italic come from the role. Emphasis composes over it: bold
raises the weight to bold, and `*em*` toggles italic. Fields set after
`forge` still win, as they do today.

## Presets and the designer

`text_scale` scales every role's size and rounds to half pixels. At 1.0 it
is the identity: tested for every role in both built-ins. Other scales
round each role on its own, rather than rounding the base and then
multiplying. At 1.1, Forge's tabs become 10.5px where today they are
13 × 0.8 = 10.4px. This is an intended change, covered by a test.

The `display_font` seed sits beside `sans_font` and `mono_font`. A preset
naming a deleted key (`text_size.*`, `tracking.*`, `font_size`,
`font_size_title`) fails with the existing "unknown style key" error. No
compatibility path is kept (Rule 8). The shipped presets do not use those
keys.

The designer's type-scale preview (`designer/src/preview.rs`), built on
`TextSize::ALL`, becomes a role preview: each role's name in its own
style. The seed editors gain the display font.

## Performance

The work per text block is unchanged except for one array read and a
`Copy` of `TextStyle`. Phase 5 measures that claim:

- Run the ui-compare `gameui` grid and `text` scenes at `272f7d0` and
  after Phase 2, on one machine, with five runs each. Compare the medians.
  Any regression beyond the larger of 2% and the run-to-run spread gets
  investigated.
- Add a role scene to the gameui bench. It has caps captions, tabular
  counters changing every frame, an overlay restyling a subtree, and
  proportional rows. It reports, per frame, the allocation count (the
  bench's counting allocator), shaping calls and cache hit rate, beside
  timing. Expectation: no allocations for steady-state frames with
  unchanged content, and one shaping call per changed counter.

## Phasing

Each phase is one or more commits. Each one builds warning-free and passes
`cargo test --lib`, clippy and the doc build (gameui `CLAUDE.md`).

0. **Foundations.** The role table (0a), tabular figures (0b), centring
   (0c), and the full per-site table recorded here (0d). Tests:
   - every role resolves;
   - an overlay override wins;
   - display falls back to sans;
   - `role_width` equals the drawn block's width in a non-default font;
   - caps, tracking and leading are applied;
   - `role_block_sized` re-derives spacing;
   - tabular digits have equal advances;
   - the flag-only transition test;
   - italic and bold centring use their own metrics.
1. **Chrome scale.** Migrate the 51 files. Delete `TextSize`, `Tracking`
   and their keys and helpers.
2. **Body text, editable text, font stack, Markdown** (2a–2d). Then
   delete the body keys and fields.
3. **Presets and designer.**
4. **Parity** against `272f7d0` in both themes, plus the contrast theme
   (4b).
5. **Performance.**
6. **Consumers.**

### Permitted differences

The parity check allows only these differences. Each is added here as the
migration finds it, naming the site and the bug:

- Sites that measured with the default font but drew in the theme font.
  They now centre correctly in a theme whose font is not the default. In
  Forge and Overlay both fonts are Plex, so no pixel difference is
  expected even there.

## Decisions

- **Roles over size steps** (Bart, 2026-11-20). Widgets name what text
  is; themes decide family, size, leading, weight, style, tracking, case
  and figures.
- **Replace both the chrome scale and body text** (Bart, 2026-11-20).
  There is one source of truth. `TextSize`, `Tracking`, `FontSize` and
  `FontSizeTitle` are removed outright, not kept beside the roles.
- **Presets get scale and the display font only** (Bart, 2026-11-20).
  Per-role preset typography is deferred.
- **The Slab theme lives in citybuilder** (Bart, 2026-11-20). gameui stays
  generic and bundles no display font.
- **Parity first.** Phase 2 reproduces today's computed sizes exactly, so
  pixel comparison can verify the refactor. Changing any look comes later.
- **A table, not keyed values.** A `TextStyle` is a compound value. It
  lives in `Typography` with resolver accessors, like `ChromeTheme`, rather
  than widening `StyleValue` with a third shape.
- **Separate roles over variants.** Widgets showing similar text at
  different sizes (the three tab kinds, menus and lists) get separate
  roles rather than a variant parameter. That keeps parity, and keeps each
  independently styleable.
- **Editable text ignores caps.** Carets and selection keep source
  offsets, and a field shows what was typed.

## Questions only building can answer

- [ ] Whether the 0d survey turns up text that no role and no
      `role_block_sized` call can express. Settled by the survey. Such a
      site would get its own design note here.

## Review

**Codex (2026-11-20)** gave twelve findings, all checked against the code
and all accepted:

- **Editable text and its measured-for cache.** Added "Editable text" and
  Phase 2b.
- **Caps vs byte offsets.** Added "Caps": an offset map for ranged text;
  editable text ignores caps.
- **Font-stack and explicit-override precedence.** Added "Precedence" and
  Phase 2c.
- **One `Tab` role cannot cover `Tabs`, `DocTabs` and `SpanTabs`.** These
  became separate roles, as did menus vs lists, compact badges and mono
  list rows. The full table is now required before Phase 1 (0d).
- **Tabular propagation.** Now lists the selection snapshot, frame diff and
  natural-width measurement, and adds a flag-only transition test.
- **Line height missing from the style.** `line_height` is now a role
  property, and geometry reads `role_line_height`.
- **Centring with the regular face.** Phase 0c.
- **The rich-text statement was wrong.** Corrected. Markdown now inherits
  from roles (Phase 2d).
- **Computed sizes and spacing.** `role_block_sized` derives spacing and
  leading from the final size.
- **Preset rounding and the designer.** The rounding change, identity at
  1.0, the display-font effect, the error for deleted keys and the
  designer's role preview are now specified.
- **Allocation claim and benchmark coverage.** The claim is narrowed to
  the proportional path and cache hits. A role bench scene counting
  allocations and shaping calls is added, with a defined comparison.
- **Parity can't prove the new behaviour.** Added the contrast test theme,
  DPI scales and a pinned baseline commit.

**Antigravity:** not run; its quota was exhausted. Bart waived it
(2026-11-20: "Just be ok with the codex one and proceed").

## References

- `src/style.rs` (`TextSize`, `Tracking`, the `*_block` helpers),
  `src/theme.rs`, `src/theme/overlay.rs`, `src/preset.rs`,
  `src/measure.rs`, `src/widgets/field_paragraphs.rs`,
  `src/widgets/draw_list.rs` (centring), `src/text_select.rs`,
  `src/render/frame_diff.rs`, `src/markdown/`.
- Citybuilder design: Claude Design "Citybuilder interface design", Flow
  1–6 (Slab).
- `docs/design/theme-designer.md` (presets, seeds).
