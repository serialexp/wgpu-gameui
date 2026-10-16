//! What the editing keys do to a [`Document`]: typing with brackets and
//! quotes closing themselves, Enter keeping the indent, Tab and Shift-Tab,
//! Backspace and Delete, the caret's steps, and the clipboard's text.
//!
//! Each works on the document's selection and leaves a new one, so the
//! widget only decides which to call.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::columns::{byte_at_column, column_of};
use crate::document::{Change, Document, EditKind, Selection};

/// The pairs that close themselves unless a language says otherwise.
pub const DEFAULT_PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

/// How the keys edit, which can depend on the language.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// Columns to a tab stop, and spaces to an indent step.
    pub tab_size: usize,
    /// Whether brackets and quotes close themselves.
    pub auto_close: bool,
    /// What closes itself: an opener and its closer. A quote is both.
    pub pairs: &'static [(char, char)],
    /// Whether a line whose text before the caret is this (trimmed at its
    /// end) opens a block, so Enter after it steps in.
    pub opens_block: fn(&str) -> bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            tab_size: 4,
            auto_close: true,
            pairs: DEFAULT_PAIRS,
            opens_block: opens_bracket,
        }
    }
}

impl Rules {
    fn indent_step(&self) -> String {
        " ".repeat(self.tab_size)
    }

    fn opener(&self, character: char) -> Option<char> {
        self.pairs
            .iter()
            .find(|&&(open, _)| open == character)
            .map(|&(_, close)| close)
    }

    fn is_closer(&self, character: char) -> bool {
        self.pairs.iter().any(|&(_, close)| close == character)
    }
}

/// Whether `head` ends with an opening bracket.
pub fn opens_bracket(head: &str) -> bool {
    head.ends_with(['{', '(', '['])
}

