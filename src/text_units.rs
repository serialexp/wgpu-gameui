//! Words and paragraphs, for selecting text a unit at a time: a double-click
//! selects the word under the pointer, a triple-click its paragraph, and a
//! drag that follows either grows the selection a whole unit at a time.
//! Text fields and read-only text ([`TextSelection`](crate::TextSelection))
//! share these rules.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// How much text a click selects, and what a drag after it grows by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextUnit {
    /// Characters: a click places the caret, a drag selects up to the pointer.
    #[default]
    Char,
    /// A word, a run of spaces, or one punctuation mark (Unicode word
    /// boundaries).
    Word,
    /// A paragraph: everything between two line breaks, with the break that
    /// ends it.
    Paragraph,
}

impl TextUnit {
    /// The unit a run of `clicks` ([`InputState::mouse_click_count`]) selects:
    /// characters for one, a word for two, a paragraph for three or more.
    ///
    /// [`InputState::mouse_click_count`]: crate::InputState::mouse_click_count
    pub fn for_clicks(clicks: u32) -> Self {
        match clicks {
            0 | 1 => Self::Char,
            2 => Self::Word,
            _ => Self::Paragraph,
        }
    }

    /// The unit of `text` around the character starting at byte `at` (the
    /// one under the pointer, not the caret nearest it). [`Char`](Self::Char)
    /// is the empty range at `at`. `at` past the end means the last
    /// character.
    pub fn range_at(self, text: &str, at: usize) -> Range<usize> {
        let at = floor_boundary(text, at.min(text.len()));
        match self {
            Self::Char => at..at,
            Self::Word => word_at(text, at),
            Self::Paragraph => paragraph_at(text, at),
        }
    }

    /// Where a drag that began on `origin` (the unit the press selected) and
    /// is now over the character at `under` (caret `caret`) puts the
    /// selection, as `(anchor, cursor)`: `origin` stays selected, and the
    /// selection grows to take in the whole unit under the pointer, on
    /// whichever side of `origin` it is.
    pub fn extend(
        self,
        text: &str,
        origin: Range<usize>,
        under: usize,
        caret: usize,
    ) -> (usize, usize) {
        if self == Self::Char {
            return (origin.start, caret);
        }
        let unit = self.range_at(text, under);
        if unit.start < origin.start {
            (origin.end, unit.start)
        } else {
            (origin.start, unit.end.max(origin.end))
        }
    }
}

/// The word, run of spaces or punctuation mark holding byte `at`.
fn word_at(text: &str, at: usize) -> Range<usize> {
    let mut last = at..at;
    for (start, segment) in text.split_word_bound_indices() {
        let range = start..start + segment.len();
        if at < range.end {
            return range;
        }
        last = range;
    }
    last
}

/// The paragraph holding byte `at`, with the line break that ends it. A byte
/// on a line break belongs to the paragraph the break ends.
fn paragraph_at(text: &str, at: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    // At the very end, the last character decides.
    let at = if at == text.len() {
        floor_boundary(text, at - 1)
    } else {
        at
    };
    let start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = text[at..]
        .find('\n')
        .map_or(text.len(), |index| at + index + 1);
    start..end
}

/// The char boundary at or before `at`.
pub(crate) fn floor_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_pick_the_unit() {
        assert_eq!(TextUnit::for_clicks(0), TextUnit::Char);
        assert_eq!(TextUnit::for_clicks(1), TextUnit::Char);
        assert_eq!(TextUnit::for_clicks(2), TextUnit::Word);
        assert_eq!(TextUnit::for_clicks(3), TextUnit::Paragraph);
        assert_eq!(TextUnit::for_clicks(7), TextUnit::Paragraph);
    }

    #[test]
    fn a_word_is_a_word_a_space_run_or_one_mark() {
        let text = "hello,  wide world";
        let word = |at| &text[TextUnit::Word.range_at(text, at)];
        assert_eq!(word(0), "hello");
        assert_eq!(word(4), "hello");
        assert_eq!(word(5), ",");
        assert_eq!(word(6), "  ");
        assert_eq!(word(9), "wide");
        // Past the end: the last word.
        assert_eq!(word(99), "world");
        assert_eq!(TextUnit::Word.range_at("", 0), 0..0);
    }

    #[test]
    fn words_hold_together_across_multibyte_characters() {
        let text = "naïve café";
        let at = text.find('ï').unwrap() + 1; // inside the ï
        assert_eq!(&text[TextUnit::Word.range_at(text, at)], "naïve");
        assert_eq!(&text[TextUnit::Word.range_at(text, text.len() - 1)], "café");
    }

    #[test]
    fn a_paragraph_runs_between_line_breaks_with_the_one_that_ends_it() {
        let text = "one\ntwo two\n\nthree";
        let paragraph = |at| &text[TextUnit::Paragraph.range_at(text, at)];
        assert_eq!(paragraph(0), "one\n");
        assert_eq!(paragraph(3), "one\n", "the break belongs to its line");
        assert_eq!(paragraph(6), "two two\n");
        assert_eq!(paragraph(12), "\n", "an empty line is its break");
        assert_eq!(paragraph(14), "three");
        assert_eq!(paragraph(text.len()), "three");
        assert_eq!(TextUnit::Paragraph.range_at("", 0), 0..0);
    }

    #[test]
    fn a_char_unit_is_the_caret() {
        assert_eq!(TextUnit::Char.range_at("abc", 2), 2..2);
        assert_eq!(TextUnit::Char.extend("abc", 1..1, 2, 3), (1, 3));
    }

    #[test]
    fn a_drag_grows_by_whole_units_on_either_side() {
        let text = "alpha beta gamma";
        let origin = TextUnit::Word.range_at(text, 7); // "beta"
        // Forward into "gamma": anchor at the start of beta, cursor after gamma.
        assert_eq!(TextUnit::Word.extend(text, origin.clone(), 12, 12), (6, 16));
        // Back into "alpha": anchor at the end of beta, cursor before alpha.
        assert_eq!(TextUnit::Word.extend(text, origin.clone(), 1, 1), (10, 0));
        // Still inside beta: beta alone.
        assert_eq!(TextUnit::Word.extend(text, origin, 8, 8), (6, 10));
    }
}
