//! A retained mode for wgpu-gameui: a tree of nodes whose properties are
//! signals, drawn by recording only what changed and replaying the rest.
//! A prototype; see `docs/design/retained-mode.md`.

pub mod signal;

pub use signal::{Graph, GraphStats, Memo, Owner, Reader, Signal};
