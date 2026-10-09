# File dialog

**Status:** In progress
**Last updated:** 2026-10-09

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

(nothing yet)

### Outstanding

Everything below is built in the working tree, with tests and gallery renders, but not yet
committed (2026-10-09). Each box is ticked once it is on main.

- [ ] Pure model (`file_dialog/model.rs`): history, up, filter and sort, type counts and chips,
  hidden, name and new-folder validation, save name with extension, primary label and state,
  type-ahead, size and date formatting, with unit tests
- [ ] `FilterChip`: Forge's latching pill key, as its own widget (replaces `badge::chip`, and
  `UiContext::chip_button` becomes `filter_chip`)
- [ ] Surface: header (title, back / forward / up, breadcrumb well, ☆ pin, filter, list / grid,
  new folder), three columns, footer, over a modal backdrop
- [ ] Places column: Recent, fixed places, rule, ★ favourites with × on hover, the "☆ in the
  toolbar pins a folder here" note
- [ ] Listing, list view: sortable header, rows with glyph or thumb, modified / size / kind
  columns, zebra, selection (accent when focused), dimmed rows, scrolling with the selection kept
  in view
- [ ] Listing, grid view: 86px cells, 52px icons, two-line names
- [ ] States: loading skeleton, error, empty, no matches, "N other files", "Show all types" and
  "Show hidden" keys
- [ ] Type strip: folder-mode caption, 0 / 1 / 2-3 / >3 types (+N expands inline), Hidden checkbox
- [ ] Typed path: click the well's empty space, ⌘L or `/`; Enter goes, a bad path keeps the field
  open with the error inline
- [ ] New folder row: inline field, Enter creates, Esc cancels, duplicates and bad characters
  flagged
- [ ] Save mode: name well with the base selected, extension suffix, Replace in danger tone
- [ ] Preview column: icon, name, Kind / Size / Items / Modified / Where / caller rows, folder
  summary
- [ ] Keys: ↑↓ (and ←→ in grid), Enter, ⌫ and ⌘↑ up, ⌥← / ⌥→ history, type-ahead, Esc backs out a
  layer at a time, Tab ring
- [ ] Gallery entry, so it can be screenshotted: three cells (folder / list, open / grid with
  type chips, save / replace) in `tests/widget_gallery.rs`, which is where the gallery lives
- [ ] Tests through the frame loop: navigation, confirm per mode, new folder, pin / unpin, typed
  path errors, Esc layering

## Why this exists

Forge specifies an in-app file dialog (`components/dialogs/FileDialog.jsx` in the Forge Design
System project) so that opening, saving and choosing folders looks and behaves the same on every
platform. gameui has no such widget. agent-ui needs one first: its server runs on another machine,
so an OS picker would show the wrong disk, and its New session dialog needs to pick or create a
project folder there.

This doc is the contract for the gameui widget. How agent-ui uses it is in agent-ui's
`docs/design/folder-picker.md`.

## Goals

- All of the Forge FileDialog: `open`, `save` and `folder` modes; list and grid views; Places with
  Recent, fixed places and favourites; breadcrumbs and a typed path; history; type chips; hidden
  files; new folder; preview; the design's keyboard model.
- The caller owns the filesystem, the dialog owns navigation. The dialog never touches a disk, so
  the same widget serves a local disk, a remote server or a game's virtual file system.
- No allocation per frame once a folder's listing has been filtered and sorted. Recompute only when
  the listing, the filter, the sort or the toggles change.

  > **2026-10-09 update:** relaxed to "no filtering or sorting per frame". The cached view is
  > reused until the listing's generation, the location, the filter, the sort or a toggle changes
  > (tested), but drawing still builds a few short strings per frame (labels, crumbs, the type
  > strip), as other gameui widgets do.

## Non-goals

- Reading the disk. A caller backed by `std::fs` is a few lines in the gallery, not part of the
  widget.
- Thumbnails beyond what `Thumb` already draws (sprite, fill, colour). Decoding images is the
  caller's job.
- Multiple selection, drag and drop, rename or delete. The design has none of these.

## Domain model

### Paths

A path is a list of segments under the dialog's root, as in the design (`["levels", "act1"]`). The
root's crumb shows `root_label` ("~", a project name). The dialog doesn't know what the root is on
disk; the caller maps segments to real paths.

### Listings: the `FileSource` trait

