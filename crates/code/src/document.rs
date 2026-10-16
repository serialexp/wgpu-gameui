//! The text being edited: a rope, the selection, and the history that undo
//! and redo walk.
//!
//! Positions are byte offsets, always on character boundaries, because
//! Tree-sitter speaks bytes. Only `\n` ends a line (the rope is built without
//! Unicode line breaks), so a line here is a row to Tree-sitter.

use std::ops::Range;

use ropey::Rope;

/// The selected text: from where it was started (`anchor`) to where the
/// caret is (`head`). Empty, it is just the caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// Just the caret, at `at`.
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// What it covers, first byte first.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// A row and a byte column in it, as Tree-sitter counts them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

/// One change to the text, as the parser is told it: what was between
/// `start` and `old_end` is now between `start` and `new_end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
    pub start_point: Point,
    pub old_end_point: Point,
    pub new_end_point: Point,
}

/// Replace `range` with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub range: Range<usize>,
    pub text: String,
}

/// What an edit was, for grouping the history: a run of typing, or of
/// deleting, is one step to undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    /// Anything else, a step of its own.
    Other,
}

/// One change as it was made, so it can be unmade.
#[derive(Clone, Debug)]
struct Applied {
    start: usize,
    removed: String,
    inserted: String,
}

/// One step of the history: its changes in the order they were made.
#[derive(Clone, Debug)]
struct Step {
    changes: Vec<Applied>,
    before: Selection,
    after: Selection,
    kind: EditKind,
}

/// The text being edited. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Document {
    rope: Rope,
    /// What Enter inserts: the file's first line ending.
    line_ending: &'static str,
    selection: Selection,
    /// Counts every change to the text, undo and redo too.
    version: u64,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The last step can still grow: nothing but more of the same kind of
    /// edit has happened since it.
    open: bool,
    /// The changes the parser hasn't been told about.
    edits: Vec<TextEdit>,
}

impl Document {
    pub fn new(text: &str) -> Self {
        let rope = Rope::from_str(text);
        let line_ending = match text.find('\n') {
            Some(at) if text[..at].ends_with('\r') => "\r\n",
            _ => "\n",
        };
        Self {
            rope,
            line_ending,
            selection: Selection::default(),
            version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            open: false,
            edits: Vec::new(),
        }
    }

    /// The whole text.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The text in `range`.
    pub fn slice(&self, range: Range<usize>) -> String {
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.slice(start..end).to_string()
    }

    /// The text in `range`, piece by piece, without copying it.
    pub fn chunks(&self, range: Range<usize>) -> impl Iterator<Item = &str> {
        self.rope.byte_slice(range).chunks()
    }

    /// Its length in bytes.
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// The text from `at` to the end of the rope's piece holding it: the
    /// parser reads the text this way, without it being copied whole.
    /// Empty at the end.
    pub fn chunk_at(&self, at: usize) -> &str {
        if at >= self.rope.len_bytes() {
            return "";
        }
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(at);
        &chunk[at - chunk_start..]
    }

    /// How many lines: one more than there are `\n`s.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line `index`, without its line ending.
    pub fn line(&self, index: usize) -> String {
        let start = self.line_start(index);
        self.slice(start..self.line_end(index))
    }

    /// Where line `index` starts.
    pub fn line_start(&self, index: usize) -> usize {
        self.rope.line_to_byte(index)
    }

    /// Where line `index` ends, before its line ending.
    pub fn line_end(&self, index: usize) -> usize {
        let line = self.rope.line(index);
        let mut end = self.line_start(index) + line.len_bytes();
        let mut chars = line.chars_at(line.len_chars()).reversed();
        if chars.next() == Some('\n') {
            end -= 1;
            if chars.next() == Some('\r') {
                end -= 1;
            }
        }
        end
    }

    /// The line `at` is on. A line's `\n` is its own.
    pub fn line_of(&self, at: usize) -> usize {
        self.rope.byte_to_line(at)
    }

