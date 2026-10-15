//! Reading what we need from an H.264 sequence parameter set (SPS) ourselves.
//!
//! `rust_h264` reads past the colour settings without keeping them, so the
//! conversion matrix and range are read here (`video_signal_type` in the
//! VUI, H.264 annex E.1.1). The same pass reports what the decoder can't
//! play (interlaced coding, more than 8 bits, other than 4:2:0) before any
//! picture is decoded, so the player can say so instead of failing midway.

use crate::MediaError;
use wgpu_gameui::{YuvColour, YuvMatrix, YuvRange};

/// What the SPS says about the pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpsInfo {
    /// The colour settings, when the stream gives them.
    pub matrix: Option<YuvMatrix>,
    /// The range, when the stream gives it.
    pub range: Option<YuvRange>,
}

impl SpsInfo {
    /// The colour settings to show a `width` × `height` picture with: what
    /// the stream says, and what players assume for anything it doesn't.
    pub fn colour(&self, width: u32, height: u32) -> YuvColour {
        let assumed = YuvColour::assumed_for(width, height);
        YuvColour {
            matrix: self.matrix.unwrap_or(assumed.matrix),
            range: self.range.unwrap_or(assumed.range),
        }
    }
}

/// Read an SPS's RBSP (emulation prevention already removed, NAL header
/// byte not included). Pictures the decoder can't play are
/// [`MediaError::Unsupported`].
pub(crate) fn read_sps(rbsp: &[u8]) -> Result<SpsInfo, MediaError> {
    read(&mut Bits::new(rbsp)).map_err(|error| match error {
        Error::Unsupported(what) => MediaError::Unsupported(what.into()),
        Error::End => MediaError::Damaged("the video's settings (SPS) end early".into()),
    })
}

enum Error {
    End,
    Unsupported(&'static str),
}

/// Profiles whose SPS carries the chroma format, bit depths and scaling
/// matrices (H.264 7.3.2.1.1).
const HIGH_PROFILES: [u8; 12] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134];

fn read(bits: &mut Bits) -> Result<SpsInfo, Error> {
    let profile_idc = bits.u(8)? as u8;
    bits.u(8)?; // constraint flags and reserved bits
    bits.u(8)?; // level_idc
    bits.ue()?; // seq_parameter_set_id
    if HIGH_PROFILES.contains(&profile_idc) {
        let chroma_format_idc = bits.ue()?;
        if chroma_format_idc != 1 {
            return Err(Error::Unsupported(
                "H.264 with colour other than 4:2:0; this player plays 4:2:0",
            ));
        }
        // bit_depth_luma_minus8 and bit_depth_chroma_minus8, compared as
        // read: a damaged code can be near `u32::MAX`.
        let luma_extra = bits.ue()?;
        let chroma_extra = bits.ue()?;
        if luma_extra != 0 || chroma_extra != 0 {
            return Err(Error::Unsupported(
                "H.264 with more than 8 bits a sample (10-bit or HDR); this player plays 8-bit",
            ));
        }
        bits.u(1)?; // qpprime_y_zero_transform_bypass_flag
        if bits.flag()? {
            // seq_scaling_matrix_present_flag: eight lists for 4:2:0.
            for list in 0..8 {
                if bits.flag()? {
                    skip_scaling_list(bits, if list < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    bits.ue()?; // log2_max_frame_num_minus4
    match bits.ue()? {
        0 => {
            bits.ue()?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            bits.u(1)?; // delta_pic_order_always_zero_flag
            bits.se()?; // offset_for_non_ref_pic
            bits.se()?; // offset_for_top_to_bottom_field
            for _ in 0..bits.ue()? {
                bits.se()?; // offset_for_ref_frame
            }
        }
        _ => {}
    }
    bits.ue()?; // max_num_ref_frames
    bits.u(1)?; // gaps_in_frame_num_value_allowed_flag
    bits.ue()?; // pic_width_in_mbs_minus1
    bits.ue()?; // pic_height_in_map_units_minus1
    if !bits.flag()? {
        // frame_mbs_only_flag clear: fields, or frames that mix fields in.
        return Err(Error::Unsupported(
            "interlaced H.264; this player plays progressive video",
        ));
    }
    bits.u(1)?; // direct_8x8_inference_flag
    if bits.flag()? {
        // frame_cropping_flag
        for _ in 0..4 {
            bits.ue()?;
        }
    }
    let mut info = SpsInfo {
        matrix: None,
        range: None,
    };
    if !bits.flag()? {
        // No VUI: nothing said about colour.
        return Ok(info);
    }
    if bits.flag()? {
        // aspect_ratio_info_present_flag
        if bits.u(8)? == 255 {
            bits.u(16)?; // sar_width
            bits.u(16)?; // sar_height
        }
    }
    if bits.flag()? {
        bits.u(1)?; // overscan_appropriate_flag
    }
    if bits.flag()? {
        // video_signal_type_present_flag
        bits.u(3)?; // video_format
        info.range = Some(if bits.flag()? {
            YuvRange::Full
        } else {
            YuvRange::Limited
        });
        if bits.flag()? {
            // colour_description_present_flag
            bits.u(8)?; // colour_primaries
            bits.u(8)?; // transfer_characteristics
            info.matrix = matrix(bits.u(8)? as u8);
        }
    }
    Ok(info)
}

/// The conversion matrix for a `matrix_coefficients` code (H.264 table
/// E-5), or `None` for "unspecified" and the ones a YUV sprite doesn't
/// have, which get the assumed matrix instead.
fn matrix(code: u8) -> Option<YuvMatrix> {
    match code {
        // BT.709, and SMPTE 240M, whose weights (Kr 0.212, Kb 0.087) are
        // close enough to BT.709's (0.2126, 0.0722) for a picture on screen.
        1 | 7 => Some(YuvMatrix::Bt709),
        // BT.470 System B/G and BT.601 (625 and 525 lines).
        5 | 6 => Some(YuvMatrix::Bt601),
        _ => None,
    }
}

/// Skip a `scaling_list` of `size` entries (H.264 7.3.2.1.1.1).
fn skip_scaling_list(bits: &mut Bits, size: usize) -> Result<(), Error> {
    let mut last = 8i64;
    let mut next = 8i64;
    for _ in 0..size {
        if next != 0 {
            next = (last + bits.se()? + 256) % 256;
        }
        if next != 0 {
            last = next;
        }
    }
    Ok(())
}

/// Reads bits most significant first, and H.264's Exp-Golomb codes.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn bit(&mut self) -> Result<u32, Error> {
        let byte = *self.bytes.get(self.at / 8).ok_or(Error::End)?;
        let bit = (byte >> (7 - self.at % 8)) & 1;
        self.at += 1;
        Ok(u32::from(bit))
    }

    fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.bit()? == 1)
    }

    /// `count` bits (at most 32) as a number.
    fn u(&mut self, count: u32) -> Result<u32, Error> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | self.bit()?;
        }
        Ok(value)
    }

    /// An unsigned Exp-Golomb code.
    fn ue(&mut self) -> Result<u32, Error> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(Error::End);
            }
        }
        Ok(((1u64 << zeros) - 1 + u64::from(self.u(zeros)?)) as u32)
    }

    /// A signed Exp-Golomb code.
    fn se(&mut self) -> Result<i64, Error> {
        let code = i64::from(self.ue()?);
        Ok(if code % 2 == 1 {
            (code + 1) / 2
        } else {
            -(code / 2)
        })
    }
}

#[cfg(test)]
#[path = "sps_tests.rs"]
mod tests;
