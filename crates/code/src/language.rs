//! The languages the editor knows: which a file is in, its grammar and
//! highlight query, and its editing rules.
//!
//! Each grammar is behind a feature of its own (all on by default); a
//! language whose grammar is left out is still known, just not coloured.

use std::sync::OnceLock;

use tree_sitter::Query;

use crate::commands::{DEFAULT_PAIRS, Rules, ends_with_word, opens_bracket};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Plain,
    Bash,
    Css,
    Go,
    Html,
    /// JavaScript with JSX.
    JavaScript,
    Json,
    Lua,
    Markdown,
    Python,
    Rust,
    Toml,
    TypeScript,
    Tsx,
    Yaml,
}

/// A language's parser and the query that colours it.
pub(crate) struct Grammar {
    pub(crate) language: tree_sitter::Language,
    pub(crate) query: Query,
}

/// Brackets and the backtick, without quotes: for prose, where a quote is
/// as often an apostrophe.
const PROSE_PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('`', '`')];
/// Without the single quote, which starts a lifetime or a character.
const RUST_PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"')];

impl Language {
    pub const ALL: [Language; 15] = [
        Language::Plain,
        Language::Bash,
        Language::Css,
        Language::Go,
        Language::Html,
        Language::JavaScript,
        Language::Json,
        Language::Lua,
        Language::Markdown,
        Language::Python,
        Language::Rust,
        Language::Toml,
        Language::TypeScript,
        Language::Tsx,
        Language::Yaml,
    ];

