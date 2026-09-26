//! Drag list — reorderable rows (Forge `DragList`).

use crate::layer::LayerStack;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::vcentered_line_y;
use crate::{DrawList, InputState};

#[cfg(feature = "phosphor-icons")]
use crate::render::PhosphorIcon;

use super::{DragCapture, DragId, DrawContext};
use super::{glyphs, material};

/// Row height (`--h-drag-row`).
pub const DRAG_ROW_HEIGHT: f32 = 23.0;
/// Padding above the first row and below the last (`padding: 3px 0`).
const PAD_Y: f32 = 3.0;
/// Padding at the row's sides (`padding: 0 9px`).
const PAD_X: f32 = 9.0;
/// Between the grip, the glyph and the label.
const GAP: f32 = 7.0;
/// The grip: a 2 × 3 dot grid, like `⠿` at 10 px.
const GRIP_DOT: f32 = 2.0;
const GRIP_PITCH: f32 = 3.0;
const GRIP_GRID: (u32, u32) = (2, 3);
/// The glyph column's width, and the default half-filled square's side
/// (`◧` at 10 px).
const GLYPH: f32 = 10.0;
const HALF_SQUARE: f32 = 7.0;
/// The dragged row's opacity while it is away.
const SOURCE_ALPHA: f32 = 0.35;
/// The insertion line: inset from the well's sides, thickness, and glow.
const LINE_INSET: f32 = 4.0;
const LINE_H: f32 = 2.0;
const LINE_GLOW_BLUR: f32 = 7.0;
const LINE_GLOW_ALPHA: f32 = 0.7;
/// The ghost: offset from the pointer, padding, fill, edge and shadows
/// (`--shadow-ghost`).
const GHOST_OFFSET: [f32; 2] = [12.0, 10.0];
const GHOST_PAD: (f32, f32) = (8.0, 3.0);
const GHOST_FILL: [f32; 4] = [31.0 / 255.0, 36.0 / 255.0, 41.0 / 255.0, 0.92];
const GHOST_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.7];
const GHOST_SHADOW: BoxShadow = BoxShadow {
    offset: [0.0, 8.0],
    blur: 20.0,
    spread: 0.0,
    color: [0.0, 0.0, 0.0, 0.6],
    inset: false,
};
const GHOST_HIGHLIGHT: BoxShadow = BoxShadow {
    offset: [0.0, 1.0],
    blur: 0.0,
    spread: 0.0,
    color: [1.0, 1.0, 1.0, 0.12],
    inset: true,
};
/// How far the pointer must move from the press before a slot is chosen:
/// until then, letting go is a click and moves nothing.
const MOVE_THRESHOLD: f32 = 3.0;

/// One row of a [`DragList`], borrowed from the caller's data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragItem<'a> {
    /// The label.
    pub label: &'a str,
    /// The glyph before the label. `None` draws Forge's default, a
    /// half-filled square.
    #[cfg(feature = "phosphor-icons")]
    pub icon: Option<PhosphorIcon>,
}

impl<'a> DragItem<'a> {
    /// A row showing `label` after the default glyph.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            #[cfg(feature = "phosphor-icons")]
            icon: None,
        }
    }

    /// Show `icon` instead of the default glyph.
    #[cfg(feature = "phosphor-icons")]
    pub fn icon(mut self, icon: PhosphorIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A finished reorder: the row at `from` now belongs at `to` (both indices
/// into the list as it was before the move).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragMove {
    /// Where the row was.
    pub from: usize,
    /// Where it goes: its index once the move is applied.
    pub to: usize,
}

impl DragMove {
    /// Apply the move to `items`, the data the list was drawn from. Out of
    /// range indices (the data changed under the drag) are ignored.
    pub fn apply<T>(&self, items: &mut [T]) {
        let (from, to) = (self.from, self.to);
        if from >= items.len() || to >= items.len() {
            return;
        }
        if from < to {
            items[from..=to].rotate_left(1);
        } else {
            items[to..=from].rotate_right(1);
        }
    }
}

/// The row being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    /// Its index.
    from: usize,
    /// Where the pointer went down.
    press: [f32; 2],
    /// The insertion slot, `0..=len`; `None` until the pointer has moved.
    slot: Option<usize>,
}

/// Caller-owned [`DragList`] state: the drag in progress, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DragListState {
    held: Option<Held>,
}

impl DragListState {
    /// No drag in progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// The index of the row being dragged.
    pub fn dragging(&self) -> Option<usize> {
        self.held.map(|h| h.from)
    }

