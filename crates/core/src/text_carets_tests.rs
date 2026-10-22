use super::*;
use crate::shaping::{LayoutSpec, shape_layout};
use crate::text::{TextAlign, TextDirection, WrapMode, shared_font_system};

const WIDTH: f32 = 90.0;

fn spec(wrap: WrapMode, direction: TextDirection) -> LayoutSpec<'static> {
    LayoutSpec {
        wrap,
        direction,
        ..LayoutSpec::plain(16.0, Some(WIDTH))
    }
}

#[test]
fn a_centred_line_starts_and_ends_its_carets_at_its_drawn_edges() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let fs = shared.font_system();
    let spec = LayoutSpec {
        align: TextAlign::Center,
        ..spec(WrapMode::None, TextDirection::Auto)
    };
    let layout = shape_layout(fs, &spec, "ab");
    let carets = caret_positions(&layout);
    let first = layout.glyphs.first().unwrap();
    let last = layout.glyphs.last().unwrap();
    assert!(first.cell_x > 1.0, "centred: {}", first.cell_x);
    assert_eq!(carets.first().unwrap().x, first.cell_x);
    let end = carets.last().unwrap();
    assert!(
        (end.x - (last.cell_x + last.advance)).abs() < 0.01,
        "{end:?}"
    );
    // An empty centred line's caret is in the middle.
    let layout = shape_layout(fs, &spec, "ab\n");
    let carets = caret_positions(&layout);
    assert_eq!(carets.last().unwrap().x, WIDTH / 2.0, "{carets:?}");
}

#[test]
fn the_text_an_ellipsis_cut_off_has_no_carets() {
    let handle = shared_font_system();
    let mut shared = handle.lock().unwrap();
    let spec = LayoutSpec {
        ellipsize: true,
        ..spec(WrapMode::None, TextDirection::Auto)
    };
    let layout = shared.layout(&spec, "a sentence far too long for its line");
    let shown = layout.shown as usize;
    assert!(shown > 0 && shown < 30, "cut at {shown}");
    let carets = caret_positions(layout);
    assert_eq!(carets.last().unwrap().byte, shown);
    assert!(carets.iter().all(|c| c.byte <= shown));
    // The last caret is after the last letter kept, before the ellipsis.
    let kept = layout
        .glyphs
        .iter()
        .rfind(|g| g.byte_start < layout.shown)
        .unwrap();
    let ellipsis = layout.glyphs.last().unwrap();
    assert!(ellipsis.byte_start >= layout.shown, "the ellipsis is drawn");
    let end = carets.last().unwrap().x;
    assert!((end - (kept.cell_x + kept.advance)).abs() < 0.01, "{end}");
    assert!(
        end <= ellipsis.cell_x + 0.01,
        "{end} against {}",
        ellipsis.cell_x
    );
    assert!(visual_glyphs(layout).iter().all(|g| g.byte_end <= shown));
}
