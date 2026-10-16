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
    // A flag is two code points, one character.
    let flag = "🇳🇱x";
    assert_eq!(column_of(flag, "🇳🇱".len(), 4), 1);
}

#[test]
fn a_column_is_found_at_the_boundary_nearest_it() {
    let line = "a\tb";
    assert_eq!(byte_at_column(line, 0, 4), 0);
    assert_eq!(byte_at_column(line, 1, 4), 1, "just before the tab");
    assert_eq!(byte_at_column(line, 2, 4), 1, "nearer the tab's start");
    assert_eq!(byte_at_column(line, 3, 4), 2, "nearer its end");
    assert_eq!(byte_at_column(line, 4, 4), 2);
    assert_eq!(byte_at_column(line, 99, 4), 3, "past the end, the end");
    assert_eq!(byte_at_column("çé", 1, 4), 2, "on a character's boundary");
}

#[test]
fn a_line_is_drawn_with_its_tabs_as_spaces() {
    let shown = Shown::new("\tif\tx", 4);
    assert_eq!(shown.text, "    if  x");
    assert_eq!(shown.to_shown(0), 0);
    assert_eq!(shown.to_shown(1), 4);
    assert_eq!(shown.to_shown(3), 6);
    assert_eq!(shown.to_shown(4), 8);
    assert_eq!(shown.to_shown(5), 9);
    // A range given in the line maps onto the drawn text.
    assert_eq!(shown.range(1..3), 4..6);
}

#[test]
fn a_line_without_tabs_is_drawn_as_it_is() {
    let shown = Shown::new("plain", 4);
    assert_eq!(shown.text, "plain");
    assert_eq!(shown.to_shown(3), 3);
}

#[test]
fn control_characters_are_drawn_as_their_pictures() {
    // A stray \r, a NUL: the reader can see something is there.
    let shown = Shown::new("a\rb\0", 4);
    assert_eq!(shown.text, "a\u{240d}b\u{2400}");
    assert_eq!(shown.to_shown(2), 4, "the picture is three bytes");
    assert_eq!(shown.to_shown(4), 8);
}

#[test]
fn a_place_in_the_drawn_text_maps_back_to_the_line() {
    let line = "\tif\tx";
    let shown = Shown::new(line, 4);
    assert_eq!(shown.to_line(line, 0), 0);
    assert_eq!(
        shown.to_line(line, 1),
        0,
        "inside the tab, nearer its start"
    );
    assert_eq!(shown.to_line(line, 2), 0, "a tie, the earlier end");
    assert_eq!(shown.to_line(line, 3), 1, "nearer its end");
    assert_eq!(shown.to_line(line, 4), 1);
    assert_eq!(shown.to_line(line, 5), 2);
    assert_eq!(shown.to_line(line, 9), 5);
    assert_eq!(shown.to_line(line, 99), 5, "past the end, the end");
    // A picture is three bytes for the line's one.
    let line = "a\rb";
    let shown = Shown::new(line, 4);
    assert_eq!(shown.to_line(line, 4), 2);
    assert_eq!(shown.to_line(line, 5), 3);
}
