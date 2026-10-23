//! winit's window events as [`HostEvent`]s (the `winit` feature).

use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key as WinitKey, NamedKey};

use crate::{HeldModifiers, HostEvent, Key, PointerButton};

/// Logical pixels a touchpad moves for one line of [`HostEvent::Wheel`].
pub const PIXELS_PER_LINE: f64 = 20.0;

impl HostEvent {
    /// Translate one winit window event, at the window's `scale_factor`.
    /// `None` for an event that is not input (a resize, a redraw) and for a
    /// key gameui has no name for.
    pub fn from_winit(event: &WindowEvent, scale_factor: f64) -> Option<Self> {
        Some(match event {
            WindowEvent::CursorMoved { position, .. } => HostEvent::PointerMoved {
                x: (position.x / scale_factor) as f32,
                y: (position.y / scale_factor) as f32,
            },
            WindowEvent::CursorLeft { .. } => HostEvent::PointerLeft,
            WindowEvent::MouseInput { state, button, .. } => HostEvent::Button {
                button: match button {
                    MouseButton::Left => PointerButton::Primary,
                    MouseButton::Right => PointerButton::Secondary,
                    MouseButton::Middle => PointerButton::Middle,
                    _ => return None,
                },
                down: *state == ElementState::Pressed,
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let [dx, dy] = wheel_lines(*delta, scale_factor);
                HostEvent::Wheel { dx, dy }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                HostEvent::Modifiers(HeldModifiers {
                    shift: state.shift_key(),
                    control: state.control_key(),
                    alt: state.alt_key(),
                    logo: state.super_key(),
                })
            }
            WindowEvent::KeyboardInput { event, .. } => HostEvent::Key {
                key: key_from_winit(&unmodified(event))?,
                down: event.state == ElementState::Pressed,
                repeat: event.repeat,
                text: event.text.as_ref().map(|text| text.to_string()),
            },
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => HostEvent::Preedit {
                text: text.clone(),
                cursor: cursor.map(|(start, end)| [start, end]),
            },
            WindowEvent::Ime(Ime::Commit(text)) => HostEvent::Commit(text.clone()),
            WindowEvent::Ime(Ime::Disabled) => HostEvent::Preedit {
                text: String::new(),
                cursor: None,
            },
            _ => return None,
        })
    }
}

/// A wheel turn in lines; a touchpad's pixels at [`PIXELS_PER_LINE`].
fn wheel_lines(delta: MouseScrollDelta, scale_factor: f64) -> [f32; 2] {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => [x, y],
        MouseScrollDelta::PixelDelta(p) => {
            let per_line = scale_factor * PIXELS_PER_LINE;
            [(p.x / per_line) as f32, (p.y / per_line) as f32]
        }
    }
}

