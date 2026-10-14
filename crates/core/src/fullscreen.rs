//! Full screen on request, like the browser's `element.requestFullscreen()`:
//! a widget asks to take over the screen, and gameui asks the app to make the
//! window full screen and tells the widget it owns the screen.
//!
//! The widget asks with [`FullscreenState::request`] (or
//! [`UiContext::request_fullscreen`](crate::UiContext::request_fullscreen))
//! and, while it owns the screen
//! ([`UiState::owns_fullscreen`](crate::UiState::owns_fullscreen), asked each
//! frame it draws), draws itself over everything else at the window's size.
//! The rest of the UI keeps drawing underneath, so leaving puts it back as it
//! was.
//!
//! The app applies the window change that
//! [`UiFrameResult::fullscreen`](crate::UiFrameResult::fullscreen) carries,
//! the way it applies the cursor, and reports the window's full-screen state
//! when it changes with
//! [`set_window_fullscreen`](FullscreenState::set_window_fullscreen).
//!
//! Full screen ends on Escape (the `cancel` intent, so a gamepad's B button
//! too) reaching the owner: a dropdown, menu or modal open over it takes the
//! key first. It also ends on [`exit`](FullscreenState::exit), when the
//! window leaves full screen by other means, and when the owner stops being
//! drawn, so a closed player can't leave the window stuck in full screen.
//!
//! A window that was already full screen when the widget asked stays full
//! screen when it leaves: only what gameui turned on is turned off.

use crate::InputState;

/// Which widget owns the screen, and the window change to ask the app for.
/// Owned by [`UiState`](crate::UiState) as its `fullscreen` field.
#[derive(Debug, Default)]
pub struct FullscreenState {
    /// The widget that owns the screen, by whatever id it is keyed by (its
    /// `FocusId` works).
    owner: Option<u64>,
    /// The owner asked or was drawn this frame.
    seen: bool,
    /// The window is full screen, as far as gameui knows.
    window: bool,
    /// gameui made the window full screen for the owner, so leaving takes it
    /// back out.
    entered: bool,
    /// The window change waiting for the frame result.
    request: Option<bool>,
    /// The window change handed to the app and not yet reported done. A
    /// report of the other state meanwhile is from before the change (macOS
    /// animates into full screen, for one) and is ignored.
    awaiting: Option<bool>,
}

impl FullscreenState {
    /// Give the screen to `owner`, taking it from any widget that had it, and
    /// ask the app to make the window full screen unless it already is.
    pub fn request(&mut self, owner: u64) {
        self.owner = Some(owner);
        self.seen = true;
        if !self.window {
            self.change_window(true);
            self.entered = true;
        }
    }

    /// End full screen: no widget owns the screen, and the window leaves full
    /// screen if gameui put it there. Does nothing when no widget owns it.
    pub fn exit(&mut self) {
        if self.owner.take().is_some() && std::mem::take(&mut self.entered) {
            self.change_window(false);
        }
    }

    /// Whether `id` owns the screen, for the owner to ask each frame it draws
    /// with the input it sees: Escape (`cancel`) in it ends full screen, and
    /// the answer is then `false`. An owner not drawn in a frame loses the
    /// screen at its end. [`UiState::owns_fullscreen`](crate::UiState::owns_fullscreen)
    /// also keeps that Escape from blurring the focused widget.
    pub fn owns(&mut self, id: u64, input: &InputState) -> bool {
        if self.owner != Some(id) {
            return false;
        }
        self.seen = true;
        if input.nav.cancel {
            self.exit();
            return false;
        }
        true
    }

    /// The widget that owns the screen, if any. Unlike [`owns`](Self::owns),
    /// asking doesn't count as drawing the owner.
    pub fn owner(&self) -> Option<u64> {
        self.owner
    }

    /// Tell gameui the window's full-screen state, when it changes: done
    /// applying a change the frame result asked for, or changed by other
    /// means (the OS's own key or button, the app's own menu). Leaving ends
    /// any widget's full screen. A report contradicting a change still in
    /// progress is taken as stale and ignored.
    pub fn set_window_fullscreen(&mut self, fullscreen: bool) {
        match self.request.or(self.awaiting) {
            Some(expected) if expected != fullscreen => return,
            Some(_) => self.awaiting = None,
            None => {}
        }
        self.window = fullscreen;
        if !fullscreen {
            self.owner = None;
            self.entered = false;
        }
    }

    /// End an owner not drawn this frame, and hand back the window change to
    /// ask the app for.
    pub(crate) fn end_frame(&mut self) -> Option<bool> {
        if !std::mem::take(&mut self.seen) {
            self.exit();
        }
        let request = self.request.take();
        if request.is_some() {
            self.awaiting = request;
        }
        request
    }

    /// Ask the app to change the window, or withdraw a change it hasn't been
    /// handed yet.
    fn change_window(&mut self, fullscreen: bool) {
        self.window = fullscreen;
        self.request = match self.request {
            Some(waiting) if waiting != fullscreen => None,
            _ => Some(fullscreen),
        };
    }
}

#[cfg(test)]
#[path = "fullscreen_tests.rs"]
mod tests;
