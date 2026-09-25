//! GPU rendering for `DrawList`.
//!
//! See [`UiRenderer`] for the entry point. Internally this module owns:
//! * a colored-quad pipeline (consumes [`crate::Vertex`] directly)
//! * a textured-quad pipeline (icons and nine-slice tessellation)
//! * a dynamic [`SpriteAtlas`]
//! * a nine-slice metadata table
//! * a [`crate::TextRenderer`] for MSDF text (cosmic-text shaping + fdsm glyph atlas)
//!
//! Per-frame GPU scratch (vertex/index/instance buffers, uniform slots) is a bump
//! arena owned by the frame, not by the pass: see [`uniform_arena`] for why passes
//! in one submission must not share bytes.
//!
//! `UiRenderer::render` consumes a `DrawList` in its paint order: every run is
//! uploaded first, then all of them are drawn in **one** render pass, switching
//! pipelines between runs. A pass per run would load and store the whole target
//! each time (costly on tile-based GPUs), and on Metal each pass holds a command
//! buffer until the submission completes — around a thousand passes in one
//! submission exhausted the queue and hung the frame.

mod atlas;
mod blur;
mod capture;
mod composite;
mod glyph_msdf;
#[cfg(feature = "phosphor-icons")]
mod icon_font;
mod image_cache;
mod msdf_atlas;
#[cfg(feature = "phosphor-icons")]
mod phosphor;
mod ui_renderer;
mod uniform_arena;

pub use glyph_msdf::{GlyphMetrics, GlyphMsdf, generate_glyph_msdf};
pub use msdf_atlas::{DEFAULT_PX_RANGE, DEFAULT_REF_PX, GlyphTile, MsdfGlyphAtlas};

#[cfg(feature = "phosphor-icons")]
pub(crate) use icon_font::icon_font_snapshot;
#[cfg(feature = "phosphor-icons")]
pub use icon_font::{
    IconFontId, IconGlyph, icon_font_data, icon_font_id, icon_glyph, register_icon_font,
};
#[cfg(feature = "phosphor-icons")]
pub use phosphor::PhosphorIcon;

pub use atlas::{AtlasRegion, SpriteAtlas, SpriteId};
pub use blur::{Backdrop, BlurParams, ColorEncoding};
#[cfg(feature = "headless")]
pub use capture::HeadlessGpu;
pub use capture::{CAPTURE_FORMAT, capture_draw_list, capture_layers, write_png};
pub use image_cache::{ImageCache, ImageEntry, ImageError};
pub(crate) use ui_renderer::ortho_matrix;
pub use ui_renderer::{NineSliceMeta, RenderStats, UiRenderer};
pub(crate) use uniform_arena::UniformArena;

pub use crate::widgets::NineSliceId;

/// Open a render pass on `view` that keeps what is already there and stores
/// the result — the only kind the UI draws with.
pub(crate) fn load_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    label: &str,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}
