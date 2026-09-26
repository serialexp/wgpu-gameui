//! AlertDialog — one message, one key (Forge `AlertDialog`).

use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::material::Tone;
use super::{
    DrawContext, DrawList, FocusId, Modal, ModalState, ScrollState, ScrollView, Severity,
    SheetAction,
};

/// Forge's default alert width.
pub const ALERT_DIALOG_WIDTH: f32 = 320.0;
/// The detail block's tallest text, before it scrolls.
const DETAIL_MAX: f32 = 96.0;
/// Padding inside the detail block: 8 px at the sides, 6 above and below.
const DETAIL_PAD: (f32, f32) = (8.0, 6.0);
/// Between the message and the detail block.
const DETAIL_GAP: f32 = 8.0;
/// Detail line height, in em.
const DETAIL_LEADING: f32 = 1.5;
/// The detail well's recess (`--well-inset-tall`) and lit lip.
const RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const LIP: [f32; 4] = [1.0, 1.0, 1.0, 0.07];

/// Caller-owned alert state: the modal and the detail block's scroll.
#[derive(Clone, Debug, Default)]
pub struct AlertDialogState {
    /// The modal underneath. Call its
    /// [`begin_frame`](ModalState::begin_frame) before the focus owner's.
    pub modal: ModalState,
    detail: ScrollState,
}

impl AlertDialogState {
    /// A closed alert.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the alert, scrolled to the top.
    pub fn open(&mut self) {
        self.detail.reset();
        self.modal.open();
    }

    /// Whether the alert is open.
    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }
}

/// One message, one key (Forge `AlertDialog`).
///
/// A [`Modal`] with a tone icon, a message, an optional mono `detail` block
/// (error text, paths) that scrolls past 96 px, and a single primary key.
/// The key takes focus, so Enter or Space dismisses; so do Escape and a
/// click on the key. On dismissal it closes its modal and gives focus back.
///
/// ```ignore
/// if AlertDialog::new("Couldn't open level").tone(Severity::Error)
///     .message("Another editor has it open.").detail(&err)
///     .draw(ALERT_ID, screen, &mut alert, &mut ctx)
/// { /* dismissed */ }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertDialog<'a> {
    title: &'a str,
    message: Option<&'a str>,
    detail: Option<&'a str>,
    tone: Severity,
    ok_label: &'a str,
    width: f32,
}

impl<'a> AlertDialog<'a> {
    /// An info alert titled `title`.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            message: None,
            detail: None,
            tone: Severity::Info,
            ok_label: "OK",
            width: ALERT_DIALOG_WIDTH,
        }
    }

    /// The message under the title.
    #[must_use]
    pub fn message(mut self, message: &'a str) -> Self {
        self.message = Some(message);
        self
    }

    /// A mono block under the message for text to read closely: an error,
    /// a path.
    #[must_use]
    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    /// The tone icon (default [`Severity::Info`]).
    #[must_use]
    pub fn tone(mut self, tone: Severity) -> Self {
        self.tone = tone;
        self
    }

    /// The key's label (default "OK").
    #[must_use]
    pub fn ok_label(mut self, label: &'a str) -> Self {
        self.ok_label = label;
        self
    }

    /// The sheet's width (default [`ALERT_DIALOG_WIDTH`]).
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// The detail text block at `(x, y)`, wrapped to `width`.
    fn detail_block(detail: &str, x: f32, y: f32, width: f32, s: &StyleResolver) -> TextBlock {
        let size = s.text_size(TextSize::Meta);
        s.mono_block(detail, x, y, TextSize::Meta, Ink::Muted)
            .with_line_height(size * DETAIL_LEADING)
            .with_max_width(width)
    }

    /// The detail block's outer height at `width`, and its text's height.
    fn detail_height(
        detail: &str,
        width: f32,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> (f32, f32) {
        let border = s.scalar(StyleKey::BorderWidth);
        let text_w = (width - 2.0 * (DETAIL_PAD.0 + border)).max(0.0);
        let text_h = list
            .measure_block(&Self::detail_block(detail, 0.0, 0.0, text_w, s))
            .1
            .ceil();
        (
            text_h.min(DETAIL_MAX) + 2.0 * (DETAIL_PAD.1 + border),
            text_h,
        )
    }

    /// Draw the alert over `bounds`, its key as focus id `id`. Returns
    /// `true` the frame it is dismissed (it has then closed itself).
    pub fn draw(
        &self,
        id: FocusId,
        bounds: Rect,
        state: &mut AlertDialogState,
        ctx: &mut DrawContext,
    ) -> bool {
        let actions = [SheetAction::new(self.ok_label).tone(Tone::Accent)];
        let mut modal = Modal::new()
            .title(self.title)
            .tone(self.tone)
            .actions(&actions)
            .width(self.width)
            .focusable(id);
        if let Some(message) = self.message {
            modal = modal.message(message);
        }
        // The detail wraps to the text column, so its height comes from the
        // column's width.
        let mut detail_h = None;
        if let Some(detail) = self.detail {
            let s = ctx.styles();
            let slot_w = modal.content_width(bounds, &s);
            let (h, text_h) = Self::detail_height(detail, slot_w, ctx.draw_list, &s);
            let gap = if self.message.is_some() {
                DETAIL_GAP
            } else {
                0.0
            };
            modal = modal.content(gap + h);
            detail_h = Some((gap, h, text_h));
        }
        let scroll = &mut state.detail;
        let out = modal.draw_with(bounds, &mut state.modal, ctx, |_, c, ctx| {
            // The content slot is the only one reserved.
            if let (Some(detail), Some((gap, h, text_h))) = (self.detail, detail_h) {
                let rect = Rect::new(c.x, c.y + gap, c.width, h);
                draw_detail(detail, rect, text_h, scroll, ctx);
            }
        });
        let done = out.dismissed || out.sheet.clicked.is_some();
        if done {
            state.modal.close(ctx.focus);
        }
        done
    }
}

