//! Optical centring of a styled [`TextBlock`]: the metrics of the face the
//! block is really set in (its family, weight and style), placed in its own
//! line box, with the band chosen from the content it displays.

use crate::text::{FontVMetrics, LINE_HEIGHT_RATIO, TextBlock, TextMeasurer};

impl FontVMetrics {
    /// These metrics for a line box of `line_height_ratio` × the font size
    /// instead of the default 1.25. The line's glyphs are
    /// centred in its box, so a taller box moves both baselines down by half
    /// the extra height; the x-height, cap height and ideographic centre are
    /// the face's and stay.
    pub fn in_line_box(self, line_height_ratio: f32) -> Self {
        let shift = (line_height_ratio - LINE_HEIGHT_RATIO) / 2.0;
        Self {
            baseline_ratio: self.baseline_ratio + shift,
            cjk_baseline_ratio: self.cjk_baseline_ratio + shift,
            ..self
        }
    }
}

impl TextMeasurer {
    /// The vertical metrics of the face `block` is set in (its font, weight
    /// and style), with the baselines in the block's own line box.
    pub fn block_vmetrics(&mut self, block: &TextBlock) -> FontVMetrics {
        let metrics = self.vmetrics(block.font.as_ref(), block.weight, block.style);
        if block.font_size > 0.0 {
            metrics.in_line_box(block.line_height / block.font_size)
        } else {
            metrics
        }
    }

    /// The top `y` that optically centres the one-line `block` over the span
    /// `[top, top + height]`: the band its displayed content occupies (x-height
    /// with lowercase, cap height without, the ideographic centre with CJK) in
    /// the face it is set in. The block's content is what is drawn, so a caps
    /// role's upper-cased text centres on the cap band.
    pub fn vcentered_block_y(&mut self, block: &TextBlock, top: f32, height: f32) -> f32 {
        let metrics = self.block_vmetrics(block);
        top + height / 2.0 - block.font_size * metrics.visual_center_ratio(&block.content)
    }

    /// The top `y` that puts the one-line `block`'s **x-height** band on `cy`
    /// whatever its letters, in the face it is set in, so a column of rows
    /// shares one baseline.
    pub fn x_centered_block_y(&mut self, block: &TextBlock, cy: f32) -> f32 {
        let metrics = self.block_vmetrics(block);
        cy - block.font_size * (metrics.baseline_ratio - metrics.x_ratio * 0.5)
    }
}

#[cfg(all(test, feature = "bundled-font"))]
mod tests {
    use crate::text::{FontHandle, LINE_HEIGHT_RATIO, TextBlock, TextMeasurer, shared_font_system};
    use crate::{Ink, StyleResolver, TextRole, Theme};
    use cosmic_text::{Style, Weight};

    fn measurer() -> (TextMeasurer, FontHandle) {
        let fs = shared_font_system();
        let spectral = crate::test_fonts::spectral(&fs);
        (TextMeasurer::with_font_system(fs), spectral)
    }

    /// The ink band's centre once `block` is placed at `y`.
    fn ink_centre(text: &mut TextMeasurer, block: &TextBlock, y: f32) -> f32 {
        let (top, bottom) = text.measure_block_ink(block).expect("the block inks");
        y + (top + bottom) / 2.0
    }

    #[test]
    fn a_taller_line_box_moves_the_baseline_by_half_the_extra() {
        let (mut text, _) = measurer();
        let size = 20.0;
        let block = TextBlock::new("HOME", 0.0, 0.0).with_size(size);
        let tall = block.clone().with_line_height(size * 2.0);
        let (top, _) = text.measure_block_ink(&block).unwrap();
        let (tall_top, _) = text.measure_block_ink(&tall).unwrap();
        let expected = size * (2.0 - LINE_HEIGHT_RATIO) / 2.0;
        assert!(
            (tall_top - top - expected).abs() < 0.5,
            "ink moved {} for {expected}",
            tall_top - top
        );
        let shift =
            text.block_vmetrics(&tall).baseline_ratio - text.block_vmetrics(&block).baseline_ratio;
        assert!((shift * size - expected).abs() < 1e-3);
    }

    #[test]
    fn a_centred_block_centres_its_ink_in_every_face_and_line_box() {
        let (mut text, spectral) = measurer();
        let plex = TextBlock::new("HOME", 0.0, 0.0).with_size(24.0);
        let blocks = [
            plex.clone(),
            plex.clone().with_weight(Weight::BOLD),
            plex.clone().with_style(Style::Italic),
            plex.clone().with_line_height(24.0 * 1.8),
            plex.clone().with_font(spectral.clone()),
            plex.with_font(spectral).with_style(Style::Italic),
        ];
        for block in blocks {
            let y = text.vcentered_block_y(&block, 0.0, 100.0);
            let centre = ink_centre(&mut text, &block, y);
            // "HOME" inks the cap band, plus the O's overshoot.
            assert!(
                (centre - 50.0).abs() < 1.0,
                "{:?} {:?} lh {}: ink centre {centre}",
                block.weight,
                block.style,
                block.line_height
            );
        }
    }

    #[test]
    fn styled_blocks_read_their_own_faces_metrics() {
        let (mut text, spectral) = measurer();
        let block = TextBlock::new("x", 0.0, 0.0)
            .with_size(20.0)
            .with_font(spectral.clone())
            .with_style(Style::Italic);
        assert_eq!(
            text.block_vmetrics(&block),
            text.vmetrics(Some(&spectral), Weight::NORMAL, Style::Italic)
        );
        let regular = text.vmetrics(None, Weight::NORMAL, Style::Normal);
        assert_ne!(text.block_vmetrics(&block), regular);
    }

    #[test]
    fn a_caps_role_centres_on_the_cap_band() {
        let theme = Theme::forge();
        let styles = StyleResolver::new(&theme);
        let block = styles.role_block("items", 0.0, 0.0, TextRole::Label, Ink::Max);
        assert_eq!(block.content.as_str(), "ITEMS");
        let (mut text, _) = measurer();
        let caps = text.vcentered_block_y(&block, 0.0, 40.0);
        let mut lower = block.clone();
        lower.content = "items".into();
        let x_band = text.vcentered_block_y(&lower, 0.0, 40.0);
        assert!(caps > x_band, "caps {caps} should sit lower than {x_band}");
        let m = text.block_vmetrics(&block);
        let expected = 20.0 - block.font_size * (m.baseline_ratio - m.cap_ratio / 2.0);
        assert!((caps - expected).abs() < 1e-4);
    }

    #[test]
    fn x_centring_ignores_the_letters() {
        let (mut text, _) = measurer();
        let lower = TextBlock::new("rows", 0.0, 0.0).with_size(14.0);
        let mut caps = lower.clone();
        caps.content = "ROWS".into();
        assert_eq!(
            text.x_centered_block_y(&lower, 30.0),
            text.x_centered_block_y(&caps, 30.0)
        );
    }
}
