//! A retained, signal-driven tree drawn with wgpu-gameui (a prototype; see
//! `docs/design/retained-mode.md`).
//!
//! Node properties are constants or memos over signals. A frame lays out
//! and records only what changed; every node with children keeps the list
//! of its subtree and is placed into its parent's again as it is. Pointer
//! input is hit-tested against the last frame's layout and written to
//! signals.

mod input;
mod layout;
pub mod node;
mod own_drawing;
mod paint;
pub mod signal;
mod texts;
mod ui;

pub use input::{Button, InputStats};
pub use node::{
    Align, Border, Direction, Insets, Look, Node, Paint, Painter, Prop, Size, Spec, Stack, Text,
};
pub use signal::{Graph, GraphStats, Memo, Owner, Reader, Signal};
pub use ui::{FrameContext, FrameStats, TreeStats, Ui};

#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod tests;
