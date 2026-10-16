//! Video and audio playback for `wgpu-gameui`: reading media files, decoding
//! them on a worker thread, sound output, and the playback clock that keeps
//! picture and sound together. See `docs/design/video-playback.md`.

//!
//! Phases 2 and 3 of that plan are here: MP4 reading, H.264 and AAC
//! decoding on a worker thread, picture choice and seeking, and the sound
//! played on the default output (resampled, mixed to its channels, faster or
//! slower with the pitch kept), with the picture following it. Without a
//! sound device the picture plays by the wall clock. [`Player`] is the way
//! in, and [`PlayerSource`] shows one in the media widgets (phase 4).
//!
//! ```no_run
//! use std::time::Instant;
//! use wgpu_gameui_media::Player;
//!
//! let mut player = Player::open("clip.mp4")?;
//! player.set_volume(0.8);
//! player.play(Instant::now());
//! // Each frame the app draws:
//! let update = player.update(Instant::now());
//! if update.new_picture {
//!     if let Some(frame) = player.frame() {
//!         // Upload `frame.planes` to a YUV sprite (`update_yuv_sprite`).
//!     }
//! }
//! // Ask for the next frame at `update.next_at`.
//! # Ok::<(), wgpu_gameui_media::MediaError>(())
//! ```

mod audio;
mod clock;
mod container;
mod convert;
mod device;
mod error;
#[cfg(test)]
mod measure;
mod mix;
mod player;
mod ring;
mod schedule;
mod sound;
mod source;
mod sps;
#[cfg(test)]
mod test_fixtures;
mod video;
mod worker;

pub use audio::AudioBlock;
pub use container::MediaReader;
pub use error::MediaError;
pub use player::{MediaInfo, Options, PlaybackState, Player, Update};
pub use schedule::SeekMode;
pub use source::{PictureSink, PlayerSource};
pub use video::VideoFrame;
pub use worker::Ahead;
