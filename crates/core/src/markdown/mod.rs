//! Markdown documents: parsed once, laid out to a width, drawn with rich text.
//!
//! [`Markdown::parse`] reads CommonMark with the GFM tables, strikethrough and
//! task lists into blocks: paragraphs, headings, lists (nested, ordered, with
//! task boxes), code blocks, quotes, rules and tables. Inline text keeps its
//! styles as byte ranges, drawn through [`TextBlock::face_ranges`] (bold,
//! italic, code in the mono face) and [`TextBlock::style_ranges`] (code and
//! link colours, link underlines), so a paragraph is still one block of text
//! that wraps and selects as one.
//!
//! [`Markdown::layout`] lays the document out to a width with a
//! [`MarkdownStyle`], keeping the last layout so a caller that measures and
//! then draws at one width lays out once. The [`MarkdownLayout`] it returns
//! has the height, paints itself, and answers which link is under a point.
//!
//! A table's columns share the width: each gets at least its widest word and
//! at most its longest line, and its cells wrap. When even the widest words
//! don't fit, the table keeps them and scrolls sideways; the caller keeps
//! each table's offset ([`MarkdownLayout::tables`]) and hands them to
//! [`paint`](MarkdownLayout::paint).
//!
//! ```no_run
//! # use wgpu_gameui::{DrawList, Markdown, MarkdownStyle, StyleResolver};
//! # fn demo(list: &mut DrawList, style: &StyleResolver) {
//! let doc = Markdown::parse("| a | b |\n|---|--:|\n| one | 2 |");
//! let look = MarkdownStyle::forge(style);
//! let layout = doc.layout(list, &look, 480.0);
//! layout.paint(list, 10.0, 10.0, &[], None);
//! # }
//! ```
//!
//! [`TextBlock::face_ranges`]: crate::TextBlock::face_ranges
//! [`TextBlock::style_ranges`]: crate::TextBlock::style_ranges

mod layout;
mod outline;
mod parse;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};

pub use layout::{MarkdownLayout, ScrollTable};

use crate::widgets::DrawList;
use crate::{FontHandle, Ink, StyleKey, StyleResolver, TextSize, Tracking};

/// A parsed markdown document. Cheap to keep: its last layout is kept with
/// it, and laid out again only for another width or style.
pub struct Markdown {
    blocks: Vec<parse::Block>,
    /// The blocks' shape, flat, for [`estimate_height`](Self::estimate_height).
    outline: outline::Outline,
    laid_out: Mutex<Option<Arc<MarkdownLayout>>>,
}

impl Markdown {
    /// Parse `source` as CommonMark with the GFM tables, strikethrough and
    /// task lists. Anything it doesn't draw as such (raw HTML, footnote
    /// references, images) shows as its text.
    pub fn parse(source: &str) -> Self {
        let blocks = parse::parse(source);
        Self {
            outline: outline::Outline::of(&blocks),
            blocks,
            laid_out: Mutex::new(None),
        }
    }

    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The document laid out at `width` in `style`: the last layout when it
    /// was for these, a new one otherwise. Threads sharing a document don't
    /// wait on each other's layouts: the last one kept is only swapped, never
    /// held while laying out.
    pub fn layout(
        &self,
        list: &mut DrawList,
        style: &MarkdownStyle,
        width: f32,
    ) -> Arc<MarkdownLayout> {
        if let Some(layout) = self.kept()
            && layout.is_for(style, width)
        {
            return layout;
        }
        let layout = Arc::new(layout::lay_out(&self.blocks, list, style, width));
        *self.laid_out.lock().expect("markdown layout poisoned") = Some(Arc::clone(&layout));
        layout
    }

    fn kept(&self) -> Option<Arc<MarkdownLayout>> {
        self.laid_out
            .lock()
            .expect("markdown layout poisoned")
            .clone()
    }

    /// A height for the document at `width` without laying it out: lines of
    /// text at an average character width, table rows wrapped in columns
    /// shared by their longest cells, gaps between blocks. For placing it
    /// before it is measured, many documents a frame; it allocates nothing,
    /// and reads two arrays the parse filled in rather than the blocks.
    pub fn estimate_height(&self, metrics: &MarkdownMetrics, width: f32) -> f32 {
        self.outline.estimate(metrics, width)
    }
}

/// The sizes and spacing of a [`MarkdownStyle`] without its colours and
/// faces: all [`Markdown::estimate_height`] needs, and cheap to make, so a
/// caller estimating many documents need not build a style for each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkdownMetrics {
    /// [`MarkdownStyle::size`].
    pub size: f32,
    /// [`MarkdownStyle::line_height`].
    pub line_height: f32,
    /// [`MarkdownStyle::code_size`].
    pub code_size: f32,
    /// [`MarkdownStyle::heading_scale`].
    pub heading_scale: [f32; 6],
    /// [`MarkdownStyle::head_size`].
    pub head_size: f32,
    /// [`MarkdownStyle::block_gap`].
    pub block_gap: f32,
    /// [`MarkdownStyle::item_gap`].
    pub item_gap: f32,
    /// [`MarkdownStyle::indent`].
    pub indent: f32,
    /// [`MarkdownStyle::quote_indent`].
    pub quote_indent: f32,
    /// [`MarkdownStyle::code_pad`].
    pub code_pad: [f32; 2],
    /// [`MarkdownStyle::cell_pad`].
    pub cell_pad: [f32; 2],
    /// [`MarkdownStyle::scrollbar_height`].
    pub scrollbar_height: f32,
}

