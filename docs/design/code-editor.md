# Code editor

**Status:** In progress
**Last updated:** 2026-10-29

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

- [x] The document: text in a rope, selections, edits, undo and redo (af820c1)
- [x] The editing rules: Tab and Shift-Tab indent and outdent (selected lines too), Enter keeps
  the indent and steps in after an opener, brackets and quotes close themselves, Backspace
  removes a pair or a whole indent step (af820c1)
- [x] Highlighting with Tree-sitter, re-parsing only what an edit touched, and colouring only
  the lines in view (af820c1)
- [x] The widget: gutter, caret-line band, selection, caret, scrolling both ways, the mouse
  (click, drag, double and triple click, Shift-click), the keys, read-only mode, a status strip
  (af820c1)
- [x] Every character on the monospace grid; only the columns in view drawn, however long the
  line (ede1408)
- [x] Word deletes (Option/Ctrl-Backspace and Delete), Cmd-Backspace and Cmd-Delete, Page Up
  and Down, and a sideways scroll width that shrinks after an edit (4e9c726)
- [x] agent-ui's Files tab shows text in it, read-only (frameless, `with_frame(false)`), and
  since 2026-10-29 edits and saves it (agent-ui 0ddc072)
- [x] Diagnostics in: `Diagnostic` (byte range, severity, message, code) and
  `CodeEditor::set_diagnostics`; ranges move with the edits made since (uncommitted)
- [x] Diagnostics drawn: the wavy underline in the severity's ink, the row wash, the gutter
  dot and number ink (uncommitted)
- [x] The message on hover (over the underline or the gutter dot), drawn by
  `CodeEditor::draw_layers` (uncommitted; see the update under Diagnostics)
- [x] Problem counts in the status strip; a click jumps to the next one (uncommitted)
- [x] Completion: the list as a word is typed or on Ctrl-Space, offering the host's items,
  the language's keywords and the document's other words; Up/Down, Enter/Tab, Escape, a
  click on a row; a function accepted gets `()` with the caret between (uncommitted)
- [x] agent-ui's Files tab calls `draw_layers`, so the list and messages show there
  (uncommitted)

### Outstanding

Round three, language servers (agent-ui; see [Language servers](#language-servers)):

- [ ] Design the client from the answered questions below: servers beside the agent-ui
  server, built-in list plus TOML, stopped when unused, fed every change, completion on
  demand
- [ ] An LSP client that starts a project's servers, syncs the open file, and passes on
  `publishDiagnostics`
- [ ] Completion items from the server through `textDocument/completion`

Later:

- [ ] Injections (code in Markdown fences, script in HTML) (deferred: no host needs it yet)
- [ ] IME composition text (preedit) drawn at the caret; the editor asks for the IME and takes
  committed text, but doesn't show what is being composed (deferred: after round two)
- [ ] Line moves (Alt-Up/Down) (deferred: after round two)
- [ ] WGSL (deferred: the only grammar on crates.io, `tree-sitter-wgsl` 0.0.6, is built on
  Tree-sitter 0.20, and the runtime `links`, so one program can hold only one version of it)

## What it is

A source editor widget, `CodeEditor`, after the Forge design system's `CodeEditor` and
`CodeView` (`components/code/` in the Forge Design System project): a line-number gutter, the
caret line's band, Tree-sitter colours, and the editing keys the design lists. Its first host is
agent-ui's Files tab, which shows files read-only until ✎ makes one editable.

It lives in its own crate, `crates/code` (`wgpu-gameui-code`), like the media crate: the rope,
the parser runtime and the grammars stay out of programs that only want the core widgets.

> **2026-10-29 update:** the round-one list that stood here moved to the checklist above.
> How far it scrolls sideways is still the widest line drawn since the last edit: a long line
> out of view doesn't count until it is scrolled to (counting every line would cost a pass over
> the file per edit). That is a choice, not a gap.

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

## Round two: diagnostics and completion

From Forge's `CodeEditor` (`components/code/CodeEditor.{jsx,d.ts,prompt.md}` and `MARKS` in
`CodeView.jsx`). The widget only draws what the host hands it; where problems come from (a
language server, a compiler's output) is the host's business, so the widget stays usable
without one.

### Diagnostics

```rust
pub struct Diagnostic {
    pub range: Range<usize>,      // bytes in the document
    pub severity: Severity,       // Error, Warning, Info
    pub message: String,
    pub code: Option<String>,     // "E0425", shown before the message
}
editor.set_diagnostics(diagnostics);
```

- **Bytes, not lines and columns.** The document speaks bytes; turning an LSP position
  (line and UTF-16 column, or UTF-8 when the server agrees to it) into bytes is the host's
  job, because only it knows which encoding was agreed. An empty range, or one that ends on
  its start, is widened to the word at its start, or one character where there is no word
  (Forge: "defaults to the end of the word at col"). Ranges are clamped to the text and to
  character boundaries.
- **They follow the edits.** Between the host's updates, each edit moves the ranges after it
  and stretches or shrinks one it falls inside, as the parser's tree is moved
  (`Document::take_edits`). An edit that deletes a range entirely leaves it empty at the
  edit's start; it is drawn as one character. So a squiggle stays on its word while the line
  above is typed into, until the server's next word replaces the list.
- **Drawn, per line in view:** a wavy underline in the severity's ink under its range (a
  polyline of the columns in view only, so a long range costs no more than the view); for
  the line's worst problem, the row wash across the code and the gutter, the number in the
  severity's ink, and a 5 px dot with a soft glow at the gutter's left. Inks (Forge `MARKS`):
  error `--danger-text` on a `--log-error-row` wash, warning `--warn-meta` on `--log-warn-row`,
  info `--syn-function` with no wash. The caret line's band gives way to a problem's wash.
