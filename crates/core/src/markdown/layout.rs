//! Laying a document out to a width, and painting the result.
//!
//! A layout is a flat list of draw operations in reading order, each with
//! the rect it covers, so painting is one pass that skips what the clip
//! hides, and selectable text gets its keys in the order it reads.

use std::ops::Range;

use cosmic_text::{Style, Weight};

use super::MarkdownStyle;
use super::parse::{Block, Inline, Item, Table, flags};
use crate::layout::Rect;
use crate::shaping::{LayoutSpec, ShapedLayout};
use crate::text::LINE_HEIGHT_RATIO;
use crate::text_select::highlight_rects;
use crate::widgets::DrawList;
use crate::{
    FaceRange, FontHandle, TextAlign, TextBlock, TextKey, TextStyleRange, Underline, WrapMode,
};

/// A document laid out at one width: what [`Markdown::layout`] returns.
///
/// [`Markdown::layout`]: super::Markdown::layout
pub struct MarkdownLayout {
    width: f32,
    style: MarkdownStyle,
    height: f32,
    ops: Vec<Placed>,
    tables: Vec<ScrollTable>,
    links: Vec<LinkBox>,
    urls: Vec<String>,
}

/// A table that is wider than the document and scrolls sideways.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollTable {
    /// The part of the table on show, relative to the document's origin.
    pub rect: Rect,
    /// The table's whole width.
    pub content_width: f32,
}

impl ScrollTable {
    /// The furthest it scrolls.
    pub fn max_scroll(&self) -> f32 {
        (self.content_width - self.rect.width).max(0.0)
    }
}

/// One draw operation, and where it is.
struct Placed {
    op: Op,
    /// What it covers, relative to the document's origin (or, in a table, to
    /// the table's unscrolled origin), for skipping what the clip hides.
    rect: Rect,
    /// The scrolling table it belongs to: drawn clipped to the table, and
    /// moved by its offset.
    table: Option<usize>,
}

enum Op {
    Text(TextBlock),
    Fill { color: [f32; 4], radius: f32 },
    Outline { color: [f32; 4], radius: f32 },
    Gradient { top: [f32; 4], bottom: [f32; 4] },
}

/// Where part of a link's text is drawn: one per line it is on.
struct LinkBox {
    rect: Rect,
    table: Option<usize>,
    /// Its address, in [`MarkdownLayout::urls`].
    url: usize,
}

impl MarkdownLayout {
    pub(super) fn is_for(&self, style: &MarkdownStyle, width: f32) -> bool {
        self.width.to_bits() == width.to_bits() && self.style == *style
    }

    /// The height it takes.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// Its tables that scroll sideways, in order; index the offsets handed
    /// to [`paint`](Self::paint) and [`link_at`](Self::link_at) by these.
    /// A table that fits isn't here.
    pub fn tables(&self) -> &[ScrollTable] {
        &self.tables
    }

    /// The scrolling table at `(x, y)`, relative to the document's origin.
    pub fn table_at(&self, x: f32, y: f32) -> Option<usize> {
        self.tables
            .iter()
            .position(|table| table.rect.contains(x, y))
    }

    /// Table `table`'s offset in `scroll`, kept within how far it scrolls.
    fn offset(&self, scroll: &[f32], table: usize) -> f32 {
        let max = self.tables[table].max_scroll();
        scroll.get(table).copied().unwrap_or(0.0).clamp(0.0, max)
    }

    /// Draw it with its origin at `(x, y)`. `scroll` holds each scrolling
    /// table's offset, by [`tables`](Self::tables) index; a missing one is
    /// `0`. With a `key`, its text is selectable, the blocks taking the keys
    /// from `key` on in reading order. Returns how many keys it took.
    ///
    /// What lies outside the draw list's clip isn't drawn.
    pub fn paint(
        &self,
        list: &mut DrawList,
        x: f32,
        y: f32,
        scroll: &[f32],
        key: Option<TextKey>,
    ) -> u64 {
        list.push_transform();
        list.translate(x, y);
        let mut group: Option<usize> = None;
        let mut taken = 0;
        for placed in &self.ops {
            if placed.table != group {
                if group.is_some() {
                    list.pop_transform();
                    list.pop_clip();
                }
                if let Some(table) = placed.table {
                    list.push_clip(self.tables[table].rect);
                    list.push_transform();
                    list.translate(-self.offset(scroll, table), 0.0);
                }
                group = placed.table;
            }
            let rect = placed.rect;
            let Op::Text(block) = &placed.op else {
                if !hidden(list, rect) {
                    paint_shape(list, &placed.op, rect);
                }
                continue;
            };
            // A hidden block still takes its key, so the keys after it
            // don't change as it scrolls in and out of view.
            let key = key.map(|key| {
                taken += 1;
                TextKey::new(key.scope, key.order + taken - 1)
            });
            if hidden(list, rect) {
                continue;
            }
            let mut block = block.clone();
            if let Some(key) = key {
                block = block.selectable(key);
            }
            list.text(block);
        }
        if group.is_some() {
            list.pop_transform();
            list.pop_clip();
        }
        for (index, table) in self.tables.iter().enumerate() {
            self.paint_scrollbar(list, table, self.offset(scroll, index));
        }
        list.pop_transform();
        taken
    }

