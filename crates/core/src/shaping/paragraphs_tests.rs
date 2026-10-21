use super::*;
use crate::text::{TextAlign, WrapMode, shared_font_system};

/// Text of every shape a paragraph comes in: wrapped, empty, after each kind
/// of line break, right to left, mixed, tabbed, and a final empty one.
const TEXTS: &[&str] = &[
    "first line\nsecond, long enough to wrap around the narrow width\n\nfourth\n",
    "a\r\nb\r\nc",
    "one\rtwo\n\rthree",
    "abc אבג\nשלום עולם\nmixed אב cd and more words to wrap",
    "tab\there\n\n\nend",
];

fn specs() -> Vec<LayoutSpec<'static>> {
    let base = LayoutSpec {
        wrap: WrapMode::WordOrGlyph,
        ..LayoutSpec::plain(16.0, Some(120.0))
    };
    vec![
        base,
        LayoutSpec {
            align: TextAlign::Center,
            ..base
        },
        LayoutSpec {
            align: TextAlign::End,
            letter_spacing: 1.5,
            ..base
        },
        LayoutSpec {
            direction: TextDirection::Rtl,
            ..base
        },
        LayoutSpec {
            direction: TextDirection::Ltr,
            wrap: WrapMode::None,
            ..base
        },
    ]
}

/// `content` with its `index`th paragraph changed.
fn edited(content: &str, index: usize) -> String {
    let ranges = paragraphs(content);
    let range = ranges[index.min(ranges.len() - 1)].clone();
    format!("{}x{}", &content[..range.end], &content[range.end..])
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn assert_same(got: &ShapedLayout, want: &ShapedLayout, what: &str) {
    assert_eq!(got.glyphs.len(), want.glyphs.len(), "{what}: glyphs");
    for (g, w) in got.glyphs.iter().zip(&want.glyphs) {
        let same = g.font_id == w.font_id
            && g.glyph_id == w.glyph_id
            && near(g.rel_x, w.rel_x)
            && near(g.cell_x, w.cell_x)
            && near(g.rel_y, w.rel_y)
            && near(g.advance, w.advance)
            && g.byte_start == w.byte_start
            && g.byte_end == w.byte_end
            && g.rtl == w.rtl;
        assert!(same, "{what}: {g:?} against {w:?}");
    }
    assert_eq!(got.lines.len(), want.lines.len(), "{what}: lines");
    for (l, w) in got.lines.iter().zip(&want.lines) {
        let same = near(l.top, w.top)
            && near(l.height, w.height)
            && near(l.baseline, w.baseline)
            && l.glyphs == w.glyphs
            && l.byte_start == w.byte_start
            && near(l.left, w.left)
            && near(l.right, w.right)
            && near(l.width, w.width);
        assert!(same, "{what}: {l:?} against {w:?}");
    }
    assert!(
        near(got.size.0, want.size.0) && near(got.size.1, want.size.1),
        "{what}: size"
    );
    assert_eq!(got.shown, want.shown, "{what}: shown");
}

#[test]
fn a_layout_from_another_s_paragraphs_is_the_layout_shaped_whole() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let fs = shared.font_system();
    for spec in specs() {
        for &text in TEXTS {
            let mut donors: Vec<String> = vec![text.to_owned(), "unrelated\ntext".to_owned()];
            donors.extend((0..4).map(|i| edited(text, i)));
            let want = shape_layout(fs, &spec, text);
            for donor in &donors {
                let donor_layout = shape_layout(fs, &spec, donor);
                let got = reshape(fs, &spec, text, donor, &donor_layout)
                    .expect("the donor's lines match its paragraphs");
                assert_same(
                    &got,
                    &want,
                    &format!(
                        "{text:?} from {donor:?}, {:?} {:?}",
                        spec.align, spec.direction
                    ),
                );
            }
        }
    }
}

#[test]
fn an_edit_shapes_only_the_paragraph_it_changed() {
    let content = "one\ntwo\nthree\nfour";
    let donor = "one\ntwo\nthree, typed in\nfour";
    assert_eq!(
        plan(content, donor, false),
        [Some(0), Some(1), None, Some(3)]
    );
    // Lines that moved, and one there twice, are found where they are.
    assert_eq!(
        plan("four\none\none", donor, false),
        [Some(3), Some(0), Some(0)]
    );
}

#[test]
fn a_forced_direction_takes_the_first_paragraph_only_from_a_first() {
    let donor = "same\nsame";
    assert_eq!(plan("same\nsame", donor, true), [Some(0), Some(1)]);
    assert_eq!(plan("other\nsame", donor, true), [None, Some(1)]);
    // Without its mark, the donor's second "same" can't be the first.
    assert_eq!(plan("same", "other\nsame", true), [None]);
    assert_eq!(plan("same", "other\nsame", false), [Some(1)]);
}

#[test]
fn shaping_only_what_changed_makes_one_shaping_pass() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let fs = shared.font_system();
    let spec = specs()[0];
    let text = TEXTS[0];
    let donor = edited(text, 1);
    let donor_layout = shape_layout(fs, &spec, &donor);
    let before = crate::text::shaping_passes();
    reshape(fs, &spec, text, &donor, &donor_layout).unwrap();
    assert_eq!(crate::text::shaping_passes() - before, 1);
    let whole = shape_layout(fs, &spec, text);
    let before = crate::text::shaping_passes();
    reshape(fs, &spec, text, text, &whole).unwrap();
    assert_eq!(crate::text::shaping_passes(), before, "nothing to shape");
}

/// 200 lines of about 50 characters: long enough to be laid out from another.
fn long_text() -> String {
    (0..200)
        .map(|i| format!("line {i}: the quick brown fox jumps over the lazy dog"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_cache_lays_a_long_text_out_from_its_last_version() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let spec = specs()[0];
    let text = long_text();
    assert!(is_long(&text));
    shared.layout(&spec, &text);
    assert_eq!(shared.layout_stats().reshaped, 0, "nothing to take it from");
    let typed = edited(&text, 120);
    let got = shared.layout(&spec, &typed);
    let got = (got.glyphs.clone(), got.lines.clone(), got.size);
    let want = shape_layout(shared.font_system(), &spec, &typed);
    let got = ShapedLayout {
        glyphs: got.0,
        lines: got.1,
        shown: typed.len() as u32,
        size: got.2,
        ink: OnceCell::new(),
    };
    assert_same(&got, &want, "typed into line 120");
    assert_eq!(shared.layout_stats().reshaped, 1, "laid out from the first");
}

#[test]
fn short_vertical_ellipsized_and_faced_text_is_shaped_whole() {
    let base = specs()[0];
    let long = long_text();
    assert!(reusable(&base, &long));
    assert!(!reusable(&base, "short\ntext"));
    assert!(!reusable(&base, &long.replace('\n', " ")), "one paragraph");
    assert!(!reusable(
        &LayoutSpec {
            vertical: true,
            ..base
        },
        &long
    ));
    assert!(!reusable(
        &LayoutSpec {
            ellipsize: true,
            ..base
        },
        &long
    ));
    let faces = [crate::text::FaceRange {
        range: 0..4,
        font: None,
        weight: cosmic_text::Weight::BOLD,
        style: cosmic_text::Style::Normal,
    }];
    assert!(!reusable(
        &LayoutSpec {
            faces: &faces,
            ..base
        },
        &long
    ));
}
