# Prepared text: shape once, lay out at any width — Design

Status: partial — Phases 0–2 landed, Phases 3–5 outstanding
Owner: Bart
Last updated: 2026-11-12

## Implementation status

Tracking the gap between this design and what's on the main branch.
Tick items as they land; move whole sections to "done" once
everything inside them is checked.

### Done

- [x] **Phase 0 — count shaping apart from layout, and measure both.**
      `shaping_passes()` and `LayoutStats::shaped` count a pass only when a
      paragraph in it had no shaping yet, counted where shaping happens, so
      a cached layout built from kept paragraphs alone counts nothing
      (`a_layout_made_only_from_kept_paragraphs_is_not_counted_as_shaped`).
      The `relayout` bench (`crates/core/benches/relayout.rs`), the
      `kept_shaping_bytes` measurement (`crates/core/tests/`, ignored), and
      `a_new_or_changed_label_is_shaped_once_per_width_it_is_laid_out_at`
      in the retained tests pin today's two shapes per changed hugging label.
      Results below ("Phase 0's numbers"). Changed from the plan: the count
      is of passes that shaped, not of paragraphs, so the existing counts
      keep their meaning; `laid_out` and the relayout check move to Phase 1,
      which first makes layouts from kept shaping; the whole path is
      `ui-compare-gameui-retained`'s `rows-one`, which already measures it
      with the renderer, rather than a second bench.
- [x] **Settle: what laying out again costs against shaping** (Phase 0's
      bench): about a twentieth (below).
- [x] **Settle the tree's shaping budget**: 64 MB by default (Bart,
      2026-11-12), about 8,500 short labels at Phase 0's 7.5 KB. A changed
      text is shaped and drawn in the same frame, the most recently used, so
      the budget decides only whether an unchanged text reshapes on a
      resize; Phase 4 checks what the bench scenes hold under it.
- [x] **Phase 1 — one extraction, and `PreparedText` in core.**
      `shape_cut_layout` is `set_buffer` (the shaping half, into
      `Buffer::new_empty`), `set_layout` (line height, width, wrap, and
      alignment set on every line) and `extract`, shared by the cache and
      `PreparedText` (`crates/core/src/shaping/prepared.rs`).
      `TextMeasurer::prepare`, `PreparedText::measure` / `layout`,
      `PreparedLayout`, `TextBlock::prepared` / `with_prepared`;
      `ShapedLayout::ink` and the handle's carets and visual glyphs are
      `OnceLock`; `LayoutStats::laid_out`; `text::lay_out_buffer` asserts in
      debug builds that a relayout shapes nothing. `SharedFontSystem` has a
      process-unique id, stamped on the handle with the generation. Tests in
      `shaping/prepared_tests.rs`. Unlike the plan, preparing also lays the
      text out as it is measured, so `laid_out` counts only layouts made
      from kept shaping: a hugging label is shaped once and laid out once
      more, not twice.
- [x] **Phase 2 — every lookup of a recorded block honours the handle.**
      `SharedFontSystem::resolve` (and `resolve_carets` / `resolve_visual`)
      returns the handle's layout when it is exactly what the site looks up.
      Every site in the table goes through it: `prepare_texts`,
      `refresh_texts`, underlines, highlights, `caret_layout` /
      `visual_layout`, `measure_block_ink`, `TextSelection` (`DrawnBlock`
      keeps the handle) and markdown's `shaped`. Tests: the renderer places
      what the cache gives and shapes nothing, through preparing and a
      refresh, and stale handles (new fonts, a changed block, a scale) fall
      back to the cache (`text_prep_tests.rs`, `draw_list_caret_tests.rs`,
      `text_select.rs`).
- [x] **Settle: cosmic-text's relayout of a kept `Buffer` gives the glyphs a
      fresh shape at that width gives.** Bit for bit, in every case
      `prepared_tests.rs` walks one prepared text through and back: ten
      contents (right to left and mixed, line breaks with CRLF and empty
      lines, an empty and a blank text, a word wider than the width,
      ligatures, CJK) in three directions, five widths with fractional ones
      and zero, five alignments, four wraps, ellipsis on and off, two line
      heights; faces, letter spacing both ways, spans, vertical text.

