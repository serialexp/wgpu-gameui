//! ConfirmDialog — ask before doing something (Forge `ConfirmDialog`).

use crate::layout::Rect;

use super::material::Tone;
use super::{Checkbox, DrawContext, FocusId, Modal, ModalState, Severity, SheetAction};

/// Forge's default confirm width.
pub const CONFIRM_DIALOG_WIDTH: f32 = 340.0;
/// Extra room between the alternative key and Cancel when the "don't ask"
/// box holds the footer's start.
const ALT_SPACE: f32 = 8.0;

/// Which key answered a [`ConfirmDialog`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmChoice {
    /// The confirm key.
    Confirm,
    /// Cancel, or Escape.
    Cancel,
    /// The alternative key ("Don't save").
    Alt,
}

/// A [`ConfirmDialog`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmOutcome {
    /// The key that answered.
    pub choice: ConfirmChoice,
    /// The "don't ask again" box was ticked.
    pub dont_ask: bool,
}

/// Caller-owned confirm state: the modal and the "don't ask" box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfirmDialogState {
    /// The modal underneath. Call its
    /// [`begin_frame`](ModalState::begin_frame) before the focus owner's.
    pub modal: ModalState,
    dont_ask: bool,
}

impl ConfirmDialogState {
    /// A closed confirm.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the confirm with the "don't ask" box clear.
    pub fn open(&mut self) {
        self.dont_ask = false;
        self.modal.open();
    }

    /// Whether the confirm is open.
    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }
}

/// Ask before doing something (Forge `ConfirmDialog`).
///
/// Keys read `[alt] ··· [Cancel] [Confirm]`, with an optional "don't ask
/// again" box at the start. A [`destructive`](Self::destructive) confirm
/// is a danger key under a warning icon, and focus starts on Cancel so a
/// reflexive Enter never destroys work; otherwise focus starts on Confirm.
/// Escape cancels. On an answer it closes its modal and gives focus back.
///
/// ```ignore
/// match ConfirmDialog::new("Delete 3 prefabs?").destructive(true).confirm_label("Delete")
///     .draw(CONFIRM_ID, screen, &mut confirm, &mut ctx)
/// {
///     Some(ConfirmOutcome { choice: ConfirmChoice::Confirm, .. }) => delete(),
///     _ => {}
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfirmDialog<'a> {
    title: &'a str,
    message: Option<&'a str>,
    tone: Option<Severity>,
    confirm_label: &'a str,
    cancel_label: &'a str,
    alt_label: Option<&'a str>,
    dont_ask_label: Option<&'a str>,
    destructive: bool,
    width: f32,
}

