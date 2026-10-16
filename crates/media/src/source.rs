//! A [`Player`] as the media widgets' [`MediaSource`]: its pictures kept in
//! a YUV sprite, and what the user does told to it at the app's moment.
//!
//! Each frame, before drawing, the app calls [`PlayerSource::update`] with
//! the moment and where pictures go (the [`UiRenderer`], or anything else
//! that is a [`PictureSink`]). That moves playback on and puts a new picture
//! in the sprite, made the first time and again only when the picture's size
//! changes. When the source is done with, [`release`](PlayerSource::release)
//! frees the sprite.
//!
//! ```no_run
//! use std::time::Instant;
//! use wgpu_gameui::{VideoPlayer, VideoPlayerState};
//! use wgpu_gameui_media::{Player, PlayerSource};
//!
//! # fn frame(renderer: &mut wgpu_gameui::UiRenderer, ctx: &mut wgpu_gameui::DrawContext, rect: wgpu_gameui::layout::Rect) -> Result<(), wgpu_gameui_media::MediaError> {
//! let mut source = PlayerSource::new(Player::open("clip.mp4")?, Instant::now());
//! let mut state = VideoPlayerState::new();
//! // Each frame:
//! source.update(Instant::now(), renderer);
//! VideoPlayer::new(1).draw(&mut source, &mut state, rect, ctx);
//! // Done with it:
//! source.release(renderer);
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use wgpu_gameui::{MediaSource, Picture, PlayState, SpriteError, SpriteId, UiRenderer};

use crate::{PlaybackState, Player, SeekMode, VideoFrame};

/// Where a [`PlayerSource`] puts its pictures.
pub trait PictureSink {
    /// A new sprite holding `frame`.
    fn create(&mut self, frame: &VideoFrame) -> Result<SpriteId, SpriteError>;
    /// Put `frame` in `sprite`, which holds one of the same size.
    fn update(&mut self, sprite: SpriteId, frame: &VideoFrame) -> Result<(), SpriteError>;
    /// Free `sprite`.
    fn unload(&mut self, sprite: SpriteId);
}

impl PictureSink for UiRenderer {
    fn create(&mut self, frame: &VideoFrame) -> Result<SpriteId, SpriteError> {
        // Each sprite needs a name of its own.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!("wgpu-gameui-media/{}", NEXT.fetch_add(1, Ordering::Relaxed));
        self.create_yuv_sprite(
            &name,
            frame.width,
            frame.height,
            frame.planes.borrowed(),
            frame.colour,
        )
    }

    fn update(&mut self, sprite: SpriteId, frame: &VideoFrame) -> Result<(), SpriteError> {
        self.update_yuv_sprite(sprite, frame.planes.borrowed())
    }

    fn unload(&mut self, sprite: SpriteId) {
        self.unload_sprite(sprite);
    }
}

/// A [`Player`] the media widgets can show. See the [module docs](self).
pub struct PlayerSource {
    player: Player,
    /// The moment of the last update, which what the user does happens at.
    now: Instant,
    /// When the player said to update again.
    next_at: Option<Instant>,
    /// Something was asked of the player since the last update, so it
    /// should be updated now.
    asked: bool,
    picture: Option<Picture>,
    /// Its pictures couldn't be shown.
    failed: Option<String>,
}

impl PlayerSource {
    /// Show `player`, from `now`.
    pub fn new(player: Player, now: Instant) -> Self {
        Self {
            player,
            now,
            next_at: None,
            asked: true,
            picture: None,
            failed: None,
        }
    }

    /// The player.
    pub fn player(&self) -> &Player {
        &self.player
    }

    /// The player, to set its waker or anything the widgets don't.
    pub fn player_mut(&mut self) -> &mut Player {
        &mut self.player
    }

    /// Move playback on to `now`, putting a new picture in `sink`. Call it
    /// each frame before drawing.
    pub fn update(&mut self, now: Instant, sink: &mut dyn PictureSink) {
        self.now = now;
        self.asked = false;
        if self.failed.is_some() {
            self.next_at = None;
            return;
        }
        let update = self.player.update(now);
        self.next_at = update.next_at;
        if update.new_picture {
            self.show(sink);
        }
    }

