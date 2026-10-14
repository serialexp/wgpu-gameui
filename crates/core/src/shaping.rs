//! Text shaping shared by measuring and drawing.
//!
//! Measuring a [`TextBlock`] and drawing it need the same thing from
//! cosmic-text: the block's glyphs, laid out. [`SharedFontSystem`] keeps that
//! layout, so a block measured for layout and then drawn in the same frame is
//! shaped once, not twice. It sits next to the `FontSystem` behind the one
//! [`FontSystemHandle`](crate::FontSystemHandle), because a layout is only
//! valid for the fonts it was shaped with: changing the font database through
//! [`SharedFontSystem::db_mut`] drops them all.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap, fontdb,
};

use crate::text::{
    FaceRange, FontHandle, FontVMetrics, LINE_HEIGHT_RATIO, TextAlign, TextBlock, TextDirection,
    WrapMode, cosmic_align, direction_prefix, ellipsis_cut, face_runs, family_hash,
    letter_spacing_em, resolve_vmetrics, style_disc, vertical_stack_string,
};

/// Roughly how many bytes of layouts [`SharedFontSystem`] keeps before it
/// drops the least recently used, down to three quarters of it (so eviction
/// runs rarely rather than on every new layout). Over four hundred thousand
/// glyphs: many screens of dense text, measured and drawn, with room for
/// scrolling back.
const LAYOUT_BUDGET: usize = 16 << 20;

/// Bookkeeping bytes counted per layout on top of its text and glyphs.
const LAYOUT_OVERHEAD: usize = 96;

/// Most per-font vertical metrics kept before starting over. Each is one font
/// at one weight and style, so only an application that cycles through fonts
/// reaches it.
const VMETRICS_CACHE_CAP: usize = 256;

/// The font system that measuring and drawing share, with the text layouts
/// they share. Reached through a [`FontSystemHandle`](crate::FontSystemHandle).
/// A renderer holds its lock for the whole of a draw list's text, glyph
/// generation included (see `TextRenderer::build_vertices`).
pub struct SharedFontSystem {
    font_system: FontSystem,
    layouts: LayoutCache,
    /// Per-font vertical metrics for optical centring, keyed by
    /// `(family_hash, weight, style_disc)`. Sampled once per font (a one-glyph
    /// shaping pass + a ttf-parser metric read), then reused every frame.
    vmetrics: HashMap<(u64, u16, u8), FontVMetrics>,
}

impl SharedFontSystem {
    /// Wrap a cosmic-text `FontSystem`.
    pub fn new(font_system: FontSystem) -> Self {
        Self {
            font_system,
            layouts: LayoutCache::default(),
            vmetrics: HashMap::new(),
        }
    }

    /// The font system, for shaping. Change its fonts through
    /// [`db_mut`](Self::db_mut) instead, so no layout outlives the fonts it
    /// was shaped with.
    pub fn font_system(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// The font database, to look fonts up.
    pub fn db(&self) -> &fontdb::Database {
        self.font_system.db()
    }

    /// The font database, to load fonts or change the default families. Drops
    /// every kept layout and font metric, since any of them may now resolve to
    /// other faces.
    pub fn db_mut(&mut self) -> &mut fontdb::Database {
        self.clear_caches();
        self.font_system.db_mut()
    }

    /// Drop every kept layout and font metric, so each block is shaped again
    /// when next measured or drawn.
    pub fn clear_caches(&mut self) {
        self.layouts.clear();
        self.vmetrics.clear();
    }

    /// The optical vertical metrics of `font` at `weight` and `style`
    /// ([`TextMeasurer::vmetrics`](crate::TextMeasurer::vmetrics)).
    pub(crate) fn vmetrics(
        &mut self,
        font: Option<&FontHandle>,
        weight: Weight,
        style: Style,
    ) -> FontVMetrics {
        let key = (family_hash(font), weight.0, style_disc(style));
        if let Some(&metrics) = self.vmetrics.get(&key) {
            return metrics;
        }
        let metrics = resolve_vmetrics(
            &mut self.font_system,
            font.map(FontHandle::family),
            weight,
            style,
        );
        if self.vmetrics.len() >= VMETRICS_CACHE_CAP {
            self.vmetrics.clear();
        }
        self.vmetrics.insert(key, metrics);
        metrics
    }

    /// Keep about `bytes` of layouts (16 MiB by default), dropping the least
    /// recently used now if more are kept. `0` keeps only the layout just
    /// measured or drawn: for a font system that measures text once, such as
    /// one on a background thread working out heights, where keeping layouts
    /// would only cost memory.
    pub fn set_layout_budget(&mut self, bytes: usize) {
        self.layouts.set_budget(bytes);
    }

    /// How the kept layouts are doing: hits, shapes, and what is kept.
    pub fn layout_stats(&self) -> LayoutStats {
        LayoutStats {
            hits: self.layouts.hits,
            shaped: self.layouts.shaped,
            layouts: self.layouts.len,
            bytes: self.layouts.bytes,
        }
    }

    /// Every kept layout of `content`, whatever its spec.
    #[cfg(test)]
    pub(crate) fn kept_layouts<'s>(
        &'s self,
        content: &'s str,
    ) -> impl Iterator<Item = &'s ShapedLayout> + 's {
        self.layouts
            .map
            .values()
            .filter_map(move |inner| inner.get(content))
            .map(|entry| &entry.layout)
    }

