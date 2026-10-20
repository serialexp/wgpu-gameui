//! Z-ordered layer stack for modals, popups, and tooltips.
//!
//! A `LayerStack` owns a base `DrawList` plus an ordered queue of additional
//! `Layer` entries pushed during a frame. Each `Layer` has a kind (Modal,
//! Popup, Tooltip) and its own `DrawList`. The renderer consumes them in
//! order: base first, then each pushed layer in push order.
//!
//! ```ignore
//! let mut layers = LayerStack::with_font_system(font_system);
//!
//! // 1. Push every layer (base + overlays) onto the stack first so we know
//! //    the FULL z-order before resolving input. Drawing into them can
//! //    happen in any order — only the eventual `render_layers` call cares
//! //    about render order, and it walks `layers.layers()` itself.
//! //
//! //    A common pattern: push the modal early (with its rect), draw into
//! //    the base layer with the dispatched input, then come back and draw
//! //    into the modal layer with its own dispatched input.
//!
//! let modal_idx = if modal_open {
//!     Some(layers.push_modal(modal_rect))
//! } else {
//!     None
//! };
//!
//! // 2. Dispatch input. `input_for_base` returns a clone of the raw input
//! //    where `mouse_consumed = true` if any open layer above is gobbling
//! //    the cursor — base widgets that read this clone won't fire while
//! //    the modal is up.
//! let base_input = layers.input_for_base(&raw_input);
//! draw_base_widgets(layers.base_mut(), &base_input);
//!
//! // 3. Each higher layer gets its own dispatch view via `input_for_layer`.
//! //    Layers above it (if any) can still consume input on top of it.
//! if let Some(idx) = modal_idx {
//!     let modal_input = layers.input_for_layer(idx, &raw_input);
//!     draw_modal_contents(&mut layers.layers_mut()[idx].list, &modal_input);
//!     layers.pop_layer();
//! }
//!
//! // 4. Render. `begin_frame` opens the frame: one submission = one frame.
//! ui_renderer.begin_frame();
//! ui_renderer.render_layers(..., &layers);
//! ```
//!
//! ## Input gobbling
//!
//! Modal layers cover all lower layers — when the mouse is anywhere on screen
//! and a modal is open, lower layers see `InputState::mouse_consumed = true`
//! unless the cursor is over *no* modal at all (in which case the modal
//! itself captures the click for "click-outside-to-close" semantics).
//!
//! Popups behave like modals but only block input within their own bounding
//! rect; clicks outside the popup pass through to lower layers.
//!
//! Tooltips never block input — they're purely visual.
//!
//! ## A press stays where it began
//!
//! A press belongs to the layer it landed on until the button is released.
//! Until then every layer below that one sees the pointer as consumed, even
//! once that layer has closed: a click that closes a dialog does not press
//! the button under it as the button is held. The stack keeps this across
//! [`clear`](LayerStack::clear), so reuse one stack from frame to frame (as
//! the renderer expects anyway); a stack made new each frame forgets it.
//! The layer is remembered by its place in the stack, so one pushed into
//! the same place while the button is still held gets the press as usual.

use std::cell::Cell;

use crate::InputState;
use crate::layout::Rect;
use crate::text::FontSystemHandle;
use crate::widgets::DrawList;

/// What kind of overlay this layer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// Always-on-top, no input passes through to lower layers.
    Modal,
    /// Blocks input only within its own rect (e.g. dropdown menu).
    Popup,
    /// Visual only — never blocks input.
    Tooltip,
}

/// A single overlay layer.
pub struct Layer {
    /// Overlay behaviour (input blocking) for this layer.
    pub kind: LayerKind,
    /// Bounding rect used for popup hit-testing. For modals this is the modal
    /// dialog rect — clicks *outside* it route to the modal layer (which can
    /// implement click-outside-to-close), not to lower layers. For popups,
    /// clicks inside it stay in the popup; clicks outside fall through.
    pub rect: Rect,
    /// Draw commands accumulated for this layer.
    pub list: DrawList,
}

