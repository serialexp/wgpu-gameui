# Changelog

## 0.6.0 (2026-11-13)

### BREAKING CHANGES

- refactor!: split into a workspace with crates/core and crates/media
- video phase 1 — sprites updated in place, YUV sprites, full screen
- feat(core)!: draw-call batching
- feat(icons)!: every Phosphor icon, generated from its stylesheet
- perf!: hold a text block's content as shared TextContent, not String

### Features

- transform gizmo seen through a perspective camera
- add stepped timelines and observation markers
- ListView rows can show a Thumb monogram
- video phases 3 and 4 — sound and the player widgets
- a code editor widget in a new crate, wgpu-gameui-code
- row action keys and multi-selection
- select an item with the held accent chip
- a fitted trend line with a legend entry and label
- pointer sense, and every pushed layer blocks the scene under it
- a trend fitted to a range of bars, not drawn before it
- scripted UI scenarios, one host input mapping, Forge cursors
- the mouse wheel scrolls a textarea
- a host can hand over the text of blocks it never drew
- diagnostics and completion in the code editor
- wrapping tags, Backspace removes the last, a kept field
- error and off tones
- a text input that wears the error ring
- keep the last frame and compare the next with it, stage by stage
- draw a list whose places are unchanged from the plan kept for it
- take the boxes of units that did not move from the last plan
- DrawList::append draws a recorded list again as part of another
- DrawList::append_translated places a recorded list with an offset and a clip
- DrawList::difference names the first record two lists differ in
- DrawList::heap_bytes counts what a list's buffers reserved
- lists that report their edits skip the frame comparison
- places keep their records on the GPU and upload only what changed
- prepared text, shaped once and laid out at any width

### Bug Fixes

- a press stays with the layer it began on until released
- keep the caret on the stop a click or arrow put it on
- the IME caret rect is on screen, through the list's transform
- opening again empty no longer leaves the caret past the end
- a block of spans measures as the text it draws
- measure a tag in the font its chip draws it in
- measure the culling viewport in logical px, not divided by the scale twice
- build without the icon font warning-free
- snap underlines with floor(x + 0.5) so they move with whole-pixel translations
- store a clip that shows nothing at the origin, however it was reached
- Up/Down on the first/last line go to the text's start/end
- a compaction that moved nothing changes nothing
- the layout cache counts its layouts' share of its tables
- text without a width is aligned within its widest line

### Performance Improvements

- order a list of one batch key without the grid
- read carets and hit tests from the layout text is drawn with
- place only the glyphs that can show inside a block's clip
- lay a long text out from its last version's paragraphs
- find carets by line, hash layout lookups with Fx
- cut an ellipsized line from its cached unwrapped layout
- draw MSDF glyphs and icons as instances of the unit quad
- pack the MSDF glyph instance into 64 bytes
- lay out a multi-line field per paragraph, only those in view
- hash the layout cache and glyph atlas lookups with Fx
- bound a glyph quad once instead of as two triangles
- look a cached layout up with one hash per level
- tune the draw-order grid's cell to the list
- share the empty range lists of a plain text block
- hash the per-glyph font-key lookup with Fx
- skip the shadow and glow sweeps for blocks that have neither
- keep a block's clip and effects once, not in every glyph placement
- bound untransformed chrome from its two corner values per axis
- place only the glyphs that can reach the target
- no libm calls or repeated conversions per glyph quad
- round without libm and reuse the last glyph's font key and sizing
- keep only the records that changed, and rest after a comparison gives up
- reuse prepared text for blocks unchanged since the last frame
- patch a changed list's records where its ancestors placed them
- keep lists only at the root and at nodes that clip
- push an analytic run once in append_translated
- count only passes that shape, and measure what kept shaping costs
- an ellipsized label that fits is shaped once
- text nodes keep their shaping, within a byte budget
- a changed prepared text reuses its buffer, and measuring makes no layout
- a tint or a span or range colour change is a look
- a list drawn without a paint stream keeps its place
- a placed log keeps a piece's numbers that are not zero, packed
- the glyph atlas lookup holds the tile itself
- bound an upright glyph quad from two corners
- a unit nothing that changed reaches keeps the order it had
- compare each frame with the last by default
- an edit to a prepared text shapes only the paragraphs it changed

### Tests

- ordered_paint_stress draws its barriers as triangle strips
- count shaping passes, bench a focused field, pin its caret
- sample the Phosphor icons instead of drawing all of them
- frame oracle and stage counters for the frame-reuse design
- pixel frame oracle and comparison-cost bench for the frame-reuse design
- the test targets build and pass without default features
- measure a warmed frame again when an icon font arrives

### Documentation

- fix unresolved and private intra-doc links
- usage docs on the types users see, and a doc check in the gate
- one long paragraph edited in place, through the fork

