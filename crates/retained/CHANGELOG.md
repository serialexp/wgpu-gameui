# Changelog

## 0.6.0 (2026-11-13)

### Features

- a signal graph with mark-then-pull propagation
- the tree, its layout and cached drawing (Phase 3)
- pointer input (Phase 4)
- TreeStats counts the bytes lists and the node table hold
- lists that report their edits skip the frame comparison
- an example in a window

### Performance Improvements

- patch a changed list's records where its ancestors placed them
- keep lists only at the root and at nodes that clip
- lay out a scroll's children only near its view
- lay out at least half the window past a scroll's view
- keep bound props in a slotmap and the log beside the list
- count only passes that shape, and measure what kept shaping costs
- text nodes keep their shaping, within a byte budget
- a placed log keeps a piece's numbers that are not zero, packed
- a clipping node out of view drops its list

### Documentation

- fix unresolved and private intra-doc links

