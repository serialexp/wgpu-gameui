//! Each language's keywords, offered by completion after the host's items.

use crate::language::Language;

const BASH: &[&str] = &[
    "case", "declare", "do", "done", "echo", "elif", "else", "esac", "exit", "export", "fi", "for",
    "function", "if", "in", "local", "readonly", "return", "select", "set", "shift", "then",
    "trap", "unset", "until", "while",
];

const CSS: &[&str] = &[
    "!important",
    "@font-face",
    "@import",
    "@keyframes",
    "@media",
    "@supports",
    "auto",
    "inherit",
    "initial",
    "none",
    "unset",
];

const GO: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "false",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "iota",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "true",
    "type",
    "var",
];

const HTML: &[&str] = &[
    "a", "body", "button", "div", "footer", "form", "head", "header", "html", "img", "input",
    "label", "li", "link", "main", "meta", "nav", "ol", "p", "script", "section", "span", "style",
    "table", "td", "template", "th", "title", "tr", "ul",
];

const JAVASCRIPT: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];

const TYPESCRIPT: &[&str] = &[
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "keyof",
    "let",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "static",
    "string",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "void",
    "while",
    "yield",
];

const JSON: &[&str] = &["false", "null", "true"];

const LUA: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const PYTHON: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "self", "try",
    "while", "with", "yield",
];

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

const TOML: &[&str] = &["false", "true"];

const YAML: &[&str] = &["false", "no", "null", "true", "yes"];

impl Language {
    /// Its keywords, offered by completion.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Plain | Self::Markdown => &[],
            Self::Bash => BASH,
            Self::Css => CSS,
            Self::Go => GO,
            Self::Html => HTML,
            Self::JavaScript => JAVASCRIPT,
            Self::TypeScript | Self::Tsx => TYPESCRIPT,
            Self::Json => JSON,
            Self::Lua => LUA,
            Self::Python => PYTHON,
            Self::Rust => RUST,
            Self::Toml => TOML,
            Self::Yaml => YAML,
        }
    }
}
