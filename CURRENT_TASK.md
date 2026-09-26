# Current task: organize the widget gallery like the Forge design system

Bart asked (2026-09-26) for every gallery entry to have a category and a
subcategory, mirroring the Forge Design System project in Claude Design
(<https://claude.ai/design/p/1f8b3bfd-a399-4bc7-b8e8-210da4ff4326>), so it is
easy to find things and to see which components we are missing.

## Status

**Done and committed (2026-09-26).** agent-ui-6d has since added its rows
(SpanTabs, plus Meter, WellChip, Waffle and Band as "not in Forge"), which
took coverage to 51 of 73. Open follow-ups live in TODO.md.

What landed (uncommitted, `tests/widget_gallery.rs`, README.md, CLAUDE.md):

- `Category` enum (15: Forge's 13 + Foundations + Engine) and the
  checked-in `FORGE_COMPONENTS` list.
- `flow.section(list, Category::X, "ForgeName", "subtitle")` — one section
  per component; duplicates and non-PascalCase names fail the test.
- Output: `widget_gallery/<category>/<Component>.png`, cells under
  `widget_gallery/<category>/<Component>/`, and `index.html` grouped by
  category with a coverage summary, "not in Forge" tags and a missing list.
- Every mixed section was split (the old "4a" row, "Widgets", status dots ·
  hue chips, banners/toasts, menu bar/sheet, …).
- Popover cell is now sized from the measured sheet (it covered its label).
- Earlier (commit `4efbdec`): no full-canvas PNG any more.

Forge coverage: **51 of 73**. Missing (22): Console; CountBubble, DragList,
DropZone; AlertDialog, ConfirmDialog, PromptDialog; Placeholder, StatusIcon;
FieldLabel; FileField, Inspector, PropertyGroup, PropertyRow; Key; Modal,
Panel, Sheet; CommandPalette; SettingRow, SettingsPanel; DockSection (drawn
inside DockStack, but has no section of its own).

Follow-ups noted in TODO.md: Foundations token cards; the 104 older
problems in `widget_gallery.debug.txt`.

## Decisions (Bart, 2026-09-26)

- **One entry per Forge component.** Split mixed sections so each component
  gets its own entry under its Forge category.
- **gameui-only things:** Foundations (Type, Icons, …) like Forge, plus a new
  **Engine** category for rendering/layout machinery.
- **Gaps are shown on the index page only.** The test does not fail on gaps.
- **Foundations token cards** are a later follow-up — noted in TODO.md.

## Forge catalogue (from `_ds_manifest.json`, 2026-09-26)

Foundations cards: Brand (Wordmark); Colors (Accent roles, Axis tints, Ink
ladder, Semantic hues, Host surfaces, Wash vocabulary); Depth (Two-line
edges, Key states, Raised · sunken · floating); Focus; Motion; Spacing
(Radius, Heights, Gaps); Type (Carved text, IBM Plex Mono, IBM Plex Sans).

Components (73):

- chrome: Console, ContextMenu, DockPanel, MenuBar, MenuSheet, StatusBar
- data: AssetGrid, Badge, CountBubble, DragList, DropZone, ListView, Table,
  Thumb, Tree
- dialogs: AlertDialog, ConfirmDialog, PromptDialog
- editors: ColorPicker, CurveEditor, GradientRamp
- feedback: Banner, BusyDots, EmptyState, Placeholder, Skeleton, Spinner,
  StatusIcon, Toast, Tooltip
- forms: Checkbox, ComboBox, Dropdown, FieldLabel, NumberField, ProgressBar,
  Radio, SearchField, Slider, Switch, TagInput, TextArea, TextField,
  VectorField
- inspector: FileField, Inspector, PropertyGroup, PropertyRow
- keys: Button, FilterChip, IconKey, Key, Keycap, Toolbar
- layout: Breadcrumb, DocumentTabs, Group, Modal, Pager, Panel, Popover,
  ScrollArea, SegmentedTabs, Separator, Sheet, SpanTabs, Splitter
- palette: CommandPalette
- settings: SettingRow, SettingsPanel
- sidebar: DockSection, DockStack
- windows: FloatingWindow

UI kits: Level Editor (Docking, Level Editor).
