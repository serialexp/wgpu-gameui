//! Selecting text outside fields: labels, transcripts, anything drawn as a
//! [`TextBlock`] marked [`selectable`](TextBlock::selectable). A press on such
//! text starts a selection, a drag extends it (across blocks too), a double
//! click takes a word and a triple click a paragraph, and the shortcut copy
//! key copies it when no text field has focus.
//!
//! Each selectable block carries a [`TextKey`]: a scope and its place in
//! that scope. A selection stays within the scope it began in, and spans the
//! blocks between its ends in key order, so the order text reads in is the
//! caller's to say, not the order it happens to be drawn.
//!
//! [`UiState`](crate::UiState) keeps a [`TextSelection`] and drives it; the
//! host adds two calls to its frame:
//!
//! 1. [`UiState::begin_frame`](crate::UiState::begin_frame) hit-tests the
//!    frame's pointer against the text drawn **last** frame — what is on
//!    screen when the user presses — and handles the copy key.
//! 2. Before drawing, [`LayerStack::set_text_highlight`] with
//!    [`TextSelection::highlight`], so each selected block paints its
//!    highlight behind its glyphs as it is drawn.
//! 3. After drawing, [`TextSelection::collect`] with the frame's
//!    [`LayerStack`], to keep what was drawn for the next frame.
//!
//! Text the selection covers but that was never drawn while it was made
//! (rows of a virtualized list scrolled past in one jump) is not in
//! [`TextSelection::text`]; a drag past a scrolling edge draws every block it
//! crosses, so it is complete.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::layer::{LayerKind, LayerStack};
use crate::layout::Rect;
use crate::shaping::{LayoutSpec, ShapedLayout};
use crate::text::{FaceRange, FontHandle, FontSystemHandle, TextBlock, TextDirection, WrapMode};
use crate::text_units::floor_boundary;
use crate::{InputState, TextAlign, TextUnit};

/// Where a selectable block sits among the text a selection can span: a
/// `scope` (one transcript, one panel) and its `order` in it. Keys compare by
/// scope, then order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextKey {
    /// Which body of text the block belongs to. A selection never leaves it.
    pub scope: u64,
    /// The block's place in its scope's reading order.
    pub order: u64,
}

impl TextKey {
    /// The block at `order` in `scope`.
    pub const fn new(scope: u64, order: u64) -> Self {
        Self { scope, order }
    }
}

/// A caret position in selectable text: a byte in the block `key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPoint {
    /// The block.
    pub key: TextKey,
    /// A byte offset in its content.
    pub byte: usize,
}

impl TextPoint {
    /// Byte `byte` of the block `key`.
    pub const fn new(key: TextKey, byte: usize) -> Self {
        Self { key, byte }
    }
}

/// What a draw list paints behind selected text: the selection, in order,
/// and its colour. Handed to [`LayerStack::set_text_highlight`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextHighlight {
    /// Where the selection starts (the earlier end).
    pub start: TextPoint,
    /// Where it ends (the later end).
    pub end: TextPoint,
    /// The highlight's fill.
    pub color: [f32; 4],
}

impl TextHighlight {
    /// The bytes of the block `key`, `len` bytes long, that the highlight
    /// covers; `None` when it covers none of them.
    pub fn range_of(&self, key: TextKey, len: usize) -> Option<Range<usize>> {
        if key < self.start.key || key > self.end.key {
            return None;
        }
        let start = if key == self.start.key {
            self.start.byte.min(len)
        } else {
            0
        };
        let end = if key == self.end.key {
            self.end.byte.min(len)
        } else {
            len
        };
        (start < end).then_some(start..end)
    }
}

/// A selectable block as last drawn: in screen space, on layer `layer` (0 is
/// the base, `n` the stack's `n`-th pushed layer).
struct Drawn {
    key: TextKey,
    layer: usize,
    block: DrawnBlock,
}

/// What the selection needs of a drawn block to lay it out again and hit-test
/// it. Kept apart from [`TextBlock`] so each frame's [`TextSelection::collect`]
/// overwrites last frame's copy in place instead of cloning whole blocks.
struct DrawnBlock {
    content: String,
    x: f32,
    y: f32,
    clip: Option<Rect>,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    letter_spacing: f32,
    font: Option<FontHandle>,
    weight: cosmic_text::Weight,
    style: cosmic_text::Style,
    faces: Arc<Vec<FaceRange>>,
    wrap: WrapMode,
    align: TextAlign,
    direction: TextDirection,
    vertical: bool,
    ellipsize: bool,
}