    /// The character just before `at`, a line ending's too.
    pub fn char_before(&self, at: usize) -> Option<char> {
        let index = self.rope.byte_to_char(at);
        (index > 0).then(|| self.rope.char(index - 1))
    }

    /// The character at `at`, a line ending's too.
    pub fn char_after(&self, at: usize) -> Option<char> {
        let index = self.rope.byte_to_char(at);
        (index < self.rope.len_chars()).then(|| self.rope.char(index))
    }

    /// `at` as a row and a byte column.
    pub fn point(&self, at: usize) -> Point {
        let row = self.rope.byte_to_line(at);
        Point {
            row,
            column: at - self.rope.line_to_byte(row),
        }
    }

    /// What Enter inserts: `"\r\n"` when the file's first line ends that
    /// way, otherwise `"\n"`.
    pub fn line_ending(&self) -> &'static str {
        self.line_ending
    }

    /// Goes up with every change to the text.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Move the caret or the selection. Moving it ends a run of typing or
    /// deleting, so what comes next is a step of its own.
    pub fn set_selection(&mut self, selection: Selection) {
        if selection != self.selection {
            self.open = false;
        }
        self.selection = selection;
    }

    /// Make `changes`, all positioned in the text as it is now, and leave
    /// `after` selected. They mustn't overlap.
    pub fn edit(&mut self, mut changes: Vec<Change>, after: Selection, kind: EditKind) {
        // Last first, so each change's positions still hold when it comes.
        changes.sort_by_key(|change| std::cmp::Reverse(change.range.start));
        debug_assert!(
            changes
                .windows(2)
                .all(|pair| pair[1].range.end <= pair[0].range.start),
            "overlapping changes"
        );
        let applied: Vec<Applied> = changes
            .into_iter()
            .map(|change| self.apply(change.range, &change.text))
            .collect();
        self.redo.clear();
        let before = self.selection;
        match self.undo.last_mut() {
            Some(last)
                if self.open
                    && kind != EditKind::Other
                    && last.kind == kind
                    && last.after == before =>
            {
                last.changes.extend(applied);
                last.after = after;
            }
            _ => self.undo.push(Step {
                changes: applied,
                before,
                after,
                kind,
            }),
        }
        self.open = kind != EditKind::Other;
        self.selection = after;
    }

    /// Unmake the last step. `false` when there is none.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        for change in step.changes.iter().rev() {
            let end = change.start + change.inserted.len();
            self.apply(change.start..end, &change.removed);
        }
        self.selection = step.before;
        self.redo.push(step);
        self.open = false;
        true
    }

    /// Make again the last step undone. `false` when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        for change in &step.changes {
            let end = change.start + change.removed.len();
            self.apply(change.start..end, &change.inserted);
        }
        self.selection = step.after;
        self.undo.push(step);
        self.open = false;
        true
    }

    /// The changes made since this was last asked, in the order they were
    /// made, for the parser.
    pub fn take_edits(&mut self) -> Vec<TextEdit> {
        std::mem::take(&mut self.edits)
    }

    /// Replace `range` with `text`, noting it for the parser.
    fn apply(&mut self, range: Range<usize>, text: &str) -> Applied {
        let start_point = self.point(range.start);
        let old_end_point = self.point(range.end);
        let removed = self.slice(range.clone());
        let start_char = self.rope.byte_to_char(range.start);
        let end_char = self.rope.byte_to_char(range.end);
        self.rope.remove(start_char..end_char);
        self.rope.insert(start_char, text);
        let new_end = range.start + text.len();
        self.edits.push(TextEdit {
            start: range.start,
            old_end: range.end,
            new_end,
            start_point,
            old_end_point,
            new_end_point: self.point(new_end),
        });
        self.version += 1;
        Applied {
            start: range.start,
            removed,
            inserted: text.to_owned(),
        }
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
