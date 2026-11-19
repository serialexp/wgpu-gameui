//! Figures ([`TextBlock::figures`]): shaping, measuring and the caches that
//! hold a layout all tell blocks with different figures apart. Set in
//! Spectral, which has both tabular (`tnum`) and proportional (`pnum`)
//! figures; its own are tabular.

use super::*;
use crate::measure::{MeasureConstraints, MeasureContext};
use crate::{FontSpec, StyleResolver};

fn measurer() -> (TextMeasurer, FontHandle) {
    let fs = shared_font_system();
    let font = crate::test_fonts::spectral(&fs);
    (TextMeasurer::with_font_system(fs), font)
}

fn digits(font: &FontHandle, figures: Figures) -> [TextBlock; 2] {
    ["1111", "0000"].map(|text| {
        TextBlock::new(text, 0.0, 0.0)
            .with_size(20.0)
            .with_font(font.clone())
            .with_figures(figures)
    })
}

#[test]
fn figures_choose_the_digit_advances() {
    let (mut text, font) = measurer();
    let mut widths = |figures| digits(&font, figures).map(|b| text.measure_block(&b).0);
    let [ones, zeros] = widths(Figures::Proportional);
    assert!(zeros - ones > 1.0, "proportional: {ones} vs {zeros}");
    for figures in [Figures::Tabular, Figures::Default] {
        let [ones, zeros] = widths(figures);
        assert!(
            (ones - zeros).abs() < 0.01,
            "{figures:?}: {ones} vs {zeros}"
        );
    }
}

#[test]
fn a_prepared_layout_does_not_hold_once_the_figures_change() {
    let (mut text, font) = measurer();
    let [ones, _] = digits(&font, Figures::Default);
    let mut prepared = text.prepare(&ones);
    let layout = prepared.layout(&mut text, &ones);
    let plain = ones.clone().with_prepared(layout);
    assert!(text.prepared_holds(&plain));
    for figures in [Figures::Tabular, Figures::Proportional] {
        assert!(!text.prepared_holds(&plain.clone().with_figures(figures)));
    }
}

#[test]
fn a_frame_with_new_figures_places_its_glyphs_again() {
    let (_, font) = measurer();
    let [ones, _] = digits(&font, Figures::Default);
    let tabular = ones.clone().with_figures(Figures::Tabular);
    assert!(crate::render::text_change(&ones, &tabular) == crate::render::Change::Place);
    assert!(crate::render::text_change(&ones, &ones.clone()) == crate::render::Change::Same);
}

#[test]
fn measure_context_judges_overflow_on_the_drawn_figures() {
    let theme = crate::Theme::forge();
    let (mut text, font) = measurer();
    let [ones, _] = digits(&font, Figures::Tabular);
    let tabular_width = text.measure_block(&ones).0;
    let proportional = ones.clone().with_figures(Figures::Proportional);
    let proportional_width = text.measure_block(&proportional).0;
    assert!(tabular_width > proportional_width + 1.0);
    // Room for the proportional ones, not the tabular ones.
    let max_width = (tabular_width + proportional_width) / 2.0;
    let mut cx = MeasureContext::new(
        &mut text,
        StyleResolver::new(&theme),
        FontSpec {
            font: Some(font),
            size: 20.0,
            ..FontSpec::default()
        },
        MeasureConstraints::with_max_width(max_width),
        1.0,
        WrapMode::Word,
    );
    let tabular = cx.text_block("1111").with_figures(Figures::Tabular);
    assert!(cx.measure_text(tabular).metrics.overflowed_width);
    let proportional = cx.text_block("1111").with_figures(Figures::Proportional);
    assert!(!cx.measure_text(proportional).metrics.overflowed_width);
}
