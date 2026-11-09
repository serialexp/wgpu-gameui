//! What a node is built from: its handle, its properties and the values they
//! hold.
//!
//! Every property is a [`Prop`]: a constant, or a memo the node reads. A
//! [`Spec`] gathers a node's properties before it is added to the tree; the
//! setters on [`Ui`](crate::Ui) change one later.

use wgpu_gameui::{BoxShadow, DrawList, FontHandle, TextAlign, TextContent, Weight};

use crate::signal::{Graph, Memo, Owner, Reader};

/// A node of the tree. A `Copy` handle; using one after its node was removed
/// panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Node {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// A property's value: a constant, or a memo the node reads, so that it
/// changes when the memo does.
#[derive(Clone, Copy, Debug)]
pub enum Prop<T> {
    /// A value that changes only through a setter.
    Const(T),
    /// The value of a memo.
    Bound(Memo<T>),
}

impl<T> From<T> for Prop<T> {
    fn from(value: T) -> Self {
        Prop::Const(value)
    }
}

impl<T> From<Memo<T>> for Prop<T> {
    fn from(memo: Memo<T>) -> Self {
        Prop::Bound(memo)
    }
}

/// How big a node is along one axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Size {
    /// This many logical pixels (rounded to whole ones).
    Fixed(f32),
    /// A share of what the parent has left along its stack, by weight; the
    /// whole of it across the stack. Where the parent's size is not known
    /// (it hugs, or scrolls along that axis), as [`Hug`](Size::Hug).
    Fill(f32),
    /// As big as its content and padding.
    Hug,
}

/// Which way a node stacks its children.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Direction {
    /// Left to right.
    Row,
    /// Top to bottom.
    #[default]
    Column,
}

impl Direction {
    /// The axis the stack runs along: 0 for x, 1 for y.
    pub(crate) fn main(self) -> usize {
        match self {
            Direction::Row => 0,
            Direction::Column => 1,
        }
    }
}

/// Where something sits in the room it has along one axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    /// At the start (left or top).
    #[default]
    Start,
    /// In the middle.
    Center,
    /// At the end (right or bottom).
    End,
}

impl Align {
    /// The share of the free room before the thing aligned.
    pub(crate) fn share(self) -> f32 {
        match self {
            Align::Start => 0.0,
            Align::Center => 0.5,
            Align::End => 1.0,
        }
    }
}

/// Space on each side, in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Insets {
    /// Left.
    pub left: f32,
    /// Top.
    pub top: f32,
    /// Right.
    pub right: f32,
    /// Bottom.
    pub bottom: f32,
}

impl Insets {
    /// The same on every side.
    pub fn all(v: f32) -> Self {
        Self {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }

    /// `x` left and right, `y` top and bottom.
    pub fn xy(x: f32, y: f32) -> Self {
        Self {
            left: x,
            top: y,
            right: x,
            bottom: y,
        }
    }

    /// The start (left or top) of `axis`.
    pub(crate) fn start(&self, axis: usize) -> f32 {
        if axis == 0 { self.left } else { self.top }
    }

    /// The end (right or bottom) of `axis`.
    pub(crate) fn end(&self, axis: usize) -> f32 {
        if axis == 0 { self.right } else { self.bottom }
    }

    /// Each side the larger of the two.
    pub(crate) fn max(self, other: Insets) -> Insets {
        Insets {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// How a node lays out its children: a stack along one axis.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stack {
    /// Which way the children stack.
    pub direction: Direction,
    /// Space between two children (below 0 is 0).
    pub gap: f32,
    /// Space between the node's edges and its content (rounded to whole
    /// pixels).
    pub padding: Insets,
    /// Where the children sit along the stack when they leave room.
    pub main: Align,
    /// Where each child sits across the stack.
    pub cross: Align,
}

impl Stack {
    /// Children left to right.
    pub fn row() -> Self {
        Self {
            direction: Direction::Row,
            ..Self::default()
        }
    }

    /// Children top to bottom.
    pub fn column() -> Self {
        Self::default()
    }

    /// With `gap` between children.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// With `padding` inside the edges.
    pub fn padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    /// With the children aligned `main` along the stack and `cross` across
    /// it.
    pub fn align(mut self, main: Align, cross: Align) -> Self {
        self.main = main;
        self.cross = cross;
        self
    }
}

/// A border drawn inside a node's edges.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Border {
    /// Its width.
    pub width: f32,
    /// Its colour, straight sRGB RGBA.
    pub color: [f32; 4],
}

/// How a node's box is drawn, besides its fill colour.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Look {
    /// A border inside the edges.
    pub border: Option<Border>,
    /// The corners' radius.
    pub radius: f32,
    /// A shadow cast outside the box (an inset one is not drawn).
    pub shadow: Option<BoxShadow>,
}

/// A node's text: what it says and how it is set. Its colour is a property of
/// its own, so that recolouring text does not lay it out again.
#[derive(Clone, PartialEq, Debug)]
pub struct Text {
    /// What it says.
    pub content: TextContent,
    /// The font size.
    pub size: f32,
    /// The line height.
    pub line_height: f32,
    /// The font; `None` for the default sans.
    pub font: Option<FontHandle>,
    /// The weight.
    pub weight: Weight,
    /// Where lines sit across the node's content width.
    pub align: TextAlign,
    /// Where the text sits in the node's content height.
    pub valign: Align,
    /// Whether it wraps at the content width.
    pub wrap: bool,
}

impl Text {
    /// `content` at `size` px, in the default font, wrapping.
    pub fn new(content: impl Into<TextContent>, size: f32) -> Self {
        Self {
            content: content.into(),
            size,
            line_height: (size * 1.25).ceil(),
            font: None,
            weight: Weight::NORMAL,
            align: TextAlign::default(),
            valign: Align::Start,
            wrap: true,
        }
    }

