//! Laying a long text out again after an edit from the paragraphs of a
//! layout already made, shaping only the paragraphs it doesn't have
//! (`docs/design/shared-text-layout.md`, Phase 2).
//!
//! cosmic-text lays each paragraph (each buffer line, see
//! [`text::paragraphs`](crate::text::paragraphs)) out on its own, from its
//! own text, the spec's metrics, width and alignment, so a paragraph's glyphs
//! and lines are the same wherever it stands; only its height and its byte
//! offset move it. The one exception is the direction mark a forced
//! direction puts at the head of the text: it sets the first paragraph's
//! direction and no other's, so the first paragraph is taken only from a
//! first paragraph, unless the direction is `Auto`.
//!
//! The paragraphs come from the layout under the same spec used last: while
//! someone types, the field's previous text. No paragraph is kept twice.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::ops::Range;

use cosmic_text::FontSystem;

use super::{LayoutSpec, ShapedGlyph, ShapedLayout, ShapedLine, shape_layout};
use crate::text::{TextDirection, paragraphs};

/// How long a text has to be, in bytes, for its paragraphs to be taken from
/// another layout rather than shaped with it: below this, shaping it whole
/// costs about as little as finding them (some 80 lines of 50 characters).
pub(super) const REUSE_MIN_BYTES: usize = 4096;

/// Whether `content` is long enough, and has paragraphs, to be laid out from
/// another layout's.
pub(super) fn is_long(content: &str) -> bool {
    content.len() >= REUSE_MIN_BYTES && content.contains(['\n', '\r'])
}

/// Whether `content` under `spec` can be laid out from another layout's
/// paragraphs: it is long, and each of its paragraphs is laid out as it
/// would be alone. Vertical text stacks its clusters in a string of its own,
/// an ellipsized one is a line cut from the whole, and face ranges address
/// the whole content.
pub(super) fn reusable(spec: &LayoutSpec<'_>, content: &str) -> bool {
    !spec.vertical && !spec.ellipsize && spec.shaped_faces().is_empty() && is_long(content)
}

/// One paragraph of a layout: its bytes in the content (without its line
/// break) and its lines.
#[derive(Clone, Debug)]
struct Paragraph {
    bytes: Range<usize>,
    lines: Range<usize>,
}

/// `layout`'s paragraphs, laid out from `content`: each paragraph's lines
/// are the run of lines that start at its first byte. `None` if the lines
/// don't match the paragraphs, which a layout from the same content does.
fn paragraphs_of(layout: &ShapedLayout, content: &str) -> Option<Vec<Paragraph>> {
    let ranges = paragraphs(content);
    let mut out = Vec::with_capacity(ranges.len());
    let mut line = 0;
    for bytes in ranges {
        let first = line;
        while layout
            .lines
            .get(line)
            .is_some_and(|l| l.byte_start as usize == bytes.start)
        {
            line += 1;
            // Two paragraphs can't start at one byte, but an empty one ends
            // where it starts: take its one line only.
            if bytes.is_empty() {
                break;
            }
        }
        if line == first {
            return None;
        }
        out.push(Paragraph {
            bytes,
            lines: first..line,
        });
    }
    (line == layout.lines.len()).then_some(out)
}

/// Where each paragraph of `content` comes from: `Some` paragraph of the
/// donor with the same text, or `None` to be shaped. `forced` is whether the
/// spec forces a direction, which the first paragraph alone takes.
pub(super) fn plan(content: &str, donor: &str, forced: bool) -> Vec<Option<usize>> {
    let ours = paragraphs(content);
    let theirs = paragraphs(donor);
    let mut by_text: HashMap<&str, usize> = HashMap::with_capacity(theirs.len());
    let rest = usize::from(forced);
    for (index, range) in theirs.iter().enumerate().skip(rest) {
        by_text.entry(&donor[range.clone()]).or_insert(index);
    }
    ours.iter()
        .enumerate()
        .map(|(index, range)| {
            let text = &content[range.clone()];
            if forced && index == 0 {
                (theirs.first().map(|first| &donor[first.clone()]) == Some(text)).then_some(0)
            } else {
                by_text.get(text).copied()
            }
        })
        .collect()
}

/// `content` laid out under `spec` as [`shape_layout`] would, from the
/// paragraphs of `donor_layout` (laid out from `donor` under the same spec)
/// and, in one shaping pass, the paragraphs it doesn't have. `None` if the
/// donor's lines don't match its paragraphs.
pub(super) fn reshape(
    font_system: &mut FontSystem,
    spec: &LayoutSpec<'_>,
    content: &str,
    donor: &str,
    donor_layout: &ShapedLayout,
) -> Option<ShapedLayout> {
    let forced = spec.direction != TextDirection::Auto;
    let donor_paragraphs = paragraphs_of(donor_layout, donor)?;
    let ranges = paragraphs(content);
    let sources = plan(content, donor, forced);

    // The paragraphs to shape, together: each is laid out on its own,
    // whatever stands next to it. Only the first paragraph keeps a forced
    // direction, which is the first to shape if it is one of them.
    let missing: Vec<usize> = (0..ranges.len())
        .filter(|&i| sources[i].is_none())
        .collect();
    let shaped = if missing.is_empty() {
        None
    } else {
        let joined = missing
            .iter()
            .map(|&i| &content[ranges[i].clone()])
            .collect::<Vec<_>>()
            .join("\n");
        let shaped_spec = LayoutSpec {
            direction: if missing[0] == 0 {
                spec.direction
            } else {
                TextDirection::Auto
            },
            ..*spec
        };
        let layout = shape_layout(font_system, &shaped_spec, &joined);
        let paragraphs = paragraphs_of(&layout, &joined)?;
        Some((layout, paragraphs))
    };

    let mut glyphs: Vec<ShapedGlyph> = Vec::with_capacity(donor_layout.glyphs.len());
    let mut lines: Vec<ShapedLine> = Vec::with_capacity(donor_layout.lines.len());
    let (mut width, mut height) = (0.0f32, 0.0f32);
    let mut next_missing = 0;
    for (index, range) in ranges.iter().enumerate() {
        let (layout, paragraph) = match sources[index] {
            Some(from) => (donor_layout, &donor_paragraphs[from]),
            None => {
                let (layout, paragraphs) = shaped.as_ref().expect("shaped what was missing");
                next_missing += 1;
                (layout, &paragraphs[next_missing - 1])
            }
        };
        let source_lines = &layout.lines[paragraph.lines.clone()];
        let dy = height - source_lines[0].top;
        let rebase = |byte: u32| (byte as usize - paragraph.bytes.start + range.start) as u32;
        for line in source_lines {
            let first = glyphs.len() as u32;
            glyphs.extend(
                layout.glyphs[line.glyphs.start as usize..line.glyphs.end as usize]
                    .iter()
                    .map(|glyph| ShapedGlyph {
                        rel_y: glyph.rel_y + dy,
                        byte_start: rebase(glyph.byte_start),
                        byte_end: rebase(glyph.byte_end),
                        ..*glyph
                    }),
            );
            lines.push(ShapedLine {
                top: height,
                baseline: line.baseline + dy,
                glyphs: first..glyphs.len() as u32,
                byte_start: range.start as u32,
                ..line.clone()
            });
            width = width.max(line.width);
            height += line.height;
        }
    }
    Some(ShapedLayout {
        glyphs,
        lines,
        shown: content.len() as u32,
        size: (width, height.max(spec.line_height)),
        ink: OnceCell::new(),
    })
}

#[cfg(test)]
#[path = "paragraphs_tests.rs"]
mod tests;
