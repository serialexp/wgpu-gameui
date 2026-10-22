//! `UiRenderer` — single-call renderer consuming a `DrawList`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::TextRenderer;
use crate::color::{Clear, ColorSpace};
use crate::layer::LayerStack;
use crate::layout::Rect;
use crate::render::batch_draws::{BatchDraws, DrawKind};
use crate::render::blur::{Backdrop, Blur, BlurParams};
use crate::render::composite::{Offscreen, TargetPlan};
use crate::render::draw_box::Target;
use crate::render::draw_plan::{DrawPlan, ResolveTextures};
use crate::render::frame_arena::FrameArena;
use crate::render::image_cache::{ImageCache, ImageEntry, ImageError, decode_rgba8};
use crate::render::textures::{
    self, AtlasId, Binding, Placement, Resolved, Sampling, SpriteError, SpriteId, Textures,
};
use crate::render::uniform_arena::UniformArena;
use crate::render::yuv::{YuvColour, YuvPlanes};
use crate::text::{FontSystemHandle, MsdfAtlas, PreparedMsdf};
use crate::widgets::{
    AnalyticInstance, CircleInstance, DrawList, IconDraw, NineSliceDraw, NineSliceId, PaintCmd,
    Vertex,
};

const SHADER: &str = include_str!("ui.wgsl");

/// Metadata describing a registered nine-slice resource.
#[derive(Clone, Debug)]
pub struct NineSliceMeta {
    /// Sprite the nine-slice samples from.
    pub sprite: SpriteId,
    /// Border insets in source pixels: [left, top, right, bottom].
    pub border: [u32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    /// `[origin_x, origin_y, logical px per physical px, 0]`: maps a fragment's
    /// position on the target back to logical space exactly (stroke segments
    /// need the same pixel position in every record that touches it).
    view: [f32; 4],
}

/// Size of one ortho uniform, and therefore of one dynamic-offset slot.
const UNIFORM_SIZE: u64 = std::mem::size_of::<Uniforms>() as u64;

/// Per-frame GPU arena bytes one submission may consume before the renderer
/// assumes its frame boundary was missed (see [`UiRenderer::begin_frame`]). A
/// renderer that crosses this limit refuses later render calls until the caller
/// opens a new frame, rather than geometrically growing an arena into wgpu's hard
/// maximum-buffer validation failure.
const ARENA_SANITY_CAP: u64 = 64 * 1024 * 1024;

fn frame_arena_within_cap(bytes: u64) -> bool {
    bytes <= ARENA_SANITY_CAP
}

/// Draw calls of small sprites in textures of their own (see
/// [`RenderStats::small_texture_batches`]) in one frame from which the frame is
/// reported: each is a texture switch an atlas would have saved.
pub const SMALL_TEXTURE_BATCHES_WARN: usize = 32;

/// Names of the small sprites in textures of their own kept per frame, to say
/// which sprites belong in an atlas.
const SMALL_TEXTURE_NAMES: usize = 3;

/// Where one list's batches were uploaded this frame: the byte offsets of each
/// family's records in its buffer, and its text and icon vertices.
struct BatchUpload {
    vertex_offset: u64,
    index_offset: u64,
    analytic_offset: u64,
    circle_offset: u64,
    sprite_offset: u64,
    nine_offset: u64,
    text: Option<PreparedMsdf>,
    #[cfg(feature = "phosphor-icons")]
    icons: Option<PreparedMsdf>,
}

/// What has been warned about, so each is said once.
#[derive(Default)]
struct Warned {
    /// Sprite names drawn but never loaded.
    sprites: HashSet<String>,
    /// Nine-slice names drawn but never registered.
    nine_slices: HashSet<String>,
    /// Nine-slices backed by a YUV sprite.
    yuv_nine_slices: HashSet<NineSliceId>,
}

/// Sprites and nine-slices resolved to the textures they draw from, warning
/// once about each that isn't there. Borrows the renderer's fields rather than
/// the renderer, so a plan can be built alongside it.
struct Resolver<'a> {
    textures: &'a Textures,
    nine_slices: &'a [NineSliceMeta],
    nine_slice_names: &'a HashMap<String, NineSliceId>,
    warned: &'a RefCell<Warned>,
}

impl<'a> Resolver<'a> {
    fn new(
        textures: &'a Textures,
        nine_slices: &'a [NineSliceMeta],
        nine_slice_names: &'a HashMap<String, NineSliceId>,
        warned: &'a RefCell<Warned>,
    ) -> Self {
        Self {
            textures,
            nine_slices,
            nine_slice_names,
            warned,
        }
    }
}

impl ResolveTextures for Resolver<'_> {
    fn sprite(&self, draw: &IconDraw) -> Option<Resolved> {
        let id = match draw.sprite {
            Some(id) => id,
            None => match self.textures.id_for(&draw.icon_key) {
                Some(id) => id,
                None => {
                    let warned = &mut self.warned.borrow_mut().sprites;
                    if !warned.contains(draw.icon_key.as_str()) {
                        warned.insert(draw.icon_key.clone());
                        log::warn!(
                            "wgpu-gameui: sprite '{}' referenced but not registered — draw skipped",
                            draw.icon_key
                        );
                    }
                    return None;
                }
            },
        };
        self.textures.resolve(id)
    }

    fn nine_slice(&self, draw: &NineSliceDraw) -> Option<(Resolved, [u32; 4])> {
        let id = match draw.nine_slice {
            Some(id) => id,
            None => match self.nine_slice_names.get(&draw.texture_key) {
                Some(id) => *id,
                None => {
                    let warned = &mut self.warned.borrow_mut().nine_slices;
                    if !warned.contains(draw.texture_key.as_str()) {
                        warned.insert(draw.texture_key.clone());
                        log::warn!(
                            "wgpu-gameui: nine-slice '{}' referenced but not registered — draw skipped",
                            draw.texture_key
                        );
                    }
                    return None;
                }
            },
        };
        let meta = self.nine_slices.get(id as usize)?;
        let at = self.textures.resolve(meta.sprite)?;
        if matches!(at.binding, Binding::Yuv(_)) {
            // The nine-slice pipeline samples one RGBA texture.
            if self.warned.borrow_mut().yuv_nine_slices.insert(id) {
                log::warn!(
                    "wgpu-gameui: nine-slice {id} uses a YUV sprite, which only draws as an image — draw skipped"
                );
            }
            return None;
        }
        Some((at, meta.border))
    }
}

/// Which textured-instance pipeline an [`InstancedRun`] draws with (the path
/// for lists without a paint stream).
#[derive(Clone, Copy)]
enum InstancedKind {
    Icon,
    NineSlice,
    /// An icon instance drawn from a YUV sprite.
    Yuv,
}

/// Uploaded icon or nine-slice instances: `count` records in `buffer` from
/// `offset`, sampling the sprite texture bound by `texture` (both captured at
/// upload, so a buffer that grows later in the frame doesn't disturb them).
struct InstancedRun {
    kind: InstancedKind,
    buffer: wgpu::Buffer,
    offset: u64,
    count: u32,
    texture: wgpu::BindGroup,
}

/// Per-instance icon/image record — matches `vs_icon` in `ui.wgsl`.
///
/// Icons, sprites, and cropped images all flow through this path. The four
/// world-space corners are baked in (the `DrawList` already applied the active
/// transform), so the vertex shader bilinearly interpolates them — rotation,
/// scale, and shear are handled with no fallback. Replaces re-tessellating 6
/// verts/icon into the textured soup + re-uploading it every frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct IconInstance {
    /// `[tl.x, tl.y, tr.x, tr.y]` (world space).
    c_tl_tr: [f32; 4],
    /// `[br.x, br.y, bl.x, bl.y]` (world space).
    c_br_bl: [f32; 4],
    /// Source UV rect `[u0, v0, u1, v1]`.
    uv_rect: [f32; 4],
    /// Tint (multiplied with the sampled texel).
    tint: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `flags[0] > 0.5`).
    clip: [f32; 4],
    /// `[clip_enabled, tile_wrap, tile_span_u, tile_span_v]`: `flags[1] > 0.5`
    /// makes the fragment shader repeat the source region modulo **one tile**
    /// (`flags[2..]` = the single-tile UV span in atlas units — the instance's
    /// `uv_rect` spans `tile_count` tiles after `apply_crop_uv`, so its own
    /// size can't serve as the wrap modulus). See [`DrawList::image_tiled`].
    flags: [f32; 4],
}

/// Per-instance icon attributes — matches [`IconInstance`] / `vs_icon`
/// (location 0 is the base-mesh corner).
const ICON_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    1 => Float32x4, // c_tl_tr
    2 => Float32x4, // c_br_bl
    3 => Float32x4, // uv_rect
    4 => Float32x4, // tint
    5 => Float32x4, // clip
    6 => Float32x4, // flags
];

const COLOR_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32,
];

/// Base unit-quad vertex (one attribute: the corner in `[0,1]²`), shared by
/// every instanced pipeline, the MSDF text one included.
pub(crate) const QUAD_ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,
];

/// Ten vec4 payloads plus a flat integer kind; location 0 is the base corner.
const ANALYTIC_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 11] = wgpu::vertex_attr_array![
    1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
    6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4,
    11 => Uint32,
];

/// Per-instance circle attributes — matches [`CircleInstance`] / `vs_circle`
/// (location 0 is the base-mesh corner).
const CIRCLE_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    1 => Float32x4, // center (cx, cy, radius, thickness)
    2 => Float32x4, // color
    3 => Float32x4, // clip
    4 => Float32x4, // params (clip_enabled, _, _, _)
];

/// Unit-quad corners (TL, TR, BR, BL) for the instanced pipelines' base mesh.
pub(crate) const QUAD_VERTS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
/// Two triangles for the unit quad above.
pub(crate) const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// Per-instance nine-slice record — matches `vs_nine_slice` in `ui.wgsl`.
///
/// Unlike [`ChromeInstance`] (built in the `DrawList`), this is built in the
/// renderer because it needs the registered nine-slice's atlas region + border,
/// which the `DrawList` doesn't know. The full affine is baked in (`lin` +
/// `translate` linear/translation parts) and applied to the local corner in the
/// vertex shader, so rotated/scaled nine-slices need no fallback — every panel,
/// transformed or not, is one instance. Replaces re-tessellating 54 verts/panel
/// into the textured soup each frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct NineSliceInstance {
    /// Affine linear part `[a, b, c, d]` (row-major, from [`Affine2`]).
    lin: [f32; 4],
    /// `[tx, ty, clip_enabled, _pad]`.
    translate: [f32; 4],
    /// Local-space `[x, y, w, h]` (pre-transform).
    origin_size: [f32; 4],
    /// Outer UV edges `[u0, v0, u3, v3]`.
    uv_outer: [f32; 4],
    /// Inner border-seam UVs `[u1, v1, u2, v2]`.
    uv_inner: [f32; 4],
    /// Border widths in screen px `[left, top, right, bottom]`.
    border: [f32; 4],
    /// Tint (multiplied with the sampled texel).
    tint: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `translate[2] > 0.5`).
    clip: [f32; 4],
}

/// Per-instance nine-slice attributes — matches [`NineSliceInstance`] /
/// `vs_nine_slice` (location 0 is the base-mesh corner).
const NINE_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    1 => Float32x4, // lin
    2 => Float32x4, // translate
    3 => Float32x4, // origin_size
    4 => Float32x4, // uv_outer
    5 => Float32x4, // uv_inner
    6 => Float32x4, // border
    7 => Float32x4, // tint
    8 => Float32x4, // clip
];

/// Detects the "freshly-constructed `DrawList`/`LayerStack` every frame" footgun.
///
/// The text caches — shaped layouts and font metrics — live on the font
/// system a list shares ([`SharedFontSystem`](crate::SharedFontSystem)), so a
/// list made with `with_font_system` every frame only reallocates its buffers.
/// But `DrawList::new()` and `LayerStack::new()` build a font system of their
/// own, loading every system font, and a list built through them every frame
/// — e.g. `LayerStack::new()` inside the render loop — re-shapes every label on
/// every frame. That is invisible in correctness terms but can cost
/// *milliseconds* (it made one HUD cost ~11ms a frame instead of <1ms). The
/// fix is always the same: build one list/stack once and reuse it (`clear()`
/// resets geometry while keeping its buffers and font system).
///
/// We can't see the cache directly from the renderer, but we can see identity:
/// each [`DrawList`] carries a unique, never-reused [`id`](DrawList::id). A reused
/// list reports the **same** id every frame; a per-frame-fresh list reports a
/// **brand-new** id every frame. So: remember the handful of ids seen recently,
/// and if the renderer is fed an id it has *never* seen for many consecutive
/// frames, the caller is rebuilding every frame — warn (once).
///
/// Patterns this deliberately does **not** flag: a single reused list (same id
/// forever), a small set of double-buffered/persistent stacks cycled round-robin
/// (their ids keep recurring), and the occasional rebuild on resize (a single
/// fresh id now and then never builds a long streak).
#[derive(Default)]
struct StaleListDetector {
    /// Bounded ring of recently observed `DrawList` ids (most-recent last). Large
    /// enough to cover any realistic set of persistent/double-buffered stacks.
    recent: VecDeque<u64>,
    /// Consecutive frames whose id had never been seen before.
    fresh_streak: u32,
    /// Latch: warn at most once per renderer (the advice is the same every time).
    warned: bool,
}

