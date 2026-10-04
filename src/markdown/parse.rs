//! Markdown source to [`Block`]s, through pulldown-cmark's events.

use std::ops::Range;

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::TextAlign;

/// Inline styles a run of text carries, as bits.
pub(crate) mod flags {
    pub const STRONG: u8 = 1;
    pub const EMPHASIS: u8 = 1 << 1;
    pub const CODE: u8 = 1 << 2;
    pub const STRIKE: u8 = 1 << 3;
    pub const LINK: u8 = 1 << 4;
}

/// Text with its inline styling: runs of [`flags`] over its bytes, and the
/// links in it. Runs are sorted, don't overlap, and leave plain text out.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Inline {
    pub text: String,
    pub runs: Vec<(Range<usize>, u8)>,
    pub links: Vec<(Range<usize>, String)>,
}

impl Inline {
    /// Add `text` in the styles `style`, joining the run before it when that
    /// is in the same styles.
    fn push(&mut self, text: &str, style: u8) {
        let start = self.text.len();
        self.text.push_str(text);
        if style == 0 || text.is_empty() {
            return;
        }
        let end = self.text.len();
        match self.runs.last_mut() {
            Some((run, last)) if *last == style && run.end == start => run.end = end,
            _ => self.runs.push((start..end, style)),
        }
    }

    /// Without the whitespace a soft break or the source left at its ends.
    fn trimmed(mut self) -> Self {
        let end = self.text.trim_end().len();
        self.text.truncate(end);
        for (run, _) in &mut self.runs {
            run.end = run.end.min(end);
        }
        for (link, _) in &mut self.links {
            link.end = link.end.min(end);
        }
        self.runs.retain(|(run, _)| run.start < run.end);
        self.links.retain(|(link, _)| link.start < link.end);
        self
    }
}

/// One block of a document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block {
    Paragraph(Inline),
    Heading {
        level: u8,
        text: Inline,
    },
    /// A fenced or indented code block, or raw HTML, shown as it is.
    Code(String),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Rule,
    Table(Table),
}

/// A list item: its blocks, and its box when it is a task.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Table {
    pub align: Vec<TextAlign>,
    pub head: Vec<Inline>,
    pub rows: Vec<Vec<Inline>>,
}

/// A container being filled while its events arrive.
enum Frame {
    /// The document, a quote, or a list item: where blocks go.
    Blocks {
        blocks: Vec<Block>,
        kind: BlocksKind,
    },
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Table {
        table: Table,
        row: Vec<Inline>,
    },
}

enum BlocksKind {
    Document,
    Quote,
    Item { task: Option<bool> },
}

/// What the inline text being collected becomes.
enum InlineKind {
    Paragraph,
    Heading(u8),
    Cell,
}

/// How many of each inline style are open; they can nest.
#[derive(Default)]
struct Open {
    emphasis: u32,
    strong: u32,
    strike: u32,
    link: u32,
}

/// The parse in progress.
struct Parse {
    /// Open containers, innermost last; the document is the first.
    frames: Vec<Frame>,
    /// Inline text being collected, and what it becomes. A tight list item's
    /// text comes with no paragraph around it, so text with none open starts
    /// one, which the next block boundary ends.
    inline: Option<(Inline, InlineKind)>,
    /// A code block or HTML block being collected.
    code: Option<String>,
    /// How many of each style are open.
    open: Open,
    /// Where each open link starts, and its address.
    links: Vec<(usize, String)>,
}

/// Parse `source` as CommonMark with the GFM tables, strikethrough and task
/// lists.
pub(crate) fn parse(source: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut parse = Parse {
        frames: vec![Frame::Blocks {
            blocks: Vec::new(),
            kind: BlocksKind::Document,
        }],
        inline: None,
        code: None,
        open: Open::default(),
        links: Vec::new(),
    };
    for event in Parser::new_ext(source, options) {
        parse.event(event);
    }
    parse.end_inline();
    // Close anything left open, innermost first; well-formed input leaves
    // only the document.
    while parse.frames.len() > 1 {
        parse.close_frame();
    }
    match parse.frames.pop() {
        Some(Frame::Blocks { blocks, .. }) => blocks,
        _ => Vec::new(),
    }
}

