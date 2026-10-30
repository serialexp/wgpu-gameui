//! Completing the word before the caret: what is offered (the host's items,
//! the language's keywords, the document's own words), which match, and
//! what accepting one does to the text.
//!
//! The host's items come from a language server, asked when the editor
//! reports an ask ([`CompletionAsk`]) and handed back with
//! [`CodeEditor::offer`](crate::CodeEditor::offer). Their edits are in the
//! text as it was at the ask; while the list is open, the only change made
//! since is the word growing or shrinking at the caret (any other edit
//! closes it), so they are moved to the text as it is now exactly
//! ([`host_changes`]).

use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;

use wgpu_gameui::SyntaxTheme;

use crate::document::{Change, Document, EditKind, Selection};

/// The most items an open list holds; it shows fewer and scrolls.
pub(crate) const MAX_ITEMS: usize = 200;
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

/// Replace `range` (bytes) with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionEdit {
    pub range: Range<usize>,
    pub text: String,
}

impl CompletionEdit {
    pub fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
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
    /// What accepting it does, in the text the ask was made in; the label
    /// over the word when there is none.
    pub edit: Option<CompletionEdit>,
    /// More it does elsewhere (an import), in the same text.
    pub extra_edits: Vec<CompletionEdit>,
    /// What the typed word is matched against, when not the label.
    pub filter_text: Option<String>,
    /// Where it goes among the host's items, when not by its label.
    pub sort_text: Option<String>,
    /// The host's key for resolving it ([`CodeEditorOutput::resolve`]):
    /// it has more to say (its extra edits) once asked.
    ///
    /// [`CodeEditorOutput::resolve`]: crate::CodeEditorOutput::resolve
    pub resolve: Option<u64>,
}

impl Completion {
    pub fn new(label: impl Into<String>, kind: CompletionKind) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: None,
            edit: None,
            extra_edits: Vec::new(),
            filter_text: None,
            sort_text: None,
            resolve: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_edit(mut self, range: Range<usize>, text: impl Into<String>) -> Self {
        self.edit = Some(CompletionEdit::new(range, text));
        self
    }

    pub fn with_extra_edit(mut self, range: Range<usize>, text: impl Into<String>) -> Self {
        self.extra_edits.push(CompletionEdit::new(range, text));
        self
    }

    pub fn with_filter_text(mut self, text: impl Into<String>) -> Self {
        self.filter_text = Some(text.into());
        self
    }

    pub fn with_sort_text(mut self, text: impl Into<String>) -> Self {
        self.sort_text = Some(text.into());
        self
    }

    pub fn with_resolve(mut self, key: u64) -> Self {
        self.resolve = Some(key);
        self
    }

    /// What the typed word is matched against.
    fn filter(&self) -> &str {
        self.filter_text.as_deref().unwrap_or(&self.label)
    }
}

/// The host's items for one ask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionList {
    /// More would match as the word grows: ask again when it changes.
    pub incomplete: bool,
    pub items: Vec<Completion>,
}

/// What made an ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Ctrl-Space.
    Invoked,
    /// One of the host's trigger characters, typed.
    Character(char),
    /// The word changed under an incomplete list.
    Incomplete,
}

/// An ask for the host's items: answer it with
/// [`CodeEditor::offer`](crate::CodeEditor::offer) and its `id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompletionAsk {
    /// Counted up per editor: only the latest is used.
    pub id: u64,
    /// The caret's byte offset.
    pub at: usize,
    /// Where the word being completed starts (`at` when there is none).
    pub from: usize,
    pub trigger: CompletionTrigger,
    /// The document's [`version`](Document::version) asked in: the text the
    /// host must have sent before it asks, and the one its edits are in.
    pub version: u64,
}

/// An ask to resolve the chosen item: answer it with
/// [`CodeEditor::resolved`](crate::CodeEditor::resolved).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolveAsk {
    /// The ask whose list holds it.
    pub id: u64,
    pub key: u64,
    /// The document's version: resolve against that text, and send it first.
    pub version: u64,
}

/// What resolving an item gave: its extra edits, in the text of `version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) key: u64,
    pub(crate) version: u64,
    pub(crate) extra_edits: Vec<CompletionEdit>,
}

