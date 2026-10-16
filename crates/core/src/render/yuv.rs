//! Pictures stored as YUV, the way video decoders give them.
//!
//! A decoded video frame is three planes: brightness (Y) at full size, and two
//! colour planes (U and V, also called Cb and Cr) at half the width and half
//! the height (4:2:0). A [YUV sprite](crate::UiRenderer::create_yuv_sprite)
//! keeps the planes as they are and converts to RGB in the shader, which costs
//! a few multiplications per pixel instead of a pass over the frame on the CPU.
//!
//! How the codes turn into colour depends on two settings carried in the video
//! stream ([`YuvColour`]): the conversion matrix and the range of codes used.
//! The result is sRGB-encoded, like every other colour the UI blends.

use std::borrow::Cow;

/// The conversion matrix a video was encoded with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvMatrix {
    /// ITU-R BT.601: standard-definition video.
    Bt601,
    /// ITU-R BT.709: high-definition video.
    Bt709,
}

impl YuvMatrix {
    /// The red and blue weights of brightness (`Kr`, `Kb`); green has the rest.
    fn weights(self) -> (f32, f32) {
        match self {
            YuvMatrix::Bt601 => (0.299, 0.114),
            YuvMatrix::Bt709 => (0.2126, 0.0722),
        }
    }
}

/// Which codes a video uses for black to white.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvRange {
    /// Brightness from 16 to 235, colour from 16 to 240: what video normally
    /// uses ("TV" or "studio" range).
    Limited,
    /// Every code from 0 to 255 ("PC" or "JPEG" range).
    Full,
}

/// How a YUV sprite's codes turn into colour. A video stream usually says; when
/// it doesn't, [`assumed_for`](Self::assumed_for) gives what players assume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YuvColour {
    /// The conversion matrix.
    pub matrix: YuvMatrix,
    /// The range of codes.
    pub range: YuvRange,
}

impl YuvColour {
    /// What players assume for a stream of this size that doesn't say: limited
    /// range, BT.709 for high definition (1280 wide or more, or taller than
    /// PAL's 576 lines) and BT.601 below that.
    pub fn assumed_for(width: u32, height: u32) -> Self {
        let matrix = if width >= 1280 || height > 576 {
            YuvMatrix::Bt709
        } else {
            YuvMatrix::Bt601
        };
        Self {
            matrix,
            range: YuvRange::Limited,
        }
    }

    /// The conversion from sampled codes (each `0.0..=1.0`, as read from an
    /// 8-bit plane) to colour.
    pub fn to_rgb(self) -> YuvToRgb {
        let (kr, kb) = self.matrix.weights();
        let kg = 1.0 - kr - kb;
        // Brightness and colour as `scale * sample + offset`, brightness in
        // 0..=1 and colour in -0.5..=0.5.
        let ((y_scale, y_offset), (c_scale, c_offset)) = match self.range {
            YuvRange::Limited => (
                (255.0 / 219.0, -16.0 / 219.0),
                (255.0 / 224.0, -128.0 / 224.0),
            ),
            YuvRange::Full => ((1.0, 0.0), (1.0, -128.0 / 255.0)),
        };
        // Each colour from brightness and the two colour differences.
        let rgb = [
            [1.0, 0.0, 2.0 * (1.0 - kr)],
            [
                1.0,
                -2.0 * kb * (1.0 - kb) / kg,
                -2.0 * kr * (1.0 - kr) / kg,
            ],
            [1.0, 2.0 * (1.0 - kb), 0.0],
        ];
        YuvToRgb {
            rows: rgb.map(|[y, u, v]| {
                [
                    y * y_scale,
                    u * c_scale,
                    v * c_scale,
                    y * y_offset + (u + v) * c_offset,
                ]
            }),
        }
    }
}

/// The affine conversion from sampled YUV codes to colour that the shader
/// applies: each of red, green and blue is
/// `row[0] * y + row[1] * u + row[2] * v + row[3]`, clamped to `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YuvToRgb {
    /// One row per colour: red, green, blue.
    pub rows: [[f32; 4]; 3],
}

impl YuvToRgb {
    /// The colour for one pixel's sampled codes, as the shader computes it.
    pub fn apply(&self, [y, u, v]: [f32; 3]) -> [f32; 3] {
        self.rows
            .map(|[ky, ku, kv, offset]| (ky * y + ku * u + kv * v + offset).clamp(0.0, 1.0))
    }
}

/// One of a YUV picture's planes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvPlane {
    /// Brightness, at the picture's size.
    Y,
    /// Blue difference, at half the width and height.
    U,
    /// Red difference, at half the width and height.
    V,
}

/// A 4:2:0 picture's three planes: `y` at the picture's size, `u` and `v` at
/// [`chroma_size`]. Each row of a plane starts its stride after the one before,
/// so a decoder's padded rows are taken as they are; a plane holds its rows
/// with or without the padding after the last. Pass the planes owned
/// (`Vec<u8>`, as decoders return them) to spare a copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YuvPlanes<'a> {
    /// Brightness.
    pub y: Cow<'a, [u8]>,
    /// Blue difference.
    pub u: Cow<'a, [u8]>,
    /// Red difference.
    pub v: Cow<'a, [u8]>,
    /// Bytes from the start of one row of `y` to the next: at least the
    /// picture's width.
    pub y_stride: u32,
    /// Bytes from the start of one row of `u` (and of `v`) to the next: at
    /// least the colour planes' width.
    pub chroma_stride: u32,
}

impl<'a> YuvPlanes<'a> {
    /// Planes for a picture `width` pixels wide, with no padding after each
    /// row.
    pub fn packed(
        width: u32,
        y: impl Into<Cow<'a, [u8]>>,
        u: impl Into<Cow<'a, [u8]>>,
        v: impl Into<Cow<'a, [u8]>>,
    ) -> Self {
        Self {
            y: y.into(),
            u: u.into(),
            v: v.into(),
            y_stride: width,
            chroma_stride: width.div_ceil(2),
        }
    }

    /// The planes, owning their bytes.
    pub fn into_owned(self) -> YuvPlanes<'static> {
        YuvPlanes {
            y: Cow::Owned(self.y.into_owned()),
            u: Cow::Owned(self.u.into_owned()),
            v: Cow::Owned(self.v.into_owned()),
            y_stride: self.y_stride,
            chroma_stride: self.chroma_stride,
        }
    }

    /// The planes, lending their bytes: for uploading planes kept
    /// elsewhere without copying them first.
    pub fn borrowed(&self) -> YuvPlanes<'_> {
        YuvPlanes {
            y: Cow::Borrowed(&self.y),
            u: Cow::Borrowed(&self.u),
            v: Cow::Borrowed(&self.v),
            y_stride: self.y_stride,
            chroma_stride: self.chroma_stride,
        }
    }

    /// Total bytes in the three planes.
    pub fn len(&self) -> usize {
        self.y.len() + self.u.len() + self.v.len()
    }

    /// Whether all three planes are empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The size of a 4:2:0 picture's colour planes: half its width and height,
/// rounded up so an odd last column or row still has colour.
pub fn chroma_size(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(2), height.div_ceil(2))
}

#[cfg(test)]
#[path = "yuv_tests.rs"]
mod tests;