    /// The layout of `content` under `spec`, shaped now if it isn't kept.
    pub(crate) fn layout(&mut self, spec: &LayoutSpec<'_>, content: &str) -> &ShapedLayout {
        self.layout_and_fonts(spec, content).0
    }

    /// [`layout`](Self::layout), with the font system still at hand for the
    /// glyph outlines the layout refers to.
    pub(crate) fn layout_and_fonts(
        &mut self,
        spec: &LayoutSpec<'_>,
        content: &str,
    ) -> (&ShapedLayout, &mut FontSystem) {
        let layout = self
            .layouts
            .get_or_shape(spec, content, &mut self.font_system);
        (layout, &mut self.font_system)
    }
}

/// A snapshot of [`SharedFontSystem`]'s layout cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    /// Layouts served without shaping, since the font system was made.
    pub hits: u64,
    /// Layouts shaped through cosmic-text, since the font system was made.
    pub shaped: u64,
    /// Layouts kept now.
    pub layouts: usize,
    /// Roughly how many bytes the kept layouts take.
    pub bytes: usize,
}

/// Everything that decides how a string is laid out, apart from the string.
/// Built from a [`TextBlock`] by [`of_block`](Self::of_block), or from a
/// measuring call's arguments.
#[derive(Clone, Copy)]
pub(crate) struct LayoutSpec<'a> {
    pub font_size: f32,
    pub line_height: f32,
    /// The width lines wrap at; `None` never wraps. Vertical text places its
    /// column within it.
    pub max_width: Option<f32>,
    pub letter_spacing: f32,
    pub font: Option<&'a FontHandle>,
    pub weight: Weight,
    pub style: Style,
    /// Byte ranges in other faces ([`TextBlock::face_ranges`]).
    pub faces: &'a [FaceRange],
    pub wrap: WrapMode,
    pub align: TextAlign,
    pub direction: TextDirection,
    pub vertical: bool,
    /// One line, cut to `max_width` with a trailing '…'.
    pub ellipsize: bool,
}

impl<'a> LayoutSpec<'a> {
    /// The layout a block is drawn with.
    pub fn of_block(block: &'a TextBlock) -> Self {
        Self {
            font_size: block.font_size,
            line_height: block.line_height,
            max_width: Some(block.max_width),
            letter_spacing: block.letter_spacing,
            font: block.font.as_ref(),
            weight: block.weight,
            style: block.style,
            faces: &block.face_ranges,
            wrap: block.wrap,
            align: block.align,
            direction: block.direction,
            vertical: block.vertical,
            ellipsize: block.ellipsize,
        }
    }

    /// Plain text at `font_size` in the default face, with the default line
    /// height: what the measuring calls without a block describe.
    pub fn plain(font_size: f32, max_width: Option<f32>) -> Self {
        Self {
            font_size,
            line_height: font_size * LINE_HEIGHT_RATIO,
            max_width,
            letter_spacing: 0.0,
            font: None,
            weight: Weight::NORMAL,
            style: Style::Normal,
            faces: &[],
            wrap: WrapMode::default(),
            align: TextAlign::default(),
            direction: TextDirection::default(),
            vertical: false,
            ellipsize: false,
        }
    }

