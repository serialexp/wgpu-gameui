//! PromptDialog — ask for one text value (Forge `PromptDialog`).

use crate::color::{HUE_DANGER, oklch};
use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::material::Tone;
use super::{
    DrawContext, DrawList, FieldLabel, FocusId, Modal, ModalState, SheetAction, TextInput,
};

/// Forge's default prompt width.
pub const PROMPT_DIALOG_WIDTH: f32 = 340.0;
/// Between the field label, the field and the note under it.
const FIELD_GAP: f32 = 4.0;
/// Note line height, in em.
const NOTE_LEADING: f32 = 1.45;
/// The error message's ink (`--danger-hint`).
const DANGER_HINT: (f32, f32) = (0.7, 0.15);

/// A [`PromptDialog`]'s answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptOutcome {
    /// OK, or Enter in the field, with a valid value: the value, trimmed.
    Submit(String),
    /// Cancel, or Escape.
    Cancel,
}

/// Caller-owned prompt state: the modal, the field, and whether it has been
/// edited (errors show only after the first edit).
#[derive(Default)]
pub struct PromptDialogState {
    /// The modal underneath. Call its
    /// [`begin_frame`](ModalState::begin_frame) before the focus owner's.
    pub modal: ModalState,
    /// The text field. Wire its clipboard here
    /// ([`TextInput::set_clipboard_get`]) for Ctrl+C/V.
    pub field: TextInput,
    touched: bool,
    /// The value as the frame started, to tell an edit (reused, so it
    /// doesn't allocate per frame).
    before: String,
}

impl PromptDialogState {
    /// A closed prompt.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the prompt holding `value`, all of it selected so typing
    /// replaces it.
    pub fn open(&mut self, value: &str) {
        self.field.value.clear();
        self.field.value.push_str(value);
        self.field.select_all();
        self.touched = false;
        self.modal.open();
    }

    /// Whether the prompt is open.
    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    /// The field's current text.
    pub fn value(&self) -> &str {
        &self.field.value
    }
}

/// Ask for one text value (Forge `PromptDialog`).
///
/// A [`Modal`] with an optional description, a [`FieldLabel`], one text
/// field, and a hint under it. The field opens focused with its text
/// selected. A [`validate`](Self::validate) function returns an error that
/// replaces the hint once the value has been edited, and blocks submitting;
/// a [`required`](Self::required) prompt (the default) also blocks an empty
/// value. Enter submits, Escape cancels. On an answer it closes its modal
/// and gives focus back.
///
/// ```ignore
/// let taken = |v: &str| layers.contains(&v.trim()).then(|| "A layer with that name already exists".into());
/// match PromptDialog::new("Rename layer").label("Name").validate(&taken)
///     .draw(PROMPT_ID, screen, &mut prompt, &mut ctx)
/// {
///     Some(PromptOutcome::Submit(name)) => rename(name),
///     _ => {}
/// }
/// ```
#[derive(Clone, Copy)]
pub struct PromptDialog<'a> {
    title: &'a str,
    description: Option<&'a str>,
    label: Option<&'a str>,
    hint: Option<&'a str>,
    placeholder: Option<&'a str>,
    validate: Option<&'a dyn Fn(&str) -> Option<String>>,
    required: bool,
    ok_label: &'a str,
    cancel_label: &'a str,
    width: f32,
}

