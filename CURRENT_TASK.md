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
- [x] C — DragList. Notes in TODO.md "Missing Forge components, batch C".
      Coverage 67 of 77.
- [x] D — inspector: PropertyRow, PropertyGroup, FileField, Inspector.
      Notes in TODO.md "Missing Forge components, batch D". Coverage 71
      of 77. Open question for Bart: Forge hides the Inspector body's
      scrollbar (`scrollbarWidth: none`); ours shows the thin overlay
      thumb, since `ScrollView` has no hidden-bar mode.
      Bart's decisions (2026-09-26):
      - Inspector body: a closure gets a vertical cursor
        (`body.take(h) -> Rect`); the body's height is measured while
        drawing and feeds the scroll (one frame of lag on the clamp is ok).
      - Slider/Checkbox "mixed" look: not in this batch — TODO item.
        PropertyRow and FileField have their own mixed state.
      - Ungate `IconKey`: only `IconKey::new(PhosphorIcon)` stays behind
        `phosphor-icons`; `IconKey::glyph` works everywhere.
      - PropertyRow matches Forge: scrub the label or step ▴▾, no typing.
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
  3cbe69e's (113 problems). After batch C it is that list minus 13 false
  `sibling_overlap`s between layers and the base (101 problems); a copy
  of the batch B list is `/tmp/probs-b.txt` while it lasts. Batch D adds
  nothing: still the same 101 (`/tmp/probs-c.txt`).
- **Scrolled content and the pointer.** `ScrollView::begin` translates
  drawing, not input. A widget that scrolls other widgets must hand them
  the pointer moved by the offset (and consumed outside the viewport):
  `DrawContext::reborrow_with_input`, as `Inspector::draw_body` does.
- **Eyeball small text closely.** The PropertyGroup "…" bug only showed at
  some x positions; zoom crops with `convert <png> -crop WxH+X+Y -scale
  600% /tmp/crop.png` catch what the full section image hides.
- Widgets whose ghost/popup floats over other content draw it on a layer
  after the base pass (the gallery does this for DragList's ghost), so the
  debug report sees it stacked, not as a sibling.
- Raised surfaces (Sheet, Toast) declare their shadow in their debug scope
  via `BoxShadow::ink_rect`; gallery cells for free-standing sheets reserve
  the shadow's room (`sheet_cell`) so it doesn't fall on neighbours.
  Bart (2026-09-26): keep that empty room — accurate warnings are worth
  the extra space. Don't pack shadowed cells tight.
