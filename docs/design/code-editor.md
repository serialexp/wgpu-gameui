# Code editor

**Status:** In progress
**Last updated:** 2026-10-16

## What it is

A source editor widget, `CodeEditor`, after the Forge design system's `CodeEditor` and
`CodeView` (`components/code/` in the Forge Design System project): a line-number gutter, the
caret line's band, Tree-sitter colours, and the editing keys the design lists. Its first host is
agent-ui's Files tab, which shows files read-only until saving lands there.

It lives in its own crate, `crates/code` (`wgpu-gameui-code`), like the media crate: the rope,
the parser runtime and the grammars stay out of programs that only want the core widgets.

## Implementation status

### Round one (done, uncommitted)

- [x] The document: text in a rope, selections, edits, undo and redo
- [x] The editing rules: Tab and Shift-Tab indent and outdent (selected lines too), Enter keeps
  the indent and steps in after an opener, brackets and quotes close themselves, Backspace
  removes a pair or a whole indent step
- [x] Highlighting with Tree-sitter, re-parsing only what an edit touched, and colouring only
  the lines in view
- [x] The widget: gutter, caret-line band, selection, caret, scrolling both ways, the mouse
  (click, drag, double and triple click, Shift-click), the keys, read-only mode, a status strip
- [x] agent-ui's Files tab shows text in it, read-only (frameless, `with_frame(false)`)

### Later

- Diagnostics (wavy underline, row wash, gutter dot, the message on hover) and completion, fed
  by a language server client
- Injections (code in Markdown fences, script in HTML)
- IME composition text (preedit) drawn at the caret; the editor asks for the IME and takes
  committed text, but doesn't show what is being composed
- Line moves (Alt-Up/Down)
- How far it scrolls sideways is the widest line drawn since the last edit: a long line out of
  view doesn't count until it is scrolled to (counting every line would cost a pass over the
  file per edit)
- WGSL: the only grammar on crates.io (`tree-sitter-wgsl` 0.0.6) is built on Tree-sitter 0.20,
  and the runtime `links`, so one program can hold only one version of it

## Decisions

**Its own crate.** Core has a small multi-line `TextInput` with Tree-sitter colours
(`text_area_syntax`). That is a form field: one `String`, the whole value highlighted again on
each change, no gutter or history. Opening any file in a file browser needs more than that, and
none of it belongs in core.

**Text in a rope (`ropey`), positions in bytes.** Tree-sitter speaks bytes and rows, so the
document does too; positions are always on character boundaries. The rope is built without
Unicode line breaks: only `\n` ends a line, as for Tree-sitter, so their row numbers agree. A
`\r` before it is kept in the text and left out of what is drawn, and Enter inserts the line
ending the file already uses.

**Incremental parsing, highlighting by the screenful.** Each edit is handed to the parser as an
`InputEdit`, so a re-parse reuses the old tree. The highlight query runs only over the lines in
view (`QueryCursor::set_byte_range`), so a long file costs no more per frame than a short one.
Where captures overlap, the innermost one wins, and of two on the same node, the query's later
pattern, as in Tree-sitter's own highlighter (TypeScript's patterns come after JavaScript's, so
they win).

**Colours from `SyntaxTheme`.** Core's palette, now compiled without the highlighting feature,
defaults to Forge's `--syn-*` inks, and maps a capture name to an ink by its first part that
names one (`keyword.function` is a keyword).

**Monospace, on a grid.** Every character takes a whole number of columns of the mono font's
advance: two for a wide one (most Chinese, Japanese and Korean, and emoji, by `unicode-width`),
one for any other, and a tab the space to its next stop. So the caret, the selection and a click
are arithmetic, never measured. Plain ASCII is drawn a run at a time; any other character alone
at its column, so one from a fallback font, at whatever width that font gives it, keeps to the
grid as a terminal's does. Only the columns in view (and a few either side) are drawn, however
long the line.

**Undo by groups.** Typing a run of characters, or deleting one with Backspace or Delete, is one
step to undo; moving the caret, a newline, or any other kind of edit starts a new step. Undo
puts the selection back as it was before the step.

**Lines run on.** A line never wraps (each text block is `WrapMode::None`); the view scrolls
sideways, and the gutter stays put over the code scrolled under it.

**The host's clipboard.** The editor takes the same `ClipboardGet`/`ClipboardSet` closures as
`TextInput` (`set_clipboard`); `UiState::clipboard()` hands out the ones the host installed.

**No time in the logic.** The caret doesn't blink, as core's `TextInput` doesn't, so nothing
needs a clock.
