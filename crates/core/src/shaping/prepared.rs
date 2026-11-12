//! Text shaped once and laid out at any width (`docs/design/prepared-text.md`).
//!
//! The shared cache keeps finished layouts, keyed by everything a layout
//! depends on, width included, and drops the cosmic-text buffer each was
//! shaped in: a block measured at one width and drawn at another is shaped
//! twice. A [`PreparedText`] keeps the buffer, whose paragraphs cosmic-text
//! keeps shaped across a change of width, alignment or wrapping, and lays it
//! out again for each; a [`PreparedLayout`] carries one of those layouts on
//! the [`TextBlock`] it was made for, to where the block is drawn.

use std::sync::{Arc, OnceLock};

use cosmic_text::Buffer;

use super::{
    BufferSource, LayoutKey, LayoutSpec, ShapedLayout, SharedFontSystem, ellipsis, extract,
    faces_hash, set_buffer, set_layout, shape_cut_layout,
};
use crate::text::{
    CaretPos, TextBlock, TextContent, TextDirection, VisualGlyph, family_hash, style_disc,
};

/// The font system a text was shaped by, and its generation then: shaping
/// is only valid while both are the same.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FontStamp {
    system: u64,
    generation: u64,
}

impl FontStamp {
    fn of(shared: &SharedFontSystem) -> Self {
        Self {
            system: shared.id,
            generation: shared.generation,
        }
    }
}

/// The half of a [`LayoutSpec`] shaping depends on. The rest (width,
/// alignment, wrapping, ellipsis, line height) only lays it out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ShapingKey {
    font_size: u32,
    letter_spacing: u32,
    family: u64,
    weight: u16,
    style: u8,
    faces: u64,
    direction: TextDirection,
    vertical: bool,
}

impl ShapingKey {
    fn of(spec: &LayoutSpec<'_>) -> Self {
        Self {
            font_size: spec.font_size.to_bits(),
            letter_spacing: spec.letter_spacing.to_bits(),
            family: family_hash(spec.font),
            weight: spec.weight.0,
            style: style_disc(spec.style),
            faces: faces_hash(spec.shaped_faces()),
            direction: spec.direction,
            vertical: spec.vertical,
        }
    }
}

/// A text shaped once, to be laid out at any width without shaping it
/// again: measure a block at the width it is offered, then draw it at the
/// width it was given, for one shape where the shared cache would make two.
///
/// Made by [`TextMeasurer::prepare`](crate::TextMeasurer::prepare) from a
/// block, with the block's content as [`DrawList::text`](crate::DrawList::text)
/// records it (spans joined). [`measure`](Self::measure) and
/// [`layout`](Self::layout) take a block too: when what its shaping depends
/// on differs from what was prepared (its content, font, size, weight,
/// style, letter spacing, faces, direction or vertical setting), or the font
/// system is another or its fonts have changed, the text is prepared again,
/// shaping it, so the result is always the block's.
///
/// An ellipsized line that has to be cut is shaped afresh as its shown
/// bytes and a '…', as the shared cache does, so the glyphs at the cut are
/// exact; one that fits is laid out from the kept shaping.
///
/// It keeps its last two layouts, the measured one and the drawn one, so
/// asking again for either lays nothing out.
pub struct PreparedText {
    buffer: Buffer,
    source: BufferSource,
    content: TextContent,
    shaping: ShapingKey,
    fonts: FontStamp,
    /// The last two layouts made, the latest first.
    kept: [Option<PreparedLayout>; 2],
}

impl std::fmt::Debug for PreparedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedText")
            .field("content", &self.content)
            .field("kept", &self.kept.iter().flatten().count())
            .finish_non_exhaustive()
    }
}

/// The spec a block is measured with: unwrapped when it is ellipsized
/// horizontally, as [`TextMeasurer::measure_block`](crate::TextMeasurer::measure_block)
/// measures it.
fn measured_spec(block: &TextBlock) -> LayoutSpec<'_> {
    let spec = LayoutSpec::of_block(block);
    if block.ellipsize && !block.vertical {
        spec.natural()
    } else {
        spec
    }
}

/// `block`'s content as it is drawn: its spans joined when it has any.
fn drawn_content(block: &TextBlock) -> TextContent {
    match block.drawn_content() {
        std::borrow::Cow::Borrowed(_) => block.content.clone(),
        std::borrow::Cow::Owned(joined) => joined.into(),
    }
}