impl DrawnBlock {
    fn of(block: &TextBlock) -> Self {
        Self {
            content: block.content.clone(),
            x: block.x,
            y: block.y,
            clip: block.clip,
            font_size: block.font_size,
            line_height: block.line_height,
            max_width: block.max_width,
            letter_spacing: block.letter_spacing,
            font: block.font.clone(),
            weight: block.weight,
            style: block.style,
            faces: block.face_ranges.clone(),
            wrap: block.wrap,
            align: block.align,
            direction: block.direction,
            vertical: block.vertical,
            ellipsize: block.ellipsize,
        }
    }

    /// Become `block`, reusing this one's allocations.
    fn update(&mut self, block: &TextBlock) {
        self.content.clone_from(&block.content);
        self.font.clone_from(&block.font);
        self.x = block.x;
        self.y = block.y;
        self.clip = block.clip;
        self.font_size = block.font_size;
        self.line_height = block.line_height;
        self.max_width = block.max_width;
        self.letter_spacing = block.letter_spacing;
        self.weight = block.weight;
        self.style = block.style;
        self.faces.clone_from(&block.face_ranges);
        self.wrap = block.wrap;
        self.align = block.align;
        self.direction = block.direction;
        self.vertical = block.vertical;
        self.ellipsize = block.ellipsize;
    }

    /// The layout it was drawn with, as [`LayoutSpec::of_block`] gives it.
    fn spec(&self) -> LayoutSpec<'_> {
        LayoutSpec {
            font_size: self.font_size,
            line_height: self.line_height,
            max_width: Some(self.max_width),
            letter_spacing: self.letter_spacing,
            font: self.font.as_ref(),
            weight: self.weight,
            style: self.style,
            faces: &self.faces,
            wrap: self.wrap,
            align: self.align,
            direction: self.direction,
            vertical: self.vertical,
            ellipsize: self.ellipsize,
        }
    }
}

/// What began the selection: the press's block, the bytes it took (empty for
/// a single click), the unit a drag grows the selection by, and the layer the
/// press landed on (a drag off the text looks for blocks there only, even
/// once the pressed block has scrolled out of view).
#[derive(Clone, Debug)]
struct Origin {
    key: TextKey,
    range: Range<usize>,
    unit: TextUnit,
    layer: usize,
}

/// Where a drag off the text is, from the block it selects up to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// Level with it: up to the point across from the pointer.
    Level,
    /// Below it: to its end.
    Above,
    /// Above it: from its start.
    Below,
}

/// The selection over selectable text, and what it needs from the last frame
/// to hit-test the pointer. See the [module docs](self).
#[derive(Default)]
pub struct TextSelection {
    anchor: Option<TextPoint>,
    focus: Option<TextPoint>,
    origin: Option<Origin>,
    /// Whether the primary button is still down from the press that began it.
    dragging: bool,
    /// The selectable text the last frame drew, bottom layer first, in draw
    /// order within each layer.
    drawn: Vec<Drawn>,
    /// The last frame's overlay layers: which of them hide what is under the
    /// pointer.
    layers: Vec<(LayerKind, Rect)>,
    font_system: Option<FontSystemHandle>,
    /// The content of every block in the selection's scope drawn since it
    /// began, for [`text`](Self::text).
    contents: BTreeMap<TextKey, String>,
}

impl TextSelection {
    /// No selection, nothing drawn yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The selection's ends, earlier first; `None` when nothing is selected
    /// (including a collapsed selection: a click that didn't drag).
    pub fn range(&self) -> Option<(TextPoint, TextPoint)> {
        let (anchor, focus) = (self.anchor?, self.focus?);
        match anchor.cmp(&focus) {
            std::cmp::Ordering::Less => Some((anchor, focus)),
            std::cmp::Ordering::Greater => Some((focus, anchor)),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.range().is_none()
    }

    /// The scope a drag is selecting in, while the button that began it is
    /// down: a scrolling view reads it to scroll when the pointer leaves it.
    pub fn dragging(&self) -> Option<u64> {
        self.dragging
            .then(|| self.anchor.map(|anchor| anchor.key.scope))
            .flatten()
    }

    /// Drop the selection.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.focus = None;
        self.origin = None;
        self.dragging = false;
        self.contents.clear();
    }