impl<'a> PromptDialog<'a> {
    /// A prompt titled `title`.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            description: None,
            label: None,
            hint: None,
            placeholder: None,
            validate: None,
            required: true,
            ok_label: "OK",
            cancel_label: "Cancel",
            width: PROMPT_DIALOG_WIDTH,
        }
    }

    /// The text under the title.
    #[must_use]
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    /// A [`FieldLabel`] above the field.
    #[must_use]
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// A dim note under the field.
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// The field's placeholder while empty.
    #[must_use]
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    /// Check the value: return an error to show and block submitting, or
    /// `None` when it is fine.
    #[must_use]
    pub fn validate(mut self, validate: &'a dyn Fn(&str) -> Option<String>) -> Self {
        self.validate = Some(validate);
        self
    }

    /// Block an empty (all-space) value (default `true`).
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// The OK key's label (default "OK").
    #[must_use]
    pub fn ok_label(mut self, label: &'a str) -> Self {
        self.ok_label = label;
        self
    }

    /// The cancel key's label (default "Cancel").
    #[must_use]
    pub fn cancel_label(mut self, label: &'a str) -> Self {
        self.cancel_label = label;
        self
    }

    /// The sheet's width (default [`PROMPT_DIALOG_WIDTH`]).
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// The error for `value`, if any.
    fn error(&self, value: &str) -> Option<String> {
        self.validate.and_then(|validate| validate(value))
    }

    /// Whether `value` may be submitted.
    fn accepts(&self, value: &str, error: &Option<String>) -> bool {
        error.is_none() && (!self.required || !value.trim().is_empty())
    }

    /// The note under the field (the error once edited, else the hint) at
    /// `(x, y)`, wrapped to `width`.
    fn note(
        &self,
        error: Option<&str>,
        x: f32,
        y: f32,
        width: f32,
        s: &StyleResolver,
    ) -> Option<TextBlock> {
        let size = s.text_size(TextSize::Dense);
        let block = match error {
            Some(error) => s
                .sans_block(error, x, y, TextSize::Dense, Ink::Dim)
                .with_color_f32(oklch(DANGER_HINT.0, DANGER_HINT.1, HUE_DANGER, 1.0)),
            None => s.sans_block(self.hint?, x, y, TextSize::Dense, Ink::Dim),
        };
        Some(
            block
                .with_line_height(size * NOTE_LEADING)
                .with_max_width(width),
        )
    }

    /// The content slot's height at `width` with `note` under the field.
    fn content_height(
        &self,
        note: Option<&TextBlock>,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> f32 {
        let mut h = s.scalar(StyleKey::InputHeight);
        if self.label.is_some() {
            h += FieldLabel::height(list, s) + FIELD_GAP;
        }
        if let Some(note) = note {
            h += FIELD_GAP + list.measure_block(note).1.ceil();
        }
        h
    }

    /// Draw the prompt over `bounds`: Cancel as focus id `id`, OK as
    /// `id + 1`, the field as `id + 2`. Returns the answer the frame it
    /// comes (it has then closed itself).
    pub fn draw(
        &self,
        id: FocusId,
        bounds: Rect,
        state: &mut PromptDialogState,
        ctx: &mut DrawContext,
    ) -> Option<PromptOutcome> {
        let field_id = id + 2;
        // Laid out from the value as the frame starts; an edit this frame
        // shows from the next.
        let error = self.error(&state.field.value);
        let ok = self.accepts(&state.field.value, &error);
        let shown = error.as_deref().filter(|_| state.touched);

        let actions = [
            SheetAction::new(self.cancel_label),
            SheetAction::new(self.ok_label)
                .tone(Tone::Accent)
                .enabled(ok),
        ];
        let mut modal = Modal::new()
            .title(self.title)
            .actions(&actions)
            .width(self.width)
            .focusable(id)
            .autofocus(Some(field_id));
        if let Some(description) = self.description {
            modal = modal.message(description);
        }
        let s = ctx.styles();
        let column = modal.content_width(bounds, &s);
        let mut note = self.note(shown, 0.0, 0.0, column, &s);
        modal = modal.content(self.content_height(note.as_ref(), ctx.draw_list, &s));

        state.before.clone_from(&state.field.value);
        let field = &mut state.field;
        let mut entered = false;
        let out = modal.draw_with(bounds, &mut state.modal, ctx, |_, c, ctx| {
            // The content slot is the only one reserved.
            let s = ctx.styles();
            let mut y = c.y;
            if let Some(label) = self.label {
                y = FieldLabel::new(label)
                    .draw(ctx.draw_list, &s, c.x, y, c.width)
                    .bottom()
                    + FIELD_GAP;
            }
            let height = s.scalar(StyleKey::InputHeight);
            field.x = c.x;
            field.y = y;
            field.width = c.width;
            field.height = height;
            field.invalid = shown.is_some();
            field.placeholder.clear();
            field
                .placeholder
                .push_str(self.placeholder.unwrap_or_default());
            field.draw(field_id, ctx);
            entered = ctx.focus.is_focused(field_id) && ctx.input.enter_pressed;
            if let Some(mut note) = note.take() {
                note.x = c.x;
                note.y = y + height + FIELD_GAP;
                ctx.draw_list.text(note);
            }
        });
        if state.field.value != state.before {
            state.touched = true;
        }

        let submit = entered || out.sheet.clicked == Some(1);
        let outcome = if out.dismissed || out.sheet.clicked == Some(0) {
            Some(PromptOutcome::Cancel)
        } else if submit {
            // Judge the value as it stands after this frame's typing.
            state.touched = true;
            let error = self.error(&state.field.value);
            self.accepts(&state.field.value, &error)
                .then(|| PromptOutcome::Submit(state.field.value.trim().to_owned()))
        } else {
            None
        };
        if outcome.is_some() {
            state.modal.close(ctx.focus);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::text_color;
    use crate::{FocusState, InputState, NavInput, Theme};

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    fn frame(
        dialog: &PromptDialog,
        state: &mut PromptDialogState,
        focus: &mut FocusState,
        mut input: InputState,
    ) -> (Option<PromptOutcome>, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        state.modal.begin_frame(&mut input);
        focus.begin_frame(&input);
        let out = {
            let mut ctx =
                DrawContext::new(&mut list, focus, &theme, &input, 800.0, 600.0).with_layer(0);
            dialog.draw(30, SCREEN, state, &mut ctx)
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

    fn typing(text: &str) -> InputState {
        InputState {
            text_input: text.to_owned(),
            ..idle()
        }
    }

    fn enter() -> InputState {
        InputState {
            enter_pressed: true,
            nav: NavInput {
                confirm: true,
                ..NavInput::default()
            },
            ..idle()
        }
    }

    fn taken(v: &str) -> Option<String> {
        (v.trim() == "Terrain").then(|| "A layer with that name already exists".to_owned())
    }

    #[test]
    fn opens_focused_with_the_value_selected() {
        let dialog = PromptDialog::new("Rename layer");
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        state.open("Props");
        frame(&dialog, &mut state, &mut focus, idle());
        assert_eq!(focus.focused(), Some(32), "the field takes focus");
        assert_eq!(state.field.selected_text(), Some("Props"));
    }

    #[test]
    fn typing_replaces_and_enter_submits_trimmed() {
        let dialog = PromptDialog::new("Rename layer");
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        state.open("Props");
        frame(&dialog, &mut state, &mut focus, idle());
        frame(&dialog, &mut state, &mut focus, typing(" Lights "));
        assert_eq!(state.value(), " Lights ");
        let (out, _) = frame(&dialog, &mut state, &mut focus, enter());
        assert_eq!(out, Some(PromptOutcome::Submit("Lights".to_owned())));
        assert!(!state.is_open());
    }

    #[test]
    fn error_shows_only_after_an_edit_and_blocks_enter() {
        let dialog = PromptDialog::new("Rename layer")
            .hint("Shown in the outliner.")
            .validate(&taken);
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        state.open("Terrain");
        let (_, list) = frame(&dialog, &mut state, &mut focus, idle());
        assert!(
            list.texts
                .iter()
                .any(|t| t.content == "Shown in the outliner."),
            "an unedited value shows the hint, even when it's invalid"
        );
        assert!(!state.field.invalid);
        frame(&dialog, &mut state, &mut focus, typing("Terrai"));
        frame(&dialog, &mut state, &mut focus, typing("n"));
        let (out, list) = frame(&dialog, &mut state, &mut focus, enter());
        assert_eq!(out, None, "an invalid value doesn't submit");
        assert!(state.is_open());
        let error = list
            .texts
            .iter()
            .find(|t| t.content == "A layer with that name already exists")
            .unwrap();
        assert_eq!(
            error.color,
            text_color(oklch(DANGER_HINT.0, DANGER_HINT.1, HUE_DANGER, 1.0))
        );
        assert!(state.field.invalid);
    }

    #[test]
    fn required_blocks_an_empty_value() {
        let dialog = PromptDialog::new("Name");
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        state.open("   ");
        frame(&dialog, &mut state, &mut focus, idle());
        assert_eq!(frame(&dialog, &mut state, &mut focus, enter()).0, None);
        let optional = dialog.required(false);
        assert_eq!(
            frame(&optional, &mut state, &mut focus, enter()).0,
            Some(PromptOutcome::Submit(String::new()))
        );
    }

    #[test]
    fn escape_cancels() {
        let dialog = PromptDialog::new("Name");
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        focus.focus(5);
        state.open("x");
        frame(&dialog, &mut state, &mut focus, idle());
        let esc = InputState {
            nav: NavInput {
                cancel: true,
                ..NavInput::default()
            },
            ..idle()
        };
        let (out, _) = frame(&dialog, &mut state, &mut focus, esc);
        assert_eq!(out, Some(PromptOutcome::Cancel));
        assert_eq!(focus.focused(), Some(5), "focus goes back");
    }

    #[test]
    fn tab_reaches_the_keys_and_back_to_the_field() {
        let dialog = PromptDialog::new("Name");
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        state.open("x");
        frame(&dialog, &mut state, &mut focus, idle());
        let tab = InputState {
            nav: NavInput {
                next: true,
                ..NavInput::default()
            },
            ..idle()
        };
        // Ring: field (32), Cancel (30), OK (31).
        frame(&dialog, &mut state, &mut focus, tab.clone());
        assert_eq!(focus.focused(), Some(30));
        frame(&dialog, &mut state, &mut focus, tab.clone());
        assert_eq!(focus.focused(), Some(31));
        frame(&dialog, &mut state, &mut focus, tab);
        assert_eq!(focus.focused(), Some(32));
    }

    #[test]
    fn label_sits_above_the_field() {
        let dialog = PromptDialog::new("Rename layer").label("Name");
        let mut state = PromptDialogState::new();
        let mut focus = FocusState::new();
        state.open("Props");
        let (_, list) = frame(&dialog, &mut state, &mut focus, idle());
        let label = list.texts.iter().find(|t| t.content == "NAME").unwrap();
        assert!(label.y < state.field.y);
        assert_eq!(label.x, state.field.x);
    }
}
