//! The undo and redo history of a [`TextInput`](super::TextInput).
//!
//! Every edit a field makes replaces one byte range of its text, so a step of
//! the history is one such replacement: where it was, what it took out and
//! what it put in, and where the caret stood on either side. A run of typing,
//! of backspacing or of forward deleting grows the step before it rather than
//! starting one, so one undo takes back a run, as other editors do; moving the
//! caret, or any other kind of edit, ends the run.
//!
//! A step holds only the text it changed, and growing a run appends to it, so
//! a keystroke costs what it typed however long the field's text is. The
//! history keeps at most [`MAX_STEPS`] steps and [`MAX_BYTES`] of their text,
//! forgetting the oldest first; the latest step is kept whatever its size.
//!
//! The field's `value` is public, so its text can change without the history
//! seeing it. A step is unmade only where the text still holds what the step
//! left there; anywhere else the history no longer describes the text, and is
//! forgotten rather than applied to text it doesn't fit.

use std::collections::VecDeque;
use std::ops::Range;

use super::text_input::overwrite;

/// The most steps kept.
const MAX_STEPS: usize = 1000;
/// The most bytes of removed and inserted text the kept steps hold.
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// What an edit was, for grouping runs of it into one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    /// Typed text: a run of it is one step.
    Typing,
    /// Backspace or Delete: a run of either is one step.
    Deleting,
    /// Anything else (a paste, a cut, a new line, clearing the field): a step
    /// of its own.
    Other,
}

/// Where the caret stood and what was selected: a field's `cursor_pos` and
/// `selection_start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Caret {
    pub(crate) cursor: usize,
    pub(crate) selection: Option<usize>,
}

/// One step: the text at `start` that was `removed` is now `inserted`.
#[derive(Clone, Debug)]
struct Step {
    start: usize,
    removed: String,
    inserted: String,
    before: Caret,
    after: Caret,
    kind: EditKind,
}

impl Step {
    fn bytes(&self) -> usize {
        self.removed.len() + self.inserted.len()
    }

    /// Grow this step by `edit` when `edit` carries on its run: typing
    /// straight after what it typed, backspacing straight before what it
    /// deleted, or deleting forward from where it did.
    fn absorb(&mut self, edit: &Edit<'_>) -> bool {
        if edit.kind != self.kind || edit.before != self.after {
            return false;
        }
        match edit.kind {
            EditKind::Typing
                if edit.removed.is_empty() && edit.start == self.start + self.inserted.len() =>
            {
                self.inserted.push_str(edit.inserted);
            }
            EditKind::Deleting if edit.inserted.is_empty() && self.inserted.is_empty() => {
                if edit.start + edit.removed.len() == self.start {
                    self.removed.insert_str(0, edit.removed);
                    self.start = edit.start;
                } else if edit.start == self.start {
                    self.removed.push_str(edit.removed);
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        self.after = edit.after;
        true
    }
}

/// An edit as a field made it: the history copies what it keeps.
pub(crate) struct Edit<'a> {
    pub(crate) start: usize,
    pub(crate) removed: &'a str,
    pub(crate) inserted: &'a str,
    pub(crate) before: Caret,
    pub(crate) after: Caret,
    pub(crate) kind: EditKind,
}

/// The edits a field can undo and redo.
#[derive(Clone, Debug, Default)]
pub(crate) struct TextHistory {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    /// The bytes the steps in `undo` and `redo` hold.
    bytes: usize,
    /// The last step can still grow: no undo or redo came since it.
    open: bool,
}

impl TextHistory {
    /// Note `edit`, which the field has just made.
    pub(crate) fn record(&mut self, edit: Edit<'_>) {
        if edit.removed.is_empty() && edit.inserted.is_empty() {
            return;
        }
        self.bytes -= self.redo.iter().map(Step::bytes).sum::<usize>();
        self.redo.clear();
        let grown = self.open
            && self.undo.back_mut().is_some_and(|last| {
                let was = last.bytes();
                let grown = last.absorb(&edit);
                if grown {
                    self.bytes += last.bytes() - was;
                }
                grown
            });
        if !grown {
            let step = Step {
                start: edit.start,
                removed: edit.removed.to_owned(),
                inserted: edit.inserted.to_owned(),
                before: edit.before,
                after: edit.after,
                kind: edit.kind,
            };
            self.bytes += step.bytes();
            self.undo.push_back(step);
        }
        self.open = edit.kind != EditKind::Other;
        while self.undo.len() > 1 && (self.undo.len() > MAX_STEPS || self.bytes > MAX_BYTES) {
            if let Some(oldest) = self.undo.pop_front() {
                self.bytes -= oldest.bytes();
            }
        }
    }

    /// Unmake the last step in `text`, and say where the caret goes. `None`
    /// when there is nothing to undo, or the text no longer fits the history
    /// (which is then forgotten).
    pub(crate) fn undo(&mut self, text: &mut String) -> Option<Caret> {
        let step = self.undo.pop_back()?;
        let caret = step.before;
        if !self.replace(text, step.start, &step.inserted, &step.removed) {
            return None;
        }
        self.redo.push(step);
        self.open = false;
        Some(caret)
    }

    /// Make again the last step undone, as [`undo`](Self::undo) unmakes one.
    pub(crate) fn redo(&mut self, text: &mut String) -> Option<Caret> {
        let step = self.redo.pop()?;
        let caret = step.after;
        if !self.replace(text, step.start, &step.removed, &step.inserted) {
            return None;
        }
        self.undo.push_back(step);
        self.open = false;
        Some(caret)
    }

    /// Put `with` in place of `was` at `start` in `text`, if `was` is what
    /// is there; otherwise forget everything, the history no longer
    /// describing the text.
    fn replace(&mut self, text: &mut String, start: usize, was: &str, with: &str) -> bool {
        let range: Range<usize> = start..start + was.len();
        if text.get(range.clone()) != Some(was) {
            self.forget();
            return false;
        }
        text.replace_range(range, with);
        true
    }

    /// Overwrite the text every step holds, then forget them: for a field
    /// that held a secret (see [`overwrite`]).
    pub(crate) fn wipe(&mut self) {
        for step in self.undo.iter_mut().chain(&mut self.redo) {
            overwrite(&mut step.removed);
            overwrite(&mut step.inserted);
        }
        self.forget();
    }

    /// Forget every step: the text was replaced from outside the field.
    pub(crate) fn forget(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.open = false;
    }

    #[cfg(test)]
    pub(crate) fn steps(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

#[cfg(test)]
#[path = "text_history_tests.rs"]
mod tests;