    /// The face ranges shaping uses: none for vertical text, which stacks
    /// its clusters in a string of its own.
    fn shaped_faces(&self) -> &'a [FaceRange] {
        if self.vertical { &[] } else { self.faces }
    }

    fn key(&self) -> LayoutKey {
        LayoutKey {
            font_size: self.font_size.to_bits(),
            line_height: self.line_height.to_bits(),
            max_width: self.max_width.map(f32::to_bits),
            letter_spacing: self.letter_spacing.to_bits(),
            family: family_hash(self.font),
            weight: self.weight.0,
            style: style_disc(self.style),
            faces: faces_hash(self.shaped_faces()),
            wrap: self.wrap,
            align: self.align,
            direction: self.direction,
            vertical: self.vertical,
            // Vertical text is never cut, so its layout doesn't depend on it.
            ellipsize: self.ellipsize && !self.vertical,
        }
    }

    fn family(&self) -> Family<'a> {
        self.font
            .map(|font| Family::Name(font.family()))
            .unwrap_or(Family::SansSerif)
    }
}

/// [`LayoutSpec`] as a hashable key: floats as their bits, the font as a hash
/// of its family name.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LayoutKey {
    font_size: u32,
    line_height: u32,
    max_width: Option<u32>,
    letter_spacing: u32,
    family: u64,
    weight: u16,
    style: u8,
    /// A hash of the face ranges; `0` for none.
    faces: u64,
    wrap: WrapMode,
    align: TextAlign,
    direction: TextDirection,
    vertical: bool,
    ellipsize: bool,
}

/// A hash of `faces` for [`LayoutKey`]: `0` when there are none, so plain
/// text costs nothing.
fn faces_hash(faces: &[FaceRange]) -> u64 {
    use std::hash::{Hash, Hasher};
    if faces.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for face in faces {
        face.range.hash(&mut hasher);
        family_hash(face.font.as_ref()).hash(&mut hasher);
        face.weight.0.hash(&mut hasher);
        style_disc(face.style).hash(&mut hasher);
    }
    hasher.finish()
}

/// One shaped glyph, **relative to the block origin**. A block's position,
/// colour, clip and effects are applied when it is drawn, so one layout serves
/// every block with the same content and [`LayoutSpec`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapedGlyph {
    pub font_id: fontdb::ID,
    /// The weight cosmic-text resolved the face at; with `font_id`, what
    /// `FontSystem::get_font` needs for the glyph's outline.
    pub font_weight: fontdb::Weight,
    pub glyph_id: u16,
    /// Pen x relative to the block's `x`.
    pub rel_x: f32,
    /// Baseline y relative to the block's `y`.
    pub rel_y: f32,
    /// Per-glyph font size (usually the block's; cosmic-text reports it per
    /// glyph, so it is kept).
    pub font_size: f32,
    /// Byte offset of the glyph's source character in the block's content, for
    /// span and style-range colours.
    pub byte_start: u32,
    /// Where the glyph's cluster ends in the content: with `byte_start`, the
    /// bytes selecting the glyph selects.
    pub byte_end: u32,
    /// The glyph's advance: its cell spans `rel_x..rel_x + advance`.
    pub advance: f32,
    /// Whether the glyph runs right to left, so its cell's left edge is its
    /// end rather than its start.
    pub rtl: bool,
}

/// One laid-out line of a [`ShapedLayout`], relative to the block origin.
#[derive(Clone, Debug)]
pub(crate) struct ShapedLine {
    /// The line box's top.
    pub top: f32,
    pub height: f32,
    /// The baseline, below the block's top.
    pub baseline: f32,
    /// Its glyphs, as indices into [`ShapedLayout::glyphs`].
    pub glyphs: std::ops::Range<u32>,
    /// Where the line starts in the content; all an empty line has.
    pub byte_start: u32,
}

/// A laid-out string: its glyphs, including outline-less ones such as spaces,
/// and the size layout reserves for it.
pub(crate) struct ShapedLayout {
    pub glyphs: Vec<ShapedGlyph>,
    /// The lines, top to bottom, including empty ones.
    pub lines: Vec<ShapedLine>,
    /// How many bytes of the content it shows: all of them, unless an
    /// ellipsis cut the rest off. Glyphs from here on are the ellipsis.
    pub shown: u32,
    /// The widest line's advance and the lines' total height: what
    /// [`TextMeasurer`](crate::TextMeasurer) reports.
    pub size: (f32, f32),
    /// The band the glyphs ink, once
    /// [`TextMeasurer::measure_block_ink`](crate::TextMeasurer::measure_block_ink)
    /// asked for it.
    pub ink: OnceCell<Option<(f32, f32)>>,
}

struct CachedLayout {
    layout: ShapedLayout,
    /// [`LayoutCache::uses`] when this layout was last looked up.
    last_used: u64,
    bytes: usize,
}

