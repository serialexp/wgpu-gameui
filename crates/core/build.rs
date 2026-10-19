//! Generates the [`PhosphorIcon`] list from Phosphor's own stylesheet.
//!
//! `assets/fonts/phosphor/regular.css` is the upstream `@phosphor-icons/web`
//! `src/regular/style.css`, vendored unchanged beside the font it describes.
//! Each `.ph.ph-<name>:before { content: "\<hex>"; }` rule becomes one
//! `Variant = '\u{hex}', "name";` line for the `icons!` macro in
//! `src/render/phosphor.rs`. A later name for a codepoint already seen is a
//! legacy alias (`folder-notch` for `folder`) and is skipped, so every variant
//! owns a distinct glyph.

use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;

const STYLESHEET: &str = "assets/fonts/phosphor/regular.css";

/// `caret-up` -> `CaretUp`.
fn variant(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect()
}

/// Every `(name, codepoint)` rule in stylesheet order.
fn rules(css: &str) -> Vec<(&str, u32)> {
    const SELECTOR: &str = ".ph.ph-";
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(start) = rest.find(SELECTOR) {
        rest = &rest[start + SELECTOR.len()..];
        let Some(name_end) = rest.find(":before") else {
            break;
        };
        let name = &rest[..name_end];
        let body = &rest[name_end..];
        let Some(open) = body.find("content: \"\\") else {
            break;
        };
        let hex = &body[open + "content: \"\\".len()..];
        let hex = &hex[..hex.find('"').expect("unterminated content string")];
        let codepoint = u32::from_str_radix(hex, 16)
            .unwrap_or_else(|_| panic!("{name}: bad codepoint {hex:?}"));
        out.push((name, codepoint));
        rest = body;
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed={STYLESHEET}");
    println!("cargo:rerun-if-changed=build.rs");
    let css =
        std::fs::read_to_string(STYLESHEET).unwrap_or_else(|e| panic!("read {STYLESHEET}: {e}"));
    let mut seen_codepoints = HashSet::new();
    let mut seen_variants = HashSet::new();
    let mut list = String::from("icons! {\n");
    for (name, codepoint) in rules(&css) {
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{name:?} is not a kebab-case icon name"
        );
        if !seen_codepoints.insert(codepoint) {
            continue;
        }
        let variant = variant(name);
        assert!(
            variant.starts_with(|c: char| c.is_ascii_alphabetic()),
            "{name:?} does not make an identifier"
        );
        assert!(
            seen_variants.insert(variant.clone()),
            "{name:?} collides as {variant}"
        );
        let _ = writeln!(list, "    /// Phosphor `{name}`.");
        let _ = writeln!(list, "    {variant} = '\\u{{{codepoint:x}}}', \"{name}\";");
    }
    list.push_str("}\n");
    assert!(
        seen_variants.len() > 1000,
        "only {} icons parsed from {STYLESHEET}",
        seen_variants.len()
    );
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("phosphor_icons.rs");
    std::fs::write(&out, list).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
}
