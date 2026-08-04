# Changelog

## 0.3.0 (2026-08-04)

### Features

- widgets declare their own allocation; app API drops the rect

### Bug Fixes

- viewport content no longer inflates its scope's bounds; ColorPicker fits its rect
- measure text ink, not the line box it is centred in
- alignment analysis only claims what the frame can support

### Documentation

- separate 'name' from 'declared rect' in the inspection docs
- rect declaration is a widget-implementor concern

## 0.2.0 (2026-08-04)

### Features

- layout inspection report, lints, and headless capture

### Chores

- gitignore stray gothab-plans/ agent artifacts

### CI

- automate releases with just-release

