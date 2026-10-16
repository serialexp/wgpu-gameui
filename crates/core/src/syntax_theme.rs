//! The inks source code is coloured with, by what each piece of it is: the
//! Forge design system's `--syn-*` tokens (`tokens/syntax.css`), one per
//! field. Cyan is never a syntax colour; it means caret and selection.
//!
//! Highlighting itself (Tree-sitter) is behind features; the palette isn't,
//! so an editor can carry one without the parser.

use crate::color::{hex, oklch};

/// Reusable palette for Tree-sitter capture categories, as sRGB-encoded RGBA values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntaxTheme {
    /// Comments and documentation.
    pub comment: [f32; 4],
    /// Strings and character literals.
    pub string: [f32; 4],
    /// Numeric literals and booleans.
    pub literal: [f32; 4],
    /// Language keywords and preprocessor forms.
    pub keyword: [f32; 4],
    /// Function, method, and constructor names.
    pub function: [f32; 4],
    /// Types, constants, and attributes.
    pub type_or_constant: [f32; 4],
    /// Parameters, properties, and labels.
    pub symbol: [f32; 4],
    /// Operators and punctuation.
    pub operator: [f32; 4],
    /// Variables not covered by a more specific capture.
    pub variable: [f32; 4],
    /// Conditional keywords (`if`/`then`/`else`); the same as
    /// [`keyword`](Self::keyword) by default.
    pub conditional: [f32; 4],
    /// Loop keywords (`while`/`for`/`repeat`); the same as
    /// [`keyword`](Self::keyword) by default.
    pub repeat: [f32; 4],
}

impl Default for SyntaxTheme {
    /// Forge's inks.
    fn default() -> Self {
        let keyword = oklch(0.76, 0.12, 315.0, 1.0);
        Self {
            comment: oklch(0.6, 0.025, 160.0, 1.0),
            string: oklch(0.8, 0.11, 135.0, 1.0),
            literal: oklch(0.8, 0.11, 65.0, 1.0),
            keyword,
            function: oklch(0.78, 0.1, 260.0, 1.0),
            type_or_constant: oklch(0.82, 0.08, 170.0, 1.0),
            symbol: oklch(0.84, 0.08, 90.0, 1.0),
            // As `--ink-glyph`.
            operator: hex(0x9aa2aa),
            // As `--ink-cell`.
            variable: hex(0xc6ced5),
            conditional: keyword,
            repeat: keyword,
        }
    }
}

impl SyntaxTheme {
    /// The ink for a highlight query's capture name (`keyword`,
    /// `function.method`, `markup.heading`), or `None` for one that keeps the
    /// text's own colour.
    ///
    /// A dotted name takes the ink of its first part that names one, so a
    /// capture inherits its base: `keyword.function` is a keyword,
    /// `variable.parameter` a variable. This is how Tree-sitter's highlighter
    /// resolves the names [`SyntaxHighlighting`](crate::SyntaxHighlighting)
    /// is configured with, which are all one part long and listed bases first.
    ///
    /// `constant.builtin` is the exception: grammars give it to `true`,
    /// `nil` and `None`, and Rust to its numbers, which are all literals.
    pub fn for_capture(&self, name: &str) -> Option<[f32; 4]> {
        if name == "constant.builtin" || name.starts_with("constant.builtin.") {
            return Some(self.literal);
        }
        name.split('.').find_map(|part| self.for_part(part))
    }

    fn for_part(&self, part: &str) -> Option<[f32; 4]> {
        Some(match part {
            "comment" => self.comment,
            "string" => self.string,
            "number" | "boolean" | "escape" => self.literal,
            "keyword" | "preproc" | "tag" => self.keyword,
            "conditional" => self.conditional,
            "repeat" => self.repeat,
            "function" | "method" | "constructor" => self.function,
            "type" | "constant" | "attribute" | "module" | "namespace" => self.type_or_constant,
            "parameter" | "property" | "label" => self.symbol,
            "operator" | "punctuation" => self.operator,
            "variable" => self.variable,
            // Markdown's names: headings, code, links.
            "title" | "heading" => self.keyword,
            "literal" | "raw" => self.string,
            "uri" | "url" | "link" => self.function,
            "reference" => self.symbol,
            _ => return None,
        })
    }
}

#[cfg(test)]
#[path = "syntax_theme_tests.rs"]
mod tests;
