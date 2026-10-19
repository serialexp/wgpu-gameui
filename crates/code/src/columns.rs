//! Columns: where a character sits on the monospace grid, and the line as
//! it is drawn there. A tab reaches the next stop, a wide character (most
//! Chinese, Japanese and Korean, and emoji) takes two columns, and any other
//! character one, so a place in a line is its column times the font's
//! advance, without measuring.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The column after `grapheme`, which starts at `column`.
fn advance(column: usize, grapheme: &str, tab_size: usize) -> usize {
    if grapheme == "\t" {
        (column / tab_size + 1) * tab_size
    } else {
        column + cells(grapheme)
    }
}

/// How many columns `grapheme`, not a tab, takes. Something drawn as
/// nothing alone (a zero-width space) still takes one, so the caret can be
/// seen to step over it, as does a control character's picture.
fn cells(grapheme: &str) -> usize {
    grapheme.width().clamp(1, 2)
}

/// The column `byte` of `line` is at.
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

/// The character boundary in `line` nearest `column`, which may fall inside
/// a column; the end of the line when it is shorter. A tie goes to the
/// earlier boundary.
pub fn byte_at_column(line: &str, column: f32, tab_size: usize) -> usize {
    let mut start = 0;
    for (at, grapheme) in line.grapheme_indices(true) {
        let end = advance(start, grapheme, tab_size);
        if column < end as f32 {
            return if column - start as f32 <= end as f32 - column {
                at
            } else {
                at + grapheme.len()
            };
        }
        start = end;
    }
    line.len()
}

/// A piece of a line as it is drawn, at its column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub column: usize,
    /// The bytes of the line it shows.
    pub source: Range<usize>,
}

/// The pieces of `line` drawn in `columns`, and how many columns the whole
/// line takes. Plain ASCII is drawn a run at a time, as the font's own
/// advance places it; any other character alone at its column, so one the
/// font doesn't have, or draws at another width, keeps to the grid. A tab
/// is the space to its stop, and a control character is drawn as its
/// picture (U+2400…), so nothing in the line is invisible.
pub fn pieces(line: &str, tab_size: usize, columns: Range<usize>) -> (Vec<Piece>, usize) {
    let mut pieces: Vec<Piece> = Vec::new();
    // Whether the last piece is a run that plain text may go on.
    let mut running = false;
    let mut column = 0;
    for (at, grapheme) in line.grapheme_indices(true) {
        let next = advance(column, grapheme, tab_size);
        let seen = next > columns.start && column < columns.end;
        let source = at..at + grapheme.len();
        let plain = matches!(grapheme.as_bytes(), [b' '..=b'~']);
        match grapheme.chars().next() {
            _ if !seen || grapheme == "\t" => running = false,
            _ if plain && running => {
                let piece = pieces.last_mut().expect("a run");
                piece.text.push_str(grapheme);
                piece.source.end = source.end;
            }
            Some(control @ ('\0'..='\x1f' | '\x7f')) if grapheme.len() == 1 => {
                running = false;
                pieces.push(Piece {
                    text: picture(control).to_string(),
                    column,
                    source,
                });
            }
            _ => {
                running = plain;
                pieces.push(Piece {
                    text: grapheme.to_owned(),
                    column,
                    source,
                });
            }
        }
        column = next;
    }
    (pieces, column)
}

/// `line` as it is drawn, as text: each piece at its column, with spaces
/// for tabs.
pub fn drawn_text(line: &str, tab_size: usize) -> String {
    let mut text = String::with_capacity(line.len());
    let mut column = 0;
    for piece in pieces(line, tab_size, 0..usize::MAX).0 {
        text.extend(std::iter::repeat_n(' ', piece.column - column));
        text.push_str(&piece.text);
        column = piece.column + line[piece.source].graphemes(true).map(cells).sum::<usize>();
    }
    text
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