    /// Free the sprite. The source shows nothing until a new picture comes.
    pub fn release(&mut self, sink: &mut dyn PictureSink) {
        if let Some(picture) = self.picture.take() {
            sink.unload(picture.sprite);
        }
    }

    /// Put the player's picture in the sprite, or let the sprite go when
    /// it has none.
    fn show(&mut self, sink: &mut dyn PictureSink) {
        let Some(frame) = self.player.frame() else {
            self.release(sink);
            return;
        };
        let size = (frame.width, frame.height);
        let shown = match self.picture {
            Some(picture) if (picture.width, picture.height) == size => {
                sink.update(picture.sprite, frame).map(|()| picture.sprite)
            }
            _ => {
                if let Some(picture) = self.picture.take() {
                    sink.unload(picture.sprite);
                }
                sink.create(frame)
            }
        };
        match shown {
            Ok(sprite) => {
                self.picture = Some(Picture {
                    sprite,
                    width: size.0,
                    height: size.1,
                })
            }
            Err(error) => {
                self.failed = Some(format!("its pictures can't be shown: {error}"));
                self.player.pause(self.now);
                self.next_at = None;
                self.release(sink);
            }
        }
    }

    fn asked(&mut self) {
        self.asked = true;
    }
}

/// How long playback at `speed` takes from `time` to its next whole second,
/// when a time readout changes.
fn until_next_second(time: Duration, speed: f64) -> Duration {
    let left = Duration::from_secs(1) - Duration::from_nanos(time.subsec_nanos().into());
    if speed > 0.0 {
        left.div_f64(speed)
    } else {
        left
    }
}

impl MediaSource for PlayerSource {
    fn state(&self) -> PlayState {
        if self.failed.is_some() {
            return PlayState::Error;
        }
        match self.player.state() {
            PlaybackState::Paused => PlayState::Paused,
            PlaybackState::Playing => PlayState::Playing,
            PlaybackState::Loading => PlayState::Loading,
            PlaybackState::Ended => PlayState::Ended,
            PlaybackState::Error => PlayState::Error,
        }
    }

    fn time(&self) -> Duration {
        self.player.time(self.now)
    }

    fn length(&self) -> Option<Duration> {
        Some(self.player.info().duration)
    }

    fn buffered(&self) -> Option<Duration> {
        Some(self.player.buffered())
    }

    fn volume(&self) -> f32 {
        self.player.volume()
    }

    fn muted(&self) -> bool {
        self.player.muted()
    }

    fn speed(&self) -> f64 {
        self.player.speed()
    }

    fn error(&self) -> Option<String> {
        self.failed
            .clone()
            .or_else(|| self.player.error().map(ToString::to_string))
    }

    fn picture(&self) -> Option<Picture> {
        self.picture
    }

    fn next_change(&self) -> Option<Duration> {
        if self.asked {
            return Some(Duration::ZERO);
        }
        let picture = self
            .next_at
            .map(|at| at.saturating_duration_since(self.now));
        let readout = (self.state() == PlayState::Playing)
            .then(|| until_next_second(self.time(), self.speed()));
        match (picture, readout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn play(&mut self) {
        self.player.play(self.now);
        self.asked();
    }

    fn pause(&mut self) {
        self.player.pause(self.now);
        self.asked();
    }

    fn seek(&mut self, to: Duration) {
        self.player.seek(to, SeekMode::Exact, self.now);
        self.asked();
    }

    fn scrub(&mut self, to: Duration) {
        self.player.seek(to, SeekMode::NearestKeyframe, self.now);
        self.asked();
    }

    fn set_volume(&mut self, volume: f32) {
        self.player.set_volume(volume);
    }

    fn set_muted(&mut self, muted: bool) {
        self.player.set_muted(muted);
    }

    fn set_speed(&mut self, speed: f64) {
        self.player.set_speed(speed, self.now);
        self.asked();
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
