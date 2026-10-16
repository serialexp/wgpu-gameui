use super::*;
use crate::color::{hex, oklch};

#[test]
fn the_default_inks_are_forges() {
    let theme = SyntaxTheme::default();
    assert_eq!(theme.comment, oklch(0.6, 0.025, 160.0, 1.0));
    assert_eq!(theme.string, oklch(0.8, 0.11, 135.0, 1.0));
    assert_eq!(theme.literal, oklch(0.8, 0.11, 65.0, 1.0));
    assert_eq!(theme.keyword, oklch(0.76, 0.12, 315.0, 1.0));
    assert_eq!(theme.function, oklch(0.78, 0.1, 260.0, 1.0));
    assert_eq!(theme.type_or_constant, oklch(0.82, 0.08, 170.0, 1.0));
    assert_eq!(theme.symbol, oklch(0.84, 0.08, 90.0, 1.0));
    assert_eq!(theme.operator, hex(0x9aa2aa));
    assert_eq!(theme.variable, hex(0xc6ced5));
    assert_eq!(theme.conditional, theme.keyword);
    assert_eq!(theme.repeat, theme.keyword);
}

#[test]
fn a_capture_takes_the_ink_of_its_category() {
    let theme = SyntaxTheme::default();
    let cases = [
        ("comment", Some(theme.comment)),
        ("string", Some(theme.string)),
        ("number", Some(theme.literal)),
        ("boolean", Some(theme.literal)),
        ("keyword", Some(theme.keyword)),
        ("conditional", Some(theme.conditional)),
        ("repeat", Some(theme.repeat)),
        ("function", Some(theme.function)),
        ("method", Some(theme.function)),
        ("constructor", Some(theme.function)),
        ("type", Some(theme.type_or_constant)),
        ("constant", Some(theme.type_or_constant)),
        ("attribute", Some(theme.type_or_constant)),
        ("property", Some(theme.symbol)),
        ("parameter", Some(theme.symbol)),
        ("label", Some(theme.symbol)),
        ("operator", Some(theme.operator)),
        ("punctuation", Some(theme.operator)),
        ("variable", Some(theme.variable)),
        ("nothing-we-know", None),
    ];
    for (capture, ink) in cases {
        assert_eq!(theme.for_capture(capture), ink, "{capture}");
    }
}

#[test]
fn a_dotted_capture_takes_its_first_known_categorys_ink() {
    let theme = SyntaxTheme::default();
    let cases = [
        // The base wins over a modifier that is a category too.
        ("keyword.function", theme.keyword),
        ("function.method", theme.function),
        ("variable.parameter", theme.variable),
        ("function.call", theme.function),
        ("punctuation.bracket", theme.operator),
        ("string.special.key", theme.string),
        ("comment.documentation", theme.comment),
        ("type.builtin", theme.type_or_constant),
        // Built-in constants are true, nil, and Rust's numbers.
        ("constant.builtin", theme.literal),
        ("constant.builtin.boolean", theme.literal),
        ("constant.character", theme.type_or_constant),
        // A first part we don't know is passed over.
        ("special.function", theme.function),
    ];
    for (capture, ink) in cases {
        assert_eq!(theme.for_capture(capture), Some(ink), "{capture}");
    }
}

#[test]
fn the_other_grammars_names_find_the_nearest_ink() {
    let theme = SyntaxTheme::default();
    let cases = [
        // HTML and JSX.
        ("tag", Some(theme.keyword)),
        ("tag.delimiter", Some(theme.keyword)),
        // Escapes in strings, as literals.
        ("escape", Some(theme.literal)),
        ("string.escape", Some(theme.string)),
        // Modules and namespaces, as types.
        ("module", Some(theme.type_or_constant)),
        ("namespace", Some(theme.type_or_constant)),
        // Markdown's headings, code and links.
        ("text.title", Some(theme.keyword)),
        ("markup.heading", Some(theme.keyword)),
        ("text.literal", Some(theme.string)),
        ("markup.raw", Some(theme.string)),
        ("text.uri", Some(theme.function)),
        ("markup.link.url", Some(theme.function)),
        ("text.reference", Some(theme.symbol)),
        // Emphasis isn't a colour; it keeps the text's own.
        ("text.emphasis", None),
        ("embedded", None),
    ];
    for (capture, ink) in cases {
        assert_eq!(theme.for_capture(capture), ink, "{capture}");
    }
}
