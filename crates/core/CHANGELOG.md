# Changelog

## 0.6.0 (2026-10-16)

### BREAKING CHANGES

- refactor!: split into a workspace with crates/core and crates/media
- video phase 1 — sprites updated in place, YUV sprites, full screen
- feat(core)!: draw-call batching

### Features

- transform gizmo seen through a perspective camera
- add stepped timelines and observation markers
- ListView rows can show a Thumb monogram
- video phases 3 and 4 — sound and the player widgets
- a code editor widget in a new crate, wgpu-gameui-code

### Tests

- ordered_paint_stress draws its barriers as triangle strips