    /// On one line, however long.
    pub fn no_wrap(mut self) -> Self {
        self.wrap = false;
        self
    }

    /// Aligned `align` across and `valign` down the content box.
    pub fn align(mut self, align: TextAlign, valign: Align) -> Self {
        self.align = align;
        self.valign = valign;
        self
    }
}

/// What a custom painter draws with: a list to draw into, at the node's
/// origin, the node's size, and the frame's clock.
///
/// A painter draws in the node's own space, from `(0, 0)` to its size. It
/// must not push a screen-space clip (`push_clip_exact`) or set a text
/// block's own clip: those do not move with the node (see the design's
/// non-goals).
pub struct Paint<'a> {
    pub(crate) list: &'a mut DrawList,
    pub(crate) size: [f32; 2],
    pub(crate) now: f64,
    pub(crate) repaint: &'a mut f64,
}

impl Paint<'_> {
    /// The list to draw into.
    pub fn list(&mut self) -> &mut DrawList {
        self.list
    }

    /// The node's size.
    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    /// The frame's time, as the host gave it.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Draw this node again at `at` (the frame's clock): an animation's
    /// next step. The earliest request wins; a request lasts until the node
    /// is drawn again.
    pub fn repaint_at(&mut self, at: f64) {
        *self.repaint = self.repaint.min(at);
    }
}

/// Builds a painter's memo and drawing once its node exists, under the
/// node's owner.
pub(crate) type MakePainter = Box<dyn FnOnce(&mut Graph, Owner) -> Box<dyn CustomPaint>>;

/// A custom painter: a memo of what it draws from, and a function drawing
/// that value. It is drawn again when the memo's value changes, when its
/// size does, and at the times it asks for.
pub struct Painter {
    pub(crate) make: MakePainter,
    pub(crate) volatile: bool,
    pub(crate) overflow: Insets,
}

