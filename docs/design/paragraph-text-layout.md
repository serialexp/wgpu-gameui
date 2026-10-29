# A multi-line field lays out its paragraphs in view

**Status:** In progress
**Last updated:** 2026-10-29

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

All landed together in the commit that added this doc (2026-10-29).

- [x] Scale checks: an 8,000-line field, focused or not, keeps at most a dozen small layouts and draws under 1 KB of text; typing at its end lays out one paragraph (`widgets/text_input_layout_tests.rs`: `a_long_field_lays_out_only_the_paragraphs_in_view`, `typing_at_the_end_of_a_long_field_lays_out_one_paragraph`)
- [x] `FieldParagraphs` (`widgets/field_paragraphs.rs`): the field's paragraphs and their heights in `RowHeights`, estimated until laid out, kept in step with each edit by splicing only the paragraphs it touched
- [x] Caret queries over paragraphs (`caret`, `byte_at`, `byte_under`, `adjacent`, `line_bounds`, `shown`), each checked against the same query over the whole text's layout on mixed text: wrapping, an empty paragraph, CRLF, right to left, a word longer than the width (`widgets/field_paragraphs_tests.rs`)
- [x] `TextInput` multi-line draws, hit-tests, selects, navigates and autoscrolls through `FieldParagraphs`, drawing only the paragraphs in view, focused or not; `nav_layout`, `render_layout`, `line_bounds` and `process_keyboard_with_layout` are gone
- [x] The scroll is kept as the paragraph at the top and how far into it (`TextInput::scroll_anchor`); `scroll_offset` is worked out from it each frame
- [x] An IME composition lays out only the caret's paragraph with the preedit in it
- [x] Syntax colours: each drawn paragraph gets the slice of the cached style ranges that falls in it (`ranges_within`)
- [x] `retained_text_field` no longer clones the value every frame: the field holds the caller's buffer going in, so it compares with that afterwards (see the note under `retained_text_field`)
- [x] A paragraph longer than 16 KB is laid out in pieces, each cut at a word break, or between graphemes when there is none in its second half; the caret at a cut ends the first piece
- [x] The layout cache budgets large layouts (over a quarter of the budget) apart from small ones, each kind to 16 MB (`shaping.rs`, `LayoutCache`; tests `a_large_layout_pushes_out_only_large_ones`, `small_layouts_push_out_only_small_ones`)

Measured with gameui alone, the 900 px focused field from "Why this exists", after the change:

| | 100 lines | 8,000 lines | one 700 KB line |
|---|---|---|---|
| Process memory after the first frame | 15 MB | 18 MB | 19 MB |
| First frame | 6 ms | 7 ms | 9 ms |
| After one more character | 15 MB | 20 MB | 21 MB |

### Outstanding

- [ ] Mouse wheel scrolls a multi-line field (deferred: today a field scrolls only to follow its caret; separate from the memory problem, worth its own change)
- [ ] The transcript's single huge message row (deferred: the same whole-block layout happens to a 700 KB message in agent-ui's transcript; that is `List` rows and markdown, not `TextInput`)

## Why this exists

A multi-line `TextInput` lays out its whole value as one text block, every frame it draws, focused
or not (`widgets/text_input.rs`, `draw`: `nav_layout`, `render_layout` and the drawn block are all
`field_text.block(self.value)`). The shared layout cache (`shaping.rs`, `LayoutCache`) keeps that
one layout, plus a caret position for every character boundary (`text_carets.rs`).

That grows with the text, not with what the field shows. Pasting 8,000 lines (700 KB) into
agent-ui's composer, a five-row `UiContext::text_area`, took its memory from about 60 MB to 1 GB.
Measured with gameui alone (a throwaway program drawing a focused 900 px multi-line field):

| | 100 lines | 8,000 lines |
|---|---|---|
| Process memory after the first frame | 19 MB | 351 MB |
| Heap at its peak while laying out | | 283 MB |
| Heap kept afterwards (layout ~45 MB, 707,561 carets × 32 B) | | 70 MB |
| First frame | 9 ms | 689 ms |
| After one more character | 20 MB | 382 MB |

Most of the peak is cosmic-text's own working state for the whole `Buffer` while it shapes
(`shape_cut_layout`), freed afterwards but kept by the allocator. The kept layout is also almost
three times the cache's 16 MB budget, so each time anything else is laid out the cache evicts
everything but it, and the rest of the UI is shaped again frame after frame. Shaping only the
paragraphs an edit touched (`shared-text-layout.md`, Phase 2) made typing cheaper but still builds
and keeps a layout of all of it.

