//! Where a caret can stand in laid-out text, and its glyphs in visual order,
//! worked out from the [`ShapedLayout`] it is drawn with
//! (`docs/design/shared-text-layout.md`), so a caret can't stand anywhere but
//! on the glyphs drawn. [`SharedFontSystem`](crate::shaping::SharedFontSystem)
//! keeps both with the layout.
//!
//! Carets stand at cluster edges. A line's first caret is at its left edge and
//! its last at its right; between them, one before each new cluster. Where a
//! wrap breaks a word, the byte it breaks at ends one line and starts the next
//! ([`caret_for_byte`](crate::text::caret_for_byte) takes the first). Text an
//! ellipsis cut off has no carets.

use crate::shaping::{ShapedGlyph, ShapedLayout, ShapedLine};
use crate::text::{CaretPos, VisualGlyph};

/// The glyphs of `line` that show content: not the ellipsis.
fn shown_glyphs<'l>(
    layout: &'l ShapedLayout,
    line: &ShapedLine,
) -> impl Iterator<Item = &'l ShapedGlyph> {
    let shown = layout.shown;
    layout.glyphs[line.glyphs.start as usize..line.glyphs.end as usize]
        .iter()
        .filter(move |g| g.byte_start < shown)
}

/// One [`CaretPos`] per cluster boundary of `layout`, line by line.
pub(crate) fn caret_positions(layout: &ShapedLayout) -> Vec<CaretPos> {
    let shown = layout.shown as usize;
    let mut out: Vec<CaretPos> = Vec::with_capacity(layout.glyphs.len() + 2 * layout.lines.len());
    for (index, line) in layout.lines.iter().enumerate() {
        let at = |byte: usize, x: f32| CaretPos {
            byte,
            x,
            line: index,
            line_top: line.top,
            line_height: line.height,
        };
        let mut glyphs = shown_glyphs(layout, line).peekable();
        let first = glyphs.peek().map_or(line.byte_start, |g| g.byte_start);
        out.push(at(first as usize, line.left));
        let mut last = None;
        for glyph in glyphs {
            let byte = glyph.byte_start as usize;
            if out.last().map(|p| p.byte) != Some(byte) {
                out.push(at(byte, glyph.cell_x));
            }
            last = Some(glyph);
        }
        if let Some(last) = last {
            let end = (last.byte_end as usize).min(shown);
            // A line an ellipsis cut ends before the ellipsis.
            let cut = layout.glyphs[line.glyphs.start as usize..line.glyphs.end as usize]
                .iter()
                .any(|g| g.byte_start >= layout.shown);
            let right = if cut {
                last.cell_x + last.advance
            } else {
                line.right
            };
            if out.last().map(|p| p.byte) != Some(end) {
                out.push(at(end, right));
            }
        }
    }
    // The end of the text is always a place for the caret.
    if let Some(&end) = out.last()
        && end.byte < shown
    {
        out.push(CaretPos { byte: shown, ..end });
    }
    out
}

/// `layout`'s glyphs in visual order, line by line, each with its cell.
pub(crate) fn visual_glyphs(layout: &ShapedLayout) -> Vec<VisualGlyph> {
    let mut out = Vec::with_capacity(layout.glyphs.len());
    for (index, line) in layout.lines.iter().enumerate() {
        out.extend(shown_glyphs(layout, line).map(|g| VisualGlyph {
            byte_start: g.byte_start.min(g.byte_end) as usize,
            byte_end: g.byte_start.max(g.byte_end) as usize,
            x: g.cell_x,
            w: g.advance,
            line: index,
            line_top: line.top,
            line_height: line.height,
            rtl: g.rtl,
        }));
    }
    out
}

#[cfg(test)]
#[path = "text_carets_tests.rs"]
mod tests;