/// Ordered collection of layers for one frame.
pub struct LayerStack {
    base: DrawList,
    layers: Vec<Layer>,
    /// Stack of indices into `layers`, recording the active layer when nested
    /// `push_*` calls happened. Top of the stack is the layer current draw
    /// commands route to. Empty = base list is current.
    active: Vec<usize>,
    font_system: Option<FontSystemHandle>,
    /// The text selection every list paints (see
    /// [`set_text_highlight`](Self::set_text_highlight)).
    text_highlight: Option<crate::TextHighlight>,
    /// The depth (0 the base, `i + 1` layer `i`) the held press landed on,
    /// from its press frame until the frame after its release. Kept across
    /// [`clear`](Self::clear); set while resolving input, which takes `&self`.
    press: Cell<Option<usize>>,
    /// The depth that takes the pointer at a point, for this frame's layers:
    /// every layer's input resolves against it, so a frame scans the stack
    /// once however many layers ask. Forgotten when a layer is pushed.
    top: Cell<Option<([f32; 2], usize)>>,
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerStack {
    /// Create an empty layer stack with a default (font-less) base list.
    pub fn new() -> Self {
        Self {
            base: DrawList::new(),
            layers: Vec::new(),
            active: Vec::new(),
            font_system: None,
            text_highlight: None,
            press: Cell::new(None),
            top: Cell::new(None),
        }
    }

    /// Create an empty layer stack whose base list (and future layers) share
    /// the given font system for text shaping.
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self {
            base: DrawList::with_font_system(font_system.clone()),
            layers: Vec::new(),
            active: Vec::new(),
            font_system: Some(font_system),
            text_highlight: None,
            press: Cell::new(None),
            top: Cell::new(None),
        }
    }

    /// Reset all layers but keep allocated capacity, and the layer a held
    /// press began on.
    pub fn clear(&mut self) {
        self.base.clear();
        self.layers.clear();
        self.active.clear();
        self.top.set(None);
    }

    /// The earliest [`DrawList::repaint_after`] any layer was asked for this
    /// frame, in seconds from when it was drawn: when the host has to build
    /// the next frame though nothing else happens.
    pub fn next_repaint(&self) -> Option<f32> {
        std::iter::once(&self.base)
            .chain(self.layers.iter().map(|layer| &layer.list))
            .filter_map(DrawList::next_repaint)
            .reduce(f32::min)
    }

    /// Borrow the base draw list (lowest layer).
    pub fn base(&self) -> &DrawList {
        &self.base
    }

    /// The base draw list's unique id ([`DrawList::id`]). The renderer keys its
    /// "fresh stack every frame" footgun detector on this: a reused stack reports
    /// the same id each frame, a per-frame-`new()`ed one reports a different id.
    pub fn base_id(&self) -> u64 {
        self.base.id()
    }

    /// Mutably borrow the base draw list (lowest layer).
    pub fn base_mut(&mut self) -> &mut DrawList {
        &mut self.base
    }

    /// All layers, in render order.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Mutable access to all layers (in render order). Useful for drawing
    /// into a specific layer's `list` after dispatching `input_for_layer`.
    pub fn layers_mut(&mut self) -> &mut [Layer] {
        // A caller may move or retype a layer through this.
        self.top.set(None);
        &mut self.layers
    }

    /// The draw list that primitive calls should currently target — the
    /// most-recently-pushed layer, or the base list if no layer is active.
    pub fn current_mut(&mut self) -> &mut DrawList {
        match self.active.last().copied() {
            Some(idx) => &mut self.layers[idx].list,
            None => &mut self.base,
        }
    }

    fn make_list(&self) -> DrawList {
        let mut list = match &self.font_system {
            Some(fs) => DrawList::with_font_system(fs.clone()),
            None => DrawList::new(),
        };
        list.set_text_highlight(self.text_highlight);
        list
    }

    /// Paint `highlight` behind selected text in the base list and every layer,
    /// including ones pushed later: set it each frame, before drawing, from
    /// [`TextSelection::highlight`](crate::TextSelection::highlight).
    pub fn set_text_highlight(&mut self, highlight: Option<crate::TextHighlight>) {
        self.text_highlight = highlight;
        self.base.set_text_highlight(highlight);
        for layer in &mut self.layers {
            layer.list.set_text_highlight(highlight);
        }
    }