/// Layouts keyed by spec, then content, so a lookup borrows the content as
/// `&str` and allocates nothing.
struct LayoutCache {
    map: HashMap<LayoutKey, HashMap<String, CachedLayout>>,
    /// The bytes kept before the least recently used layouts go
    /// ([`LAYOUT_BUDGET`]), and what eviction trims down to.
    budget: usize,
    low_water: usize,
    /// Lookups so far: the clock least-recently-used eviction goes by.
    uses: u64,
    len: usize,
    bytes: usize,
    hits: u64,
    shaped: u64,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self::with_budget(LAYOUT_BUDGET)
    }
}

impl LayoutCache {
    fn with_budget(budget: usize) -> Self {
        Self {
            map: HashMap::new(),
            budget,
            low_water: low_water(budget),
            uses: 0,
            len: 0,
            bytes: 0,
            hits: 0,
            shaped: 0,
        }
    }

    fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.low_water = low_water(budget);
        if self.bytes > budget {
            self.evict();
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.len = 0;
        self.bytes = 0;
    }

    fn get_or_shape(
        &mut self,
        spec: &LayoutSpec<'_>,
        content: &str,
        font_system: &mut FontSystem,
    ) -> &ShapedLayout {
        let key = spec.key();
        self.uses += 1;
        let kept = self
            .map
            .get(&key)
            .is_some_and(|inner| inner.contains_key(content));
        if kept {
            self.hits += 1;
        } else {
            self.shaped += 1;
            let layout = shape_layout(font_system, spec, content);
            let bytes = content.len()
                + layout.glyphs.len() * std::mem::size_of::<ShapedGlyph>()
                + layout.lines.len() * std::mem::size_of::<ShapedLine>()
                + LAYOUT_OVERHEAD;
            if self.bytes + bytes > self.budget {
                self.evict();
            }
            self.len += 1;
            self.bytes += bytes;
            self.map.entry(key).or_default().insert(
                content.to_owned(),
                CachedLayout {
                    layout,
                    last_used: self.uses,
                    bytes,
                },
            );
        }
        let entry = self
            .map
            .get_mut(&key)
            .and_then(|inner| inner.get_mut(content))
            .expect("the layout was kept or just shaped");
        entry.last_used = self.uses;
        &entry.layout
    }

    /// Drop the least recently used layouts down to the low-water mark.
    fn evict(&mut self) {
        let mut uses: Vec<(u64, usize)> = self
            .map
            .values()
            .flat_map(|inner| inner.values().map(|entry| (entry.last_used, entry.bytes)))
            .collect();
        uses.sort_unstable_by_key(|&(last_used, _)| std::cmp::Reverse(last_used));
        // Keep the most recently used layouts while they fit under the
        // low-water mark; the first that doesn't, and everything older, goes.
        let mut kept = 0;
        let Some(cutoff) = uses.into_iter().find_map(|(last_used, bytes)| {
            kept += bytes;
            (kept > self.low_water).then_some(last_used)
        }) else {
            return;
        };
        let (mut len, mut total) = (0, 0);
        for inner in self.map.values_mut() {
            inner.retain(|_, entry| {
                let keep = entry.last_used > cutoff;
                if keep {
                    len += 1;
                    total += entry.bytes;
                }
                keep
            });
        }
        self.map.retain(|_, inner| !inner.is_empty());
        self.len = len;
        self.bytes = total;
    }
}

/// What eviction trims a cache with `budget` down to: three quarters of it,
/// so eviction runs rarely rather than on every new layout.
fn low_water(budget: usize) -> usize {
    budget / 4 * 3
}