impl MarkdownMetrics {
    /// [`MarkdownStyle::forge`]'s sizes and spacing.
    pub fn forge(style: &StyleResolver<'_>) -> Self {
        let size = style.scalar(StyleKey::FontSize);
        Self {
            size,
            line_height: 1.55,
            code_size: style.scalar(StyleKey::TextSize(TextSize::Dense)),
            heading_scale: [1.45, 1.25, 1.1, 1.0, 1.0, 1.0],
            head_size: style.scalar(StyleKey::TextSize(TextSize::Caption)),
            block_gap: (size * 0.6).round(),
            item_gap: (size * 0.25).round(),
            indent: (size * 1.4).round(),
            quote_indent: (size * 0.9).round(),
            code_pad: [10.0, 7.0],
            cell_pad: [9.0, 5.0],
            scrollbar_height: 3.0,
        }
    }
}

/// How a [`Markdown`] document looks. [`forge`](Self::forge) gives Forge's
/// look from a theme; change any field after.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownStyle {
    /// Body face; `None` is the default sans-serif.
    pub font: Option<FontHandle>,
    /// Face for code; `None` is the default sans-serif.
    pub mono: Option<FontHandle>,
    /// Body text size, in pixels.
    pub size: f32,
    /// Body line height, as a multiple of `size`.
    pub line_height: f32,
    /// Code size, in pixels: code blocks, and inline code in body text.
    pub code_size: f32,
    /// Heading sizes, as multiples of `size`, `#` first.
    pub heading_scale: [f32; 6],
    /// Body text.
    pub ink: [f32; 4],
    /// Headings.
    pub heading_ink: [f32; 4],
    /// Quoted text, list markers and struck-through text.
    pub muted_ink: [f32; 4],
    /// Code, inline and in blocks.
    pub code_ink: [f32; 4],
    /// Links, which are also underlined.
    pub link_ink: [f32; 4],
    /// Behind a code block, and behind a table.
    pub well: [f32; 4],
    /// The edge around a code block or table.
    pub edge: [f32; 4],
    /// A rule, and the line between table rows.
    pub rule: [f32; 4],
    /// The bar beside a quote.
    pub quote_bar: [f32; 4],
    /// A table's header strip, top to bottom.
    pub head_fill: [[f32; 4]; 2],
    /// A table's header text.
    pub head_ink: [f32; 4],
    /// Header face; `None` is the default sans-serif.
    pub head_font: Option<FontHandle>,
    /// Header size, in pixels.
    pub head_size: f32,
    /// Header letter spacing, in pixels.
    pub head_tracking: f32,
    /// Whether headers are set in capitals.
    pub head_caps: bool,
    /// Every other table row, over the well.
    pub zebra: [f32; 4],
    /// A scrolling table's scrollbar thumb.
    pub scrollbar: [f32; 4],
    /// Space between blocks.
    pub block_gap: f32,
    /// Space between list items.
    pub item_gap: f32,
    /// How far a list's items sit in from its markers' left edge.
    pub indent: f32,
    /// How far a quote's text sits in from its bar.
    pub quote_indent: f32,
    /// Space inside a code block: horizontal, vertical.
    pub code_pad: [f32; 2],
    /// Space inside a table cell: horizontal, vertical.
    pub cell_pad: [f32; 2],
    /// Corner radius of wells.
    pub radius: f32,
    /// Height of a scrolling table's scrollbar, under it.
    pub scrollbar_height: f32,
}

impl MarkdownStyle {
    /// Forge's look under `style`: body text at the theme's size in the row
    /// ink, code in the mono face, tables in a sunken well with a header strip
    /// of mono capitals, as Forge's data table has.
    pub fn forge(style: &StyleResolver<'_>) -> Self {
        let theme = style.theme();
        let metrics = MarkdownMetrics::forge(style);
        let caption = metrics.head_size;
        Self {
            font: theme.font.clone(),
            mono: theme.mono_font.clone(),
            size: metrics.size,
            line_height: metrics.line_height,
            code_size: metrics.code_size,
            heading_scale: metrics.heading_scale,
            ink: style.ink(Ink::Row),
            heading_ink: style.ink(Ink::Title),
            muted_ink: style.ink(Ink::Muted),
            code_ink: style.ink(Ink::Value),
            link_ink: style.color(StyleKey::AccentGlyph),
            well: [0.0, 0.0, 0.0, 0.3],
            edge: style.color(StyleKey::EdgeHard),
            rule: [1.0, 1.0, 1.0, 0.08],
            quote_bar: [1.0, 1.0, 1.0, 0.14],
            head_fill: [[1.0, 1.0, 1.0, 0.09], [1.0, 1.0, 1.0, 0.025]],
            head_ink: style.ink(Ink::Body2),
            head_font: theme.mono_font.clone(),
            head_size: caption,
            head_tracking: caption * style.scalar(StyleKey::Tracking(Tracking::Caption)),
            head_caps: true,
            zebra: style.color(StyleKey::RowZebra),
            scrollbar: [1.0, 1.0, 1.0, 0.22],
            block_gap: metrics.block_gap,
            item_gap: metrics.item_gap,
            indent: metrics.indent,
            quote_indent: metrics.quote_indent,
            code_pad: metrics.code_pad,
            cell_pad: metrics.cell_pad,
            radius: 1.0,
            scrollbar_height: metrics.scrollbar_height,
        }
    }

    /// Its sizes and spacing, for [`Markdown::estimate_height`].
    pub fn metrics(&self) -> MarkdownMetrics {
        MarkdownMetrics {
            size: self.size,
            line_height: self.line_height,
            code_size: self.code_size,
            heading_scale: self.heading_scale,
            head_size: self.head_size,
            block_gap: self.block_gap,
            item_gap: self.item_gap,
            indent: self.indent,
            quote_indent: self.quote_indent,
            code_pad: self.code_pad,
            cell_pad: self.cell_pad,
            scrollbar_height: self.scrollbar_height,
        }
    }
}
