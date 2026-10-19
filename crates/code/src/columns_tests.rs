use super::*;

#[test]
fn a_tab_reaches_the_next_stop() {
    let line = "\ta\tbc\td";
    assert_eq!(column_of(line, 0, 4), 0);
    assert_eq!(column_of(line, 1, 4), 4, "after the first tab");
    assert_eq!(column_of(line, 2, 4), 5);
    assert_eq!(column_of(line, 3, 4), 8, "a tab from column 5 reaches 8");
    assert_eq!(column_of(line, 6, 4), 12, "from 10 to 12");
    assert_eq!(column_of(line, line.len(), 4), 13);
}

#[test]
fn a_character_of_several_bytes_is_one_column() {
    let line = "çé→x";
    assert_eq!(column_of(line, "çé→".len(), 4), 3);
    // An accent that combines with the letter before it is one character.
    assert_eq!(column_of("e\u{301}x", "e\u{301}".len(), 4), 1);
}

#[test]
fn a_wide_character_is_two_columns() {
    let line = "日本x";
    assert_eq!(column_of(line, "日".len(), 4), 2);
    assert_eq!(column_of(line, "日本".len(), 4), 4);
    // A flag is two code points, one character, two columns.
    assert_eq!(column_of("🇳🇱x", "🇳🇱".len(), 4), 2);
    // A family is several people joined: still one character of two.
    let family = "👨\u{200d}👩\u{200d}👧";
    assert_eq!(column_of(family, family.len(), 4), 2);
}

#[test]
fn a_character_drawn_as_nothing_still_takes_a_column() {
    // A zero-width space alone: the caret can still be seen to step over it.
    assert_eq!(column_of("a\u{200b}b", "a\u{200b}".len(), 4), 2);
    // A control character is drawn as its picture, one column.
    assert_eq!(column_of("\0x", 1, 4), 1);
}

#[test]
fn a_column_is_found_at_the_boundary_nearest_it() {
    let line = "a\tb";
    assert_eq!(byte_at_column(line, 0.0, 4), 0);
    assert_eq!(byte_at_column(line, 1.0, 4), 1, "just before the tab");
    assert_eq!(byte_at_column(line, 2.0, 4), 1, "nearer the tab's start");
    assert_eq!(byte_at_column(line, 2.5, 4), 1, "a tie, the earlier end");
    assert_eq!(byte_at_column(line, 2.6, 4), 2, "nearer its end");
    assert_eq!(byte_at_column(line, 4.0, 4), 2);
    assert_eq!(byte_at_column(line, 99.0, 4), 3, "past the end, the end");
    assert_eq!(
        byte_at_column(line, -3.0, 4),
        0,
        "before the start, the start"
    );
    assert_eq!(byte_at_column("çé", 1.0, 4), 2, "on a character's boundary");
    assert_eq!(
        byte_at_column("日x", 0.9, 4),
        0,
        "nearer a wide one's start"
    );
    assert_eq!(byte_at_column("日x", 1.1, 4), 3, "nearer its end");
}

#[test]
fn plain_text_is_drawn_in_one_piece_and_the_rest_one_character_each() {
    let (pieces, columns) = pieces("let s = \"日é\";", 4, 0..usize::MAX);
    let drawn: Vec<(&str, usize)> = pieces
        .iter()
        .map(|piece| (piece.text.as_str(), piece.column))
        .collect();
    assert_eq!(
        drawn,
        [("let s = \"", 0), ("日", 9), ("é", 11), ("\";", 12)]
    );
    assert_eq!(columns, 14);
    // Each says which bytes of the line it shows.
    assert_eq!(pieces[1].source, 9..12);
    assert_eq!(pieces[2].source, 12..14);
}

#[test]
fn a_tab_is_drawn_as_the_space_to_the_next_stop() {
    let (pieces, columns) = pieces("\tif\tx", 4, 0..usize::MAX);
    let drawn: Vec<(&str, usize)> = pieces
        .iter()
        .map(|piece| (piece.text.as_str(), piece.column))
        .collect();
    assert_eq!(drawn, [("if", 4), ("x", 8)]);
    assert_eq!(columns, 9);
    assert_eq!(drawn_text("\tif\tx", 4), "    if  x");
}

#[test]
fn control_characters_are_drawn_as_their_pictures() {
    // A stray \r, a NUL: the reader can see something is there.
    assert_eq!(drawn_text("a\rb\0", 4), "a\u{240d}b\u{2400}");
    let (drawn, _) = pieces("a\rb", 4, 0..usize::MAX);
    assert_eq!(drawn[1].text, "\u{240d}");
    assert_eq!(drawn[1].source, 1..2);
}

#[test]
fn only_the_columns_asked_for_are_drawn() {
    let line = "a".repeat(10_000);
    let (run, columns) = pieces(&line, 4, 5_000..5_100);
    assert_eq!(run.len(), 1);
    assert_eq!(run[0].column, 5_000);
    assert_eq!(run[0].text.len(), 100);
    assert_eq!(run[0].source, 5_000..5_100);
    assert_eq!(columns, 10_000, "the whole line's width all the same");
    // A wide character half in view is drawn whole.
    let (wide, _) = pieces("日本語", 4, 3..4);
    assert_eq!(wide.len(), 1);
    assert_eq!((wide[0].text.as_str(), wide[0].column), ("本", 2));
}

#[test]
fn a_wide_character_fills_its_two_columns_in_the_drawn_text() {
    assert_eq!(drawn_text("日\tx", 4), "日  x");
}
