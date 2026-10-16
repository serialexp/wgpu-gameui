//! Fitting sound to the device's channels: the file's sound is mono or
//! stereo (all the AAC decoder gives), the device anything from one up.

/// Append `input` (frames of `from` channels, interleaved) to `out` as frames
/// of `to` channels. Mono goes to every channel; stereo to mono is the
/// average of the two; stereo on more speakers plays on the first two (front
/// left and right in every layout cpal's hosts use) and leaves the rest
/// silent. A part frame at the end of `input` is left out.
pub(crate) fn mix(input: &[f32], from: u16, to: u16, out: &mut Vec<f32>) {
    let (from, to) = (usize::from(from.max(1)), usize::from(to.max(1)));
    let frames = input.chunks_exact(from);
    out.reserve(frames.len() * to);
    if from == to {
        out.extend_from_slice(&input[..frames.len() * from]);
        return;
    }
    for frame in frames {
        match (from, to) {
            (1, _) => out.extend(std::iter::repeat_n(frame[0], to)),
            (_, 1) => out.push(frame.iter().sum::<f32>() / from as f32),
            _ => {
                let shared = from.min(to);
                out.extend_from_slice(&frame[..shared]);
                out.extend(std::iter::repeat_n(0.0, to - shared));
            }
        }
    }
}

#[cfg(test)]
#[path = "mix_tests.rs"]
mod tests;