- **The message on hover**, over an underlined range or the gutter dot: a tooltip surface
  with the severity's label in its ink and caps ("ERROR E0425"), then the message, below the
  line (above it when there is no room). It is drawn into a popup layer the host pushes, as
  `DropdownState` does: `CodeEditor::push_layer(&mut LayerStack)` at the top of the frame and
  `draw_layer` after the base. The geometry is last frame's.

  > **2026-10-29 update:** shipped as one call, `CodeEditor::draw_layers(&mut LayerStack,
  > &StyleResolver, viewport)` after the base, drawing on tooltip layers rather than a popup
  > layer. A popup layer takes all input, keys included, while the pointer is over it, so
  > resting the pointer on the list would have stopped typing. A tooltip layer never takes
  > input; the editor handles clicks and hover on the list's rows itself, against last
  > frame's geometry, and keeps the list inside its own body so no other widget is under it.
  > The label comes first on the message's line ("ERROR E0425 message"), wrapped at 380 px.
- **The status strip** counts errors and warnings at its left (the ⊗ and ⚠ glyphs and the
  count, in the severity's ink when non-zero, muted when zero). A click on either moves the
  caret to the next of its kind after the caret, wrapping, and brings it into view.

### Completion

- **Opens** when a word character is typed (the word before the caret is the prefix) or on
  Ctrl-Space (an empty prefix allowed); **closes** on Escape, a click, a caret move other than
  typing or Backspace, focus leaving, or no item left.
- **Offers**, in order and without repeats: the host's items (`set_completions`), the
  language's keywords (`Language::keywords`), and the identifiers in the document (scanned
  once per text version, not per frame). Matched by a case-insensitive prefix, at most 8, the
  exact prefix itself left out.
- **Keys** while open: Up and Down move (wrapping), Enter or Tab accepts, Escape closes; the
  editor takes them so they don't move the caret. Accepting replaces the prefix with the
  label; a `Function` gets `()` with the caret between, unless `(` follows already.
- **Drawn** in the same popup layer as the hover message, under the caret line (above it when
  there is no room): a dropdown surface, 20 px rows, a kind tag (`fn`, `kw`, `ty`, `fd`, `k`,
  `v`) in the kind's syntax ink, the label with the typed prefix bolder, the detail
  right-aligned and dimmer; the chosen row on the accent with `--ink-on-accent`.

> **2026-10-29 update:** drawn on a tooltip layer, as the message is (see the update under
> Diagnostics); the pointer resting on a row chooses it, a click accepts it. Ctrl-Space is the
> Control key on macOS too (Cmd-Space is Spotlight): `InputState::control_down` is new for it,
> since `ctrl_pressed` means Cmd there. The word being typed is left out of the document's
> words, so a half-typed word isn't offered back to itself. `CodeEditorOutput::completion_asked`
> tells the host Ctrl-Space was pressed, for round three's request to the server. The list
> stays inside the editor: in a short one it shows the rows that fit and scrolls to keep the
> chosen one in view; in a narrow one it is cut to the editor's width and leaves out a
> detail that would run into its label; where not one row fits it closes. While its word is
> scrolled out of view it stays open, unshown. A jump from the status strip, or turning the
> editor read-only, closes it.

```rust
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,     // Function, Keyword, Type, Field, Constant, Variable
    pub detail: Option<String>,   // a signature or type, right-aligned
}
editor.set_completions(items);
```

## Language servers

Round three is agent-ui's, and not designed yet. What is known:

- The project's files are on the machine the agent-ui server runs on (leviticus), and a
  language server needs the project on disk next to it, so servers run beside the agent-ui
  server, not the desktop.
- The desktop already sends the open file's text only on save. A server wants
  `didOpen`/`didChange` as the text changes, so the desktop would send edits (or the whole
  text, debounced) while a file is being edited.
- Diagnostics arrive whenever the server likes (`textDocument/publishDiagnostics`), so they
  need a way back to the desktop: the event stream the desktop already listens on, or a
  stream per open file.

### Open questions for review

1. **Which servers, and who says so?** A built-in list by language (rust-analyzer,
   typescript-language-server, gopls, pyright, …), started when found on `PATH`; or a section
   in the providers TOML naming each command; or both, the TOML overriding.

   > **2026-10-29, Bart:** both: the built-in list, from `PATH`, and the TOML overriding or
   > adding.
2. **When does a server start and stop?** On the first file of its language opened in a
   project, kept while any is open plus a grace; or kept for the project's lifetime once
   started. rust-analyzer on a large workspace takes minutes and gigabytes to warm up, which
   favours keeping it.

   > **2026-10-29, Bart:** stop when unused: a while after the last file of its language
   > closes.
3. **What the server sees of unsaved text.** Every change (debounced) so problems update as
   you type; or only what is saved, simpler, but squiggles go stale while editing.

   > **2026-10-29, Bart:** as you type, debounced.
4. **Completion over the network.** A round trip to leviticus per keystroke is felt on a slow
   link. Ask on Ctrl-Space and on a trigger character only, and filter locally as the word
   grows; or ask on every word start.

   > **2026-10-29, Bart:** on demand and on trigger characters, narrowed locally.
