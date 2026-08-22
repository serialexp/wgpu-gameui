# Changelog

## Unreleased

### Changed

- `DrawList` method calls now preserve painter's submission order across
  primitive families instead of globally forcing text above icons and geometry.
- Added retained, transformed, clipped hit geometry and topmost-first interaction
  responses for stable-ID widgets; legacy immediate widget APIs remain available.

## 0.2.0 (2026-08-04)

### Features

- layout inspection report, lints, and headless capture

### Chores

- gitignore stray gothab-plans/ agent artifacts

### CI

- automate releases with just-release

