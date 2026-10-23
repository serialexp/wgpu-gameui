//! Input as the window reports it, before it becomes [`InputState`].
//!
//! A host used to write its own `match` over winit's events, setting
//! `InputState` fields by hand, and every host got a slightly different
//! version: one dropped a key tapped between two frames, another a click whose
//! press and release landed in the same frame. [`HostEvent`] is the one
//! vocabulary those events come in as, and [`InputState::apply`] the one
//! mapping onto the fields. A scripted test feeds the same events, so it
//! exercises the path a real window takes. With the `winit` feature,
//! [`HostEvent::from_winit`](crate::HostEvent::from_winit) translates winit's
//! events.
//!
//! An app with shortcuts of its own (a paste that reads the clipboard, a key
//! that opens a dialog) handles those in its own event function and calls
//! `apply` for the rest.

use crate::{InputState, Key};

/// Where the pointer is put when it leaves the window: far enough away that
/// nothing on screen is under it.
pub const POINTER_AWAY: f32 = -1.0e6;

/// A pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// The left button, or a tap.
    Primary,
    /// The right button.
    Secondary,
    /// The middle button, or a wheel press.
    Middle,
}

/// The modifier keys held, as the keyboard has them. [`primary`](Self::primary)
/// is the shortcut key, which differs between platforms; an app that needs
/// Control on macOS (Ctrl+U to empty a field) reads [`control`](Self::control).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldModifiers {
    /// Either Shift.
    pub shift: bool,
    /// Either Control.
    pub control: bool,
    /// Either Alt (Option on macOS).
    pub alt: bool,
    /// The logo key: Command on macOS, the Windows key elsewhere.
    pub logo: bool,
}

impl HeldModifiers {
    /// Shift alone.
    pub const SHIFT: Self = Self::only(true, false, false, false);
    /// Control alone.
    pub const CONTROL: Self = Self::only(false, true, false, false);
    /// Alt alone.
    pub const ALT: Self = Self::only(false, false, true, false);
    /// The logo key alone.
    pub const LOGO: Self = Self::only(false, false, false, true);

    const fn only(shift: bool, control: bool, alt: bool, logo: bool) -> Self {
        Self {
            shift,
            control,
            alt,
            logo,
        }
    }

    /// No modifier is held.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The shortcut key is held: Command on macOS, Control elsewhere.
    pub fn primary(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.logo
        } else {
            self.control
        }
    }
}

/// Both sets held: `HeldModifiers::CONTROL | HeldModifiers::SHIFT`.
impl std::ops::BitOr for HeldModifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            shift: self.shift || other.shift,
            control: self.control || other.control,
            alt: self.alt || other.alt,
            logo: self.logo || other.logo,
        }
    }
}

/// One input event from the window, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// The pointer moved to `(x, y)`.
    PointerMoved {
        /// Logical x.
        x: f32,
        /// Logical y.
        y: f32,
    },
    /// The pointer left the window.
    PointerLeft,
    /// A button went down or came up.
    Button {
        /// Which button.
        button: PointerButton,
        /// `true` for a press.
        down: bool,
    },
    /// The wheel or touchpad scrolled, in lines (positive is up and left, as
    /// [`InputState::scroll_delta`] has it).
    Wheel {
        /// Sideways lines.
        dx: f32,
        /// Lines.
        dy: f32,
    },
    /// A key went down (or repeated) or came up.
    Key {
        /// The key, as the layout names it without modifiers.
        key: Key,
        /// `true` for a press or a repeat.
        down: bool,
        /// The OS repeating a held key.
        repeat: bool,
        /// The text the press types, if any.
        text: Option<String>,
    },
    /// The held modifiers changed.
    Modifiers(HeldModifiers),
    /// The input method is composing `text` (shown, not yet typed), with its
    /// cursor at the byte range `cursor`. An empty `text` ends the
    /// composition.
    Preedit {
        /// The composition.
        text: String,
        /// The IME's cursor within it.
        cursor: Option<[usize; 2]>,
    },
    /// The input method typed `text`.
    Commit(String),
}

