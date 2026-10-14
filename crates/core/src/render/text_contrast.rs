//! Contrast and gamma correction for text coverage.
//!
//! Text is anti-aliased by turning how much of a pixel the glyph covers into
//! alpha, and gameui blends that alpha in sRGB space, like a browser. In sRGB
//! space a half-covered pixel of light text on a dark background comes out
//! darker than half as bright to the eye, so light-on-dark text looks thinner
//! and fainter than the same text dark-on-light. DirectWrite corrects for this
//! with a "gamma" and an "enhanced contrast" applied to coverage; Windows
//! Terminal ports that correction to a shader, and gpui (Zed) took it from
//! there. This is the same correction, with gpui's defaults.
//!
//! The maths runs in `ui_msdf.wgsl`; `corrected_coverage` below is the same
//! formula on the CPU, so the tests can pin its numbers.
//!
//! The formulas and the ratio table are adapted from Windows Terminal's
//! `dwrite.hlsl` / `DWrite_GetGammaRatios` (Copyright (c) Microsoft
//! Corporation, MIT licence), by way of gpui.

/// How text coverage is corrected for its colour. Set it with
/// [`UiRenderer::set_text_contrast`](crate::UiRenderer::set_text_contrast).
///
/// The default ([`TextContrast::GPUI`]) makes light text on dark backgrounds
/// a little heavier, and leaves dark text on light backgrounds about as it
/// was. [`TextContrast::OFF`] draws coverage as it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextContrast {
    /// The display gamma the correction assumes, from 1.0 (no gamma
    /// correction) to 2.2. Rounded to the nearest tenth, the steps of the
    /// ratio table. Higher values thicken light text more.
    pub gamma: f32,
    /// How much darker-than-white text is thickened, from 0.0 (not at all)
    /// up. It fades out as the text colour gets lighter and is gone for
    /// colours brighter than 75%.
    pub enhanced_contrast: f32,
}

impl TextContrast {
    /// No correction: coverage is drawn as it is.
    pub const OFF: Self = Self {
        gamma: 1.0,
        enhanced_contrast: 0.0,
    };

    /// gpui's defaults for grayscale text (`ZED_FONTS_GAMMA` 1.8,
    /// `ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST` 1.0).
    pub const GPUI: Self = Self {
        gamma: 1.8,
        enhanced_contrast: 1.0,
    };

    /// The four gamma ratios the shader uses for [`gamma`](Self::gamma).
    pub(crate) fn gamma_ratios(self) -> [f32; 4] {
        gamma_correction_ratios(self.gamma)
    }

    /// [`enhanced_contrast`](Self::enhanced_contrast), never negative.
    pub(crate) fn contrast(self) -> f32 {
        self.enhanced_contrast.max(0.0)
    }
}

impl Default for TextContrast {
    fn default() -> Self {
        Self::GPUI
    }
}

/// DirectWrite's gamma ratios for a gamma-incorrect (sRGB-blending) target,
/// one row per gamma from 1.0 to 2.2 in steps of 0.1.
const GAMMA_INCORRECT_TARGET_RATIOS: [[f32; 4]; 13] = [
    [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0], // gamma = 1.0
    [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0], // gamma = 1.1
    [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0], // gamma = 1.2
    [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0], // gamma = 1.3
    [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0], // gamma = 1.4
    [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0], // gamma = 1.5
    [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0], // gamma = 1.6
    [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0], // gamma = 1.7
    [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0], // gamma = 1.8
    [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0], // gamma = 1.9
    [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0], // gamma = 2.0
    [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0], // gamma = 2.1
    [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0], // gamma = 2.2
];

/// The gamma ratios for `gamma`, rounded to the table's tenths and clamped to
/// 1.0–2.2, scaled the way DirectWrite scales them for 8-bit coverage.
fn gamma_correction_ratios(gamma: f32) -> [f32; 4] {
    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / 255.0 * 4.0) as f32;
    let gamma = if gamma.is_finite() { gamma } else { 1.0 };
    let index = ((gamma * 10.0).round() as i32).clamp(10, 22) as usize - 10;
    let r = GAMMA_INCORRECT_TARGET_RATIOS[index];
    [r[0] * NORM13, r[1] * NORM24, r[2] * NORM13, r[3] * NORM24]
}

/// Perceived brightness of an sRGB-encoded colour (Rec. 601 weights).
#[cfg(test)]
fn brightness(rgb: [f32; 3]) -> f32 {
    0.30 * rgb[0] + 0.59 * rgb[1] + 0.11 * rgb[2]
}

