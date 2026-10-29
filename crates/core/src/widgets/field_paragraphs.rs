//! A multi-line field's text as paragraphs, each laid out on its own and
//! only when it is drawn or asked about, so a field's cost follows what it
//! shows rather than how long its value is
//! (`docs/design/paragraph-text-layout.md`).
//!
//! A line break never wraps across paragraphs and bidi resolves per
//! paragraph, so a paragraph laid out alone places its glyphs and carets as
//! the whole text laid out at once would. A paragraph longer than
//! [`PIECE_BYTES`] is laid out in pieces, each cut at a word break and
//! starting a line of its own, so no single layout is unbounded.
//!
//! Heights are kept in [`RowHeights`], one row per paragraph (or piece):
//! estimated from its length until it is laid out, then measured.

use std::ops::Range;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use super::DrawList;
use super::row_heights::{Row, RowHeights};
use crate::text::{
    CaretPos, TextBlock, WrapMode, byte_at_point, byte_under_point, caret_for_byte, on_line,
};

/// The most bytes laid out as one block. A longer paragraph is cut into
/// pieces of at most this many.
pub(super) const PIECE_BYTES: usize = 16 * 1024;

/// A guess at the average advance of a character, as a share of the font
/// size, for estimating how many lines a paragraph not yet laid out wraps to.
const AVERAGE_ADVANCE: f32 = 0.55;

/// How a field lays its text out: the parts of its drawn block that place
/// glyphs. Every caret, click, selection and scroll position is read from the
/// layout of a block built here, so it is the layout the text is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FieldText {
    pub font_size: f32,
    pub wrap: WrapMode,
    pub max_width: f32,
    pub direction: crate::TextDirection,
}

impl FieldText {
    /// `content` laid out as the field draws it, at (`x`, `y`).
    pub fn block(self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size)
            .with_wrap(self.wrap)
            .with_max_width(self.max_width)
            .with_direction(self.direction)
    }

    /// The height of one laid-out line (`TextBlock::with_size`'s).
    pub fn line_height(self) -> f32 {
        self.font_size * 1.25
    }

    /// A paragraph's height before it is laid out: its characters at an
    /// average advance, wrapped to the width.
    fn estimate(self, text: &str) -> Row {
        let lines = if self.wrap == WrapMode::None || self.max_width <= 0.0 {
            1.0
        } else {
            let width = text.chars().count() as f32 * self.font_size * AVERAGE_ADVANCE;
            (width / self.max_width).ceil().max(1.0)
        };
        Row::estimated(lines * self.line_height())
    }
}

/// Where a caret stands, in the field's own coordinates: `top` from the top
/// of the first paragraph, `x` from the text's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Caret {
    /// The paragraph (or piece) it is in.
    pub segment: usize,
    /// Its visual line within that paragraph.
    pub line: usize,
    pub byte: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
}

/// A paragraph that meets the band asked about: its index, top, and carets.
pub(super) struct Shown {
    pub segment: usize,
    pub top: f32,
    pub carets: Arc<[CaretPos]>,
}

/// What an edit did to the paragraphs: the first `front` kept their place,
/// and from `old_back` on (`new_back` now) they moved past the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Splice {
    front: usize,
    old_back: usize,
    new_back: usize,
    /// There were no paragraphs before: the first split of a text.
    first: bool,
}

impl Splice {
    /// Where paragraph `old` is now, and whether it is the same text: one
    /// the edit replaced maps to the first paragraph it put in their place.
    pub fn index(self, old: usize) -> (usize, bool) {
        if self.first {
            (0, false)
        } else if old < self.front {
            (old, true)
        } else if old >= self.old_back {
            (old - self.old_back + self.new_back, true)
        } else {
            (self.front, false)
        }
    }
}

/// A multi-line field's paragraphs, their heights, and their layouts on
/// demand. See the module docs.
#[derive(Default)]
pub(super) struct FieldParagraphs {
    /// The value they were split from.
    text: String,
    /// Each paragraph's (or piece's) text, line break left out, in order.
    segments: Vec<Range<usize>>,
    heights: RowHeights,
    /// What the heights were measured for.
    field: Option<FieldText>,
}