```rust
pub enum Listing<'a> {
    Loading,
    Failed(&'a str),
    Entries(&'a [FileEntry]),
}

pub trait FileSource {
    /// The listing of `path`. Ask for it (once) and return `Loading` until it arrives.
    fn list(&mut self, path: &[String]) -> Listing<'_>;
}
```

> **2026-10-09 update:** `Entries` carries a `generation: u64` beside the entries, which changes
> whenever they do. The dialog keys its cached view on it, so a caller can refresh a listing
> (after creating a folder, say) without the dialog comparing entries.

It is the design's `fs(path) -> entries | null | { error }`. The dialog calls `list` for the folder
on show, and for a typed path when Enter is pressed. A typed path that comes back `Loading` is
gone to once it loads; one that comes back `Failed` keeps the field open with the error.

`FileEntry` mirrors the design's: `name`, `kind` (folder / file), `size`, `modified` (Unix
seconds), `hidden`, `thumb` (`Thumb` settings), `count`, `info` rows, and `dir` (for Recent
entries).

### State and output

`FileDialogState` is caller-owned and holds what the design keeps in React state: the path,
whether Recent is showing, history, selection, view, sort, filter text, types toggled off, "+N"
expanded, hidden toggle, the name field, the path field and its error, the new-folder field, the
favourites, the scroll positions and the type-ahead buffer. It also caches the filtered and sorted
view of the current listing, keyed by what it was built from.

`FileDialog` is the per-frame builder (mode, title, root label, places, recent, accept, save
extension, size, clock). `draw` returns at most one `FileDialogEvent`:

- `Confirm { path, name, replaces }`: the primary key, Enter, or a double-click on a file in open
  mode
- `Cancel`
- `CreateFolder { path, name }`: the caller creates it and refreshes that listing
- `FavouritesChanged`: read the list from the state and persist it

The dialog closes itself on `Confirm` and `Cancel`, like `PromptDialog`.

> **2026-10-09 update:** `draw` returns `FileDialogOutput { event, slot }`. `slot` is a footer
> rect reserved with `.footer_slot(width)`, where the caller draws a control of its own (agent-ui
> puts its "git init" checkbox there).

### Dates

The design shows "Today 14:02", "Yesterday 09:10", "Oct 3" or "Oct 3, 2024". gameui has no clock
or time zone database, so the builder takes `clock(now, utc_offset)` (Unix seconds and the local
offset in seconds), and the civil date is computed with the days-from-civil algorithm. Without a
clock, dates show as "—".

### Drawing

The surface is drawn by the widget itself, over a `Modal`-style backdrop with `ModalState` for
focus capture and Escape. It is not a `Sheet`, because the design's header and footer don't fit a
sheet's title / content / actions shape. Inside it, it reuses `IconKey`, `Breadcrumb` (for the
crumb trail's measure and paint), `SearchField`, `Checkbox`, `Thumb`, `ScrollView`, `TextInput`,
`Button` and the new `FilterChip`. Colours come from the theme's style keys where Forge has a
token; the few the design writes as literals (the folder glyph's oklch gradient, the places
column's shade) are constants in the widget.

> **2026-10-09 update:** the crumbs are drawn by the dialog rather than `Breadcrumb`, because they
> shrink earlier crumbs first and hand clicks on the well's empty space to the typed path; the
> listing is a `List` (which owns its `ScrollView`), so list and grid share selection and keys.

> **2026-10-09 update:** for hosts that build frames only when something changed (agent-ui's
> desktop), the dialog asks for a frame at the next local midnight while it shows dates against a
> `Clock`, and `FileDialogState::tick_clocks(dt)` lets ticks that draw nothing count toward the
> type-ahead's 700 ms pause (gameui has no wall clock; this mirrors `UiState::tick_clocks`).

## Phasing

1. Model and its unit tests.
2. `FilterChip`.
3. Surface, places, list view, footer, folder and open modes.
4. Typed path, new folder, history keys, type-ahead.
5. Grid view, save mode, preview, type strip, states.
6. Gallery entry and frame-loop tests.

## References

- Forge Design System (`1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`): `components/dialogs/FileDialog.jsx`,
  `FileDialog.d.ts`, `FileDialog.prompt.md`, `fileKit.jsx`, `filedialog.card.html`,
  `filedialog-states.card.html`; `components/keys/FilterChip.jsx`
- agent-ui `docs/design/folder-picker.md`, the first caller
