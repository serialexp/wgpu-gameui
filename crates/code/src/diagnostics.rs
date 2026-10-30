//! Problems in the text, as a host reports them (a language server, a
//! compiler's output): each a range of bytes with a severity and a message.
//! Between the host's reports they follow the edits made, so a squiggle
//! stays on its word while the lines around it change.

use std::ops::Range;

use crate::document::{Document, TextEdit};

/// How bad a problem is. Ordered worst first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    /// Its name as the hover message shows it, before the message.
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Info => "Info",
        }
    }
}

/// A problem in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Bytes of the document. Empty: drawn as the character after it.
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    /// A short code shown before the message: `E0425`.
    pub code: Option<String>,
}

impl Diagnostic {
    pub fn new(range: Range<usize>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            range,
            severity,
            message: message.into(),
            code: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// On the bytes `range` (a line, say): overlapping it, or, empty, at a
    /// place in it.
    fn touches(&self, range: &Range<usize>) -> bool {
        self.range.start <= range.end
            && (self.range.end > range.start
                || (self.range.is_empty() && self.range.start >= range.start))
    }
}

/// The problems in a document, in order of where they start.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    list: Vec<Diagnostic>,
    /// The longest range, so those touching a place are found from a little
    /// before it rather than from the start.
    longest: usize,
    counts: [usize; 3],
}

impl Diagnostics {
    /// Replace them with `list`, its ranges in `document`'s text as it is
    /// now. Each is clamped to the text and its characters; an empty one is
    /// widened to the word at its start, or the one character there.
    pub fn set(&mut self, document: &Document, mut list: Vec<Diagnostic>) {
        for diagnostic in &mut list {
            let start = document.boundary(diagnostic.range.start);
            let end = document.boundary(diagnostic.range.end).max(start);
            diagnostic.range = if start == end {
                widened(document, start)
            } else {
                start..end
            };
        }
        self.list = list;
        self.order();
    }

    /// Move them by `edits`, made in this order since they were set or last
    /// moved.
    pub fn follow(&mut self, edits: &[TextEdit]) {
        if edits.is_empty() || self.list.is_empty() {
            return;
        }
        for edit in edits {
            for diagnostic in &mut self.list {
                diagnostic.range = moved(diagnostic.range.clone(), edit);
            }
        }
        self.order();
    }

    pub fn all(&self) -> &[Diagnostic] {
        &self.list
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// How many there are of `severity`.
    pub fn count(&self, severity: Severity) -> usize {
        self.counts[severity as usize]
    }

    /// Those on the bytes `range`, in order of where they start.
    pub fn on(&self, range: Range<usize>) -> impl Iterator<Item = &Diagnostic> {
        let from = range.start.saturating_sub(self.longest);
        let first = self.list.partition_point(|d| d.range.start < from);
        let last = self.list.partition_point(|d| d.range.start <= range.end);
        self.list[first..last.max(first)]
            .iter()
            .filter(move |d| d.touches(&range))
    }

    /// The worst severity on the bytes `range`, if any.
    pub fn worst_on(&self, range: Range<usize>) -> Option<Severity> {
        self.on(range).map(|d| d.severity).min()
    }

    /// The first of `severity` starting after `at`, or, with none, the first
    /// of all: what the status strip's count steps to.
    pub fn next_after(&self, at: usize, severity: Severity) -> Option<&Diagnostic> {
        let mut of_kind = self.list.iter().filter(|d| d.severity == severity);
        let first = of_kind.clone().next();
        of_kind.find(|d| d.range.start > at).or(first)
    }

    /// Put them in order of where they start, and note what that changed.
    fn order(&mut self) {
        self.list
            .sort_by_key(|d| (d.range.start, d.severity, d.range.end));
        self.measure();
    }

    fn measure(&mut self) {
        self.longest = self.list.iter().map(|d| d.range.len()).max().unwrap_or(0);
        self.counts = [0; 3];
        for diagnostic in &self.list {
            self.counts[diagnostic.severity as usize] += 1;
        }
    }
}

/// The word starting at `at`, or the one character there, or nothing at the
/// end of the text.
fn widened(document: &Document, at: usize) -> Range<usize> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut end = at;
    while let Some(c) = document.char_after(end).filter(|&c| word(c)) {
        end += c.len_utf8();
    }
    if end == at
        && let Some(c) = document.char_after(at).filter(|&c| c != '\n' && c != '\r')
    {
        end += c.len_utf8();
    }
    at..end
}

/// `range` after `edit`: text typed before its start (or right at it) moves
/// it along, text typed inside stretches it, and what is deleted of it goes.
/// Deleted or replaced whole, it is left empty where it started.
fn moved(range: Range<usize>, edit: &TextEdit) -> Range<usize> {
    let deletes = edit.old_end > edit.start;
    if deletes && !range.is_empty() && edit.start <= range.start && edit.old_end >= range.end {
        return edit.start..edit.start;
    }
    let shift = |at: usize| at + edit.new_end - edit.old_end;
    let start = if range.start >= edit.old_end {
        shift(range.start)
    } else if range.start <= edit.start {
        range.start
    } else {
        // Its start was deleted: it starts after what replaced it.
        edit.new_end
    };
    let end = if range.end <= edit.start {
        range.end
    } else if range.end >= edit.old_end {
        shift(range.end)
    } else {
        // Its end was deleted: it ends where the edit began.
        edit.start
    };
    if end < start { end..end } else { start..end }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