/// An open completion list.
#[derive(Clone, Debug)]
pub(crate) struct Completing {
    /// Where the word being completed starts.
    pub(crate) from: usize,
    pub(crate) prefix: String,
    /// What matches it, at most [`MAX_ITEMS`]: the host's first.
    pub(crate) items: Vec<Completion>,
    /// How many of `items` are the host's.
    pub(crate) hosted: usize,
    pub(crate) chosen: usize,
    /// The keywords and the document's words, gathered when the list
    /// opened; none when a trigger character opened it.
    pub(crate) pool: Vec<Completion>,
    /// The host's items for `offered`, in their order.
    pub(crate) host: Vec<Completion>,
    /// The latest ask made for this list, and the one `host` answers.
    pub(crate) asked: Option<CompletionAsk>,
    pub(crate) offered: Option<CompletionAsk>,
    /// The host's list was incomplete: a change to the word asks again.
    pub(crate) incomplete: bool,
    /// What resolving has given, for the host's items.
    pub(crate) resolved: Vec<Resolved>,
    /// The widest row, measured once per set of items.
    pub(crate) widest: Cell<Option<f32>>,
}

impl Completing {
    /// A list for the word from `from`, `prefix`, out of `pool`, waiting on
    /// `asked` if the host was asked; none when nothing matches and nothing
    /// is awaited.
    pub(crate) fn open(
        from: usize,
        prefix: String,
        pool: Vec<Completion>,
        asked: Option<CompletionAsk>,
    ) -> Option<Self> {
        let mut open = Self {
            from,
            prefix,
            items: Vec::new(),
            hosted: 0,
            chosen: 0,
            pool,
            host: Vec::new(),
            asked,
            offered: None,
            incomplete: false,
            resolved: Vec::new(),
            widest: Cell::new(None),
        };
        open.rematch();
        open.alive().then_some(open)
    }

    /// Whether it has items to show, or an answer still to come.
    pub(crate) fn alive(&self) -> bool {
        !self.items.is_empty() || self.waiting()
    }

    /// The host was asked and hasn't answered that ask yet.
    pub(crate) fn waiting(&self) -> bool {
        self.asked
            .is_some_and(|asked| self.offered.map(|o| o.id) != Some(asked.id))
    }

    /// The word grew or shrank: match again, keeping the choice where it can.
    /// False when nothing matches any more and nothing is awaited.
    pub(crate) fn refine(&mut self, from: usize, prefix: String) -> bool {
        self.from = from;
        self.prefix = prefix;
        self.rematch();
        self.alive()
    }

    /// The host's answer to `ask`.
    pub(crate) fn offer(&mut self, ask: CompletionAsk, list: CompletionList) {
        let mut host = list.items;
        // Stable, so items with the same sort text keep the host's order.
        host.sort_by(|a, b| {
            let key = |c: &Completion| c.sort_text.clone().unwrap_or_else(|| c.label.clone());
            key(a).cmp(&key(b))
        });
        self.host = host;
        self.offered = Some(ask);
        self.incomplete = list.incomplete;
        self.resolved.clear();
        // A new answer starts the choice at its first item.
        self.items.clear();
        self.rematch();
    }

    fn rematch(&mut self) {
        let chosen = self.items.get(self.chosen).map(|c| c.label.clone());
        let (items, hosted) = matching(&self.host, &self.pool, &self.prefix);
        self.items = items;
        self.hosted = hosted;
        self.chosen = chosen
            .and_then(|label| self.items.iter().position(|c| c.label == label))
            .unwrap_or(0);
        self.widest.set(None);
    }

    /// Move the choice by `step`, wrapping round.
    pub(crate) fn step(&mut self, step: isize) {
        let count = self.items.len() as isize;
        if count > 0 {
            self.chosen = (self.chosen as isize + step).rem_euclid(count) as usize;
        }
    }

    /// Whether item `index` is the host's.
    pub(crate) fn is_hosts(&self, index: usize) -> bool {
        index < self.hosted
    }