    /// The insertion slot the dragged row would land in (`0` is above the
    /// first row, `len` below the last), once the pointer has moved.
    pub fn slot(&self) -> Option<usize> {
        self.held.and_then(|h| h.slot)
    }
}

/// What a [`DragList`] frame reported.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DragListOutput {
    /// The reorder finished this frame (the row was let go in a new place).
    pub moved: Option<DragMove>,
    /// A row is being dragged.
    pub dragging: bool,
}

/// Reorderable rows (Forge `DragList`).
///
/// Press a row and drag it: the row dims in place, a 2 px accent line with a
/// soft glow marks the slot it will land in, and a ghost of its label follows
/// the pointer. Letting go reports a [`DragMove`]; the caller applies it to
/// its own data (see [`DragMove::apply`]). A press that never moves reorders
/// nothing.
///
/// Persistent state is caller-owned: a [`DragListState`] per list, and the
/// surface's shared [`DragCapture`] so the drag can't fight a slider or
/// splitter for the same pointer.
///
/// The ghost goes above everything else, so it is drawn separately, after
/// the rest of the UI: [`draw_ghost_layer`](Self::draw_ghost_layer) puts it
/// on a tooltip layer of a [`LayerStack`] (visual only, never takes input),
/// and [`draw_ghost`](Self::draw_ghost) paints it into any draw list.
///
/// ```ignore
/// let items: Vec<DragItem> = layers.iter().map(|l| DragItem::new(&l.name)).collect();
/// let list = DragList::new(&items);
/// let rect = Rect::new(x, y, 220.0, DragList::height(items.len(), &styles));
/// let out = list.draw(LAYERS_ID, rect, &mut drag_state, &mut capture, &mut ctx);
/// list.draw_ghost_layer(&mut layer_stack, &drag_state, &styles, &input);
/// if let Some(m) = out.moved {
///     m.apply(&mut layers);
/// }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct DragList<'a> {
    items: &'a [DragItem<'a>],
}