/// `coverage` (0–1) corrected for text of colour `rgb` (sRGB-encoded, 0–1).
/// The CPU twin of `corrected_coverage` in `ui_msdf.wgsl`; keep them equal.
///
/// Two steps, as in DirectWrite:
/// 1. **Enhanced contrast** thickens text darker than 75% brightness,
///    fading in below that: `a·(k+1) / (a·k+1)` with `k` the contrast.
/// 2. **Gamma** moves partial coverage by `a·(1-a)·correction`, where the
///    correction depends on the brightness: up for light text, slightly down
///    for dark text, so the two end up looking equally heavy.
///
/// Full and zero coverage are never changed, so glyph interiors and the
/// space around them stay exactly as they were; only the anti-aliased edge
/// moves.
#[cfg(test)]
fn corrected_coverage(coverage: f32, rgb: [f32; 3], contrast: TextContrast) -> f32 {
    let g = contrast.gamma_ratios();
    let b = brightness(rgb);
    let k = contrast.contrast() * (4.0 * (0.75 - b)).clamp(0.0, 1.0);
    let a = coverage * (k + 1.0) / (coverage * k + 1.0);
    let correction = (g[0] * b + g[1]) * a + (g[2] * b + g[3]);
    (a + a * (1.0 - a) * correction).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
    const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

    #[test]
    fn gamma_one_has_no_ratios() {
        assert_eq!(gamma_correction_ratios(1.0), [0.0; 4]);
    }

    #[test]
    fn ratios_match_gpui_at_its_default_gamma() {
        // gpui's get_gamma_correction_ratios(1.8), worked by hand.
        let r = gamma_correction_ratios(1.8);
        let expected = [0.14806, -0.89460, 1.47595, -0.32467];
        for (got, want) in r.iter().zip(expected) {
            assert!((got - want).abs() < 1e-4, "{r:?} vs {expected:?}");
        }
    }

    #[test]
    fn gamma_rounds_to_tenths_and_clamps() {
        assert_eq!(gamma_correction_ratios(1.84), gamma_correction_ratios(1.8));
        assert_eq!(gamma_correction_ratios(0.2), gamma_correction_ratios(1.0));
        assert_eq!(gamma_correction_ratios(9.0), gamma_correction_ratios(2.2));
        assert_eq!(
            gamma_correction_ratios(f32::NAN),
            gamma_correction_ratios(1.0)
        );
    }

    #[test]
    fn off_leaves_coverage_as_it_is() {
        for i in 0..=20 {
            let a = i as f32 / 20.0;
            for rgb in [WHITE, BLACK, [0.5, 0.6, 0.7]] {
                assert_eq!(corrected_coverage(a, rgb, TextContrast::OFF), a);
            }
        }
    }

    #[test]
    fn full_and_empty_coverage_never_move() {
        for rgb in [WHITE, BLACK, [0.8, 0.85, 0.9]] {
            assert_eq!(corrected_coverage(0.0, rgb, TextContrast::GPUI), 0.0);
            assert_eq!(corrected_coverage(1.0, rgb, TextContrast::GPUI), 1.0);
        }
    }

    #[test]
    fn light_text_gets_heavier() {
        // Half-covered white: 0.5 + 0.25 × (1.1513 − 0.7465 × 0.5) ≈ 0.6945.
        let a = corrected_coverage(0.5, WHITE, TextContrast::GPUI);
        assert!((a - 0.6945).abs() < 1e-3, "{a}");
        // Every partial edge pixel of light text gains ink.
        for i in 1..20 {
            let c = i as f32 / 20.0;
            assert!(corrected_coverage(c, WHITE, TextContrast::GPUI) > c);
        }
    }

    #[test]
    fn dark_text_stays_about_as_heavy() {
        // Enhanced contrast and gamma pull in opposite directions for black.
        for i in 1..20 {
            let c = i as f32 / 20.0;
            let a = corrected_coverage(c, BLACK, TextContrast::GPUI);
            assert!((a - c).abs() < 0.06, "{c} → {a}");
        }
    }

    #[test]
    fn coverage_stays_in_order() {
        // A smoother edge must not turn into a ridge: more coverage in, more out.
        for rgb in [WHITE, BLACK, [0.4, 0.5, 0.6], [0.85, 0.87, 0.9]] {
            let mut last = 0.0;
            for i in 1..=100 {
                let a = corrected_coverage(i as f32 / 100.0, rgb, TextContrast::GPUI);
                assert!(a >= last, "{rgb:?} at {i}: {a} < {last}");
                last = a;
            }
        }
    }

    #[test]
    fn negative_contrast_counts_as_none() {
        let none = TextContrast {
            gamma: 1.8,
            enhanced_contrast: 0.0,
        };
        let negative = TextContrast {
            gamma: 1.8,
            enhanced_contrast: -3.0,
        };
        assert_eq!(
            corrected_coverage(0.4, BLACK, negative),
            corrected_coverage(0.4, BLACK, none)
        );
    }
}
