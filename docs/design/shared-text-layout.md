# One text layout for drawing, caret and hit testing

**Status:** Done
**Last updated:** 2026-10-22

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

- [x] Phase 0: a count of every cosmic-text shaping pass, `text::shaping_passes()` (2026-10-21; per thread rather than in `LayoutStats`, since the caret functions get a bare `FontSystem` and tests run side by side; every pass goes through `text::shape_buffer`)
- [x] Phase 0: the `focused_text_input` bench group, numbers below (2026-10-21)
- [x] Phase 0: tests pinning today's caret behaviour (2026-10-21; `widgets/text_input_layout_tests.rs`: clicks and the caret against the drawn glyphs, right-to-left edges, a password, an IME composition, both kinds of soft wrap, a final newline, a combining cluster for the ligature case)
- [x] Line starts follow cosmic-text's `LineIter` (CR, CRLF, LFCR), in `shape_layout`, `text_caret_layout` and `text_visual_layout` (2026-10-21, `text::line_starts`, ahead of Phase 1)
- [x] Phase 1: one normalisation of a block (spans joined into `content`, the transform's scale) shared by `DrawList::text` and the caret calls (5edf11b, 2026-10-21; `span_content`, `scaled_spec` and `DrawList::text_scale` in `widgets/draw_list.rs`)
- [x] Phase 1: `ShapedLayout` keeps what the caret needs: each glyph's cell x without its pen offset, each line's left and right edge, no glyph for the direction mark (5edf11b, 2026-10-21; `ShapedGlyph::cell_x`, `ShapedLine::left` / `right`)
- [x] Phase 1: caret positions and visual glyphs worked out from a `ShapedLayout` the first time they're asked for, counted in the cache's bytes (5edf11b, 2026-10-21; `text_carets.rs`, `LayoutCache::derived`)
- [x] Phase 1: `DrawList::caret_layout` / `visual_layout` take the block that is drawn; `text_cursor_positions`, `text_caret_layout`, `text_visual_layout` and their `DrawList` wrappers go (5edf11b, 2026-10-21)
- [x] Phase 1: `TextInput` takes every caret, click, selection, scroll and IME position from the layout of the string it draws (value, mask or composition) (5edf11b, 2026-10-21; `FieldText::block` builds both the drawn block and the ones its layouts are read from)
- [x] Phase 1: a multi-line field composing with the IME places its caret and IME rect on the composition's own line (5edf11b, 2026-10-21)
- [x] Phase 1: `text_select.rs` hit testing uses the cell x rather than the pen x (5edf11b, 2026-10-21; underlines too)
- [x] Phase 1: tests: the caret sits on the drawn glyphs for a block with its own font, weight, letter spacing and centred text; under a scale transform; an idle focused frame makes no shaping pass and an edit one; still right after the entry is evicted (5edf11b, 2026-10-21; `widgets/draw_list_caret_tests.rs`, `widgets/text_input_layout_tests.rs`)
- [x] Glyphs outside a block's clip are not placed (411f80a, 2026-10-21; `text_prep::glyphs_in_clip`, `cull_bounds`)
- [x] Phase 2: a long text missing from the cache is laid out from the paragraphs of the long layout last used under the same spec, shaping only the paragraphs it doesn't have, in one pass (2026-10-21; `shaping/paragraphs.rs`, `LayoutCache::reshaped`, `LayoutStats::reshaped`; tests in `shaping/paragraphs_tests.rs` compare the result with the text shaped whole). Shipped differently from the design, see the note under "Phase 2 is conditional"
- [x] The caret lookups binary-search the line they need, and a multi-line selection draws only the lines in view (afece08, 2026-10-22; `text::on_line`, `text_lookup_tests.rs`). Not in the design: found profiling the Phase 2 bench, along with SipHash over the whole content on every lookup, now Fx
- [x] Phase 3: an ellipsized line's cut is read from the cached unwrapped layout (`LayoutSpec::natural`, which measuring the block also looks up) and the cached ellipsis, so cutting shapes only the cut line (2026-10-22; `shaping/ellipsis.rs`, `LayoutCache::ellipsis_cut`, tests in `shaping/ellipsis_tests.rs`)
- [x] Closed the TODO.md entries "Ellipsis shapes the full text twice", "Caret, cursor and visual layouts shape on their own" and the gpui entry "One text layout for drawing, caret and hit testing" (2026-10-22)

### Outstanding

Nothing.

## Why this exists

Text that is drawn goes through one cache: `SharedFontSystem::layout` (`crates/core/src/shaping.rs`)
shapes a string once per `LayoutSpec` (font, size, line height, width, wrap, alignment, direction,
faces, letter spacing) and keeps the `ShapedLayout` until the memory budget pushes it out.
Selectable text (`text_select.rs`) already hit-tests through that cache.

`TextInput` doesn't. It places its caret, maps clicks to characters, draws selections, scrolls to
the caret and tells the IME where the caret is through three other functions in `text.rs`:
`text_cursor_positions` (single-line positions), `text_caret_layout` (positions with line
geometry, for multi-line) and `text_visual_layout` (cells in visual order, for right-to-left text).
Each builds its own cosmic-text `Buffer` on every call, with no cache, and the cache's counters
don't see them. In one frame of a non-empty focused field:

- Multi-line: two fresh shapes of the whole text on top of the drawn layout, `nav_layout` before
  input (`widgets/text_input.rs` ~1052) and `render_layout` after (~1228), plus one more while
  composing (~1455). That is every frame the field draws, including the ones where only the caret
  blinks. agent-ui's composer is such a field (`UiContext::text_area`, `main_view.rs`
  `draw_composer`).
