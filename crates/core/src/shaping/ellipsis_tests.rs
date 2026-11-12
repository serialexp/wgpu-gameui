use super::super::*;
use crate::text::{shaping_passes, shared_font_system};

const LONG: &str = "a label far too long for the hundred and twenty pixels it gets";

fn ellipsized(max_width: f32) -> LayoutSpec<'static> {
    LayoutSpec {
        ellipsize: true,
        ..LayoutSpec::plain(14.0, Some(max_width))
    }
}

#[test]
fn a_cut_label_is_shaped_unwrapped_once_then_cut() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    // The ellipsis is laid out once for the spec, with the first label cut.
    shared.layout(
        &ellipsized(120.0),
        "another label far too long for its width",
    );
    let before = shaping_passes();
    let shown = shared.layout(&ellipsized(120.0), LONG).shown as usize;
    assert!(shown < LONG.len(), "cut at {shown}");
    assert_eq!(
        shaping_passes() - before,
        2,
        "the text unwrapped, then its cut line"
    );
    // Measuring the block reads the unwrapped layout the cut was read from.
    let before = shaping_passes();
    shared.layout(&ellipsized(120.0).natural(), LONG);
    assert_eq!(shaping_passes() - before, 0, "measuring the label");
}

#[test]
fn a_label_that_fits_and_is_only_drawn_is_shaped_once() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let spec = ellipsized(1000.0);
    let before = (shaping_passes(), shared.layout_stats());
    let shown = shared.layout(&spec, LONG).shown as usize;
    assert_eq!(shown, LONG.len(), "it fits");
    let stats = shared.layout_stats();
    assert_eq!(
        shaping_passes() - before.0,
        1,
        "unwrapped, then laid out again"
    );
    assert_eq!(stats.shaped - before.1.shaped, 1);
    assert_eq!(stats.laid_out - before.1.laid_out, 1);
    // Measuring it afterwards finds the unwrapped layout kept.
    let before = shaping_passes();
    shared.layout(&spec.natural(), LONG);
    assert_eq!(shaping_passes(), before, "measuring the label");
}

#[test]
fn a_label_measured_then_drawn_is_shaped_twice_as_before() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let spec = ellipsized(1000.0);
    shared.layout(&spec.natural(), LONG);
    let before = shaping_passes();
    shared.layout(&spec, LONG);
    assert_eq!(shaping_passes() - before, 1, "the cut layout's own shape");
}

/// Laid out again from the unwrapped shaping, a fitting label is what
/// shaping it within its width gives, whatever its alignment, direction or
/// lines.
#[test]
fn a_fitting_label_laid_out_again_is_the_one_shaped_within_its_width() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let texts = ["short", "שלום עולם", "two\nlines", "mixed אב text"];
    let aligns = [
        crate::text::TextAlign::Start,
        crate::text::TextAlign::Center,
        crate::text::TextAlign::End,
        crate::text::TextAlign::Right,
    ];
    for text in texts {
        for align in aligns {
            let spec = LayoutSpec {
                align,
                ..ellipsized(300.5)
            };
            let got = shared.layout(&spec, text);
            let got = (got.glyphs.clone(), got.lines.clone(), got.shown, got.size);
            let want = shape_layout(shared.font_system(), &spec, text);
            let what = format!("{text:?} {align:?}");
            assert_eq!(got.0.len(), want.glyphs.len(), "{what}");
            let key = |g: &ShapedGlyph| {
                (
                    g.glyph_id,
                    g.rel_x.to_bits(),
                    g.rel_y.to_bits(),
                    g.byte_start,
                )
            };
            for (g, w) in got.0.iter().zip(&want.glyphs) {
                assert_eq!(key(g), key(w), "{what}");
            }
            for (l, w) in got.1.iter().zip(&want.lines) {
                assert_eq!(
                    (l.left.to_bits(), l.right.to_bits(), l.top.to_bits()),
                    (w.left.to_bits(), w.right.to_bits(), w.top.to_bits()),
                    "{what}"
                );
            }
            assert_eq!((got.2, got.3), (want.shown, want.size), "{what}");
        }
    }
}

#[test]
fn a_label_that_fits_is_not_cut() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    assert_eq!(shared.layout(&ellipsized(1000.0), "short").shown, 5);
}

#[test]
fn a_cut_label_ends_in_an_ellipsis_and_fits() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let layout = shared.layout(&ellipsized(80.0), LONG);
    let shown = layout.shown as usize;
    assert!(0 < shown && shown < LONG.len(), "cut at {shown}");
    assert!(layout.size.0 <= 80.0, "{} wide", layout.size.0);
    let last = layout.glyphs.last().expect("glyphs");
    assert!(last.byte_start as usize >= shown, "the ellipsis is last");
}

#[test]
fn with_no_room_for_a_letter_a_label_is_just_the_ellipsis() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let layout = shared.layout(&ellipsized(2.0), "anything");
    assert_eq!(layout.shown, 0);
    assert_eq!(layout.glyphs.len(), 1, "the ellipsis alone");
}

#[test]
fn a_right_to_left_label_keeps_the_start_of_its_text() {
    let text = "שלום עולם, זהו משפט ארוך מאוד שלא נכנס ברוחב";
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    for direction in [TextDirection::Auto, TextDirection::Rtl] {
        let spec = LayoutSpec {
            direction,
            ..ellipsized(90.0)
        };
        let layout = shared.layout(&spec, text);
        let shown = layout.shown as usize;
        assert!(
            0 < shown && shown < text.len(),
            "{direction:?}: cut at {shown}"
        );
        assert!(
            layout.size.0 <= 90.0,
            "{direction:?}: {} wide",
            layout.size.0
        );
        assert!(
            layout.glyphs.iter().any(|g| g.byte_start == 0),
            "{direction:?}: the first letter is kept"
        );
    }
}