impl FieldParagraphs {
    /// Bring the paragraphs in step with `value` laid out as `field`. Only
    /// the paragraphs an edit touched are split again and estimated; the
    /// others keep their heights. A new width or font re-estimates them all.
    /// Returns what moved, when the text changed.
    pub fn sync(&mut self, value: &str, field: FieldText) -> Option<Splice> {
        if self.field.is_some_and(|old| old != field) {
            let text = &self.text;
            let segments = &self.segments;
            self.heights
                .reestimate(|i| field.estimate(&text[segments[i].clone()]));
        }
        self.field = Some(field);
        if !self.segments.is_empty() && self.text == value {
            return None;
        }
        let old = std::mem::take(&mut self.segments);
        let new = segments(value);
        let prefix = common_prefix(&self.text, value);
        let suffix =
            common_suffix(&self.text, value).min(self.text.len().min(value.len()) - prefix);
        let old_tail = self.text.len() - suffix;
        let new_tail = value.len() - suffix;
        let front = old
            .iter()
            .zip(&new)
            .take_while(|(a, b)| a == b && a.end < prefix)
            .count();
        let back = old[front..]
            .iter()
            .rev()
            .zip(new[front..].iter().rev())
            .take_while(|(a, b)| {
                a.start >= old_tail
                    && a.start - old_tail + new_tail == b.start
                    && a.len() == b.len()
            })
            .count();
        let splice = Splice {
            front,
            old_back: old.len() - back,
            new_back: new.len() - back,
            first: old.is_empty(),
        };
        self.heights.splice(
            front..splice.old_back,
            new[front..splice.new_back]
                .iter()
                .map(|range| field.estimate(&value[range.clone()])),
        );
        self.text.clear();
        self.text.push_str(value);
        self.segments = new;
        Some(splice)
    }

    /// How many paragraphs (and pieces) there are: at least one.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Paragraph `segment`'s text range in the value, line break left out.
    pub fn range(&self, segment: usize) -> Range<usize> {
        self.segments[segment].clone()
    }

    /// Paragraph `segment`'s text.
    pub fn text(&self, segment: usize) -> &str {
        &self.text[self.range(segment)]
    }

    /// Where paragraph `segment` starts, from the top of the first; `len()`
    /// gives the height of them all. Estimated paragraphs above count at
    /// their estimates.
    pub fn top(&self, segment: usize) -> f32 {
        self.heights.offset_of(segment) as f32
    }

    /// Paragraph `segment`'s height, measured or estimated.
    pub fn height(&self, segment: usize) -> f32 {
        self.heights.height_of(segment)
    }

    /// Whether paragraph `segment` has been laid out.
    #[cfg(test)]
    pub fn is_measured(&self, segment: usize) -> bool {
        self.heights.is_measured(segment)
    }

    /// The paragraph at height `y`, the first above them and the last below.
    pub fn segment_at(&self, y: f32) -> usize {
        if y < 0.0 {
            return 0;
        }
        self.heights
            .row_at(f64::from(y))
            .unwrap_or(self.segments.len() - 1)
    }

    /// The paragraph holding `byte`; a byte in a line break belongs to the
    /// paragraph it ends.
    fn segment_of(&self, byte: usize) -> usize {
        self.segments
            .partition_point(|range| range.start <= byte)
            .saturating_sub(1)
    }

    /// The paragraph a caret at `byte` is drawn in: where a long paragraph
    /// was cut into pieces, the byte at the cut ends the first piece, as the
    /// byte at a soft wrap ends its line.
    fn caret_segment(&self, byte: usize) -> usize {
        let segment = self.segment_of(byte);
        if segment > 0
            && self.segments[segment].start == byte
            && self.segments[segment - 1].end == byte
        {
            segment - 1
        } else {
            segment
        }
    }

    /// Paragraph `segment`'s carets, laid out as `field` draws it (through
    /// the shared layout cache), its height measured on the way.
    pub fn carets(
        &mut self,
        list: &mut DrawList,
        field: FieldText,
        segment: usize,
    ) -> Arc<[CaretPos]> {
        let range = self.range(segment);
        let carets = list.caret_layout(&field.block(&self.text[range], 0.0, 0.0));
        let height = carets
            .iter()
            .map(|caret| caret.line_top + caret.line_height)
            .fold(0.0, f32::max);
        if !self.heights.is_measured(segment) || self.heights.height_of(segment) != height {
            self.heights.set_measured(segment, height);
        }
        carets
    }

    /// The paragraphs meeting the band from `top` to `bottom`, each laid out
    /// (and so measured) top to bottom, so each one's top counts the
    /// measured heights of those drawn above it.
    pub fn shown(
        &mut self,
        list: &mut DrawList,
        field: FieldText,
        top: f32,
        bottom: f32,
    ) -> Vec<Shown> {
        let mut shown = Vec::new();
        let mut segment = self.segment_at(top);
        while segment < self.segments.len() {
            // Its top counts only the paragraphs above, measured by now.
            let segment_top = self.top(segment);
            if segment_top >= bottom && !shown.is_empty() {
                break;
            }
            let carets = self.carets(list, field, segment);
            shown.push(Shown {
                segment,
                top: segment_top,
                carets,
            });
            segment += 1;
        }
        shown
    }

