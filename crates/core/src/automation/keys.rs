//! Key chords as a scenario spells them: `"cmd-a"`, `"shift-tab"`, `"f5"`.

use crate::{HeldModifiers, Key};

/// One key pressed with modifiers held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    /// Held while the key goes down and up.
    pub modifiers: HeldModifiers,
    /// The key.
    pub key: Key,
}

impl Chord {
    /// The text the press types: a character or a space, unless a modifier
    /// other than Shift turns it into a shortcut.
    pub fn text(&self) -> Option<String> {
        let held = self.modifiers;
        if held.control || held.alt || held.logo {
            return None;
        }
        match self.key {
            Key::Char(c) if held.shift => Some(c.to_uppercase().collect()),
            Key::Char(c) => Some(c.to_string()),
            Key::Space => Some(" ".into()),
            _ => None,
        }
    }
}

/// Parse space-separated chords. A chord is modifiers and a key joined by
/// `-`: `cmd`, `ctrl` and `primary` are the shortcut key (Command on macOS,
/// Control elsewhere), so one file works on both; `control` is the Control
/// key itself; then `shift`, `alt` (or `option`). Keys are a single
/// character, `minus`, `enter` (`return`), `escape` (`esc`), `tab`, `space`,
/// `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, the
/// arrows `left` `right` `up` `down`, and `f1` to `f24`.
pub fn parse_keys(keys: &str) -> Result<Vec<Chord>, String> {
    let chords: Vec<Chord> = keys
        .split_whitespace()
        .map(parse_chord)
        .collect::<Result<_, _>>()?;
    if chords.is_empty() {
        return Err("there are no keys to press".into());
    }
    Ok(chords)
}

fn parse_chord(chord: &str) -> Result<Chord, String> {
    let mut parts: Vec<&str> = chord.split('-').collect();
    let key = parts.pop().unwrap_or_default();
    let mut modifiers = HeldModifiers::default();
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "ctrl" | "primary" => {
                if cfg!(target_os = "macos") {
                    modifiers.logo = true;
                } else {
                    modifiers.control = true;
                }
            }
            "control" => modifiers.control = true,
            "shift" => modifiers.shift = true,
            "alt" | "option" => modifiers.alt = true,
            _ => return Err(format!("{part:?} in {chord:?} is not a modifier")),
        }
    }
    let key = parse_key(key).ok_or_else(|| format!("{key:?} in {chord:?} is not a key"))?;
    Ok(Chord { modifiers, key })
}

fn parse_key(name: &str) -> Option<Key> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c
            .is_ascii_graphic()
            .then(|| Key::Char(c.to_ascii_lowercase()));
    }
    let lower = name.to_ascii_lowercase();
    let key = match lower.as_str() {
        "minus" => Key::Char('-'),
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        _ => {
            let n: u8 = lower.strip_prefix('f')?.parse().ok()?;
            return (1..=24).contains(&n).then_some(Key::F(n));
        }
    };
    Some(key)
}
