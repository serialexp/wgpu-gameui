# Changelog

## 0.6.0 (2026-10-29)

### BREAKING CHANGES

- refactor!: split into a workspace with crates/core and crates/media
- video phase 1 — sprites updated in place, YUV sprites, full screen
- feat(core)!: draw-call batching
- feat(icons)!: every Phosphor icon, generated from its stylesheet

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
- a host can hand over the text of blocks it never drew

### Bug Fixes

- a press stays with the layer it began on until released
- keep the caret on the stop a click or arrow put it on
- the IME caret rect is on screen, through the list's transform
- opening again empty no longer leaves the caret past the end

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

### Tests

- ordered_paint_stress draws its barriers as triangle strips
- count shaping passes, bench a focused field, pin its caret
- sample the Phosphor icons instead of drawing all of them

