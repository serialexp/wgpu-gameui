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
fn the_pool_is_the_keywords_then_the_documents_words_each_once() {
    let words = vec!["static".to_owned(), "speed".to_owned()];
    let pool = pool(&["static", "struct"], &words);
    assert_eq!(labels(&pool), vec!["static", "struct", "speed"]);
    assert_eq!(
        pool[0].kind,
        CompletionKind::Keyword,
        "the keyword, not the word"
    );
    assert_eq!(pool[2].kind, CompletionKind::Variable);
}

#[test]
fn matches_start_with_the_prefix_whatever_its_case() {
    let pool = pool(
        &[],
        &[
            "Player".to_owned(),
            "plan".to_owned(),
            "replay".to_owned(),
            "pl".to_owned(),
        ],
    );
    let (items, hosted) = matching(&[], &pool, "pl");
    assert_eq!(
        labels(&items),
        vec!["Player", "plan"],
        "not the prefix itself"
    );
    assert_eq!(hosted, 0);
    let (items, _) = matching(&[], &pool, "");
    assert_eq!(labels(&items), vec!["Player", "plan", "replay", "pl"]);
}

#[test]
fn the_hosts_items_come_first_matched_by_their_filter_text() {
    let host = vec![
        Completion::new("spawn(kind)", CompletionKind::Function).with_filter_text("spawn"),
        Completion::new("speed", CompletionKind::Field),
        Completion::new("other", CompletionKind::Field),
    ];
    let pool = pool(&[], &["speed".to_owned(), "spare".to_owned()]);
    let (items, hosted) = matching(&host, &pool, "sp");
    assert_eq!(labels(&items), vec!["spawn(kind)", "speed", "spare"]);
    assert_eq!(hosted, 2);
    assert_eq!(
        items[1].kind,
        CompletionKind::Field,
        "the host's, not the word's"
    );
}

#[test]
fn a_host_item_that_is_the_word_itself_is_still_offered() {
    let host = vec![Completion::new("len", CompletionKind::Function)];
    let (items, hosted) = matching(&host, &[], "len");
    assert_eq!((labels(&items), hosted), (vec!["len"], 1));
}

#[test]
fn at_most_max_items_are_held() {
    let words: Vec<String> = (0..MAX_ITEMS + 50).map(|n| format!("item{n}")).collect();
    let pool = pool(&[], &words);
    assert_eq!(matching(&[], &pool, "it").0.len(), MAX_ITEMS);
}

fn ask(id: u64, at: usize, from: usize) -> CompletionAsk {
    CompletionAsk {
        id,
        at,
        from,
        trigger: CompletionTrigger::Invoked,
        version: 1,
    }
}

#[test]
fn a_list_waiting_on_the_host_stays_open_with_nothing_yet() {
    let asked = ask(1, 4, 4);
    let mut open = Completing::open(4, String::new(), Vec::new(), Some(asked)).expect("waits");
    assert!(open.items.is_empty() && open.waiting());
    open.offer(asked, CompletionList::default());
    assert!(!open.alive(), "answered with nothing");
    assert!(Completing::open(4, String::new(), Vec::new(), None).is_none());
}

#[test]
fn the_hosts_items_go_in_their_sort_order() {
    let asked = ask(1, 2, 0);
    let mut open = Completing::open(0, "sp".to_owned(), Vec::new(), Some(asked)).unwrap();
    open.offer(
        asked,
        CompletionList {
            incomplete: true,
            items: vec![
                Completion::new("spawn", CompletionKind::Function).with_sort_text("2"),
                Completion::new("speed", CompletionKind::Field).with_sort_text("1"),
                Completion::new("spin", CompletionKind::Field).with_sort_text("2"),
            ],
        },
    );
    assert_eq!(
        labels(&open.items),
        vec!["speed", "spawn", "spin"],
        "ties keep their order"
    );
    assert!(open.incomplete && !open.waiting());
}

#[test]
fn a_choice_survives_the_word_growing() {
    let pool = pool(&[], &["player".to_owned(), "plan".to_owned()]);
    let mut open = Completing::open(0, "p".to_owned(), pool, None).unwrap();
    open.step(1);
    assert_eq!(open.items[open.chosen].label, "plan");
    assert!(open.refine(0, "pla".to_owned()));
    assert_eq!(open.items[open.chosen].label, "plan");
    assert!(!open.refine(0, "plx".to_owned()));
}

fn applied(text: &str, at: usize, changes: Vec<Change>, caret: usize) -> (String, usize) {
    let mut doc = Document::new(text);
    doc.set_selection(Selection::caret(at));
    doc.edit(changes, Selection::caret(caret), EditKind::Other);
    (doc.text(), doc.selection().head)
}