    /// A thin thumb under a scrolling table, showing which part is on show.
    fn paint_scrollbar(&self, list: &mut DrawList, table: &ScrollTable, offset: f32) {
        let view = table.rect.width;
        let height = self.style.scrollbar_height;
        let thumb = (view * view / table.content_width)
            .max(height * 8.0)
            .min(view);
        let travel = view - thumb;
        let x = table.rect.x + travel * offset / table.max_scroll();
        let rect = Rect::new(
            x,
            table.rect.y + table.rect.height + SCROLLBAR_GAP,
            thumb,
            height,
        );
        if !hidden(list, rect) {
            list.rounded_rect(rect, height / 2.0, self.style.scrollbar);
        }
    }

    /// The address of the link at `(x, y)`, relative to the document's
    /// origin, with the tables scrolled by `scroll` as they were painted.
    pub fn link_at(&self, x: f32, y: f32, scroll: &[f32]) -> Option<&str> {
        let link = self.links.iter().find(|link| match link.table {
            Some(table) => {
                self.tables[table].rect.contains(x, y)
                    && link.rect.contains(x + self.offset(scroll, table), y)
            }
            None => link.rect.contains(x, y),
        })?;
        self.urls.get(link.url).map(String::as_str)
    }
}

/// Space between a scrolling table and its scrollbar.
pub(super) const SCROLLBAR_GAP: f32 = 2.0;
/// A code block's line height, as a multiple of its size.
pub(super) const CODE_LINE_HEIGHT: f32 = 1.45;

/// Whether `rect`, under the list's transform, lies wholly outside its clip.
fn hidden(list: &DrawList, rect: Rect) -> bool {
    let Some(clip) = list.current_clip() else {
        return false;
    };
    let world = list.current_transform().transform_rect_aabb(rect);
    world.intersection(clip).is_none()
}

fn paint_shape(list: &mut DrawList, op: &Op, rect: Rect) {
    match op {
        Op::Fill { color, radius } if *radius > 0.0 => list.rounded_rect(rect, *radius, *color),
        Op::Fill { color, .. } => list.quad(rect.x, rect.y, rect.width, rect.height, *color),
        Op::Outline { color, radius } => list.rounded_rect_outline(rect, *radius, 1.0, *color),
        Op::Gradient { top, bottom } => list.vertical_gradient(rect, *top, *bottom),
        Op::Text(_) => {}
    }
}

/// How a run of inline text is set before its own styles apply.
#[derive(Clone)]
struct Set {
    font: Option<FontHandle>,
    size: f32,
    line_height: f32,
    /// Letter spacing, in pixels.
    tracking: f32,
    weight: Weight,
    ink: [f32; 4],
}

/// A layout being made.
struct Lay<'a> {
    list: &'a mut DrawList,
    style: &'a MarkdownStyle,
    ops: Vec<Placed>,
    tables: Vec<ScrollTable>,
    links: Vec<LinkBox>,
    urls: Vec<String>,
    /// The scrolling table being laid out, which new ops belong to.
    table: Option<usize>,
}

/// Lay `blocks` out at `width` in `style`.
pub(super) fn lay_out(
    blocks: &[Block],
    list: &mut DrawList,
    style: &MarkdownStyle,
    width: f32,
) -> MarkdownLayout {
    let width = width.max(1.0);
    let mut lay = Lay {
        list,
        style,
        ops: Vec::new(),
        tables: Vec::new(),
        links: Vec::new(),
        urls: Vec::new(),
        table: None,
    };
    let body = lay.body();
    let height = lay.blocks(blocks, 0.0, 0.0, width, &body, 0);
    MarkdownLayout {
        width,
        style: style.clone(),
        height,
        ops: lay.ops,
        tables: lay.tables,
        links: lay.links,
        urls: lay.urls,
    }
}

