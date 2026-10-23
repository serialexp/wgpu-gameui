//! Generates tables from vendored stylesheets, so a new upstream copy is the
//! whole update.
//!
//! **Phosphor icons.** The [`PhosphorIcon`] list comes from Phosphor's own
//! stylesheet. `assets/fonts/phosphor/regular.css` is the upstream `@phosphor-icons/web`
//! `src/regular/style.css`, vendored unchanged beside the font it describes.
//! Each `.ph.ph-<name>:before { content: "\<hex>"; }` rule becomes one
//! `Variant = '\u{hex}', "name";` line for the `icons!` macro in
//! `src/render/phosphor.rs`. A later name for a codepoint already seen is a
//! legacy alias (`folder-notch` for `folder`) and is skipped, so every variant
//! owns a distinct glyph.
//!
//! **Forge cursors.** `assets/cursors/cursors.css` is the Forge design system's
//! `tokens/cursors.css`, vendored unchanged (Forge generates it from
//! `components/cursor/cursors.js`). Each
//! `--cursor-<name>: url("data:image/svg+xml,<svg>") <x> <y>, <fallback>;`
//! declaration becomes one `ForgeCursor` in `FORGE_CURSORS`, read by
//! `src/cursor.rs`: the name, the hotspot, the native keyword to fall back to
//! and the decoded SVG.

use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;

const STYLESHEET: &str = "assets/fonts/phosphor/regular.css";
const CURSORS: &str = "assets/cursors/cursors.css";

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

/// `%3Csvg%20…` -> `<svg …`.
fn percent_decode(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = encoded
                .get(i + 1..i + 3)
                .unwrap_or_else(|| panic!("truncated escape at byte {i}"));
            out.push(u8::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad escape %{hex}")));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("decoded cursor SVG is not UTF-8")
}

/// One `--cursor-…` declaration: name, hotspot, fallback keyword and SVG.
/// Declarations that aren't an image (`--cursor-none: none;`) are skipped.
fn cursor_rules(css: &str) -> Vec<(&str, [u16; 2], &str, String)> {
    const PROPERTY: &str = "--cursor-";
    const URL: &str = ": url(\"data:image/svg+xml,";
    let mut out = Vec::new();
    for line in css.lines() {
        let Some(rest) = line.trim_start().strip_prefix(PROPERTY) else {
            continue;
        };
        let Some(name_end) = rest.find(URL) else {
            continue;
        };
        let name = &rest[..name_end];
        let rest = &rest[name_end + URL.len()..];
        let (data, rest) = rest
            .split_once("\")")
            .unwrap_or_else(|| panic!("{name}: unterminated url"));
        let (hotspot, rest) = rest
            .trim_start()
            .split_once(',')
            .unwrap_or_else(|| panic!("{name}: no fallback after the hotspot"));
        let hotspot: Vec<u16> = hotspot
            .split_whitespace()
            .map(|n| {
                n.parse()
                    .unwrap_or_else(|_| panic!("{name}: bad hotspot {n:?}"))
            })
            .collect();
        let [x, y] = hotspot[..] else {
            panic!("{name}: the hotspot is not two numbers");
        };
        let fallback = rest
            .split(';')
            .next()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| panic!("{name}: no fallback keyword"));
        out.push((name, [x, y], fallback, percent_decode(data)));
    }
    out
}

fn forge_cursors() {
    println!("cargo:rerun-if-changed={CURSORS}");
    let css = std::fs::read_to_string(CURSORS).unwrap_or_else(|e| panic!("read {CURSORS}: {e}"));
    let rules = cursor_rules(&css);
    assert!(
        rules.len() > 1,
        "only {} cursors parsed from {CURSORS}",
        rules.len()
    );
    let mut table = format!(
        "/// Every cursor in `{CURSORS}`, in its order.\n\
         static FORGE_CURSORS: [ForgeCursor; {}] = [\n",
        rules.len()
    );
    for (name, [x, y], fallback, svg) in rules {
        assert!(
            name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
            "{name:?} is not a kebab-case cursor name"
        );
        assert!(svg.starts_with("<svg"), "{name}: not an SVG");
        let _ = writeln!(
            table,
            "    ForgeCursor {{ name: {name:?}, hotspot: [{x}, {y}], fallback: {fallback:?}, svg: {svg:?} }},"
        );
    }
    table.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("forge_cursors.rs");
    std::fs::write(&out, table).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    phosphor_icons();
    forge_cursors();
}

fn phosphor_icons() {
    println!("cargo:rerun-if-changed={STYLESHEET}");
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