    /// Open a modal layer covering `rect` (typically the modal dialog rect).
    /// Returns the layer index for the caller's records.
    pub fn push_modal(&mut self, rect: Rect) -> usize {
        let list = self.make_list();
        self.layers.push(Layer {
            kind: LayerKind::Modal,
            rect,
            list,
        });
        let idx = self.layers.len() - 1;
        self.active.push(idx);
        self.top.set(None);
        idx
    }

    /// Open a popup layer with the given bounding rect.
    pub fn push_popup(&mut self, rect: Rect) -> usize {
        let list = self.make_list();
        self.layers.push(Layer {
            kind: LayerKind::Popup,
            rect,
            list,
        });
        let idx = self.layers.len() - 1;
        self.active.push(idx);
        self.top.set(None);
        idx
    }

    /// Open a tooltip layer (purely visual).
    pub fn push_tooltip(&mut self, rect: Rect) -> usize {
        let list = self.make_list();
        self.layers.push(Layer {
            kind: LayerKind::Tooltip,
            rect,
            list,
        });
        let idx = self.layers.len() - 1;
        self.active.push(idx);
        self.top.set(None);
        idx
    }

    /// Pop the most recently opened layer.
    pub fn pop_layer(&mut self) {
        debug_assert!(
            !self.active.is_empty(),
            "LayerStack::pop_layer called with no active layer"
        );
        self.active.pop();
    }

    /// Returns whether any layer is currently active (i.e. inside a
    /// push/pop block). Useful for debug asserts at end-of-frame.
    pub fn has_active_layer(&self) -> bool {
        !self.active.is_empty()
    }

    /// Resolve the input state that a layer at `index` should see, taking
    /// into account higher-z modals/popups that may have captured the cursor.
    ///
    /// A higher-z **modal** always consumes input for everything below it. A
    /// higher-z **popup** consumes input only when the cursor is inside its
    /// rect. **Tooltips** never consume input. And while the button is held,
    /// the layers below the one the press landed on see it consumed (see
    /// [the module docs](self)).
    pub fn input_for_layer(&self, index: usize, base: &InputState) -> InputState {
        self.input_at_depth(index + 1, base)
    }

    /// Resolve the input state the base layer should see.
    pub fn input_for_base(&self, base: &InputState) -> InputState {
        self.input_at_depth(0, base)
    }

    fn input_at_depth(&self, depth: usize, base: &InputState) -> InputState {
        if depth < self.reach(base) {
            base.consumed()
        } else {
            base.clone()
        }
    }

    /// The lowest depth `input` reaches: the one that takes the pointer
    /// now, or the one its held press landed on if that is higher. Updates
    /// the press: a press edge lands on what takes the pointer, and a frame
    /// with the button up (after its release frame) lets it go. Input a
    /// layer already consumed has lost its edges, so it leaves the press be.
    fn reach(&self, input: &InputState) -> usize {
        let top = self.top_at(input.mouse_x, input.mouse_y);
        let press = if input.mouse_consumed {
            self.press.get()
        } else if input.mouse_clicked {
            Some(top)
        } else if input.mouse_down || input.mouse_released {
            self.press.get()
        } else {
            None
        };
        self.press.set(press);
        press.map_or(top, |press| press.max(top))
    }

    /// The depth that takes the pointer at (`x`, `y`): the topmost modal or
    /// popup holding the point, or the base (0).
    fn top_at(&self, x: f32, y: f32) -> usize {
        if let Some((at, depth)) = self.top.get()
            && at == [x, y]
        {
            return depth;
        }
        let depth = self
            .layers
            .iter()
            .rposition(|layer| match layer.kind {
                LayerKind::Modal => true,
                LayerKind::Popup => layer.rect.contains(x, y),
                LayerKind::Tooltip => false,
            })
            .map_or(0, |index| index + 1);
        self.top.set(Some(([x, y], depth)));
        depth
    }

