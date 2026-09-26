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
- [ ] B — dialogs: Modal (a `LayerKind::Modal` exists in `src/layer.rs`,
      no visual widget), AlertDialog, ConfirmDialog, PromptDialog, Sheet
- [ ] C — DragList
- [ ] D — inspector: PropertyRow, PropertyGroup, FileField, Inspector
- [ ] E — mobile: AppBar, TabBar
- [ ] F — CommandPalette
- [ ] G — Console
- [ ] H — SettingRow, SettingsPanel (overlap with the existing
      `SettingsForm` in `src/widgets/settings.rs` — ask Bart how to
      reconcile before building)

## Notes
