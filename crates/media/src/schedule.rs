//! The decisions playback makes, apart from any thread or clock: which
//! picture to show at a time, and where decoding starts for a seek.

use crate::container::Sample;
use crate::video::VideoFrame;
use std::collections::VecDeque;

/// Move the shown picture on to `time` (nanoseconds): the last queued
/// picture whose time has come replaces `shown`, and the ones it passes over
/// are dropped, so a machine that falls behind skips pictures instead of
/// drifting behind the sound. Returns how many were dropped, and whether
/// the shown picture changed.
pub(crate) fn advance(
    queue: &mut VecDeque<VideoFrame>,
    shown: &mut Option<VideoFrame>,
    time: i64,
) -> (usize, bool) {
    let due = queue.partition_point(|frame| frame.shown_at <= time);
    if due == 0 {
        return (0, false);
    }
    let dropped = due - 1;
    queue.drain(..dropped);
    *shown = queue.pop_front();
    (dropped, true)
}

/// How a seek lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SeekMode {
    /// On the picture at the time asked for: decoded from the keyframe
    /// before it, which takes a moment on a long run of pictures between
    /// keyframes.
    #[default]
    Exact,
    /// On the keyframe nearest the time asked for: quick, since nothing
    /// after it needs decoding first. For a scrubber being dragged, with an
    /// exact seek when the drag ends.
    NearestKeyframe,
}

/// Where decoding starts for a seek, and the time it lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SeekPlan {
    /// The video sample to decode from (a keyframe), if there is video.
    pub video_from: usize,
    /// The sound sample to decode from, if there is sound: one before the
    /// target's, as AAC needs the block before to rebuild the first one
    /// heard.
    pub sound_from: usize,
    /// The time playback lands on, in nanoseconds.
    pub target: i64,
}

/// The indices of a video track's keyframes, in decode order (which is
/// also the order they're shown in).
pub(crate) fn keyframes(video: &[Sample]) -> Vec<usize> {
    (0..video.len())
        .filter(|&index| video[index].keyframe)
        .collect()
}

/// Plan a seek to `target` (nanoseconds) in tracks with these samples (each
/// in decode order), `keyframes` being the video's from [`keyframes`]. Both
/// searches are binary, so a scrubber dragged over a long video seeks as
/// cheaply as over a short one.
pub(crate) fn plan_seek(
    video: &[Sample],
    keyframes: &[usize],
    sound: &[Sample],
    target: i64,
    mode: SeekMode,
) -> SeekPlan {
    let shown = |key: usize| video[keyframes[key]].shown_at;
    // Keyframes shown at or before the target.
    let before = keyframes.partition_point(|&index| video[index].shown_at <= target);
    let key = match mode {
        // The last of those; the first keyframe if the target is before any.
        SeekMode::Exact => before.saturating_sub(1),
        // That one or the next, whichever is nearer.
        SeekMode::NearestKeyframe => {
            if before == 0 {
                0
            } else if before < keyframes.len()
                && shown(before) - target < target - shown(before - 1)
            {
                before
            } else {
                before - 1
            }
        }
    };
    let video_from = keyframes.get(key).copied().unwrap_or(0);
    let target = match (mode, video.get(video_from)) {
        (SeekMode::NearestKeyframe, Some(keyframe)) => keyframe.shown_at,
        _ => target,
    };
    // The last block starting at or before the target, then one more back.
    let heard_from = sound.partition_point(|sample| sample.shown_at <= target);
    let sound_from = heard_from.saturating_sub(2);
    SeekPlan {
        video_from,
        sound_from,
        target,
    }
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;
