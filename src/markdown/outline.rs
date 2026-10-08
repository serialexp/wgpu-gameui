//! A document's shape for [`Markdown::estimate_height`]: its blocks in order,
//! each text's words by their lengths, laid flat in one buffer when the
//! document is parsed.
//!
//! A transcript estimates every message it holds whenever the width changes.
//! Walking each document's tree of blocks for that reaches into a separate
//! allocation for every block, item and cell, which costs more than the
//! arithmetic; an outline is one allocation, read front to back.
//!
//! The buffer is `u16` units. Each entry starts with a tag: its kind in the
//! low byte, a text's [`Face`] in the high one. Then, by kind:
//! - a [`TEXT`] has its words, as a count (two units) and the words;
//! - a [`QUOTE`] or an [`ITEM`] has its blocks' length in units (two), then
//!   the blocks;
//! - a [`LIST`] has its items' length in units (two), then the [`ITEM`]s;
//! - a [`RULE`] has nothing;
//! - a [`TABLE`] has its columns and its rows (two units each), [`COLUMN`]
//!   units of [`ColumnChars`] for each column, then words as a [`TEXT`] has
//!   them for each cell of the header and of each row.
//!
//! [`Markdown::estimate_height`]: super::Markdown::estimate_height

use super::MarkdownMetrics;
use super::layout::{CODE_LINE_HEIGHT, SCROLLBAR_GAP, between, column_shares};
use super::parse::{Block, Table};
use crate::text::LINE_HEIGHT_RATIO;

/// What the estimate takes a sans character's advance to be, in ems: a
/// little over the average.
const SANS_ADVANCE: f32 = 0.55;
/// A mono character's advance, in ems.
const MONO_ADVANCE: f32 = 0.6;
/// A table header's characters: mono, letter-spaced.
const CAPS_ADVANCE: f32 = 0.75;

/// Among a text's words, a line break; no word is this short.
const BREAK: u16 = 0;

const TEXT: u16 = 1;
const QUOTE: u16 = 2;
const LIST: u16 = 3;
const ITEM: u16 = 4;
const RULE: u16 = 5;
const TABLE: u16 = 6;
/// Units a column's [`ColumnChars`] take.
const COLUMN: usize = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Outline {
    units: Box<[u16]>,
}

/// How a text is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Face {
    Body,
    Heading(u8),
    Code,
}

impl Face {
    /// The face as a tag's high byte: a heading by its level.
    fn byte(self) -> u8 {
        match self {
            Face::Body => 0,
            Face::Heading(level) => level.clamp(1, 6),
            Face::Code => u8::MAX,
        }
    }

    fn of(byte: u8) -> Self {
        match byte {
            0 => Face::Body,
            u8::MAX => Face::Code,
            level => Face::Heading(level),
        }
    }
}

/// A table column's longest line and longest word, in characters, in its
/// header and in its body apart, since the two are set differently: what the
/// estimate shares the width by without measuring. Kept as [`COLUMN`] units,
/// each held at `u16::MAX`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ColumnChars {
    head_line: u32,
    head_word: u32,
    line: u32,
    word: u32,
}

impl Outline {
    pub fn of(blocks: &[Block]) -> Self {
        let mut writer = Writer::default();
        writer.blocks(blocks);
        Self {
            units: writer.units.into_boxed_slice(),
        }
    }

    /// A rough height at `width`, as [`Lay::blocks`] lays the document out.
    ///
    /// [`Lay::blocks`]: super::layout
    pub fn estimate(&self, metrics: &MarkdownMetrics, width: f32) -> f32 {
        Estimate { m: metrics }.blocks(&self.units, width)
    }

    /// The outline's entries, each container's contents in brackets after
    /// it, and a text's or a cell's word count after a colon.
    #[cfg(test)]
    pub fn describe(&self) -> String {
        fn entries(units: &[u16], out: &mut Vec<String>) {
            let mut read = Reader::new(units);
            while let Some(tag) = read.next() {
                let inner = |units: &[u16], name: &str| {
                    let mut inner = Vec::new();
                    entries(units, &mut inner);
                    format!("{name}[{}]", inner.join(" "))
                };
                out.push(match tag & 0xFF {
                    TEXT => {
                        let name = match Face::of((tag >> 8) as u8) {
                            Face::Body => "text".to_owned(),
                            Face::Heading(level) => format!("h{level}"),
                            Face::Code => "code".to_owned(),
                        };
                        format!("{name}:{}", read.words().len())
                    }
                    QUOTE => inner(read.contents(), "quote"),
                    LIST => inner(read.contents(), "list"),
                    ITEM => inner(read.contents(), "item"),
                    RULE => "rule".to_owned(),
                    TABLE => {
                        let (columns, rows) = (read.u32() as usize, read.u32() as usize);
                        read.take(columns * COLUMN);
                        let mut parts = vec!["column".to_owned(); columns];
                        for _ in 0..columns * (rows + 1) {
                            parts.push(format!("cell:{}", read.words().len()));
                        }
                        format!("table {columns}x{rows}[{}]", parts.join(" "))
                    }
                    other => format!("unknown {other}"),
                });
            }
        }
        let mut out = Vec::new();
        entries(&self.units, &mut out);
        out.join(" ")
    }
}

