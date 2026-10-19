//! A source code editor for wgpu-gameui, after the Forge design system's
//! `CodeEditor`: text in a rope, the editing rules programmers expect, and
//! Tree-sitter colours. See `docs/design/code-editor.md`.

mod columns;
mod commands;
mod document;
mod editor;
mod highlight;
mod language;

pub use columns::{Piece, byte_at_column, column_of, drawn_text, pieces};
pub use commands::{
    DEFAULT_PAIRS, Motion, Rules, backspace, copied, cut, delete_forward, ends_with_word, indent,
    line_at, move_caret, move_lines, newline, opens_bracket, outdent, paste, select_all, type_text,
    word_at,
};
pub use document::{Change, Document, EditKind, Point, Selection, TextEdit};
pub use editor::{CodeEditor, CodeEditorOutput, CodeTheme};
pub use highlight::{Span, Syntax};
pub use language::Language;