fn same_content(a: &TextContent, b: &str) -> bool {
    std::ptr::eq(a.as_str(), b) || a.as_str() == b
}

impl PreparedText {
    /// Shape `block` and lay it out as it is measured.
    pub(crate) fn new(shared: &mut SharedFontSystem, block: &TextBlock) -> Self {
        Self::shape(shared, &measured_spec(block), drawn_content(block))
    }

    /// Shape `content` under `spec`'s shaping half, laid out under all of it.
    fn shape(shared: &mut SharedFontSystem, spec: &LayoutSpec<'_>, content: TextContent) -> Self {
        let (mut buffer, source) = set_buffer(spec, &content, None);
        set_layout(&mut buffer, spec);
        let before = crate::text::shaping_passes();
        crate::text::shape_buffer(&mut buffer, &mut shared.font_system);
        shared.layouts.shaped += crate::text::shaping_passes() - before;
        let fonts = FontStamp::of(shared);
        let first = PreparedLayout::new(
            extract(&buffer, spec, &source),
            spec.key(),
            content.clone(),
            fonts,
        );
        Self {
            buffer,
            source,
            content,
            shaping: ShapingKey::of(spec),
            fonts,
            kept: [Some(first), None],
        }
    }

    /// `block`'s size as [`TextMeasurer::measure_block`](crate::TextMeasurer::measure_block)
    /// gives it: unwrapped when it is ellipsized horizontally, else at its
    /// `max_width`. Lays the kept shaping out, shaping nothing unless the
    /// block's shaping differs from what was prepared.
    pub fn measure(&mut self, measurer: &mut crate::TextMeasurer, block: &TextBlock) -> (f32, f32) {
        let content = block.drawn_content();
        if content.is_empty() {
            return (0.0, block.line_height);
        }
        let mut shared = measurer.shared();
        self.layout_at(&mut shared, &measured_spec(block), &content)
            .layout()
            .size
    }

    /// The layout `block` is drawn with, to attach to it with
    /// [`TextBlock::with_prepared`]. Lays the kept shaping out, shaping
    /// nothing unless the block's shaping differs from what was prepared or
    /// an ellipsis cuts it.
    pub fn layout(
        &mut self,
        measurer: &mut crate::TextMeasurer,
        block: &TextBlock,
    ) -> PreparedLayout {
        let content = block.drawn_content();
        let mut shared = measurer.shared();
        let spec = LayoutSpec::of_block(block);
        match spec.cut_width() {
            Some(max_width) => self.ellipsized(&mut shared, &spec, &content, max_width),
            None => self.layout_at(&mut shared, &spec, &content),
        }
    }

    /// Prepare again if `spec`'s shaping half, `content` or the fonts differ
    /// from what was shaped.
    fn refresh(&mut self, shared: &mut SharedFontSystem, spec: &LayoutSpec<'_>, content: &str) {
        let stale = self.fonts != FontStamp::of(shared)
            || self.shaping != ShapingKey::of(spec)
            || !same_content(&self.content, content);
        if stale {
            let content = if same_content(&self.content, content) {
                self.content.clone()
            } else {
                TextContent::from(content)
            };
            *self = Self::shape(shared, spec, content);
        }
    }

    /// The layout kept under `key`, made the latest.
    fn kept(&mut self, key: &LayoutKey) -> Option<PreparedLayout> {
        let at = self
            .kept
            .iter()
            .position(|kept| kept.as_ref().is_some_and(|kept| kept.0.key == *key))?;
        self.kept.swap(0, at);
        self.kept[0].clone()
    }

    fn keep(&mut self, layout: PreparedLayout) -> PreparedLayout {
        self.kept[1] = self.kept[0].replace(layout.clone());
        layout
    }

    /// `content` laid out under `spec` from the kept shaping, uncut.
    fn layout_at(
        &mut self,
        shared: &mut SharedFontSystem,
        spec: &LayoutSpec<'_>,
        content: &str,
    ) -> PreparedLayout {
        self.refresh(shared, spec, content);
        let key = spec.key();
        if let Some(kept) = self.kept(&key) {
            return kept;
        }
        set_layout(&mut self.buffer, spec);
        crate::text::lay_out_buffer(&mut self.buffer, &mut shared.font_system);
        shared.layouts.laid_out += 1;
        let layout = PreparedLayout::new(
            extract(&self.buffer, spec, &self.source),
            key,
            self.content.clone(),
            self.fonts,
        );
        self.keep(layout)
    }