/// Whether `head` ends with `word` as a word of its own.
pub fn ends_with_word(head: &str, word: &str) -> bool {
    head.strip_suffix(word)
        .is_some_and(|before| !before.chars().next_back().is_some_and(is_word))
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Type `text`: over the selection, or around it for an opener; a pair for
/// an opener, or a step over the closer already there.
pub fn type_text(document: &mut Document, text: &str, rules: &Rules) {
    let selection = document.selection();
    let range = selection.range();
    let mut characters = text.chars();
    if let (Some(typed), None, true) = (characters.next(), characters.next(), rules.auto_close) {
        let close = rules.opener(typed);
        if !selection.is_empty() {
            if let Some(close) = close {
                let width = typed.len_utf8();
                let shifted = |at: usize| at + width;
                document.edit(
                    vec![
                        change(range.start..range.start, &typed.to_string()),
                        change(range.end..range.end, &close.to_string()),
                    ],
                    Selection {
                        anchor: shifted(selection.anchor),
                        head: shifted(selection.head),
                    },
                    EditKind::Other,
                );
                return;
            }
        } else {
            let at = selection.head;
            let next = document.char_after(at);
            if rules.is_closer(typed) && next == Some(typed) {
                document.set_selection(Selection::caret(at + typed.len_utf8()));
                return;
            }
            if let Some(close) = close {
                let quote = close == typed;
                let before_word = next.is_some_and(is_word);
                let after_word = quote && document.char_before(at).is_some_and(is_word);
                if !before_word && !after_word {
                    document.edit(
                        vec![change(at..at, &format!("{typed}{close}"))],
                        Selection::caret(at + typed.len_utf8()),
                        EditKind::Typing,
                    );
                    return;
                }
            }
        }
    }
    document.edit(
        vec![change(range.clone(), text)],
        Selection::caret(range.start + text.len()),
        EditKind::Typing,
    );
}

/// Enter: a new line with the indent of this one, a step further in after
/// a block's opener; between an opener and its closer, the closer goes onto
/// a line of its own below.
pub fn newline(document: &mut Document, rules: &Rules) {
    let selection = document.selection();
    let range = selection.range();
    let line_start = document.line_start(document.line_of(range.start));
    let head = document.slice(line_start..range.start);
    let indent: String = head
        .chars()
        .take_while(|&character| character == ' ' || character == '\t')
        .collect();
    let opens = (rules.opens_block)(head.trim_end());
    let ending = document.line_ending();
    let step = if opens {
        rules.indent_step()
    } else {
        String::new()
    };
    let closer_next = document
        .char_after(range.end)
        .is_some_and(|next| ")]}".contains(next));
    let (text, caret) = if opens && closer_next {
        let inner = format!("{ending}{indent}{step}");
        (
            format!("{inner}{ending}{indent}"),
            range.start + inner.len(),
        )
    } else {
        let text = format!("{ending}{indent}{step}");
        let caret = range.start + text.len();
        (text, caret)
    };
    document.edit(
        vec![change(range, &text)],
        Selection::caret(caret),
        EditKind::Other,
    );
}

/// Tab: indent the selected lines, or put spaces to the next tab stop.
pub fn indent(document: &mut Document, rules: &Rules) {
    let selection = document.selection();
    let range = selection.range();
    let Some(lines) = selected_lines(document, &range) else {
        let line = document.line_of(range.start);
        let line_start = document.line_start(line);
        let column = column_of(
            &document.line(line),
            range.start - line_start,
            rules.tab_size,
        );
        let spaces = " ".repeat(rules.tab_size - column % rules.tab_size);
        let caret = range.start + spaces.len();
        document.edit(
            vec![change(range, &spaces)],
            Selection::caret(caret),
            EditKind::Other,
        );
        return;
    };
    let step = rules.indent_step();
    let changes: Vec<Change> = lines
        .clone()
        .filter(|&line| document.line_start(line) != document.line_end(line))
        .map(|line| {
            let at = document.line_start(line);
            change(at..at, &step)
        })
        .collect();
    let grown = changes.len() * step.len();
    let start = document.line_start(*lines.start());
    let end = document.line_end(*lines.end()) + grown;
    let after = whole_lines(selection, start, end);
    document.edit(changes, after, EditKind::Other);
}

/// Shift-Tab: take a step of indent (or what there is of one) off the
/// caret's line or the selected lines.
pub fn outdent(document: &mut Document, rules: &Rules) {
    let selection = document.selection();
    let range = selection.range();
    let multiple = selected_lines(document, &range);
    let lines = multiple.clone().unwrap_or_else(|| {
        let line = document.line_of(range.start);
        line..=line
    });
    let mut changes = Vec::new();
    let mut taken = 0;
    for line in lines.clone() {
        let text = document.line(line);
        let width = if text.starts_with('\t') {
            1
        } else {
            text.bytes()
                .take(rules.tab_size)
                .take_while(|&byte| byte == b' ')
                .count()
        };
        if width > 0 {
            let at = document.line_start(line);
            changes.push(change(at..at + width, ""));
            taken += width;
        }
    }
    if changes.is_empty() {
        return;
    }
    let after = if multiple.is_some() {
        let start = document.line_start(*lines.start());
        let end = document.line_end(*lines.end()) - taken;
        whole_lines(selection, start, end)
    } else {
        let line_start = document.line_start(*lines.start());
        let moved = |at: usize| line_start.max(at.saturating_sub(taken));
        Selection {
            anchor: moved(selection.anchor),
            head: moved(selection.head),
        }
    };
    document.edit(changes, after, EditKind::Other);
}

/// The lines a selection over more than one line covers: from its first
/// line to its last, leaving out a last line it only reaches the start of.
/// `None` when it is on one line.
fn selected_lines(
    document: &Document,
    range: &Range<usize>,
) -> Option<std::ops::RangeInclusive<usize>> {
    let first = document.line_of(range.start);
    let mut last = document.line_of(range.end);
    if first == last {
        return None;
    }
    if range.end == document.line_start(last) {
        last -= 1;
    }
    Some(first..=last)
}

/// `start..end` selected, the same way round as `like`.
fn whole_lines(like: Selection, start: usize, end: usize) -> Selection {
    if like.anchor <= like.head {
        Selection {
            anchor: start,
            head: end,
        }
    } else {
        Selection {
            anchor: end,
            head: start,
        }
    }
}

/// Backspace: the selection; a pair together; a step of indent; or the
/// character before the caret, the line ending at a line's start.
pub fn backspace(document: &mut Document, rules: &Rules) {
    let selection = document.selection();
    if !selection.is_empty() {
        delete(document, selection.range());
        return;
    }
    let at = selection.head;
    if at == 0 {
        return;
    }
    let line = document.line_of(at);
    let line_start = document.line_start(line);
    if at == line_start {
        delete(document, document.line_end(line - 1)..at);
        return;
    }
    if rules.auto_close
        && let (Some(before), Some(after)) = (document.char_before(at), document.char_after(at))
        && rules.opener(before) == Some(after)
    {
        delete(document, at - before.len_utf8()..at + after.len_utf8());
        return;
    }
    let head = document.slice(line_start..at);
    if head.bytes().all(|byte| byte == b' ') {
        let column = head.len();
        let width = match column % rules.tab_size {
            0 => rules.tab_size,
            partial => partial,
        };
        delete(document, at - width..at);
        return;
    }
    let last = head.graphemes(true).next_back().map_or(0, str::len);
    delete(document, at - last..at);
}

/// Delete: the selection, or the character after the caret, the line
/// ending at a line's end.
pub fn delete_forward(document: &mut Document) {
    let selection = document.selection();
    if !selection.is_empty() {
        delete(document, selection.range());
        return;
    }
    let at = selection.head;
    let end = next_position(document, at);
    if end > at {
        delete(document, at..end);
    }
}

fn delete(document: &mut Document, range: Range<usize>) {
    let start = range.start;
    document.edit(
        vec![change(range, "")],
        Selection::caret(start),
        EditKind::Deleting,
    );
}

/// Where the caret goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// The line's indent, or its start when already there.
    Home,
    End,
    DocumentStart,
    DocumentEnd,
}