`CodeEditor` (`crates/code`) doesn't have this problem: it draws only the lines in view. It can,
because it is monospace on a grid with no wrapping, so its caret and selection are arithmetic
(`code-editor.md`). A form field has proportional text, word wrap, right-to-left runs and IME
composition, so it needs layouts, but it can take the same shape: lay out only what is in view.

This doc is the contract for making a multi-line field's cost follow what it shows.

## Goals

- A multi-line field's memory and per-frame work depend on the paragraphs in view, not on the
  length of its value. Pasting 8,000 lines keeps the composer within a few MB of where it was.
- Everything a multi-line field does today keeps working the same: click, drag, double and triple
  click, Shift-click, Up/Down with a sticky column, line Home/End, selection drawing, autoscroll to
  the caret, IME composition and its candidate window, syntax colours, the placeholder.
- An edit lays out only the paragraphs it changed and the ones that come into view.
- Tests count work (bytes shaped, layouts made), never time, as the repo's rules ask.

## Non-goals

- Single-line fields. They stay one block: a single line can't be split by paragraph, and its
  value is rarely long. (A very long single line has the same problem as a very long paragraph;
  see Open questions.)
- Changing `CodeEditor`, `List`, the markdown view or the transcript.
- A rope or any other change to how the value is stored. It stays one `String`; `value` is public
  and hosts read it.
- Mouse-wheel scrolling of a field (Outstanding, deferred).

## Design

### Paragraphs, the unit of layout

A paragraph is the text between two line breaks, the same split cosmic-text uses
(`text::paragraphs`, which follows `LineIter`: LF, CR, CRLF, LFCR). A line break never wraps
across paragraphs and bidi resolves per paragraph, so laying each one out alone gives the same
glyphs, carets and line heights as laying out the whole text. The tests compare the two.

