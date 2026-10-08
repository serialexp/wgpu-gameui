//! A document's shape for [`Markdown::estimate_height`]: its blocks in order,
//! each text's words by their lengths, laid flat in two arrays when the
//! document is parsed.
//!
//! A transcript estimates every message it holds whenever the width changes,
//! each frame of dragging a window's edge. Walking each document's tree of
//! blocks for that reaches into a separate allocation for every block, item
//! and cell, which costs more than the arithmetic; an outline is read in
//! order instead.
//!
//! [`Markdown::estimate_height`]: super::Markdown::estimate_height

use super::MarkdownMetrics;
use super::layout::{CODE_LINE_HEIGHT, SCROLLBAR_GAP, between, column_shares};
use super::parse::{Block, Inline};
use crate::text::LINE_HEIGHT_RATIO;

/// What the estimate takes a sans character's advance to be, in ems: a
/// little over the average.
const SANS_ADVANCE: f32 = 0.55;
/// A mono character's advance, in ems.
const MONO_ADVANCE: f32 = 0.6;
/// A table header's characters: mono, letter-spaced.
const CAPS_ADVANCE: f32 = 0.75;

/// In `words`, a line break; no word is this short.
const BREAK: u16 = 0;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Outline {
    shapes: Vec<Shape>,
    /// Every text's words by their lengths in characters, [`BREAK`] between
    /// its lines; each [`Span`] is a stretch of these.
    words: Vec<u16>,
}

/// A stretch of [`Outline::words`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Span {
    start: u32,
    end: u32,
}

/// One entry of an outline. A container is followed by its contents, `len`
/// entries of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shape {
    Text {
        face: Face,
        words: Span,
    },
    /// Followed by its blocks.
    Quote {
        len: u32,
    },
    /// Followed by its [`Shape::Item`]s.
    List {
        len: u32,
    },
    /// Followed by its blocks.
    Item {
        len: u32,
    },
    Rule,
    /// Followed by `columns` [`Shape::Column`]s, then a [`Shape::Cell`] for
    /// each column of the header and of each of its `rows`.
    Table {
        columns: u32,
        rows: u32,
    },
    Column(ColumnChars),
    Cell(Span),
}

/// How a text is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Face {
    Body,
    Heading(u8),
    Code,
}

/// A table column's longest line and longest word, in characters, in its
/// header and in its body apart, since the two are set differently: what the
/// estimate shares the width by without measuring.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ColumnChars {
    head_line: u32,
    head_word: u32,
    line: u32,
    word: u32,
}

impl Outline {
    pub fn of(blocks: &[Block]) -> Self {
        let mut outline = Self::default();
        outline.blocks(blocks);
        outline
    }

