//! Colouring with Tree-sitter: the document parsed once, re-parsed after
//! each edit reusing the old tree, and the highlight query run only over
//! the text asked for (the lines in view).
//!
//! Where captures overlap, the innermost wins; of two on the same node, the
//! later pattern, as in Tree-sitter's own highlighter.

use std::ops::Range;

use tree_sitter::{InputEdit, Parser, QueryCursor, StreamingIterator, Tree};
use wgpu_gameui::SyntaxTheme;

use crate::document::{Document, TextEdit};
use crate::language::Language;

/// A coloured piece of the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub ink: [f32; 4],
}

/// A document's syntax tree, kept up to date with its edits.
pub struct Syntax {
    language: Language,
    /// With the tree, `None` for a language without a grammar.
    parsed: Option<(Parser, Tree)>,
}

impl std::fmt::Debug for Syntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Syntax")
            .field("language", &self.language)
            .field("parsed", &self.parsed.is_some())
            .finish()
    }
}

impl Syntax {
    /// Parse `document` as `language`. The edits it was waiting to tell
    /// the parser are taken: this parse has them.
    pub fn new(language: Language, document: &mut Document) -> Self {
        document.take_edits();
        let parsed = language.grammar().and_then(|grammar| {
            let mut parser = Parser::new();
            parser.set_language(&grammar.language).ok()?;
            let tree = parse(&mut parser, document, None)?;
            Some((parser, tree))
        });
        Self { language, parsed }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Catch up with `document`'s edits since the last parse.
    pub fn update(&mut self, document: &mut Document) {
        let edits = document.take_edits();
        let Some((parser, tree)) = &mut self.parsed else {
            return;
        };
        if edits.is_empty() {
            return;
        }
        for edit in &edits {
            tree.edit(&input_edit(edit));
        }
        if let Some(new) = parse(parser, document, Some(tree)) {
            *tree = new;
        }
    }

    /// The coloured pieces of `range`, in order and apart, with `theme`'s
    /// inks.
    pub fn spans(
        &self,
        document: &Document,
        range: Range<usize>,
        theme: &SyntaxTheme,
    ) -> Vec<Span> {
        let (Some((_, tree)), Some(grammar)) = (&self.parsed, self.language.grammar()) else {
            return Vec::new();
        };
        if range.is_empty() {
            return Vec::new();
        }
        let inks: Vec<Option<[f32; 4]>> = grammar
            .query
            .capture_names()
            .iter()
            .map(|name| theme.for_capture(name))
            .collect();
        // Each capture with its node's range and pattern, in the order they
        // are painted: outer before inner, and earlier patterns before later
        // ones on the same node, so the one painted last wins.
        let mut found: Vec<(Range<usize>, usize, [f32; 4])> = Vec::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.clone());
        let text = |node: tree_sitter::Node| {
            document
                .chunks(node.byte_range())
                .map(str::as_bytes)
                .collect::<Vec<_>>()
                .into_iter()
        };
        let mut captures = cursor.captures(&grammar.query, tree.root_node(), text);
        while let Some((found_match, index)) = captures.next() {
            let capture = found_match.captures()[*index];
            if let Some(ink) = inks[capture.index as usize] {
                found.push((capture.node.byte_range(), found_match.pattern_index, ink));
            }
        }
        found.sort_by(|a, b| {
            a.0.start
                .cmp(&b.0.start)
                .then(b.0.end.cmp(&a.0.end))
                .then(a.1.cmp(&b.1))
        });
        paint(&found, range)
    }
}

/// Paint `found` in order over `range`, the last paint of each byte
/// winning, and gather the result into spans.
fn paint(found: &[(Range<usize>, usize, [f32; 4])], range: Range<usize>) -> Vec<Span> {
    let mut canvas: Vec<Option<u32>> = vec![None; range.len()];
    for (index, (piece, _, _)) in found.iter().enumerate() {
        let start = piece.start.max(range.start) - range.start;
        let end = piece.end.min(range.end).saturating_sub(range.start);
        if start < end {
            canvas[start..end].fill(Some(index as u32));
        }
    }
    let mut spans: Vec<Span> = Vec::new();
    for (offset, paint) in canvas.into_iter().enumerate() {
        let Some(index) = paint else {
            continue;
        };
        let at = range.start + offset;
        let ink = found[index as usize].2;
        match spans.last_mut() {
            Some(last) if last.range.end == at && last.ink == ink => last.range.end = at + 1,
            _ => spans.push(Span {
                range: at..at + 1,
                ink,
            }),
        }
    }
    spans
}

/// Parse `document`, reading it piece by piece.
fn parse(parser: &mut Parser, document: &Document, old: Option<&Tree>) -> Option<Tree> {
    parser.parse_with_options(&mut |at, _| document.chunk_at(at).as_bytes(), old, None)
}

fn input_edit(edit: &TextEdit) -> InputEdit {
    let point = |point: crate::document::Point| tree_sitter::Point {
        row: point.row,
        column: point.column,
    };
    InputEdit {
        start_byte: edit.start,
        old_end_byte: edit.old_end,
        new_end_byte: edit.new_end,
        start_position: point(edit.start_point),
        old_end_position: point(edit.old_end_point),
        new_end_position: point(edit.new_end_point),
    }
}

#[cfg(test)]
#[path = "highlight_tests.rs"]
mod tests;
