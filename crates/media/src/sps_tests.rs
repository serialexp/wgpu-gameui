use super::*;

/// Writes an SPS bit by bit, for the cases the fixtures don't cover.
#[derive(Default)]
struct Writer {
    bits: Vec<bool>,
}

impl Writer {
    fn u(&mut self, count: u32, value: u32) -> &mut Self {
        for bit in (0..count).rev() {
            self.bits.push((value >> bit) & 1 == 1);
        }
        self
    }

    fn ue(&mut self, value: u32) -> &mut Self {
        let code = u64::from(value) + 1;
        let length = 64 - code.leading_zeros();
        for _ in 1..length {
            self.bits.push(false);
        }
        for bit in (0..length).rev() {
            self.bits.push((code >> bit) & 1 == 1);
        }
        self
    }

    fn bytes(&self) -> Vec<u8> {
        self.bits
            .chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .enumerate()
                    .fold(0u8, |byte, (i, &bit)| byte | (u8::from(bit) << (7 - i)))
            })
            .collect()
    }
}

/// A Main-profile SPS for a 1920×1080 picture, up to its VUI flag, which
/// `vui` then writes.
fn sps(vui: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::default();
    w.u(8, 77).u(8, 0).u(8, 40).ue(0); // profile, flags, level, id
    w.ue(0).ue(0).ue(0); // frame_num bits, POC type 0, POC bits
    w.ue(4).u(1, 0); // reference frames, no gaps
    w.ue(119).ue(67).u(1, 1).u(1, 1); // 120×68 macroblocks, frames only, 8×8 direct
    w.u(1, 1).ue(0).ue(0).ue(0).ue(4); // cropped to 1080 lines
    vui(&mut w);
    w.u(1, 1); // rbsp stop bit
    w.bytes()
}

#[test]
fn without_a_vui_nothing_is_said_and_the_size_decides() {
    let info = read_sps(&sps(|w| {
        w.u(1, 0);
    }))
    .expect("reads");
    assert_eq!(
        info,
        SpsInfo {
            matrix: None,
            range: None
        }
    );
    assert_eq!(info.colour(1920, 1080), YuvColour::assumed_for(1920, 1080));
}

#[test]
fn the_signal_type_gives_range_and_matrix() {
    let info = read_sps(&sps(|w| {
        w.u(1, 1); // VUI
        w.u(1, 1).u(8, 255).u(16, 1).u(16, 1); // extended sample aspect ratio
        w.u(1, 1).u(1, 0); // overscan said
        w.u(1, 1).u(3, 5).u(1, 1); // signal type: unspecified format, full range
        w.u(1, 1).u(8, 6).u(8, 6).u(8, 6); // BT.601 (525 lines)
    }))
    .expect("reads");
    assert_eq!(
        info.colour(1920, 1080),
        YuvColour {
            matrix: YuvMatrix::Bt601,
            range: YuvRange::Full
        }
    );
}

#[test]
fn a_matrix_a_sprite_does_not_have_falls_back_to_the_assumed_one() {
    let info = read_sps(&sps(|w| {
        w.u(1, 1).u(1, 0).u(1, 0);
        w.u(1, 1).u(3, 5).u(1, 0);
        w.u(1, 1).u(8, 9).u(8, 16).u(8, 9); // BT.2020
    }))
    .expect("reads");
    assert_eq!(info.range, Some(YuvRange::Limited));
    assert_eq!(info.matrix, None);
}

#[test]
fn smpte_240m_is_drawn_with_bt709() {
    let info = read_sps(&sps(|w| {
        w.u(1, 1).u(1, 0).u(1, 0);
        w.u(1, 1).u(3, 5).u(1, 0);
        w.u(1, 1).u(8, 7).u(8, 7).u(8, 7); // SMPTE 240M
    }))
    .expect("reads");
    assert_eq!(info.matrix, Some(YuvMatrix::Bt709));
}

/// A bit depth code near `u32::MAX` is refused, not added to.
#[test]
fn a_huge_bit_depth_is_unsupported() {
    let mut w = Writer::default();
    w.u(8, 100).u(8, 0).u(8, 40).ue(0);
    w.ue(1).ue(u32::MAX - 1).ue(0); // 4:2:0, then a luma depth past any real one
    w.u(1, 1);
    assert!(matches!(
        read_sps(&w.bytes()),
        Err(MediaError::Unsupported(_))
    ));
}

#[test]
fn a_cut_sps_is_damaged() {
    let whole = sps(|w| {
        w.u(1, 0);
    });
    assert!(matches!(read_sps(&whole[..4]), Err(MediaError::Damaged(_))));
}

#[test]
fn high_profile_scaling_lists_are_read_past() {
    let mut w = Writer::default();
    w.u(8, 100).u(8, 0).u(8, 40).ue(0);
    w.ue(1).ue(0).ue(0).u(1, 0); // 4:2:0, 8-bit, no bypass
    w.u(1, 1); // scaling matrices follow
    for list in 0..8 {
        if list == 2 {
            // One list given: +8 to 16, then -16 to zero, which ends it
            // (signed codes: +8 is 15, -16 is 32).
            w.u(1, 1).ue(15).ue(32);
        } else {
            w.u(1, 0);
        }
    }
    w.ue(0).ue(2).ue(1).u(1, 0); // frame_num bits, POC type 2, 1 reference, no gaps
    w.ue(10).ue(8).u(1, 1).u(1, 1).u(1, 0); // 11×9 macroblocks, frames only, no crop
    w.u(1, 1).u(1, 0).u(1, 0).u(1, 1).u(3, 5).u(1, 1).u(1, 0); // VUI: full range
    w.u(1, 1);
    let info = read_sps(&w.bytes()).expect("reads");
    assert_eq!(info.range, Some(YuvRange::Full));
}