    /// The language of the file at `path`, by its name.
    pub fn for_path(path: &str) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path);
        match name {
            ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" | ".zprofile" => {
                return Language::Bash;
            }
            "Cargo.lock" => return Language::Toml,
            _ => {}
        }
        let Some((_, extension)) = name.rsplit_once('.') else {
            return Language::Plain;
        };
        match extension.to_ascii_lowercase().as_str() {
            "sh" | "bash" | "zsh" => Language::Bash,
            "css" => Language::Css,
            "go" => Language::Go,
            "html" | "htm" => Language::Html,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "json" => Language::Json,
            "lua" => Language::Lua,
            "md" | "markdown" => Language::Markdown,
            "py" | "pyi" => Language::Python,
            "rs" => Language::Rust,
            "toml" => Language::Toml,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "yml" | "yaml" => Language::Yaml,
            _ => Language::Plain,
        }
    }

    /// Its name, for the status strip.
    pub fn label(self) -> &'static str {
        match self {
            Language::Plain => "Plain text",
            Language::Bash => "Shell",
            Language::Css => "CSS",
            Language::Go => "Go",
            Language::Html => "HTML",
            Language::JavaScript => "JavaScript",
            Language::Json => "JSON",
            Language::Lua => "Lua",
            Language::Markdown => "Markdown",
            Language::Python => "Python",
            Language::Rust => "Rust",
            Language::Toml => "TOML",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Yaml => "YAML",
        }
    }

    /// How the keys edit it, with tab stops every `tab_size` columns.
    pub fn rules(self, tab_size: usize) -> Rules {
        let (pairs, opens_block): (_, fn(&str) -> bool) = match self {
            Language::Plain | Language::Markdown => (PROSE_PAIRS, opens_bracket),
            Language::Rust => (RUST_PAIRS, opens_bracket),
            Language::Lua => (DEFAULT_PAIRS, lua_opens_block),
            Language::Python | Language::Yaml => (DEFAULT_PAIRS, colon_opens_block),
            Language::Bash => (DEFAULT_PAIRS, shell_opens_block),
            _ => (DEFAULT_PAIRS, opens_bracket),
        };
        Rules {
            tab_size,
            auto_close: true,
            pairs,
            opens_block,
        }
    }

    /// Its grammar and highlight query, built the first time they're
    /// asked for; `None` for plain text, or a grammar left out.
    pub(crate) fn grammar(self) -> Option<&'static Grammar> {
        static GRAMMARS: [OnceLock<Option<Grammar>>; Language::ALL.len()] =
            [const { OnceLock::new() }; Language::ALL.len()];
        GRAMMARS[self as usize]
            .get_or_init(|| self.build_grammar())
            .as_ref()
    }

    fn build_grammar(self) -> Option<Grammar> {
        let (language, queries) = self.grammar_source()?;
        let source = queries.concat();
        match Query::new(&language, &source) {
            Ok(query) => Some(Grammar { language, query }),
            Err(error) => {
                log::warn!(
                    "the {} highlight query doesn't build: {error}",
                    self.label()
                );
                None
            }
        }
    }

    /// The grammar and its highlight queries, in order: a later pattern
    /// wins over an earlier one on the same node.
    fn grammar_source(self) -> Option<(tree_sitter::Language, Vec<&'static str>)> {
        match self {
            Language::Plain => None,
            #[cfg(feature = "bash")]
            Language::Bash => Some((
                tree_sitter_bash::LANGUAGE.into(),
                vec![tree_sitter_bash::HIGHLIGHT_QUERY],
            )),
            #[cfg(feature = "css")]
            Language::Css => Some((
                tree_sitter_css::LANGUAGE.into(),
                vec![tree_sitter_css::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "go")]
            Language::Go => Some((
                tree_sitter_go::LANGUAGE.into(),
                vec![tree_sitter_go::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "html")]
            Language::Html => Some((
                tree_sitter_html::LANGUAGE.into(),
                vec![tree_sitter_html::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "javascript")]
            Language::JavaScript => Some((
                tree_sitter_javascript::LANGUAGE.into(),
                vec![
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                ],
            )),
            #[cfg(feature = "json")]
            Language::Json => Some((
                tree_sitter_json::LANGUAGE.into(),
                vec![tree_sitter_json::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "lua")]
            Language::Lua => Some((
                tree_sitter_lua::LANGUAGE.into(),
                vec![tree_sitter_lua::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "markdown")]
            Language::Markdown => Some((
                tree_sitter_md::LANGUAGE.into(),
                vec![tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
            )),
            #[cfg(feature = "python")]
            Language::Python => Some((
                tree_sitter_python::LANGUAGE.into(),
                vec![tree_sitter_python::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "rust")]
            Language::Rust => Some((
                tree_sitter_rust::LANGUAGE.into(),
                vec![tree_sitter_rust::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "toml")]
            Language::Toml => Some((
                tree_sitter_toml_ng::LANGUAGE.into(),
                vec![tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
            )),
            #[cfg(feature = "typescript")]
            Language::TypeScript => Some((
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                vec![
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                ],
            )),
            #[cfg(feature = "typescript")]
            Language::Tsx => Some((
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                vec![
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                ],
            )),
            #[cfg(feature = "yaml")]
            Language::Yaml => Some((
                tree_sitter_yaml::LANGUAGE.into(),
                vec![tree_sitter_yaml::HIGHLIGHTS_QUERY],
            )),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }
}

/// Lua's blocks: brackets, `then`, `do`, `else`, `repeat`, and a
/// `function(…)` header.
fn lua_opens_block(head: &str) -> bool {
    opens_bracket(head)
        || ["then", "do", "else", "repeat"]
            .iter()
            .any(|word| ends_with_word(head, word))
        || (head.contains("function") && head.ends_with(')'))
}

/// Python's and YAML's blocks: brackets, or a line ending in `:`.
fn colon_opens_block(head: &str) -> bool {
    opens_bracket(head) || head.ends_with(':')
}

/// The shell's blocks: brackets, `then`, `do` and `else`.
fn shell_opens_block(head: &str) -> bool {
    opens_bracket(head)
        || ["then", "do", "else"]
            .iter()
            .any(|word| ends_with_word(head, word))
}

#[cfg(test)]
#[path = "language_tests.rs"]
mod tests;