Each paragraph is drawn as its own `TextBlock` (`FieldText::block` over the paragraph's text, at
the paragraph's top), so the shared layout cache keeps one small entry per paragraph, and only for
paragraphs that were drawn or asked about. An unchanged paragraph hits the cache after an edit
elsewhere, so the Phase 2 reshaping path isn't needed for fields.

### `FieldParagraphs`

New, `widgets/field_paragraphs.rs`, held by `TextInput` (multi-line only):

- `starts: Vec<usize>`, where each paragraph begins in the value.
- `heights: RowHeights` (`widgets/row_heights.rs`, the transcript's height store): one row per
  paragraph, estimated until it is laid out, then measured. It already gives a paragraph's top
  (`offset_of`), the paragraph at a height (`row_at`) and the total, from Fenwick sums.
- The width and text spec the heights were measured at; a change re-estimates them all
  (`RowHeights::set_width`, `reestimate`).

An estimate is the paragraph's character count times an average advance for the font size,
divided by the width, rounded up to whole lines (one line for an empty paragraph). It only needs
to be close: the paragraphs in view are always measured before they are drawn.

**Keeping it in step with edits.** After an edit, the old and new value share a prefix and a
suffix. The paragraphs wholly inside them keep their rows (and measured heights); only the
paragraphs covering the changed bytes are spliced (`RowHeights::splice`). Finding the shared ends
is a byte comparison of the two strings, linear but cheap next to any layout. A paste of 8,000
lines is one splice of 8,000 estimated rows; nothing is laid out until it is in view.

### Caret queries over paragraphs

The functions `TextInput` uses today take the whole text's `[CaretPos]` (`text.rs`:
`caret_for_byte`, `byte_at_point`, `byte_on_adjacent_line`, `on_line`, and `text_input.rs`'s
`line_bounds`, `line_byte_range`, `line_x_for_byte`, `byte_under`). They become methods on
`FieldParagraphs` with the same meaning, each finding its paragraph first (by byte through
`starts`, by height through `row_at`) and then running today's function on that paragraph's
`caret_layout`, with byte offsets and tops shifted into the field's coordinates:

- `caret(byte)`: today's `caret_for_byte`.
- `byte_at(x, y)`: today's `byte_at_point`.
- `adjacent(byte, dir, x)`: today's `byte_on_adjacent_line`; past the first or last visual line of
  a paragraph it continues on the last or first line of the one before or after, laying that one
  out if it hasn't been.
- `line_bounds(byte)`: the visual line holding `byte`, within its paragraph.
- `lines_in(top, bottom)`: the visual lines in a band, for drawing the selection.

Global visual line numbers go away: with estimated paragraphs nobody knows them, and nothing
needs them. A line is a paragraph index and a line within it.

Every paragraph a query lays out is measured as a side effect, so its height becomes exact.

### Drawing

`draw` asks `FieldParagraphs` for the paragraphs that meet the inner box, lays out and measures
each, then draws the selection, the text and the caret per paragraph. A focused field also lays
out the caret's paragraph (autoscroll needs its caret). Nothing else is laid out. The unfocused
field does the same; today it lays out the whole value too.

`nav_layout` and `render_layout` go. Their one-frame-latency contract stays: keyboard and clicks
read the paragraphs as they stood before this frame's edits, the drawing reads them after.

### Scrolling

Today `scroll_offset` is a pixel offset. With estimated paragraphs above, a pixel offset moves
the shown text whenever one of them is measured. The scroll is kept as the paragraph at the top
and how far into it, as `List` keeps its scroll (`measured-row-heights.md`); the pixel offset is
worked out from that each frame. `scroll_offset` stays as a public field for reading, written by
the field.

### IME composition

Today a composing multi-line field lays out the whole value with the preedit spliced in. Instead
only the caret's paragraph is composed (`compose_preedit` over that paragraph, the caret rebased);
its composed height is used for that frame, pushing the paragraphs below it down, and dropped when
the composition ends.

### Syntax colours

`update_syntax_cache` keeps highlighting the whole value on an edit (that is a parse, not a
layout, and out of scope). Each drawn paragraph gets the style ranges that fall inside it, found
by binary search on their start and shifted to the paragraph's start.

### `retained_text_field`

It clones the whole value every frame to compare after drawing (`ui_context.rs`,
`let before = ti.value.clone()`): 700 KB a frame for the composer above, plus a full comparison
against the caller's buffer. `TextInput` gets an edit count that every change to `value` bumps;
the field compares counts instead.

> **2026-10-29 update:** shipped without an edit count. `value` is public and widgets write it
> directly (the search field's clear key), so a count misses edits. The field is set to the
> caller's buffer before it draws, so comparing the two afterwards finds any edit, and needs no
> copy.

## Phasing

One change, in this order, each step keeping the existing tests green:

1. The scale test, failing.
2. `FieldParagraphs` with its own tests: splitting, estimates, splicing after edits (insert,
   delete, a newline added or removed, CRLF), and each query against the same query over the
   whole text's layout.
3. `TextInput` multi-line switched over, `text_input_layout_tests.rs` and the field's tests
   passing unchanged, plus tests for the paragraph seams: Up/Down across them, a selection over
   many paragraphs drawn only in view, a click on an estimated paragraph.
4. Scroll anchoring, composition, syntax slices, `retained_text_field`.
5. Long paragraphs in pieces, and the cache split in two.

## Open questions for review

1. **A single paragraph that is itself huge** (a 700 KB line of minified JSON or base64). Paragraphs
   don't help it. Options:
   - **Lay it out in pieces of at most N bytes** (say 16 KB), each piece cut at a word break
     inside it, each starting a new visual line. Cheap and bounded. The cost is a wrap point
     that can differ from what one layout would choose, once per piece, and bidi resolved per
     piece. Recommended.
   - **Leave it**, and accept that one enormous line still costs its full layout.

   > **2026-10-29, Bart:** lay it out in pieces.
2. **Layouts larger than the cache's budget.** After this change a field doesn't make them, but a
   long markdown block or a giant label still can, and one entry over the budget makes the cache
   evict everything else on every insert. Options: don't count an over-budget entry against the
   others (it is kept while used and goes first), or don't keep it at all (it is shaped again
   every frame it is drawn, which is worse for the one using it). Recommended: the first, as a
   small separate fix.

   > **2026-10-29, Bart:** two caches. Small layouts in one with its 16 MB budget, any layout over
   > a quarter of that in another with a 16 MB budget of its own, so a large one can only push out
   > other large ones.

## References

- `crates/core/src/widgets/text_input.rs`: `TextInput::draw`, `handle_keyboard`, `FieldText`
- `crates/core/src/text.rs`: `CaretPos`, `caret_for_byte`, `byte_at_point`, `byte_on_adjacent_line`, `paragraphs`
- `crates/core/src/shaping.rs`: `LayoutCache`, `LAYOUT_BUDGET`
- `crates/core/src/widgets/row_heights.rs`: `RowHeights`
- `crates/core/src/ui_context.rs`: `retained_text_field`, `text_area`
- `docs/design/shared-text-layout.md`, `docs/design/measured-row-heights.md`, `docs/design/code-editor.md`
- agent-ui: `crates/desktop/src/main_view.rs` (the composer, `ui.text_area`)
