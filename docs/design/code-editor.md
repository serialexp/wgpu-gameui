# Code editor

**Status:** In progress
**Last updated:** 2026-10-30

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
  `CodeEditor::set_diagnostics`; ranges move with the edits made since (41413a3)
- [x] Diagnostics drawn: the wavy underline in the severity's ink, the row wash, the gutter
  dot and number ink (41413a3)
- [x] The message on hover (over the underline or the gutter dot), drawn by
  `CodeEditor::draw_layers` (41413a3; see the update under Diagnostics)
- [x] Problem counts in the status strip; a click jumps to the next one (41413a3)
- [x] Completion: the list as a word is typed or on Ctrl-Space, offering the host's items,
  the language's keywords and the document's other words; Up/Down, Enter/Tab, Escape, a
  click on a row; a function accepted gets `()` with the caret between (41413a3)
- [x] agent-ui's Files tab calls `draw_layers`, so the list and messages show there
  (agent-ui e6af3bb)
- [x] R3.1 gameui: numbered completion asks with their trigger, the host's items offered
  for one ask, items that carry their own edit (a range and text) and extra edits, filtered
  by `filter_text` and kept in the server's order, a list longer than the rows shown, a
  re-ask while an incomplete list narrows, trigger characters, edits mapped through the
  typing since the ask, and the chosen item resolved before it is accepted (3fa7955). The
  revision the desktop sends is `document().version()`; a waiting accept counts its second
  down with each frame's `frame_dt` and asks for a frame by its end; a list that comes while
  an accept waits is dropped, so the item accepted stays the one taken

### Outstanding