/// Shape `content` under `spec` through cosmic-text.
pub(crate) fn shape_layout(
    font_system: &mut FontSystem,
    spec: &LayoutSpec<'_>,
    content: &str,
) -> ShapedLayout {
    let family = spec.family();
    let faces = spec.shaped_faces();

    // An ellipsized block is one line, cut to `max_width` with a trailing '…'.
    // Vertical text ignores ellipsis: it stacks the whole content.
    let cut = match spec.max_width {
        Some(max_width) if spec.ellipsize && !spec.vertical => ellipsis_cut(
            font_system,
            content,
            spec.font_size,
            spec.line_height,
            max_width,
            family,
            spec.weight,
            spec.style,
            faces,
            spec.letter_spacing,
        ),
        _ => None,
    };
    let truncated;
    let (content, shown): (&str, usize) = match cut {
        Some(cut) => {
            truncated = format!("{}…", &content[..cut]);
            (&truncated, cut)
        }
        None => (content, content.len()),
    };

    // Force the base paragraph direction (if requested) by prepending a
    // zero-width strong mark; cosmic-text has no base-direction API. The mark
    // shifts every glyph's byte offset by its UTF-8 length, undone below via
    // `prefix_len`. Vertical text skips this — base direction is meaningless
    // for a one-glyph-per-row column — and instead stacks each grapheme
    // cluster on its own line (see `vertical_stack_string`).
    let prefix = if spec.vertical {
        ""
    } else {
        direction_prefix(spec.direction)
    };
    let prefix_len = prefix.len();
    let shaped_text: Cow<str> = if spec.vertical {
        Cow::Owned(vertical_stack_string(content))
    } else if prefix.is_empty() {
        Cow::Borrowed(content)
    } else {
        Cow::Owned(format!("{prefix}{content}"))
    };

    let mut buffer = Buffer::new(font_system, Metrics::new(spec.font_size, spec.line_height));
    if spec.vertical {
        // One cluster per line, no wrapping; shrink to the content so the
        // manual centring below governs horizontal placement.
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
    } else if spec.ellipsize {
        // The cut line, never wrapped, but still aligned within `max_width`:
        // a centred, ellipsized button label sits in the middle of its key.
        buffer.set_wrap(Wrap::None);
        buffer.set_size(spec.max_width, None);
    } else {
        buffer.set_wrap(spec.wrap.into());
        buffer.set_size(Some(spec.max_width.unwrap_or(f32::MAX / 4.0)), None);
    }
    let attrs = Attrs::new()
        .family(family)
        .weight(spec.weight)
        .style(spec.style)
        .letter_spacing(letter_spacing_em(spec.letter_spacing, spec.font_size));
    // `Start` is cosmic-text's default, so only the rest override it.
    // Vertical text never sets a cosmic align: it centres each row within
    // the column itself (below), whatever the shrink-to-content width.
    let align = cosmic_align(spec.align).filter(|_| !spec.vertical);
    if faces.is_empty() {
        buffer.set_text(&shaped_text, &attrs, Shaping::Advanced, align);
    } else {
        // The ranges address the content: past the direction prefix, and
        // not into an ellipsis.
        let runs = face_runs(&shaped_text, prefix_len, shown, faces, &attrs);
        buffer.set_rich_text(runs, &attrs, Shaping::Advanced, align);
    }
    buffer.shape_until_scroll(font_system, false);

    // Vertical: the column is as wide as the widest cluster row; each row is
    // centred within it by shifting its glyphs right by half the slack.
    let column_w = if spec.vertical {
        buffer
            .layout_runs()
            .fold(0.0f32, |widest, run| widest.max(run.line_w))
    } else {
        0.0
    };

    // Vertical: the whole column sits within `max_width` per `align`, as
    // horizontal text does — `Start`/`Left` flush left, `Center` centres,
    // `End`/`Right` flush right. A column wider than `max_width` stays at the
    // origin rather than shifting off the left edge. (Vertical text has no
    // bidi, so `Start`/`End` are left/right.)
    let column_off_x = if spec.vertical {
        let slack = (spec.max_width.unwrap_or(0.0) - column_w).max(0.0);
        match spec.align {
            TextAlign::Center => slack / 2.0,
            TextAlign::End | TextAlign::Right => slack,
            TextAlign::Start | TextAlign::Left => 0.0,
        }
    } else {
        0.0
    };

    // cosmic-text reports `glyph.start` relative to the glyph's own buffer
    // line, and span colours address the whole content, so each shaped line's
    // start byte is added back. Vertical text also subtracts the `line_i`
    // separators `vertical_stack_string` inserted.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(
            shaped_text
                .bytes()
                .enumerate()
                .filter(|&(_, byte)| byte == b'\n')
                .map(|(index, _)| index + 1),
        )
        .collect();

    let mut glyphs = Vec::new();
    let mut lines = Vec::new();
    let mut width = 0.0f32;
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height += run.line_height;
        let line_off_x = if spec.vertical {
            column_off_x + (column_w - run.line_w) / 2.0
        } else {
            0.0
        };
        let line_base = line_starts.get(run.line_i).copied().unwrap_or(0);
        // The direction prefix sits once at the head of the shaped string
        // (never inside later lines), so it comes off the absolute byte once,
        // not off the line base.
        let content_byte = |shaped: usize| {
            if spec.vertical {
                (line_base + shaped).saturating_sub(run.line_i)
            } else {
                (line_base + shaped).saturating_sub(prefix_len)
            }
        };
        let first = glyphs.len() as u32;
        for glyph in run.glyphs {
            let font_size = glyph.font_size;
            glyphs.push(ShapedGlyph {
                font_id: glyph.font_id,
                font_weight: glyph.font_weight,
                glyph_id: glyph.glyph_id,
                rel_x: glyph.x + font_size * glyph.x_offset + line_off_x,
                rel_y: run.line_y + glyph.y - font_size * glyph.y_offset,
                font_size,
                byte_start: content_byte(glyph.start) as u32,
                byte_end: content_byte(glyph.end) as u32,
                advance: glyph.w,
                rtl: glyph.level.is_rtl(),
            });
        }
        lines.push(ShapedLine {
            top: run.line_top,
            height: run.line_height,
            baseline: run.line_y,
            glyphs: first..glyphs.len() as u32,
            byte_start: content_byte(0) as u32,
        });
    }

    let size = if content.is_empty() {
        (0.0, spec.line_height)
    } else {
        (width, height.max(spec.line_height))
    };
    ShapedLayout {
        glyphs,
        lines,
        shown: shown as u32,
        size,
        ink: OnceCell::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::shared_font_system;

    fn bytes_of(cache: &LayoutCache, content: &str) -> usize {
        cache
            .map
            .values()
            .find_map(|inner| inner.get(content))
            .map_or(0, |entry| entry.bytes)
    }

    #[test]
    fn an_ellipsized_line_keeps_its_alignment_within_its_width() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let spec = |align: TextAlign| LayoutSpec {
            align,
            ellipsize: true,
            ..LayoutSpec::plain(12.0, Some(200.0))
        };
        let start = shape_layout(shared.font_system(), &spec(TextAlign::Start), "Send");
        let center = shape_layout(shared.font_system(), &spec(TextAlign::Center), "Send");
        let end = shape_layout(shared.font_system(), &spec(TextAlign::End), "Send");
        let w = start.size.0;
        assert_eq!(center.size, start.size, "the measured size is the text's");
        assert!(start.glyphs[0].rel_x.abs() < 0.01);
        assert!(
            (center.glyphs[0].rel_x - (200.0 - w) / 2.0).abs() < 0.5,
            "centred: {} of {w}",
            center.glyphs[0].rel_x
        );
        assert!((end.glyphs[0].rel_x - (200.0 - w)).abs() < 0.5);

        // Too long: cut to one line that still fits, from the left edge.
        let long = "a label far too long for the two hundred pixels it is given here";
        let cut = shape_layout(shared.font_system(), &spec(TextAlign::Center), long);
        assert!(cut.size.0 <= 200.0 && cut.size.1 < 20.0, "{:?}", cut.size);
        assert!(cut.glyphs[0].rel_x >= 0.0);
    }

    #[test]
    fn a_layout_is_shaped_once_per_spec_and_content() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let plain = LayoutSpec::plain(16.0, Some(200.0));
        let wide = LayoutSpec::plain(16.0, Some(400.0));
        let first = shared.layout(&plain, "one line of text").size;
        assert_eq!(shared.layout(&plain, "one line of text").size, first);
        shared.layout(&wide, "one line of text");
        shared.layout(&plain, "another line");
        let stats = shared.layout_stats();
        assert_eq!((stats.shaped, stats.hits, stats.layouts), (3, 1, 3));
        assert!(stats.bytes > 0);
    }

    #[test]
    fn vertical_text_shares_a_layout_whether_or_not_it_asks_for_an_ellipsis() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let vertical = LayoutSpec {
            vertical: true,
            ..LayoutSpec::plain(16.0, Some(40.0))
        };
        let cut = LayoutSpec {
            ellipsize: true,
            ..vertical
        };
        let size = shared.layout(&vertical, "縦書き").size;
        assert_eq!(shared.layout(&cut, "縦書き").size, size);
        let stats = shared.layout_stats();
        assert_eq!((stats.shaped, stats.hits), (1, 1));
    }

    #[test]
    fn with_no_layout_budget_only_the_last_layout_is_kept() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let spec = LayoutSpec::plain(16.0, Some(200.0));
        shared.layout(&spec, "first");
        shared.layout(&spec, "second");
        shared.set_layout_budget(0);
        assert_eq!(
            shared.layout_stats().layouts,
            0,
            "lowering it trims at once"
        );
        let size = shared.layout(&spec, "third").size;
        assert!(size.0 > 0.0, "the layout is still there to be read");
        shared.layout(&spec, "fourth");
        let stats = shared.layout_stats();
        assert_eq!((stats.layouts, stats.shaped), (1, 4));
        assert_eq!(shared.kept_layouts("fourth").count(), 1);
    }

    #[test]
    fn changing_the_fonts_drops_every_layout() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        shared.layout(&LayoutSpec::plain(16.0, None), "text");
        assert_eq!(shared.layout_stats().layouts, 1);
        shared.db_mut();
        let stats = shared.layout_stats();
        assert_eq!((stats.layouts, stats.bytes), (0, 0));
        shared.layout(&LayoutSpec::plain(16.0, None), "text");
        assert_eq!(shared.layout_stats().shaped, 2, "shaped again");
    }

    #[test]
    fn over_budget_the_least_recently_used_go() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let fonts = shared.font_system();
        let spec = LayoutSpec::plain(16.0, None);
        // Size the budget to about ten layouts of this shape.
        let mut probe = LayoutCache::default();
        probe.get_or_shape(&spec, "row 00", fonts);
        let one = bytes_of(&probe, "row 00");
        let mut cache = LayoutCache::with_budget(one * 10);

        for row in 0..16 {
            cache.get_or_shape(&spec, &format!("row {row:02}"), fonts);
            assert!(cache.bytes <= one * 10, "{} > {}", cache.bytes, one * 10);
        }
        let kept: Vec<_> = (0..16)
            .filter(|row| bytes_of(&cache, &format!("row {row:02}")) > 0)
            .collect();
        assert_eq!(kept.last(), Some(&15), "the newest stays: {kept:?}");
        assert!(!kept.contains(&0), "the oldest went first: {kept:?}");
        assert!(
            kept.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "what stays is the most recent run: {kept:?}"
        );
        let counted: usize = cache.map.values().map(HashMap::len).sum();
        assert_eq!(cache.len, counted);
    }

    fn face(range: std::ops::Range<usize>, weight: Weight, font: Option<FontHandle>) -> FaceRange {
        FaceRange {
            range,
            font,
            weight,
            style: Style::Normal,
        }
    }

    /// The advance of the glyphs whose bytes are in `range`.
    fn advance_of(layout: &ShapedLayout, range: std::ops::Range<usize>) -> f32 {
        layout
            .glyphs
            .iter()
            .filter(|glyph| range.contains(&(glyph.byte_start as usize)))
            .map(|glyph| glyph.advance)
            .sum()
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn a_face_range_sets_its_bytes_in_its_own_face() {
        let handle = shared_font_system();
        let mono = crate::bundled_mono_font(&handle);
        let mut shared = handle.lock().unwrap();
        let text = "plain bold illicit plain";
        let plain = shape_layout(shared.font_system(), &LayoutSpec::plain(16.0, None), text);
        let faces = [
            face(6..10, Weight::BOLD, None),
            face(11..18, Weight::NORMAL, mono),
        ];
        let spec = LayoutSpec {
            faces: &faces,
            ..LayoutSpec::plain(16.0, None)
        };
        let faced = shape_layout(shared.font_system(), &spec, text);

        assert!(
            advance_of(&faced, 6..10) > advance_of(&plain, 6..10),
            "bold is wider"
        );
        assert!(
            advance_of(&faced, 11..18) > advance_of(&plain, 11..18) * 1.3,
            "mono gives the narrow letters a full cell"
        );
        // The words around them keep the block's face.
        assert_eq!(advance_of(&faced, 0..5), advance_of(&plain, 0..5));
        let font_of = |layout: &ShapedLayout, byte: u32| {
            layout
                .glyphs
                .iter()
                .find(|g| g.byte_start == byte)
                .unwrap()
                .font_id
        };
        assert_ne!(font_of(&faced, 6), font_of(&plain, 6));
        assert_eq!(font_of(&faced, 19), font_of(&plain, 19));
    }

    #[test]
    fn face_ranges_are_part_of_the_layout_key() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let plain = LayoutSpec::plain(16.0, None);
        let faces = [face(0..4, Weight::BOLD, None)];
        let bold = LayoutSpec {
            faces: &faces,
            ..plain
        };
        let plain_w = shared.layout(&plain, "bold words").size.0;
        let bold_w = shared.layout(&bold, "bold words").size.0;
        assert!(bold_w > plain_w);
        assert_eq!(
            shared.layout_stats().shaped,
            2,
            "not served from the plain layout"
        );
    }

    #[test]
    fn a_face_range_that_does_not_fit_is_skipped() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let text = "café au lait";
        let plain = shape_layout(shared.font_system(), &LayoutSpec::plain(16.0, None), text);
        // From inside the `é` (bytes 3..5), empty, and one starting before
        // the one ahead of it ends.
        let faces = [
            face(4..9, Weight::BOLD, None),
            face(9..9, Weight::BOLD, None),
            face(6..8, Weight::BOLD, None),
            face(7..10, Weight::BOLD, None),
        ];
        let spec = LayoutSpec {
            faces: &faces,
            ..LayoutSpec::plain(16.0, None)
        };
        let faced = shape_layout(shared.font_system(), &spec, text);
        assert_eq!(advance_of(&faced, 0..6), advance_of(&plain, 0..6));
        assert!(
            advance_of(&faced, 6..8) > advance_of(&plain, 6..8),
            "`au` fits"
        );
        assert_eq!(
            advance_of(&faced, 8..13),
            advance_of(&plain, 8..13),
            "the overlap doesn't"
        );
        // One running past the end is cut at it.
        let past = [face(9..99, Weight::BOLD, None)];
        let spec = LayoutSpec {
            faces: &past,
            ..LayoutSpec::plain(16.0, None)
        };
        let faced = shape_layout(shared.font_system(), &spec, text);
        assert!(advance_of(&faced, 9..13) > advance_of(&plain, 9..13));
    }

    #[test]
    fn a_face_range_counts_from_the_content_past_a_direction_prefix() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let text = "bold then plain";
        let faces = [face(0..4, Weight::BOLD, None)];
        let ltr = LayoutSpec {
            faces: &faces,
            direction: TextDirection::Ltr,
            ..LayoutSpec::plain(16.0, None)
        };
        let auto = LayoutSpec {
            faces: &faces,
            ..LayoutSpec::plain(16.0, None)
        };
        let with_prefix = shape_layout(shared.font_system(), &ltr, text);
        let without = shape_layout(shared.font_system(), &auto, text);
        assert_eq!(advance_of(&with_prefix, 0..4), advance_of(&without, 0..4));
        assert_eq!(advance_of(&with_prefix, 5..15), advance_of(&without, 5..15));
    }

    #[test]
    fn an_ellipsis_cuts_faced_text_by_its_faced_width() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let text = "a rather long project name that will not fit";
        let faces = [face(0..text.len(), Weight::BOLD, None)];
        let cut = |faces: &[FaceRange], shared: &mut SharedFontSystem| {
            let spec = LayoutSpec {
                faces,
                ellipsize: true,
                ..LayoutSpec::plain(16.0, Some(150.0))
            };
            let layout = shape_layout(shared.font_system(), &spec, text);
            (layout.shown, layout.size.0)
        };
        let (plain_shown, plain_w) = cut(&[], &mut shared);
        let (bold_shown, bold_w) = cut(&faces, &mut shared);
        assert!(plain_shown < text.len() as u32);
        assert!(
            bold_shown < plain_shown,
            "bold letters are wider, so fewer fit"
        );
        assert!(plain_w <= 150.0 && bold_w <= 150.0, "{plain_w} {bold_w}");
    }

    #[test]
    fn a_hit_counts_as_use() {
        let handle = shared_font_system();
        let mut shared = handle.lock().unwrap();
        let fonts = shared.font_system();
        let spec = LayoutSpec::plain(16.0, None);
        let mut probe = LayoutCache::default();
        probe.get_or_shape(&spec, "row 0", fonts);
        let one = bytes_of(&probe, "row 0");
        let mut cache = LayoutCache::with_budget(one * 4);
        for row in 0..3 {
            cache.get_or_shape(&spec, &format!("row {row}"), fonts);
        }
        // "row 0" is used again, then new rows push past budget: "row 1" is
        // now the least recently used, though "row 0" is older.
        cache.get_or_shape(&spec, "row 0", fonts);
        cache.get_or_shape(&spec, "row 3", fonts);
        cache.get_or_shape(&spec, "row 4", fonts);
        assert!(bytes_of(&cache, "row 0") > 0, "used again");
        assert_eq!(bytes_of(&cache, "row 1"), 0);
        assert!(bytes_of(&cache, "row 4") > 0);
    }
}
