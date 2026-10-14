use super::*;

/// Within about two steps of 8-bit output: the reference colours are rounded
/// to whole 8-bit codes, so the result can't be exact.
fn assert_rgb(got: [f32; 3], want: [f32; 3]) {
    for (channel, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() < 0.01,
            "channel {channel}: got {got:?}, want {want:?}"
        );
    }
}

/// An 8-bit sample as the shader reads it from an `R8Unorm` plane.
fn code(yuv: [u8; 3]) -> [f32; 3] {
    yuv.map(|c| f32::from(c) / 255.0)
}

#[test]
fn limited_range_spans_16_to_235_for_brightness() {
    for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709] {
        let convert = YuvColour {
            matrix,
            range: YuvRange::Limited,
        }
        .to_rgb();
        assert_rgb(convert.apply(code([16, 128, 128])), [0.0; 3]);
        assert_rgb(convert.apply(code([235, 128, 128])), [1.0; 3]);
        // Neutral colour planes give grey under either matrix.
        let grey = convert.apply(code([126, 128, 128]));
        assert!(
            (grey[0] - grey[1]).abs() < 1e-5 && (grey[1] - grey[2]).abs() < 1e-5,
            "{grey:?}"
        );
    }
}

#[test]
fn full_range_uses_every_code() {
    let convert = YuvColour {
        matrix: YuvMatrix::Bt601,
        range: YuvRange::Full,
    }
    .to_rgb();
    assert_rgb(convert.apply(code([0, 128, 128])), [0.0; 3]);
    assert_rgb(convert.apply(code([255, 128, 128])), [1.0; 3]);
}

#[test]
fn the_matrices_give_their_own_reds_greens_and_blues() {
    // 8-bit limited-range codes for the primaries, from the matrices'
    // published coefficients (Kr, Kb: 0.299 and 0.114 for BT.601, 0.2126 and
    // 0.0722 for BT.709).
    let bt601 = YuvColour {
        matrix: YuvMatrix::Bt601,
        range: YuvRange::Limited,
    }
    .to_rgb();
    assert_rgb(bt601.apply(code([81, 90, 240])), [1.0, 0.0, 0.0]);
    assert_rgb(bt601.apply(code([145, 54, 34])), [0.0, 1.0, 0.0]);
    assert_rgb(bt601.apply(code([41, 240, 110])), [0.0, 0.0, 1.0]);
    let bt709 = YuvColour {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Limited,
    }
    .to_rgb();
    assert_rgb(bt709.apply(code([63, 102, 240])), [1.0, 0.0, 0.0]);
    assert_rgb(bt709.apply(code([173, 42, 26])), [0.0, 1.0, 0.0]);
    assert_rgb(bt709.apply(code([32, 240, 118])), [0.0, 0.0, 1.0]);
    // The same codes mean different colours: decoding with the wrong matrix
    // is visibly off.
    let wrong = bt601.apply(code([63, 102, 240]));
    assert!((wrong[0] - 1.0).abs() > 0.05, "{wrong:?}");
}

#[test]
fn codes_past_the_range_are_clamped_to_displayable_colour() {
    let convert = YuvColour {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Limited,
    }
    .to_rgb();
    assert_rgb(convert.apply(code([0, 128, 128])), [0.0; 3]);
    assert_rgb(convert.apply(code([255, 128, 128])), [1.0; 3]);
}

#[test]
fn a_stream_that_does_not_say_gets_what_players_assume_for_its_size() {
    let hd = YuvColour {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Limited,
    };
    let sd = YuvColour {
        matrix: YuvMatrix::Bt601,
        range: YuvRange::Limited,
    };
    assert_eq!(YuvColour::assumed_for(1920, 1080), hd);
    assert_eq!(YuvColour::assumed_for(1280, 720), hd);
    // Wide but short, like a cropped HD picture.
    assert_eq!(YuvColour::assumed_for(1280, 536), hd);
    assert_eq!(YuvColour::assumed_for(720, 576), sd);
    assert_eq!(YuvColour::assumed_for(640, 480), sd);
}

#[test]
fn the_colour_planes_are_half_the_size_rounded_up() {
    assert_eq!(chroma_size(1920, 1080), (960, 540));
    assert_eq!(chroma_size(5, 3), (3, 2));
    assert_eq!(chroma_size(1, 1), (1, 1));
}