impl InputState {
    /// Apply one event: the fields it sets, and its edges for this frame.
    ///
    /// Edges only ever turn on here, so a press and its release between two
    /// frames both reach the next one: a quick click has `mouse_clicked` and
    /// `mouse_released` together, and a quick tap of a key still sets its
    /// flag. [`end_frame`](Self::end_frame) clears them.
    pub fn apply(&mut self, event: &HostEvent) {
        match event {
            HostEvent::PointerMoved { x, y } => {
                self.mouse_x = *x;
                self.mouse_y = *y;
            }
            // A drag keeps following the pointer, which the window still
            // reports while a button is held.
            HostEvent::PointerLeft => {
                if !self.mouse_down {
                    self.mouse_x = POINTER_AWAY;
                    self.mouse_y = POINTER_AWAY;
                }
            }
            HostEvent::Button { button, down } => {
                let (held, pressed, released) = match button {
                    PointerButton::Primary => (
                        &mut self.mouse_down,
                        &mut self.mouse_clicked,
                        &mut self.mouse_released,
                    ),
                    PointerButton::Secondary => (
                        &mut self.mouse_right_down,
                        &mut self.mouse_right_clicked,
                        &mut self.mouse_right_released,
                    ),
                    PointerButton::Middle => (
                        &mut self.mouse_middle_down,
                        &mut self.mouse_middle_clicked,
                        &mut self.mouse_middle_released,
                    ),
                };
                *pressed |= *down && !*held;
                *released |= !*down && *held;
                *held = *down;
            }
            HostEvent::Wheel { dx, dy } => {
                self.scroll_delta += dy;
                self.scroll_delta_x += dx;
            }
            HostEvent::Key {
                key,
                down,
                repeat,
                text,
            } => self.apply_key(*key, *down, *repeat, text.as_deref()),
            HostEvent::Modifiers(held) => {
                self.shift_pressed = held.shift;
                self.ctrl_pressed = held.primary();
                self.alt_pressed |= held.alt && !self.alt_down;
                self.alt_released |= !held.alt && self.alt_down;
                self.alt_down = held.alt;
            }
            HostEvent::Preedit { text, cursor } => {
                self.preedit.clone_from(text);
                self.preedit_cursor = if text.is_empty() { None } else { *cursor };
            }
            HostEvent::Commit(text) => {
                self.preedit.clear();
                self.preedit_cursor = None;
                self.text_input.push_str(text);
            }
        }
    }

    fn apply_key(&mut self, key: Key, down: bool, repeat: bool, text: Option<&str>) {
        // A widget's own shortcut fires once per press: holding F mustn't
        // flick a video in and out of full screen. A key outside the
        // vocabulary (é) has no shortcut, only its text.
        if !repeat && key.slot().is_some() {
            self.keys.key_event(key, down);
        }
        if !down {
            return;
        }
        let flag = match key {
            Key::Enter => Some(&mut self.enter_pressed),
            Key::Backspace => Some(&mut self.backspace_pressed),
            Key::Left => Some(&mut self.key_left),
            Key::Right => Some(&mut self.key_right),
            Key::Up => Some(&mut self.key_up),
            Key::Down => Some(&mut self.key_down),
            Key::Home => Some(&mut self.key_home),
            Key::End => Some(&mut self.key_end),
            Key::Delete => Some(&mut self.key_delete),
            Key::Tab => Some(&mut self.key_tab),
            Key::Escape => Some(&mut self.key_escape),
            Key::Space => Some(&mut self.key_space),
            Key::Char(_) | Key::F(_) | Key::PageUp | Key::PageDown | Key::Insert => None,
        };
        if let Some(flag) = flag {
            *flag = true;
        }
        if self.ctrl_pressed {
            let edit = match key {
                Key::Char(c) => match c.to_ascii_lowercase() {
                    'a' => Some(&mut self.key_select_all),
                    'c' => Some(&mut self.key_copy),
                    'x' => Some(&mut self.key_cut),
                    'v' => Some(&mut self.key_paste),
                    _ => None,
                },
                _ => None,
            };
            if let Some(edit) = edit {
                *edit = true;
            }
        } else if let Some(text) = text {
            self.text_input
                .extend(text.chars().filter(|character| !character.is_control()));
        }
    }
}

#[cfg(test)]
#[path = "host_event_tests.rs"]
mod tests;