/// Move the caret, or with `extend` its end of the selection. Left and
/// Right without it collapse a selection to that side.
pub fn move_caret(document: &mut Document, motion: Motion, extend: bool) {
    let selection = document.selection();
    let at = selection.head;
    let range = selection.range();
    let target = match motion {
        Motion::Left if !extend && !selection.is_empty() => range.start,
        Motion::Right if !extend && !selection.is_empty() => range.end,
        Motion::Left => previous_position(document, at),
        Motion::Right => next_position(document, at),
        Motion::WordLeft => word_left(document, at),
        Motion::WordRight => word_right(document, at),
        Motion::Home => {
            let line = document.line_of(at);
            let line_start = document.line_start(line);
            let text = document.line(line);
            let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
            if at == line_start + indent {
                line_start
            } else {
                line_start + indent
            }
        }
        Motion::End => document.line_end(document.line_of(at)),
        Motion::DocumentStart => 0,
        Motion::DocumentEnd => document.len(),
    };
    select(document, selection, target, extend);
}

/// Move `lines` lines down (up when negative), keeping to `column`: the
/// column the caret had when a run of these began, which this fills in.
/// Past the first line is the start, past the last the end.
pub fn move_lines(
    document: &mut Document,
    lines: isize,
    extend: bool,
    column: &mut Option<usize>,
    tab_size: usize,
) {
    let selection = document.selection();
    let at = selection.head;
    let line = document.line_of(at);
    let wanted = *column.get_or_insert_with(|| {
        column_of(
            &document.line(line),
            at - document.line_start(line),
            tab_size,
        )
    });
    let target = line as isize + lines;
    let to = if target < 0 {
        0
    } else if target as usize >= document.line_count() {
        document.len()
    } else {
        let target = target as usize;
        document.line_start(target) + byte_at_column(&document.line(target), wanted, tab_size)
    };
    select(document, selection, to, extend);
}

fn select(document: &mut Document, from: Selection, to: usize, extend: bool) {
    document.set_selection(if extend {
        Selection {
            anchor: from.anchor,
            head: to,
        }
    } else {
        Selection::caret(to)
    });
}

pub fn select_all(document: &mut Document) {
    document.set_selection(Selection {
        anchor: 0,
        head: document.len(),
    });
}