    /// Whether a layer above the base keeps the pointer at (`x`, `y`) from
    /// it: any modal, or a popup whose rect holds the point. What
    /// [`input_for_base`](Self::input_for_base) decides for a new press, for
    /// a host asking between frames about one the base layer would act on
    /// there and then (the window's title bar starting a drag).
    pub fn blocks_base_at(&self, x: f32, y: f32) -> bool {
        self.top_at(x, y) > 0
    }
}

impl Drop for LayerStack {
    fn drop(&mut self) {
        debug_assert!(
            self.active.is_empty(),
            "LayerStack dropped with {} unbalanced push/pop pair(s)",
            self.active.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_clicked: true,
            mouse_down: true,
            scroll_delta: 1.0,
            ..InputState::default()
        }
    }

    #[test]
    fn the_next_repaint_is_the_earliest_any_layer_asked_for() {
        let mut s = LayerStack::new();
        assert_eq!(s.next_repaint(), None);
        s.base_mut().repaint_after(0.5);
        s.push_tooltip(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.current_mut().repaint_after(0.2);
        s.pop_layer();
        assert_eq!(s.next_repaint(), Some(0.2));
        s.clear();
        assert_eq!(
            s.next_repaint(),
            None,
            "a request lasts the frame that drew it"
        );
    }

    #[test]
    fn the_base_is_blocked_under_a_popup_and_everywhere_under_a_modal() {
        let mut s = LayerStack::new();
        assert!(!s.blocks_base_at(5.0, 5.0));
        s.push_tooltip(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.pop_layer();
        assert!(!s.blocks_base_at(5.0, 5.0), "a tooltip blocks nothing");
        s.push_popup(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.pop_layer();
        assert!(s.blocks_base_at(5.0, 5.0));
        assert!(!s.blocks_base_at(50.0, 50.0));
        s.push_modal(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.pop_layer();
        assert!(s.blocks_base_at(50.0, 50.0));
    }

    #[test]
    fn current_mut_routes_to_base_when_no_layers() {
        let mut s = LayerStack::new();
        // A translate-only quad records one SDF chrome instance (not soup).
        s.current_mut().quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(s.base().chrome_instance_count(), 1);
    }

    #[test]
    fn current_mut_routes_to_top_layer() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 100.0, 100.0));
        s.current_mut().quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(s.base().chrome_instance_count(), 0);
        assert_eq!(s.layers()[0].list.chrome_instance_count(), 1);
        s.pop_layer();
    }

    #[test]
    fn modal_consumes_input_for_lower_layers() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(10.0, 10.0, 50.0, 50.0));
        s.pop_layer();