impl<'a> DragList<'a> {
    /// A list of `items`, in order.
    pub fn new(items: &'a [DragItem<'a>]) -> Self {
        Self { items }
    }

    /// The height of a list of `rows` rows, border included.
    pub fn height(rows: usize, s: &StyleResolver) -> f32 {
        2.0 * s.scalar(StyleKey::BorderWidth) + 2.0 * PAD_Y + rows as f32 * DRAG_ROW_HEIGHT
    }

    /// The top of the first row in a list drawn at `rect`.
    fn rows_top(rect: Rect, s: &StyleResolver) -> f32 {
        rect.y + s.scalar(StyleKey::BorderWidth) + PAD_Y
    }

    /// The insertion slot nearest the pointer's `y`.
    fn slot_at(&self, rect: Rect, y: f32, s: &StyleResolver) -> usize {
        let t = (y - Self::rows_top(rect, s)) / DRAG_ROW_HEIGHT;
        (t.round().max(0.0) as usize).min(self.items.len())
    }

    /// The row under `(x, y)`, if any.
    fn row_at(&self, rect: Rect, x: f32, y: f32, s: &StyleResolver) -> Option<usize> {
        let border = s.scalar(StyleKey::BorderWidth);
        let top = Self::rows_top(rect, s);
        let inside = x >= rect.x + border && x < rect.right() - border && y >= top;
        let i = ((y - top) / DRAG_ROW_HEIGHT).floor() as usize;
        (inside && i < self.items.len()).then_some(i)
    }

    /// Draw the list into `rect` and run the drag. `id` is the list's
    /// [`DragId`] in the surface's `capture`.
    pub fn draw(
        &self,
        id: DragId,
        rect: Rect,
        state: &mut DragListState,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> DragListOutput {
        ctx.push_debug_scope_rect("DragList", material::deep_well_ink(rect));
        let s = ctx.styles();
        let input = ctx.input;
        let mut out = DragListOutput::default();

        // A drag this list no longer owns (another widget cleared the
        // capture), or rows that vanished under it, ends without a move.
        if state
            .held
            .is_some_and(|h| !capture.is_active(id) || h.from >= self.items.len())
        {
            state.held = None;
            capture.release(id);
        }

        if let Some(held) = &mut state.held {
            let moved = (input.mouse_x - held.press[0]).hypot(input.mouse_y - held.press[1]);
            if held.slot.is_some() || moved >= MOVE_THRESHOLD {
                held.slot = Some(self.slot_at(rect, input.mouse_y, &s));
            }
            if !input.mouse_down {
                if let Some(slot) = held.slot {
                    let to = if slot > held.from { slot - 1 } else { slot };
                    if to != held.from {
                        out.moved = Some(DragMove {
                            from: held.from,
                            to,
                        });
                    }
                }
                state.held = None;
                capture.release(id);
            }
        } else if input.mouse_clicked && !input.mouse_consumed && capture.is_free() {
            if let Some(from) = self.row_at(rect, input.mouse_x, input.mouse_y, &s) {
                capture.try_begin(id);
                state.held = Some(Held {
                    from,
                    press: [input.mouse_x, input.mouse_y],
                    slot: None,
                });
            }
        }
        out.dragging = state.held.is_some();

        let list = &mut *ctx.draw_list;
        let inner = material::draw_deep_well(list, &s, rect);
        // The well is a viewport: rows past its height, and the insertion
        // line's glow, are cut at its edge on purpose.
        list.push_clip_viewport(inner);
        let top = Self::rows_top(rect, &s);
        let source = state.dragging();
        for (i, item) in self.items.iter().enumerate() {
            let row = Rect::new(
                inner.x,
                top + i as f32 * DRAG_ROW_HEIGHT,
                inner.width,
                DRAG_ROW_HEIGHT,
            );
            if row.y >= inner.bottom() {
                break;
            }
            let dimmed = source == Some(i);
            if dimmed {
                list.push_tint();
                list.multiply_tint([1.0, 1.0, 1.0, SOURCE_ALPHA]);
            }
            Self::draw_row(item, row, list, &s);
            if dimmed {
                list.pop_tint();
            }
        }
        if let Some(slot) = state.slot() {
            Self::draw_insertion(
                Rect::new(
                    inner.x + LINE_INSET,
                    top + slot as f32 * DRAG_ROW_HEIGHT - LINE_H * 0.5,
                    (inner.width - 2.0 * LINE_INSET).max(0.0),
                    LINE_H,
                ),
                list,
                &s,
            );
        }
        list.pop_clip();
        ctx.pop_debug_scope();
        out
    }

    /// One row's grip, glyph and label.
    fn draw_row(item: &DragItem, row: Rect, list: &mut DrawList, s: &StyleResolver) {
        let cy = row.y + row.height * 0.5;
        let mut x = row.x + PAD_X;
        let grip = list.dot_grid(
            [x, cy - grip_span(GRIP_GRID.1) * 0.5],
            GRIP_GRID,
            GRIP_DOT,
            GRIP_PITCH,
            s.ink(Ink::DisabledGlyph),
        );
        x = grip.right() + GAP;

        let glyph = Rect::new(x, cy - GLYPH * 0.5, GLYPH, GLYPH);
        let muted = s.ink(Ink::Muted);
        #[cfg(feature = "phosphor-icons")]
        let drew_icon = item.icon.map(|icon| list.phosphor_icon(glyph, icon, muted));
        #[cfg(not(feature = "phosphor-icons"))]
        let drew_icon: Option<()> = None;
        if drew_icon.is_none() {
            glyphs::half_square(list, glyph, HALF_SQUARE, muted);
        }
        x = glyph.right() + GAP;

        let size = s.text_size(TextSize::Menu);
        let label_w = (row.right() - PAD_X - x).max(0.0);
        list.text(
            s.sans_block(
                item.label,
                x,
                vcentered_line_y(row.y, row.height, size),
                TextSize::Menu,
                Ink::Title,
            )
            .with_max_width(label_w)
            .with_ellipsis(),
        );
    }

    /// The accent line where the dragged row would land, with its glow.
    fn draw_insertion(line: Rect, list: &mut DrawList, s: &StyleResolver) {
        let mut glow = s.color(StyleKey::Accent);
        glow[3] = LINE_GLOW_ALPHA;
        list.box_shadow_outset(
            line,
            CornerRadii::default(),
            BoxShadow {
                blur: LINE_GLOW_BLUR,
                color: glow,
                ..BoxShadow::default()
            },
        );
        list.quad(
            line.x,
            line.y,
            line.width,
            line.height,
            s.color(StyleKey::AccentTick),
        );
    }

    /// Paint the ghost of the dragged row at the pointer into `list` (draw
    /// it last, so it sits over everything). Returns its box, or `None` when
    /// nothing is being dragged.
    pub fn draw_ghost(
        &self,
        list: &mut DrawList,
        state: &DragListState,
        s: &StyleResolver,
        input: &InputState,
    ) -> Option<Rect> {
        let item = self.items.get(state.dragging()?)?;
        let mut block = s.sans_block(item.label, 0.0, 0.0, TextSize::Row, Ink::Value);
        let (text_w, line_h) = list.measure_block(&block);
        let size = s.text_size(TextSize::Row);
        let border = s.scalar(StyleKey::BorderWidth);
        let rect = Rect::new(
            input.mouse_x + GHOST_OFFSET[0],
            input.mouse_y + GHOST_OFFSET[1],
            (text_w + 2.0 * (GHOST_PAD.0 + border)).ceil(),
            (line_h + 2.0 * (GHOST_PAD.1 + border)).ceil(),
        );
        let radius = s.scalar(StyleKey::BorderRadius);
        list.push_debug_scope_rect("DragList ghost", rect.union(GHOST_SHADOW.ink_rect(rect)));
        list.box_shadow_outset(rect, CornerRadii::uniform(radius), GHOST_SHADOW);
        list.chrome_rect(rect, radius, border, GHOST_FILL, GHOST_EDGE);
        let inner = rect.inset(border);
        list.box_shadow_inset(
            inner,
            CornerRadii::uniform((radius - border).max(0.0)),
            GHOST_HIGHLIGHT,
        );
        block.x = inner.x + GHOST_PAD.0;
        block.y = vcentered_line_y(inner.y, inner.height, size);
        list.text(block);
        list.pop_debug_scope();
        Some(rect)
    }

    /// [`draw_ghost`](Self::draw_ghost) onto a fresh tooltip layer of
    /// `layers`: above every other layer pushed so far, and never taking
    /// input.
    pub fn draw_ghost_layer(
        &self,
        layers: &mut LayerStack,
        state: &DragListState,
        s: &StyleResolver,
        input: &InputState,
    ) -> Option<Rect> {
        state.dragging()?;
        let index = layers.push_tooltip(Rect::default());
        let rect = self.draw_ghost(layers.current_mut(), state, s, input);
        if let Some(rect) = rect {
            layers.layers_mut()[index].rect = rect;
        }
        layers.pop_layer();
        rect
    }
}

