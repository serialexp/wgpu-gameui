//! `DrawList::caret_layout` and `visual_layout` against the glyphs
//! `DrawList::text` draws (`docs/design/shared-text-layout.md`, Phase 1).

use crate::shaping::LayoutSpec;
use crate::text::{CaretPos, TextAlign, TextBlock, caret_for_byte, shaping_passes};
use crate::{DrawList, Weight};

const TEXT: &str = "Wide hello";
/// How far a caret may sit from the edge it belongs at, in drawn px.
const NEAR: f32 = 0.01;

fn block() -> TextBlock {
    TextBlock::new(TEXT, 10.0, 20.0)
        .with_size(16.0)
        .with_max_width(300.0)
}

/// `block` drawn into `list`, and its carets: each glyph's left edge, and the
/// end of the line, checked against the cells of the layout it was drawn with.
/// `scale` is the list's: the drawn layout is in drawn px, the carets in the
/// block's.
fn assert_carets_on_the_drawn_glyphs(list: &mut DrawList, block: TextBlock, scale: f32) {
    let carets: Vec<CaretPos> = list.caret_layout(&block).to_vec();
    list.text(block);
    let drawn = list.texts.last().expect("the block was drawn").clone();
    let handle = list.text_measurer.font_system_handle();
    let mut shared = handle.lock().unwrap();
    let layout = shared.layout(&LayoutSpec::of_block(&drawn), &drawn.content);
    assert!(!layout.glyphs.is_empty());
    for glyph in &layout.glyphs {
        let caret = caret_for_byte(&carets, glyph.byte_start as usize);
        assert!(
            (caret.x * scale - glyph.cell_x).abs() < NEAR,
            "byte {}: caret {} against cell {}",
            glyph.byte_start,
            caret.x * scale,
            glyph.cell_x
        );
    }
    let last = layout.glyphs.last().unwrap();
    let end = caret_for_byte(&carets, TEXT.len());
    assert!(
        (end.x * scale - (last.cell_x + last.advance)).abs() < NEAR,
        "end caret {} against {}",
        end.x * scale,
        last.cell_x + last.advance
    );
}

#[cfg(feature = "bundled-font")]
#[test]
fn the_caret_sits_on_the_glyphs_of_a_block_in_its_own_font() {
    let mut list = DrawList::new();
    let mono = crate::text::bundled_mono_font(&list.text_measurer.font_system_handle())
        .expect("the bundled mono face");
    assert_carets_on_the_drawn_glyphs(&mut list, block().with_font(mono), 1.0);
}

#[test]
fn the_caret_sits_on_the_glyphs_of_a_bold_block() {
    let mut list = DrawList::new();
    assert_carets_on_the_drawn_glyphs(&mut list, block().with_weight(Weight::BOLD), 1.0);
}

#[test]
fn the_caret_sits_on_the_glyphs_of_a_letter_spaced_block() {
    let mut list = DrawList::new();
    assert_carets_on_the_drawn_glyphs(&mut list, block().with_letter_spacing(3.0), 1.0);
}

#[test]
fn the_caret_sits_on_the_glyphs_of_a_centred_block() {
    let mut list = DrawList::new();
    let carets = list.caret_layout(&block().with_align(TextAlign::Center));
    assert!(
        carets[0].x > 50.0,
        "centred, not at the left: {:?}",
        carets[0]
    );
    assert_carets_on_the_drawn_glyphs(&mut list, block().with_align(TextAlign::Center), 1.0);
}

#[test]
fn the_caret_sits_on_the_glyphs_of_a_block_drawn_under_a_scale() {
    let mut list = DrawList::new();
    list.push_transform();
    list.scale(2.0, 2.0);
    assert_carets_on_the_drawn_glyphs(&mut list, block().with_letter_spacing(2.0), 2.0);
    list.pop_transform();
}

#[test]
fn letter_spacing_grows_with_a_zoom_as_the_text_does() {
    let spaced = block().with_letter_spacing(3.0);
    let plain = DrawList::new().caret_layout(&spaced).to_vec();
    let mut zoomed = DrawList::new();
    zoomed.push_transform();
    zoomed.scale(2.0, 2.0);
    let carets = zoomed.caret_layout(&spaced).to_vec();
    zoomed.pop_transform();
    assert_eq!(plain.len(), carets.len());
    for (plain, zoomed) in plain.iter().zip(&carets) {
        assert!(
            (plain.x - zoomed.x).abs() < 0.05,
            "in the block's px: {plain:?} against {zoomed:?}"
        );
    }
}

#[test]
fn an_empty_centred_block_has_its_caret_in_the_middle() {
    let mut list = DrawList::new();
    let block = TextBlock::new("", 0.0, 0.0)
        .with_size(16.0)
        .with_max_width(300.0)
        .with_align(TextAlign::Center);
    let carets = list.caret_layout(&block);
    assert_eq!(carets.len(), 1, "{carets:?}");
    assert_eq!((carets[0].byte, carets[0].x), (0, 150.0));
    assert!(carets[0].line_height > 0.0);
    assert!(list.visual_layout(&block).is_empty());
}

#[test]
fn carets_are_worked_out_once_per_layout() {
    let mut list = DrawList::new();
    let first = list.caret_layout(&block());
    let before = shaping_passes();
    let again = list.caret_layout(&block());
    assert_eq!(shaping_passes(), before, "the layout was kept");
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "and its carets with it"
    );
}

#[test]
fn carets_are_right_again_after_their_layout_is_evicted() {
    let mut list = DrawList::new();
    let carets = list.caret_layout(&block());
    let visual = list.visual_layout(&block());
    list.text_measurer
        .font_system_handle()
        .lock()
        .unwrap()
        .set_layout_budget(0);
    list.caret_layout(&block().with_size(20.0));
    let before = shaping_passes();
    let again = list.caret_layout(&block());
    assert_eq!(shaping_passes() - before, 1, "shaped again, once evicted");
    assert!(!std::sync::Arc::ptr_eq(&again, &carets), "worked out again");
    assert_eq!(again, carets, "the same as the carets still held");
    assert_eq!(list.visual_layout(&block()), visual);
}
