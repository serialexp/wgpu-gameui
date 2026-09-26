//! Modal — a sheet over a dimmed backdrop (Forge `Modal`).

use crate::InputState;
use crate::layout::Rect;

use super::{
    DrawContext, FocusId, FocusState, Severity, Sheet, SheetAction, SheetAlign, SheetOutput,
    SheetSlot,
};

/// Forge's default modal width.
pub const MODAL_WIDTH: f32 = 300.0;
/// The backdrop blur Forge puts under a modal, in px, for
/// [`UiRenderer::blur_backdrop`](crate::UiRenderer::blur_backdrop).
pub const MODAL_BACKDROP_BLUR: f32 = 3.0;
/// Room kept between the backdrop's edge and the sheet.
const BACKDROP_PAD: f32 = 12.0;

/// Caller-owned modal state: whether it is open, the Escape it claims, and
/// the focus to give back when it closes.
///
/// One per modal. Drive it around the frame:
///
/// ```ignore
/// modal.begin_frame(&mut input); // before focus.begin_frame: claims Escape
/// focus.begin_frame(&input);
/// let layer = modal.is_open().then(|| { let i = layers.push_modal(screen); layers.pop_layer(); i });
/// // ... base UI with layers.input_for_base(&input) ...
/// if let Some(i) = layer {
///     let input = layers.input_for_layer(i, &input);
///     let mut ctx = DrawContext::new(&mut layers.layers_mut()[i].list, &mut focus, &theme, &input, w, h)
///         .with_layer(i);
///     let out = Modal::new().title("Quit?").actions(&keys).focusable(ID).draw(screen, &mut modal, &mut ctx);
///     if out.dismissed || out.sheet.clicked.is_some() { modal.close(ctx.focus); }
/// }
/// focus.end_frame(layer); // Tab stays inside the modal
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModalState {
    open: bool,
    /// Focus has been moved into the modal (on its first drawn frame).
    seeded: bool,
    /// What held focus before the modal took it.
    restore: Option<FocusId>,
    /// This frame's Escape, claimed in [`begin_frame`](Self::begin_frame).
    cancel: bool,
}

impl ModalState {
    /// A closed modal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the modal. Focus moves in on the next draw.
    pub fn open(&mut self) {
        if !self.open {
            *self = Self {
                open: true,
                ..Self::default()
            };
        }
    }

    /// Whether the modal is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Claim this frame's cancel (Escape) while open, so the focus owner
    /// doesn't also blur on it. Call before
    /// [`FocusState::begin_frame`], with the raw input.
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.cancel = false;
        if self.open {
            self.cancel = input.nav.cancel;
            input.nav.cancel = false;
        }
    }

    /// Close the modal and give focus back to what held it before.
    pub fn close(&mut self, focus: &mut FocusState) {
        if self.seeded {
            match self.restore {
                Some(id) => focus.focus(id),
                None => focus.blur(),
            }
        }
        *self = Self::default();
    }
}

/// What one [`Modal::draw`] laid out and what happened.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModalOutput {
    /// The sheet's slots and clicked action.
    pub sheet: SheetOutput,
    /// Escape was pressed, or the backdrop clicked when
    /// [`dismiss_on_backdrop`](Modal::dismiss_on_backdrop) is set. The
    /// caller closes the modal (and treats it as a cancel).
    pub dismissed: bool,
}

/// A sheet over a dimmed backdrop (Forge `Modal`).
///
/// It covers its bounds with the backdrop wash and centres a
/// [`Sheet`](super::Sheet) in it. On its first frame focus moves to the
/// [`autofocus`](Self::autofocus) target (the first key by default); Escape
/// dismisses; [`ModalState::close`] gives focus back. Draw it into a modal
/// layer ([`LayerStack::push_modal`](crate::LayerStack::push_modal)) with a
/// context [`with_layer`](super::DrawContext::with_layer), so the UI under it
/// ignores the pointer and Tab stays inside. See [`ModalState`] for the
/// frame order.
///
/// The backdrop is a flat wash. Forge also blurs what is under it; a game
/// that renders its scene to a texture can add that with
/// [`UiRenderer::blur_backdrop`](crate::UiRenderer::blur_backdrop) at
/// [`MODAL_BACKDROP_BLUR`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modal<'a> {
    sheet: Sheet<'a>,
    autofocus: Option<Option<FocusId>>,
    dismiss_on_backdrop: bool,
}

impl Default for Modal<'_> {
    fn default() -> Self {
        Self {
            sheet: Sheet::new().width(MODAL_WIDTH),
            autofocus: None,
            dismiss_on_backdrop: false,
        }
    }
}

impl<'a> Modal<'a> {
    /// An empty modal, [`MODAL_WIDTH`] wide.
    pub fn new() -> Self {
        Self::default()
    }

    /// The title.
    #[must_use]
    pub fn title(mut self, title: &'a str) -> Self {
        self.sheet = self.sheet.title(title);
        self
    }

    /// The message under the title.
    #[must_use]
    pub fn message(mut self, message: &'a str) -> Self {
        self.sheet = self.sheet.description(message);
        self
    }