    /// What resolving item `index` gave for the text at `version`, if it
    /// has been resolved for it.
    pub(crate) fn resolved_for(&self, index: usize, version: u64) -> Option<&Resolved> {
        let key = self.items.get(index)?.resolve?;
        self.resolved
            .iter()
            .find(|r| r.key == key && r.version == version)
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

/// The keywords and the document's words on offer, each label once.
pub(crate) fn pool(keywords: &[&str], words: &[String]) -> Vec<Completion> {
    let mut seen = HashSet::new();
    let keywords = keywords
        .iter()
        .map(|k| Completion::new(*k, CompletionKind::Keyword));
    let words = words
        .iter()
        .map(|w| Completion::new(w.clone(), CompletionKind::Variable));
    keywords
        .chain(words)
        .filter(|c| seen.insert(c.label.clone()))
        .collect()
}

/// What matches `prefix`, whatever its case: the host's items whose filter
/// text starts with it, in their order, then the pool's that start with it
/// but aren't it and aren't already offered; at most [`MAX_ITEMS`]. And how
/// many of them are the host's.
pub(crate) fn matching(
    host: &[Completion],
    pool: &[Completion],
    prefix: &str,
) -> (Vec<Completion>, usize) {
    let lower = prefix.to_lowercase();
    let starts = |text: &str| text.to_lowercase().starts_with(&lower);
    let mut items: Vec<Completion> = host
        .iter()
        .filter(|c| starts(c.filter()))
        .take(MAX_ITEMS)
        .cloned()
        .collect();
    let hosted = items.len();
    let offered: HashSet<String> = items.iter().map(|c| c.label.clone()).collect();
    items.extend(
        pool.iter()
            .filter(|c| c.label != prefix && starts(&c.label) && !offered.contains(&c.label))
            .take(MAX_ITEMS - hosted)
            .cloned(),
    );
    (items, hosted)
}

/// Put `item`, one of the pool's, in place of the word from `from` to the
/// caret: a function gets `()` with the caret between, unless a `(` follows
/// already.
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

/// The changes accepting the host's `item` makes, in the text as it is now,
/// with the caret `at`, and where the caret goes after them. `asked` is the
/// ask its edits answer; `resolved`, extra edits resolving gave for the text
/// as it is now, in place of the item's own.
///
/// While the list is open the only change since the ask is the word from
/// `asked.from` having grown or shrunk at the caret, from `asked.at` to
/// `at`; so a range ending at or before the word's start stands, and one
/// starting at or after the asked caret moves by as much. The item's edit
/// must start at or before both carets and end at or after the asked one; it
/// is stretched by the change. A range touching the word otherwise can't be
/// moved: then the item's label replaces the word, as a pool item's does.
pub(crate) fn host_changes(
    item: &Completion,
    asked: &CompletionAsk,
    at: usize,
    resolved: Option<&[CompletionEdit]>,
) -> (Vec<Change>, usize) {
    let shift = at as isize - asked.at as isize;
    let moved = |range: &Range<usize>| -> Option<Range<usize>> {
        if range.end <= asked.from {
            Some(range.clone())
        } else if range.start >= asked.at {
            let start = range.start.checked_add_signed(shift)?;
            let end = range.end.checked_add_signed(shift)?;
            Some(start..end)
        } else {
            None
        }
    };
    let fallback = || {
        (
            vec![Change {
                range: asked.from.min(at)..at,
                text: item.label.clone(),
            }],
            asked.from.min(at) + item.label.len(),
        )
    };
    let own = item
        .edit
        .clone()
        .unwrap_or_else(|| CompletionEdit::new(asked.from..asked.at, item.label.clone()));
    if own.range.start > asked.at.min(at)
        || own.range.end < asked.at
        || own.range.start > own.range.end
    {
        return fallback();
    }
    let Some(end) = own.range.end.checked_add_signed(shift) else {
        return fallback();
    };
    let main = Change {
        range: own.range.start..end,
        text: own.text,
    };
    let mut changes = vec![main.clone()];
    match resolved {
        // Already in the text as it is now.
        Some(extra) => changes.extend(extra.iter().map(|e| Change {
            range: e.range.clone(),
            text: e.text.clone(),
        })),
        None => {
            for edit in &item.extra_edits {
                let Some(range) = moved(&edit.range) else {
                    return fallback();
                };
                changes.push(Change {
                    range,
                    text: edit.text.clone(),
                });
            }
        }
    }
    // They mustn't overlap one another (edits that merely touch are fine).
    let mut sorted: Vec<&Change> = changes.iter().collect();
    sorted.sort_by_key(|c| (c.range.start, c.range.end));
    if sorted
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start)
    {
        return fallback();
    }
    // The caret after the main edit's text, moved by what the edits before
    // it added or took away.
    let before: isize = changes[1..]
        .iter()
        .filter(|c| c.range.end <= main.range.start)
        .map(|c| c.text.len() as isize - c.range.len() as isize)
        .sum();
    let caret = (main.range.start + main.text.len()).saturating_add_signed(before);
    (changes, caret)
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
