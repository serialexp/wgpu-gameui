use super::*;
use crate::completion::{Completion, CompletionKind};

/// `layout`, for a list there is room for.
fn laid(
    open: &Completing,
    word: Rect,
    bounds: Rect,
    first: usize,
    styles: &StyleResolver,
    list: &mut DrawList,
) -> Popup {
    layout(open, word, bounds, first, styles, list).expect("room for it")
}

fn open(labels: &[&str], prefix: &str) -> Completing {
    let pool = labels
        .iter()
        .map(|l| Completion::new(*l, CompletionKind::Variable))
        .collect();
    Completing::open(0, prefix.to_owned(), pool, None).expect("something matches")
}

#[test]
fn the_typed_part_is_matched_whatever_its_case() {
    assert_eq!(typed_len("Player", "pl"), 2);
    assert_eq!(typed_len("player", ""), 0);
    assert_eq!(typed_len("été", "ÉT"), "ét".len());
}

#[test]
fn the_list_goes_under_the_word_its_labels_in_line_with_it() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    let open = open(&["player", "plan"], "pl");
    let word = Rect::new(200.0, 100.0, 1.0, 18.0);
    let popup = laid(
        &open,
        word,
        Rect::new(0.0, 0.0, 800.0, 600.0),
        0,
        &styles,
        &mut list,
    );
    assert_eq!(popup.rows.len(), 2);
    assert_eq!(popup.rect.y, 120.0);
    let border = styles.dropdown().surface.border_widths.left;
    assert_eq!(
        popup.rows[0].x + ROW_PAD + TAG_WIDTH + GAP,
        200.0,
        "{border}"
    );
    assert!(popup.rect.width >= MIN_WIDTH);
    assert_eq!(
        popup.row_at(popup.rows[1].x + 1.0, popup.rows[1].y + 1.0),
        Some(1)
    );
    assert_eq!(popup.row_at(0.0, 0.0), None);
}

#[test]
fn without_room_below_it_goes_over_the_word() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    let open = open(&["player", "plan"], "pl");
    let bounds = Rect::new(0.0, 0.0, 800.0, 140.0);
    let word = Rect::new(200.0, 110.0, 1.0, 18.0);
    let popup = laid(&open, word, bounds, 0, &styles, &mut list);
    assert_eq!(popup.rect.bottom(), 108.0);
}

#[test]
fn the_list_is_painted_with_each_items_tag_and_label() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut layers = LayerStack::new();
    let open = Completing::open(
        0,
        "sp".to_owned(),
        vec![
            Completion::new("spawn", CompletionKind::Function).with_detail("(kind, x, y)"),
            Completion::new("speed", CompletionKind::Variable),
        ],
        None,
    )
    .unwrap();
    let popup = laid(
        &open,
        Rect::new(100.0, 50.0, 1.0, 18.0),
        Rect::new(0.0, 0.0, 800.0, 600.0),
        0,
        &styles,
        layers.base_mut(),
    );
    paint(
        &mut layers,
        &styles,
        &SyntaxTheme::default(),
        &open,
        &popup,
        Rect::new(0.0, 0.0, 800.0, 600.0),
    );
    let layer = layers.layers().last().unwrap();
    let texts: Vec<&str> = layer
        .list
        .texts
        .iter()
        .map(|t| t.content.as_str())
        .collect();
    assert_eq!(
        texts,
        vec!["fn", "sp", "awn", "(kind, x, y)", "v", "sp", "eed"]
    );
}

fn many(count: usize) -> Completing {
    let labels: Vec<String> = (0..count).map(|n| format!("item{n}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    open(&labels, "it")
}

#[test]
fn where_neither_side_has_room_it_shows_what_fits_on_the_roomier() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    let open = many(8);
    let bounds = Rect::new(0.0, 0.0, 800.0, 100.0);
    let word = Rect::new(200.0, 40.0, 1.0, 18.0);
    let popup = laid(&open, word, bounds, 0, &styles, &mut list);
    assert!(
        popup.rect.y >= bounds.y && popup.rect.bottom() <= bounds.bottom(),
        "{popup:?}"
    );
    assert!(popup.rows.len() < 8 && !popup.rows.is_empty());
    assert_eq!(popup.first, 0);
}

#[test]
fn a_cut_short_list_scrolls_to_keep_the_choice_in_view() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    let mut open = many(8);
    let bounds = Rect::new(0.0, 0.0, 800.0, 100.0);
    let word = Rect::new(200.0, 0.0, 1.0, 18.0);
    let shown = laid(&open, word, bounds, 0, &styles, &mut list).rows.len();
    open.chosen = 7;
    let popup = laid(&open, word, bounds, 0, &styles, &mut list);
    assert_eq!(popup.first, 8 - shown, "the last row the chosen one");
    let last = popup.rows[shown - 1];
    assert_eq!(popup.row_at(last.x + 1.0, last.y + 1.0), Some(7));
    // Choosing back up the list moves it only once the choice leaves it.
    open.chosen = 8 - shown;
    let again = laid(&open, word, bounds, popup.first, &styles, &mut list);
    assert_eq!(again.first, popup.first);
    open.chosen = 0;
    assert_eq!(
        laid(&open, word, bounds, popup.first, &styles, &mut list).first,
        0
    );
}

#[test]
fn it_is_no_wider_than_the_editor() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    let open = open(&["a_very_long_name_indeed_that_runs_on_and_on"], "a");
    let bounds = Rect::new(10.0, 0.0, 150.0, 400.0);
    let popup = laid(
        &open,
        Rect::new(40.0, 20.0, 1.0, 18.0),
        bounds,
        0,
        &styles,
        &mut list,
    );
    assert_eq!(popup.rect.x, 10.0);
    assert_eq!(popup.rect.width, 150.0);
}

#[test]
fn where_not_one_row_fits_there_is_no_list() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    let bounds = Rect::new(0.0, 0.0, 800.0, 40.0);
    let word = Rect::new(200.0, 6.0, 1.0, 18.0);
    assert!(layout(&many(8), word, bounds, 0, &styles, &mut list).is_none());
}

#[test]
fn a_detail_that_would_run_into_the_label_is_left_out() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut layers = LayerStack::new();
    let open = Completing::open(
        0,
        "sp".to_owned(),
        vec![
            Completion::new("spawn", CompletionKind::Function)
                .with_detail("abcdefghijklmnopqrstuvwxyz0123456789"),
        ],
        None,
    )
    .unwrap();
    let bounds = Rect::new(0.0, 0.0, 150.0, 400.0);
    let word = Rect::new(40.0, 20.0, 1.0, 18.0);
    let popup = laid(&open, word, bounds, 0, &styles, layers.base_mut());
    paint(
        &mut layers,
        &styles,
        &SyntaxTheme::default(),
        &open,
        &popup,
        bounds,
    );
    let layer = layers.layers().last().unwrap();
    let texts: Vec<&str> = layer
        .list
        .texts
        .iter()
        .map(|t| t.content.as_str())
        .collect();
    assert_eq!(texts, vec!["fn", "sp", "awn"]);
}
