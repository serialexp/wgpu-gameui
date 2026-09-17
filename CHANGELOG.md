# Changelog

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

