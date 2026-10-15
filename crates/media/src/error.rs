//! What can go wrong opening or playing a file.

use std::fmt;

/// Why a file can't be played.
#[derive(Debug)]
pub enum MediaError {
    /// The file couldn't be read.
    Io(std::io::Error),
    /// The file isn't an MP4 this reader understands, or its data is damaged.
    Damaged(String),
    /// The file is fine, but uses something this player doesn't play: another
    /// codec, interlaced or 10-bit H.264, a kind of AAC other than AAC-LC.
    Unsupported(String),
    /// Something panicked while playing: a decoder on this file's data, or
    /// the app's [waker](crate::Player::set_waker). The panic was caught on
    /// the worker; nothing else is affected.
    Panicked(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Io(error) => write!(f, "can't read the file: {error}"),
            MediaError::Damaged(what) => write!(f, "the file is damaged: {what}"),
            MediaError::Unsupported(what) => write!(f, "not supported: {what}"),
            MediaError::Panicked(what) => write!(f, "playing failed: {what}"),
        }
    }
}

impl std::error::Error for MediaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MediaError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MediaError {
    fn from(error: std::io::Error) -> Self {
        MediaError::Io(error)
    }
}

impl From<re_mp4::Error> for MediaError {
    fn from(error: re_mp4::Error) -> Self {
        match error {
            re_mp4::Error::Io(error) => MediaError::Io(error),
            other => MediaError::Damaged(other.to_string()),
        }
    }
}