    /// Drop the selection if it is in `scope`: for when that text is
    /// replaced, and its keys stop meaning what they did.
    pub fn clear_scope(&mut self, scope: u64) {
        if self.anchor.is_some_and(|anchor| anchor.key.scope == scope) {
            self.clear();
        }
    }

    /// The highlight to draw the selection with, in `color`; `None` when
    /// nothing is selected.
    pub fn highlight(&self, color: [f32; 4]) -> Option<TextHighlight> {
        let (start, end) = self.range()?;
        Some(TextHighlight { start, end, color })
    }

    /// The selected text, blocks joined by line breaks; `None` when nothing is
    /// selected.
    pub fn text(&self) -> Option<String> {
        let (start, end) = self.range()?;
        let mut out = String::new();
        for (index, (&key, content)) in self.contents.range(start.key..=end.key).enumerate() {
            let from = if key == start.key {
                floor_boundary(content, start.byte)
            } else {
                0
            };
            let to = if key == end.key {
                floor_boundary(content, end.byte)
            } else {
                content.len()
            };
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&content[from..to.max(from)]);
        }
        Some(out)
    }

    /// Whether selectable text is under `(x, y)`, where nothing above hides
    /// it: where to show a text cursor.
    pub fn hovers_text(&self, x: f32, y: f32) -> bool {
        self.hit(x, y).is_some()
    }

    /// Keep what `layers` drew for the next frame's hit-testing, and the
    /// content of what it drew in the selection's scope for
    /// [`text`](Self::text). Call after drawing the frame. A frame that drew
    /// nothing of the selection's scope drops the selection: its text went
    /// away (another page, a closed view, or a caller giving its text a new
    /// scope because the old keys no longer hold).
    pub fn collect(&mut self, layers: &LayerStack) {
        self.layers.clear();
        self.font_system = Some(layers.base().text_measurer.font_system_handle());
        let lists = std::iter::once(layers.base()).chain(layers.layers().iter().map(|l| &l.list));
        let mut count = 0;
        for (layer, list) in lists.enumerate() {
            for (key, block) in list.selectable_texts() {
                match self.drawn.get_mut(count) {
                    Some(drawn) => {
                        drawn.key = key;
                        drawn.layer = layer;
                        drawn.block.update(block);
                    }
                    None => self.drawn.push(Drawn {
                        key,
                        layer,
                        block: DrawnBlock::of(block),
                    }),
                }
                count += 1;
            }
        }
        self.drawn.truncate(count);
        for layer in layers.layers() {
            self.layers.push((layer.kind, layer.rect));
        }
        if let Some(scope) = self.anchor.map(|anchor| anchor.key.scope)
            && !self.drawn.iter().any(|d| d.key.scope == scope)
        {
            self.clear();
        }
        self.remember_drawn();
    }

    /// Keep the content of the drawn blocks in the selection's scope.
    fn remember_drawn(&mut self) {
        let Some(scope) = self.anchor.map(|anchor| anchor.key.scope) else {
            return;
        };
        for drawn in self.drawn.iter().filter(|d| d.key.scope == scope) {
            if self.contents.get(&drawn.key) != Some(&drawn.block.content) {
                self.contents.insert(drawn.key, drawn.block.content.clone());
            }
        }
    }

    /// Act on the frame's pointer: a press on selectable text begins a
    /// selection there (or, with Shift, extends the one there is), a press
    /// anywhere else drops it, and a drag moves its far end. Called by
    /// [`UiState::begin_frame`](crate::UiState::begin_frame).
    pub(crate) fn handle_input(&mut self, input: &InputState) {
        if input.mouse_clicked {
            self.press(input);
        } else if self.dragging && input.mouse_down {
            self.drag(input.mouse_x, input.mouse_y);
        }
        if !input.mouse_down {
            self.dragging = false;
        }
    }

    fn press(&mut self, input: &InputState) {
        let Some(index) = self.hit(input.mouse_x, input.mouse_y) else {
            self.clear();
            return;
        };
        let (key, layer) = (self.drawn[index].key, self.drawn[index].layer);
        let Some((caret, under)) = self.point_in(index, input.mouse_x, input.mouse_y) else {
            self.clear();
            return;
        };
        let content = &self.drawn[index].block.content;
        let unit = TextUnit::for_clicks(input.mouse_click_count);
        let extending = input.shift_pressed
            && unit == TextUnit::Char
            && self
                .anchor
                .is_some_and(|anchor| anchor.key.scope == key.scope);
        if extending {
            let anchor = self.anchor.expect("checked above");
            self.origin = Some(Origin {
                key: anchor.key,
                range: anchor.byte..anchor.byte,
                unit,
                layer,
            });
        } else {
            self.contents.clear();
            let range = match unit {
                TextUnit::Char => caret..caret,
                unit => unit.range_at(content, under),
            };
            self.origin = Some(Origin {
                key,
                range,
                unit,
                layer,
            });
        }
        self.dragging = true;
        self.place(index, caret, under);
    }

    fn drag(&mut self, x: f32, y: f32) {
        let Some(scope) = self.anchor.map(|anchor| anchor.key.scope) else {
            return;
        };
        let target = self
            .hit(x, y)
            .filter(|&index| self.drawn[index].key.scope == scope)
            .map(|index| (index, Side::Level))
            .or_else(|| self.nearest(scope, x, y));
        let Some((index, side)) = target else {
            return;
        };
        let len = self.drawn[index].block.content.len();
        let point = match side {
            Side::Level => self.point_in(index, x, y),
            Side::Above => Some((len, len)),
            Side::Below => Some((0, 0)),
        };
        if let Some((caret, under)) = point {
            self.place(index, caret, under);
        }
    }

    /// Set the selection for the pointer over the character at `under` (caret
    /// `caret`) in drawn block `index`: from the origin, grown by its unit.
    fn place(&mut self, index: usize, caret: usize, under: usize) {
        let Some(origin) = self.origin.clone() else {
            return;
        };
        let drawn = &self.drawn[index];
        let (key, content) = (drawn.key, &drawn.block.content);
        let (anchor, focus) = if key == origin.key {
            let (anchor, focus) = origin.unit.extend(content, origin.range, under, caret);
            (TextPoint::new(key, anchor), TextPoint::new(key, focus))
        } else {
            let unit = origin.unit.range_at(content, under);
            if key > origin.key {
                let end = if origin.unit == TextUnit::Char {
                    caret
                } else {
                    unit.end
                };
                (
                    TextPoint::new(origin.key, origin.range.start),
                    TextPoint::new(key, end),
                )
            } else {
                let start = if origin.unit == TextUnit::Char {
                    caret
                } else {
                    unit.start
                };
                (
                    TextPoint::new(origin.key, origin.range.end),
                    TextPoint::new(key, start),
                )
            }
        };
        self.anchor = Some(anchor);
        self.focus = Some(focus);
        self.remember_drawn();
    }

    /// The topmost drawn block under `(x, y)` that no overlay above hides.
    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let floor = self.lowest_visible_layer(x, y);
        let font_system = self.font_system.as_ref()?;
        self.drawn
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, drawn)| {
                (drawn.layer >= floor
                    && !self.is_tooltip(drawn.layer)
                    && self.bounds(font_system, drawn).contains(x, y))
                .then_some(index)
            })
    }

    /// The lowest layer the pointer at `(x, y)` reaches: past the topmost
    /// modal, or popup it is over.
    fn lowest_visible_layer(&self, x: f32, y: f32) -> usize {
        self.layers
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, &(kind, rect))| match kind {
                LayerKind::Modal => Some(index + 1),
                LayerKind::Popup if rect.contains(x, y) => Some(index + 1),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn is_tooltip(&self, layer: usize) -> bool {
        layer > 0 && self.layers.get(layer - 1).map(|l| l.0) == Some(LayerKind::Tooltip)
    }

    /// For a drag off the text: the block in `scope` nearest `(x, y)` among
    /// those in view, and where it is. One level with the pointer, the closest
    /// across; else the last one above it; else the first below.
    fn nearest(&self, scope: u64, x: f32, y: f32) -> Option<(usize, Side)> {
        let font_system = self.font_system.as_ref()?;
        let layer = self.origin.as_ref()?.layer;
        let candidates: Vec<(usize, Rect)> = self
            .drawn
            .iter()
            .enumerate()
            .filter(|(_, d)| d.key.scope == scope && d.layer == layer)
            .map(|(index, d)| (index, self.bounds(font_system, d)))
            .filter(|(_, bounds)| bounds.width > 0.0 && bounds.height > 0.0)
            .collect();
        let across = |bounds: &Rect| {
            if x < bounds.x {
                bounds.x - x
            } else {
                (x - bounds.right()).max(0.0)
            }
        };
        let level = candidates
            .iter()
            .filter(|(_, b)| y >= b.y && y < b.bottom())
            .min_by(|a, b| across(&a.1).total_cmp(&across(&b.1)));
        let above = || {
            candidates
                .iter()
                .filter(|(_, b)| b.bottom() <= y)
                .max_by(|a, b| {
                    let (ka, kb) = (self.drawn[a.0].key, self.drawn[b.0].key);
                    a.1.bottom().total_cmp(&b.1.bottom()).then(ka.cmp(&kb))
                })
        };
        let below = || {
            candidates.iter().filter(|(_, b)| b.y > y).min_by(|a, b| {
                let (ka, kb) = (self.drawn[a.0].key, self.drawn[b.0].key);
                a.1.y.total_cmp(&b.1.y).then(ka.cmp(&kb))
            })
        };
        level
            .map(|&(index, _)| (index, Side::Level))
            .or_else(|| above().map(|&(index, _)| (index, Side::Above)))
            .or_else(|| below().map(|&(index, _)| (index, Side::Below)))
    }

    /// Where the block's text can be hit, in screen space: its laid-out box
    /// (the full wrap width, where lines wrap or align within it) cut by its
    /// clip.
    fn bounds(&self, font_system: &FontSystemHandle, drawn: &Drawn) -> Rect {
        let block = &drawn.block;
        let (width, height) = {
            let mut shared = font_system.lock().expect("FontSystem poisoned");
            shared.layout(&block.spec(), &block.content).size
        };
        let spans_the_width =
            !block.ellipsize && (block.wrap != WrapMode::None || block.align != TextAlign::Start);
        let width = if spans_the_width {
            width.max(block.max_width)
        } else {
            width
        };
        let own = Rect::new(block.x, block.y, width, height);
        match block.clip {
            Some(clip) => own
                .intersection(clip)
                .unwrap_or(Rect::new(own.x, own.y, 0.0, 0.0)),
            None => own,
        }
    }

    /// The caret nearest `(x, y)` and the character under it, in drawn block
    /// `index`; the point is clamped into the block.
    fn point_in(&self, index: usize, x: f32, y: f32) -> Option<(usize, usize)> {
        let block = &self.drawn[index].block;
        let mut shared = self.font_system.as_ref()?.lock().ok()?;
        let layout = shared.layout(&block.spec(), &block.content);
        let (caret, under) = hit_layout(layout, x - block.x, y - block.y);
        let len = block.content.len();
        Some((
            floor_boundary(&block.content, caret.min(len)),
            floor_boundary(&block.content, under.min(len)),
        ))
    }
}