    /// Where the caret at `byte` stands: [`caret_for_byte`] over its
    /// paragraph.
    pub fn caret(&mut self, list: &mut DrawList, field: FieldText, byte: usize) -> Caret {
        let segment = self.caret_segment(byte);
        let carets = self.carets(list, field, segment);
        let start = self.segments[segment].start;
        let local = caret_for_byte(&carets, byte.saturating_sub(start));
        Caret {
            segment,
            line: local.line,
            byte: start + local.byte,
            x: local.x,
            top: self.top(segment) + local.line_top,
            height: local.line_height,
        }
    }

    /// The caret nearest the point (`x`, `y`) in the field's coordinates:
    /// [`byte_at_point`] over the paragraph at `y`.
    pub fn byte_at(&mut self, list: &mut DrawList, field: FieldText, x: f32, y: f32) -> usize {
        let segment = self.segment_at(y);
        let carets = self.carets(list, field, segment);
        self.segments[segment].start + byte_at_point(&carets, x, y - self.top(segment))
    }

    /// The character under the point: [`byte_under_point`] over the
    /// paragraph at `y`.
    pub fn byte_under(&mut self, list: &mut DrawList, field: FieldText, x: f32, y: f32) -> usize {
        let segment = self.segment_at(y);
        let carets = self.carets(list, field, segment);
        self.segments[segment].start + byte_under_point(&carets, x, y - self.top(segment))
    }

    /// The caret nearest `desired_x` on the visual line `dir` steps from the
    /// one holding `byte` (`-1` up, `1` down), into the paragraph before or
    /// after at its edges; `byte` itself at the first or last line.
    pub fn adjacent(
        &mut self,
        list: &mut DrawList,
        field: FieldText,
        byte: usize,
        dir: i32,
        desired_x: f32,
    ) -> usize {
        let caret = self.caret(list, field, byte);
        let lines = self
            .carets(list, field, caret.segment)
            .last()
            .map_or(0, |p| p.line);
        let (segment, line) = match dir.signum() {
            -1 if caret.line > 0 => (caret.segment, Some(caret.line - 1)),
            -1 if caret.segment > 0 => (caret.segment - 1, None),
            1 if caret.line < lines => (caret.segment, Some(caret.line + 1)),
            1 if caret.segment + 1 < self.segments.len() => (caret.segment + 1, Some(0)),
            _ => return byte,
        };
        let carets = self.carets(list, field, segment);
        // `None`: the last line of the paragraph above.
        let line = line.unwrap_or_else(|| carets.last().map_or(0, |p| p.line));
        on_line(&carets, line, |p| p.line)
            .iter()
            .min_by(|a, b| (a.x - desired_x).abs().total_cmp(&(b.x - desired_x).abs()))
            .map_or(byte, |p| self.segments[segment].start + p.byte)
    }

    /// The first and last byte of the visual line holding the caret at
    /// `byte`: where line-relative Home and End go.
    pub fn line_bounds(
        &mut self,
        list: &mut DrawList,
        field: FieldText,
        byte: usize,
    ) -> (usize, usize) {
        let caret = self.caret(list, field, byte);
        let carets = self.carets(list, field, caret.segment);
        let start = self.segments[caret.segment].start;
        let line = on_line(&carets, caret.line, |p| p.line);
        let bytes = || line.iter().map(|p| start + p.byte);
        bytes().min().zip(bytes().max()).unwrap_or((byte, byte))
    }
}

/// `text`'s paragraphs as cosmic-text splits them (`text::paragraphs`), each
/// longer than [`PIECE_BYTES`] cut into pieces.
fn segments(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for paragraph in crate::text::paragraphs(text) {
        let mut start = paragraph.start;
        while paragraph.end - start > PIECE_BYTES {
            let end = start + piece_len(&text[start..paragraph.end]);
            out.push(start..end);
            start = end;
        }
        out.push(start..paragraph.end);
    }
    out
}

/// How much of `text`, longer than [`PIECE_BYTES`], the next piece takes:
/// up to the last word break that fits, or failing one in its second half,
/// the last whole grapheme that does.
fn piece_len(text: &str) -> usize {
    let mut limit = PIECE_BYTES;
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let window = &text[..limit];
    let word = window
        .split_word_bound_indices()
        .map(|(i, _)| i)
        .rfind(|&i| i >= PIECE_BYTES / 2);
    word.or_else(|| window.grapheme_indices(true).map(|(i, _)| i).next_back())
        .filter(|&i| i > 0)
        .unwrap_or(limit)
}

/// How many bytes `a` and `b` start with in common, on a character boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    while !a.is_char_boundary(n) {
        n -= 1;
    }
    n
}

/// How many bytes `a` and `b` end with in common, on a character boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = a
        .bytes()
        .rev()
        .zip(b.bytes().rev())
        .take_while(|(x, y)| x == y)
        .count();
    while !a.is_char_boundary(a.len() - n) {
        n -= 1;
    }
    n
}

#[cfg(test)]
#[path = "field_paragraphs_tests.rs"]
mod tests;