    fn blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(inline) => self.text(Face::Body, &inline.text),
                Block::Heading { level, text } => self.text(Face::Heading(*level), &text.text),
                Block::Code(code) => self.text(Face::Code, code),
                Block::Quote(blocks) => {
                    let at = self.open(Shape::Quote { len: 0 });
                    self.blocks(blocks);
                    self.close(at);
                }
                Block::List { items, .. } => {
                    let list = self.open(Shape::List { len: 0 });
                    for item in items {
                        let at = self.open(Shape::Item { len: 0 });
                        self.blocks(&item.blocks);
                        self.close(at);
                    }
                    self.close(list);
                }
                Block::Rule => self.shapes.push(Shape::Rule),
                Block::Table(table) => {
                    let columns = std::iter::once(&table.head)
                        .chain(&table.rows)
                        .map(Vec::len)
                        .fold(table.align.len(), usize::max);
                    self.shapes.push(Shape::Table {
                        columns: columns as u32,
                        rows: table.rows.len() as u32,
                    });
                    let first = self.shapes.len();
                    self.shapes
                        .resize(first + columns, Shape::Column(ColumnChars::default()));
                    for (in_head, row) in std::iter::once((true, &table.head))
                        .chain(table.rows.iter().map(|row| (false, row)))
                    {
                        for column in 0..columns {
                            let cell = self.cell(row.get(column));
                            if let Shape::Column(chars) = &mut self.shapes[first + column] {
                                chars.add(in_head, &self.words[cell.range()]);
                            }
                            self.shapes.push(Shape::Cell(cell));
                        }
                    }
                }
            }
        }
    }

    fn text(&mut self, face: Face, text: &str) {
        let words = self.words(text);
        self.shapes.push(Shape::Text { face, words });
    }

    fn cell(&mut self, cell: Option<&Inline>) -> Span {
        self.words(cell.map_or("", |cell| cell.text.as_str()))
    }

    /// Count `text`'s words onto the end of [`Self::words`].
    fn words(&mut self, text: &str) -> Span {
        let start = self.words.len() as u32;
        count_words(text, &mut self.words);
        Span {
            start,
            end: self.words.len() as u32,
        }
    }

    /// Start container `shape`, at the index this returns.
    fn open(&mut self, shape: Shape) -> usize {
        self.shapes.push(shape);
        self.shapes.len() - 1
    }

    /// End the container at `at`: what was added since is its contents.
    fn close(&mut self, at: usize) {
        let contents = (self.shapes.len() - at - 1) as u32;
        if let Shape::Quote { len } | Shape::List { len } | Shape::Item { len } =
            &mut self.shapes[at]
        {
            *len = contents;
        }
    }

    /// A rough height at `width`, as [`Lay::blocks`] lays the document out.
    ///
    /// [`Lay::blocks`]: super::layout
    pub fn estimate(&self, metrics: &MarkdownMetrics, width: f32) -> f32 {
        Estimate {
            m: metrics,
            words: &self.words,
        }
        .blocks(&self.shapes, width)
    }

    #[cfg(test)]
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }
}

impl Span {
    fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl ColumnChars {
    /// Take in a cell of this column with `words`, in the header or not.
    fn add(&mut self, in_head: bool, words: &[u16]) {
        let (mut longest_line, mut longest_word, mut line) = (0, 0, 0);
        for &word in words {
            if word == BREAK {
                line = 0;
                continue;
            }
            let word = u32::from(word);
            line += if line == 0 { word } else { word + 1 };
            longest_line = longest_line.max(line);
            longest_word = longest_word.max(word);
        }
        let (most, widest) = if in_head {
            (&mut self.head_line, &mut self.head_word)
        } else {
            (&mut self.line, &mut self.word)
        };
        *most = (*most).max(longest_line);
        *widest = (*widest).max(longest_word);
    }
}

/// Count `text`'s words by their lengths in characters onto `words`,
/// [`BREAK`] between its lines.
pub(super) fn count_words(text: &str, words: &mut Vec<u16>) {
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            words.push(BREAK);
        }
        words.extend(
            line.split_whitespace()
                .map(|word| word.chars().count().min(usize::from(u16::MAX)) as u16),
        );
    }
}

/// Lines `words` take at `per_line` characters a line, wrapped whole as the
/// shaper wraps them and a word longer than a line broken: each of its own
/// lines at least one.
pub(super) fn lines(words: &[u16], per_line: usize) -> usize {
    let per_line = per_line.max(1);
    let (mut count, mut used) = (1, 0);
    for &word in words {
        if word == BREAK {
            count += 1;
            used = 0;
            continue;
        }
        let chars = usize::from(word);
        if used == 0 {
            used = chars;
        } else if used + 1 + chars <= per_line {
            used += 1 + chars;
        } else {
            count += 1;
            used = chars;
        }
        while used > per_line {
            count += 1;
            used -= per_line;
        }
    }
    count
}

struct Estimate<'a> {
    m: &'a MarkdownMetrics,
    words: &'a [u16],
}