        let inp = input_at(0.0, 0.0); // outside modal rect
        let base_in = s.input_for_base(&inp);
        assert!(base_in.mouse_consumed);
        assert!(!base_in.mouse_clicked);
        assert_eq!(base_in.scroll_delta, 0.0);
    }

    #[test]
    fn popup_only_consumes_input_inside_its_rect() {
        let mut s = LayerStack::new();
        s.push_popup(Rect::new(100.0, 100.0, 50.0, 50.0));
        s.pop_layer();

        // Outside popup -> not consumed.
        let outside = input_at(0.0, 0.0);
        assert!(!s.input_for_base(&outside).mouse_consumed);

        // Inside popup -> consumed.
        let inside = input_at(120.0, 120.0);
        assert!(s.input_for_base(&inside).mouse_consumed);
    }

    #[test]
    fn tooltip_never_consumes_input() {
        let mut s = LayerStack::new();
        s.push_tooltip(Rect::new(0.0, 0.0, 200.0, 200.0));
        s.pop_layer();
        let inp = input_at(50.0, 50.0);
        assert!(!s.input_for_base(&inp).mouse_consumed);
    }

    #[test]
    fn nested_push_pop_balanced() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.push_popup(Rect::new(5.0, 5.0, 5.0, 5.0));
        s.pop_layer();
        s.pop_layer();
        assert!(!s.has_active_layer());
    }

    #[test]
    fn layer_below_popup_unaffected_outside_popup_rect() {
        let mut s = LayerStack::new();
        // First layer (index 0) — modal-ish thing
        let _ = s.push_modal(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        s.pop_layer();
        // Second layer (index 1) — popup
        let _ = s.push_popup(Rect::new(500.0, 500.0, 50.0, 50.0));
        s.pop_layer();

        // Layer 0 (modal): when computing input for it, only layers above
        // (the popup at index 1) may consume. Cursor at (0,0) is outside the
        // popup -> layer 0 still gets full input.
        let inp = input_at(0.0, 0.0);
        let layer0 = s.input_for_layer(0, &inp);
        assert!(!layer0.mouse_consumed);

        // But base layer is below the modal, so it IS consumed.
        let base = s.input_for_base(&inp);
        assert!(base.mouse_consumed);
    }

    #[test]
    fn full_frame_modal_dispatch_routes_input_correctly() {
        // Simulates: push modal, then ask `input_for_base` and
        // `input_for_layer` for a single raw input. The base view must be
        // marked consumed; the modal view must NOT be (no higher layer is
        // above it), so the modal's "close button" hit-test would fire.
        let mut s = LayerStack::new();
        let modal_idx = s.push_modal(Rect::new(100.0, 100.0, 200.0, 200.0));

        // Mouse over a "base button" rect at (10,10)-(40,40).
        let raw = InputState {
            mouse_x: 25.0,
            mouse_y: 25.0,
            mouse_clicked: true,
            mouse_down: true,
            scroll_delta: -2.0,
            ..InputState::default()
        };

        let base_view = s.input_for_base(&raw);
        // Base layer must see the input as consumed -> a base hit-test would
        // not fire.
        assert!(base_view.mouse_consumed);
        assert!(!base_view.mouse_clicked);
        assert_eq!(base_view.scroll_delta, 0.0);
        assert!(base_view.scroll_consumed);

        // The modal layer is the topmost; nothing above it consumes input.
        let modal_view = s.input_for_layer(modal_idx, &raw);
        assert!(!modal_view.mouse_consumed);
        assert!(modal_view.mouse_clicked);
        assert_eq!(modal_view.scroll_delta, -2.0);
        assert!(!modal_view.scroll_consumed);

        s.pop_layer();
    }

    /// The button held at (`x`, `y`) after its press frame.
    fn held_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            ..InputState::default()
        }
    }

    /// The button let go at (`x`, `y`) this frame.
    fn released_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_released: true,
            ..InputState::default()
        }
    }

    /// The button up at (`x`, `y`).
    fn up_at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    #[test]
    fn a_press_on_a_dialog_it_closes_stays_from_the_base_until_released() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 100.0, 100.0));
        s.pop_layer();
        assert!(s.input_for_base(&input_at(20.0, 20.0)).mouse_consumed);

        // The press closed the dialog; the button stays down over the base.
        s.clear();
        let held = s.input_for_base(&held_at(20.0, 20.0));
        assert!(held.mouse_consumed, "the press is still the dialog's");
        s.clear();
        let released = s.input_for_base(&released_at(20.0, 20.0));
        assert!(released.mouse_consumed && !released.mouse_released);

        // The next press is the base's.
        s.clear();
        assert!(!s.input_for_base(&up_at(20.0, 20.0)).mouse_consumed);
        s.clear();
        let press = s.input_for_base(&input_at(20.0, 20.0));
        assert!(!press.mouse_consumed && press.mouse_clicked);
    }

    #[test]
    fn a_press_and_release_in_one_frame_is_let_go_the_frame_after() {
        let mut s = LayerStack::new();
        s.push_popup(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.pop_layer();
        let click = InputState {
            mouse_released: true,
            ..input_at(5.0, 5.0)
        };
        assert!(s.input_for_base(&click).mouse_consumed);
        s.clear();
        assert!(!s.input_for_base(&up_at(5.0, 5.0)).mouse_consumed);
    }

    #[test]
    fn a_press_on_a_popup_over_a_dialog_stays_from_the_dialog_once_the_popup_closes() {
        let mut s = LayerStack::new();
        let dialog = s.push_modal(Rect::new(0.0, 0.0, 400.0, 400.0));
        s.pop_layer();
        s.push_popup(Rect::new(10.0, 10.0, 50.0, 50.0));
        s.pop_layer();
        assert!(
            !s.input_for_layer(dialog, &input_at(20.0, 20.0))
                .mouse_clicked
        );

        s.clear();
        let dialog = s.push_modal(Rect::new(0.0, 0.0, 400.0, 400.0));
        s.pop_layer();
        assert!(
            s.input_for_layer(dialog, &held_at(20.0, 20.0))
                .mouse_consumed
        );
        assert!(s.input_for_base(&held_at(20.0, 20.0)).mouse_consumed);
    }

    #[test]
    fn a_press_on_the_base_is_not_taken_by_moving_out_from_under_a_popup() {
        // The base keeps its own press; the popup takes the pointer only
        // while it is over it, as before.
        let mut s = LayerStack::new();
        s.push_popup(Rect::new(100.0, 100.0, 50.0, 50.0));
        s.pop_layer();
        assert!(s.input_for_base(&input_at(0.0, 0.0)).mouse_clicked);
        s.clear();
        s.push_popup(Rect::new(100.0, 100.0, 50.0, 50.0));
        s.pop_layer();
        assert!(s.input_for_base(&held_at(120.0, 120.0)).mouse_consumed);
        assert!(!s.input_for_base(&held_at(0.0, 0.0)).mouse_consumed);
    }

    #[test]
    fn input_a_layer_already_consumed_leaves_the_press_be() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.pop_layer();
        let _ = s.input_for_base(&input_at(5.0, 5.0));
        s.clear();
        // A consumed release has lost its edge and looks like the button up.
        let _ = s.input_for_base(&released_at(5.0, 5.0).consumed());
        assert!(s.input_for_base(&released_at(5.0, 5.0)).mouse_consumed);
    }

    #[test]
    fn a_layer_moved_after_its_push_is_dispatched_where_it_is_now() {
        let mut s = LayerStack::new();
        let popup = s.push_popup(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.pop_layer();
        assert!(s.input_for_base(&up_at(5.0, 5.0)).mouse_consumed);
        s.layers_mut()[popup].rect = Rect::new(50.0, 50.0, 10.0, 10.0);
        assert!(!s.input_for_base(&up_at(5.0, 5.0)).mouse_consumed);
    }

    /// The input every layer sees by the rule itself: a layer is consumed
    /// when any modal is above it, or a popup above it holds the point.
    fn naive_consumed(s: &LayerStack, depth: usize, x: f32, y: f32) -> bool {
        s.layers()[depth..].iter().any(|layer| match layer.kind {
            LayerKind::Modal => true,
            LayerKind::Popup => layer.rect.contains(x, y),
            LayerKind::Tooltip => false,
        })
    }

    #[test]
    fn two_thousand_layers_dispatch_as_the_rule_says() {
        let mut s = LayerStack::new();
        for i in 0..2_000 {
            let at = (i % 97) as f32 * 10.0;
            let rect = Rect::new(at, at, 40.0, 40.0);
            match i % 5 {
                0 if i % 500 == 0 => s.push_modal(rect),
                0 | 1 => s.push_tooltip(rect),
                _ => s.push_popup(rect),
            };
            s.pop_layer();
        }
        for (x, y) in [(0.0, 0.0), (15.0, 15.0), (500.0, 505.0), (5_000.0, 5_000.0)] {
            let input = up_at(x, y);
            assert_eq!(
                s.input_for_base(&input).mouse_consumed,
                naive_consumed(&s, 0, x, y)
            );
            for index in 0..s.layers().len() {
                assert_eq!(
                    s.input_for_layer(index, &input).mouse_consumed,
                    naive_consumed(&s, index + 1, x, y),
                    "layer {index} at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    #[should_panic]
    fn drop_with_unbalanced_layer_panics_in_debug() {
        let mut s = LayerStack::new();
        s.push_modal(Rect::new(0.0, 0.0, 1.0, 1.0));
        // forgot to pop -> drop fires debug_assert
    }
}
