//! Completing the word before the caret: what is offered (the host's items,
//! the language's keywords, the document's own words), which match, and
//! what accepting one does to the text.

use std::collections::HashSet;

use wgpu_gameui::SyntaxTheme;

use crate::document::{Change, Document, EditKind, Selection};

/// The most offered at once.
pub(crate) const MAX_ITEMS: usize = 8;
/// The shortest word of the document's offered: a single letter is quicker
/// typed than picked.
const MIN_WORD: usize = 2;

/// What an offered item is, for its tag and ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    Function,
    Keyword,
    Type,
    Field,
    Constant,
    Variable,
}

impl CompletionKind {
    /// Its short tag, before the label.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Keyword => "kw",
            Self::Type => "ty",
            Self::Field => "fd",
            Self::Constant => "k",
            Self::Variable => "v",
        }
    }

    /// The tag's ink: the syntax colour of what it names.
    pub fn ink(self, theme: &SyntaxTheme) -> [f32; 4] {
        match self {
            Self::Function => theme.function,
            Self::Keyword => theme.keyword,
            Self::Type => theme.type_or_constant,
            Self::Field => theme.symbol,
            Self::Constant => theme.literal,
            Self::Variable => theme.variable,
        }
    }
}

/// One item completion can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// A signature or a type, shown at the right.
    pub detail: Option<String>,
}

impl Completion {
    pub fn new(label: impl Into<String>, kind: CompletionKind) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// An open completion list.
#[derive(Clone, Debug)]
pub(crate) struct Completing {
    /// Where the word being completed starts.
    pub(crate) from: usize,
    pub(crate) prefix: String,
    /// What matches it, at most [`MAX_ITEMS`].
    pub(crate) items: Vec<Completion>,
    pub(crate) chosen: usize,
    /// Everything on offer, gathered when the list opened.
    pub(crate) pool: Vec<Completion>,
}

impl Completing {
    /// A list for the word from `from`, `prefix`, out of `pool`; none when
    /// nothing in it matches.
    pub(crate) fn open(from: usize, prefix: String, pool: Vec<Completion>) -> Option<Self> {
        let items = matching(&pool, &prefix);
        (!items.is_empty()).then_some(Self {
            from,
            prefix,
            items,
            chosen: 0,
            pool,
        })
    }

    /// The word grew or shrank: match again, keeping the choice where it can.
    /// False when nothing matches any more.
    pub(crate) fn refine(&mut self, from: usize, prefix: String) -> bool {
        let chosen = self.items.get(self.chosen).map(|c| c.label.clone());
        self.items = matching(&self.pool, &prefix);
        self.from = from;
        self.prefix = prefix;
        self.chosen = chosen
            .and_then(|label| self.items.iter().position(|c| c.label == label))
            .unwrap_or(0);
        !self.items.is_empty()
    }

    /// Move the choice by `step`, wrapping round.
    pub(crate) fn step(&mut self, step: isize) {
        let count = self.items.len() as isize;
        if count > 0 {
            self.chosen = (self.chosen as isize + step).rem_euclid(count) as usize;
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word before `at` on its line, and where it starts: empty when there
/// is none, or when it starts with a digit (a number, not a name).
pub(crate) fn prefix_at(document: &Document, at: usize) -> (usize, String) {
    let line_start = document.line_start(document.line_of(at));
    let mut start = at;
    while start > line_start {
        match document.char_before(start) {
            Some(c) if is_word(c) => start -= c.len_utf8(),
            _ => break,
        }
    }
    let word = document.slice(start..at);
    if word.starts_with(|c: char| c.is_ascii_digit()) {
        (at, String::new())
    } else {
        (start, word)
    }
}

/// The names in `document`, each once, in the order they first come,
/// leaving out the word starting at `typing` (the one being completed, which
/// as typed so far is no name to offer).
pub(crate) fn identifiers(document: &Document, typing: Option<usize>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    let mut word = String::new();
    let mut start = 0;
    let mut take = |word: &mut String, start: usize| {
        if Some(start) != typing
            && word.chars().count() >= MIN_WORD
            && !word.starts_with(|c: char| c.is_ascii_digit())
            && seen.insert(word.clone())
        {
            found.push(word.clone());
        }
        word.clear();
    };
    let mut at = 0;
    for chunk in document.chunks(0..document.len()) {
        for c in chunk.chars() {
            if is_word(c) {
                if word.is_empty() {
                    start = at;
                }
                word.push(c);
            } else if !word.is_empty() {
                take(&mut word, start);
            }
            at += c.len_utf8();
        }
    }
    take(&mut word, start);
    found
}

/// Everything on offer, each label once: the host's items, then `keywords`,
/// then `words`.
pub(crate) fn pool(host: &[Completion], keywords: &[&str], words: &[String]) -> Vec<Completion> {
    let mut seen = HashSet::new();
    let keywords = keywords
        .iter()
        .map(|k| Completion::new(*k, CompletionKind::Keyword));
    let words = words
        .iter()
        .map(|w| Completion::new(w.clone(), CompletionKind::Variable));
    host.iter()
        .cloned()
        .chain(keywords)
        .chain(words)
        .filter(|c| seen.insert(c.label.clone()))
        .collect()
}

/// Those of `pool` starting with `prefix`, whatever its case, but not the
/// prefix itself: at most [`MAX_ITEMS`].
pub(crate) fn matching(pool: &[Completion], prefix: &str) -> Vec<Completion> {
    let lower = prefix.to_lowercase();
    pool.iter()
        .filter(|c| c.label != prefix && c.label.to_lowercase().starts_with(&lower))
        .take(MAX_ITEMS)
        .cloned()
        .collect()
}

/// Put `item` in place of the word from `from` to the caret: a function gets
/// `()` with the caret between, unless a `(` follows already.
pub(crate) fn accept(document: &mut Document, from: usize, item: &Completion) {
    let at = document.selection().head.max(from);
    let call = item.kind == CompletionKind::Function && document.char_after(at) != Some('(');
    let text = if call {
        format!("{}()", item.label)
    } else {
        item.label.clone()
    };
    let caret = from + item.label.len() + usize::from(call);
    document.edit(
        vec![Change {
            range: from..at,
            text,
        }],
        Selection::caret(caret),
        EditKind::Other,
    );
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