/// The detail well at `rect` with its text scrolled inside.
fn draw_detail(
    detail: &str,
    rect: Rect,
    text_h: f32,
    scroll: &mut ScrollState,
    ctx: &mut DrawContext,
) {
    let s = ctx.styles();
    let list = &mut *ctx.draw_list;
    let radius = s.scalar(StyleKey::BorderRadius);
    let border = s.scalar(StyleKey::BorderWidth);
    let radii = CornerRadii::uniform(radius);
    list.box_shadow_outset(
        rect,
        radii,
        BoxShadow {
            offset: [0.0, 1.0],
            color: LIP,
            ..BoxShadow::default()
        },
    );
    list.chrome_rect(
        rect,
        radius,
        border,
        s.color(StyleKey::WellDeep),
        s.color(StyleKey::EdgeHard),
    );
    let viewport = rect.inset(border);
    list.box_shadow_inset(
        viewport,
        CornerRadii::uniform((radius - border).max(0.0)),
        BoxShadow {
            offset: [0.0, 2.0],
            blur: 5.0,
            color: RECESS,
            inset: true,
            ..BoxShadow::default()
        },
    );
    scroll.content_size = [viewport.width, text_h + 2.0 * DETAIL_PAD.1];
    let mut input = ctx.input.clone();
    let text_w = (viewport.width - 2.0 * DETAIL_PAD.0).max(0.0);
    ScrollView::new(viewport).overlay().vertical_only().draw(
        scroll,
        list,
        &s,
        &mut input,
        |list, inner| {
            list.text(AlertDialog::detail_block(
                detail,
                inner.x + DETAIL_PAD.0,
                inner.y + DETAIL_PAD.1,
                text_w,
                &s,
            ));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, NavInput, Theme};

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    fn frame(
        alert: &AlertDialog,
        state: &mut AlertDialogState,
        focus: &mut FocusState,
        mut input: InputState,
    ) -> (bool, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        state.modal.begin_frame(&mut input);
        focus.begin_frame(&input);
        let done = {
            let mut ctx =
                DrawContext::new(&mut list, focus, &theme, &input, 800.0, 600.0).with_layer(0);
            alert.draw(7, SCREEN, state, &mut ctx)
        };
        focus.end_frame(Some(0));
        (done, list)
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..InputState::default()
        }
    }

    fn nav(nav: NavInput) -> InputState {
        InputState { nav, ..idle() }
    }

    #[test]
    fn enter_on_the_focused_key_dismisses_and_restores_focus() {
        let alert = AlertDialog::new("Saved");
        let mut state = AlertDialogState::new();
        let mut focus = FocusState::new();
        focus.focus(2);
        state.open();
        let (done, _) = frame(&alert, &mut state, &mut focus, idle());
        assert!(!done);
        assert_eq!(focus.focused(), Some(7), "the key takes focus");
        let confirm = nav(NavInput {
            confirm: true,
            ..NavInput::default()
        });
        let (done, _) = frame(&alert, &mut state, &mut focus, confirm);
        assert!(done);
        assert!(!state.is_open());
        assert_eq!(focus.focused(), Some(2));
    }

    #[test]
    fn escape_dismisses() {
        let alert = AlertDialog::new("Saved");
        let mut state = AlertDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&alert, &mut state, &mut focus, idle());
        let esc = nav(NavInput {
            cancel: true,
            ..NavInput::default()
        });
        assert!(frame(&alert, &mut state, &mut focus, esc).0);
    }

    #[test]
    fn info_tone_by_default() {
        let alert = AlertDialog::new("Saved");
        let mut state = AlertDialogState::new();
        let mut focus = FocusState::new();
        let (_, list) = frame(&alert, &mut state, &mut focus, idle());
        assert!(list.texts.iter().any(|t| t.content == "i"));
    }

    #[test]
    fn long_detail_stops_growing_at_the_cap() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let short = AlertDialog::detail_height("one line", 240.0, &mut list, &s);
        let long_text = "line\n".repeat(40);
        let long = AlertDialog::detail_height(&long_text, 240.0, &mut list, &s);
        let chrome = 2.0 * (DETAIL_PAD.1 + s.scalar(StyleKey::BorderWidth));
        assert!(short.0 < DETAIL_MAX + chrome);
        assert_eq!(long.0, DETAIL_MAX + chrome);
        assert!(long.1 > DETAIL_MAX, "the text itself is taller and scrolls");
    }

    #[test]
    fn detail_sits_under_the_message_in_mono() {
        let alert = AlertDialog::new("Couldn't open")
            .message("Close it there.")
            .detail("EBUSY: level.lvl");
        let mut state = AlertDialogState::new();
        let mut focus = FocusState::new();
        let (_, list) = frame(&alert, &mut state, &mut focus, idle());
        let message = list
            .texts
            .iter()
            .find(|t| t.content == "Close it there.")
            .unwrap();
        let detail = list
            .texts
            .iter()
            .find(|t| t.content == "EBUSY: level.lvl")
            .unwrap();
        assert!(detail.y > message.y);
        assert_eq!(detail.x, message.x + 1.0 + DETAIL_PAD.0);
    }
}
