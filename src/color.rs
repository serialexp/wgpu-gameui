//! Color helpers: hex / OKLCH constructors, host-boundary sRGB conversion, and
//! HSV(A) ↔ RGB(A) conversion.
//!
//! # The crate's colour convention
//!
//! Every colour in the crate — theme fields, [`StyleKey`](crate::StyleKey)
//! values, draw-list vertices, chrome, shadows, tints, text — is **straight
//! (non-premultiplied) sRGB-encoded RGBA in `[0, 1]`**: exactly what a CSS hex
//! value means. `#3ebfc6` is `[0x3e/255, 0xbf/255, 0xc6/255, 1.0]`, and
//! [`rgb8`] / [`hex`] build it for you.
//!
//! The renderer blends, filters and interpolates gradients **in sRGB space**,
//! the way a browser does, so a translucent layer or a two-stop gradient looks
//! the same here as in the HTML design it came from. (It draws into a non-sRGB
//! view of the target; for an `*Srgb` host target it draws offscreen and
//! composites — see [`UiRenderer`](crate::UiRenderer).)
//!
//! Linear light only appears at the host boundary: a `wgpu::Color` clear value
//! for an `*Srgb` target, or a scene texture sampled through an sRGB view. Use
//! [`srgb_to_linear`] / [`linear_to_srgb`] there — never for theme or widget
//! colours.
//!
//! Design tokens authored as `oklch(L C H)` should be written with [`oklch`] at
//! the definition site rather than copied as hand-converted hex: a wrong
//! hand conversion is exactly how a second, off-hue accent once crept in.
//!
//! **Why a dedicated HSV type?** An interactive color picker must keep HSV as
//! its source of truth: HSV→RGB is total, but RGB→HSV is *lossy* at the
//! degenerate points — at `value == 0` (black) every hue/saturation maps to the
//! same RGB, and at `saturation == 0` (gray) every hue does. If a picker stored
//! RGB and re-derived HSV each frame, the hue/saturation cursors would snap to
//! zero the instant you dragged value or saturation to an edge. Storing [`Hsva`]
//! avoids that round-trip entirely.

/// Opaque colour from 8-bit sRGB channels: `rgb8([0x3e, 0xbf, 0xc6])` is CSS
/// `#3ebfc6`.
pub const fn rgb8(rgb: [u8; 3]) -> [f32; 4] {
    rgba8(rgb, 1.0)
}

/// Colour from 8-bit sRGB channels plus a straight alpha in `[0, 1]`:
/// `rgba8([0, 0, 0], 0.6)` is CSS `rgba(0,0,0,0.6)`.
pub const fn rgba8(rgb: [u8; 3], alpha: f32) -> [f32; 4] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        alpha,
    ]
}

/// Opaque colour from a `0xRRGGBB` literal: `hex(0x3ebfc6)` is CSS `#3ebfc6`.
pub const fn hex(rgb: u32) -> [f32; 4] {
    rgb8([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

/// A CSS `oklch(L C H / alpha)` colour, converted to sRGB the way browsers
/// render it: OKLab → linear sRGB → sRGB transfer, with each out-of-gamut
/// channel clipped to `[0, 1]`.
///
/// `l` is the lightness in `[0, 1]` (CSS `0.74`, not `74%`), `c` the chroma,
/// `h` the hue in degrees. `oklch(0.74, 0.11, 200.0, 1.0)` is `#3ebfc6`.
pub fn oklch(l: f32, c: f32, h: f32, alpha: f32) -> [f32; 4] {
    let (sin, cos) = h.to_radians().sin_cos();
    let (a, b) = (c * cos, c * sin);
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let b = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    [
        linear_channel_to_srgb(r.clamp(0.0, 1.0)),
        linear_channel_to_srgb(g.clamp(0.0, 1.0)),
        linear_channel_to_srgb(b.clamp(0.0, 1.0)),
        alpha,
    ]
}

/// Return `color` with its alpha replaced by `alpha`.
pub const fn with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], alpha]
}