/// Hit-test a point relative to a layout's origin: the caret nearest it and
/// the byte of the character under it. Above the first line counts as on
/// it, below the last as on that; left and right of a line, its ends.
pub(crate) fn hit_layout(layout: &ShapedLayout, x: f32, y: f32) -> (usize, usize) {
    let Some(line) = layout
        .lines
        .iter()
        .find(|line| y < line.top + line.height)
        .or(layout.lines.last())
    else {
        return (0, 0);
    };
    let glyphs = &layout.glyphs[line.glyphs.start as usize..line.glyphs.end as usize];
    let edges = |g: &crate::shaping::ShapedGlyph| {
        let (left, right) = if g.rtl {
            (g.byte_end, g.byte_start)
        } else {
            (g.byte_start, g.byte_end)
        };
        (left as usize, right as usize)
    };
    let Some(leftmost) = glyphs.iter().min_by(|a, b| a.rel_x.total_cmp(&b.rel_x)) else {
        let start = line.byte_start as usize;
        return (start, start);
    };
    let rightmost = glyphs
        .iter()
        .max_by(|a, b| (a.rel_x + a.advance).total_cmp(&(b.rel_x + b.advance)))
        .expect("the line has a glyph");
    if x < leftmost.rel_x {
        return (edges(leftmost).0, leftmost.byte_start as usize);
    }
    let over = glyphs
        .iter()
        .find(|g| x >= g.rel_x && x < g.rel_x + g.advance)
        .unwrap_or(rightmost);
    let (left, right) = edges(over);
    let caret = if x < over.rel_x + over.advance / 2.0 {
        left
    } else {
        right
    };
    (caret, over.byte_start as usize)
}

