use super::*;

#[test]
fn a_file_is_known_by_its_name() {
    let cases = [
        ("/src/main.rs", Language::Rust),
        ("app.ts", Language::TypeScript),
        ("view.TSX", Language::Tsx),
        ("index.mjs", Language::JavaScript),
        ("button.jsx", Language::JavaScript),
        ("package.json", Language::Json),
        ("init.lua", Language::Lua),
        ("README.md", Language::Markdown),
        ("tool.py", Language::Python),
        ("main.go", Language::Go),
        ("build.sh", Language::Bash),
        ("/home/x/.zshrc", Language::Bash),
        ("site.css", Language::Css),
        ("page.html", Language::Html),
        ("ci.yml", Language::Yaml),
        ("Cargo.toml", Language::Toml),
        ("Cargo.lock", Language::Toml),
        ("notes.txt", Language::Plain),
        ("Makefile", Language::Plain),
        ("/a.rs/readme", Language::Plain),
    ];
    for (path, language) in cases {
        assert_eq!(Language::for_path(path), language, "{path}");
    }
}

#[test]
fn every_grammar_and_its_query_build() {
    for language in Language::ALL {
        if language == Language::Plain {
            assert!(language.grammar().is_none());
        } else {
            assert!(language.grammar().is_some(), "{language:?}");
        }
    }
}

#[test]
fn each_language_has_a_name_to_show() {
    assert_eq!(Language::Rust.label(), "Rust");
    assert_eq!(Language::Tsx.label(), "TSX");
    assert_eq!(Language::Plain.label(), "Plain text");
}

#[test]
fn rust_leaves_single_quotes_to_lifetimes() {
    let rules = Language::Rust.rules(4);
    assert!(!rules.pairs.iter().any(|&(open, _)| open == '\''));
    assert!(Language::JavaScript.rules(4).pairs.contains(&('\'', '\'')));
}

#[test]
fn prose_leaves_quotes_alone() {
    for language in [Language::Plain, Language::Markdown] {
        let rules = language.rules(4);
        assert!(
            !rules
                .pairs
                .iter()
                .any(|&(open, _)| open == '\'' || open == '"')
        );
    }
}

#[test]
fn each_language_knows_its_blocks() {
    let cases = [
        (Language::Rust, "fn main() {", true),
        (Language::Rust, "let x = 1;", false),
        (Language::Lua, "if ready then", true),
        (Language::Lua, "for i = 1, 3 do", true),
        (Language::Lua, "local f = function(a)", true),
        (Language::Lua, "print(x)", false),
        (Language::Python, "def f(x):", true),
        (Language::Python, "x = {", true),
        (Language::Python, "x = y", false),
        (Language::Yaml, "jobs:", true),
        (Language::Yaml, "name: x", false),
        (Language::Bash, "if true; then", true),
        (Language::Bash, "for f in *; do", true),
    ];
    for (language, head, opens) in cases {
        assert_eq!(
            (language.rules(4).opens_block)(head),
            opens,
            "{language:?}: {head}"
        );
    }
}

#[test]
fn the_tab_size_is_the_editors() {
    assert_eq!(Language::Go.rules(8).tab_size, 8);
}