impl Painter {
    /// Draw `draw(value)` where `value` is what `compute` reads.
    pub fn new<T, C, D>(compute: C, draw: D) -> Self
    where
        T: PartialEq + 'static,
        C: FnMut(&mut Reader<'_>) -> T + 'static,
        D: Fn(&T, &mut Paint<'_>) + 'static,
    {
        Self {
            make: Box::new(move |graph, owner| {
                let memo = graph.memo_in(owner, compute);
                Box::new(Custom {
                    memo,
                    draw: Box::new(draw),
                })
            }),
            volatile: false,
            overflow: Insets::default(),
        }
    }

    /// Drawn on every frame: what it draws depends on something the graph
    /// does not see. Its ancestors are recorded again on every frame too.
    pub fn volatile(mut self) -> Self {
        self.volatile = true;
        self
    }

    /// It draws this far outside the node's box, so that it is not culled
    /// while that part shows.
    pub fn overflow(mut self, overflow: Insets) -> Self {
        self.overflow = overflow;
        self
    }
}

/// A painter with its value's type erased.
pub(crate) trait CustomPaint {
    fn observe(&self, graph: &mut Graph, key: u64);
    fn version(&self, graph: &mut Graph) -> u64;
    fn draw(&self, graph: &mut Graph, paint: &mut Paint<'_>);
}

type DrawFn<T> = Box<dyn Fn(&T, &mut Paint<'_>)>;

struct Custom<T> {
    memo: Memo<T>,
    draw: DrawFn<T>,
}

impl<T: PartialEq + 'static> CustomPaint for Custom<T> {
    fn observe(&self, graph: &mut Graph, key: u64) {
        graph.observe(self.memo, key);
    }

    fn version(&self, graph: &mut Graph) -> u64 {
        graph.version(self.memo)
    }

    fn draw(&self, graph: &mut Graph, paint: &mut Paint<'_>) {
        graph.with(self.memo, |value| (self.draw)(value, paint));
    }
}

/// What a node shows inside its box.
pub(crate) enum SpecContent {
    None,
    Text(Prop<Text>, Prop<[f32; 4]>),
    Custom(Painter),
}

/// A node's properties, gathered before it is added to the tree.
///
/// A node is a box: its size per axis, a stack of children with padding and
/// gap, a fill colour and a [`Look`]. It shows either children, or text, or a
/// custom painter (a node with text or a painter takes no children). It may
/// clip its children to its box, scroll them along its stack, or sit at an
/// offset in its parent instead of in the parent's stack.
pub struct Spec {
    pub(crate) width: Prop<Size>,
    pub(crate) height: Prop<Size>,
    pub(crate) stack: Prop<Stack>,
    pub(crate) fill: Prop<[f32; 4]>,
    pub(crate) look: Prop<Look>,
    pub(crate) offset: Option<Prop<[f32; 2]>>,
    pub(crate) content: SpecContent,
    pub(crate) clip: bool,
    pub(crate) scroll: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Self::new()
    }
}

impl Spec {
    /// A box that hugs its content, stacks a column, and draws nothing.
    pub fn new() -> Self {
        Self {
            width: Prop::Const(Size::Hug),
            height: Prop::Const(Size::Hug),
            stack: Prop::Const(Stack::default()),
            fill: Prop::Const([0.0; 4]),
            look: Prop::Const(Look::default()),
            offset: None,
            content: SpecContent::None,
            clip: false,
            scroll: false,
        }
    }

    /// Its width.
    pub fn width(mut self, width: impl Into<Prop<Size>>) -> Self {
        self.width = width.into();
        self
    }

    /// Its height.
    pub fn height(mut self, height: impl Into<Prop<Size>>) -> Self {
        self.height = height.into();
        self
    }

    /// How it stacks its children.
    pub fn stack(mut self, stack: impl Into<Prop<Stack>>) -> Self {
        self.stack = stack.into();
        self
    }

    /// Its fill colour (straight sRGB RGBA; transparent draws no fill).
    pub fn fill(mut self, fill: impl Into<Prop<[f32; 4]>>) -> Self {
        self.fill = fill.into();
        self
    }

    /// Its border, corners and shadow.
    pub fn look(mut self, look: impl Into<Prop<Look>>) -> Self {
        self.look = look.into();
        self
    }

    /// Placed at `offset` in its parent's content box rather than in its
    /// stack. Drawn after the stacked children, in the order added.
    pub fn absolute(mut self, offset: impl Into<Prop<[f32; 2]>>) -> Self {
        self.offset = Some(offset.into());
        self
    }

    /// Shows `text` in `color`.
    pub fn text(mut self, text: impl Into<Prop<Text>>, color: impl Into<Prop<[f32; 4]>>) -> Self {
        self.content = SpecContent::Text(text.into(), color.into());
        self
    }

    /// Shows what `painter` draws.
    pub fn custom(mut self, painter: Painter) -> Self {
        self.content = SpecContent::Custom(painter);
        self
    }

    /// Clips its children to its box; children wholly outside it are not
    /// drawn.
    pub fn clip(mut self) -> Self {
        self.clip = true;
        self
    }

    /// Scrolls its children along its stack (and clips them). Its size along
    /// the stack should be known (fixed, or a fill its parent can give).
    pub fn scroll(mut self) -> Self {
        self.clip = true;
        self.scroll = true;
        self
    }
}