### Outstanding

- [ ] **Phase 3 — an ellipsized text that fits is shaped once when only
      drawn.** When its unwrapped layout is not in the cache, the block is
      shaped uncut under its ellipsis key first: if it fits, that is its
      layout; if not, the cut is read from its advances and `prefix…` shaped
      (Bart, 2026-11-12). A cut label still shapes `prefix…` fresh (Bart,
      2026-11-12).
- [ ] **Phase 4 — the retained tree prepares its texts.** A text node
      prepares at its first measure (deferred rows cost nothing), keeps its
      `PreparedText` for its life within the tree's byte budget (Bart,
      2026-11-12), works out its drawn layout when its layout is settled, and
      draws with the handle. It prepares again only when what shaping
      depends on changes, or the font system or its generation does.
- [ ] **Phase 5 — the rows bench and the docs.** `rows-one` shapes one text
      a frame (about 229 in 230 frames, as immediate mode); TODO, TODO_DONE,
      `retained-mode.md`, `shared-text-layout.md` (a note under its "one
      cache" premise) and the bench README updated.
- [ ] **Defer: a prepared text editing its paragraphs in place.** The shared
      cache shapes only the paragraphs a long text's edit touched
      (`shaping/paragraphs.rs`); a `PreparedText` prepared again on an edit
      shapes the whole text. cosmic-text's `Buffer::lines` are per paragraph,
      so replacing only those that changed is possible later. Until then an
      edit to a long text in a retained node shapes it whole: a 100 KB
      paragraph is the representative size to time when this is taken up.
- [ ] **Defer: the unwrapped layout of right-to-left text sits at
      x ≈ 8.5e37** (found by both reviewers, 2026-11-12): `natural()` has no
      width, `shape_cut_layout` lays it out at `f32::MAX / 4`, and
      cosmic-text starts a right-to-left line at the width, so the glyphs'
      x lose their precision. Only advances and sizes are read from it
      today (the ellipsis cut, measuring), so nothing is drawn wrong; noted
      in `TODO.md`.

## Why this exists

A text in the retained tree is shaped twice whenever it is new or changed;
immediate mode shapes it once. The comparison counted 512 layouts in 230
`rows-one` frames against 229 (`bench/ui-compare/README.md`, `TODO.md` "text
shaped twice").

The retained layout measures a text at the width it is offered
(`crates/retained/src/layout.rs`, `measure_text`: the `Avail` width, or 1e6
unbounded) and sizes the node; a node that hugs its text is then as wide as
the text, ceiled. It is drawn (`own_drawing.rs`, `paint_node` through
`text_place`) with `max_width` the node's content width. `LayoutSpec::
of_block` keeps the block's `max_width` whatever the wrap (`shaping.rs:267`)
and the cache keys on it (`LayoutKey`), so the measure and the draw are two
entries, each shaped. A node of exact width measures and draws at the same
width and is shaped once already.

Two fixes were turned down (Bart, 2026-11-11): drawing at the width measured
at (right for start-aligned text only), and dropping `max_width` from the key
for unwrapped start-aligned text (a special case). Asked for other ideas
(2026-11-12), Codex and Antigravity both pointed at the same root: core keeps
only the finished layout. `shape_cut_layout` (`shaping.rs:824-929`) builds a
cosmic-text `Buffer`, shapes it, copies out the glyphs and drops the buffer,
though cosmic-text keeps each paragraph's shaping (`ShapeLine`) across a
change of width or alignment and only lays it out again
(`cosmic-text-0.19.0/src/buffer.rs:818`, `buffer_line.rs:148`, `:245`).
Two layouts need not be two shapes.

Bart chose (2026-11-12) an explicit prepared text carried from measuring to
drawing, over a second cache of shaped paragraphs beneath the layout cache,
aligning at draw time, or a flag to align within the hugged width.

## Goals

- A retained text node is shaped once per change of what its shaping depends
  on; measuring it, sizing it and drawing it at another width lays it out
  again without shaping.
- What is drawn is exactly what a fresh `TextBlock` at the drawn width would
  draw: line breaks, alignment (all five, left-to-right and right-to-left),
  wrap modes, ellipsis, vertical text, letter spacing, faces, spans.
- A width change later (a resize) lays out again without shaping, the node
  keeping its shaping for its life, within the tree's byte budget.
- A fitting ellipsized text that is only drawn is shaped once, in immediate
  mode too.
- The API is public: any caller that measures a block and then draws it can
  prepare it (Bart, 2026-11-12).
- Counted, not timed: tests assert paragraphs shaped and layouts made.

## Non-goals (v1)

- A cut ellipsized label keeps shaping `prefix…` fresh, so the glyphs at the
  cut (ligatures, contextual forms, bidi) are exact (Bart, 2026-11-12).
- An immediate caller that measures an ellipsized block (unwrapped, as
  `measure_block` does) and then draws it without preparing keeps its two
  shapes; preparing is the way to one.
- No change to the shared cache's keys or what it shares between callers.
- Editing a prepared text's paragraphs in place (deferred, above).
- Retained text selection, carets and input (deferred in `retained-mode.md`).

## The API (core)

```rust
// Shapes `block` as `DrawList::text` would record it (spans joined, the
// same normalisation) and keeps the shaping.
let mut prepared: PreparedText = measurer.prepare(&block);

// Its size, as `measure_block` gives it: unwrapped for an ellipsized
// horizontal block, else at `block.max_width`. Lays out; shapes nothing.
let (w, h) = prepared.measure(&mut measurer, &block);

// The layout `block` is drawn with. Kept, so asking again is free.
let layout: PreparedLayout = prepared.layout(&mut measurer, &block);
list.text(block.with_prepared(layout));
```

### What shaping depends on, and what only layout does

| Shaping (a change prepares again) | Layout only (a change lays out again) |
| --- | --- |
| the drawn content (spans joined), face ranges, font, size, weight, style, letter spacing (em, from the size), direction (its marks), vertical | `max_width`, `align`, `wrap`, `ellipsize`, line height |

A layout sets every layout field on the `Buffer` each time, alignment on
every line (`BufferLine::set_align`, including back to `None` for `Start`),
so a prepared text moved through A → B → A, centre → start, wrap on → off
lays out what a fresh shape would. Vertical text keeps the buffer unsized
and unaligned, as `shape_cut_layout` does, and the shared extraction centres
its column within the width.

### The types

- `PreparedText` owns the `Buffer` (made with `Buffer::new_empty`, so no
  empty string is shaped), the text it shaped (with its direction marks, or
  stacked for vertical text) and the paragraph starts the extraction needs,
  the shaping half of the spec, the font system's identity and generation,
  and its last two finished layouts with their keys (the measured and the
  drawn one, so a hugging node measured at the offered width and drawn at
  its content width lays out each once and then neither). It holds no lock;
  each call locks the shared font system as `TextMeasurer` does.
- `measure` and `layout` take a block. One whose shaping half differs from
  the prepared one, or a font system or generation that moved, is prepared
  again (counted as shaping) rather than refused, so a caller that changes
  a field and forgets to prepare gets the right text and the counters say
  what it cost.
- `PreparedLayout` is an `Arc` of the finished `ShapedLayout`, with the
  `LayoutKey`, the content, and the font system's identity and generation
  it was made for. `TextBlock` clones (`Placement::text`, used by
  `append_translated` and patching) keep it.
- `ShapedLayout::ink` and the caret and visual data kept for a prepared
  layout are `OnceLock`, not `OnceCell`, so `Arc<ShapedLayout>`, `TextBlock`
  and `DrawList` stay `Send` and `Sync`; compile-time assertions pin it.
- `TextBlock::prepared: Option<PreparedLayout>` (`with_prepared`). Its fields
  stay public: the handle is checked where it is used, never trusted.
- Laying out again goes through one function beside `text::shape_buffer`,
  which counts the layout and asserts in debug builds that no line needed
  shaping.

### Ellipsis

A prepared ellipsized block is shaped unwrapped. `measure` returns that
size, as `measure_block` does. `layout` at a width the text fits lays the
kept shaping out uncut within the width (no shaping). At a width it does not
fit, the cut is read from the prepared unwrapped layout and the cached `…`
(the measuring of `…` shared with the cache), and the cut line is shaped
through `shape_cut_layout`'s cut path, so its `shown` and its face ranges
stop before the `…` as today; the `PreparedLayout` holds that layout.

In the shared cache (Phase 3): `ellipsis_cut` decides the fit from the
unwrapped layout when it is cached (a caller that measured first), as today.
When it is not, the block is shaped uncut under its own key first: every
line within the width means it fits and that is its layout, one shape where
today there are two. Otherwise the cut is read from that layout's advances
(the same glyphs as the unwrapped one, unwrapped, the same advances) and the
cut line shaped, two shapes as today. An empty text, or a width that is not
positive and finite, takes the paths it takes today.

## Where a recorded block's layout is looked up (Phase 2)

Every site takes its layout through one resolver: given the spec and content
the site would have looked up (unscaled before `DrawList::text` scales the
block, scaled after; joined spans), and the block's handle, it returns the
handle's layout when its key, content, font system and generation are
exactly those, else the shared cache's.

| Site | What it reads | Spec it looks up |
| --- | --- | --- |
| `text_prep.rs:246` (`prepare_texts`) | glyphs to place | the recorded block's |
| `text_prep.rs:338` (`refresh_texts`) | glyphs to place again | the recorded block's |
| `widgets/draw_list.rs:1278` (`underline_segments`) | lines for underlines | before scaling |
| `widgets/draw_list.rs:3005` | highlight rectangles | before scaling |
| `DrawList::caret_layout` / `visual_layout` (`scaled_spec`, `:3642`, before the block is recorded) | caret positions and visual glyphs | scaled, spans joined |
| `text.rs:2420` (`measure_block_ink`) | ink band | the block's |
| `text_select.rs:624`, `:647` | hit testing, bounds; `DrawnBlock` (`:141`) keeps the handle when it copies a block | the drawn block's |
| `markdown/layout.rs:517` | what markdown measures of its blocks | the block's |

`TextMeasurer::measure_block` (`text.rs:2391`) keeps its own policy (an
ellipsized block measured unwrapped) and does not read the handle, which
holds the drawn, cut layout; a caller wanting a prepared size asks the
`PreparedText`. Under a scale transform the recorded block's key no longer
matches the handle, so the renderer uses the cache, shaping at the scaled
size as today; the unscaled lookups before it still match. Caret and visual
data for a prepared layout are kept with it the first time asked for
(`text_carets.rs` keeps them per cache slot today).

The list is a grep on 2026-11-12 for `LayoutSpec::of_block`, `scaled_spec`,
`layout_and_fonts` and `shared.layout` outside tests; Phase 2 runs it again.

`frame_diff.rs` names every field of a `TextBlock` (`:854`), so the handle
is added there and left out of the comparison: two blocks equal in every
other field have the same layout from a handle or the cache. The tests
below check that through what is drawn, not through that comparison.

## The retained tree (Phase 4)

- `TextContent` (`crates/retained/src/ui.rs`, behind its `Box`, so
  `NodeData` does not grow) gains the node's `PreparedText`, made at its
  first measure and again when its text changes in what shaping depends on
  or the font system's generation moves. A colour change, a scale change
  (the device scale is in the renderer's projection, not in text sizes) and
  a deferred node's text change (until it is measured) do not touch it.
- `measure_text` calls `prepared.measure` at the offered width. When the
  node's size is settled, its drawn layout is made (`prepared.layout` at the
  content width) and kept as a `PreparedLayout` on `TextContent`, so
  `paint_node` and the frame drawn from nothing only read it. For a node of
  exact width the two are the same layout.
- The tree keeps a byte budget for its prepared shaping (Bart, 2026-11-12):
  past it, the shaping of the nodes laid out longest ago is dropped (their
  drawn layout stays); such a node prepares again if it is measured again.
  The bytes are counted in `TreeStats`. The default is 64 MB (Bart,
  2026-11-12, from Phase 0's numbers), settable on the tree.

## Tests and checks

- Counted (paragraphs shaped, layouts made, through `layout_stats()` across
  measure, layout and the renderer's preparing): a new label in a hugging
  node shapes once and lays out twice (offered and content width); in an
  exact node once and once; a changed label once more; a colour change, a
  move, an unchanged frame nothing; a width change no shape and the layouts
  it needs; a font generation change, or another font system, one shape.
- Exact: a prepared block's layout at a width equals the same block shaped
  fresh through the cache at that width, for the five alignments, both
  directions, the four wrap modes, a word wider than the width, fractional
  break widths, ellipsis fitting and cut (with hard breaks), vertical text,
  letter spacing positive and negative, faces, spans, CRLF and empty lines;
  and after transitions on one prepared text (width A → B → A, centre →
  start, wrap on → off, line height, ellipsis on → off), and with an old
  handle used after newer layouts.
- A handle on a block whose fields changed after it was attached (content,
  width, a scale transform), from another font system, or from an older
  generation, is not used: what is drawn is the cache's.
- The renderer: a prepared block's placed glyphs equal the cache's, through
  `prepare_texts`, the colour-only `refresh_texts` path, a replayed
  unchanged frame, and a real font generation change.
- The fitting-ellipsis count: a block only drawn shapes once fitting and
  twice cut; measured and drawn, twice either way, as today;
  `ellipsis_tests.rs`' warmed cut test stays at two.
- `Send` and `Sync` of `TextBlock`, `PreparedLayout`, `DrawList`.
- The retained oracle (`every_frame_is_the_tree_drawn_from_nothing`), the
  deferral twin, and the deferral counts (`a_scroll_lays_out_only_the_rows_
  near_its_view`: nothing prepared for a row not yet measured).
- Scale: 10,000 labels, repeated and unique, scrolled through whole and
  changed with old frames alive: bytes and allocations held, under the
  budget.
- Bench: `rows-one` shapes near 229 in 230 frames; Phase 0's benches.

## Phasing

As the checklist: 0 (counters, benches, bytes, the baseline test) before
anything moves; 1 and 2 together make the API usable; 3 is independent of
the rest; 4 uses 1 and 2; 5 closes.

## Decisions

- **Prepared text carried from measure to draw** (Bart, 2026-11-12), over a
  shaping cache beneath the layout cache, aligning at draw time, and a
  hugged-box alignment flag.
- **The handle travels in the record** (`TextBlock::prepared`) rather than
  seeding the shared cache (Bart, 2026-11-12): no eviction between the frame
  and the render.
- **The shaping lives as long as the node** (Bart, 2026-11-12), **within a
  byte budget** (Bart, 2026-11-12, after Antigravity's estimate of 1.6 to
  2 KB a short label, 16 to 20 MB for the grid's 10,000), prepared at the
  first measure, not at creation. The default budget is 64 MB (Bart,
  2026-11-12), chosen once Phase 0 measured a kept buffer at 7.5 KB a
  label rather than the estimate.
- **A cut ellipsis keeps its fresh shape** (Bart, 2026-11-12).
- **A fitting ellipsis in the cache: uncut first** when the unwrapped layout
  is not cached (Bart, 2026-11-12), over laying the unwrapped layout out at
  its own width and moving it per line.
- **Public API** for immediate-mode callers (Bart, 2026-11-12).
- **The handle is checked, not trusted**: `TextBlock`'s fields are public
  and `DrawList::text` changes them, so the handle carries what it was made
  for, font system identity included, and is used only when that is what a
  site looks up.

## Review

Reviewed by Codex and Antigravity on 2026-11-12, both read-only against the
code. Applied:

- The first draft's fitting-ellipsis fix moved the unwrapped layout by the
  alignment offset; both found that the unwrapped layout of right-to-left
  text sits at `f32::MAX / 4` with its x lost to precision, and Codex that
  ellipsized text keeps hard breaks, so lines need offsets of their own.
  Replaced by "uncut first" (Bart's choice); the right-to-left coordinates
  are a deferred bullet.
- A font generation alone does not say which font system (Codex): the stamp
  carries its identity too.
- `ShapedLayout::ink` is a `OnceCell`, so an `Arc` of it would make
  `TextBlock` and `DrawList` neither `Send` nor `Sync` (both): `OnceLock`,
  with assertions.
- Preparing at node creation would shape the 10,000 deferred rows (both):
  prepared at the first measure.
- Keeping one finished layout contradicts the counts for a hugging node
  (Codex): the last two are kept.
- The lookup table missed `measure_block` (its policy is now stated) and
  `TextSelection`'s `DrawnBlock`, which copies fields and would drop the
  handle (both); the underline and highlight lookups run before the
  block is scaled and the caret ones before it is recorded (Antigravity):
  the resolver takes the spec a site would have used.
- `frame_diff.rs` names every field, so it does change (both); tests check
  the prepared glyphs through what is drawn, `refresh_texts` included
  (Codex).
- The split between shaping and layout fields (line height and wrap are
  layout-only in cosmic-text), alignment set on every line, vertical text
  left unsized, `Buffer::new_empty`, spans joined before preparing, one
  extraction function shared with the cache, the direction marks' paragraph
  starts kept (both).
- The cut path under a prepared text reads the prepared unwrapped layout and
  keeps `shown` and the face ranges before the `…` (Codex).
- Counting moves to where shaping happens, and a relayout asserts it shaped
  nothing (Codex); `ellipsis_tests.rs`' two passes are a cut label's and
  stay (Codex).
- A device scale change does not prepare again: it is in the projection
  (Antigravity).
- Memory and allocation checks at 10,000 labels, the whole path benched,
  and a representative size for the deferred whole-text edit (Codex).

Rejected, each checked in the code:

- That unwrapped start-aligned text already drops `max_width` from the key,
  so `rows-one` could not shape twice (Antigravity): `LayoutSpec::of_block`
  keeps `Some(block.max_width)` (`shaping.rs:267`), and the bench's labels
  are `no_wrap` (`ui-compare-gameui-retained.rs:357`).
- That `Placement::text` builds a block field by field and would drop the
  handle (Antigravity): it clones the block (`draw_list.rs:3578`).
- That `draw_from_nothing` takes `&self` (Antigravity): it takes
  `&mut self` (`paint.rs:430`). The drawn layout is still made at layout
  time, so painting only reads it.
- That `DrawList::text` turns spans into face ranges (Antigravity): it joins
  their texts into the content (`draw_list.rs:3051`); spans are joined
  before preparing all the same.

## Phase 0's numbers (2026-11-12)

On leviticus under load (load average 16), so the times are rough; the
counts and bytes are not.

| `relayout` bench | shape at a width | lay out again |
| --- | --- | --- |
| label, 16 characters, unwrapped | 4.0 µs | 0.20 µs |
| sentence, 70 characters, unwrapped | 23 µs | 1.1 µs |
| paragraph, about 600 characters, wrapped | 150 µs | 6.1 µs |

Laying out again costs about a twentieth of shaping, so the premise holds.

A kept cosmic-text `Buffer` is much larger than the first draft assumed
(Antigravity's 1.6 to 2 KB a label). One 16-character label holds about
7.5 KB in 15 heap blocks: 1.3 KB set (the line, its text and attributes),
4.5 KB in 11 blocks shaped, 1.7 KB in 3 blocks laid out. cosmic-text keeps
a reset layout's allocation for reuse, so the laid-out part cannot be freed
without dropping the line. 10,000 kept buffers hold 74 MB, repeated or
unique labels alike (each node holds its own), against 11.3 MB of heap for
the cache's 10,000 finished layouts of the same labels.

`rows-one` today: 458 shapes in 230 frames, two for each of the 229 changes.

## Questions only building can answer

- What laying out again costs against shaping, per label, sentence and
  paragraph, and on the whole path (Phase 0).
- The bytes and allocations a kept `Buffer` holds, so the budget's default
  can be settled with Bart.
- Whether cosmic-text's relayout gives exactly a fresh shape's glyphs in
  every case tested; a difference stops the work and goes to Bart.

## References

- `docs/design/shared-text-layout.md` — one layout for drawing, carets and
  hit testing; the shaping counter; the ellipsis cut. Its premise that drawn
  text goes through one cache gains an exception: a prepared block's layout
  is its handle's.
- `docs/design/paragraph-text-layout.md` — paragraphs in a long field.
- `docs/design/retained-mode.md` — the retained tree.
- Codex and Antigravity's answers on alternatives and their reviews,
  2026-11-12 (in the session that wrote this doc).