/// The key as the layout names it without modifiers, so Shift+1 is still 1
/// and Cmd+V still V.
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn unmodified(event: &winit::event::KeyEvent) -> WinitKey {
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
    event.key_without_modifiers()
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
fn unmodified(event: &winit::event::KeyEvent) -> WinitKey {
    event.logical_key.clone()
}

/// gameui's name for a winit key: one character, or a named key gameui has.
pub fn key_from_winit(key: &WinitKey) -> Option<Key> {
    match key {
        WinitKey::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(' '), None) => Some(Key::Space),
                (Some(c), None) => Some(Key::Char(c)),
                _ => None,
            }
        }
        WinitKey::Named(named) => Some(match named {
            NamedKey::ArrowLeft => Key::Left,
            NamedKey::ArrowRight => Key::Right,
            NamedKey::ArrowUp => Key::Up,
            NamedKey::ArrowDown => Key::Down,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Enter => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Tab => Key::Tab,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::Insert => Key::Insert,
            NamedKey::Space => Key::Space,
            NamedKey::F1 => Key::F(1),
            NamedKey::F2 => Key::F(2),
            NamedKey::F3 => Key::F(3),
            NamedKey::F4 => Key::F(4),
            NamedKey::F5 => Key::F(5),
            NamedKey::F6 => Key::F(6),
            NamedKey::F7 => Key::F(7),
            NamedKey::F8 => Key::F(8),
            NamedKey::F9 => Key::F(9),
            NamedKey::F10 => Key::F(10),
            NamedKey::F11 => Key::F(11),
            NamedKey::F12 => Key::F(12),
            NamedKey::F13 => Key::F(13),
            NamedKey::F14 => Key::F(14),
            NamedKey::F15 => Key::F(15),
            NamedKey::F16 => Key::F(16),
            NamedKey::F17 => Key::F(17),
            NamedKey::F18 => Key::F(18),
            NamedKey::F19 => Key::F(19),
            NamedKey::F20 => Key::F(20),
            NamedKey::F21 => Key::F(21),
            NamedKey::F22 => Key::F(22),
            NamedKey::F23 => Key::F(23),
            NamedKey::F24 => Key::F(24),
            _ => return None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;
    use winit::event::DeviceId;

    use super::*;

    fn device() -> DeviceId {
        DeviceId::dummy()
    }

    #[test]
    fn the_pointer_is_in_logical_pixels() {
        let event = WindowEvent::CursorMoved {
            device_id: device(),
            position: PhysicalPosition::new(200.0, 100.0),
        };
        assert_eq!(
            HostEvent::from_winit(&event, 2.0),
            Some(HostEvent::PointerMoved { x: 100.0, y: 50.0 })
        );
    }

    #[test]
    fn buttons_map_and_others_are_dropped() {
        let press = |button| WindowEvent::MouseInput {
            device_id: device(),
            state: ElementState::Pressed,
            button,
        };
        assert_eq!(
            HostEvent::from_winit(&press(MouseButton::Right), 1.0),
            Some(HostEvent::Button {
                button: PointerButton::Secondary,
                down: true
            })
        );
        assert_eq!(HostEvent::from_winit(&press(MouseButton::Back), 1.0), None);
    }

    #[test]
    fn a_touchpad_scrolls_a_line_per_twenty_logical_pixels() {
        let event = WindowEvent::MouseWheel {
            device_id: device(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(-40.0, 80.0)),
            phase: winit::event::TouchPhase::Moved,
        };
        assert_eq!(
            HostEvent::from_winit(&event, 2.0),
            Some(HostEvent::Wheel { dx: -1.0, dy: 2.0 })
        );
    }

    #[test]
    fn keys_map_to_gameui_names() {
        let character = |text: &str| WinitKey::Character(text.into());
        assert_eq!(key_from_winit(&character("v")), Some(Key::Char('v')));
        assert_eq!(key_from_winit(&character(" ")), Some(Key::Space));
        assert_eq!(key_from_winit(&character("é")), Some(Key::Char('é')));
        assert_eq!(key_from_winit(&character("ab")), None);
        assert_eq!(
            key_from_winit(&WinitKey::Named(NamedKey::F12)),
            Some(Key::F(12))
        );
        assert_eq!(
            key_from_winit(&WinitKey::Named(NamedKey::ArrowUp)),
            Some(Key::Up)
        );
        assert_eq!(key_from_winit(&WinitKey::Named(NamedKey::Shift)), None);
    }

    #[test]
    fn the_input_method_composes_commits_and_ends() {
        let preedit = WindowEvent::Ime(Ime::Preedit("にほ".into(), Some((3, 6))));
        assert_eq!(
            HostEvent::from_winit(&preedit, 1.0),
            Some(HostEvent::Preedit {
                text: "にほ".into(),
                cursor: Some([3, 6])
            })
        );
        assert_eq!(
            HostEvent::from_winit(&WindowEvent::Ime(Ime::Disabled), 1.0),
            Some(HostEvent::Preedit {
                text: String::new(),
                cursor: None
            })
        );
        assert_eq!(
            HostEvent::from_winit(&WindowEvent::Ime(Ime::Enabled), 1.0),
            None
        );
    }
}