impl Estimate<'_> {
    /// Lines `span`'s words take in `width` at `advance` pixels a character.
    fn lines(&self, span: Span, width: f32, advance: f32) -> f32 {
        lines(
            &self.words[span.range()],
            (width / advance).floor() as usize,
        ) as f32
    }

    /// The height of the blocks `shapes` lists, at `width`.
    fn blocks(&self, shapes: &[Shape], width: f32) -> f32 {
        let m = self.m;
        let width = width.max(1.0);
        let line = m.size * m.line_height;
        let (mut height, mut at) = (0.0, 0);
        while let Some(&shape) = shapes.get(at) {
            if at > 0 {
                height += m.block_gap;
            }
            at += 1;
            height += match shape {
                Shape::Text {
                    face: Face::Body,
                    words,
                } => self.lines(words, width, m.size * SANS_ADVANCE) * line,
                Shape::Text {
                    face: Face::Heading(level),
                    words,
                } => {
                    let size = m.size * m.heading_scale[usize::from(level.clamp(1, 6) - 1)];
                    self.lines(words, width, size * SANS_ADVANCE) * size * LINE_HEIGHT_RATIO
                }
                Shape::Text {
                    face: Face::Code,
                    words,
                } => {
                    let inner = width - m.code_pad[0] * 2.0;
                    self.lines(words, inner, m.code_size * MONO_ADVANCE)
                        * m.code_size
                        * CODE_LINE_HEIGHT
                        + m.code_pad[1] * 2.0
                }
                Shape::Quote { len } => {
                    let blocks = &shapes[at..at + len as usize];
                    at += len as usize;
                    self.blocks(blocks, width - m.quote_indent)
                }
                Shape::List { len } => {
                    let end = at + len as usize;
                    let (mut items, mut count) = (0.0, 0u32);
                    while let Some(&Shape::Item { len }) = shapes[..end].get(at) {
                        let blocks = &shapes[at + 1..at + 1 + len as usize];
                        at += 1 + len as usize;
                        items += self.blocks(blocks, width - m.indent).max(line);
                        count += 1;
                    }
                    at = end;
                    items + count.saturating_sub(1) as f32 * m.item_gap
                }
                Shape::Rule => m.size,
                Shape::Table { columns, rows } => {
                    let (columns, rows) = (columns as usize, rows as usize);
                    let end = at + columns + columns * (rows + 1);
                    let height = self.table(&shapes[at..end], columns, width);
                    at = end;
                    height
                }
                // Only ever inside the containers above.
                Shape::Item { .. } | Shape::Column(_) | Shape::Cell(_) => 0.0,
            };
        }
        height
    }

    /// A table's height at `width` from `shapes`, its `columns` columns and
    /// then its cells: the columns shared out as the layout shares them, from
    /// character counts rather than measured widths, and each row as tall as
    /// its cell that wraps most.
    fn table(&self, shapes: &[Shape], columns: usize, width: f32) -> f32 {
        let m = self.m;
        let [pad_x, pad_y] = m.cell_pad;
        let (head_advance, advance) = (m.head_size * CAPS_ADVANCE, m.size * SANS_ADVANCE);
        let (chars, cells) = shapes.split_at(columns);
        let chars = || {
            chars.iter().map(|shape| match shape {
                Shape::Column(chars) => *chars,
                _ => ColumnChars::default(),
            })
        };
        let narrowest = |c: ColumnChars| {
            (c.head_word as f32 * head_advance).max(c.word as f32 * advance) + pad_x * 2.0
        };
        let widest = |c: ColumnChars| {
            (c.head_line as f32 * head_advance).max(c.line as f32 * advance) + pad_x * 2.0
        };
        let least: f32 = chars().map(narrowest).sum();
        let most: f32 = chars().map(widest).sum();
        let share = column_shares(least, most, width);
        let row_lines = |row: &[Shape], advance: f32| {
            row.iter()
                .zip(chars())
                .map(|(cell, c)| match cell {
                    Shape::Cell(words) => {
                        let width = between(narrowest(c), widest(c), share);
                        self.lines(*words, width - pad_x * 2.0, advance)
                    }
                    _ => 1.0,
                })
                .fold(1.0, f32::max)
        };
        let mut rows = cells.chunks_exact(columns.max(1));
        let head = rows
            .next()
            .map_or(1.0, |head| row_lines(head, head_advance));
        let head = head * m.head_size * LINE_HEIGHT_RATIO + pad_y * 2.0;
        let line = m.size * m.line_height;
        let body: f32 = rows
            .map(|row| row_lines(row, advance) * line + pad_y * 2.0)
            .sum();
        let scrollbar = if least > width + 0.5 {
            SCROLLBAR_GAP + m.scrollbar_height
        } else {
            0.0
        };
        head + body + scrollbar
    }
}