impl StaleListDetector {
    /// How many distinct recent ids to remember. Comfortably exceeds any sane
    /// number of persistent stacks an app cycles between.
    const RECENT_CAP: usize = 16;
    /// Consecutive all-fresh frames before we conclude the caller rebuilds every
    /// frame. ~2s at 60fps — long enough that brief transients never trip it.
    const WARN_AFTER: u32 = 120;

    /// Record one rendered list id. Returns `true` exactly once — on the frame the
    /// footgun is first detected — so the caller can emit the warning.
    fn observe(&mut self, id: u64) -> bool {
        if self.recent.contains(&id) {
            // Seen before → the list is being reused. Reset the streak.
            self.fresh_streak = 0;
        } else {
            self.fresh_streak = self.fresh_streak.saturating_add(1);
            self.recent.push_back(id);
            if self.recent.len() > Self::RECENT_CAP {
                self.recent.pop_front();
            }
        }
        if !self.warned && self.fresh_streak >= Self::WARN_AFTER {
            self.warned = true;
            return true;
        }
        false
    }
}

/// Cheap counters describing the GPU work encoded by one UI render call.
///
/// Non-exhaustive so counters can be added: start from
/// [`RenderStats::default`] and set the fields wanted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderStats {
    /// Draw lists rendered, including layers.
    pub draw_lists: usize,
    /// Total submitted primitives.
    pub primitives: usize,
    /// Paint commands recorded: each a run of one family in paint order, as
    /// the list recorded them.
    pub paint_runs: usize,
    /// GPU draw calls encoded.
    pub draw_calls: usize,
    /// Batches the paint streams were drawn in: draws grouped by what they
    /// overlap (`docs/design/draw-call-batching.md`), each one draw call
    /// unless it had nothing to draw.
    pub batches: usize,
    /// Units that were ordered: those with a pixel on the target that their
    /// clip keeps. [`primitives`](Self::primitives) counts everything recorded.
    pub units_ordered: usize,
    /// Overlap tests ordering the units took.
    pub order_overlap_tests: u64,
    /// Grid cells ordering the units visited.
    pub order_cell_visits: u64,
    /// Soup/chrome/circle/shadow batches.
    pub color_runs: usize,
    /// Analytic shadow instances submitted.
    pub shadow_instances: usize,
    /// Analytic stripe instances submitted (hatching, dashed lines).
    pub stripe_instances: usize,
    /// Analytic stroke segment instances submitted (lines and polylines).
    pub segment_instances: usize,
    /// Render passes opened: one per draw list with a paint stream, and on
    /// lists filled through the public arrays without one, one per family
    /// drawn (per texture for sprites and nine-slices).
    pub render_passes: usize,
    /// Text batches.
    pub text_runs: usize,
    /// Atlas-image, nine-slice, and vector-icon batches.
    pub icon_runs: usize,
    /// Queue buffer-write operations.
    pub buffer_write_calls: usize,
    /// Bytes written to dynamic GPU buffers.
    pub buffer_bytes_uploaded: u64,
    /// Full or partial atlas texture uploads.
    pub atlas_uploads: usize,
    /// Approximate bytes uploaded to atlas textures.
    pub atlas_bytes_uploaded: u64,
    /// Sprites uploaded into textures of their own.
    pub texture_uploads: usize,
    /// Bytes uploaded into sprites' own textures.
    pub texture_bytes_uploaded: u64,
    /// Textures created for sprites of their own: one at each new sprite's
    /// first upload. An update in place writes into the texture it has.
    pub textures_created: usize,
    /// Draw calls of small sprites (see [`SMALL_SPRITE_EDGE`](crate::render::SMALL_SPRITE_EDGE))
    /// in textures of their own: each costs a texture switch that putting the
    /// sprites in a shared atlas ([`UiRenderer::create_atlas`]) would save.
    pub small_texture_batches: usize,
    /// Dynamic buffers replaced to increase capacity: vertex, index and
    /// instance buffers, and the renderer's and the text pipeline's uniform
    /// arenas.
    pub buffer_reallocations: usize,
}

impl RenderStats {
    /// Draw calls per primitive; values near one mean almost every primitive
    /// took a draw call of its own, rather than simply a large UI.
    pub fn fragmentation_ratio(self) -> f32 {
        self.draw_calls as f32 / self.primitives.max(1) as f32
    }

    /// Human-readable performance findings suitable for logs and debug reports.
    pub fn warnings(self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.draw_calls >= 128 && self.fragmentation_ratio() >= 0.75 {
            warnings.push(format!(
                "{} draw calls for {} primitives ({:.2} per primitive); primitives of different families overlap so often that few can share a draw call",
                self.draw_calls,
                self.primitives,
                self.fragmentation_ratio()
            ));
        }
        if self.buffer_reallocations > 0 {
            warnings.push(format!(
                "{} dynamic GPU buffer reallocation(s); capacity is warming or the frame exceeded its previous high-water mark",
                self.buffer_reallocations
            ));
        }
        if self.atlas_uploads > 1 {
            warnings.push(format!(
                "{} atlas uploads in one frame; batch resource/glyph registration before rendering where possible",
                self.atlas_uploads
            ));
        }
        if self.small_texture_batches >= SMALL_TEXTURE_BATCHES_WARN {
            warnings.push(format!(
                "{} draw calls of small sprites in textures of their own; each is a texture switch, so load sprites drawn together into a shared atlas (UiRenderer::create_atlas, Placement::Atlas)",
                self.small_texture_batches
            ));
        }
        warnings
    }
}

#[derive(Default)]
struct RenderPressureDetector {
    streak: u32,
    warned: bool,
}

impl RenderPressureDetector {
    const WARN_AFTER: u32 = 120;

    fn observe(&mut self, stats: RenderStats) -> bool {
        if stats.warnings().is_empty() {
            self.streak = 0;
        } else {
            self.streak = self.streak.saturating_add(1);
        }
        if !self.warned && self.streak >= Self::WARN_AFTER {
            self.warned = true;
            true
        } else {
            false
        }
    }
}

/// Public renderer.
///
/// # Colour and target formats
///
/// UI colours are sRGB-encoded and blended in sRGB space, like a browser (see
/// [`crate::color`]). How the UI reaches the target depends on the `format`
/// passed to [`new`](Self::new):
///
/// * **Non-sRGB 8/10-bit targets** (`Bgra8Unorm`, `Rgba8Unorm`, …): the UI
///   draws straight into the target. Everything — including translucency over
///   your own scene — blends exactly like the browser. Prefer this when you can
///   pick the surface format.
/// * **`*Srgb` and float targets** (which store linear light): the UI draws into
///   an internal offscreen layer, then composites it onto the target in one
///   extra full-target pass per [`render`](Self::render) /
///   [`render_layers`](Self::render_layers) call. UI-on-UI blending still
///   matches the browser; only translucent UI over your own scene mixes in
///   linear light.
///
/// Either way, clear the target with [`clear_color`](Self::clear_color) so the
/// background matches the theme.
pub struct UiRenderer {
    // Pipelines
    color_pipeline: wgpu::RenderPipeline,
    icon_pipeline: wgpu::RenderPipeline,
    yuv_pipeline: wgpu::RenderPipeline,
    analytic_pipeline: wgpu::RenderPipeline,
    circle_pipeline: wgpu::RenderPipeline,
    nine_slice_pipeline: wgpu::RenderPipeline,

    // Uniforms (shared by both pipelines via group(0)). One arena slot per pass:
    // the projection is per pass (two passes can target differently sized views),
    // and passes sharing a slot would all read the last matrix written. Reset by
    // `begin_frame`; see `render::uniform_arena`.
    uniform: UniformArena,
    /// The slot the pass being recorded binds. Set by `prepare_pass`.
    pass_uniform_offset: u32,
    /// Logical canvas point drawn at the target's top-left corner. See
    /// [`set_view_origin`](Self::set_view_origin).
    view_origin: [f32; 2],
    /// Once an arena crosses the submission cap, refuse further rendering until
    /// `begin_frame` resets all cursors. This turns a missed frame boundary into
    /// one actionable log rather than a fatal oversized-buffer allocation.
    frame_boundary_required: bool,

    // Sprite textures: one per sprite, or shared atlases (see `render::textures`).
    textures: Textures,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    yuv_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,

    // Nine-slice registry
    nine_slices: Vec<NineSliceMeta>,
    nine_slice_names: HashMap<String, NineSliceId>,

    // Decoded-image cache (path/key -> sprite + dimensions)
    image_cache: ImageCache,

    // Per-frame scratch, each a bump arena (see `render::frame_arena`) reset by
    // `begin_frame`: soup vertices and indices, and the instances of the
    // families drawn on the unit quad below.
    color_vbo: FrameArena,
    color_ibo: FrameArena,
    icon_insts: FrameArena,
    analytic_insts: FrameArena,
    circle_insts: FrameArena,
    nine_insts: FrameArena,
    // Buffers the arenas replaced as they grew, kept alive for the renderer's
    // lifetime: wgpu work can outlive the frame it was submitted in, and
    // geometric growth keeps this set small without backend-specific
    // completion polling.
    retired_buffers: Vec<wgpu::Buffer>,

    // The unit quad every instanced family (chrome and shadows, circles,
    // icons, nine-slices) draws its instances on.
    chrome_base_vbo: wgpu::Buffer,
    chrome_base_ibo: wgpu::Buffer,

    // Host target format (the lazily-built blur pipeline draws into it) and how
    // the UI reaches it: directly, or through the offscreen layer.
    host_format: wgpu::TextureFormat,
    plan: TargetPlan,
    offscreen: Option<Offscreen>,
    // Backdrop blur — built on first `blur_backdrop` call so `new` is unchanged.
    blur: Option<Blur>,

    // Text
    text_renderer: TextRenderer,

    // What has been warned about, so a missing sprite referenced every frame
    // is reported once. A RefCell because textures are resolved through `&self`.
    warned: RefCell<Warned>,

    // Batching (`docs/design/draw-call-batching.md`): the target the pass draws
    // onto, the plan of the list being drawn, and its records in draw order.
    // Reused from list to list.
    pass_target: Target,
    draw_plan: DrawPlan,
    batch_draws: BatchDraws,
    /// Draw in paint order, as before batching. See
    /// [`set_paint_order`](Self::set_paint_order).
    #[cfg(feature = "paint-order-oracle")]
    paint_order: bool,

    // Detects callers that rebuild their DrawList/LayerStack every frame (cold
    // text-measure cache → per-frame reshaping). Fed the rendered list's id in
    // `render`/`render_layers`; warns once if it's always a brand-new list.
    stale_list: StaleListDetector,
    pressure: RenderPressureDetector,
    frame_stats: RenderStats,
    /// A few of the small sprites drawn from textures of their own this frame,
    /// named when the frame is reported for them.
    small_texture_names: Vec<String>,
}