impl Parse {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.code {
                Some(code) => code.push_str(&text),
                None => self.text(&text, self.style()),
            },
            Event::Code(text) => self.text(&text, self.style() | flags::CODE),
            Event::Html(text) => match &mut self.code {
                Some(code) => code.push_str(&text),
                None => self.text(&text, self.style()),
            },
            Event::InlineHtml(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                self.text(&text, self.style());
            }
            Event::FootnoteReference(name) => self.text(&format!("[^{name}]"), self.style()),
            Event::SoftBreak => self.text(" ", self.style()),
            Event::HardBreak => self.text("\n", self.style()),
            Event::Rule => {
                self.end_inline();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Frame::Blocks {
                    kind: BlocksKind::Item { task },
                    ..
                }) = self.frames.last_mut()
                {
                    *task = Some(done);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.begin_inline(InlineKind::Paragraph),
            Tag::Heading { level, .. } => self.begin_inline(InlineKind::Heading(heading(level))),
            Tag::TableCell => self.begin_inline(InlineKind::Cell),
            Tag::BlockQuote(_) => self.open_frame(Frame::Blocks {
                blocks: Vec::new(),
                kind: BlocksKind::Quote,
            }),
            Tag::List(start) => self.open_frame(Frame::List {
                start,
                items: Vec::new(),
            }),
            Tag::Item => self.open_frame(Frame::Blocks {
                blocks: Vec::new(),
                kind: BlocksKind::Item { task: None },
            }),
            Tag::Table(align) => self.open_frame(Frame::Table {
                table: Table {
                    align: align.into_iter().map(text_align).collect(),
                    head: Vec::new(),
                    rows: Vec::new(),
                },
                row: Vec::new(),
            }),
            // A fenced block's info string isn't shown.
            Tag::CodeBlock(_) | Tag::HtmlBlock => {
                self.end_inline();
                self.code = Some(String::new());
            }
            Tag::Emphasis => self.open.emphasis += 1,
            Tag::Strong => self.open.strong += 1,
            Tag::Strikethrough => self.open.strike += 1,
            Tag::Link { dest_url, .. } => {
                let at = self.inline_mut().text.len();
                self.links.push((at, dest_url.into_string()));
                self.open.link += 1;
            }
            // An image shows as its alt text, which arrives as text.
            Tag::Image { .. }
            | Tag::TableHead
            | Tag::TableRow
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.end_inline(),
            TagEnd::TableCell => {
                if let Some((inline, _)) = self.inline.take()
                    && let Some(Frame::Table { row, .. }) = self.frames.last_mut()
                {
                    row.push(inline.trimmed());
                }
            }
            TagEnd::TableHead => {
                if let Some(Frame::Table { table, row }) = self.frames.last_mut() {
                    table.head = std::mem::take(row);
                }
            }
            TagEnd::TableRow => {
                if let Some(Frame::Table { table, row }) = self.frames.last_mut() {
                    table.rows.push(std::mem::take(row));
                }
            }
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::Table => {
                self.close_frame();
            }
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                if let Some(mut code) = self.code.take() {
                    let end = code.trim_end_matches('\n').len();
                    code.truncate(end);
                    self.push_block(Block::Code(code));
                }
            }
            TagEnd::Emphasis => self.open.emphasis = self.open.emphasis.saturating_sub(1),
            TagEnd::Strong => self.open.strong = self.open.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.open.strike = self.open.strike.saturating_sub(1),
            TagEnd::Link => {
                self.open.link = self.open.link.saturating_sub(1);
                if let Some((start, url)) = self.links.pop()
                    && let Some((inline, _)) = &mut self.inline
                {
                    let end = inline.text.len();
                    if start < end {
                        inline.links.push((start..end, url));
                    }
                }
            }
            TagEnd::Image
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    /// The styles open now, as [`flags`].
    fn style(&self) -> u8 {
        let mut style = 0;
        for (open, bit) in [
            (self.open.emphasis, flags::EMPHASIS),
            (self.open.strong, flags::STRONG),
            (self.open.strike, flags::STRIKE),
            (self.open.link, flags::LINK),
        ] {
            if open > 0 {
                style |= bit;
            }
        }
        style
    }

    fn text(&mut self, text: &str, style: u8) {
        self.inline_mut().push(text, style);
    }

    /// The inline text being collected, starting a paragraph if none is.
    fn inline_mut(&mut self) -> &mut Inline {
        &mut self
            .inline
            .get_or_insert_with(|| (Inline::default(), InlineKind::Paragraph))
            .0
    }

    fn begin_inline(&mut self, kind: InlineKind) {
        self.end_inline();
        self.inline = Some((Inline::default(), kind));
    }

    /// End the inline text being collected, as the block it was to become.
    fn end_inline(&mut self) {
        let Some((inline, kind)) = self.inline.take() else {
            return;
        };
        let inline = inline.trimmed();
        match kind {
            InlineKind::Paragraph if inline.text.is_empty() => {}
            InlineKind::Paragraph => self.push_block(Block::Paragraph(inline)),
            InlineKind::Heading(level) => self.push_block(Block::Heading {
                level,
                text: inline,
            }),
            // A cell ends at its own end tag; one left open has nowhere to go.
            InlineKind::Cell => {}
        }
    }

    fn open_frame(&mut self, frame: Frame) {
        self.end_inline();
        self.frames.push(frame);
    }

    /// Close the innermost container, adding it to the one around it.
    fn close_frame(&mut self) {
        self.end_inline();
        if self.frames.len() < 2 {
            return;
        }
        let Some(frame) = self.frames.pop() else {
            return;
        };
        match frame {
            Frame::Blocks {
                blocks,
                kind: BlocksKind::Item { task },
            } => {
                if let Some(Frame::List { items, .. }) = self.frames.last_mut() {
                    items.push(Item { task, blocks });
                }
            }
            Frame::Blocks {
                blocks,
                kind: BlocksKind::Quote,
            } => self.push_block(Block::Quote(blocks)),
            Frame::Blocks {
                kind: BlocksKind::Document,
                ..
            } => {}
            Frame::List { start, items } => self.push_block(Block::List { start, items }),
            Frame::Table { table, .. } => self.push_block(Block::Table(table)),
        }
    }

    /// Add `block` to the innermost container that holds blocks.
    fn push_block(&mut self, block: Block) {
        for frame in self.frames.iter_mut().rev() {
            if let Frame::Blocks { blocks, .. } = frame {
                blocks.push(block);
                return;
            }
        }
    }
}

fn heading(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn text_align(align: Alignment) -> TextAlign {
    match align {
        Alignment::None | Alignment::Left => TextAlign::Left,
        Alignment::Center => TextAlign::Center,
        Alignment::Right => TextAlign::Right,
    }
}
