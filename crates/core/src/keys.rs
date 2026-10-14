//! The generic key vocabulary: [`Key`] names a key, [`KeyState`] records which
//! keys are held and which went down or up this frame.
//!
//! [`InputState`](crate::InputState) has named flags for the keys text editing
//! and focus navigation need (`key_left`, `key_space`, …). Shortcuts need any
//! key — a timeline's `M` for "add marker", `,`/`.` for previous/next key,
//! menu accelerators — so the host also reports every key it maps through
//! [`KeyState::key_event`], into [`InputState::keys`](crate::InputState::keys).
//!
//! The host bridge is one match from the windowing system's physical key to a
//! [`Key`]; with winit:
//!
//! ```ignore
//! WindowEvent::KeyboardInput { event, .. } => {
//!     if let PhysicalKey::Code(code) = event.physical_key {
//!         let key = match code {
//!             KeyCode::KeyA => Some(Key::Char('A')), // … through KeyZ
//!             KeyCode::Digit0 => Some(Key::Char('0')), // … through Digit9
//!             KeyCode::Comma => Some(Key::Char(',')),
//!             KeyCode::Period => Some(Key::Char('.')),
//!             KeyCode::Minus => Some(Key::Char('-')),
//!             KeyCode::Equal => Some(Key::Char('=')),
//!             KeyCode::ArrowLeft => Some(Key::Left),
//!             KeyCode::F1 => Some(Key::F(1)),
//!             _ => None,
//!         };
//!         if let Some(key) = key {
//!             input.keys.key_event(key, event.state == ElementState::Pressed);
//!         }
//!     }
//! }
//! ```
//!
//! Key repeat is the host's business: a repeated press arrives as another
//! press while the key is already down, and counts as a press edge again.

/// A key on its own, independent of modifiers. Used by
/// [`Accelerator`](crate::Accelerator) and [`KeyState`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// An ASCII character. Case is folded, so `'A'` and `'a'` are the same key.
    Char(char),
    /// A function key, `F(1)`..=`F(24)`.
    F(u8),
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Enter/Return.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Insert.
    Insert,
    /// Space.
    Space,
}

/// The named (non-character, non-function) keys in index order.
const NAMED: [Key; 15] = [
    Key::Left,
    Key::Right,
    Key::Up,
    Key::Down,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::Enter,
    Key::Escape,
    Key::Tab,
    Key::Backspace,
    Key::Delete,
    Key::Insert,
    Key::Space,
];
/// Characters take slots 0..128 (ASCII), function keys 128..152, named keys
/// 152..167.
const F_BASE: usize = 128;
const NAMED_BASE: usize = F_BASE + 24;
const SLOTS: usize = NAMED_BASE + NAMED.len();
const WORDS: usize = SLOTS.div_ceil(64);

impl Key {
    /// This key's bit, or `None` for a key outside the vocabulary (a
    /// non-ASCII character, `F(0)`, `F(25)`…).
    fn slot(self) -> Option<usize> {
        match self {
            Key::Char(c) if c.is_ascii() => Some(c.to_ascii_uppercase() as usize),
            Key::Char(_) => None,
            Key::F(n @ 1..=24) => Some(F_BASE + n as usize - 1),
            Key::F(_) => None,
            named => NAMED
                .iter()
                .position(|k| *k == named)
                .map(|i| NAMED_BASE + i),
        }
    }

    /// The key at `slot` (the inverse of [`slot`](Self::slot)).
    fn from_slot(slot: usize) -> Key {
        if slot < F_BASE {
            Key::Char(slot as u8 as char)
        } else if slot < NAMED_BASE {
            Key::F((slot - F_BASE + 1) as u8)
        } else {
            NAMED[slot - NAMED_BASE]
        }
    }
}

/// Which keys are held, and which went down or up this frame.
///
/// Three fixed bitsets, so recording and querying never allocate. `down` is
/// held state, like the modifier flags; the press and release edges are
/// per-frame and cleared by [`end_frame`](Self::end_frame) (which
/// [`InputState::end_frame`](crate::InputState::end_frame) calls).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyState {
    down: [u64; WORDS],
    pressed: [u64; WORDS],
    released: [u64; WORDS],
}

fn bit(slot: usize) -> (usize, u64) {
    (slot / 64, 1u64 << (slot % 64))
}

