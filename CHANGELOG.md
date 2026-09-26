# Changelog

## 0.6.0 (2026-09-26)

### BREAKING CHANGES

- feat!: sidebar widgets, shared shaping cache, cosmic-text 0.19
- feat!: sidebar GroupList, toolbar tooltips, hollow keys, danger menu rows
- feat!: merge badge, badge_toned and hue_chip into one Badge widget
- feat!: add Pressable (Forge Key) and build every key on it
- feat!: add CountBubble, StatusIcon, Placeholder, DropZone and FieldLabel; rebuild Panel on Forge
- feat!: add Sheet, Modal, AlertDialog, ConfirmDialog and PromptDialog

### Features

- rotated chrome stays SDF; new meter/waffle/span-tab widgets; Forge-style gallery
- add UiRenderer::set_view_origin to render a window of a larger canvas
- add DragList
- add PropertyRow, PropertyGroup, FileField and Inspector

### Bug Fixes

- stop filling shadow order slots past the end
- remove the seam across quads rounded on one side only
- stop flagging layers as overlapping the content under them
- build all targets without default features

### Performance Improvements

- match ASCII filter text byte by byte

### Tests

- wipe the whole gallery output directory before each run
- browse the gallery through an index page, drop the full image
- sync gallery FORGE_COMPONENTS with the current Forge manifest
- bench DragList frames up to 100,000 items

### Documentation

- add Forge widget gallery reference and screenshot
- move back to 0.x after yanking 1.0.0

## 1.0.0 (2026-09-24)

### BREAKING CHANGES

- feat!: blend UI colours in sRGB space like the browser
- fix!: use DesignSync accent and palette values
- feat!: make state changes instant; keep smooth scrolling

### Features

- return repaint requirements and deadlines from a UI frame
- add GPU-native forged chrome and analytic shadows
- conform chrome to Forge dark design, add recursive menus and UiContext verbs
- add movable Window widget and immediate submenu hover

### Bug Fixes

- match Forge design sizing — 12px body text, 28px wells, 26px buttons
- match widget sizing to Forge design tokens
- use Phosphor vector icons for close buttons and stepper carets

### Styles

- apply rustfmt to the settings-form set

## 0.5.0 (2026-09-17)

### Bug Fixes

- stop pinning shaped text width in the height-clip test

## 0.5.0 (2026-09-17)

### Features

- add contextual widget measurement
- add optional syntax highlighting
- add letter spacing and dropdown intent claiming
- add a one-level menubar widget
- implement the 4a default UI system
- add toolbar, dock panel, and app shell widgets
- add menu-screen primitives (scrim, MenuList, arrow focus, ImageFit::Tile)
- add declarative settings form with control-rebinding fields

### Bug Fixes

- make the submission, not the call, the GPU scratch lifetime
- map glyphs and dotted captures to the right styles
- settings dropdown popup honors the form's style overrides

### Styles

- apply rustfmt to the toolbar/dock/shell set

## 0.4.0 (2026-09-02)

### Features

- add selection clipboard and scrolling

## 0.3.1 (2026-08-23)

### Bug Fixes

- generate MSDF glyphs lazily

## 0.3.0 (2026-08-22)

### Features

- widgets declare their own allocation; app API drops the rect
- report layout-peer overlaps by default
- fit text buttons to content by default
- fit labels and selection controls to content
- add phosphor icon buttons
- order painting and unify hit geometry
- add binding-neutral stack declarations

### Bug Fixes

- viewport content no longer inflates its scope's bounds; ColorPicker fits its rect
- measure text ink, not the line box it is centred in
- alignment analysis only claims what the frame can support
- use logical dimensions for the text ortho projection
- ellipsize labels and enforce formatting
- ellipsize labels within their rows
- stabilize ordered paint uploads

### Documentation

- separate 'name' from 'declared rect' in the inspection docs
- rect declaration is a widget-implementor concern

### Chores

- satisfy Rust 1.97 clippy
- enforce format and strict clippy

### Other

- vector icon support (phosphor + custom icon fonts)

## Unreleased

### Changed

- `DrawList` method calls now preserve painter's submission order across
  primitive families instead of globally forcing text above icons and geometry.
- Added retained, transformed, clipped hit geometry and topmost-first interaction
  responses for stable-ID widgets; legacy immediate widget APIs remain available.
- Added binding-neutral declarative stack children, identity-bearing layout
  results, and immediate local rectangle scopes for drawing resolved layouts once.

## 0.2.0 (2026-08-04)

### Features

- layout inspection report, lints, and headless capture

### Chores

- gitignore stray gothab-plans/ agent artifacts

### CI

- automate releases with just-release