    /// A [`StatusIcon`](super::StatusIcon) beside the title.
    #[must_use]
    pub fn tone(mut self, tone: Severity) -> Self {
        self.sheet = self.sheet.tone(tone);
        self
    }

    /// The footer keys, left to right; the primary one goes last.
    #[must_use]
    pub fn actions(mut self, actions: &'a [SheetAction<'a>]) -> Self {
        self.sheet = self.sheet.actions(actions);
        self
    }

    /// Reserve `height` px under the message for the caller's controls
    /// ([`SheetOutput::content`]).
    #[must_use]
    pub fn content(mut self, height: f32) -> Self {
        self.sheet = self.sheet.content(height);
        self
    }

    /// Reserve `width` px at the footer's start ([`SheetOutput::lead`]).
    #[must_use]
    pub fn lead(mut self, width: f32) -> Self {
        self.sheet = self.sheet.lead(width);
        self
    }

    /// The sheet's width (default [`MODAL_WIDTH`]).
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.sheet = self.sheet.width(width);
        self
    }

    /// Put the keys in the Tab ring as `base`, `base + 1`, … in action
    /// order.
    #[must_use]
    pub fn focusable(mut self, base: FocusId) -> Self {
        self.sheet = self.sheet.focusable(base);
        self
    }

    /// What takes focus when the modal opens: `Some(id)`, or `None` to
    /// leave focus alone. Default: the first key, when
    /// [`focusable`](Self::focusable).
    #[must_use]
    pub fn autofocus(mut self, target: Option<FocusId>) -> Self {
        self.autofocus = Some(target);
        self
    }

    /// A click on the backdrop dismisses the modal.
    #[must_use]
    pub fn dismiss_on_backdrop(mut self, dismiss: bool) -> Self {
        self.dismiss_on_backdrop = dismiss;
        self
    }

    /// The sheet this modal draws.
    pub fn sheet(&self) -> &Sheet<'a> {
        &self.sheet
    }

    /// The width of the [`content`](Self::content) slot when the modal
    /// covers `bounds`, for sizing what goes in it.
    pub fn content_width(&self, bounds: Rect, s: &crate::StyleResolver) -> f32 {
        let width = self.sheet.placed_width(bounds.inset(BACKDROP_PAD));
        self.sheet.content_width(width, s)
    }

    /// Where the sheet goes in `bounds`.
    pub fn sheet_rect(&self, bounds: Rect, ctx: &mut DrawContext) -> Rect {
        let s = ctx.styles();
        self.sheet.place(
            bounds.inset(BACKDROP_PAD),
            SheetAlign::Center,
            ctx.draw_list,
            &s,
        )
    }

    /// Draw the backdrop over `bounds` and the sheet centred in it. Nothing
    /// paints outside `bounds` (the sheet's shadow is clipped at its edge), so
    /// pass the screen, or the region the modal covers.
    pub fn draw(&self, bounds: Rect, state: &mut ModalState, ctx: &mut DrawContext) -> ModalOutput {
        self.draw_with(bounds, state, ctx, |_, _, _| {})
    }

    /// [`draw`](Self::draw), calling `fill` for each reserved slot before
    /// the keys (see [`Sheet::draw_with`]).
    pub fn draw_with(
        &self,
        bounds: Rect,
        state: &mut ModalState,
        ctx: &mut DrawContext,
        fill: impl FnMut(SheetSlot, Rect, &mut DrawContext),
    ) -> ModalOutput {
        state.open = true;
        if !state.seeded {
            state.seeded = true;
            state.restore = ctx.focus.focused();
            let target = self
                .autofocus
                .unwrap_or_else(|| self.sheet.action_focus_id(0));
            if let Some(id) = target {
                ctx.focus.focus(id);
            }
        }

        let s = ctx.styles();
        ctx.push_debug_scope_rect("Modal", bounds);
        // The modal owns `bounds` and paints nothing outside it: the sheet's
        // shadow stops at the edge, as it would at the screen's. That cut is
        // by design, so it is a viewport clip (the debug report then counts
        // only what stays visible).
        ctx.draw_list.push_clip_viewport(bounds);
        ctx.draw_list.quad(
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            s.sheet().backdrop,
        );
        let rect = self.sheet_rect(bounds, ctx);
        let sheet = self.sheet.draw_with(rect, ctx, fill);
        ctx.draw_list.pop_clip();
        ctx.pop_debug_scope();

        let input = ctx.input;
        let backdrop_click = self.dismiss_on_backdrop
            && input.mouse_clicked
            && !input.mouse_consumed
            && bounds.contains(input.mouse_x, input.mouse_y)
            && !rect.contains(input.mouse_x, input.mouse_y);
        ModalOutput {
            sheet,
            dismissed: state.cancel || backdrop_click,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, NavInput, Theme};

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    fn frame(
        modal: &Modal,
        state: &mut ModalState,
        focus: &mut FocusState,
        mut input: InputState,
    ) -> ModalOutput {
        let theme = Theme::default();
        let mut list = DrawList::new();
        state.begin_frame(&mut input);
        focus.begin_frame(&input);
        let out = {
            let mut ctx =
                DrawContext::new(&mut list, focus, &theme, &input, 800.0, 600.0).with_layer(0);
            modal.draw(SCREEN, state, &mut ctx)
        };
        focus.end_frame(Some(0));
        out
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..InputState::default()
        }
    }

    fn keys() -> [SheetAction<'static>; 2] {
        [SheetAction::new("Cancel"), SheetAction::new("OK")]
    }

    #[test]
    fn opening_moves_focus_in_and_closing_gives_it_back() {
        let keys = keys();
        let modal = Modal::new().title("Quit?").actions(&keys).focusable(10);
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        focus.focus(3);
        state.open();
        frame(&modal, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(10), "the first key takes focus");
        state.close(&mut focus);
        assert_eq!(focus.focused(), Some(3));
        assert!(!state.is_open());
    }

    #[test]
    fn autofocus_picks_the_target() {
        let keys = keys();
        let modal = Modal::new()
            .actions(&keys)
            .focusable(10)
            .autofocus(Some(11));
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&modal, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(11));
    }

    #[test]
    fn escape_dismisses_without_blurring() {
        let keys = keys();
        let modal = Modal::new().actions(&keys).focusable(10);
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&modal, &mut state, &mut focus, idle());
        let esc = InputState {
            nav: NavInput {
                cancel: true,
                ..NavInput::default()
            },
            ..idle()
        };
        let out = frame(&modal, &mut state, &mut focus, esc);
        assert!(out.dismissed);
        assert_eq!(focus.focused(), Some(10), "the modal claimed the Escape");
    }

    #[test]
    fn tab_cycles_inside_the_modal() {
        let keys = keys();
        let modal = Modal::new().actions(&keys).focusable(10);
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&modal, &mut state, &mut focus, idle());
        let tab = InputState {
            nav: NavInput {
                next: true,
                ..NavInput::default()
            },
            ..idle()
        };
        frame(&modal, &mut state, &mut focus, tab.clone());
        assert_eq!(focus.focused(), Some(11));
        frame(&modal, &mut state, &mut focus, tab);
        assert_eq!(focus.focused(), Some(10), "wraps to the first key");
    }

    #[test]
    fn backdrop_click_dismisses_only_when_asked() {
        let click = InputState {
            mouse_x: 5.0,
            mouse_y: 5.0,
            mouse_clicked: true,
            ..InputState::default()
        };
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        let out = frame(
            &Modal::new().title("A"),
            &mut state,
            &mut focus,
            click.clone(),
        );
        assert!(!out.dismissed);
        let out = frame(
            &Modal::new().title("A").dismiss_on_backdrop(true),
            &mut state,
            &mut focus,
            click,
        );
        assert!(out.dismissed);
    }

    #[test]
    fn a_click_on_the_sheet_is_not_a_backdrop_click() {
        let modal = Modal::new().title("A").dismiss_on_backdrop(true);
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        let out = frame(&modal, &mut state, &mut focus, idle());
        let r = out.sheet.rect;
        let click = InputState {
            mouse_x: r.x + 4.0,
            mouse_y: r.y + 4.0,
            mouse_clicked: true,
            ..InputState::default()
        };
        assert!(!frame(&modal, &mut state, &mut focus, click).dismissed);
    }

    #[test]
    fn sheet_is_centred_in_the_backdrop() {
        let mut state = ModalState::new();
        let mut focus = FocusState::new();
        let out = frame(&Modal::new().title("A"), &mut state, &mut focus, idle());
        let r = out.sheet.rect;
        assert_eq!(r.width, MODAL_WIDTH);
        assert_eq!(r.x, (800.0 - MODAL_WIDTH) * 0.5);
    }

    #[test]
    fn nothing_paints_outside_the_bounds() {
        // A modal over a region smaller than its sheet's shadow reach.
        let bounds = Rect::new(40.0, 30.0, 360.0, 220.0);
        let keys = keys();
        let modal = Modal::new().title("Quit?").actions(&keys);
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut state = ModalState::new();
        let input = idle();
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 800.0, 600.0);
            modal.draw(bounds, &mut state, &mut ctx);
        }
        assert!(list.shadow_instance_count() > 0, "the sheet casts shadows");
        for shadow in list.shadow_instances() {
            assert_eq!(shadow.translation[2], 1.0, "shadow is clipped");
            assert_eq!(
                shadow.clip,
                [bounds.x, bounds.y, bounds.width, bounds.height]
            );
        }
        let report = crate::debug::DebugReport::from_draw_list(&list, SCREEN);
        assert!(
            !report
                .problems()
                .iter()
                .any(|p| p.code() == "overflows_declared"),
            "{:?}",
            report.problems()
        );
    }

    #[test]
    fn closed_modal_claims_no_escape() {
        let mut state = ModalState::new();
        let mut input = InputState {
            nav: NavInput {
                cancel: true,
                ..NavInput::default()
            },
            ..idle()
        };
        state.begin_frame(&mut input);
        assert!(input.nav.cancel);
    }
}
