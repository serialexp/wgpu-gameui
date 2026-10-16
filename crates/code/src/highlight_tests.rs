use super::*;
use crate::document::{Change, EditKind, Selection};

/// Each coloured piece of `range`'s text with its ink.
fn coloured(syntax: &Syntax, document: &Document, range: Range<usize>) -> Vec<(String, [f32; 4])> {
    syntax
        .spans(document, range, &SyntaxTheme::default())
        .into_iter()
        .map(|span| (document.slice(span.range), span.ink))
        .collect()
}

fn ink_of(parts: &[(String, [f32; 4])], text: &str) -> Option<[f32; 4]> {
    parts
        .iter()
        .find(|(part, _)| part == text)
        .map(|&(_, ink)| ink)
}

fn all(document: &Document) -> Range<usize> {
    0..document.len()
}

#[test]
fn rust_is_coloured_by_what_each_piece_is() {
    let theme = SyntaxTheme::default();
    let mut document = Document::new("fn main() {\n    let x = 42; // hi\n    \"s\"\n}\n");
    let syntax = Syntax::new(Language::Rust, &mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert_eq!(ink_of(&parts, "fn"), Some(theme.keyword));
    assert_eq!(ink_of(&parts, "main"), Some(theme.function));
    assert_eq!(ink_of(&parts, "42"), Some(theme.literal));
    assert_eq!(ink_of(&parts, "// hi"), Some(theme.comment));
    assert_eq!(ink_of(&parts, "\"s\""), Some(theme.string));
}

#[test]
fn spans_are_in_order_apart_and_inside_what_was_asked() {
    let text: String = (0..200)
        .map(|n| format!("fn f{n}(a: u32) -> u32 {{ a + {n} }} // {n}\n"))
        .collect();
    let mut document = Document::new(&text);
    let syntax = Syntax::new(Language::Rust, &mut document);
    let range = document.line_start(100)..document.line_start(103);
    let spans = syntax.spans(&document, range.clone(), &SyntaxTheme::default());
    assert!(!spans.is_empty());
    for pair in spans.windows(2) {
        assert!(pair[0].range.end <= pair[1].range.start, "{pair:?}");
    }
    for span in &spans {
        assert!(
            span.range.start >= range.start && span.range.end <= range.end,
            "{span:?} outside {range:?}"
        );
    }
}

#[test]
fn a_later_pattern_on_the_same_node_wins() {
    // JavaScript's query colours every identifier as a variable first, and
    // a function's name as a function after.
    let theme = SyntaxTheme::default();
    let mut document = Document::new("function greet(name) { return name; }");
    let syntax = Syntax::new(Language::JavaScript, &mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert_eq!(ink_of(&parts, "greet"), Some(theme.function));
    assert_eq!(ink_of(&parts, "function"), Some(theme.keyword));
}

#[test]
fn typescript_adds_its_own_to_javascripts() {
    let theme = SyntaxTheme::default();
    let mut document = Document::new("function f(x: number): string { return `${x}`; }");
    let syntax = Syntax::new(Language::TypeScript, &mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert_eq!(ink_of(&parts, "f"), Some(theme.function));
    assert_eq!(ink_of(&parts, "number"), Some(theme.type_or_constant));
}

#[test]
fn an_inner_piece_wins_over_the_one_around_it() {
    // An escape inside a string.
    let theme = SyntaxTheme::default();
    let mut document = Document::new("let s = \"a\\nb\";");
    let syntax = Syntax::new(Language::Rust, &mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert_eq!(ink_of(&parts, "\\n"), Some(theme.literal));
    assert_eq!(ink_of(&parts, "\"a"), Some(theme.string));
    assert_eq!(ink_of(&parts, "b\""), Some(theme.string));
}

#[test]
fn an_edit_is_coloured_as_the_new_text_is() {
    let theme = SyntaxTheme::default();
    let mut document = Document::new("let x = 42;\nlet y = 1;\n");
    let mut syntax = Syntax::new(Language::Rust, &mut document);
    document.edit(
        vec![Change {
            range: 8..10,
            text: "\"text\"".into(),
        }],
        Selection::caret(14),
        EditKind::Other,
    );
    syntax.update(&mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert_eq!(ink_of(&parts, "\"text\""), Some(theme.string));
    assert_eq!(
        ink_of(&parts, "1"),
        Some(theme.literal),
        "the next line as before"
    );

    document.undo();
    syntax.update(&mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert_eq!(ink_of(&parts, "42"), Some(theme.literal));
}

#[test]
fn opening_a_comment_colours_the_lines_after_it() {
    let theme = SyntaxTheme::default();
    let mut document = Document::new("let a = 1;\nlet b = 2;\nlet c = 3;\n");
    let mut syntax = Syntax::new(Language::Rust, &mut document);
    document.edit(
        vec![Change {
            range: 0..0,
            text: "/*".into(),
        }],
        Selection::caret(2),
        EditKind::Typing,
    );
    syntax.update(&mut document);
    let last = document.line_start(2)..document.line_end(2);
    let parts = coloured(&syntax, &document, last);
    assert_eq!(parts.len(), 1, "{parts:?}");
    assert_eq!(parts[0], ("let c = 3;".to_owned(), theme.comment));
}

#[test]
fn an_edit_at_the_top_of_a_long_file_leaves_its_end_right() {
    let theme = SyntaxTheme::default();
    let text: String = (0..100_000).map(|n| format!("let v{n} = {n};\n")).collect();
    let mut document = Document::new(&text);
    let mut syntax = Syntax::new(Language::Rust, &mut document);
    document.edit(
        vec![Change {
            range: 0..0,
            text: "// top\n".into(),
        }],
        Selection::caret(7),
        EditKind::Other,
    );
    syntax.update(&mut document);
    let line = document.line_count() - 2;
    let range = document.line_start(line)..document.line_end(line);
    let parts = coloured(&syntax, &document, range);
    assert_eq!(document.line(line), "let v99999 = 99999;");
    assert_eq!(ink_of(&parts, "99999"), Some(theme.literal));
    assert_eq!(ink_of(&parts, "let"), Some(theme.keyword));
}

#[test]
fn plain_text_has_no_colours() {
    let mut document = Document::new("fn main() {}");
    let syntax = Syntax::new(Language::Plain, &mut document);
    assert!(coloured(&syntax, &document, all(&document)).is_empty());
}

#[test]
fn the_parser_is_told_each_edit_once() {
    let mut document = Document::new("let a = 1;");
    let mut syntax = Syntax::new(Language::Rust, &mut document);
    assert!(
        document.take_edits().is_empty(),
        "the first parse took them"
    );
    document.edit(
        vec![Change {
            range: 0..0,
            text: " ".into(),
        }],
        Selection::caret(1),
        EditKind::Typing,
    );
    syntax.update(&mut document);
    assert!(document.take_edits().is_empty());
}

#[test]
fn markdown_colours_its_headings_and_code() {
    let theme = SyntaxTheme::default();
    let mut document = Document::new("# Title\n\ntext\n\n```\ncode\n```\n");
    let syntax = Syntax::new(Language::Markdown, &mut document);
    let parts = coloured(&syntax, &document, all(&document));
    assert!(
        parts.iter().any(|(_, ink)| *ink == theme.keyword),
        "a heading: {parts:?}"
    );
}