/// Quantise one `0.0..=1.0` channel to 8 bits: clamped, then rounded to the
/// nearest step (so `hex`/`rgb8` values round-trip exactly).
pub fn unit_to_u8(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Quantise an `[r, g, b, a]` colour to 8-bit RGBA via [`unit_to_u8`].
pub fn to_rgba8(color: [f32; 4]) -> [u8; 4] {
    color.map(unit_to_u8)
}

/// Decode one sRGB-encoded channel to linear light. Host boundary only — see
/// the module docs.
pub fn srgb_channel_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode one linear-light channel with the sRGB transfer function. Host
/// boundary only — see the module docs.
pub fn linear_channel_to_srgb(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// Decode straight sRGB-encoded RGBA to straight linear RGBA (alpha unchanged).
///
/// Only for the host boundary — e.g. turning a theme colour into the
/// `wgpu::Color` clear value of an `*Srgb` target, whose clear values are
/// linear. Theme and widget colours stay sRGB-encoded.
pub fn srgb_to_linear(rgba: [f32; 4]) -> [f32; 4] {
    [
        srgb_channel_to_linear(rgba[0]),
        srgb_channel_to_linear(rgba[1]),
        srgb_channel_to_linear(rgba[2]),
        rgba[3],
    ]
}

/// Encode straight linear RGBA as straight sRGB-encoded RGBA (alpha
/// unchanged). The inverse of [`srgb_to_linear`]; host boundary only.
pub fn linear_to_srgb(rgba: [f32; 4]) -> [f32; 4] {
    [
        linear_channel_to_srgb(rgba[0]),
        linear_channel_to_srgb(rgba[1]),
        linear_channel_to_srgb(rgba[2]),
        rgba[3],
    ]
}

/// A color in HSVA space.
///
/// - `h` (hue) is in degrees, normalized to `[0, 360)`.
/// - `s` (saturation), `v` (value/brightness) and `a` (alpha) are in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    /// Hue in degrees, `[0, 360)`.
    pub h: f32,
    /// Saturation, `[0, 1]`.
    pub s: f32,
    /// Value / brightness, `[0, 1]`.
    pub v: f32,
    /// Alpha / opacity, `[0, 1]`.
    pub a: f32,
}

impl Hsva {
    /// Construct an [`Hsva`]. Hue wraps into `[0, 360)`; `s`/`v`/`a` clamp to
    /// `[0, 1]` so a constructed value is always in range.
    pub fn new(h: f32, s: f32, v: f32, a: f32) -> Self {
        Self {
            h: h.rem_euclid(360.0),
            s: s.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    /// Opaque [`Hsva`] (`a = 1`).
    pub fn opaque(h: f32, s: f32, v: f32) -> Self {
        Self::new(h, s, v, 1.0)
    }

    /// Convert to straight (non-premultiplied) RGBA in `[0, 1]`.
    pub fn to_rgba(self) -> [f32; 4] {
        let [r, g, b] = hsv_to_rgb(self.h, self.s, self.v);
        [r, g, b, self.a]
    }

    /// Best-effort conversion from RGBA. Alpha passes through. Hue is `0` for
    /// grays/black (where it's undefined) — see the module note on why pickers
    /// shouldn't rely on this mid-drag.
    pub fn from_rgba(rgba: [f32; 4]) -> Self {
        let (h, s, v) = rgb_to_hsv([rgba[0], rgba[1], rgba[2]]);
        Self {
            h,
            s,
            v,
            a: rgba[3].clamp(0.0, 1.0),
        }
    }
}

/// HSV → RGB. `h` in degrees (wrapped to `[0, 360)`), `s`/`v` in `[0, 1]`;
/// returns straight RGB in `[0, 1]`. Standard six-sextant conversion.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s; // chroma
    let h6 = h / 60.0;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        // 5 and the h == 360→0 wrap (h6 can be exactly 6.0 only if h were 360,
        // which rem_euclid prevents, but guard the arm anyway).
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r1 + m, g1 + m, b1 + m]
}

/// RGB → HSV. `rgb` in `[0, 1]`; returns `(h_degrees, s, v)` with `h ∈ [0, 360)`.
/// Hue is `0` when undefined (achromatic: `max == min`).
pub fn rgb_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let r = rgb[0].clamp(0.0, 1.0);
    let g = rgb[1].clamp(0.0, 1.0);
    let b = rgb[2].clamp(0.0, 1.0);

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    let s = if max <= 0.0 { 0.0 } else { delta / max };

    let h = if delta <= 0.0 {
        0.0 // achromatic — hue undefined
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    (h.rem_euclid(360.0), s, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn rgb_close(a: [f32; 3], b: [f32; 3]) -> bool {
        close(a[0], b[0]) && close(a[1], b[1]) && close(a[2], b[2])
    }

    #[test]
    fn srgb_conversion_decodes_rgb_and_preserves_alpha() {
        let converted = srgb_to_linear([0.5, 0.04045, 0.0, 0.37]);
        assert!(close(converted[0], 0.214_041_14));
        assert!(close(converted[1], 0.003_130_805));
        assert_eq!(converted[2], 0.0);
        assert_eq!(converted[3], 0.37);
    }

    #[test]
    fn linear_to_srgb_inverts_srgb_to_linear() {
        for v in [0.0, 0.002, 0.04045, 0.2, 0.5, 0.73, 1.0] {
            let back = linear_to_srgb(srgb_to_linear([v, v, v, 0.4]));
            assert!(close(back[0], v), "{v} round-trips, got {}", back[0]);
            assert_eq!(back[3], 0.4, "alpha untouched");
        }
    }

    #[test]
    fn hex_constructors_are_plain_srgb_channels() {
        let expected = [16.0 / 255.0, 128.0 / 255.0, 1.0, 1.0];
        assert_eq!(rgb8([0x10, 0x80, 0xff]), expected);
        assert_eq!(hex(0x1080ff), expected);
        assert_eq!(rgba8([0x10, 0x80, 0xff], 0.25)[3], 0.25);
        assert_eq!(
            with_alpha(expected, 0.5),
            [expected[0], expected[1], 1.0, 0.5]
        );
    }

    fn to8(c: [f32; 4]) -> [u8; 3] {
        let [r, g, b, _] = to_rgba8(c);
        [r, g, b]
    }

    #[test]
    fn oklch_matches_browser_rendering_of_forge_tokens() {
        // Reference hex values for these Forge `tokens/colors.css` entries,
        // from an independent OKLab reference conversion with per-channel
        // clipping (the 2026-09-24 token audit).
        let cases: [((f32, f32, f32), u32); 6] = [
            ((0.74, 0.11, 200.0), 0x3ebfc6), // --accent
            ((0.82, 0.10, 200.0), 0x6bd8de), // --accent-key-top
            ((0.68, 0.12, 200.0), 0x00aeb5), // --accent-key-bottom (clipped)
            ((0.60, 0.18, 25.0), 0xd74745),  // --danger-ring-edge
            ((0.70, 0.14, 145.0), 0x61b565), // --ok
            ((0.62, 0.13, 25.0), 0xc8635d),  // --axis-x
        ];
        for ((l, c, h), want) in cases {
            let got = to8(oklch(l, c, h, 1.0));
            let want = to8(hex(want));
            for i in 0..3 {
                assert!(
                    (got[i] as i32 - want[i] as i32).abs() <= 1,
                    "oklch({l} {c} {h}) = {got:02x?}, want {want:02x?}"
                );
            }
        }
        assert_eq!(
            oklch(0.74, 0.11, 200.0, 0.16)[3],
            0.16,
            "alpha passes through"
        );
    }

    #[test]
    fn primary_and_secondary_hue_stops() {
        assert!(rgb_close(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]), "red");
        assert!(
            rgb_close(hsv_to_rgb(60.0, 1.0, 1.0), [1.0, 1.0, 0.0]),
            "yellow"
        );
        assert!(
            rgb_close(hsv_to_rgb(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]),
            "green"
        );
        assert!(
            rgb_close(hsv_to_rgb(180.0, 1.0, 1.0), [0.0, 1.0, 1.0]),
            "cyan"
        );
        assert!(
            rgb_close(hsv_to_rgb(240.0, 1.0, 1.0), [0.0, 0.0, 1.0]),
            "blue"
        );
        assert!(
            rgb_close(hsv_to_rgb(300.0, 1.0, 1.0), [1.0, 0.0, 1.0]),
            "magenta"
        );
    }

    #[test]
    fn saturation_zero_is_gray() {
        // s = 0 → r == g == b == v regardless of hue.
        let g = hsv_to_rgb(123.0, 0.0, 0.5);
        assert!(rgb_close(g, [0.5, 0.5, 0.5]), "gray at v=0.5");
    }

    #[test]
    fn value_zero_is_black() {
        assert!(rgb_close(hsv_to_rgb(200.0, 0.8, 0.0), [0.0, 0.0, 0.0]));
    }

    #[test]
    fn hue_wraps() {
        // 360 wraps to 0 (red); negative wraps too.
        assert!(rgb_close(hsv_to_rgb(360.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(-60.0, 1.0, 1.0), [1.0, 0.0, 1.0]));
    }

    #[test]
    fn rgb_to_hsv_known_values() {
        let (h, s, v) = rgb_to_hsv([1.0, 0.0, 0.0]);
        assert!(close(h, 0.0) && close(s, 1.0) && close(v, 1.0), "red");
        let (h, s, v) = rgb_to_hsv([0.0, 1.0, 0.0]);
        assert!(close(h, 120.0) && close(s, 1.0) && close(v, 1.0), "green");
        let (h, s, v) = rgb_to_hsv([0.0, 0.0, 1.0]);
        assert!(close(h, 240.0) && close(s, 1.0) && close(v, 1.0), "blue");
        // Gray: hue undefined → 0, saturation 0.
        let (h, s, v) = rgb_to_hsv([0.4, 0.4, 0.4]);
        assert!(close(h, 0.0) && close(s, 0.0) && close(v, 0.4), "gray");
    }

    #[test]
    fn round_trips_for_nondegenerate_colors() {
        for &(h, s, v) in &[
            (30.0, 0.7, 0.9),
            (210.0, 0.4, 0.6),
            (290.0, 1.0, 0.5),
            (95.0, 0.55, 0.8),
        ] {
            let rgb = hsv_to_rgb(h, s, v);
            let (h2, s2, v2) = rgb_to_hsv(rgb);
            assert!(close(h, h2), "h round-trip {h} != {h2}");
            assert!(close(s, s2), "s round-trip {s} != {s2}");
            assert!(close(v, v2), "v round-trip {v} != {v2}");
        }
    }

    #[test]
    fn hsva_round_trip_and_alpha_passthrough() {
        let c = Hsva::new(210.0, 0.4, 0.6, 0.33);
        let rgba = c.to_rgba();
        assert!(close(rgba[3], 0.33), "alpha preserved to rgba");
        let back = Hsva::from_rgba(rgba);
        assert!(close(back.h, 210.0) && close(back.s, 0.4) && close(back.v, 0.6));
        assert!(close(back.a, 0.33), "alpha preserved from rgba");
    }

    #[test]
    fn new_normalizes_and_clamps() {
        let c = Hsva::new(400.0, 1.5, -0.2, 2.0);
        assert!(close(c.h, 40.0), "hue wrapped into [0,360)");
        assert_eq!(c.s, 1.0);
        assert_eq!(c.v, 0.0);
        assert_eq!(c.a, 1.0);
    }
}