/// Builds an outline's units.
#[derive(Default)]
struct Writer {
    units: Vec<u16>,
}

impl Writer {
    fn blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(inline) => self.text(Face::Body, &inline.text),
                Block::Heading { level, text } => self.text(Face::Heading(*level), &text.text),
                Block::Code(code) => self.text(Face::Code, code),
                Block::Quote(blocks) => {
                    let at = self.open(QUOTE);
                    self.blocks(blocks);
                    self.close(at);
                }
                Block::List { items, .. } => {
                    let list = self.open(LIST);
                    for item in items {
                        let at = self.open(ITEM);
                        self.blocks(&item.blocks);
                        self.close(at);
                    }
                    self.close(list);
                }
                Block::Rule => self.units.push(RULE),
                Block::Table(table) => self.table(table),
            }
        }
    }

    fn text(&mut self, face: Face, text: &str) {
        self.units.push(TEXT | u16::from(face.byte()) << 8);
        self.words(text);
    }

    fn table(&mut self, table: &Table) {
        let columns = std::iter::once(&table.head)
            .chain(&table.rows)
            .map(Vec::len)
            .fold(table.align.len(), usize::max);
        self.units.push(TABLE);
        self.push_u32(columns);
        self.push_u32(table.rows.len());
        let first = self.units.len();
        self.units.resize(first + columns * COLUMN, 0);
        for (in_head, row) in
            std::iter::once((true, &table.head)).chain(table.rows.iter().map(|row| (false, row)))
        {
            for column in 0..columns {
                let words = self.words(row.get(column).map_or("", |cell| cell.text.as_str()));
                let at = first + column * COLUMN;
                let mut chars = ColumnChars::read(&self.units[at..]);
                chars.add(in_head, &self.units[words]);
                chars.write(&mut self.units[at..at + COLUMN]);
            }
        }
    }

    /// Count `text`'s words on: their count, then the words. Returns where
    /// the words are.
    fn words(&mut self, text: &str) -> std::ops::Range<usize> {
        let at = self.units.len();
        self.push_u32(0);
        count_words(text, &mut self.units);
        self.set_u32(at, self.units.len() - at - 2);
        at + 2..self.units.len()
    }

    /// Start a container tagged `tag`. Returns where its length goes, for
    /// [`close`](Self::close).
    fn open(&mut self, tag: u16) -> usize {
        self.units.push(tag);
        let at = self.units.len();
        self.push_u32(0);
        at
    }

    /// End the container whose length goes at `at`: what was added since is
    /// its contents.
    fn close(&mut self, at: usize) {
        self.set_u32(at, self.units.len() - at - 2);
    }

    fn push_u32(&mut self, value: usize) {
        self.units.extend([0, 0]);
        self.set_u32(self.units.len() - 2, value);
    }

    /// Two units at `at`, the low half first.
    fn set_u32(&mut self, at: usize, value: usize) {
        let value = value as u32;
        self.units[at] = value as u16;
        self.units[at + 1] = (value >> 16) as u16;
    }
}

/// Reads an outline's units front to back.
struct Reader<'a> {
    units: &'a [u16],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(units: &'a [u16]) -> Self {
        Self { units, at: 0 }
    }

    fn next(&mut self) -> Option<u16> {
        let unit = *self.units.get(self.at)?;
        self.at += 1;
        Some(unit)
    }

    fn u32(&mut self) -> u32 {
        let [low, high] = [self.units[self.at], self.units[self.at + 1]];
        self.at += 2;
        u32::from(low) | u32::from(high) << 16
    }

    fn take(&mut self, len: usize) -> &'a [u16] {
        let taken = &self.units[self.at..self.at + len];
        self.at += len;
        taken
    }

    /// A text's or a cell's words.
    fn words(&mut self) -> &'a [u16] {
        let count = self.u32() as usize;
        self.take(count)
    }

    /// A container's contents.
    fn contents(&mut self) -> &'a [u16] {
        let len = self.u32() as usize;
        self.take(len)
    }
}

