//! GPU rendering for `DrawList`.
//!
//! See [`UiRenderer`] for the entry point. Internally this module owns:
//! * a colored-quad pipeline (consumes [`crate::Vertex`] directly)
//! * a textured-quad pipeline (icons and nine-slice tessellation)
//! * sprite textures: one per sprite by default, or shared [`SpriteAtlas`]es the
//!   application opts into (`textures`)
//! * a nine-slice metadata table
//! * a [`crate::TextRenderer`] for MSDF text (cosmic-text shaping + fdsm glyph atlas)
//!
//! Per-frame GPU scratch (vertex/index/instance buffers, uniform slots) is a bump
//! arena owned by the frame, not by the pass: see [`uniform_arena`] for why passes
//! in one submission must not share bytes.
//!
//! `UiRenderer::render` draws a `DrawList` in batches: primitives that can't
//! change the same pixel may be regrouped, so every non-overlapping box goes in
//! one draw call and every glyph in another, while primitives that overlap keep
//! their paint order (`docs/design/draw-call-batching.md`; `draw_box`,
//! `draw_order`, `draw_plan` and `batch_draws`). Every batch is uploaded first,
//! then all of them are drawn in **one** render pass, switching pipelines
//! between batches. A pass per batch would load and store the whole target each
//! time (costly on tile-based GPUs), and on Metal each pass holds a command
//! buffer until the submission completes — around a thousand passes in one
//! submission exhausted the queue and hung the frame.

mod atlas;
mod batch_draws;
mod blur;
mod capture;
mod composite;
mod draw_box;
mod draw_order;
mod draw_plan;
mod frame_arena;
mod glyph_msdf;
#[cfg(feature = "phosphor-icons")]
mod icon_font;
mod image_cache;
mod msdf_atlas;
#[cfg(feature = "phosphor-icons")]
mod phosphor;
mod text_contrast;
pub(crate) mod textures;
mod ui_renderer;
mod uniform_arena;
mod yuv;

pub use glyph_msdf::{GlyphMetrics, GlyphMsdf, generate_glyph_msdf};
pub use msdf_atlas::{
    DEFAULT_PX_RANGE, DEFAULT_REF_PX, GlyphSizing, GlyphTile, MAX_HINTED_PX, MsdfGlyphAtlas,
};

#[cfg(feature = "phosphor-icons")]
pub(crate) use icon_font::icon_font_snapshot;
#[cfg(feature = "phosphor-icons")]
pub use icon_font::{
    IconFontId, IconGlyph, icon_font_data, icon_font_id, icon_glyph, register_icon_font,
};
#[cfg(feature = "phosphor-icons")]
pub use phosphor::PhosphorIcon;

pub use atlas::{AtlasRegion, AtlasSlot, SpriteAtlas};
pub use blur::{Backdrop, BlurParams, ColorEncoding};
#[cfg(feature = "headless")]
pub use capture::HeadlessGpu;
pub use capture::{
    CAPTURE_FORMAT, CAPTURE_SPACE, capture_draw_list, capture_layers, capture_with, write_png,
};
pub(crate) use frame_arena::FrameArena;
pub use image_cache::{ImageCache, ImageEntry, ImageError};
pub use text_contrast::TextContrast;
pub use textures::{AtlasId, Placement, SMALL_SPRITE_EDGE, SpriteError, SpriteId};
pub(crate) use ui_renderer::ortho_matrix;
pub use ui_renderer::{NineSliceMeta, RenderStats, SMALL_TEXTURE_BATCHES_WARN, UiRenderer};
pub(crate) use uniform_arena::UniformArena;
pub use yuv::{YuvColour, YuvMatrix, YuvPlane, YuvPlanes, YuvRange, YuvToRgb, chroma_size};

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