/// The highlight rectangles for bytes `range` of a layout, relative to its
/// origin: per line, the glyphs in the range, merged where they touch.
pub(crate) fn highlight_rects(layout: &ShapedLayout, range: Range<usize>) -> Vec<Rect> {
    const TOUCHING: f32 = 0.5;
    let mut out = Vec::new();
    for line in &layout.lines {
        let glyphs = &layout.glyphs[line.glyphs.start as usize..line.glyphs.end as usize];
        let mut spans: Vec<(f32, f32)> = glyphs
            .iter()
            .filter(|g| (g.byte_start as usize) < range.end && (g.byte_end as usize) > range.start)
            .map(|g| (g.rel_x, g.rel_x + g.advance))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut spans = spans.into_iter();
        let Some(mut current) = spans.next() else {
            continue;
        };
        for (left, right) in spans {
            if left <= current.1 + TOUCHING {
                current.1 = current.1.max(right);
            } else {
                out.push(Rect::new(
                    current.0,
                    line.top,
                    current.1 - current.0,
                    line.height,
                ));
                current = (left, right);
            }
        }
        out.push(Rect::new(
            current.0,
            line.top,
            current.1 - current.0,
            line.height,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::shared_font_system;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::OnceLock;

    const SCOPE: u64 = 1;
    const OTHER: u64 = 2;
    const COLOR: [f32; 4] = [0.2, 0.4, 0.8, 0.5];

    fn fonts() -> FontSystemHandle {
        static FONTS: OnceLock<FontSystemHandle> = OnceLock::new();
        FONTS.get_or_init(shared_font_system).clone()
    }

    fn key(order: u64) -> TextKey {
        TextKey::new(SCOPE, order)
    }

    /// Two paragraphs of `SCOPE` at y 10 and 40 (the first two lines long), a
    /// block of `OTHER` at y 100, and a modal over everything when `modal`.
    fn frame(layers: &mut LayerStack, selection: &mut TextSelection, modal: bool) {
        layers.clear();
        layers.set_text_highlight(selection.highlight(COLOR));
        let list = layers.base_mut();
        let block = |text: &str, y: f32, key: TextKey| {
            TextBlock::new(text, 10.0, y)
                .with_size(16.0)
                .with_max_width(300.0)
                .selectable(key)
        };
        list.text(block("hello world\nsecond line", 10.0, key(0)));
        list.text(block("third block", 60.0, key(1)));
        list.text(block("elsewhere", 100.0, TextKey::new(OTHER, 0)));
        // Not selectable.
        list.text(TextBlock::new("button", 10.0, 200.0).with_size(16.0));
        if modal {
            layers.push_modal(Rect::new(0.0, 0.0, 50.0, 50.0));
            layers.pop_layer();
        }
        selection.collect(layers);
    }

    fn press(selection: &mut TextSelection, x: f32, y: f32, clicks: u32, shift: bool) {
        selection.handle_input(&InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: clicks,
            shift_pressed: shift,
            ..InputState::default()
        });
    }

    fn drag(selection: &mut TextSelection, x: f32, y: f32) {
        selection.handle_input(&InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            ..InputState::default()
        });
    }

    fn release(selection: &mut TextSelection) {
        selection.handle_input(&InputState::default());
    }

    fn setup() -> (LayerStack, TextSelection) {
        let mut layers = LayerStack::with_font_system(fonts());
        let mut selection = TextSelection::new();
        frame(&mut layers, &mut selection, false);
        (layers, selection)
    }

    #[test]
    fn a_click_selects_nothing_and_a_drag_selects_across_blocks() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 11.0, 15.0, 1, false);
        assert!(selection.is_empty(), "a click alone selects nothing");
        assert_eq!(selection.dragging(), Some(SCOPE));
        // Past the end of the second block's line.
        drag(&mut selection, 290.0, 65.0);
        assert_eq!(
            selection.text().as_deref(),
            Some("hello world\nsecond line\nthird block")
        );
        release(&mut selection);
        assert_eq!(selection.dragging(), None);
        assert!(!selection.is_empty(), "the selection outlives the drag");
    }

    #[test]
    fn a_drag_off_the_text_stays_on_the_pressed_layer_after_the_press_scrolls_away() {
        let (mut layers, mut selection) = setup();
        press(&mut selection, 11.0, 15.0, 1, false);
        // The pressed block is gone; the rest of the scope is on the base
        // and, nearer the pointer, on a popup.
        layers.clear();
        layers.base_mut().text(
            TextBlock::new("third block", 10.0, 60.0)
                .with_size(16.0)
                .selectable(key(1)),
        );
        layers.push_popup(Rect::new(0.0, 280.0, 300.0, 60.0));
        layers.current_mut().text(
            TextBlock::new("popup text", 10.0, 300.0)
                .with_size(16.0)
                .selectable(key(5)),
        );
        layers.pop_layer();
        selection.collect(&layers);
        drag(&mut selection, 11.0, 400.0);
        assert_eq!(
            selection.text().as_deref(),
            Some("hello world\nsecond line\nthird block")
        );
    }

    #[test]
    fn a_drag_backwards_selects_up_to_the_press() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 290.0, 65.0, 1, false);
        drag(&mut selection, 11.0, 35.0); // start of "second line"
        assert_eq!(
            selection.text().as_deref(),
            Some("second line\nthird block")
        );
    }

    #[test]
    fn a_double_click_takes_a_word_and_a_triple_its_paragraph() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 14.0, 15.0, 2, false);
        assert_eq!(selection.text().as_deref(), Some("hello"));
        release(&mut selection);
        press(&mut selection, 14.0, 35.0, 3, false);
        assert_eq!(selection.text().as_deref(), Some("second line"));
        // A drag after a double click grows by words into the next block.
        release(&mut selection);
        press(&mut selection, 14.0, 15.0, 2, false);
        drag(&mut selection, 14.0, 65.0);
        assert_eq!(
            selection.text().as_deref(),
            Some("hello world\nsecond line\nthird")
        );
    }

    #[test]
    fn shift_click_extends_from_the_anchor() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 14.0, 15.0, 2, false); // "hello"
        release(&mut selection);
        press(&mut selection, 290.0, 65.0, 1, true);
        assert_eq!(
            selection.text().as_deref(),
            Some("hello world\nsecond line\nthird block")
        );
    }

    #[test]
    fn a_press_off_the_text_drops_the_selection() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 14.0, 15.0, 2, false);
        release(&mut selection);
        assert!(!selection.is_empty());
        // Over the unselectable "button".
        press(&mut selection, 14.0, 205.0, 1, false);
        assert!(selection.is_empty());
        assert_eq!(selection.dragging(), None);
    }

    #[test]
    fn a_drag_stays_in_the_scope_it_began_in() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 11.0, 65.0, 1, false); // start of "third block"
        drag(&mut selection, 40.0, 105.0); // over "elsewhere", below ours
        assert_eq!(selection.text().as_deref(), Some("third block"));
    }

    #[test]
    fn a_drag_off_the_text_goes_to_the_nearest_block() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 11.0, 65.0, 1, false);
        // Above everything: the start of the first block.
        drag(&mut selection, 100.0, 0.0);
        assert_eq!(
            selection.text().as_deref(),
            Some("hello world\nsecond line\n")
        );
        // In the gap between the blocks: from the end of the one above, which
        // is only the break between them.
        drag(&mut selection, 200.0, 55.0);
        assert_eq!(selection.text().as_deref(), Some("\n"));
    }

    #[test]
    fn a_modal_hides_the_text_under_it() {
        let mut layers = LayerStack::with_font_system(fonts());
        let mut selection = TextSelection::new();
        frame(&mut layers, &mut selection, true);
        assert!(!selection.hovers_text(14.0, 15.0));
        press(&mut selection, 14.0, 15.0, 2, false);
        assert!(selection.is_empty());
        frame(&mut layers, &mut selection, false);
        assert!(selection.hovers_text(14.0, 15.0));
        assert!(!selection.hovers_text(14.0, 205.0), "not selectable");
    }

    #[test]
    fn the_selection_is_painted_behind_its_blocks() {
        let (mut layers, mut selection) = setup();
        let before = layers.base().chrome_instance_count();
        press(&mut selection, 14.0, 15.0, 3, false);
        frame(&mut layers, &mut selection, false);
        assert!(layers.base().chrome_instance_count() > before);
        // Dropped: nothing painted.
        selection.clear();
        frame(&mut layers, &mut selection, false);
        assert_eq!(layers.base().chrome_instance_count(), before);
    }

    #[test]
    fn a_frame_without_the_selections_text_drops_it() {
        let (mut layers, mut selection) = setup();
        press(&mut selection, 14.0, 105.0, 2, false); // "elsewhere"
        assert!(!selection.is_empty());
        // Only `SCOPE` drawn.
        layers.clear();
        layers.base_mut().text(
            TextBlock::new("hello", 10.0, 10.0)
                .with_size(16.0)
                .selectable(key(0)),
        );
        selection.collect(&layers);
        assert!(selection.is_empty());
        assert_eq!(selection.text(), None);
    }

    #[test]
    fn clearing_another_scope_keeps_the_selection() {
        let (_layers, mut selection) = setup();
        press(&mut selection, 14.0, 15.0, 2, false);
        selection.clear_scope(OTHER);
        assert!(!selection.is_empty());
        selection.clear_scope(SCOPE);
        assert!(selection.is_empty());
    }

    #[test]
    fn the_copy_key_copies_the_selection_unless_a_field_has_focus() {
        let (_layers, selection) = setup();
        let mut ui = crate::UiState::new();
        ui.text_selection = selection;
        let copied = Rc::new(RefCell::new(String::new()));
        let sink = copied.clone();
        ui.set_clipboard(String::new, move |text| *sink.borrow_mut() = text);
        let theme = crate::Theme::default();
        let mut input = InputState {
            mouse_x: 14.0,
            mouse_y: 15.0,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: 2,
            ..InputState::default()
        };
        ui.begin_frame(&mut input, &theme, 0.0, &crate::ManualNav);
        ui.end_frame();
        let mut copy = InputState {
            key_copy: true,
            ..InputState::default()
        };
        ui.begin_frame(&mut copy, &theme, 0.0, &crate::ManualNav);
        ui.end_frame();
        assert_eq!(*copied.borrow(), "hello");
    }

    #[test]
    fn a_layout_hit_clamps_to_its_lines_and_picks_the_character_under() {
        let fonts = fonts();
        let mut shared = fonts.lock().unwrap();
        let block = TextBlock::new("ab\ncd", 0.0, 0.0).with_size(16.0);
        let layout = shared.layout(&LayoutSpec::of_block(&block), &block.content);
        let b = &layout.glyphs[1];
        // Right half of "b": caret after it, but "b" is under the pointer.
        let right_half = b.rel_x + b.advance * 0.75;
        assert_eq!(hit_layout(layout, right_half, 5.0), (2, 1));
        // Above the text: the first line; below it: the last.
        assert_eq!(hit_layout(layout, -5.0, -50.0).0, 0);
        assert_eq!(hit_layout(layout, 500.0, 500.0).0, 5);
        let rects = highlight_rects(layout, 1..4);
        assert_eq!(rects.len(), 2, "one per line: {rects:?}");
    }

    #[test]
    fn a_drawn_block_is_hit_tested_in_its_faces() {
        let fonts = fonts();
        let mut shared = fonts.lock().unwrap();
        let plain = TextBlock::new("bold plain", 0.0, 0.0).with_size(16.0);
        let faced = plain.clone().with_face_ranges(vec![FaceRange {
            range: 0..4,
            font: None,
            weight: cosmic_text::Weight::BOLD,
            style: cosmic_text::Style::Normal,
        }]);
        let drawn = DrawnBlock::of(&faced);
        let again = shared.layout(&drawn.spec(), &drawn.content).size.0;
        let as_drawn = shared
            .layout(&LayoutSpec::of_block(&faced), &faced.content)
            .size
            .0;
        let unfaced = shared
            .layout(&LayoutSpec::of_block(&plain), &plain.content)
            .size
            .0;
        assert_eq!(again, as_drawn);
        assert!(again > unfaced, "the bold word is in the layout");
    }
}