/// The height (or width) of `n` grip dots.
fn grip_span(n: u32) -> f32 {
    (n.max(1) - 1) as f32 * GRIP_PITCH + GRIP_DOT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, LayerKind, Theme};

    const ID: DragId = 4;
    const LABELS: [&str; 4] = ["Sky", "Hills", "Trees", "Player"];

    fn items() -> Vec<DragItem<'static>> {
        LABELS.iter().map(|l| DragItem::new(l)).collect()
    }

    fn rect() -> Rect {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        Rect::new(10.0, 20.0, 200.0, DragList::height(LABELS.len(), &s))
    }

    /// The vertical centre of row `i` in [`rect`].
    fn row_y(i: usize) -> f32 {
        20.0 + 1.0 + PAD_Y + (i as f32 + 0.5) * DRAG_ROW_HEIGHT
    }

    fn at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn press(x: f32, y: f32) -> InputState {
        InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..at(x, y)
        }
    }

    fn hold(x: f32, y: f32) -> InputState {
        InputState {
            mouse_down: true,
            ..at(x, y)
        }
    }

    fn frame(
        state: &mut DragListState,
        capture: &mut DragCapture,
        input: &InputState,
    ) -> (DragListOutput, DrawList) {
        let items = items();
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            DragList::new(&items).draw(ID, rect(), state, capture, &mut ctx)
        };
        (out, list)
    }

    /// Press row `from`, move to `y`, and let go there.
    fn drag(from: usize, y: f32) -> Option<DragMove> {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        frame(&mut state, &mut capture, &press(50.0, row_y(from)));
        frame(&mut state, &mut capture, &hold(50.0, y));
        let (out, _) = frame(&mut state, &mut capture, &at(50.0, y));
        assert!(capture.is_free(), "letting go frees the capture");
        assert_eq!(state.dragging(), None);
        out.moved
    }

    #[test]
    fn height_is_rows_plus_padding_and_border() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        assert_eq!(DragList::height(0, &s), 2.0 + 6.0);
        assert_eq!(DragList::height(4, &s), 2.0 + 6.0 + 4.0 * 23.0);
    }

    #[test]
    fn dragging_down_moves_the_row_below_its_neighbours() {
        // Row 0 let go on the boundary under row 2 (slot 3) lands at index 2.
        let slot3 = row_y(2) + DRAG_ROW_HEIGHT * 0.5;
        assert_eq!(drag(0, slot3), Some(DragMove { from: 0, to: 2 }));
        // Past the last row it clamps to the end.
        assert_eq!(drag(1, 500.0), Some(DragMove { from: 1, to: 3 }));
    }

    #[test]
    fn dragging_up_moves_the_row_above() {
        let slot0 = row_y(0) - DRAG_ROW_HEIGHT * 0.4;
        assert_eq!(drag(3, slot0), Some(DragMove { from: 3, to: 0 }));
        assert_eq!(drag(2, -50.0), Some(DragMove { from: 2, to: 0 }));
    }

    #[test]
    fn letting_go_either_side_of_itself_moves_nothing() {
        let above = row_y(2) - DRAG_ROW_HEIGHT * 0.5;
        let below = row_y(2) + DRAG_ROW_HEIGHT * 0.5;
        assert_eq!(drag(2, above), None);
        assert_eq!(drag(2, below), None);
    }

    #[test]
    fn a_click_without_moving_picks_no_slot_and_moves_nothing() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        let (out, _) = frame(&mut state, &mut capture, &press(50.0, row_y(1)));
        assert!(out.dragging);
        assert_eq!(state.dragging(), Some(1));
        assert_eq!(state.slot(), None, "no slot until the pointer moves");
        let (out, _) = frame(&mut state, &mut capture, &at(50.0, row_y(1) + 1.0));
        assert_eq!(out.moved, None);
        assert!(!out.dragging);
    }

    #[test]
    fn a_consumed_press_starts_nothing() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        let input = InputState {
            mouse_consumed: true,
            ..press(50.0, row_y(1))
        };
        let (out, _) = frame(&mut state, &mut capture, &input);
        assert!(!out.dragging);
        assert!(capture.is_free());
    }

    #[test]
    fn a_press_while_another_widget_drags_starts_nothing() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        assert!(capture.try_begin(99));
        let (out, _) = frame(&mut state, &mut capture, &press(50.0, row_y(1)));
        assert!(!out.dragging);
        assert!(capture.is_active(99));
    }

    #[test]
    fn a_press_in_the_padding_or_outside_starts_nothing() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        for (x, y) in [(50.0, 22.0), (5.0, row_y(0)), (50.0, row_y(4))] {
            let (out, _) = frame(&mut state, &mut capture, &press(x, y));
            assert!(!out.dragging, "press at ({x}, {y})");
        }
    }

    #[test]
    fn losing_the_capture_ends_the_drag_without_a_move() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        frame(&mut state, &mut capture, &press(50.0, row_y(0)));
        frame(&mut state, &mut capture, &hold(50.0, 500.0));
        capture.clear();
        let (out, _) = frame(&mut state, &mut capture, &at(50.0, 500.0));
        assert_eq!(out.moved, None);
        assert_eq!(state.dragging(), None);
    }

    #[test]
    fn the_insertion_line_sits_on_the_slot_boundary() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        frame(&mut state, &mut capture, &press(50.0, row_y(0)));
        let (_, list) = frame(&mut state, &mut capture, &hold(50.0, row_y(1) + 9.0));
        assert_eq!(state.slot(), Some(2));
        let theme = Theme::default();
        let accent = StyleResolver::new(&theme).color(StyleKey::AccentTick);
        let line = list
            .chrome_instances()
            .find(|c| c.bg == accent)
            .expect("an accent line is drawn");
        let top = 20.0 + 1.0 + PAD_Y + 2.0 * DRAG_ROW_HEIGHT;
        assert_eq!(
            line.rect,
            [10.0 + 1.0 + 4.0, top - 1.0, 200.0 - 2.0 - 8.0, 2.0]
        );
    }

    #[test]
    fn no_line_before_the_pointer_moves() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        let (_, list) = frame(&mut state, &mut capture, &press(50.0, row_y(0)));
        let theme = Theme::default();
        let accent = StyleResolver::new(&theme).color(StyleKey::AccentTick);
        assert!(list.chrome_instances().all(|c| c.bg != accent));
    }

    #[test]
    fn the_dragged_row_dims_and_the_others_do_not() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        let (_, list) = frame(&mut state, &mut capture, &press(50.0, row_y(1)));
        let alpha = |label: &str| {
            let t = list.texts.iter().find(|t| t.content == label).unwrap();
            t.color.a() as f32 / 255.0
        };
        assert!((alpha("Hills") - alpha("Sky") * SOURCE_ALPHA).abs() < 0.01);
        assert_eq!(alpha("Trees"), alpha("Sky"));
    }

    #[test]
    fn labels_sit_after_the_grip_and_glyph_in_row_order() {
        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        let (_, list) = frame(&mut state, &mut capture, &at(-10.0, -10.0));
        let ys: Vec<f32> = LABELS
            .iter()
            .map(|l| list.texts.iter().find(|t| t.content == *l).unwrap().y)
            .collect();
        assert!(
            ys.windows(2)
                .all(|w| (w[1] - w[0] - DRAG_ROW_HEIGHT).abs() < 0.01)
        );
        let x = list.texts[0].x;
        assert_eq!(x, 10.0 + 1.0 + PAD_X + grip_span(2) + GAP + GLYPH + GAP);
    }

    #[test]
    fn a_long_list_draws_only_the_rows_that_fit() {
        let labels: Vec<String> = (0..100_000).map(|i| format!("Row {i}")).collect();
        let items: Vec<DragItem> = labels.iter().map(|l| DragItem::new(l)).collect();
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = at(-10.0, -10.0);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        // Room for ten rows and a bit of an eleventh.
        let r = Rect::new(0.0, 0.0, 200.0, 2.0 + 6.0 + 10.5 * DRAG_ROW_HEIGHT);
        DragList::new(&items).draw(
            ID,
            r,
            &mut DragListState::new(),
            &mut DragCapture::new(),
            &mut ctx,
        );
        assert_eq!(list.texts.len(), 11, "the cut-off row still draws");
    }

    #[test]
    fn apply_moves_the_item_and_shifts_the_rest() {
        let mut v = [0, 1, 2, 3];
        DragMove { from: 0, to: 2 }.apply(&mut v);
        assert_eq!(v, [1, 2, 0, 3]);
        DragMove { from: 3, to: 1 }.apply(&mut v);
        assert_eq!(v, [1, 3, 2, 0]);
        DragMove { from: 1, to: 9 }.apply(&mut v);
        assert_eq!(v, [1, 3, 2, 0], "out of range is ignored");
    }

    #[test]
    fn the_ghost_follows_the_pointer_only_while_dragging() {
        let items = items();
        let list_w = DragList::new(&items);
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let idle = DragListState::new();
        assert_eq!(list_w.draw_ghost(&mut list, &idle, &s, &at(0.0, 0.0)), None);
        assert!(list.texts.is_empty());

        let mut state = DragListState::new();
        let mut capture = DragCapture::new();
        frame(&mut state, &mut capture, &press(50.0, row_y(2)));
        let ghost = list_w
            .draw_ghost(&mut list, &state, &s, &hold(300.0, 200.0))
            .unwrap();
        assert_eq!((ghost.x, ghost.y), (312.0, 210.0));
        let label = list.texts.iter().find(|t| t.content == "Trees").unwrap();
        assert!(ghost.contains(label.x, label.y + 1.0));
    }

    #[test]
    fn the_ghost_layer_is_a_tooltip_that_never_takes_input() {
        let items = items();
        let list_w = DragList::new(&items);
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut layers = LayerStack::new();
        let mut state = DragListState::new();
        assert_eq!(
            list_w.draw_ghost_layer(&mut layers, &state, &s, &at(0.0, 0.0)),
            None
        );
        assert!(layers.layers().is_empty(), "no layer while idle");

        let mut capture = DragCapture::new();
        frame(&mut state, &mut capture, &press(50.0, row_y(0)));
        let input = hold(100.0, 100.0);
        let ghost = list_w
            .draw_ghost_layer(&mut layers, &state, &s, &input)
            .unwrap();
        let layer = &layers.layers()[0];
        assert!(matches!(layer.kind, LayerKind::Tooltip));
        assert_eq!(layer.rect, ghost);
        assert!(!layers.has_active_layer(), "the layer is popped");
        assert!(!layers.input_for_base(&input).mouse_consumed);
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn an_icon_replaces_the_default_glyph() {
        let items = [DragItem::new("Sky").icon(PhosphorIcon::Check)];
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let input = at(-10.0, -10.0);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
        let r = Rect::new(0.0, 0.0, 200.0, 40.0);
        DragList::new(&items).draw(
            ID,
            r,
            &mut DragListState::new(),
            &mut DragCapture::new(),
            &mut ctx,
        );
        assert_eq!(list.icons_msdf.len(), 1);
    }
}