#[test]
fn a_host_items_edit_is_moved_by_what_was_typed_since_the_ask() {
    // Asked at "x.fo|obar"; "o" typed since: "x.foo|obar".
    let item = Completion::new("foobar", CompletionKind::Field).with_edit(2..8, "foobar");
    let asked = ask(1, 4, 2);
    let (changes, caret) = host_changes(&item, &asked, 5, None);
    assert_eq!(
        applied("x.fooobar;", 5, changes, caret),
        ("x.foobar;".to_owned(), 8),
        "the whole word replaced, the caret after it"
    );
}

#[test]
fn without_an_edit_the_label_replaces_the_word() {
    let item = Completion::new("player", CompletionKind::Variable);
    let asked = ask(1, 2, 0);
    // "pl" asked; "a" typed since.
    let (changes, caret) = host_changes(&item, &asked, 3, None);
    assert_eq!(
        applied("pla = 1", 3, changes, caret),
        ("player = 1".to_owned(), 6)
    );
}

#[test]
fn a_host_function_gets_no_parentheses_of_the_editors() {
    let item = Completion::new("spawn", CompletionKind::Function).with_edit(0..2, "spawn");
    let (changes, caret) = host_changes(&item, &ask(1, 2, 0), 2, None);
    assert_eq!(
        applied("sp\n", 2, changes, caret),
        ("spawn\n".to_owned(), 5)
    );
}

#[test]
fn extra_edits_before_the_word_stand_and_move_the_caret() {
    // An auto-import at the top, the word on the second line.
    let text = "fn main() {}\nHashM";
    let item = Completion::new("HashMap", CompletionKind::Type)
        .with_edit(13..18, "HashMap")
        .with_extra_edit(0..0, "use std::collections::HashMap;\n");
    // "HashM" asked at its end, nothing typed since.
    let (changes, caret) = host_changes(&item, &ask(1, 18, 13), 18, None);
    let (after, caret) = applied(text, 18, changes, caret);
    assert_eq!(
        after,
        "use std::collections::HashMap;\nfn main() {}\nHashMap"
    );
    assert_eq!(caret, after.len(), "after the word, moved by the import");
}

#[test]
fn extra_edits_after_the_caret_move_with_the_typing() {
    // "ab| xy": an extra edit replacing "xy", asked at 2; "c" typed since.
    let item = Completion::new("abc", CompletionKind::Variable)
        .with_edit(0..2, "abc")
        .with_extra_edit(3..5, "zz");
    let (changes, caret) = host_changes(&item, &ask(1, 2, 0), 3, None);
    assert_eq!(
        applied("abc xy", 3, changes, caret),
        ("abc zz".to_owned(), 3)
    );
}

#[test]
fn an_extra_edit_touching_the_word_falls_back_to_the_label() {
    let item = Completion::new("player", CompletionKind::Variable)
        .with_edit(0..2, "player()")
        .with_extra_edit(1..2, "x");
    let (changes, caret) = host_changes(&item, &ask(1, 2, 0), 2, None);
    assert_eq!(applied("pl", 2, changes, caret), ("player".to_owned(), 6));
}

#[test]
fn an_edit_that_does_not_cover_the_asked_caret_falls_back_to_the_label() {
    let item = Completion::new("player", CompletionKind::Variable).with_edit(0..1, "player");
    let (changes, caret) = host_changes(&item, &ask(1, 2, 0), 2, None);
    assert_eq!(applied("pl", 2, changes, caret), ("player".to_owned(), 6));
}

#[test]
fn overlapping_edits_fall_back_to_the_label() {
    let item = Completion::new("ab", CompletionKind::Variable)
        .with_edit(4..6, "ab")
        .with_extra_edit(0..2, "x")
        .with_extra_edit(1..3, "y");
    let (changes, caret) = host_changes(&item, &ask(1, 6, 4), 6, None);
    assert_eq!(
        applied("0123ab", 6, changes, caret),
        ("0123ab".to_owned(), 6)
    );
}

#[test]
fn resolved_extra_edits_are_used_as_they_are() {
    // Resolved for the text as it is now: not moved by the typing since.
    let item = Completion::new("abc", CompletionKind::Variable)
        .with_edit(4..6, "abc")
        .with_extra_edit(0..0, "stale");
    let resolved = [CompletionEdit::new(0..0, "use x;\n")];
    let (changes, caret) = host_changes(&item, &ask(1, 6, 4), 7, Some(&resolved));
    let (after, caret) = applied("123 abc", 7, changes, caret);
    assert_eq!(after, "use x;\n123 abc");
    assert_eq!(caret, after.len());
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