impl ColumnChars {
    fn read(units: &[u16]) -> Self {
        Self {
            head_line: u32::from(units[0]),
            head_word: u32::from(units[1]),
            line: u32::from(units[2]),
            word: u32::from(units[3]),
        }
    }

    fn write(self, units: &mut [u16]) {
        let fields = [self.head_line, self.head_word, self.line, self.word];
        for (unit, field) in units.iter_mut().zip(fields) {
            *unit = u16::try_from(field).unwrap_or(u16::MAX);
        }
    }

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
}

impl Estimate<'_> {
    /// Lines `words` take in `width` at `advance` pixels a character.
    fn lines(&self, words: &[u16], width: f32, advance: f32) -> f32 {
        lines(words, (width / advance).floor() as usize) as f32
    }

    /// The height of the blocks in `units`, at `width`.
    fn blocks(&self, units: &[u16], width: f32) -> f32 {
        let m = self.m;
        let width = width.max(1.0);
        let line = m.size * m.line_height;
        let mut read = Reader::new(units);
        let mut height = 0.0;
        while let Some(tag) = read.next() {
            if read.at > 1 {
                height += m.block_gap;
            }
            height += match tag & 0xFF {
                TEXT => {
                    let words = read.words();
                    match Face::of((tag >> 8) as u8) {
                        Face::Body => self.lines(words, width, m.size * SANS_ADVANCE) * line,
                        Face::Heading(level) => {
                            let size = m.size * m.heading_scale[usize::from(level - 1)];
                            self.lines(words, width, size * SANS_ADVANCE) * size * LINE_HEIGHT_RATIO
                        }
                        Face::Code => {
                            let inner = width - m.code_pad[0] * 2.0;
                            self.lines(words, inner, m.code_size * MONO_ADVANCE)
                                * m.code_size
                                * CODE_LINE_HEIGHT
                                + m.code_pad[1] * 2.0
                        }
                    }
                }
                QUOTE => self.blocks(read.contents(), width - m.quote_indent),
                LIST => {
                    let mut items = Reader::new(read.contents());
                    let (mut height, mut count) = (0.0, 0u32);
                    while items.next().is_some() {
                        height += self.blocks(items.contents(), width - m.indent).max(line);
                        count += 1;
                    }
                    height + count.saturating_sub(1) as f32 * m.item_gap
                }
                RULE => m.size,
                TABLE => {
                    let (columns, rows) = (read.u32() as usize, read.u32() as usize);
                    let chars = read.take(columns * COLUMN);
                    self.table(&mut read, chars, rows, width)
                }
                // An item is only ever read by its list.
                _ => 0.0,
            };
        }
        height
    }

    /// A table's height at `width`, from its columns' `chars` and its cells,
    /// which `read` is at: the columns shared out as the layout shares them,
    /// from character counts rather than measured widths, and each row as
    /// tall as its cell that wraps most.
    fn table(&self, read: &mut Reader<'_>, chars: &[u16], rows: usize, width: f32) -> f32 {
        let m = self.m;
        let [pad_x, pad_y] = m.cell_pad;
        let (head_advance, advance) = (m.head_size * CAPS_ADVANCE, m.size * SANS_ADVANCE);
        let columns = || {
            chars
                .as_chunks::<COLUMN>()
                .0
                .iter()
                .map(|c| ColumnChars::read(c))
        };
        let narrowest = |c: ColumnChars| {
            (c.head_word as f32 * head_advance).max(c.word as f32 * advance) + pad_x * 2.0
        };
        let widest = |c: ColumnChars| {
            (c.head_line as f32 * head_advance).max(c.line as f32 * advance) + pad_x * 2.0
        };
        let least: f32 = columns().map(narrowest).sum();
        let most: f32 = columns().map(widest).sum();
        let share = column_shares(least, most, width);
        let mut row_lines = |advance: f32| {
            let mut most = 1.0f32;
            for c in columns() {
                let width = between(narrowest(c), widest(c), share);
                most = most.max(self.lines(read.words(), width - pad_x * 2.0, advance));
            }
            most
        };
        let head = row_lines(head_advance) * m.head_size * LINE_HEIGHT_RATIO + pad_y * 2.0;
        let line = m.size * m.line_height;
        // A table without columns has no cells to read.
        let rows = if chars.is_empty() { 0 } else { rows };
        let body: f32 = (0..rows)
            .map(|_| row_lines(advance) * line + pad_y * 2.0)
            .sum();
        let scrollbar = if least > width + 0.5 {
            SCROLLBAR_GAP + m.scrollbar_height
        } else {
            0.0
        };
        head + body + scrollbar
    }
}
