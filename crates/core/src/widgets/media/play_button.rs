//! Play button — a latch key that plays and pauses (Forge `PlayButton`).
//!
//! Playing, or waiting to ([`PlayState::Loading`]), holds the key in with
//! the accent face, so the state reads at a glance without the icon. The
//! icon is play while paused, pause while playing, a spinner while loading
//! and replay once ended. One that can't play shows a red "!" and takes no
//! click.
//!
//! An optional [`progress`](PlayButton::progress) strip runs 2 px along the
//! bottom of the face, for previews in list rows.

use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::style::{StyleKey, StyleResolver};

use super::super::material::{self, Tone};
use super::super::{DrawContext, FocusId, Icon, PressState, Pressable, spinner};
use super::PlayState;

/// The progress strip's height.
const PROGRESS: f32 = 2.0;
/// The progress strip's opacity on a held key (`opacity: 0.8`).
const PROGRESS_HELD_ALPHA: f32 = 0.8;
/// How far the loading spinner turns a second, in radians.
const SPIN: f32 = 8.0;
/// The arc the spinner draws.
const SWEEP: f32 = 1.6;

/// A play / pause key. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct PlayButton {
    state: PlayState,
    size: f32,
    progress: Option<f32>,
    phase: f32,
    focus_id: Option<FocusId>,
}

impl PlayButton {
    /// The face's side at the toolbar size.
    pub const SIZE: f32 = 24.0;

    /// A key for playback in `state`, at the toolbar size.
    pub fn new(state: PlayState) -> Self {
        Self {
            state,
            size: Self::SIZE,
            progress: None,
            phase: 0.0,
            focus_id: None,
        }
    }

    /// The face's side.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Show how far it has played, `0..=1`, along the bottom of the face.
    pub fn progress(mut self, progress: f32) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Where the loading spinner has turned to, in seconds of turning (the
    /// app's clock, or a player's).
    pub fn phase(mut self, seconds: f32) -> Self {
        self.phase = seconds;
        self
    }

    /// Join the Tab ring under `id`; Space and Enter press it while focused.
    pub fn focusable(mut self, id: FocusId) -> Self {
        self.focus_id = Some(id);
        self
    }

    fn pressable(&self) -> Pressable {
        let mut key = Pressable::new()
            .tone(Tone::Latch)
            .held(self.state.is_playing())
            .name(format!("PlayButton {:?}", self.state));
        if let Some(id) = self.focus_id.filter(|_| self.state != PlayState::Error) {
            key = key.focusable(id);
        }
        key
    }

    /// The rect it needs: its face plus the plinth travel beneath.
    pub fn outer_size(&self, s: &StyleResolver) -> [f32; 2] {
        [self.size, self.size + self.pressable().travel_px(s)]
    }

    /// Draw it in `rect` (normally [`outer_size`](Self::outer_size)).
    /// `clicked` is never set while it can't play.
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> crate::Response {
        let mut response = self
            .pressable()
            .draw(rect, ctx, |key, ctx| self.draw_face(key, ctx));
        if self.state == PlayState::Error {
            response.clicked = false;
        }
        response
    }

    fn draw_face(&self, key: &PressState, ctx: &mut DrawContext) {
        let s = ctx.styles();
        let face = key.face;
        let center = (face.x + face.width * 0.5, face.y + face.height * 0.5);
        let ink = material::latch_ink(&s, key.hovered, key.down);
        let icon = match self.state {
            PlayState::Paused => Some((PhosphorIcon::Play, ink)),
            PlayState::Playing => Some((PhosphorIcon::Pause, ink)),
            PlayState::Ended => Some((PhosphorIcon::ArrowCounterClockwise, ink)),
            PlayState::Error => {
                Some((PhosphorIcon::ExclamationMark, s.color(StyleKey::DangerText)))
            }
            PlayState::Loading => None,
        };
        match icon {
            Some((icon, tint)) => {
                // The design's glyph: 42% of the key, drawn a quarter larger.
                let side = (self.size * 0.42 * 1.25).round();
                let rect = Rect::new(
                    (center.0 - side * 0.5).round(),
                    (center.1 - side * 0.5).round(),
                    side,
                    side,
                );
                Icon::new(icon).tint(tint).draw(rect, ctx.draw_list);
            }
            None => spinner(
                ctx.draw_list,
                &s,
                center,
                (self.size * 0.25).round(),
                self.phase * SPIN,
                SWEEP,
            ),
        }
        if let Some(progress) = self.progress.filter(|_| self.state != PlayState::Error) {
            let color = if key.down {
                let ink = s.color(StyleKey::OnLatch);
                [ink[0], ink[1], ink[2], ink[3] * PROGRESS_HELD_ALPHA]
            } else {
                s.color(StyleKey::AccentTick)
            };
            let width = face.width * progress.clamp(0.0, 1.0);
            if width > 0.0 {
                ctx.draw_list.chrome_rect(
                    Rect::new(face.x, face.bottom() - PROGRESS, width, PROGRESS),
                    0.0,
                    0.0,
                    color,
                    [0.0; 4],
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "play_button_tests.rs"]
mod tests;