- Single-line, unmasked, not composing: three, `text_visual_layout` for moving the caret
  (~1073) and for the selection and caret (~1318), and `text_cursor_positions` for scrolling to
  the caret (~1261). A mask leaves out the first; composing leaves out the second and adds one
  for the composition (~1455). A click (~1113), a drag (~1178) and a word pick (~516, as well as
  the click's) each add one more.
- An empty field shapes nothing (the functions return early).
- Typing in a long text reshapes all of it: the drawn layout is keyed on the whole string, so
  each keystroke misses the cache, and the fresh shapes above come on top.

The `DrawList` wrappers don't lay text out the way it is drawn. They pass the default sans face
and a line height of `font_size * 1.25` (the free functions take a family and a line height, but
the wrappers fill in those defaults) and no weight, style, letter spacing, faces or alignment;
`text_cursor_positions` also has no direction mark and an unlimited width. `TextInput` draws its
block with the same face, size and line height, so for left-to-right text the caret matches the
glyphs today. For a field with a set right-to-left direction the single-line positions already
differ from what is drawn (the visual layout, which does take the direction, covers the caret and
selection there). And nothing ties the two together: a field given its own font, letter spacing
or centred text would draw its caret off the glyphs.

gpui shapes a string once and uses that layout to draw it, place the caret and map a click to a
character, and caches per line, so typing in a long field reshapes only the edited line (TODO.md,
"Learned from gpui"). This doc is the contract for doing the same here.

## Goals

- One layout per string and spec, used for drawing, the caret, hit testing, selection, scrolling
  and the IME caret rect, so they can't disagree.
- A focused field makes no shaping pass on a frame where its text and settings didn't change, and
  one per edit, while the cache has room for the frame's text.
- The caret and visual data worked out once per layout, not once per call.
- Today's editing behaviour kept, pinned by tests written before the change.
- Tests that count shaping passes, never time.
- If the numbers after Phase 1 show it matters: an edit in a long text reshapes only the
  paragraph it touches.

## Non-goals

- `CodeEditor` (`crates/code`). It draws one block per piece of a line in view (`columns.rs`
  `pieces`), each its own cache entry, and places its caret by column arithmetic on the mono
  advance. Nothing here changes it.
- New editing behaviour. A caret inside a ligature, boundary affinity at a soft wrap, and the
  caret's edge in multi-line right-to-left text stay as they are (see "Behaviour kept").
- Caret calls for vertical text. No widget edits vertical text; the new calls return an empty
  layout for a vertical block.
- The single-line caret band's height (`line_height * 1.1`, centred on the field). That is
  drawing geometry, not shaping.
- `TextBlock`'s three heap allocations (TODO P1, its own entry). The field builds its block before
  input as well as after, one more per focused frame, which that entry would make free.

## Domain model

### Today

- `LayoutSpec<'a>` (`shaping.rs`): everything that decides a layout apart from the string;
  `LayoutSpec::of_block(&TextBlock)` is the spec a block is drawn with. `LayoutKey` is its hashable
  form.
- `ShapedLayout`: `glyphs: Vec<ShapedGlyph>` (font, glyph id, `rel_x` including the pen offset,
  `rel_y`, `byte_start`, `byte_end`, `advance`, `rtl`), `lines: Vec<ShapedLine>` (`top`, `height`,
  `baseline`, glyph range, `byte_start`), `shown` (bytes before an ellipsis), `size`, and
  `ink: OnceCell<…>`, worked out the first time `measure_block_ink` asks. The direction mark's
  glyph is kept, with its byte range saturated to `0..0` (~660).
- `LayoutCache` inside `SharedFontSystem`: a map from `LayoutKey` to a map from content to
  `CachedLayout`, with a byte budget counted when an entry is inserted, and least-recently-used
  eviction. `layout_stats()` reports hits, misses (`shaped`), layouts kept and bytes.
- `DrawList::text` (`widgets/draw_list.rs` ~2670) changes a block before it is drawn: span texts
  are joined into `content` (~2680), and under a scale transform the font size, line height and
  width are multiplied by the scale (~2813). The renderer (`text_prep.rs` ~131) looks each block up
  with the font system locked for the whole pass.
- Caret types (`text.rs`, public): `CaretPos { byte, x, line, line_top, line_height }`,
  `VisualGlyph { byte_start, byte_end, x, w, line, line_top, line_height, rtl }`, and the helpers
  that read them: `caret_for_byte`, `byte_at_point`, `byte_under_point`,
  `byte_on_adjacent_line`, `selection_rects`, `visual_caret_neighbor`, `visual_caret_pos`. The
  helpers take slices, so they don't care where the data came from.
- Callers of the three layout functions: only `TextInput` and its tests. agent-ui calls none.

### New

- **One normalisation.** The span joining and the transform scale move out of `DrawList::text`
  into one function both it and the caret calls use, so the caret is looked up under exactly the
  spec the renderer will use. The caret calls return coordinates in the block's own (unscaled)
  space, divided back by the scale, as the widget expects.
- **`ShapedLayout` keeps what the caret needs.**
  - `ShapedGlyph::cell_x`: the cell's left edge without the pen offset (`glyph.x` plus the line's
    offset). `rel_x` includes `x_offset`, which moves combining marks off their cell; the caret
    functions use `glyph.x`.
  - `ShapedLine::left` and `right`: the line's edges in block coordinates, alignment offset
    included. A line's first caret is at its left edge (its right for a right-to-left line) and
    its end caret at the other, not at 0 and `line_w` as today. An empty line's caret is where its
    alignment puts it: the left for start, the middle for centred, the right for end.
  - No glyph for the direction mark: `shape_layout` drops it rather than saturating its bytes.