Round three, language servers (agent-ui; see [Round three design](#round-three-design)):

- [x] Design the client from the answered questions below (this doc, 2026-10-30)
- [ ] R3.2 `crates/lsp`: message framing, requests and replies, a session actor that owns
  the process and serializes everything sent to it, documents with the server's versions
  and the text of the latest, positions converted to and from bytes; tested against an
  in-process fake server
- [ ] R3.3 server: the built-in server list, finding a file's project root, one session per
  (command, root) started on first use and stopped a while after its last document closes,
  process groups and bounded waits; the routes; saves reported as `didSave`; tested against
  a fake server process
- [ ] R3.4 server: file watching for servers that register for it
  (`workspace/didChangeWatchedFiles`)
- [ ] R3.5 desktop: following the open file's events, sending its text as it is edited
  (debounced), asking for completions and resolving the one accepted, handing all of it to
  the editor
- [ ] R3.6 tried with `typescript-language-server` and `gopls` (an ignored test, and the app)
- [ ] R3.7 overrides and additions to the built-in list in the server's settings (deferred:
  after R3.6, and see question 1's update)
- [ ] Incremental text sync (`didChange` with ranges) (deferred: whole text first)
- [ ] Pull diagnostics (`textDocument/diagnostic`) for servers that only offer those
  (deferred: the servers tried push theirs)

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

Round three is agent-ui's. What was known before it was designed:

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

   > **2026-10-30 update:** the server no longer reads a providers TOML; providers and
   > `ServerSettings` live in its database and are edited in Settings
   > (`crates/server/src/settings.rs`). So the overrides are proposed as a field of
   > `ServerSettings` instead (R3.7), deferred until the built-in list has been tried. To
   > be confirmed by Bart.
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
## Round three design

Written 2026-10-30 from the answers above and the code as it stands, then revised twice the
same day after Codex's reviews: versions, completion edits, saves and watched files,
ordering, incomplete lists, auto-imports, reconnects, process cleanup and size limits; then
request snapshots, how completion edits map to the text now, resolving before an accept,
stale resolve keys, every opening sending its text, save ordering, watches from the
registrations, one cleanup for every end, and positions.

### Goals

- Problems from a real language server appear in the Files tab's editor as they are typed,
  and completion offers the server's items on Ctrl-Space and its trigger characters,
  including the edits they carry (an auto-import adds its import).
- Servers run beside the agent-ui server, where the project is, started when first needed
  and stopped when not, and never left running.
- Nothing about one language server is special-cased: the built-in list is data.

### Non-goals

- Go to definition, hover documentation, rename, formatting, code actions, and an item's
  `command`. The client is built so they can follow, but round three is problems and
  completion; capabilities it can't honour are not advertised.
- Incremental text sync and pull diagnostics (deferred; see the checklist).
- Two clients editing one file at once. Both can follow it; the text last sent is the
  document, and each sees problems only when they are for its own text (below).
- Files the Files tab shows cut short (over `MAX_VIEWED_BYTES`) or not as text.

### Where each part lives

```
desktop (exodus or the Mac)               agent-ui server (leviticus)        language server
Files tab ── CodeEditor                    routes ── LanguageServers hub      (process group)
   │  text as edited (debounced) ──PUT──▶    │        one session actor ──── stdio, JSON-RPC
   │  completion ask / resolve ───POST──▶    │        per (command, root)
   ◀── events (ready, problems) ──GET────    │        files.rs save ─▶ didSave
```

- **`crates/lsp` (new).** The protocol and nothing about HTTP: framing
  (`Content-Length` headers), JSON-RPC requests, replies and server-to-client requests, and
  the session actor below. Message shapes from the `lsp-types` crate. Generic over
  `AsyncRead`/`AsyncWrite`, so its tests drive it through an in-process fake server over
  `tokio::io::duplex`, with no process.
- **`crates/server/src/language_servers.rs` (new).** The hub: the built-in list, finding a
  root, the sessions, the routes. `files.rs`'s save tells it a file was saved.
- **`crates/protocol/src/language.rs` (new).** The request and event shapes both sides share.
- **Desktop.** `working_client.rs` gains requests and updates as the file watch has
  (`Request::WatchFile`, `Update::FileVersion`); `files_tab.rs` drives them.
- **gameui (`crates/code`).** The API changes under R3.1.

### Which server, and its project root

A built-in table, keyed by the file's extension:

| Files | Command | Root marker (nearest folder up holding one) |
|---|---|---|
| `.ts .tsx .js .jsx .mjs .cjs` | `typescript-language-server --stdio` | `tsconfig.json`, `jsconfig.json`, `package.json` |
| `.rs` | `rust-analyzer` | the outermost `Cargo.toml` with `[workspace]`, else the nearest `Cargo.toml` |
| `.go` | `gopls` | `go.work`, else `go.mod` |
| `.py` | `pyright-langserver --stdio`, else `basedpyright-langserver --stdio` | `pyproject.toml`, `setup.py`, `requirements.txt` |

- The command is looked up on the server process's `PATH`. Under systemd or launchd that
  is short (no nvm, no `~/go/bin`), so the hub also looks in `~/.cargo/bin`, `~/go/bin`,
  `~/.local/bin` and the newest `~/.nvm/versions/node/*/bin`, and a missing command is a
  `failed` event naming it, which the Files tab shows.
- The root is found walking up from the file, never above the home folder (as the file
  routes keep to it, `files.rs::inside`); with no marker, the file's own folder.
- One session per (command, root): two TypeScript files under one `package.json` share a
  server.

### The session actor

Each session is one task that owns the process's stdin and stdout and all of the session's
state: its documents, their texts and versions, the requests awaiting replies. Everything
else talks to it through a channel, with a reply channel where an answer is wanted. So the
order things reach the server is the order the actor handled them: `initialize` is answered
and `initialized` sent before any `didOpen`; a document's `didOpen` comes before its
`didChange`s, its `didSave`s after the text they saved, and its `didClose` after them all.

- **States:** Starting (spawned, `initialize` sent), Ready, Stopping, Gone. Opens and texts
  that arrive while Starting are queued in order and sent once Ready. Once Stopping, the hub
  hands a new opening to a new session; the old one only finishes stopping. The hub's map
  from (command, root) to session and a session's state change under one lock, so an opening
  can't join a session that has just committed to stop.
- **Bounded waits:** `initialize` 60 s, any request 30 s, `shutdown` 5 s. A request whose
  time runs out is answered as failed; a server that misses `initialize` or `shutdown` is
  stopped.
- **Process:** spawned in its own process group (`process_group(0)`), the root as working
  directory, stdin and stdout piped, stderr to a short ring kept for the `failed` message.
  The child's `PATH` starts with the folder the command was found in, so a launcher that
  starts `#!/usr/bin/env node` finds the `node` beside it.
- **One way to end.** Every end (stopping when unused, the agent-ui server stopping,
  `initialize` failing, the process exiting on its own, the actor's task being cancelled or
  panicking) runs one cleanup, held by a guard that owns the process group's id: pending
  requests are answered as failed, the documents' streams get a `failed` event, and the
  whole group is killed and reaped (so `tsserver` under `typescript-language-server`, or
  rust-analyzer's proc-macro server, goes too). A deliberate stop first sends `shutdown` and
  `exit` and waits briefly.
- **Server-to-client requests** are answered: `client/registerCapability` and
  `client/unregisterCapability` (file watchers, R3.4), `workspace/configuration` (empty
  settings), `window/workDoneProgress/create`; any other is answered with "method not
  found", never left waiting.
- **A failing server:** a session that ends on its own within a minute of starting is
  remembered as failing for its (command, root) for five minutes, so a broken server isn't
  restarted in a loop; openings meanwhile get a `failed` event saying so.
- **Stopping when unused:** when its last document closes, a grace starts (five minutes,
  `LANGUAGE_SERVER_GRACE`); an opening during it cancels it. The grace takes its clock as an
  argument, so its tests count, never wait.

### Positions

- `initialize` offers `general.positionEncodings: ["utf-8", "utf-16"]`; the server's
  `positionEncoding` answer is used, UTF-16 when it gives none (the protocol's default).
- A position is a line and a character offset in that encoding, with lines as the protocol
  counts them: `\n`, `\r\n` and a lone `\r` each end one. That is not how the editor counts
  lines (only `\n`, keeping a `\r` in the line), so the actor keeps its own line index of
  each text for the protocol; only byte offsets cross between the two. A character offset
  past a line's end is clamped to it (never between a `\r` and its `\n`); one inside a
  UTF-16 surrogate pair or a UTF-8 sequence is moved to the character's start.
- All conversion happens in the actor, against the text the position belongs to. The
  desktop and the editor see only bytes.

### Documents, openings and versions

- **An opening** is one events stream following one path: the hub gives it an id, sent in
  its first event. A document is open on the server while at least one opening follows it;
  each opening holds a guard, and the last guard dropped sends `didClose`. A reconnect is a
  new opening with a new id, so a late cleanup of the old one can't close the new one.
- **Every opening sends its text first.** The desktop sends the editor's whole text as the
  opening's first revision as soon as it has the opening's id, whether or not it holds
  unsaved edits; the server's document starts from the first text sent (`didOpen`), never
  from the disk. So what the server analyses is always what an editor shows, even when the
  file changed on disk between the desktop reading it and the stream opening.
- **The document's version is the hub's**, counted up by one for each text it sends the
  server, whoever sent it. The desktop numbers its own texts (`revision`) per opening; the
  hub records which (opening, revision) each version came from. A text identical to the
  document's current one sends nothing to the server and just records that this (opening,
  revision) is the current text too.
- **After the first text,** the whole text after each change, at most once per 300 ms
  while typing and at once when the typing stops. Over `MAX_SAVED_BYTES` is refused. A text
  with an older revision than the opening's last is ignored.
- **The actor keeps the latest text** of each document, and, with each request still
  awaiting a reply, the text that request was made against (the texts are `Arc<str>`, so
  that is a reference, not a copy). Problems for an older version are dropped before
  conversion; only problems for the latest version are converted and sent.
- **Saves.** Saving in the Files tab first sends any text still waiting on the debounce,
  then saves through `files.rs`, which, once written, tells the hub the path and the saved
  text. The actor handles that after the text before it, so a `didSave` never overtakes its
  `didChange`; it sends `didSave` to a server that asked for save notifications, with the
  saved text when it asked for that. A save from another client is reported the same way.
  rust-analyzer runs `cargo check` on save, so its build errors come this way.

### Problems

- Sent to every opening of the document as `problems {for, exact, problems}`, ranges in
  bytes of the latest text, `for` naming every (opening, revision) that text is.
- **Versioned publishes** (rust-analyzer, gopls) are exact: `exact: true`.
- **Unversioned publishes** (`typescript-language-server` sends no version) can't be tied
  to a text, so they are taken as for the latest one and marked `exact: false`: best effort,
  perhaps a moment stale, replaced by the next.
- The desktop applies a publish when `for` names its opening's latest revision. When the
  desktop has moved on, the editor's copies keep following its edits until a publish for its
  newer text arrives.
- The actor keeps the last problems sent, with the version they were for, and forgets them
  when a new text is sent. A new opening whose first text matches the document's current
  text, while the kept problems are for that text, is sent them at once (with `for` now
  naming it too), so a second follower or a reconnect sees them without waiting for the
  server's next publish. Otherwise it gets them at the server's next publish.

### Completion

- **An ask** from the editor has a number (counted per editor), where it was made (the
  caret's byte offset), the start of the word there, and how it was triggered (Ctrl-Space,
  a trigger character and which, or a re-ask of an incomplete list). The desktop sends
  `{opening, revision, offset, trigger}` once its latest text has been answered (its
  requests for one document go out in order, one at a time). The actor refuses an ask for a
  text that is not the latest, and converts the reply against the text the ask was made for,
  whatever has been sent since.
- **The reply** is a list with `incomplete` and items, each with: label, kind, detail, its
  edit (a byte range in the asked text and the new text: from `textEdit`, else
  `insertText` or the label over the word), extra edits (`additionalTextEdits`) in bytes of
  the same text, `filter_text`, `sort_text`, and a resolve key when it needs resolving (the
  server's `data` stays in the actor). Every range is in the asked text: the reply carries
  its revision.
- **The editor** uses a reply only for its latest ask while the list (or the caret's word)
  is still where it was asked, and drops replies when the file it was for is closed. It
  filters by `filter_text` (else the label) with the same case-insensitive prefix match as
  now, in `sort_text` order, and shows as many as match (scrolling, as a short editor's list
  already does).
- **Mapping edits to the text now.** The list closes on every edit but typing or deleting at
  the caret within the word, so while it is open, the only change since the ask is the word
  between its start and the caret having grown or shrunk by some bytes. That makes the
  mapping exact: a range ending before the word's start is unchanged; one starting at or
  after the asked caret moves by the change; the item's own edit has its end moved by it; any
  other range touching the word (an extra edit overlapping the word) makes the item's edits
  unusable, and accepting it falls back to replacing the word with its label.
- **Accepting** applies the item's edit and its extra edits as one undo step, the extra
  edits in the reply's coordinates mapped as above; no `()` is added to a server's item.
- **An incomplete list** is asked again (a re-ask) when the word grows or shrinks, the reply
  replacing the items, debounced as the text is.
- **Resolving before committing.** A server resolves against the text it holds when the
  resolve arrives, not the text the list was asked in, so a resolve names the revision it is
  for: the desktop sends the text first, then `{key, revision}`, and the actor refuses one
  that is not for its latest text and converts the answer against that text. The resolved
  extra edits are then in that revision's text, and the editor uses them only while its text
  is still that revision.
  - For an item with a resolve key, the desktop resolves the chosen item when it is chosen,
    and again for the new text when the word changes while it stays chosen (debounced as the
    text is), the client advertising `resolveSupport` for `additionalTextEdits` and `detail`
    only. So by the time Enter is pressed a resolve for the text as it is has usually come
    back, and accepting is immediate.
  - When it has not, the editor holds the accept until a resolve for its current revision
    comes back or 1 s passes, then accepts with what it has (without extra edits if none
    came). While it holds, keys are kept, not acted on, and played in order after the
    accept, so typing on after Enter loses nothing and can't change the text the resolve is
    for.
  - So an accept is always one step, which Undo takes back whole; nothing arrives to be
    applied after it.
- **A resolve key is good for the session's latest completion request only.**
  `typescript-language-server` keeps the data its items resolve from per request and starts
  over at the next, whichever document asked, so the actor counts completion requests per
  session and answers a resolve for an older one as stale; accepting such an item uses its
  edit without the extra edits.
- **Kinds:** Function and Method to Function, Class/Struct/Interface/Enum to Type, Field and
  Property to Field, Constant and EnumMember to Constant, Keyword to Keyword, the rest to
  Variable.
- **Trigger characters** come from the server's `completionProvider.triggerCharacters` in
  the opening's first event.

### Watched files (R3.4)

gopls and rust-analyzer register `workspace/didChangeWatchedFiles` watchers and rely on
being told when files change outside the editor (an agent editing a dependency, a
`git checkout`).

- The actor keeps the registrations (`client/registerCapability`, removed by
  `client/unregisterCapability`): each watcher's glob, its base and the kinds of change it
  wants. The base is a relative pattern's `baseUri`; for a plain glob, the folder its
  leading literal part names when it is absolute (`/home/me/dep/**/*.go` is watched from
  `/home/me/dep`), else the workspace root.
- The hub watches each distinct base recursively (`notify`, as `files.rs` watches a
  folder), inside the home folder only, so a Go `replace` pointing at a sibling module is
  watched when gopls registers it. A folder named `.git`, `node_modules` or `target` below a
  base is left unwatched only when no registered glob can match anything under it (tested by
  matching the folder's path against each glob as a prefix), which keeps a build's churn out
  without hiding what a server asked for.
- Each change is matched against the globs and kinds and sent in batches. Watches go with
  the registration or the session.
- A base that can't be watched (the watch limit reached) sends a `degraded {message}` event
  to the session's openings, which the Files tab shows, and the server only learns of
  changes to its open documents.

### Routes

All under the existing login and peer gate.

- `GET /api/v1/language/events?path=`: follow a document. The first event is
  `ready {opening, server, trigger_characters}` (or `failed`); then `problems`, `degraded`,
  and `failed {message}` when the server ends. `404` when no server serves that kind of
  file.
- `PUT /api/v1/language/documents` `{path, opening, revision, text}`: the text. `204`;
  `409` when the opening is no longer following (the desktop reopens, which sends the text
  again); `413` over `MAX_SAVED_BYTES`.
- `POST /api/v1/language/completion` `{path, opening, revision, offset, trigger}`: the list,
  with the revision its ranges are in.
- `POST /api/v1/language/resolve` `{path, opening, revision, key}`: the chosen item's extra
  edits (in that revision's text) and detail; stale when the key is from an older completion
  request or the revision is not the latest.

### R3.1 gameui changes

- `CodeEditorOutput::completion_asked` becomes `Option<CompletionAsk { id, at, from,
  trigger }>`, set on Ctrl-Space, a trigger character, and a change to the word while an
  incomplete host list is open.
- `CodeEditor::set_trigger_characters(&[char])`: typing one opens the list (empty prefix
  after it) and asks.
- `CodeEditor::offer(id, CompletionList { incomplete, items })` replaces `set_completions`:
  used only for the latest ask while its word is still there, dropped when the list closes.
- `Completion` gains `edit: Option<(Range<usize>, String)>`, `extra_edits`,
  `filter_text`, `sort_text`, and `resolve: Option<u64>` (the host's key). The ranges are in
  the text the ask was made in; the editor maps them as above.
- `CodeEditorOutput::resolve` asks for the chosen item to be resolved (when it is chosen,
  and when the word changes under it), and `CodeEditor::resolved(id, key, revision,
  extra_edits, detail)` hands back what resolving gave, used only while the editor's text
  is that revision (the host tells the editor its revision with each text it sends). An
  accept of an item not resolved for the current text holds as above, keys kept and played
  after it, the wait's end a time passed in (no clock inside the editor) with a repaint asked
  for at it.
- The list is no longer capped at eight items: it shows `MAX_ROWS` and scrolls.

### Tests

- `crates/lsp`: framing split across reads and several messages in one read; a reply
  matched to its request out of order; server requests answered (registered, unregistered,
  or "method not found"); a request that times out answered as failed; UTF-8 and UTF-16
  positions with multi-byte and astral characters, a position inside a surrogate pair, past
  a line's end, and `\r\n` and lone `\r` line ends; opens queued while Starting reach the server in order after
  `initialized`; a completion converted against the asked text after a newer one was sent;
  an ask for a stale text refused; problems for an old version dropped; kept problems not
  replayed after a newer text; a resolve after a newer completion request, or for an older
  revision, answered stale; a save after a debounced text reaching the server
  after it.
- Server, against a **fake language server** (the test binary run again with an
  environment variable, as `crates/claude/src/mcp_tests.rs` does) that answers `initialize`,
  publishes a problem per line containing `bad` (versioned or not, by an option), completes
  from a fixed list with an incomplete flag, a `textEdit`, an extra edit and an item needing
  resolve, registers a file watcher, spawns a child of its own, and can be told to hang on
  `shutdown` or exit: two files under one root share a session; two roots get two; the grace
  stops it after its last opening (clock passed in) and an opening during the grace keeps
  it; an opening while Stopping gets a new session; the child process is gone after every
  kind of end, the hung shutdown and the unexpected exit included; a missing command is a
  `failed` event; a failing server is not restarted in a loop; a file outside home, or a text
  over the size limit, is refused; a save reaches it as `didSave` after its text; a reconnect
  with the same text gets the latest problems at once and an old opening's cleanup doesn't
  close the new one; two openings' revisions don't collide; a watched file changing reaches
  it, and stops after it unregisters.
- Desktop: the debounce (clock passed in); a publish for another opening's text or an older
  revision not applied; an ask's reply applied only for the latest ask at its word, and not
  after the file closes; every opening sending the text first; Save flushing the text first;
  the chosen item resolved.
- gameui: each R3.1 change: a 300-item list scrolling; edits mapped through typing since the
  ask; an extra edit overlapping the word falling back to the label; a resolve for an older
  revision not used; an accept held for a resolve keeping the keys typed meanwhile and
  playing them after it, and ending at the time passed in; Undo after an accept with extra
  edits taking it all back.
- By hand: `typescript-language-server` (problems, completion after `.`, an auto-import) and
  `gopls` (problems after a dependency changes on disk), an `#[ignore]`d test that runs only
  when the command is installed, and the Files tab.

### Security

A language server runs code from the project: rust-analyzer runs build scripts and
procedural macros, the TypeScript server loads plugins named in `tsconfig.json`. The agents
this server runs already execute anything in the project (`bypassPermissions`, `yolo`), so
this adds no new trust, but opening a file in an untrusted folder can now run code from it
without an agent turn. Only files inside the home folder are served, as for the file
routes.

### Open questions for review

5. **When a file gets a server.** On opening it in the Files tab, so a read-only view shows
   its problems too; or only once ✎ makes it editable, which never starts rust-analyzer
   just to look at a file. Proposed: on opening, since the grace stops it soon after.
6. **Overrides (R3.7).** A `language_servers` list in `ServerSettings`, each entry an
   extension list, a command with arguments, and root markers, ahead of the built-in rows;
   edited in Settings → Server. Proposed, deferred until the built-in list has been tried.