impl UiRenderer {
    /// Build the renderer: compiles the UI shader, creates the pipelines,
    /// atlases, and bind groups, and wires up the text sub-renderer.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: FontSystemHandle,
    ) -> Self {
        let plan = TargetPlan::for_host(format);
        let work_format = plan.work_format;
        let offscreen = plan
            .offscreen
            .then(|| Offscreen::new(device, work_format, format));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        // Uniforms (bind group 0), one slot per pass — see `uniform_arena` for why.
        let uniform = UniformArena::new(
            device,
            "ui uniform buffer",
            UNIFORM_SIZE,
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        );

        let uniform_bgl = uniform.layout();

        // Sprite textures are created as sprites load; how they are sampled, now.
        let (sampler, texture_bgl) = create_sprite_sampling(device);
        let yuv_bgl = create_yuv_layout(device);
        let textures = Textures::new(device.limits().max_texture_dimension_2d);

        // Color pipeline (no texture binding)
        let color_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui color pipeline layout"),
                bind_group_layouts: &[uniform_bgl],
                push_constant_ranges: &[],
            });

        let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui color pipeline"),
            layout: Some(&color_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_color"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &COLOR_VERTEX_ATTRIBS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_color"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: work_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Icon (instanced textured-quad) pipeline. Shares the ortho uniform
        // (group 0) AND the atlas texture (group 1). Two vertex buffers: the
        // chrome unit-quad base mesh + per-instance records. The YUV pipeline
        // draws the same instances from a YUV sprite's planes instead.
        let icon_pipeline = create_icon_pipeline(
            device,
            &shader,
            work_format,
            &[uniform_bgl, &texture_bgl],
            "fs_icon",
            "ui icon pipeline",
        );
        let yuv_pipeline = create_icon_pipeline(
            device,
            &shader,
            work_format,
            &[uniform_bgl, &yuv_bgl],
            "fs_yuv",
            "ui yuv pipeline",
        );

        // Chrome (instanced SDF rounded-rect) pipeline. Shares the ortho uniform
        // (group 0); no texture. Two vertex buffers: unit-quad base + instances.
        let chrome_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui chrome pipeline layout"),
                bind_group_layouts: &[uniform_bgl],
                push_constant_ranges: &[],
            });

        let analytic_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui analytic chrome/shadow pipeline"),
            layout: Some(&chrome_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_analytic"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &QUAD_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<AnalyticInstance>()
                            as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ANALYTIC_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_analytic"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: work_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Circle (instanced SDF disc/ring) pipeline. Same shape as chrome —
        // ortho uniform (group 0), no texture, unit-quad base + instances — but
        // its fragment computes a circle SDF instead of a rounded-rect one.
        let circle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui circle pipeline"),
            layout: Some(&chrome_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_circle"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &QUAD_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<CircleInstance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &CIRCLE_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_circle"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: work_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Nine-slice (instanced) pipeline. Shares the ortho uniform (group 0)
        // AND the atlas texture (group 1, like the textured path). Two vertex
        // buffers: the chrome unit-quad base mesh + per-instance records.
        let nine_slice_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui nine-slice pipeline layout"),
                bind_group_layouts: &[uniform_bgl, &texture_bgl],
                push_constant_ranges: &[],
            });

        let nine_slice_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui nine-slice pipeline"),
            layout: Some(&nine_slice_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_nine_slice"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &QUAD_ATTRIBS,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<NineSliceInstance>()
                            as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &NINE_INSTANCE_ATTRIBS,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_nine_slice"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: work_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let chrome_base_vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui chrome base vbo"),
            contents: bytemuck::cast_slice(&QUAD_VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let chrome_base_ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui chrome base ibo"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Per-frame scratch, sized for a typical frame; it grows on demand.
        let arena =
            |label, usage, capacity: usize| FrameArena::new(device, label, usage, capacity as u64);
        let vertex = wgpu::BufferUsages::VERTEX;
        let color_vbo = arena("ui color vbo", vertex, 4096 * std::mem::size_of::<Vertex>());
        let color_ibo = arena(
            "ui color ibo",
            wgpu::BufferUsages::INDEX,
            8192 * std::mem::size_of::<u32>(),
        );
        let icon_insts = arena(
            "ui icon inst buffer",
            vertex,
            1024 * std::mem::size_of::<IconInstance>(),
        );
        let analytic_insts = arena(
            "ui analytic inst buffer",
            vertex,
            1024 * std::mem::size_of::<AnalyticInstance>(),
        );
        let nine_insts = arena(
            "ui nine-slice inst buffer",
            vertex,
            256 * std::mem::size_of::<NineSliceInstance>(),
        );
        let circle_insts = arena(
            "ui circle inst buffer",
            vertex,
            1024 * std::mem::size_of::<CircleInstance>(),
        );

        // Glyphs are generated lazily from the first frame's actual working set.
        // Eagerly rasterizing every ASCII and Phosphor glyph here made renderer
        // construction needlessly CPU-heavy for small or usually-hidden UIs.
        let text_renderer = TextRenderer::with_font_system(device, queue, work_format, font_system);

        Self {
            color_pipeline,
            icon_pipeline,
            yuv_pipeline,
            analytic_pipeline,
            circle_pipeline,
            nine_slice_pipeline,
            uniform,
            pass_uniform_offset: 0,
            view_origin: [0.0, 0.0],
            frame_boundary_required: false,
            textures,
            texture_bind_group_layout: texture_bgl,
            yuv_bind_group_layout: yuv_bgl,
            sampler,
            nine_slices: Vec::new(),
            nine_slice_names: HashMap::new(),
            image_cache: ImageCache::new(),
            color_vbo,
            color_ibo,
            icon_insts,
            analytic_insts,
            circle_insts,
            nine_insts,
            retired_buffers: Vec::new(),
            chrome_base_vbo,
            chrome_base_ibo,
            host_format: format,
            plan,
            offscreen,
            blur: None,
            text_renderer,
            warned: RefCell::new(Warned::default()),
            pass_target: Target::new((0, 0), 1.0, [0.0, 0.0]),
            draw_plan: DrawPlan::new(),
            batch_draws: BatchDraws::default(),
            #[cfg(feature = "paint-order-oracle")]
            paint_order: false,
            stale_list: StaleListDetector::default(),
            pressure: RenderPressureDetector::default(),
            frame_stats: RenderStats::default(),
            small_texture_names: Vec::new(),
        }
    }

    /// Create a shared atlas that may grow to `max_size` pixels on each side
    /// (capped at [`max_texture_size`](Self::max_texture_size)). Sprites loaded
    /// with [`Placement::Atlas`] naming it are packed into one texture, so
    /// draws of them can share a draw call wherever paint order allows: use one
    /// for many small sprites drawn together (icons, thumbnails). No GPU memory is used until
    /// a sprite goes in; a full atlas refuses further sprites
    /// ([`SpriteError::AtlasFull`]) rather than growing past `max_size`.
    pub fn create_atlas(&mut self, max_size: u32) -> AtlasId {
        self.textures.create_atlas(max_size)
    }

    /// The GPU's largest texture edge: the most a sprite in a texture of its own
    /// may measure, and the most an atlas may grow to.
    pub fn max_texture_size(&self) -> u32 {
        self.textures.max_dimension()
    }

    /// Load a sprite from raw RGBA8 bytes, into a texture of its own or the
    /// atlas `placement` names. A `name` already in use moves to the new sprite
    /// and the old one is unloaded.
    ///
    /// The pixels are buffered CPU-side and uploaded lazily in
    /// [`UiRenderer::render`] (or eagerly via [`UiRenderer::flush_textures`]),
    /// so back-to-back loads into an atlas coalesce into a single GPU upload.
    /// Pass them owned (a `Vec<u8>`) to spare a texture of its own a copy.
    pub fn load_sprite_rgba8<'a>(
        &mut self,
        name: &str,
        w: u32,
        h: u32,
        pixels: impl Into<Cow<'a, [u8]>>,
        placement: Placement,
    ) -> Result<SpriteId, SpriteError> {
        let sprite = self.textures.insert(Some(name), w, h, pixels, placement)?;
        // An image loaded under the same key lost its sprite to this one; its
        // cache entry would hand out a freed id.
        self.image_cache.remove(name);
        Ok(sprite)
    }

    /// Register a live GPU texture as an image sprite, without copying its
    /// pixels through the CPU. Re-registering `name` replaces the view/size and
    /// keeps the returned SpriteId stable. Release it with
    /// [`unload_sprite`](Self::unload_sprite).
    ///
    /// The view must be single-sampled, 2D, filterable float and created with
    /// TEXTURE_BINDING usage. Its sampled values must be sRGB-encoded colors
    /// (use a plain Rgba8Unorm view for a sprite render target). As with wgpu
    /// bind groups, incompatible views report a device validation error.
    /// Record writes to the texture before rendering UI that samples it.
    /// The bind group retains the view; this renderer never uploads its pixels.
    pub fn register_texture_view(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<SpriteId, SpriteError> {
        if width == 0 || height == 0 {
            return Err(SpriteError::Empty);
        }
        if width > self.max_texture_size() || height > self.max_texture_size() {
            return Err(SpriteError::TooLarge {
                width,
                height,
                max: self.max_texture_size(),
            });
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui external sprite"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let id = self.textures.external(name, width, height, bind_group)?;
        self.image_cache.remove(name);
        Ok(id)
    }

    /// Replace a sprite's pixels, keeping its id and its texture: for a picture
    /// that changes (a video frame, a live preview), anything drawn with
    /// `sprite` shows the new pixels from the next render, and no texture is
    /// created or freed. Only a sprite with a texture of its own
    /// ([`Placement::Own`]) can be updated, and only at its own size; an atlas
    /// is uploaded whole, so its sprites refuse ([`SpriteError::InAtlas`]).
    ///
    /// Uploaded at the next [`flush_textures`](Self::flush_textures) (which
    /// [`render`](Self::render) does); updating again before then replaces the
    /// waiting pixels, so only the latest is uploaded. Pass them owned to
    /// spare a copy.
    ///
    /// An id is given to a new sprite once its own is unloaded, so stop
    /// updating a sprite when you unload it: a stale id would write into
    /// whichever sprite has it now.
    pub fn update_sprite_rgba8<'a>(
        &mut self,
        sprite: SpriteId,
        pixels: impl Into<Cow<'a, [u8]>>,
    ) -> Result<(), SpriteError> {
        self.textures.update_rgba(sprite, pixels)
    }

    /// Load a `width`×`height` picture stored as YUV 4:2:0, the way video
    /// decoders give it, converted to colour in the shader as `colour` says
    /// (see [`YuvColour`]). It draws with [`DrawList::image`](crate::DrawList::image)
    /// and its siblings like any other sprite (fit, crop, tint, clip and
    /// tiling all apply), from textures of its own: a nine-slice can't use it.
    /// A `name` already in use moves to the new sprite and the old one is
    /// unloaded. Free it with [`unload_sprite`](Self::unload_sprite).
    pub fn create_yuv_sprite(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        planes: YuvPlanes<'_>,
        colour: YuvColour,
    ) -> Result<SpriteId, SpriteError> {
        let sprite = self
            .textures
            .insert_yuv(Some(name), width, height, planes, colour)?;
        self.image_cache.remove(name);
        Ok(sprite)
    }

    /// Replace a YUV sprite's planes, keeping its id and its textures, like
    /// [`update_sprite_rgba8`](Self::update_sprite_rgba8) does for RGBA.
    pub fn update_yuv_sprite(
        &mut self,
        sprite: SpriteId,
        planes: YuvPlanes<'_>,
    ) -> Result<(), SpriteError> {
        self.textures.update_yuv(sprite, planes)
    }

    /// Free a sprite however it was loaded, and its name; an image loaded
    /// through `load_image_*` is forgotten by the image cache too, so loading
    /// it again decodes it again. A texture of its own is released once the
    /// GPU has finished work already submitted with it. Returns `false` for an
    /// unknown or already freed sprite.
    pub fn unload_sprite(&mut self, sprite: SpriteId) -> bool {
        if let Some(name) = self.textures.name(sprite) {
            self.image_cache.remove(name);
        }
        self.textures.remove(sprite)
    }

    /// Look up a sprite id by name.
    pub fn sprite_id(&self, name: &str) -> Option<SpriteId> {
        self.textures.id_for(name)
    }

    /// Pre-generate MSDF tiles for icon glyphs from an application-registered
    /// icon font, so the first frame that shows them doesn't hitch on generation.
    ///
    /// This is opt-in for both the built-in Phosphor font and fonts added via
    /// [`register_icon_font`](crate::render::register_icon_font). Pass only the
    /// glyphs an application expects to need; all others remain lazily generated.
    #[cfg(feature = "phosphor-icons")]
    pub fn prewarm_icons(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: &[crate::render::IconGlyph],
    ) {
        self.text_renderer
            .prewarm_icon_glyphs(device, queue, glyphs);
    }

    /// True if `key` resolves to a loaded sprite — a loaded image
    /// (`load_image_*`), an out-of-band sprite (`load_sprite_rgba8`), or any
    /// registered name. Broader than
    /// [`UiRenderer::has_image`] (which only sees the decoded-image cache); a
    /// caller draining deferred image-load requests uses this to skip keys that
    /// are already drawable (so it never tries to `fs::read` an out-of-band key).
    pub fn has_sprite(&self, key: &str) -> bool {
        self.textures.id_for(key).is_some()
    }

    /// Decode and load an encoded image (PNG/JPEG) from disk where `placement`
    /// says, returning its sprite handle. Cached by path: a repeat load of the
    /// same path is free and returns the existing handle (no re-decode, and
    /// wherever it was first placed). Backs Teardown's `UiImage`.
    pub fn load_image_file(
        &mut self,
        path: impl AsRef<std::path::Path>,
        placement: Placement,
    ) -> Result<SpriteId, ImageError> {
        let key = path.as_ref().to_string_lossy().into_owned();
        if let Some(entry) = self.image_cache.get(&key) {
            return Ok(entry.sprite);
        }
        let bytes = std::fs::read(path.as_ref()).map_err(ImageError::Io)?;
        let (w, h, rgba) = decode_rgba8(&bytes).map_err(ImageError::Decode)?;
        self.insert_image(&key, w, h, rgba, placement)
            .map_err(ImageError::Sprite)
    }

    /// Decode and load an encoded image from in-memory bytes under an explicit
    /// `key` (e.g. an `include_bytes!` asset). Cached by `key` like
    /// [`UiRenderer::load_image_file`].
    pub fn load_image_bytes(
        &mut self,
        key: &str,
        bytes: &[u8],
        placement: Placement,
    ) -> Result<SpriteId, ImageError> {
        if let Some(entry) = self.image_cache.get(key) {
            return Ok(entry.sprite);
        }
        let (w, h, rgba) = decode_rgba8(bytes).map_err(ImageError::Decode)?;
        self.insert_image(key, w, h, rgba, placement)
            .map_err(ImageError::Sprite)
    }

    /// Load an already-decoded RGBA8 image under an explicit `key`, skipping the
    /// decode step entirely. This is the decode-free sibling of
    /// [`UiRenderer::load_image_bytes`]: use it when the caller already holds the
    /// raw pixels (e.g. a notification daemon that rendered an icon into a
    /// buffer) to avoid a pointless RGBA→PNG→decode round-trip.
    ///
    /// Like the other `load_image_*` methods, the image is registered in the
    /// decoded-image cache, so [`has_image`](Self::has_image) /
    /// [`image_size`](Self::image_size) /
    /// [`unload_image`](Self::unload_image) all see it. (Contrast
    /// [`load_sprite_rgba8`](Self::load_sprite_rgba8), which registers only the
    /// sprite name and bypasses the cache.) Cached by `key`: a repeat load is free
    /// and returns the existing handle. Pass the pixels owned (a `Vec<u8>`) to
    /// spare a texture of its own a copy.
    pub fn load_image_rgba8<'a>(
        &mut self,
        key: &str,
        w: u32,
        h: u32,
        rgba: impl Into<Cow<'a, [u8]>>,
        placement: Placement,
    ) -> Result<SpriteId, SpriteError> {
        if let Some(entry) = self.image_cache.get(key) {
            return Ok(entry.sprite);
        }
        self.insert_image(key, w, h, rgba, placement)
    }

    /// Load decoded RGBA8 pixels as a sprite and record the cache entry.
    fn insert_image<'a>(
        &mut self,
        key: &str,
        w: u32,
        h: u32,
        rgba: impl Into<Cow<'a, [u8]>>,
        placement: Placement,
    ) -> Result<SpriteId, SpriteError> {
        let sprite = self.textures.insert(Some(key), w, h, rgba, placement)?;
        self.image_cache.insert(
            key,
            ImageEntry {
                sprite,
                width: w,
                height: h,
            },
        );
        Ok(sprite)
    }

    /// Pixel dimensions of a loaded image, if `key` has been loaded. Backs
    /// Teardown's `UiGetImageSize`.
    pub fn image_size(&self, key: &str) -> Option<(u32, u32)> {
        self.image_cache.get(key).map(|e| (e.width, e.height))
    }

    /// Whether an image `key` has been loaded. Backs Teardown's `UiHasImage`.
    pub fn has_image(&self, key: &str) -> bool {
        self.image_cache.contains(key)
    }

    /// Drop the cache entry for an image `key` (next load re-decodes), and free
    /// its sprite. Backs Teardown's `UiUnloadImage`.
    ///
    /// A texture of its own is released (once the GPU has finished with work
    /// already submitted). An atlas slot is tombstoned and recycled by a later
    /// load, and the atlas is re-uploaded without the sprite's pixels on the
    /// next render; shelf *fragmentation* left by the removal is reclaimed lazily
    /// by [`compact_atlas`](Self::compact_atlas), which a long-running app can
    /// call on its own schedule. Idempotent for an unknown key.
    pub fn unload_image(&mut self, key: &str) {
        if let Some(entry) = self.image_cache.remove(key) {
            self.textures.remove(entry.sprite);
        }
    }

    /// Reclaim shelf fragmentation an atlas has from unloaded sprites,
    /// repacking every live sprite into fresh contiguous shelves without
    /// changing any `SpriteId`. Safe to call at any time; idempotent when
    /// nothing has been removed. Returns `false` for an unknown atlas, or when
    /// the repacked sprites wouldn't fit, which leaves the atlas as it was.
    ///
    /// The texture dimensions are not shrunk (only the *packing* is tightened),
    /// so this keeps the atlas from filling up under churn but does not release
    /// the peak texture size. Call it periodically — e.g. a daemon that loads and
    /// discards many one-off icons — to keep growth bounded.
    pub fn compact_atlas(&mut self, atlas: AtlasId) -> bool {
        self.textures.compact(atlas)
    }

    /// An atlas' current texture dimensions in pixels, as `(width, height)`, or
    /// `None` for an unknown atlas. It grows (1024 → 2048 → …) only when a
    /// sprite doesn't fit, up to the maximum it was created with.
    pub fn atlas_size(&self, atlas: AtlasId) -> Option<(u32, u32)> {
        self.textures.atlas_size(atlas)
    }

    /// Register a nine-slice resource referencing an existing sprite.
    pub fn register_nine_slice(
        &mut self,
        name: &str,
        sprite: SpriteId,
        border: [u32; 4],
    ) -> NineSliceId {
        let id = self.nine_slices.len() as NineSliceId;
        self.nine_slices.push(NineSliceMeta { sprite, border });
        self.nine_slice_names.insert(name.to_string(), id);
        id
    }

    /// Look up a registered nine-slice id by the name it was registered under.
    pub fn nine_slice_id(&self, name: &str) -> Option<NineSliceId> {
        self.nine_slice_names.get(name).copied()
    }

    /// Notify the text sub-renderer of viewport changes. `scale_factor` is the
    /// logical → physical ratio (pass `1.0` when unknown).
    ///
    /// This is the *public* resize hook for callers that change the viewport
    /// outside a frame; the per-pass resize (which reserves the text pipeline's
    /// uniform slot) happens inside `render`/`render_layers`.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) {
        self.text_renderer
            .resize(device, queue, width, height, scale_factor);
    }

    /// Draw a window of a larger canvas: the logical point `(x, y)` lands on the
    /// target's top-left corner, and the target shows the `viewport`-sized
    /// region from there. Clip rects stay in canvas coordinates, so nothing
    /// else changes. The default is `(0, 0)`.
    ///
    /// Use it to render a canvas taller or wider than the GPU's texture limit
    /// in tiles, or to capture one part of a UI. It applies to every later
    /// [`render`](Self::render), [`render_layers`](Self::render_layers) and
    /// [`blur_backdrop`](Self::blur_backdrop) until set again. Keep `x` and `y`
    /// to whole logical pixels so pixel-snapped edges stay sharp.
    pub fn set_view_origin(&mut self, x: f32, y: f32) {
        self.view_origin = [x, y];
        self.text_renderer.set_view_origin(x, y);
    }

    /// The text and MSDF icons the last render prepared.
    #[cfg(all(test, feature = "headless"))]
    pub(crate) fn text_prepared(&self) -> &crate::text::MsdfPrepared {
        self.text_renderer.prepared()
    }

    /// Draw every list in paint order, one batch per paint command, as before
    /// batching, instead of in batches grouped by what overlaps. The pixels
    /// should be the same either way; tests compare the two. The paint-order
    /// path reads no draw boxes and culls nothing.
    #[cfg(feature = "paint-order-oracle")]
    pub fn set_paint_order(&mut self, on: bool) {
        self.paint_order = on;
    }

    /// Check every list's batches, as they are planned, against a brute-force
    /// oracle that tests every pair of draws, and panic where the orders or the
    /// batch count differ. Slow: for tests.
    #[cfg(feature = "paint-order-oracle")]
    pub fn set_check_plan(&mut self, on: bool) {
        self.draw_plan.set_check(on);
    }

    /// Whether lists are drawn in paint order (see `set_paint_order`).
    fn paint_order(&self) -> bool {
        #[cfg(feature = "paint-order-oracle")]
        return self.paint_order;
        #[cfg(not(feature = "paint-order-oracle"))]
        false
    }

    /// How hard glyph origins are pulled onto the device pixel grid before their
    /// quads are emitted — our equivalent of a rasterizer's grid fitting.
    ///
    /// Defaults to [`GlyphSnap::Baseline`], which rounds baselines to whole
    /// device pixels so horizontal strokes stay crisp, matching what FreeType's
    /// `hintslight` (the usual Linux browser setting) does. See [`GlyphSnap`] for
    /// the alternatives and what each costs.
    ///
    /// Snapping quantises glyph positions, so text that slides by fractions of a
    /// pixel — a panel animating open, a smooth-scrolled list — steps instead of
    /// gliding. Set [`GlyphSnap::Off`] for the duration if that shows.
    pub fn set_glyph_snap(&mut self, snap: crate::text::GlyphSnap) {
        self.text_renderer.set_glyph_snap(snap);
    }

    /// The current glyph snapping mode (see
    /// [`set_glyph_snap`](Self::set_glyph_snap)).
    pub fn glyph_snap(&self) -> crate::text::GlyphSnap {
        self.text_renderer.glyph_snap()
    }

    /// Generate small text from outlines **hinted** for the size they are drawn
    /// at, rather than from one shared size-independent distance field. On by
    /// default.
    ///
    /// This is what makes baselines, x-heights and cap-heights land on whole
    /// pixels instead of near them. It costs one atlas entry per glyph per
    /// distinct device size up to
    /// [`MAX_HINTED_PX`](crate::render::MAX_HINTED_PX), so prefer a small set of
    /// text sizes and warm them with
    /// [`TextRenderer::prewarm_ascii`](crate::TextRenderer::prewarm_ascii).
    ///
    /// Requires [`set_glyph_snap`](Self::set_glyph_snap) to be anything but
    /// [`GlyphSnap::Off`](crate::GlyphSnap::Off) — a hinted outline is fitted to
    /// a pixel grid, so the glyph has to be placed on that grid for it to mean
    /// anything.
    pub fn set_glyph_hinting(&mut self, hint: bool) {
        self.text_renderer.set_glyph_hinting(hint);
    }

    /// Whether small text is generated from hinted outlines (see
    /// [`set_glyph_hinting`](Self::set_glyph_hinting)).
    pub fn glyph_hinting(&self) -> bool {
        self.text_renderer.glyph_hinting()
    }

    /// How text and icon edges are corrected for their colour, so light text
    /// on a dark background looks as heavy as dark text on a light one.
    ///
    /// Anti-aliased edges blend in sRGB space, where a half-covered pixel of
    /// light text looks fainter than half, so light-on-dark text otherwise
    /// comes out thinner. Defaults to [`TextContrast::GPUI`], the correction
    /// gpui (Zed) and DirectWrite apply; [`TextContrast::OFF`] turns it off.
    /// Only anti-aliased edges change: glyph interiors, shadows and glows
    /// draw as before. Applies from the next [`render`](Self::render).
    pub fn set_text_contrast(&mut self, contrast: crate::TextContrast) {
        self.text_renderer.set_text_contrast(contrast);
    }

    /// The current text contrast (see
    /// [`set_text_contrast`](Self::set_text_contrast)).
    pub fn text_contrast(&self) -> crate::TextContrast {
        self.text_renderer.text_contrast()
    }

    /// The text glyph atlas's dimensions in pixels (see
    /// [`TextRenderer::glyph_atlas_size`](crate::TextRenderer::glyph_atlas_size)).
    pub fn glyph_atlas_size(&self) -> (u32, u32) {
        self.text_renderer.glyph_atlas_size()
    }

    /// The logical canvas point drawn at the target's top-left corner (see
    /// [`set_view_origin`](Self::set_view_origin)).
    pub fn view_origin(&self) -> (f32, f32) {
        (self.view_origin[0], self.view_origin[1])
    }

    /// Upload pending sprite changes to the GPU: new sprites of their own, and
    /// atlases that changed. Called automatically by `render()`, exposed for
    /// callers that want to control timing.
    pub fn flush_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_flush_textures").entered();
        let sampling = Sampling {
            sampler: &self.sampler,
            rgba: &self.texture_bind_group_layout,
            yuv: &self.yuv_bind_group_layout,
        };
        let uploads = self.textures.flush(device, queue, &sampling);
        self.frame_stats.atlas_uploads += uploads.atlases;
        self.frame_stats.atlas_bytes_uploaded += uploads.atlas_bytes;
        self.frame_stats.texture_uploads += uploads.own;
        self.frame_stats.texture_bytes_uploaded += uploads.own_bytes;
        self.frame_stats.textures_created += uploads.created;
    }

    /// The counters for the frame so far: everything rendered since the last
    /// [`begin_frame`](Self::begin_frame). The same value the latest render call
    /// returned.
    pub fn frame_stats(&self) -> RenderStats {
        self.frame_stats
    }

    /// Open a frame: everything rendered until the next `begin_frame` is expected to
    /// reach the GPU through a single `Queue::submit`.
    ///
    /// **Call this exactly once per frame**, before the first render call whose passes
    /// are submitted together (the widget stack and a modal drawn into one encoder are
    /// one frame; a second encoder submitted separately is another). It resets the
    /// per-frame GPU bump arenas — colour, chrome, circle, icon, nine-slice, text and
    /// uniform — and the frame's [`RenderStats`].
    ///
    /// The boundary is what keeps the passes from destroying each other.
    /// `Queue::write_buffer` does not take effect when it is called: it is staged and
    /// executed at the start of the next `submit`, *before* any recorded pass runs. Two
    /// passes in one submission therefore read whichever bytes were written last — the
    /// last pass's — unless each pass owns a distinct byte range. Those ranges only
    /// exist within a frame, so a renderer that is not told where the frame starts
    /// cannot separate the passes.
    ///
    /// Forgetting the call is not silent: the arenas keep growing, so the buffers
    /// reallocate every frame, which trips the sustained-render-pressure warning, and a
    /// single frame crossing [`ARENA_SANITY_CAP`] logs a dedicated message.
    pub fn begin_frame(&mut self) {
        // Judge the *previous* frame here rather than at the end of a render call: the
        // counters describe a whole frame, and a frame can contain several calls.
        let previous = self.frame_stats;
        if self.pressure.observe(previous) {
            log::warn!(
                "wgpu-gameui: sustained render pressure for {}+ frames: {}. Inspect the returned RenderStats or attach it to DebugReport::with_render_stats().",
                RenderPressureDetector::WARN_AFTER,
                pressure_findings(previous, &self.small_texture_names)
            );
        }
        self.frame_stats = RenderStats::default();
        self.small_texture_names.clear();
        self.frame_boundary_required = false;
        for arena in [
            &mut self.color_vbo,
            &mut self.color_ibo,
            &mut self.icon_insts,
            &mut self.analytic_insts,
            &mut self.circle_insts,
            &mut self.nine_insts,
        ] {
            arena.reset();
        }
        self.uniform.reset();
        self.text_renderer.begin_frame();
        if let Some(blur) = self.blur.as_mut() {
            blur.begin_frame();
        }
    }

    /// Render the entire DrawList in one call.
    /// Render a single `DrawList`.
    ///
    /// `viewport` is the **physical** render-target size in pixels. `scale_factor`
    /// is the display's logical→physical ratio (e.g. `1.0` on a standard display,
    /// `2.0` on a Retina/HiDPI output — pass the window's scale factor). The UI is
    /// laid out in **logical** pixels; the renderer divides the physical viewport
    /// by `scale_factor` when building its projection, so geometry stays the same
    /// logical size while text rasterizes against the higher-resolution
    /// framebuffer (the MSDF path sharpens automatically). Pass `1.0` to disable.
    ///
    /// `viewport` must be the size of `view`. Nothing can check it (a
    /// `TextureView` doesn't report its size), and draws are grouped by which
    /// pixels of a target of that size they reach, so a wrong size can draw
    /// overlapping primitives out of order, not just stretch the frame.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
        scale_factor: f32,
        draw_list: &DrawList,
    ) -> RenderStats {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render").entered();
        if !self.can_render_this_frame("render") {
            return self.frame_stats;
        }
        if self.stale_list.observe(draw_list.id()) {
            Self::warn_stale_list("render", "DrawList");
        }
        self.prepare_pass(device, queue, viewport, scale_factor);
        let target = self.begin_target(device, encoder, view, viewport);
        self.render_one(device, queue, encoder, &target, draw_list);
        self.end_target(encoder, view);
        self.check_frame_arena();
        self.frame_stats
    }

    /// Render a `LayerStack`: base list first, then each layer in push order.
    /// Each layer is drawn after the one below it, so a higher-z layer's quads
    /// correctly overlap a lower-z layer's text/icons.
    ///
    /// As with [`render`](Self::render), `viewport` must be the size of `view`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
        scale_factor: f32,
        layers: &LayerStack,
    ) -> RenderStats {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render_layers").entered();
        if !self.can_render_this_frame("render_layers") {
            return self.frame_stats;
        }
        // Key the footgun detector on the BASE list only: modal/popup/tooltip
        // layers are legitimately transient (built per push), so their ids would
        // be false positives.
        if self.stale_list.observe(layers.base_id()) {
            Self::warn_stale_list("render_layers", "LayerStack");
        }
        self.prepare_pass(device, queue, viewport, scale_factor);
        let target = self.begin_target(device, encoder, view, viewport);
        self.render_one(device, queue, encoder, &target, layers.base());
        for layer in layers.layers() {
            self.render_one(device, queue, encoder, &target, &layer.list);
        }
        self.end_target(encoder, view);
        self.check_frame_arena();
        self.frame_stats
    }

    /// The view this render call's UI passes draw into: the host `view` itself
    /// (direct path), or the offscreen layer, freshly cleared (see the
    /// type-level docs).
    fn begin_target(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
    ) -> wgpu::TextureView {
        match self.offscreen.as_mut() {
            Some(offscreen) => offscreen.begin(device, encoder, viewport),
            None => view.clone(),
        }
    }

    /// Composite the offscreen layer onto the host `view` (no-op on the direct
    /// path).
    fn end_target(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        if let Some(offscreen) = self.offscreen.as_ref() {
            offscreen.composite(encoder, view);
        }
    }

    /// The clear value that shows the sRGB-encoded `color` — typically
    /// `theme.background` — on this renderer's target. Linear-light targets
    /// (`*Srgb`, float) need the decoded value; direct targets take it as-is.
    ///
    /// A surface's format comes from whatever the adapter offers, so which of
    /// [`Srgb`](crate::color::Srgb) / [`Linear`](crate::color::Linear) a
    /// renderer needs is a run-time fact: this returns the [`Clear`] that
    /// holds whichever one. It is already converted for the target this
    /// renderer was built for, so hand it straight to `wgpu`:
    ///
    /// ```ignore
    /// let ops = wgpu::Operations {
    ///     load: wgpu::LoadOp::Clear(ui.clear_color(theme.background).color()),
    ///     store: wgpu::StoreOp::Store,
    /// };
    /// ```
    ///
    /// Somewhere the space *is* known statically — a capture texture is
    /// always sRGB — take the concrete type instead, and the wrong one stops
    /// compiling rather than showing the wrong colour.
    pub fn clear_color(&self, color: [f32; 4]) -> Clear {
        self.plan.clear_color(color)
    }

    /// What this renderer's target stores — which arm
    /// [`clear_color`](Self::clear_color) returns.
    pub fn color_space(&self) -> ColorSpace {
        self.plan.space()
    }

    /// Whether this renderer draws through its offscreen layer (the target
    /// stores linear light) instead of straight into the target.
    pub fn uses_offscreen_layer(&self) -> bool {
        self.offscreen.is_some()
    }

    /// Emit the once-per-renderer warning that the caller is feeding a
    /// freshly-constructed list/stack every frame (see [`StaleListDetector`]).
    fn warn_stale_list(method: &str, kind: &str) {
        log::warn!(
            "wgpu-gameui: `UiRenderer::{method}` has been called with a \
             freshly-constructed `{kind}` for {}+ consecutive frames. Each one \
             reallocates its buffers, and one built with `new()` also builds \
             its own font system, so every label is re-shaped every frame \
             (this can cost milliseconds per frame). Construct ONE `{kind}` \
             and reuse it across frames, calling `.clear()` each frame to reset \
             geometry while keeping its buffers and warm text caches.",
            StaleListDetector::WARN_AFTER,
        );
    }

    /// Blur an **app-provided** scene texture into `target` over `region`, for
    /// menu/pause "frosted glass" backdrops.
    ///
    /// The renderer never samples its own framebuffer, so the app supplies the
    /// already-rendered scene as a sampleable [`Backdrop`] (a `TextureView` with
    /// `TEXTURE_BINDING` usage plus its physical size). This records a separable
    /// two-pass Gaussian into `encoder` and writes the blurred region straight
    /// into `target`; draw the UI panels afterwards so they sit crisp on top.
    ///
    /// `region` is in **logical** pixels (UI coordinates); `viewport` is the
    /// target's **physical** size and `scale_factor` the logical→physical ratio,
    /// matching [`render`](Self::render). A degenerate region is a no-op. With a
    /// [view origin](Self::set_view_origin) set, `region` is still in canvas
    /// coordinates and the backdrop must show the same window as the target.
    ///
    /// Typical frame: render scene → `blur_backdrop(region)` → `render(panels)`,
    /// all into one encoder under one [`begin_frame`](Self::begin_frame).
    #[allow(clippy::too_many_arguments)]
    pub fn blur_backdrop(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        backdrop: &Backdrop,
        region: Rect,
        viewport: (u32, u32),
        scale_factor: f32,
        params: &BlurParams,
    ) {
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        // `region` is on the canvas; the target (and the backdrop, which mirrors
        // it) shows the window starting at the view origin.
        let region_phys = [
            (region.x - self.view_origin[0]) * scale,
            (region.y - self.view_origin[1]) * scale,
            region.width * scale,
            region.height * scale,
        ];
        if region_phys[2] <= 0.0 || region_phys[3] <= 0.0 {
            return;
        }
        if self.blur.is_none() {
            self.blur = Some(Blur::new(
                device,
                self.host_format,
                self.plan.space() == ColorSpace::Linear,
            ));
        }
        let blur = self.blur.as_mut().expect("blur just ensured");
        blur.run(
            device,
            queue,
            encoder,
            target,
            backdrop,
            region_phys,
            viewport,
            params,
        );
    }

    /// Per-pass setup: give this pass its own ortho slot, point the text renderer at
    /// the pass's viewport, and upload any pending atlas change.
    ///
    /// Deliberately resets nothing — the arenas span the whole frame
    /// ([`begin_frame`](Self::begin_frame)), which is what keeps the passes of one
    /// submission from overwriting each other.
    fn prepare_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: (u32, u32),
        scale_factor: f32,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_prepare_pass").entered();
        // The framebuffer is `viewport` physical pixels, but the UI is laid out
        // in logical pixels. Project logical → NDC by dividing the physical size
        // by the scale factor: a logical point still maps to the same NDC, while
        // the GPU rasterizes onto the full physical target (text self-sharpens).
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let logical_w = viewport.0 as f32 / scale;
        let logical_h = viewport.1 as f32 / scale;
        let uniforms = Uniforms {
            view_proj: ortho_matrix(self.view_origin, logical_w, logical_h),
            view: [self.view_origin[0], self.view_origin[1], 1.0 / scale, 0.0],
        };
        let (slot, grew) = self.uniform.allocate(device);
        if grew {
            self.frame_stats.buffer_reallocations += 1;
        }
        queue.write_buffer(
            self.uniform.buffer(),
            slot,
            bytemuck::cast_slice(&[uniforms]),
        );
        self.frame_stats.buffer_write_calls += 1;
        self.frame_stats.buffer_bytes_uploaded += UNIFORM_SIZE;
        self.pass_uniform_offset = slot as u32;
        self.pass_target = Target::new(viewport, scale, self.view_origin);
        self.text_renderer
            .resize(device, queue, viewport.0, viewport.1, scale);
        self.flush_textures(device, queue);
        self.check_frame_arena();
    }

    /// Sum of every per-frame arena cursor — how many bytes of GPU scratch the
    /// current frame (or, if [`begin_frame`](Self::begin_frame) was never called, a
    /// growing pile of frames) is holding.
    fn frame_arena_bytes(&self) -> u64 {
        [
            &self.color_vbo,
            &self.color_ibo,
            &self.icon_insts,
            &self.analytic_insts,
            &self.circle_insts,
            &self.nine_insts,
        ]
        .iter()
        .map(|arena| arena.used())
        .sum::<u64>()
            + self.uniform.bytes_used()
            + self.text_renderer.frame_arena_bytes()
    }

    /// Return whether a new render call may consume this submission's arenas.
    /// Once the cap has been crossed, later calls are refused until `begin_frame`
    /// resets the cursors; this protects applications that forgot the explicit
    /// frame boundary from a late, fatal wgpu max-buffer validation error.
    fn can_render_this_frame(&mut self, method: &str) -> bool {
        if !self.frame_boundary_required {
            return true;
        }
        log::error!(
            "wgpu-gameui: refusing `UiRenderer::{method}` because its per-frame GPU arenas exceeded {} MiB without a `UiRenderer::begin_frame()` reset. Call `begin_frame()` exactly once before encoding each submission.",
            ARENA_SANITY_CAP / (1024 * 1024)
        );
        false
    }

    /// Arm the missed-frame gate as soon as the current submission exceeds the
    /// sanity cap. The offending render still completed safely; later calls are
    /// refused before another geometric growth allocation is attempted.
    fn check_frame_arena(&mut self) {
        if !frame_arena_within_cap(self.frame_arena_bytes()) {
            self.frame_boundary_required = true;
            log::error!(
                "wgpu-gameui: this submission has bumped more than {} MiB through per-frame GPU arenas. `UiRenderer::begin_frame()` was probably not called before it; rendering is now blocked until that method resets the arenas, preventing an oversized GPU-buffer allocation.",
                ARENA_SANITY_CAP / (1024 * 1024)
            );
        }
    }

    fn render_one(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        draw_list: &DrawList,
    ) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_render_one").entered();

        self.frame_stats.draw_lists += 1;
        self.frame_stats.primitives += draw_list.prim_counts().total();
        self.frame_stats.shadow_instances += draw_list.shadow_instance_count();
        self.frame_stats.stripe_instances += draw_list.stripe_instance_count();
        self.frame_stats.segment_instances += draw_list.segment_instance_count();
        self.frame_stats.paint_runs += draw_list.paint_commands().len();

        if draw_list.paint_cmds.is_empty() {
            self.render_unordered(device, queue, encoder, view, draw_list);
        } else {
            self.render_batches(device, queue, encoder, view, draw_list);
        }
        // The text renderer's own buffer writes, this call's and those since
        // `begin_frame` outside one.
        let text = self.text_renderer.take_upload_stats();
        self.frame_stats.buffer_write_calls += text.buffer_write_calls;
        self.frame_stats.buffer_bytes_uploaded += text.buffer_bytes_uploaded;
        self.frame_stats.buffer_reallocations += text.buffer_reallocations;
    }

    /// Draw a list's paint stream in batches (`docs/design/draw-call-batching.md`):
    /// plan it, upload each family's records once in batch order, and draw
    /// every batch in one pass.
    fn render_batches(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        draw_list: &DrawList,
    ) {
        // Place the whole list's glyphs and icons, and build their instances,
        // before any batch captures an atlas bind group: the atlas it binds
        // has to hold every glyph the list draws.
        let text_commands = draw_list.paint_cmds.iter().filter_map(|cmd| match cmd {
            PaintCmd::Text { draws } => Some(draws.clone()),
            _ => None,
        });
        self.text_renderer
            .prepare_texts(device, queue, &draw_list.texts, text_commands);
        #[cfg(feature = "phosphor-icons")]
        self.text_renderer
            .prepare_icons(device, queue, &draw_list.icons_msdf);

        let paint_order = self.paint_order();
        let resolver = Resolver::new(
            &self.textures,
            &self.nine_slices,
            &self.nine_slice_names,
            &self.warned,
        );
        let prepared = self.text_renderer.prepared();
        if paint_order {
            self.draw_plan
                .build_paint_order(draw_list, prepared, &resolver);
        } else {
            self.draw_plan
                .build(draw_list, &self.pass_target, prepared, &resolver);
            #[cfg(feature = "paint-order-oracle")]
            self.draw_plan.check();
        }
        self.batch_draws.fill(&self.draw_plan, draw_list, prepared);
        // Soup recorded after the last paint command, or appended to the
        // public arrays after it, paints over everything else.
        let committed = draw_list.soup_committed_indices as usize;
        if let Some(tail) = draw_list.indices.get(committed..) {
            self.batch_draws.push_soup(tail);
        }

        let order = self.draw_plan.stats();
        self.frame_stats.batches += self.draw_plan.batches().len();
        self.frame_stats.units_ordered += order.units_ordered as usize;
        self.frame_stats.order_overlap_tests += order.overlap_tests;
        self.frame_stats.order_cell_visits += order.cell_visits;

        if self.batch_draws.draws.is_empty() {
            return;
        }
        let upload = self.upload_batches(device, queue, draw_list);
        self.frame_stats.render_passes += 1;
        let mut pass = super::load_pass(encoder, view, "ui paint stream pass");
        self.draw_batches(&mut pass, &upload);
    }

    /// Draw a list filled through the public arrays, with no paint stream:
    /// each family in a pass of its own, nine-slices, soup, sprites, icons and
    /// then text.
    ///
    /// Kept as it was before batching: it builds its nine-slice and sprite
    /// instances into `Vec`s of its own each call rather than through
    /// [`BatchDraws`], which only lists with a paint stream use.
    fn render_unordered(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        draw_list: &DrawList,
    ) {
        let (nine, bindings) = self.build_nine_slice_instances(&draw_list.nine_slices);
        if !nine.is_empty() {
            for run in self.upload_nine_slices(device, queue, &nine, &bindings) {
                self.draw_instanced_pass(encoder, view, &run);
            }
        }
        if !draw_list.vertices.is_empty() && !draw_list.indices.is_empty() {
            self.draw_color(
                device,
                queue,
                encoder,
                view,
                &draw_list.vertices,
                &draw_list.indices,
            );
        }
        let (icons, bindings) = self.build_icon_instances(&draw_list.icons);
        if !icons.is_empty() {
            for run in self.upload_icons(device, queue, &icons, &bindings) {
                self.draw_instanced_pass(encoder, view, &run);
            }
        }
        #[cfg(feature = "phosphor-icons")]
        {
            let icons = &draw_list.icons_msdf;
            self.text_renderer.prepare_icons(device, queue, icons);
            if let Some(run) =
                self.text_renderer
                    .upload_icon_command(device, queue, 0..icons.len() as u32)
            {
                self.frame_stats.render_passes += 1;
                self.frame_stats.draw_calls += 1;
                self.frame_stats.icon_runs += 1;
                let mut pass = super::load_pass(encoder, view, "msdf icon pass");
                self.text_renderer.draw_prepared(&mut pass, &run);
            }
        }
        let texts = &draw_list.texts;
        self.text_renderer.prepare_texts(
            device,
            queue,
            texts,
            std::iter::once(0..texts.len() as u32),
        );
        if let Some(run) = self.text_renderer.upload_text_command(device, queue, 0) {
            self.frame_stats.render_passes += 1;
            self.frame_stats.draw_calls += 1;
            self.frame_stats.text_runs += 1;
            let mut pass = super::load_pass(encoder, view, "msdf text pass");
            self.text_renderer.draw_prepared(&mut pass, &run);
        }
    }

    /// Sprites and nine-slices resolved to their textures, warning once about
    /// each that isn't there.
    fn resolver(&self) -> Resolver<'_> {
        Resolver::new(
            &self.textures,
            &self.nine_slices,
            &self.nine_slice_names,
            &self.warned,
        )
    }

    /// Each drawable icon's instance, and the texture it samples.
    fn build_icon_instances(&self, icons: &[IconDraw]) -> (Vec<IconInstance>, Vec<Binding>) {
        let resolver = self.resolver();
        icons
            .iter()
            .filter_map(|icon| {
                let at = resolver.sprite(icon)?;
                Some((icon_instance(icon, &at), at.binding))
            })
            .unzip()
    }

    /// Each drawable panel's instance, and the texture it samples.
    fn build_nine_slice_instances(
        &self,
        draws: &[NineSliceDraw],
    ) -> (Vec<NineSliceInstance>, Vec<Binding>) {
        let resolver = self.resolver();
        draws
            .iter()
            .filter_map(|draw| {
                let (at, border) = resolver.nine_slice(draw)?;
                Some((nine_slice_instance(draw, &at, border), at.binding))
            })
            .unzip()
    }

    fn draw_color(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        verts: &[Vertex],
        indices: &[u32],
    ) {
        let (retired, stats) = (&mut self.retired_buffers, &mut self.frame_stats);
        let v_off =
            self.color_vbo
                .write(device, queue, bytemuck::cast_slice(verts), retired, stats);
        let i_off =
            self.color_ibo
                .write(device, queue, bytemuck::cast_slice(indices), retired, stats);

        self.frame_stats.render_passes += 1;
        let mut pass = super::load_pass(encoder, view, "ui color pass");
        pass.set_pipeline(&self.color_pipeline);
        pass.set_bind_group(0, self.uniform.bind_group(), &[self.pass_uniform_offset]);
        // Index 0 maps to the first vertex of this pass's slice (base_vertex 0
        // + the sliced vertex buffer), so per-pass indices stay 0-based.
        pass.set_vertex_buffer(0, self.color_vbo.buffer().slice(v_off..));
        pass.set_index_buffer(
            self.color_ibo.buffer().slice(i_off..),
            wgpu::IndexFormat::Uint32,
        );
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        self.frame_stats.draw_calls += 1;
        self.frame_stats.color_runs += 1;
    }

    /// Upload one list's batches: each family's records, in draw order, in one
    /// buffer write (soup's vertices as the list has them, its indices in
    /// draw order). Uploading per paint command once rewrote the whole soup
    /// and instance arrays for each, which overflowed the buffers and turned
    /// soup into screen-sized triangles.
    fn upload_batches(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw_list: &DrawList,
    ) -> BatchUpload {
        let draws = &self.batch_draws;
        let (retired, stats) = (&mut self.retired_buffers, &mut self.frame_stats);
        let mut write = |arena: &mut FrameArena, data: &[u8]| {
            if data.is_empty() {
                0
            } else {
                arena.write(device, queue, data, retired, stats)
            }
        };
        let (vertex_offset, index_offset) = if draws.indices.is_empty() {
            (0, 0)
        } else {
            (
                write(
                    &mut self.color_vbo,
                    bytemuck::cast_slice(&draw_list.vertices),
                ),
                write(&mut self.color_ibo, bytemuck::cast_slice(&draws.indices)),
            )
        };
        let analytic_offset = write(
            &mut self.analytic_insts,
            bytemuck::cast_slice(&draws.analytic),
        );
        let circle_offset = write(&mut self.circle_insts, bytemuck::cast_slice(&draws.circles));
        let sprite_offset = write(&mut self.icon_insts, bytemuck::cast_slice(&draws.sprites));
        let nine_offset = write(
            &mut self.nine_insts,
            bytemuck::cast_slice(&draws.nine_slices),
        );
        let text = self.text_renderer.upload_instances(
            device,
            queue,
            &self.batch_draws.text,
            MsdfAtlas::Glyphs,
        );
        #[cfg(feature = "phosphor-icons")]
        let icons = self.text_renderer.upload_instances(
            device,
            queue,
            &self.batch_draws.icons,
            MsdfAtlas::Icons,
        );
        BatchUpload {
            vertex_offset,
            index_offset,
            analytic_offset,
            circle_offset,
            sprite_offset,
            nine_offset,
            text,
            #[cfg(feature = "phosphor-icons")]
            icons,
        }
    }

    /// Draw one list's batches, in draw order, into the list's pass.
    fn draw_batches(&mut self, pass: &mut wgpu::RenderPass<'_>, upload: &BatchUpload) {
        let quad = 0..QUAD_INDICES.len() as u32;
        // Taken for the loop, so drawing can note small textures; put back after.
        let draws = std::mem::take(&mut self.batch_draws.draws);
        for draw in &draws {
            let range = draw.range.clone();
            // Text binds a uniform of its own at group 0, so every draw binds
            // what it needs.
            match draw.kind {
                DrawKind::Soup => {
                    pass.set_pipeline(&self.color_pipeline);
                    pass.set_bind_group(0, self.uniform.bind_group(), &[self.pass_uniform_offset]);
                    pass.set_vertex_buffer(
                        0,
                        self.color_vbo.buffer().slice(upload.vertex_offset..),
                    );
                    pass.set_index_buffer(
                        self.color_ibo.buffer().slice(upload.index_offset..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(range, 0, 0..1);
                    self.frame_stats.color_runs += 1;
                }
                DrawKind::Analytic | DrawKind::Circle => {
                    let (pipeline, buffer, offset) = match draw.kind {
                        DrawKind::Analytic => (
                            &self.analytic_pipeline,
                            self.analytic_insts.buffer(),
                            upload.analytic_offset,
                        ),
                        _ => (
                            &self.circle_pipeline,
                            self.circle_insts.buffer(),
                            upload.circle_offset,
                        ),
                    };
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, self.uniform.bind_group(), &[self.pass_uniform_offset]);
                    pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
                    pass.set_vertex_buffer(1, buffer.slice(offset..));
                    pass.set_index_buffer(
                        self.chrome_base_ibo.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.draw_indexed(quad.clone(), 0, range);
                    self.frame_stats.color_runs += 1;
                }
                DrawKind::Sprite(binding) | DrawKind::NineSlice(binding) => {
                    // Every texture is uploaded by `prepare_pass`; skip rather
                    // than bind nothing should a caller draw before it.
                    let texture = self.textures.bind_group(binding);
                    debug_assert!(
                        texture.is_some(),
                        "{binding:?} drawn before its texture was uploaded"
                    );
                    let Some(texture) = texture else {
                        continue;
                    };
                    let (pipeline, buffer, offset) = match (draw.kind, binding) {
                        (DrawKind::NineSlice(_), _) => (
                            &self.nine_slice_pipeline,
                            self.nine_insts.buffer(),
                            upload.nine_offset,
                        ),
                        (_, Binding::Yuv(_)) => (
                            &self.yuv_pipeline,
                            self.icon_insts.buffer(),
                            upload.sprite_offset,
                        ),
                        _ => (
                            &self.icon_pipeline,
                            self.icon_insts.buffer(),
                            upload.sprite_offset,
                        ),
                    };
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, self.uniform.bind_group(), &[self.pass_uniform_offset]);
                    pass.set_bind_group(1, texture, &[]);
                    pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
                    pass.set_vertex_buffer(1, buffer.slice(offset..));
                    pass.set_index_buffer(
                        self.chrome_base_ibo.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.draw_indexed(quad.clone(), 0, range);
                    self.frame_stats.icon_runs += 1;
                    if let Some(id) = draw.small_texture {
                        self.frame_stats.small_texture_batches += 1;
                        self.note_small_texture(id);
                    }
                }
                DrawKind::Text => {
                    let run = upload
                        .text
                        .as_ref()
                        .expect("text batches uploaded their instances");
                    self.text_renderer.draw_prepared_range(pass, run, range);
                    self.frame_stats.text_runs += 1;
                }
                #[cfg(feature = "phosphor-icons")]
                DrawKind::IconMsdf => {
                    let run = upload
                        .icons
                        .as_ref()
                        .expect("icon batches uploaded their instances");
                    self.text_renderer.draw_prepared_range(pass, run, range);
                    self.frame_stats.icon_runs += 1;
                }
            }
            self.frame_stats.draw_calls += 1;
        }
        self.batch_draws.draws = draws;
    }

    /// Upload icon/image instances, one instanced draw per stretch sampling one
    /// texture (`bindings`, one per instance): the chrome unit-quad base mesh +
    /// one [`IconInstance`] per icon. The vertex shader bilinearly interpolates
    /// the baked-in corners and the fragment samples the sprite's texture.
    fn upload_icons(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[IconInstance],
        bindings: &[Binding],
    ) -> Vec<InstancedRun> {
        let off = self.icon_insts.write(
            device,
            queue,
            bytemuck::cast_slice(instances),
            &mut self.retired_buffers,
            &mut self.frame_stats,
        );
        let buffer = self.icon_insts.buffer().clone();
        let stride = std::mem::size_of::<IconInstance>() as u64;
        self.instanced_runs(InstancedKind::Icon, &buffer, off, stride, bindings)
    }

    /// One run per stretch of `bindings` sampling the same texture, over the
    /// instances written to `buffer` from `offset`, `stride` bytes each. Counts
    /// each run of a small sprite in a texture of its own.
    fn instanced_runs(
        &mut self,
        kind: InstancedKind,
        buffer: &wgpu::Buffer,
        offset: u64,
        stride: u64,
        bindings: &[Binding],
    ) -> Vec<InstancedRun> {
        let mut runs = Vec::new();
        for (binding, range) in textures::batches(bindings) {
            // Every texture is uploaded by `prepare_pass`; skip rather than bind
            // nothing should a caller draw before it.
            let texture = self.textures.bind_group(binding).cloned();
            debug_assert!(
                texture.is_some(),
                "{binding:?} drawn before its texture was uploaded"
            );
            let Some(texture) = texture else {
                continue;
            };
            if let Binding::Own(id) = binding
                && self.textures.resolve(id).is_some_and(|at| at.small_own())
            {
                self.frame_stats.small_texture_batches += 1;
                self.note_small_texture(id);
            }
            let kind = match binding {
                Binding::Yuv(_) => InstancedKind::Yuv,
                _ => kind,
            };
            runs.push(InstancedRun {
                kind,
                buffer: buffer.clone(),
                offset: offset + range.start as u64 * stride,
                count: range.len() as u32,
                texture,
            });
        }
        runs
    }

    /// Remember a small sprite drawn from a texture of its own, to name it when
    /// the frame is reported.
    fn note_small_texture(&mut self, id: SpriteId) {
        if self.small_texture_names.len() >= SMALL_TEXTURE_NAMES {
            return;
        }
        // Compared before anything is copied: this runs for every such draw.
        let names = &mut self.small_texture_names;
        match self.textures.name(id) {
            Some(name) => {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_owned());
                }
            }
            None => {
                let unnamed = |n: &String| {
                    n.strip_prefix("sprite #")
                        .is_some_and(|rest| rest.parse() == Ok(id))
                };
                if !names.iter().any(unnamed) {
                    names.push(format!("sprite #{id}"));
                }
            }
        }
    }

    /// Draw an uploaded icon or nine-slice run into the shared pass.
    fn draw_instanced(&self, pass: &mut wgpu::RenderPass<'_>, run: &InstancedRun) {
        pass.set_pipeline(match run.kind {
            InstancedKind::Icon => &self.icon_pipeline,
            InstancedKind::NineSlice => &self.nine_slice_pipeline,
            InstancedKind::Yuv => &self.yuv_pipeline,
        });
        pass.set_bind_group(0, self.uniform.bind_group(), &[self.pass_uniform_offset]);
        pass.set_bind_group(1, &run.texture, &[]);
        pass.set_vertex_buffer(0, self.chrome_base_vbo.slice(..));
        pass.set_vertex_buffer(1, run.buffer.slice(run.offset..));
        pass.set_index_buffer(self.chrome_base_ibo.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..QUAD_INDICES.len() as u32, 0, 0..run.count);
    }

    /// Draw an uploaded icon or nine-slice run in a pass of its own (the
    /// unordered compatibility path).
    fn draw_instanced_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        run: &InstancedRun,
    ) {
        self.frame_stats.render_passes += 1;
        self.frame_stats.draw_calls += 1;
        self.frame_stats.icon_runs += 1;
        let mut pass = super::load_pass(encoder, view, "ui instanced pass");
        self.draw_instanced(&mut pass, run);
    }

    /// Upload nine-slice panels, one instanced draw per stretch sampling one
    /// texture (`bindings`, one per panel): the chrome unit-quad base mesh + one
    /// [`NineSliceInstance`] per panel. The fragment remaps local coords →
    /// source UV (nine-region piecewise map) and samples the sprite's texture.
    fn upload_nine_slices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[NineSliceInstance],
        bindings: &[Binding],
    ) -> Vec<InstancedRun> {
        let off = self.nine_insts.write(
            device,
            queue,
            bytemuck::cast_slice(instances),
            &mut self.retired_buffers,
            &mut self.frame_stats,
        );
        let buffer = self.nine_insts.buffer().clone();
        let stride = std::mem::size_of::<NineSliceInstance>() as u64;
        self.instanced_runs(InstancedKind::NineSlice, &buffer, off, stride, bindings)
    }
}