impl Lay<'_> {
    fn body(&self) -> Set {
        Set {
            font: self.style.font.clone(),
            size: self.style.size,
            line_height: self.style.size * self.style.line_height,
            tracking: 0.0,
            weight: Weight::NORMAL,
            ink: self.style.ink,
        }
    }

    fn push(&mut self, op: Op, rect: Rect) {
        self.ops.push(Placed {
            op,
            rect,
            table: self.table,
        });
    }

    /// Lay `blocks` out from `(x, y)` in `width`, a gap between each; their
    /// height. `depth` is how deep in lists they are.
    fn blocks(
        &mut self,
        blocks: &[Block],
        x: f32,
        y: f32,
        width: f32,
        set: &Set,
        depth: usize,
    ) -> f32 {
        let mut at = y;
        for (index, block) in blocks.iter().enumerate() {
            if index > 0 {
                at += self.style.block_gap;
            }
            at += self.block(block, x, at, width, set, depth);
        }
        at - y
    }

    fn block(&mut self, block: &Block, x: f32, y: f32, width: f32, set: &Set, depth: usize) -> f32 {
        match block {
            Block::Paragraph(inline) => self.inline(inline, x, y, width, set, TextAlign::Start),
            Block::Heading { level, text } => {
                let scale = self.style.heading_scale[usize::from((*level).clamp(1, 6) - 1)];
                let size = self.style.size * scale;
                let heading = Set {
                    size,
                    line_height: size * LINE_HEIGHT_RATIO,
                    weight: Weight::BOLD,
                    ink: self.style.heading_ink,
                    ..set.clone()
                };
                self.inline(text, x, y, width, &heading, TextAlign::Start)
            }
            Block::Code(code) => self.code(code, x, y, width),
            Block::Quote(blocks) => {
                let quoted = Set {
                    ink: self.style.muted_ink,
                    ..set.clone()
                };
                let indent = self.style.quote_indent;
                let height = self.blocks(
                    blocks,
                    x + indent,
                    y,
                    (width - indent).max(1.0),
                    &quoted,
                    depth,
                );
                let bar = Rect::new(x, y, 2.0, height);
                self.push(
                    Op::Fill {
                        color: self.style.quote_bar,
                        radius: 0.0,
                    },
                    bar,
                );
                height
            }
            Block::List { start, items } => self.list_items(*start, items, x, y, width, set, depth),
            Block::Rule => {
                let height = self.style.size;
                let rule = Rect::new(x, y + (height / 2.0).round(), width, 1.0);
                self.push(
                    Op::Fill {
                        color: self.style.rule,
                        radius: 0.0,
                    },
                    rule,
                );
                height
            }
            Block::Table(table) => self.table(table, x, y, width, set),
        }
    }

    /// `inline` as one block of text from `(x, y)` in `width`; its height.
    fn inline(
        &mut self,
        inline: &Inline,
        x: f32,
        y: f32,
        width: f32,
        set: &Set,
        align: TextAlign,
    ) -> f32 {
        let block = self.text_block(inline, set, width).with_align(align);
        let block = TextBlock { x, y, ..block };
        let decorations = self.decorate(&block, inline);
        let (w, h) = decorations.size;
        self.push(Op::Text(block), Rect::new(x, y, w.max(width), h));
        for strike in decorations.strikes {
            let line = Rect::new(
                x + strike.x,
                y + strike.y + (strike.height * 0.55).round(),
                strike.width,
                (set.size * 0.07).max(1.0),
            );
            self.push(
                Op::Fill {
                    color: self.style.muted_ink,
                    radius: 0.0,
                },
                line,
            );
        }
        for (rect, url) in decorations.links {
            let index = match self.urls.iter().position(|known| *known == url) {
                Some(index) => index,
                None => {
                    self.urls.push(url);
                    self.urls.len() - 1
                }
            };
            self.links.push(LinkBox {
                rect: Rect::new(x + rect.x, y + rect.y, rect.width, rect.height),
                table: self.table,
                url: index,
            });
        }
        h
    }

    /// `inline` set in `set`, wrapped at `width`, at the origin, with its
    /// styles as face and colour ranges.
    fn text_block(&self, inline: &Inline, set: &Set, width: f32) -> TextBlock {
        let style = self.style;
        let mut faces = Vec::new();
        let mut colours = Vec::new();
        for (range, bits) in &inline.runs {
            let code = bits & flags::CODE != 0;
            if bits & (flags::STRONG | flags::EMPHASIS | flags::CODE) != 0 {
                faces.push(FaceRange {
                    range: range.clone(),
                    font: if code {
                        style.mono.clone()
                    } else {
                        set.font.clone()
                    },
                    weight: if bits & flags::STRONG != 0 {
                        Weight::BOLD
                    } else {
                        set.weight
                    },
                    style: if bits & flags::EMPHASIS != 0 {
                        Style::Italic
                    } else {
                        Style::Normal
                    },
                });
            }
            let (color, underline) = if bits & flags::LINK != 0 {
                (Some(style.link_ink), Underline::Inherit)
            } else if code {
                (Some(style.code_ink), Underline::None)
            } else if bits & flags::STRIKE != 0 {
                (Some(style.muted_ink), Underline::None)
            } else {
                (None, Underline::None)
            };
            if color.is_some() {
                colours.push(TextStyleRange {
                    range: range.clone(),
                    color,
                    underline,
                });
            }
        }
        let mut block = TextBlock::new(inline.text.clone(), 0.0, 0.0)
            .with_size(set.size)
            .with_line_height(set.line_height)
            .with_max_width(width.max(1.0))
            .with_weight(set.weight)
            .with_color_f32(set.ink)
            .with_font_opt(set.font.clone())
            .with_face_ranges(faces)
            .with_style_ranges(colours);
        block.letter_spacing = set.tracking;
        block
    }

    /// What `block` (of `inline`) measures, and where its struck-through
    /// text and links are, relative to its origin.
    fn decorate(&mut self, block: &TextBlock, inline: &Inline) -> Decorations {
        let strikes: Vec<Range<usize>> = inline
            .runs
            .iter()
            .filter(|(_, bits)| bits & flags::STRIKE != 0)
            .map(|(range, _)| range.clone())
            .collect();
        self.shaped(block, |layout| Decorations {
            size: layout.size,
            strikes: strikes
                .into_iter()
                .flat_map(|range| highlight_rects(layout, range))
                .collect(),
            links: inline
                .links
                .iter()
                .flat_map(|(range, url)| {
                    highlight_rects(layout, range.clone())
                        .into_iter()
                        .map(|rect| (rect, url.clone()))
                })
                .collect(),
        })
    }

    /// Run `read` on `block`'s shaped layout.
    fn shaped<R>(&mut self, block: &TextBlock, read: impl FnOnce(&ShapedLayout) -> R) -> R {
        let handle = self.list.text_measurer.font_system_handle();
        let mut shared = handle.lock().expect("FontSystem poisoned");
        read(shared.layout(&LayoutSpec::of_block(block), &block.content))
    }

    /// The size `block` takes.
    fn size(&mut self, block: &TextBlock) -> (f32, f32) {
        if block.content.is_empty() {
            return (0.0, block.line_height);
        }
        self.shaped(block, |layout| layout.size)
    }

    fn code(&mut self, code: &str, x: f32, y: f32, width: f32) -> f32 {
        let style = self.style;
        let [pad_x, pad_y] = style.code_pad;
        let block = TextBlock::new(code, x + pad_x, y + pad_y)
            .with_size(style.code_size)
            .with_line_height(style.code_size * CODE_LINE_HEIGHT)
            .with_max_width((width - pad_x * 2.0).max(1.0))
            .with_color_f32(style.code_ink)
            .with_font_opt(style.mono.clone());
        let (_, text_h) = self.size(&block);
        let well = Rect::new(x, y, width, text_h + pad_y * 2.0);
        self.push(
            Op::Fill {
                color: style.well,
                radius: style.radius,
            },
            well,
        );
        self.push(
            Op::Text(block),
            Rect::new(x + pad_x, y + pad_y, width, text_h),
        );
        self.push(
            Op::Outline {
                color: style.edge,
                radius: style.radius,
            },
            well,
        );
        well.height
    }

    #[allow(clippy::too_many_arguments)]
    fn list_items(
        &mut self,
        start: Option<u64>,
        items: &[Item],
        x: f32,
        y: f32,
        width: f32,
        set: &Set,
        depth: usize,
    ) -> f32 {
        let style = self.style;
        // Each marker sits on its item's first line, right-aligned in a
        // gutter wide enough for the widest of them, a little apart from the
        // text: numbers past 9 widen it.
        let gap = (set.size * 0.45).round();
        let markers: Vec<TextBlock> = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let marker = match (item.task, start) {
                    (Some(true), _) => "☑".to_owned(),
                    (Some(false), _) => "☐".to_owned(),
                    (None, Some(first)) => format!("{}.", first + index as u64),
                    (None, None) => ["•", "◦", "▪"][depth % 3].to_owned(),
                };
                TextBlock::new(marker, x, y)
                    .with_size(set.size)
                    .with_line_height(set.line_height)
                    .with_wrap(WrapMode::None)
                    .with_align(TextAlign::Right)
                    .with_color_f32(style.muted_ink)
                    .with_font_opt(if start.is_some() {
                        style.mono.clone()
                    } else {
                        set.font.clone()
                    })
            })
            .collect();
        let widest = markers
            .iter()
            .map(|marker| self.size(&marker.clone().with_max_width(f32::MAX / 4.0)).0)
            .fold(0.0f32, f32::max);
        let indent = style.indent.max((widest + gap).ceil());
        let mut at = y;
        for (index, (item, marker)) in items.iter().zip(markers).enumerate() {
            if index > 0 {
                at += style.item_gap;
            }
            let marker = TextBlock { y: at, ..marker }.with_max_width(indent - gap);
            let marker_h = self.size(&marker).1;
            self.push(Op::Text(marker), Rect::new(x, at, indent, marker_h));
            let height = self.blocks(
                &item.blocks,
                x + indent,
                at,
                (width - indent).max(1.0),
                set,
                depth + 1,
            );
            at += height.max(marker_h);
        }
        at - y
    }

    fn table(&mut self, table: &Table, x: f32, y: f32, width: f32, set: &Set) -> f32 {
        let style = self.style;
        let columns = table
            .rows
            .iter()
            .map(Vec::len)
            .chain([table.head.len(), table.align.len()])
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return 0.0;
        }
        let [pad_x, _] = style.cell_pad;
        let head: Vec<Inline> = (0..columns)
            .map(|column| {
                let inline = table.head.get(column).cloned().unwrap_or_default();
                if style.head_caps {
                    // Capitals can change byte lengths, so the header's own
                    // styles don't carry over.
                    Inline {
                        text: inline.text.to_uppercase(),
                        ..Inline::default()
                    }
                } else {
                    inline
                }
            })
            .collect();
        let rows: Vec<Vec<Inline>> = table
            .rows
            .iter()
            .map(|row| {
                (0..columns)
                    .map(|column| row.get(column).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
        let head_set = Set {
            font: style.head_font.clone(),
            size: style.head_size,
            line_height: style.head_size * LINE_HEIGHT_RATIO,
            tracking: style.head_tracking,
            weight: Weight::NORMAL,
            ink: style.head_ink,
        };

        // Each column's narrowest (its widest word) and widest (its longest
        // line), padding included. Measured once, at no width.
        let mut narrowest = vec![0.0f32; columns];
        let mut widest = vec![0.0f32; columns];
        for (row, row_set) in
            std::iter::once((&head, &head_set)).chain(rows.iter().map(|row| (row, set)))
        {
            for (column, inline) in row.iter().enumerate() {
                let block = self.text_block(inline, row_set, 1.0);
                let line = self
                    .size(
                        &block
                            .clone()
                            .with_wrap(WrapMode::None)
                            .with_max_width(f32::MAX / 4.0),
                    )
                    .0;
                let word = self.size(&block.with_wrap(WrapMode::Word)).0;
                narrowest[column] = narrowest[column].max(word.min(line) + pad_x * 2.0);
                widest[column] = widest[column].max(line + pad_x * 2.0);
            }
        }
        let widths = column_widths(&narrowest, &widest, width);
        let content_width: f32 = widths.iter().sum();
        let view_width = content_width.min(width);
        let lefts: Vec<f32> = widths
            .iter()
            .scan(0.0, |left, w| {
                let at = *left;
                *left += w;
                Some(at)
            })
            .collect();

        let scrolls = content_width > width + 0.5;
        let outer = self.table;
        let index = self.tables.len();
        if scrolls {
            self.tables.push(ScrollTable {
                rect: Rect::new(x, y, view_width, 0.0),
                content_width,
            });
        }

        let well = self.ops.len();
        self.push(
            Op::Fill {
                color: style.well,
                radius: style.radius,
            },
            Rect::new(x, y, view_width, 0.0),
        );
        if scrolls {
            self.table = Some(index);
        }

        // The header strip, then each row, the cells of a row set side by
        // side and the row as tall as its tallest.
        let columns = Columns {
            lefts: &lefts,
            widths: &widths,
            align: &table.align,
        };
        let mut at = y;
        let head_h = self.row(&head, &head_set, x, at, &columns);
        at += head_h;
        for (number, row) in rows.iter().enumerate() {
            let first = self.ops.len();
            let row_h = self.row(row, set, x, at, &columns);
            // Lines between rows, and every other row lit faintly, under its
            // text.
            let mut under = vec![Placed {
                op: Op::Fill {
                    color: style.rule,
                    radius: 0.0,
                },
                rect: Rect::new(x, at, content_width, 1.0),
                table: self.table,
            }];
            if number % 2 == 1 {
                under.push(Placed {
                    op: Op::Fill {
                        color: style.zebra,
                        radius: 0.0,
                    },
                    rect: Rect::new(x, at, content_width, row_h),
                    table: self.table,
                });
            }
            self.ops.splice(first..first, under);
            at += row_h;
        }
        let height = at - y;
        self.table = outer;

        // Now the height is known: the well, the header strip under the
        // header text, and the edge around it all.
        self.ops[well].rect.height = height;
        let head_strip = Placed {
            op: Op::Gradient {
                top: style.head_fill[0],
                bottom: style.head_fill[1],
            },
            rect: Rect::new(x, y, content_width, head_h),
            table: if scrolls { Some(index) } else { outer },
        };
        self.ops.insert(well + 1, head_strip);
        self.push(
            Op::Outline {
                color: style.edge,
                radius: style.radius,
            },
            Rect::new(x, y, view_width, height),
        );
        if scrolls {
            self.tables[index].rect.height = height;
            height + SCROLLBAR_GAP + style.scrollbar_height
        } else {
            height
        }
    }

    /// One table row: `cells` set in `set`, side by side in `columns` from
    /// `x`, at `y`; its height, the tallest cell's.
    fn row(&mut self, cells: &[Inline], set: &Set, x: f32, y: f32, columns: &Columns<'_>) -> f32 {
        let [pad_x, pad_y] = self.style.cell_pad;
        let mut tallest = 0.0f32;
        for (column, inline) in cells.iter().enumerate() {
            let left = x + columns.lefts[column] + pad_x;
            let width = columns.widths[column] - pad_x * 2.0;
            let align = columns
                .align
                .get(column)
                .copied()
                .unwrap_or(TextAlign::Left);
            let height = self.inline(inline, left, y + pad_y, width, set, align);
            tallest = tallest.max(height);
        }
        tallest + pad_y * 2.0
    }
}

/// Where a table's columns are, and how their text is aligned.
struct Columns<'a> {
    /// Each column's left edge, from the table's.
    lefts: &'a [f32],
    widths: &'a [f32],
    align: &'a [TextAlign],
}

