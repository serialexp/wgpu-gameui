# Current task: build the Forge components the gallery is still missing

Bart asked (2026-09-26): "pull the currently missing things from designsync
and build those components". Source of truth is the Forge Design System in
Claude Design (project `1f8b3bfd-a399-4bc7-b8e8-210da4ff4326`): each
component has `components/<group>/<Name>.jsx`, `.prompt.md` and `.d.ts`.
Fetch them with the DesignSync tool (`get_file`).

The previous task (Forge-style gallery) is done; its notes are in git
history (`git log -- CURRENT_TASK.md`).

## Missing (23 of 77, from `test_output/widget_gallery/index.html`)

Batches, smallest first; one commit per batch:

- [x] A — small: CountBubble, DropZone, FieldLabel, StatusIcon, Placeholder,
      Panel (reworked to Forge), DockSection (gallery section only).
      Notes in TODO.md "Missing Forge components, batch A".
- [x] B — dialogs: Modal, AlertDialog, ConfirmDialog, PromptDialog, Sheet.
      Notes in TODO.md "Missing Forge components, batch B". Coverage is
      66 of 77.
- [ ] C — DragList
- [ ] D — inspector: PropertyRow, PropertyGroup, FileField, Inspector
- [ ] E — mobile: AppBar, TabBar
- [ ] F — CommandPalette
- [ ] G — Console
- [ ] H — SettingRow, SettingsPanel (overlap with the existing
      `SettingsForm` in `src/widgets/settings.rs` — ask Bart how to
      reconcile before building)

## Notes

- **Gallery pages.** The canvas is too tall for one texture, so the
  gallery renders pages of at most `PAGE_MAX` (4096) rows through
  `UiRenderer::set_view_origin`, split between sections. A section taller
  than a page fails the test with a clear message. Bart: no full-canvas
  image is wanted, only the section and cell images and the index page.
- **Debug-report check per batch.** Compare the gallery's problem list
  with the previous commit's. Export that commit's tree into /tmp with
  `git show <rev>:<path>` for every file of `git ls-tree -r --name-only
  <rev>` (no `git archive`/`worktree`: not on the git allowlist), run its
  gallery with `CARGO_TARGET_DIR=/tmp/gameui-head-target`, and diff
  `code name` pairs from `test_output/widget_gallery.debug.json`, sorted,
  with `#<digits>` normalised to `#N`. After batch B the list equals
  3cbe69e's (113 problems).
- Raised surfaces (Sheet, Toast) declare their shadow in their debug scope
  via `BoxShadow::ink_rect`; gallery cells for free-standing sheets reserve
  the shadow's room (`sheet_cell`) so it doesn't fall on neighbours.
