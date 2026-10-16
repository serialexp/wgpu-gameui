//! Columns: where a character sits on the monospace grid, with tabs reaching
//! the next stop, and the line as it is drawn.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// The column after `grapheme`, which starts at `column`.
fn advance(column: usize, grapheme: &str, tab_size: usize) -> usize {
    if grapheme == "\t" {
        (column / tab_size + 1) * tab_size
    } else {
        column + 1
    }
}

/// The column `byte` of `line` is at. Each character is one column; a tab
/// reaches the next multiple of `tab_size`.
pub fn column_of(line: &str, byte: usize, tab_size: usize) -> usize {
    let mut column = 0;
    for (at, grapheme) in line.grapheme_indices(true) {
        if at >= byte {
            break;
        }
        column = advance(column, grapheme, tab_size);
    }
    column
}

/// The character boundary in `line` nearest `column`; the end of the line
/// when it is shorter. A tie goes to the earlier boundary.
pub fn byte_at_column(line: &str, column: usize, tab_size: usize) -> usize {
    let mut start = 0;
    for (at, grapheme) in line.grapheme_indices(true) {
        let end = advance(start, grapheme, tab_size);
        if column < end {
            return if column - start <= end - column {
                at
            } else {
                at + grapheme.len()
            };
        }
        start = end;
    }
    line.len()
}

/// A line as it is drawn: tabs as spaces to the next stop, and control
/// characters as their pictures (U+2400…), so nothing in it is invisible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    /// Where the drawn text stops matching the line byte for byte: from
    /// each `(line byte, extra)` on, a byte of the line is `extra` bytes
    /// further into the drawn text.
    shifts: Vec<(usize, isize)>,
}

impl Shown {
    pub fn new(line: &str, tab_size: usize) -> Self {
        let mut text = String::with_capacity(line.len());
        let mut shifts = Vec::new();
        let mut column = 0;
        for (at, grapheme) in line.grapheme_indices(true) {
            let next = advance(column, grapheme, tab_size);
            let before = text.len();
            match grapheme.chars().next() {
                Some('\t') => text.extend(std::iter::repeat_n(' ', next - column)),
                Some(control @ ('\0'..='\x1f' | '\x7f')) if grapheme.len() == 1 => {
                    text.push(picture(control));
                }
                _ => text.push_str(grapheme),
            }
            let grown = (text.len() - before) as isize - grapheme.len() as isize;
            if grown != 0 {
                let extra = shifts.last().map_or(0, |&(_, extra)| extra) + grown;
                shifts.push((at + grapheme.len(), extra));
            }
            column = next;
        }
        Self { text, shifts }
    }

    /// Where `byte` of the line is in the drawn text.
    pub fn to_shown(&self, byte: usize) -> usize {
        let index = self.shifts.partition_point(|&(at, _)| at <= byte);
        let extra = if index == 0 {
            0
        } else {
            self.shifts[index - 1].1
        };
        (byte as isize + extra) as usize
    }

    /// The boundary of `line` (the line this was made from) drawn nearest
    /// `shown`, a byte of the drawn text. Inside a tab, the nearer of its
    /// ends; a tie goes to the earlier one.
    pub fn to_line(&self, line: &str, shown: usize) -> usize {
        let boundaries: Vec<usize> = line
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .chain(std::iter::once(line.len()))
            .collect();
        let index = boundaries.partition_point(|&at| self.to_shown(at) < shown);
        if index == 0 {
            return 0;
        }
        if index == boundaries.len() {
            return line.len();
        }
        let (before, after) = (boundaries[index - 1], boundaries[index]);
        if shown - self.to_shown(before) <= self.to_shown(after) - shown {
            before
        } else {
            after
        }
    }

    /// `range` of the line, in the drawn text.
    pub fn range(&self, range: Range<usize>) -> Range<usize> {
        self.to_shown(range.start)..self.to_shown(range.end)
    }
}

/// The Unicode picture of a C0 control character or DEL.
fn picture(control: char) -> char {
    if control == '\x7f' {
        '\u{2421}'
    } else {
        char::from_u32(0x2400 + control as u32).unwrap_or('\u{fffd}')
    }
}

#[cfg(test)]
#[path = "columns_tests.rs"]
mod tests;