/// What [`Lay::decorate`] finds.
struct Decorations {
    size: (f32, f32),
    strikes: Vec<Rect>,
    links: Vec<(Rect, String)>,
}

/// Each column's width, from its narrowest and widest, in `width`.
///
/// Columns that all fit at their widest take that. Otherwise each gets its
/// narrowest plus a share of what is left in proportion to how much more it
/// would take, so a column of long text gives way before a column of short
/// numbers. When even the narrowest don't fit, the columns keep them and the
/// table is wider than `width`.
pub(super) fn column_widths(narrowest: &[f32], widest: &[f32], width: f32) -> Vec<f32> {
    let least: f32 = narrowest.iter().sum();
    let most: f32 = widest.iter().sum();
    let share = column_shares(least, most, width);
    narrowest
        .iter()
        .zip(widest)
        .map(|(&low, &high)| between(low, high, share))
        .collect()
}

/// `share` of the way from `low` to `high`: exactly one of them at 0 or 1.
pub(super) fn between(low: f32, high: f32, share: f32) -> f32 {
    low * (1.0 - share) + high * share
}

/// How far from its narrowest to its widest each column goes, as a fraction,
/// when the columns take `least` at their narrowest and `most` at their
/// widest: as [`column_widths`] shares `width` out.
pub(super) fn column_shares(least: f32, most: f32, width: f32) -> f32 {
    if most <= width {
        1.0
    } else if least >= width {
        0.0
    } else {
        (width - least) / (most - least)
    }
}
