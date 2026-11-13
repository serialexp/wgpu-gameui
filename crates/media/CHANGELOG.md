# Changelog

## 0.6.0 (2026-11-13)

### BREAKING CHANGES

- refactor!: split into a workspace with crates/core and crates/media

### Features

- video phase 2 — MP4, H.264 and AAC decoding on a worker, and a Player
- video phases 3 and 4 — sound and the player widgets

### Tests

- a slow device's test waits for the sound thread to have it
- the out-of-pictures test plays a file without sound

### Documentation

- usage docs on the types users see, and a doc check in the gate

