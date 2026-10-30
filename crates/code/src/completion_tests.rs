use super::*;

fn labels(items: &[Completion]) -> Vec<&str> {
    items.iter().map(|c| c.label.as_str()).collect()
}

#[test]
fn the_word_before_the_caret_is_the_prefix() {
    let doc = Document::new("let value = pla\n");
    assert_eq!(prefix_at(&doc, 15), (12, "pla".to_owned()));
    // After a space: none.
    assert_eq!(prefix_at(&doc, 12), (12, String::new()));
    // A number isn't the start of a word to complete.
    let doc = Document::new("x = 12");
    assert_eq!(prefix_at(&doc, 6), (6, String::new()));
}

#[test]
fn a_prefix_stays_on_its_line() {
    let doc = Document::new("abc\ndef");
    assert_eq!(prefix_at(&doc, 4), (4, String::new()));
}

#[test]
fn identifiers_are_each_named_once_in_the_order_they_come() {
    let doc = Document::new("let player = spawn(player, 4);\nmove_to(player, x9)\n");
    let found = identifiers(&doc, None);
    assert_eq!(found, vec!["let", "player", "spawn", "move_to", "x9"]);
}

#[test]
fn the_hosts_items_come_first_then_keywords_then_the_documents_words() {
    let host = vec![Completion::new("spawn", CompletionKind::Function).with_detail("(kind, x, y)")];
    let words = vec!["spawn".to_owned(), "speed".to_owned()];
    let pool = pool(&host, &["static", "struct"], &words);
    assert_eq!(labels(&pool), vec!["spawn", "static", "struct", "speed"]);
    assert_eq!(
        pool[0].kind,
        CompletionKind::Function,
        "the host's, not the word's"
    );
    assert_eq!(pool[1].kind, CompletionKind::Keyword);
    assert_eq!(pool[3].kind, CompletionKind::Variable);
}

#[test]
fn matches_start_with_the_prefix_whatever_its_case() {
    let pool = pool(
        &[],
        &[],
        &[
            "Player".to_owned(),
            "plan".to_owned(),
            "replay".to_owned(),
            "pl".to_owned(),
        ],
    );
    assert_eq!(
        labels(&matching(&pool, "pl")),
        vec!["Player", "plan"],
        "not the prefix itself"
    );
    assert_eq!(
        labels(&matching(&pool, "")),
        vec!["Player", "plan", "replay", "pl"]
    );
}

#[test]
fn at_most_eight_are_offered() {
    let words: Vec<String> = (0..20).map(|n| format!("item{n}")).collect();
    let pool = pool(&[], &[], &words);
    assert_eq!(matching(&pool, "it").len(), MAX_ITEMS);
}

#[test]
fn accepting_replaces_the_prefix() {
    let mut doc = Document::new("let x = pl\n");
    doc.set_selection(Selection::caret(10));
    accept(
        &mut doc,
        8,
        &Completion::new("player", CompletionKind::Variable),
    );
    assert_eq!(doc.text(), "let x = player\n");
    assert_eq!(doc.selection().head, 14);
}

#[test]
fn a_function_gets_its_parentheses_with_the_caret_between() {
    let mut doc = Document::new("sp\n");
    doc.set_selection(Selection::caret(2));
    accept(
        &mut doc,
        0,
        &Completion::new("spawn", CompletionKind::Function),
    );
    assert_eq!(doc.text(), "spawn()\n");
    assert_eq!(doc.selection().head, 6);
}

#[test]
fn a_function_already_called_gets_none() {
    let mut doc = Document::new("sp(1)\n");
    doc.set_selection(Selection::caret(2));
    accept(
        &mut doc,
        0,
        &Completion::new("spawn", CompletionKind::Function),
    );
    assert_eq!(doc.text(), "spawn(1)\n");
    assert_eq!(doc.selection().head, 5);
}

#[test]
fn accepting_is_one_step_to_undo() {
    let mut doc = Document::new("pl\n");
    doc.set_selection(Selection::caret(2));
    accept(
        &mut doc,
        0,
        &Completion::new("player", CompletionKind::Variable),
    );
    doc.undo();
    assert_eq!(doc.text(), "pl\n");
}

#[test]
fn a_large_documents_words_are_found_in_one_pass() {
    // Representative scale: 50,000 lines, 5,000 distinct names.
    let text: String = (0..50_000)
        .map(|n| format!("let name{} = other;\n", n % 5_000))
        .collect();
    let doc = Document::new(&text);
    let found = identifiers(&doc, None);
    assert_eq!(found.len(), 5_000 + 2, "let, other and the names");
    assert_eq!(found[0], "let");
}

#[test]
fn the_word_being_typed_is_not_offered_to_itself() {
    let doc = Document::new("player pla plan pla");
    assert_eq!(identifiers(&doc, Some(7)), vec!["player", "plan", "pla"]);
    assert_eq!(identifiers(&doc, Some(16)), vec!["player", "pla", "plan"]);
}