impl KeyState {
    /// Record a key going down (`down == true`) or up. The host calls this for
    /// every key event it maps; a press while the key is already down (key
    /// repeat) is a press edge again.
    ///
    /// A key outside the vocabulary (a non-ASCII `Char`) is ignored, and a
    /// debug assertion, because a host should not map one.
    pub fn key_event(&mut self, key: Key, down: bool) {
        let Some(slot) = key.slot() else {
            debug_assert!(false, "{key:?} is outside the key vocabulary");
            return;
        };
        let (word, mask) = bit(slot);
        if down {
            self.down[word] |= mask;
            self.pressed[word] |= mask;
        } else {
            self.down[word] &= !mask;
            self.released[word] |= mask;
        }
    }

    fn test(set: &[u64; WORDS], key: Key) -> bool {
        key.slot().is_some_and(|slot| {
            let (word, mask) = bit(slot);
            set[word] & mask != 0
        })
    }

    /// The key is held.
    pub fn is_down(&self, key: Key) -> bool {
        Self::test(&self.down, key)
    }

    /// The key went down this frame.
    pub fn was_pressed(&self, key: Key) -> bool {
        Self::test(&self.pressed, key)
    }

    /// The key came up this frame.
    pub fn was_released(&self, key: Key) -> bool {
        Self::test(&self.released, key)
    }

    /// The keys that went down this frame, in `Key` order (characters,
    /// function keys, then named keys). Characters come out upper-case.
    pub fn pressed(&self) -> impl Iterator<Item = Key> + '_ {
        (0..SLOTS)
            .filter(|&slot| {
                let (word, mask) = bit(slot);
                self.pressed[word] & mask != 0
            })
            .map(Key::from_slot)
    }

    /// Clear this frame's press and release edges; held keys stay held.
    pub fn end_frame(&mut self) {
        self.pressed = [0; WORDS];
        self.released = [0; WORDS];
    }

    /// The same held keys with no edges: what a layer under a modal or popup
    /// sees (see [`InputState::consumed`](crate::InputState::consumed)).
    #[must_use]
    pub fn without_edges(&self) -> Self {
        Self {
            down: self.down,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_round_trips_through_its_slot() {
        let mut keys: Vec<Key> = (0u8..128).map(|c| Key::Char(c as char)).collect();
        keys.retain(|k| !matches!(k, Key::Char(c) if c.is_ascii_lowercase()));
        keys.extend((1..=24).map(Key::F));
        keys.extend(NAMED);
        let mut seen = std::collections::HashSet::new();
        for key in keys {
            let slot = key.slot().expect("in the vocabulary");
            assert!(slot < SLOTS, "{key:?} slot {slot}");
            assert!(seen.insert(slot), "{key:?} shares slot {slot}");
            assert_eq!(Key::from_slot(slot), key);
        }
        assert_eq!(seen.len(), SLOTS - 26, "lower-case letters fold away");
    }

    #[test]
    fn characters_fold_case() {
        let mut state = KeyState::default();
        state.key_event(Key::Char('m'), true);
        assert!(state.was_pressed(Key::Char('M')));
        assert!(state.is_down(Key::Char('m')));
        assert_eq!(state.pressed().collect::<Vec<_>>(), [Key::Char('M')]);
    }

    #[test]
    fn edges_clear_at_end_of_frame_and_held_keys_stay() {
        let mut state = KeyState::default();
        state.key_event(Key::Char(','), true);
        state.key_event(Key::F(5), true);
        state.key_event(Key::F(5), false);
        assert!(state.was_pressed(Key::Char(',')));
        assert!(state.was_pressed(Key::F(5)) && state.was_released(Key::F(5)));
        assert!(!state.is_down(Key::F(5)));
        state.end_frame();
        assert!(!state.was_pressed(Key::Char(',')));
        assert!(!state.was_released(Key::F(5)));
        assert!(state.is_down(Key::Char(',')));
    }

    #[test]
    fn pressed_lists_keys_in_vocabulary_order() {
        let mut state = KeyState::default();
        state.key_event(Key::Space, true);
        state.key_event(Key::F(2), true);
        state.key_event(Key::Char('='), true);
        assert_eq!(
            state.pressed().collect::<Vec<_>>(),
            [Key::Char('='), Key::F(2), Key::Space]
        );
    }

    #[test]
    fn without_edges_keeps_only_held_keys() {
        let mut state = KeyState::default();
        state.key_event(Key::Delete, true);
        let under = state.without_edges();
        assert!(under.is_down(Key::Delete));
        assert!(!under.was_pressed(Key::Delete));
    }

    #[test]
    fn keys_outside_the_vocabulary_never_match() {
        let state = KeyState::default();
        assert!(!state.was_pressed(Key::F(0)));
        assert!(!state.is_down(Key::Char('é')));
    }
}