    /// An ellipsized line within `max_width`: laid out from the kept shaping
    /// when it fits, else cut where the shared cache would cut it, read from
    /// the unwrapped layout, and the cut shaped as the cache shapes it.
    fn ellipsized(
        &mut self,
        shared: &mut SharedFontSystem,
        spec: &LayoutSpec<'_>,
        content: &str,
        max_width: f32,
    ) -> PreparedLayout {
        if content.is_empty() || !max_width.is_finite() || max_width <= 0.0 {
            return self.layout_at(shared, spec, content);
        }
        let natural = self.layout_at(shared, &spec.natural(), content);
        if natural.layout().size.0 <= max_width {
            return self.layout_at(shared, spec, content);
        }
        let key = spec.key();
        if let Some(kept) = self.kept(&key) {
            return kept;
        }
        // The ellipsis is in the block's face, not one of its ranges', and
        // its width is the cache's, as the cache's own cut reads it.
        let mark = LayoutSpec {
            faces: &[],
            ..spec.natural()
        };
        let budget = max_width - shared.layout(&mark, "…").size.0;
        let cut = if budget <= 0.0 {
            0
        } else {
            ellipsis::cut(natural.layout(), content, budget)
        };
        let before = crate::text::shaping_passes();
        let layout = shape_cut_layout(&mut shared.font_system, spec, content, Some(cut));
        shared.layouts.shaped += crate::text::shaping_passes() - before;
        let layout = PreparedLayout::new(layout, key, self.content.clone(), self.fonts);
        self.keep(layout)
    }
}

/// One layout of a [`PreparedText`], carried on the [`TextBlock`] it was
/// made for ([`TextBlock::with_prepared`]) to where the block is drawn, so
/// drawing it shapes nothing. It is used only where it is exactly what would
/// be looked up: the same spec, content and fonts. A block changed after it
/// was attached, or drawn under a scale, falls back to the shared cache.
///
/// Cheap to clone: a reference count.
#[derive(Clone)]
pub struct PreparedLayout(Arc<Made>);

struct Made {
    layout: ShapedLayout,
    key: LayoutKey,
    content: TextContent,
    fonts: FontStamp,
    /// Where carets stand in it, and its glyphs in visual order, worked out
    /// the first time they're asked for, as the cache keeps them.
    carets: OnceLock<Arc<[CaretPos]>>,
    visual: OnceLock<Arc<[VisualGlyph]>>,
}

impl std::fmt::Debug for PreparedLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedLayout")
            .field("content", &self.0.content)
            .field("size", &self.0.layout.size)
            .finish_non_exhaustive()
    }
}

impl PreparedLayout {
    fn new(layout: ShapedLayout, key: LayoutKey, content: TextContent, fonts: FontStamp) -> Self {
        Self(Arc::new(Made {
            layout,
            key,
            content,
            fonts,
            carets: OnceLock::new(),
            visual: OnceLock::new(),
        }))
    }

    /// Its width and height, as measuring the block it was made for gives.
    pub fn size(&self) -> (f32, f32) {
        self.0.layout.size
    }

    pub(crate) fn layout(&self) -> &ShapedLayout {
        &self.0.layout
    }

    /// Whether it is the layout of `content` under `spec` with `shared`'s
    /// fonts as they are now.
    pub(crate) fn is_for(
        &self,
        spec: &LayoutSpec<'_>,
        content: &str,
        shared: &SharedFontSystem,
    ) -> bool {
        self.0.fonts == FontStamp::of(shared)
            && self.0.key == spec.key()
            && same_content(&self.0.content, content)
    }

    /// Where carets stand in it, worked out once.
    pub(crate) fn carets(&self) -> Arc<[CaretPos]> {
        Arc::clone(
            self.0
                .carets
                .get_or_init(|| crate::text_carets::caret_positions(&self.0.layout).into()),
        )
    }

    /// Its glyphs in visual order, worked out once.
    pub(crate) fn visual(&self) -> Arc<[VisualGlyph]> {
        Arc::clone(
            self.0
                .visual
                .get_or_init(|| crate::text_carets::visual_glyphs(&self.0.layout).into()),
        )
    }
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;
