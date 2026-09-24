//! Fixed-size values used by the analytic box-shadow primitive.

/// Four corner radii in clockwise order, starting at the top-left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CornerRadii {
    /// Top-left radius.
    pub top_left: f32,
    /// Top-right radius.
    pub top_right: f32,
    /// Bottom-right radius.
    pub bottom_right: f32,
    /// Bottom-left radius.
    pub bottom_left: f32,
}

impl CornerRadii {
    /// Construct four independent corner radii.
    pub const fn new(top_left: f32, top_right: f32, bottom_right: f32, bottom_left: f32) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    /// Use one radius for every corner.
    pub const fn uniform(radius: f32) -> Self {
        Self::new(radius, radius, radius, radius)
    }

    /// Return the radii in top-left, top-right, bottom-right, bottom-left order.
    pub const fn as_array(self) -> [f32; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }
}

impl From<f32> for CornerRadii {
    fn from(value: f32) -> Self {
        Self::uniform(value)
    }
}

/// An authored CSS-compatible box shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    /// Local-space x/y offset.
    pub offset: [f32; 2],
    /// Authored blur radius. The renderer converts this to `sigma = blur / 2`.
    pub blur: f32,
    /// CSS spread radius.
    pub spread: f32,
    /// Straight sRGB-encoded RGBA color (see [`crate::color`]).
    pub color: [f32; 4],
    /// Whether this is an inset shadow.
    pub inset: bool,
}

impl Default for BoxShadow {
    fn default() -> Self {
        Self {
            offset: [0.0; 2],
            blur: 0.0,
            spread: 0.0,
            color: [0.0; 4],
            inset: false,
        }
    }
}

/// Fixed-size GPU record for one full-affine analytic shadow.
///
/// Geometry stays local while blur is isotropic in browser/screen space.
/// `linear` and `translation` carry the complete forward affine; `params`
/// carries the corresponding local covariance without changing the renderer's
/// fixed ten-`vec4` instance layout. Clipping remains in world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowInstance {
    /// Forward affine linear part `[a, b, c, d]`.
    pub linear: [f32; 4],
    /// `[tx, ty, clip_enabled, inset]`.
    pub translation: [f32; 4],
    /// Local quad rasterized by the vertex shader `[x, y, width, height]`.
    pub raster_rect: [f32; 4],
    /// Offset/spread-adjusted shadow source or inset hole.
    pub shadow_rect: [f32; 4],
    /// Original border box (outset) or explicit padding box (inset).
    pub element_rect: [f32; 4],
    /// Straight sRGB-encoded RGBA color, with draw-list tint applied.
    pub color: [f32; 4],
    /// Adjusted source/hole radii in clockwise order from top-left.
    pub shadow_radii: [f32; 4],
    /// Original element radii in clockwise order from top-left.
    pub element_radii: [f32; 4],
    /// World-space clip `[x, y, width, height]`.
    pub clip: [f32; 4],
    /// `[local_sigma_y, collapsed_hole, local_sigma_x_given_y, x_mean_per_y]`.
    /// These encode the pulled-back screen-isotropic Gaussian as a y marginal
    /// and an x-conditional distribution.
    pub params: [f32; 4],
}
