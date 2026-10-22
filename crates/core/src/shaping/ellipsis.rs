//! Where an ellipsized line is cut. The cache lays the text out unwrapped
//! (the layout measuring the block reads too) and the ellipsis on its own,
//! and [`cut`] reads from them how much of the text fits before the '…'.

use super::ShapedLayout;

/// How many bytes of `content`, laid out unwrapped as `layout`, fit in
/// `budget` before the ellipsis, with trailing whitespace dropped. Whole
/// clusters are kept from the start of the text, in its own order, so
/// right-to-left text keeps its start rather than whatever is drawn leftmost.
pub(super) fn cut(layout: &ShapedLayout, content: &str, budget: f32) -> usize {
    // Each glyph's cluster, as ascending bytes, and its advance.
    let mut glyphs: Vec<(u32, u32, f32)> = layout
        .glyphs
        .iter()
        .map(|g| {
            (
                g.byte_start.min(g.byte_end),
                g.byte_start.max(g.byte_end),
                g.advance,
            )
        })
        .collect();
    glyphs.sort_by_key(|&(start, _, _)| start);
    let mut width = 0.0;
    let mut cut = 0;
    for cluster in glyphs.chunk_by(|a, b| a.0 == b.0) {
        width += cluster.iter().map(|&(_, _, advance)| advance).sum::<f32>();
        if width > budget {
            break;
        }
        cut = cluster.iter().map(|&(_, end, _)| end).fold(cut, u32::max);
    }
    let cut = (cut as usize).min(content.len());
    content[..cut].trim_end().len()
}

#[cfg(test)]
#[path = "ellipsis_tests.rs"]
mod tests;
