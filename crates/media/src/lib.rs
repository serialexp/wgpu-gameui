//! Video and audio playback for `wgpu-gameui`: reading media files, decoding
//! them on a worker thread, sound output, and the playback clock that keeps
//! picture and sound together. See `docs/design/video-playback.md`.

//!
//! Phase 2 of that plan is here: MP4 reading, H.264 and AAC decoding on a
//! worker thread, the playback clock for a video without sound, picture
//! choice and seeking. [`Player`] is the way in. Sound output (phase 3) and
//! the widgets (phase 4) come next; until then the decoded sound is let go
//! as its time passes.
//!
//! ```no_run
//! use std::time::Instant;
//! use wgpu_gameui_media::Player;
//!
//! let mut player = Player::open("clip.mp4")?;
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
mod error;
#[cfg(test)]
mod measure;
mod player;
mod schedule;
mod sps;
#[cfg(test)]
mod test_fixtures;
mod video;
mod worker;

pub use audio::AudioBlock;
pub use container::MediaReader;
pub use error::MediaError;
pub use player::{MediaInfo, PlaybackState, Player, Update};
pub use schedule::SeekMode;
pub use video::VideoFrame;
pub use worker::Ahead;