impl<'a> ConfirmDialog<'a> {
    /// A confirm titled `title`.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            message: None,
            tone: None,
            confirm_label: "OK",
            cancel_label: "Cancel",
            alt_label: None,
            dont_ask_label: None,
            destructive: false,
            width: CONFIRM_DIALOG_WIDTH,
        }
    }

    /// The message under the title.
    #[must_use]
    pub fn message(mut self, message: &'a str) -> Self {
        self.message = Some(message);
        self
    }

    /// The tone icon. Default: none, or [`Severity::Warning`] when
    /// [`destructive`](Self::destructive).
    #[must_use]
    pub fn tone(mut self, tone: Severity) -> Self {
        self.tone = Some(tone);
        self
    }

    /// The confirm key's label (default "OK").
    #[must_use]
    pub fn confirm_label(mut self, label: &'a str) -> Self {
        self.confirm_label = label;
        self
    }

    /// The cancel key's label (default "Cancel").
    #[must_use]
    pub fn cancel_label(mut self, label: &'a str) -> Self {
        self.cancel_label = label;
        self
    }

    /// A third key at the start of the row ("Don't save").
    #[must_use]
    pub fn alt_label(mut self, label: &'a str) -> Self {
        self.alt_label = Some(label);
        self
    }

    /// A "don't ask again" box at the footer's start, reading `label`.
    #[must_use]
    pub fn dont_ask_label(mut self, label: &'a str) -> Self {
        self.dont_ask_label = Some(label);
        self
    }

    /// The confirm destroys work: a danger key, a warning icon, and focus
    /// starting on Cancel.
    #[must_use]
    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    /// The sheet's width (default [`CONFIRM_DIALOG_WIDTH`]).
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Draw the confirm over `bounds`. Its keys take focus ids `id`,
    /// `id + 1`, … left to right, and the "don't ask" box the one after.
    /// Returns the answer the frame it comes (it has then closed itself).
    pub fn draw(
        &self,
        id: FocusId,
        bounds: Rect,
        state: &mut ConfirmDialogState,
        ctx: &mut DrawContext,
    ) -> Option<ConfirmOutcome> {
        let (alt_tone, confirm_tone) = if self.destructive {
            (Tone::Default, Tone::Danger)
        } else {
            (Tone::Ghost, Tone::Accent)
        };
        // With the "don't ask" box at the start the alternative key joins
        // the trailing pair, set 8 px apart from it; alone it leads.
        let alt = self.alt_label.map(|label| {
            let key = SheetAction::new(label).tone(alt_tone);
            if self.dont_ask_label.is_some() {
                key.space_after(ALT_SPACE)
            } else {
                key.leading(true)
            }
        });
        let cancel = SheetAction::new(self.cancel_label);
        let confirm = SheetAction::new(self.confirm_label).tone(confirm_tone);
        let keys_with_alt;
        let keys_plain;
        let actions: &[SheetAction] = match alt {
            Some(alt) => {
                keys_with_alt = [alt, cancel, confirm];
                &keys_with_alt
            }
            None => {
                keys_plain = [cancel, confirm];
                &keys_plain
            }
        };
        let offset = FocusId::from(alt.is_some());
        let (cancel_id, confirm_id) = (id + offset, id + offset + 1);
        let check_id = confirm_id + 1;

        let checkbox = Checkbox::new().focusable(check_id);
        let s = ctx.styles();
        let lead = self
            .dont_ask_label
            .map(|label| checkbox.intrinsic_size(label, ctx.draw_list, &s));

        let mut modal = Modal::new()
            .title(self.title)
            .actions(actions)
            .width(self.width)
            .focusable(id)
            .autofocus(Some(if self.destructive {
                cancel_id
            } else {
                confirm_id
            }));
        if let Some(tone) = self.tone.or(self.destructive.then_some(Severity::Warning)) {
            modal = modal.tone(tone);
        }
        if let Some(message) = self.message {
            modal = modal.message(message);
        }
        if let Some((w, _)) = lead {
            modal = modal.lead(w.ceil());
        }
        let dont_ask = &mut state.dont_ask;
        let out = modal.draw_with(bounds, &mut state.modal, ctx, |_, slot, ctx| {
            // The lead slot is the only one reserved.
            if let (Some(label), Some((w, h))) = (self.dont_ask_label, lead) {
                let rect = Rect::new(slot.x, slot.y + ((slot.height - h) * 0.5).round(), w, h);
                if checkbox.draw(*dont_ask, label, rect, ctx) {
                    *dont_ask = !*dont_ask;
                }
            }
        });

        let choice = match out.sheet.clicked {
            _ if out.dismissed => Some(ConfirmChoice::Cancel),
            Some(i) if id + i as FocusId == confirm_id => Some(ConfirmChoice::Confirm),
            Some(i) if id + i as FocusId == cancel_id => Some(ConfirmChoice::Cancel),
            Some(_) => Some(ConfirmChoice::Alt),
            None => None,
        };
        let outcome = choice.map(|choice| ConfirmOutcome {
            choice,
            dont_ask: state.dont_ask,
        });
        if outcome.is_some() {
            state.modal.close(ctx.focus);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, NavInput, Theme};

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    fn frame(
        dialog: &ConfirmDialog,
        state: &mut ConfirmDialogState,
        focus: &mut FocusState,
        mut input: InputState,
    ) -> (Option<ConfirmOutcome>, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        state.modal.begin_frame(&mut input);
        focus.begin_frame(&input);
        let out = {
            let mut ctx =
                DrawContext::new(&mut list, focus, &theme, &input, 800.0, 600.0).with_layer(0);
            dialog.draw(20, SCREEN, state, &mut ctx)
        };
        focus.end_frame(Some(0));
        (out, list)
    }

    fn idle() -> InputState {
        InputState {
            mouse_x: -100.0,
            mouse_y: -100.0,
            ..InputState::default()
        }
    }

    fn confirm_key() -> InputState {
        InputState {
            nav: NavInput {
                confirm: true,
                ..NavInput::default()
            },
            ..idle()
        }
    }

    #[test]
    fn plain_confirm_focuses_confirm_so_enter_confirms() {
        let dialog = ConfirmDialog::new("Apply?");
        let mut state = ConfirmDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&dialog, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(21));
        let (out, _) = frame(&dialog, &mut state, &mut focus, confirm_key());
        assert_eq!(out.unwrap().choice, ConfirmChoice::Confirm);
        assert!(!state.is_open());
    }

    #[test]
    fn destructive_focuses_cancel_so_enter_cancels() {
        let dialog = ConfirmDialog::new("Delete?").destructive(true);
        let mut state = ConfirmDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        let (_, list) = frame(&dialog, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(20));
        assert!(list.texts.iter().any(|t| t.content == "!"), "warning icon");
        let (out, _) = frame(&dialog, &mut state, &mut focus, confirm_key());
        assert_eq!(out.unwrap().choice, ConfirmChoice::Cancel);
    }

    #[test]
    fn escape_cancels() {
        let dialog = ConfirmDialog::new("Apply?");
        let mut state = ConfirmDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&dialog, &mut state, &mut focus, idle());
        let esc = InputState {
            nav: NavInput {
                cancel: true,
                ..NavInput::default()
            },
            ..idle()
        };
        let (out, _) = frame(&dialog, &mut state, &mut focus, esc);
        assert_eq!(out.unwrap().choice, ConfirmChoice::Cancel);
    }

    #[test]
    fn alt_key_answers_alt_and_shifts_the_ids() {
        let dialog = ConfirmDialog::new("Save changes?").alt_label("Don't save");
        let mut state = ConfirmDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&dialog, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(22), "confirm is third");
        focus.focus(20);
        let (out, _) = frame(&dialog, &mut state, &mut focus, confirm_key());
        assert_eq!(out.unwrap().choice, ConfirmChoice::Alt);
    }

    #[test]
    fn dont_ask_box_comes_first_in_the_tab_ring() {
        let dialog = ConfirmDialog::new("Delete?")
            .destructive(true)
            .dont_ask_label("Don't ask again");
        let mut state = ConfirmDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&dialog, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(20), "Cancel");
        let back = InputState {
            nav: NavInput {
                prev: true,
                ..NavInput::default()
            },
            ..idle()
        };
        frame(&dialog, &mut state, &mut focus, back);
        assert_eq!(
            focus.focused(),
            Some(22),
            "Shift+Tab from Cancel reaches the box"
        );
    }

    #[test]
    fn dont_ask_box_toggles_and_rides_along() {
        let dialog = ConfirmDialog::new("Delete?")
            .destructive(true)
            .dont_ask_label("Don't ask again");
        let mut state = ConfirmDialogState::new();
        let mut focus = FocusState::new();
        state.open();
        frame(&dialog, &mut state, &mut focus, idle());
        // The box is the focus id after the keys; Space toggles it.
        focus.focus(22);
        let (out, _) = frame(&dialog, &mut state, &mut focus, confirm_key());
        assert!(out.is_none());
        focus.focus(21);
        let (out, _) = frame(&dialog, &mut state, &mut focus, confirm_key());
        assert_eq!(
            out,
            Some(ConfirmOutcome {
                choice: ConfirmChoice::Confirm,
                dont_ask: true
            })
        );
    }
}