/// The sampler and bind group layout every sprite texture is drawn through
/// (group 1 of the icon and nine-slice pipelines).
fn create_sprite_sampling(device: &wgpu::Device) -> (wgpu::Sampler, wgpu::BindGroupLayout) {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ui sprite texture bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    (sampler, bgl)
}

/// The bind group layout a YUV sprite is drawn through (group 1 of the YUV
/// pipeline): its three planes, the sprite sampler, and its colour conversion.
fn create_yuv_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let plane = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ui yuv sprite bgl"),
        entries: &[
            plane(0),
            plane(1),
            plane(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

/// A pipeline drawing icon instances (`vs_icon`) with the fragment shader
/// `fragment`, sampling what `layouts[1]` binds.
fn create_icon_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    work_format: wgpu::TextureFormat,
    layouts: &[&wgpu::BindGroupLayout],
    fragment: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: layouts,
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_icon"),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &QUAD_ATTRIBS,
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<IconInstance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &ICON_INSTANCE_ATTRIBS,
                },
            ],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment),
            targets: &[Some(wgpu::ColorTargetState {
                format: work_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

/// The sustained-pressure report for a frame: its warnings, naming a few of the
/// small sprites in textures of their own when those are among them.
fn pressure_findings(stats: RenderStats, small_texture_names: &[String]) -> String {
    let mut findings = stats.warnings().join("; ");
    if stats.small_texture_batches >= SMALL_TEXTURE_BATCHES_WARN && !small_texture_names.is_empty()
    {
        findings.push_str(&format!(
            " (among them: {})",
            small_texture_names.join(", ")
        ));
    }
    findings
}

/// Project the logical window `origin .. origin + (width, height)` onto the whole
/// target: `origin` lands on the target's top-left corner. Positive Y is down,
/// matching the DrawList coordinate system.
pub(crate) fn ortho_matrix(origin: [f32; 2], width: f32, height: f32) -> [[f32; 4]; 4] {
    let (l, r, t, b) = (origin[0], origin[0] + width, origin[1], origin[1] + height);
    [
        [2.0 / (r - l), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (t - b), 0.0, 0.0],
        [0.0, 0.0, 0.5, 0.0],
        [-(r + l) / (r - l), -(t + b) / (t - b), 0.5, 1.0],
    ]
}

/// Resolve an optional normalized within-sprite crop against a sprite's full
/// atlas UV rect `[u0, v0, u1, v1]`. `src` components are 0..1 fractions of the
/// sprite; `None` returns the full rect unchanged.
fn apply_crop_uv(full: [f32; 4], src: Option<[f32; 4]>) -> [f32; 4] {
    match src {
        Some([u0, v0, u1, v1]) => {
            let span_u = full[2] - full[0];
            let span_v = full[3] - full[1];
            [
                full[0] + u0 * span_u,
                full[1] + v0 * span_v,
                full[0] + u1 * span_u,
                full[1] + v1 * span_v,
            ]
        }
        None => full,
    }
}

/// The instance drawing sprite `icon` from `at`.
pub(crate) fn icon_instance(icon: &IconDraw, at: &Resolved) -> IconInstance {
    let (region, (aw, ah)) = (at.region, at.texture);
    let uv = apply_crop_uv(region.uv(aw, ah), icon.src);
    let clip = icon.clip.map(|c| [c.x, c.y, c.width, c.height]);
    // One-tile UV span for the tile path: the sprite's own atlas span divided
    // by the tile count requested along each axis (`icon.src` u1/v1 are those
    // counts). Computed from the *region* — not the mapped `uv`, which spans
    // tile_count tiles after apply_crop_uv.
    let tile_span = match (icon.wrap, icon.src) {
        (true, Some([_, _, tu, tv])) if tu > 0.0 && tv > 0.0 => {
            let full = region.uv(aw, ah);
            [(full[2] - full[0]) / tu, (full[3] - full[1]) / tv]
        }
        _ => [0.0, 0.0],
    };
    build_icon_instance(icon.corners, uv, icon.tint, clip, icon.wrap, tile_span)
}

/// The instance drawing panel `draw` from `at`, with border insets `border`.
pub(crate) fn nine_slice_instance(
    draw: &NineSliceDraw,
    at: &Resolved,
    border: [u32; 4],
) -> NineSliceInstance {
    let (region, (aw, ah)) = (at.region, at.texture);
    build_nine_slice_instance(
        draw.local.x,
        draw.local.y,
        draw.local.width,
        draw.local.height,
        &draw.transform,
        draw.tint,
        draw.clip.map(|c| [c.x, c.y, c.width, c.height]),
        region,
        border,
        aw,
        ah,
    )
}

/// Build one [`IconInstance`] from a quad's 4 world-space corners (TL, TR, BR,
/// BL — matching `Affine2::transform_rect_corners`), its source UV rect, tint,
/// and clip. The corners are baked in and bilinearly interpolated in `vs_icon`,
/// so any rotation/scale/shear from the active transform is preserved with no
/// fallback. Mirrors the per-vertex UV assignment the old tessellator did.
fn build_icon_instance(
    corners: [[f32; 2]; 4],
    uv: [f32; 4], // u0, v0, u1, v1
    tint: [f32; 4],
    clip: Option<[f32; 4]>,
    wrap: bool,
    tile_span: [f32; 2], // one-tile UV span (atlas units); [0,0] unless wrapping
) -> IconInstance {
    let (clip_rect, clip_enabled) = match clip {
        Some(r) => (r, 1.0),
        None => ([0.0; 4], 0.0),
    };
    let tl = corners[0];
    let tr = corners[1];
    let br = corners[2];
    let bl = corners[3];
    IconInstance {
        c_tl_tr: [tl[0], tl[1], tr[0], tr[1]],
        c_br_bl: [br[0], br[1], bl[0], bl[1]],
        uv_rect: uv,
        tint,
        clip: clip_rect,
        // flags[1] = tile-wrap: the fragment shader folds `uv` modulo the
        // one-tile span in flags[2..4] so one instance tiles any dest.
        flags: [
            clip_enabled,
            u8::from(wrap) as f32,
            tile_span[0],
            tile_span[1],
        ],
    }
}

/// Build one [`NineSliceInstance`] from a panel's local rect, transform, tint,
/// clip, and the resolved atlas region + border. The 9-region UV map and the
/// border collapse are evaluated per-pixel in `fs_nine_slice`; here we only
/// resolve the outer/inner UV stops and pack the affine. Mirrors the UV math the
/// old `tessellate_nine_slice` did on the CPU.
#[allow(clippy::too_many_arguments)]
fn build_nine_slice_instance(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    transform: &crate::affine::Affine2,
    tint: [f32; 4],
    clip: Option<[f32; 4]>,
    region: crate::render::AtlasRegion,
    border: [u32; 4],
    atlas_w: u32,
    atlas_h: u32,
) -> NineSliceInstance {
    let bl = border[0] as f32;
    let bt = border[1] as f32;
    let br = border[2] as f32;
    let bb = border[3] as f32;

    // UVs: convert source-pixel borders against atlas dimensions.
    let aw = atlas_w as f32;
    let ah = atlas_h as f32;
    let u0 = region.x as f32 / aw;
    let u1 = (region.x as f32 + bl) / aw;
    let u2 = ((region.x + region.w) as f32 - br) / aw;
    let u3 = (region.x + region.w) as f32 / aw;
    let v0 = region.y as f32 / ah;
    let v1 = (region.y as f32 + bt) / ah;
    let v2 = ((region.y + region.h) as f32 - bb) / ah;
    let v3 = (region.y + region.h) as f32 / ah;

    let (clip_rect, clip_enabled) = match clip {
        Some(r) => (r, 1.0),
        None => ([0.0; 4], 0.0),
    };

    NineSliceInstance {
        lin: [transform.a, transform.b, transform.c, transform.d],
        translate: [transform.tx, transform.ty, clip_enabled, 0.0],
        origin_size: [x, y, w, h],
        uv_outer: [u0, v0, u3, v3],
        uv_inner: [u1, v1, u2, v2],
        border: [bl, bt, br, bb],
        tint,
        clip: clip_rect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::AtlasRegion;

    fn approx4(a: [f32; 4], b: [f32; 4]) {
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-5, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn crop_uv_none_is_identity() {
        let full = [0.25, 0.5, 0.75, 1.0];
        approx4(apply_crop_uv(full, None), full);
    }

    #[test]
    fn frame_arena_cap_accepts_limit_and_rejects_overflow() {
        assert!(frame_arena_within_cap(ARENA_SANITY_CAP));
        assert!(!frame_arena_within_cap(ARENA_SANITY_CAP + 1));
    }

    // ---- StaleListDetector ----

    #[test]
    fn stale_detector_reused_list_never_warns() {
        // The correct pattern: one list reused every frame → same id forever.
        let mut d = StaleListDetector::default();
        for _ in 0..(StaleListDetector::WARN_AFTER * 3) {
            assert!(!d.observe(42), "reusing one list must never warn");
        }
        assert_eq!(d.fresh_streak, 0);
    }

    #[test]
    fn stale_detector_fresh_list_each_frame_warns_once() {
        // The footgun: a brand-new id every frame (LayerStack::new() in the loop).
        let mut d = StaleListDetector::default();
        let mut warnings = 0;
        for id in 1u64..=(StaleListDetector::WARN_AFTER * 2) as u64 {
            if d.observe(id) {
                warnings += 1;
            }
            // never repeats — every frame is a fresh list
        }
        assert_eq!(warnings, 1, "must warn exactly once, not spam every frame");
    }

    #[test]
    fn stale_detector_warns_exactly_at_threshold() {
        let mut d = StaleListDetector::default();
        // WARN_AFTER-1 fresh frames: not yet.
        for id in 1..StaleListDetector::WARN_AFTER as u64 {
            assert!(!d.observe(id));
        }
        // The WARN_AFTER-th consecutive fresh frame trips it.
        assert!(d.observe(StaleListDetector::WARN_AFTER as u64));
    }

    #[test]
    fn stale_detector_double_buffered_stacks_dont_warn() {
        // Two persistent stacks alternating (a legitimate double-buffer): both ids
        // keep recurring, so neither is ever "never seen". Must not warn.
        let mut d = StaleListDetector::default();
        for i in 0..(StaleListDetector::WARN_AFTER * 4) {
            let id = if i % 2 == 0 { 7 } else { 9 };
            assert!(
                !d.observe(id),
                "alternating persistent stacks must not warn"
            );
        }
    }

    #[test]
    fn stale_detector_occasional_rebuild_does_not_warn() {
        // A reused list that gets rebuilt now and then (e.g. on resize) produces a
        // lone fresh id occasionally — never a long enough streak to trip.
        let mut d = StaleListDetector::default();
        let mut id = 1u64;
        for frame in 0..(StaleListDetector::WARN_AFTER * 5) {
            if frame % 20 == 0 {
                id += 1; // rebuild: new id this frame, then reused for the next 19
            }
            assert!(!d.observe(id), "occasional rebuilds must not warn");
        }
    }

    #[test]
    fn render_stats_warns_about_fragmentation_not_merely_size() {
        let fragmented = RenderStats {
            primitives: 160,
            draw_calls: 140,
            ..RenderStats::default()
        };
        assert_eq!(fragmented.warnings().len(), 1);
        let large_batched = RenderStats {
            primitives: 10_000,
            draw_calls: 20,
            ..RenderStats::default()
        };
        assert!(large_batched.warnings().is_empty());
    }

    #[test]
    fn many_small_sprites_in_textures_of_their_own_are_reported_and_named() {
        let few = RenderStats {
            small_texture_batches: SMALL_TEXTURE_BATCHES_WARN - 1,
            ..RenderStats::default()
        };
        assert!(few.warnings().is_empty(), "a handful costs little");
        let many = RenderStats {
            small_texture_batches: SMALL_TEXTURE_BATCHES_WARN,
            ..RenderStats::default()
        };
        let warnings = many.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("shared atlas"), "{}", warnings[0]);
        let names = ["thumb:a".to_owned(), "thumb:b".to_owned()];
        assert!(pressure_findings(many, &names).ends_with("(among them: thumb:a, thumb:b)"));
        // Other findings don't name the sprites that happened to be drawn.
        let fragmented = RenderStats {
            primitives: 160,
            draw_calls: 140,
            small_texture_batches: 2,
            ..RenderStats::default()
        };
        assert!(!pressure_findings(fragmented, &names).contains("among them"));
    }

    #[test]
    fn pressure_detector_requires_sustained_bad_frames_and_warns_once() {
        let bad = RenderStats {
            primitives: 160,
            draw_calls: 140,
            ..RenderStats::default()
        };
        let mut detector = RenderPressureDetector::default();
        for _ in 1..RenderPressureDetector::WARN_AFTER {
            assert!(!detector.observe(bad));
        }
        assert!(detector.observe(bad));
        assert!(!detector.observe(bad));
    }

    // ---- DrawList / LayerStack identity (backs the detector) ----

    #[test]
    fn distinct_draw_lists_have_distinct_ids() {
        let a = DrawList::new();
        let b = DrawList::new();
        assert_ne!(a.id(), b.id(), "each DrawList must get a unique id");
    }

    #[test]
    fn clear_preserves_id() {
        let mut a = DrawList::new();
        let before = a.id();
        a.clear();
        assert_eq!(a.id(), before, "clear() must keep the same list identity");
    }

    #[test]
    fn layer_stack_base_id_matches_base_list() {
        let s = LayerStack::new();
        assert_eq!(s.base_id(), s.base().id());
        // A second stack is a distinct base list.
        let s2 = LayerStack::new();
        assert_ne!(s.base_id(), s2.base_id());
    }

    /// Replicate the vertex shaders' `vec4(pos, 0, 1) * view_proj` (row-vector ×
    /// matrix) to get the NDC of a pixel-space point under a given ortho.
    fn project(m: &[[f32; 4]; 4], x: f32, y: f32) -> (f32, f32) {
        let v = [x, y, 0.0, 1.0];
        let mut out = [0.0f32; 4];
        for (j, o) in out.iter_mut().enumerate() {
            *o = v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j] + v[3] * m[3][j];
        }
        (out[0], out[1])
    }

    #[test]
    fn ortho_maps_logical_corners_to_ndc() {
        let m = ortho_matrix([0.0, 0.0], 800.0, 600.0);
        // Top-left logical origin → NDC top-left (-1, +1); Y is down in pixels.
        let (x0, y0) = project(&m, 0.0, 0.0);
        assert!(
            (x0 + 1.0).abs() < 1e-5 && (y0 - 1.0).abs() < 1e-5,
            "{x0},{y0}"
        );
        // Bottom-right logical extent → NDC (+1, -1).
        let (x1, y1) = project(&m, 800.0, 600.0);
        assert!(
            (x1 - 1.0).abs() < 1e-5 && (y1 + 1.0).abs() < 1e-5,
            "{x1},{y1}"
        );
        // Center → origin.
        let (xc, yc) = project(&m, 400.0, 300.0);
        assert!(xc.abs() < 1e-5 && yc.abs() < 1e-5, "{xc},{yc}");
    }

    #[test]
    fn ortho_with_an_origin_maps_that_window_onto_the_target() {
        let m = ortho_matrix([100.0, 5000.0], 800.0, 600.0);
        // The origin is the target's top-left corner…
        let (x0, y0) = project(&m, 100.0, 5000.0);
        assert!(
            (x0 + 1.0).abs() < 1e-5 && (y0 - 1.0).abs() < 1e-5,
            "{x0},{y0}"
        );
        // …the far corner of the window its bottom-right…
        let (x1, y1) = project(&m, 900.0, 5600.0);
        assert!(
            (x1 - 1.0).abs() < 1e-5 && (y1 + 1.0).abs() < 1e-5,
            "{x1},{y1}"
        );
        // …and canvas (0, 0) now lies off the target, up and to the left.
        let (xa, ya) = project(&m, 0.0, 0.0);
        assert!(xa < -1.0 && ya > 1.0, "{xa},{ya}");
    }

    #[test]
    fn dpi_scale_is_logical_size_invariant() {
        // prepare_pass builds ortho from (physical / scale). Two (physical,
        // scale) pairs with the SAME logical size must project a logical point
        // identically — the scale factor only changes which framebuffer the
        // identical logical UI rasterizes onto.
        let logical = |w: f32, s: f32| ortho_matrix([0.0, 0.0], w / s, w / s); // square for brevity
        let a = logical(800.0, 1.0); // physical 800, scale 1 → logical 800
        let b = logical(1600.0, 2.0); // physical 1600, scale 2 → logical 800
        for &(px, py) in &[(0.0, 0.0), (250.0, 700.0), (800.0, 800.0)] {
            let (ax, ay) = project(&a, px, py);
            let (bx, by) = project(&b, px, py);
            assert!(
                (ax - bx).abs() < 1e-6 && (ay - by).abs() < 1e-6,
                "logical point ({px},{py}) projected differently: a=({ax},{ay}) b=({bx},{by})"
            );
        }
    }

    #[test]
    fn crop_uv_maps_into_region() {
        // Sprite occupies [0.2,0.4]..[0.6,0.8] of the atlas. Crop its centre
        // quarter [0.25,0.25]..[0.75,0.75] -> a centred sub-rect of the region.
        let full = [0.2, 0.4, 0.6, 0.8];
        let got = apply_crop_uv(full, Some([0.25, 0.25, 0.75, 0.75]));
        // span_u = 0.4, span_v = 0.4
        approx4(got, [0.3, 0.5, 0.5, 0.7]);
    }

    #[test]
    fn icon_instance_packs_corners_uv_clip() {
        // Corners in TL, TR, BR, BL order (as transform_rect_corners yields).
        let corners = [[10.0, 20.0], [110.0, 20.0], [110.0, 70.0], [10.0, 70.0]];
        let inst = build_icon_instance(
            corners,
            [0.1, 0.2, 0.3, 0.4],
            [1.0, 0.5, 0.25, 1.0],
            Some([1.0, 2.0, 3.0, 4.0]),
            false,
            [0.0, 0.0],
        );
        assert_eq!(inst.c_tl_tr, [10.0, 20.0, 110.0, 20.0]);
        assert_eq!(inst.c_br_bl, [110.0, 70.0, 10.0, 70.0]);
        assert_eq!(inst.uv_rect, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(inst.tint, [1.0, 0.5, 0.25, 1.0]);
        assert_eq!(inst.clip, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(inst.flags[0], 1.0); // clip enabled
    }

    #[test]
    fn icon_instance_clip_none_disables() {
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let inst = build_icon_instance(
            corners,
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            None,
            false,
            [0.0, 0.0],
        );
        assert_eq!(inst.flags[0], 0.0); // clip disabled
        assert_eq!(inst.clip, [0.0; 4]);
    }

    #[test]
    fn nine_slice_instance_packs_affine_and_size() {
        let region = AtlasRegion {
            x: 0,
            y: 0,
            w: 32,
            h: 32,
        };
        // Translation-only transform: linear part is identity, translate carries
        // the offset. clip None => clip_enabled 0.
        let t = crate::affine::Affine2::translation(7.0, 11.0);
        let inst = build_nine_slice_instance(
            5.0,
            6.0,
            100.0,
            80.0,
            &t,
            [1.0; 4],
            None,
            region,
            [4, 4, 4, 4],
            64,
            64,
        );
        assert_eq!(inst.lin, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(inst.translate, [7.0, 11.0, 0.0, 0.0]);
        assert_eq!(inst.origin_size, [5.0, 6.0, 100.0, 80.0]);
        assert_eq!(inst.border, [4.0, 4.0, 4.0, 4.0]);
    }

    #[test]
    fn nine_slice_instance_uvs_match_border() {
        let region = AtlasRegion {
            x: 0,
            y: 0,
            w: 32,
            h: 32,
        };
        let identity = crate::affine::Affine2::IDENTITY;
        let inst = build_nine_slice_instance(
            0.0,
            0.0,
            100.0,
            80.0,
            &identity,
            [1.0; 4],
            None,
            region,
            [8, 8, 8, 8],
            64,
            64,
        );
        // Outer UV origin = region origin (0,0); far edge = 32/64 = 0.5.
        approx4(inst.uv_outer, [0.0, 0.0, 0.5, 0.5]);
        // Inner seams: left/top = 8/64 = 0.125; right/bottom = (32-8)/64 = 0.375.
        approx4(inst.uv_inner, [0.125, 0.125, 0.375, 0.375]);
    }

    #[test]
    fn nine_slice_instance_records_affine_rotation_and_clip() {
        let region = AtlasRegion {
            x: 16,
            y: 16,
            w: 16,
            h: 16,
        };
        // A rotation injects nonzero off-diagonals — proving the full affine is
        // baked in (no axis-aligned fallback needed for nine-slices).
        let r = crate::affine::Affine2::rotation(std::f32::consts::FRAC_PI_2);
        let inst = build_nine_slice_instance(
            0.0,
            0.0,
            10.0,
            10.0,
            &r,
            [1.0; 4],
            Some([1.0, 2.0, 3.0, 4.0]),
            region,
            [2, 2, 2, 2],
            64,
            64,
        );
        assert!(inst.lin[1].abs() > 1e-3 || inst.lin[2].abs() > 1e-3);
        assert_eq!(inst.translate[2], 1.0); // clip_enabled
        assert_eq!(inst.clip, [1.0, 2.0, 3.0, 4.0]);
    }
}