- **Caret data worked out once.** `carets: OnceCell<Arc<[CaretPos]>>` and
  `visual: OnceCell<Arc<[VisualGlyph]>>` beside `ink`, built from `glyphs` and `lines` by the rules
  the free functions use now (see "Behaviour kept"). Their bytes are added to the entry's and the
  cache's count when they are built. An `Arc` a widget still holds after its entry is evicted
  isn't counted; a widget holds at most this frame's.
- **`DrawList::caret_layout(&TextBlock) -> Arc<[CaretPos]>` and
  `visual_layout(&TextBlock) -> Arc<[VisualGlyph]>`.** They normalise the block, lock the font
  system, look the layout up (shaping it if it isn't kept, the entry the renderer will then hit),
  clone the `Arc` out and unlock, so the lock isn't held while the widget works.
- **Single-line callers.** `text_cursor_positions`' `(byte, x)` table isn't `CaretPos` with the line
  dropped: it keeps only increasing bytes and ends runs differently. Its five callers move to
  `CaretPos` and the existing helpers (`byte_at_point`, `caret_for_byte`) one by one, with the
  Phase 0 tests pinning their results on left-to-right text first.
- **`TextInput` and its three strings.** A field draws one of: its value, its masked value, or the
  composition (value with the preedit). It asks for the caret layout of the string it draws, and
  maps bytes back with the mappings it has (`display_to_value_byte`, `value_to_display_byte`, the
  composition's caret byte). An empty value draws its placeholder, but its caret layout is that of
  the empty value (no shaping), so navigation never walks the placeholder.

  > **2026-10-21 update:** the empty value's layout goes through the cache like any other (one
  > pass the first time, kept after), rather than a fixed caret at 0, so the caret of an empty
  > centred or right-aligned block stands where its text would start. The derived data lives in
  > `Option` fields of the cache entry, filled by `LayoutCache::derived`, rather than `OnceCell`s.
  > A single-line composition's caret takes its edge from the composition's visual layout, as the
  > value's caret does, so it is right in right-to-left preedit too. Found on the way and fixed:
  > `DrawList::text` scaled the font size under a zoom but not the letter spacing (pixels), so
  > zoomed letter-spaced text was set too tight.
  >
  > From Codex's review of the change: a single-line field picks a click's byte from its visual
  > layout (`text::visual_hit`) and scrolls to its caret there, rather than from `CaretPos`, so a
  > click in right-to-left text lands on the edge the caret is drawn at (the old `(byte, x)` table
  > had the same fault). An ellipsized line's last caret stands before the ellipsis. Derived data
  > that takes the cache over its budget evicts older layouts, keeping the one in use.
- **Before and after input.** The field builds its block before input and after. Before input it
  holds the string and settings of last frame, so when nothing changed between frames (no new
  value set from outside, same width, style and direction) it is last frame's drawn entry and
  costs no shaping pass. After input it is the block that is drawn.
- **Eviction.** Other blocks can evict the entry between the field's lookup and the render. That
  costs a shaping pass, not a wrong caret: the `Arc` the field holds was made from the same spec
  and string. The "no pass on an idle frame" goal holds while the frame's text fits the budget;
  a test checks the caret is still right with a budget too small to keep anything.

### Behaviour kept

Shared glyphs don't change these, and Phase 0's tests pin them:

- Carets stand at cluster edges. Multi-line logical movement can step a character into a
  ligature's cluster (`text_input.rs` ~618), and `caret_for_byte` then rounds forward to the
  cluster's end (`text.rs` ~2614). Kept as is.
- Where a wrap breaks a word, its byte is both the end of one line and the start of the next,
  and `caret_for_byte` takes the end of the first line. Where it wraps after a space, the next
  letter starts the second line. (Pinned by Phase 0's tests, 2026-10-21.)
- Ellipsized text: caret data covers the shown bytes only, clamped to `shown` (the ellipsis's own
  glyphs have bytes past the content).

### Phase 2 is conditional

Phase 1 removes all shaping from idle frames and all but one pass per edit. What's left is the
one pass over the whole text per keystroke.

> **2026-10-21:** after Phase 1 that pass is about 66 ms for 5,000 lines (see "Phase 1
> numbers"). Bart to decide. Whether that matters depends on how long real texts
get, so Phase 0's bench measures 10, 500 and 5,000 lines, and Phase 2 happens only if the
5,000-line keystroke still costs too much after Phase 1 (Bart to decide on the numbers).

If it goes ahead: cosmic-text 0.19.0 (`Cargo.lock`) lays out each paragraph on its own
(`Buffer::line_layout` hands each `BufferLine` its own text, attributes and alignment), so
shaping paragraphs one by one with the same inputs gives the same glyphs. `shape_layout` would
split the content, look up or shape each paragraph under the spec (face ranges cut to it and
shifted to its start), and stack the results. Points to settle then:

- Splitting must follow cosmic-text's `LineIter`, which ends a paragraph at CRLF, CR and LFCR as
  well as LF (`text::line_starts` does, since 2026-10-21).
- Empty paragraphs and the empty line after a final newline are kept.
- Only the first paragraph gets the direction mark, as today ("later lines auto-detect").
  Marking each would be a change in behaviour.
- Stacking still copies and rebases every glyph per edit, and each whole-string version holds a
  copy. Paragraph caching saves the shaping, not all of an edit's cost; keeping the paragraphs as
  a list, without a stacked copy, would save the rest, at the cost of every reader walking a list.
- Many small entries cost more fixed overhead each (`LAYOUT_OVERHEAD`); the budget may need to count
  them differently.
- Vertical and ellipsized text are one stack or one line, so they keep the whole-string path.

> **2026-10-21 update:** Bart chose to go ahead, together with the clip culling. It shipped as
> paragraphs taken from another layout instead of paragraphs kept one by one. Kept one by one,
> every paragraph would sit in the cache next to the stacked copy each whole string still needs,
> about twice the glyphs; at 5,000 lines one layout is already some 10 MB of the 16 MiB budget.
> Instead, when a text isn't kept, `LayoutCache::reshaped` takes the long layout most recently used
> under the same `LayoutKey` (after an edit, the version before it), finds which of its paragraphs
> the new text still has (by their text), joins the missing ones with line breaks and shapes them
> in one pass, then stacks the lot with glyphs and bytes rebased. Nothing is kept twice, and a
> keystroke shapes the paragraph typed in.
>
> What it covers: text of at least `REUSE_MIN_BYTES` (4096 bytes, some 80 lines of 50
> characters) with a line break; below that, shaping it whole costs about as little. Vertical and
> ellipsized text keep the whole-string path, as planned, and so does text with face ranges
> (`LayoutSpec::faces`): those would have to be cut to each paragraph. agent-ui's transcript
> code spans use faces, so they don't benefit; a field's plain or syntax-coloured value does,
> since colour doesn't take part in shaping. A forced direction marks the first paragraph only, so
> the first paragraph comes from a donor's first paragraph unless the direction is `Auto`.

### Phase 0 numbers

> **2026-10-21:** `focused_text_input`, through the renderer, on a busy machine. Lines are
> about 50 characters, none wrapped. An edit is two frames (a letter typed, then erased).
>
> | field | idle frame | edit | click |
> |---|---|---|---|
> | one line | 0.17 ms, 3 passes | 0.43 ms, 7 passes | 0.26 ms, 4 passes |
> | 10 lines | 0.63 ms, 2 passes | 0.88 ms, 5 passes | 0.81 ms, 2 passes |
> | 500 lines | 20 ms, 2 passes | 33 ms, 5 passes | 17 ms, 2 passes |
> | 5,000 lines | 168 ms, 2 passes | 328 ms, 5 passes | 165 ms, 2 passes |
>
> The idle frame is what the caret's blink costs: a focused 500-line field spends 20 ms on
> it. Phase 1 is meant to make that column 0 passes.

### Phase 1 numbers

> **2026-10-21:** the same bench after Phase 1, against the Phase 0 baseline.
>
> | field | idle frame | edit | click |
> |---|---|---|---|
> | one line | 0.18 ms, 0 passes | 0.35 ms, 1 pass | 0.18 ms, 0 passes |
> | 10 lines | 0.18 ms, 0 passes | 0.36 ms, 1 pass | 0.18 ms, 0 passes |
> | 500 lines | 3.8 ms, 0 passes | 7.9 ms, 1 pass | 3.9 ms, 0 passes |
> | 5,000 lines | 39 ms, 0 passes | 210 ms, 2 passes | 42 ms, 0 passes |
>
> The passes are counted the first time. Erasing the letter returns to a value still kept, so an
> edit is one pass, except at 5,000 lines, where the two versions don't both fit the cache. For
> the same reason the edit's time includes shaping only at 5,000 lines: in Criterion's repeated
> iterations both versions are kept at the smaller sizes. One 5,000-line keystroke is then about
> (210 − 2 × 39) / 2 ≈ 66 ms of shaping and deriving, on top of a 39 ms frame.
>
> The 39 ms idle frame is no longer layout. A sample of it (macOS `sample`, 8 s) puts about 1.5%
> in layout lookups and nearly all the rest in the renderer: `text::append_placements` places
> every one of the ~250K glyphs although the field clips all but a screenful, and their quads
> are ordered (`draw_box`) and uploaded (`write_buffer`, a quarter of the samples). Culling
> glyphs outside the clip is its own fix (TODO.md, "Text places glyphs outside its clip"); it
> isn't Phase 2's to make.
>
> **2026-10-21 update:** that fix is in (`text_prep::glyphs_in_clip`). Idle frames now take
> 0.59 ms at 500 lines and 1.3 ms at 5,000; the 5,000-line edit (two frames) is 134 ms, nearly
> all of it the two shaping passes Phase 2 is for.

### Phase 2 numbers

> **2026-10-21:** the same bench after the clip culling and Phase 2, load average about 8.
>
> | field | idle frame | edit | click |
> |---|---|---|---|
> | one line | 0.19 ms, 0 passes | 0.38 ms, 1 pass | 0.19 ms, 0 passes |
> | 10 lines | 0.26 ms, 0 passes | 0.55 ms, 1 pass | 0.26 ms, 0 passes |
> | 500 lines | 0.61 ms, 0 passes | 1.2 ms, 1 pass | 0.59 ms, 0 passes |
> | 5,000 lines | 1.3 ms, 0 passes | 6.5 ms, 2 passes | 1.7 ms, 0 passes |
>
> At 5,000 lines both passes are of one paragraph: the two versions still don't both fit the
> cache, so the erase is laid out from the typed version as well. A 5,000-line keystroke is now
> about (6.5 − 2 × 1.3) / 2 ≈ 2 ms on top of its frame, against 66 ms after Phase 1. Most of
> what's left of the idle frame is hashing: each lookup hashes the whole content, two or three
> times, with the standard library's SipHash.

## Phasing

1. **Phase 0, measure and pin:** the pass counter, the bench cases, and the tests of today's caret
   behaviour, all before any change (see Outstanding).
2. **Phase 1, one layout:** normalisation, the `ShapedLayout` fields, the derived data, the
   `DrawList` calls, `TextInput` moved over (with the multi-line composition fix), `text_select`
   on cell x, and the tests in Outstanding. The match with the drawn glyphs is checked by comparing
   each caret position with the glyph cells of the same block's layout. The existing `text_input.rs`
   caret tests (`caret_layout` ~2472 and the navigation tests after it) move to the new call and
   must pass unchanged.
3. **Phase 2, paragraphs:** only if the numbers after Phase 1 call for it.
4. **Phase 3, ellipsis:** today `ellipsis_cut` (`text.rs` ~3127) shapes the content with no
   wrap, no width and start alignment, and measures the ellipsis apart (~3172). It would look that
   layout up in the cache under a fixed cut spec (the block's font, size and spacing; no wrap, no
   width, start alignment) and read the cut from its cells, keeping the ellipsis's width reserved
   as now. Toggling the block's own `ellipsize` wouldn't do: that keeps the block's wrap, width
   and alignment.

   > **2026-10-22 update:** shipped as planned, with the cut spec keeping the block's direction
   > and faces (it is what `measure_block` already looked up, so measuring and cutting share one
   > layout), and with a fix found on the way: the cut kept every glyph whose right edge fit,
   > which for right-to-left text is its end, not its start, so a Hebrew label was "cut" to nearly
   > all of itself and overflowed. It now keeps whole clusters from the start of the text, in
   > its own order, while their advances fit. A cut label shapes twice on a miss (the text
   > unwrapped, then the cut line) instead of four times with its measure, and the ellipsis is
   > looked up only when the text doesn't fit.

## Open questions for review

1. **Hand out `Arc<[…]>`, or run a closure under the lock?** `Arc` costs one allocation per
   layout the first time it's asked for and an atomic increment per call, and frees the lock at
   once. A `with_caret_layout(&block, |carets| …)` costs nothing extra, but holds the font system
   lock (not reentrant) while the widget runs its navigation, so nothing in it may measure.
   Proposed: `Arc`.

   > **2026-10-21, Bart:** `Arc`, a shared copy.
2. **Keep `CaretPos` and `VisualGlyph` as they are, or merge them?** They overlap (both have the
   byte, x, line and line box). Merging would be a bigger public change to `text.rs`'s helpers for
   little gain. Proposed: keep both.

   > **2026-10-21, Bart:** keep both.
3. **Phase 2 threshold.** What keystroke cost at 5,000 lines is acceptable after Phase 1? To
   decide once Phase 0 and Phase 1 have numbers.

   > **2026-10-21, Bart:** decided after the numbers.

## Review notes

> **2026-10-21, Codex review of the first draft.** Points taken in: the cache's miss counter can't
> show the caret functions' shaping (Phase 0 now counts every pass); `cell_x` and a width weren't
> enough (direction mark glyph, aligned line ends, the single-line table's own rules); the caret
> must be looked up after the same normalisation `DrawList::text` applies (spans, scale); the
> before-input hit holds only when nothing changed, and the value, mask and composition need
> their own byte mappings (and an empty value must not navigate the placeholder); eviction and
> the derived arrays' bytes; the multi-line composition caret bug; `LineIter`'s line endings,
> empty paragraphs and the first-paragraph mark for Phase 2; ligatures, ellipsis clamping and
> vertical text stated; `text_select` on pen x; Phase 3's cut spec; and corrections to the
> per-frame counts, the wrappers' defaults, the right-to-left mismatch today and `CodeEditor`
> drawing pieces, not lines.

## References

- `crates/core/src/shaping.rs`: `LayoutSpec`, `LayoutCache`, `shape_layout`, `ShapedLayout`
- `crates/core/src/text.rs`: `text_cursor_positions` (~2374), `CaretPos`, `text_caret_layout`
  (~2475), the caret helpers (~2614–2700), `VisualGlyph`, `text_visual_layout` (~2757),
  `selection_rects`, `visual_caret_neighbor`, `ellipsis_cut` (~3127)
- `crates/core/src/widgets/draw_list.rs`: `DrawList::text` (~2670), the caret wrappers (~937,
  ~1026, ~1046)
- `crates/core/src/text_prep.rs`: the renderer's text pass (~131)
- `crates/core/src/widgets/text_input.rs`: the call sites listed in "Why this exists"
- `crates/core/src/text_select.rs`: hit testing on the shared cache (~613, pen x at ~644)
- cosmic-text 0.19.0: `Buffer::line_layout`, `BufferLine`, `LineIter`
- TODO.md: "Caret, cursor and visual layouts shape on their own", "Ellipsis shapes the full text
  twice", "A `TextBlock` costs 3 heap allocations", and the gpui entry
- `docs/design/measured-row-heights.md`, the List work this follows in form