/// The run `at` is in, for a double click: a word, a run of punctuation or
/// of spaces. At a line's end, the run before it.
pub fn word_at(document: &Document, at: usize) -> Range<usize> {
    let line = document.line_of(at);
    let line_start = document.line_start(line);
    let column = at - line_start;
    let runs = runs(&document.line(line));
    let run = runs
        .iter()
        .find(|run| run.range.contains(&column))
        .or_else(|| runs.last().filter(|run| run.range.end == column))
        .map_or(column..column, |run| run.range.clone());
    line_start + run.start..line_start + run.end
}

/// The line `at` is on, with its line ending, for a triple click.
pub fn line_at(document: &Document, at: usize) -> Range<usize> {
    let line = document.line_of(at);
    let end = if line + 1 < document.line_count() {
        document.line_start(line + 1)
    } else {
        document.len()
    };
    document.line_start(line)..end
}

/// The selected text, if any.
pub fn copied(document: &Document) -> Option<String> {
    let selection = document.selection();
    (!selection.is_empty()).then(|| document.slice(selection.range()))
}

/// Take the selected text out and give it back.
pub fn cut(document: &mut Document) -> Option<String> {
    let text = copied(document)?;
    let range = document.selection().range();
    document.edit(
        vec![change(range.clone(), "")],
        Selection::caret(range.start),
        EditKind::Other,
    );
    Some(text)
}

/// Put `text` over the selection, with the document's own line endings.
pub fn paste(document: &mut Document, text: &str) {
    let unified = text.replace("\r\n", "\n");
    let text = match document.line_ending() {
        "\n" => unified,
        ending => unified.replace('\n', ending),
    };
    let range = document.selection().range();
    let caret = range.start + text.len();
    document.edit(
        vec![change(range, &text)],
        Selection::caret(caret),
        EditKind::Other,
    );
}

fn change(range: Range<usize>, text: &str) -> Change {
    Change {
        range,
        text: text.to_owned(),
    }
}

/// The caret position one character before `at`: over a whole line ending
/// at a line's start.
fn previous_position(document: &Document, at: usize) -> usize {
    let line = document.line_of(at);
    let line_start = document.line_start(line);
    if at == line_start {
        return if line == 0 {
            0
        } else {
            document.line_end(line - 1)
        };
    }
    let head = document.slice(line_start..at);
    at - head.graphemes(true).next_back().map_or(0, str::len)
}

/// The caret position one character after `at`: over a whole line ending
/// at a line's end.
fn next_position(document: &Document, at: usize) -> usize {
    let line = document.line_of(at);
    let line_end = document.line_end(line);
    if at >= line_end {
        return if line + 1 < document.line_count() {
            document.line_start(line + 1)
        } else {
            at
        };
    }
    let rest = document.slice(at..line_end);
    at + rest.graphemes(true).next().map_or(0, str::len)
}

fn word_left(document: &Document, at: usize) -> usize {
    let line = document.line_of(at);
    let line_start = document.line_start(line);
    if at == line_start {
        return previous_position(document, at);
    }
    let column = at - line_start;
    runs(&document.line(line))
        .iter()
        .rev()
        .find(|run| run.class != Class::Space && run.range.start < column)
        .map_or(line_start, |run| line_start + run.range.start)
}

fn word_right(document: &Document, at: usize) -> usize {
    let line = document.line_of(at);
    let line_start = document.line_start(line);
    let line_end = document.line_end(line);
    if at >= line_end {
        return next_position(document, at);
    }
    let column = at - line_start;
    runs(&document.line(line))
        .iter()
        .find(|run| run.class != Class::Space && run.range.end > column)
        .map_or(line_end, |run| line_start + run.range.end)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punctuation,
}

#[derive(Clone, Debug)]
struct Run {
    range: Range<usize>,
    class: Class,
}

/// `line` cut into runs of word characters, of punctuation and of spaces.
fn runs(line: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (at, character) in line.char_indices() {
        let class = if character.is_whitespace() {
            Class::Space
        } else if is_word(character) {
            Class::Word
        } else {
            Class::Punctuation
        };
        let end = at + character.len_utf8();
        match runs.last_mut() {
            Some(run) if run.class == class => run.range.end = end,
            _ => runs.push(Run {
                range: at..end,
                class,
            }),
        }
    }
    runs
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
