//! MSDF text rendering.
//!
//! Shaping and layout go through **cosmic-text** (`crate::shaping`), whose
//! layouts [`TextMeasurer`] and [`TextRenderer`] share through the font system,
//! so a block is shaped once for measuring and drawing. The glyphs are not
//! rasterised by cosmic-text: each is rendered from a **multi-channel
//! signed distance field** ([`crate::render::MsdfGlyphAtlas`]). This gives crisp
//! fill at any size and is the foundation for outline/shadow/glow effects
//! (Teardown `UiTextOutline`/`UiTextShadow` parity) added in later phases.
//!
//! [`TextRenderer`] is self-contained: it owns the MSDF atlas, a linear-sampled
//! `Rgba8Unorm` (NOT sRGB — the texels are distances, not colors) GPU texture, the
//! MSDF pipeline, and its own ortho uniform. [`TextRenderer::render`] lays out each
//! [`TextBlock`], emits one quad per glyph, lazily generates any unseen glyph into
//! the atlas, uploads the atlas if it changed, and draws — all in one call.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use unicode_segmentation::UnicodeSegmentation;

use crate::layout::Rect;
#[cfg(feature = "phosphor-icons")]
use crate::render::{DEFAULT_PX_RANGE, IconGlyph, PhosphorIcon, icon_font_snapshot};
use crate::render::{GlyphTile, MsdfGlyphAtlas, UniformArena, ortho_matrix};
use crate::shaping::{LayoutSpec, ShapedGlyph, SharedFontSystem};
#[cfg(feature = "phosphor-icons")]
use crate::widgets::IconMsdf;

use cosmic_text::{
    Align as CosmicAlign, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style,
    Weight, Wrap, fontdb,
};

const MSDF_SHADER: &str = include_str!("render/ui_msdf.wgsl");

/// Reference EM size (pixels) the icon distance fields are generated at. Higher
/// than the text default (icons are square and may render large in galleries)
/// for crisp scale-up headroom, at a modest atlas cost for the small curated set.
#[cfg(feature = "phosphor-icons")]
const ICON_REF_PX: f32 = 64.0;

/// Size of the ortho uniform this renderer writes — one dynamic-offset arena slot.
const UNIFORM_SIZE: u64 = std::mem::size_of::<[[f32; 4]; 4]>() as u64;

/// Shared handle to the font system.
///
/// Both `TextRenderer` and `TextMeasurer` hold the same handle, so measured text
/// (used for layout) matches rendered glyphs (used for output) — including any
/// custom fonts loaded into the system later — and a block measured and then
/// drawn is shaped once: they share its layout (see [`SharedFontSystem`]).
pub type FontSystemHandle = Arc<Mutex<SharedFontSystem>>;

/// Create a new shared `FontSystem` handle.
///
/// `FontSystem::new()` loads the host's system fonts (used for broad script /
/// emoji fallback). With the default `bundled-font` feature on, IBM Plex Sans
/// is embedded and registered as the default sans-serif; IBM Plex Mono is
/// registered as the companion technical face (see [`register_bundled_fonts`]
/// and [`bundled_mono_font`]). Thus unstyled text renders identically on every
/// machine rather than depending on which system font happens to be installed.
pub fn shared_font_system() -> FontSystemHandle {
    let handle = Arc::new(Mutex::new(SharedFontSystem::new(FontSystem::new())));
    register_bundled_fonts(&handle);
    handle
}

/// Embed IBM Plex Sans (regular/bold/italic/bold-italic) into `fs` and register
/// it as the default sans-serif, so `Family::SansSerif` — and any [`TextBlock`]
/// without an explicit font — resolves to it deterministically on every machine
/// instead of an OS-dependent system font. The bold/italic faces share the family
/// name, so [`TextBlock::bold`] / [`TextBlock::italic`] select them by default.
///
/// This also registers IBM Plex Mono's matching four faces, returned by
/// [`bundled_mono_font`] for applications' technical UI (coordinates, shortcuts,
/// status readouts, and other data-dense text).
///
/// Returns the IBM Plex Sans family [`FontHandle`] (also usable directly via
/// [`TextBlock::with_font`]). With the `bundled-font` feature **disabled** this is
/// a no-op that returns `None` and the default stays the system sans-serif.
/// [`shared_font_system`] calls this for you; call it yourself only when you
/// construct a `FontSystem` by other means.
pub fn register_bundled_fonts(fs: &FontSystemHandle) -> Option<FontHandle> {
    #[cfg(feature = "bundled-font")]
    {
        let sans = load_font_family(
            fs,
            IBM_PLEX_SANS_REGULAR_TTF,
            IBM_PLEX_SANS_BOLD_TTF,
            IBM_PLEX_SANS_ITALIC_TTF,
            IBM_PLEX_SANS_BOLD_ITALIC_TTF,
        )
        .ok()?;
        let _mono = load_font_family(
            fs,
            IBM_PLEX_MONO_REGULAR_TTF,
            IBM_PLEX_MONO_BOLD_TTF,
            IBM_PLEX_MONO_ITALIC_TTF,
            IBM_PLEX_MONO_BOLD_ITALIC_TTF,
        )
        .ok()?;
        {
            let mut guard = fs.lock().expect("FontSystem poisoned");
            guard
                .db_mut()
                .set_sans_serif_family(sans.family().to_string());
        }
        Some(sans)
    }
    #[cfg(not(feature = "bundled-font"))]
    {
        let _ = fs;
        None
    }
}

/// Returns the bundled IBM Plex Mono family handle after registering the bundled
/// faces into `fs`. Use it with [`TextBlock::with_font`] for technical UI such as
/// coordinates, shortcuts, status readouts, or compact all-caps labels.
///
/// With the `bundled-font` feature disabled this returns `None` and does not load
/// a font. [`shared_font_system`] already registers the family, so there this
/// only returns the handle; the faces are loaded only into a font system that
/// doesn't have them yet. The default [`Theme::mono_font`](crate::Theme::mono_font)
/// names the same family.
pub fn bundled_mono_font(fs: &FontSystemHandle) -> Option<FontHandle> {
    #[cfg(feature = "bundled-font")]
    {
        let registered = {
            let guard = fs.lock().expect("FontSystem poisoned");
            guard.db().faces().any(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name == BUNDLED_MONO_FAMILY)
            })
        };
        if registered {
            return Some(FontHandle(BUNDLED_MONO_FAMILY.to_string()));
        }
        load_font_family(
            fs,
            IBM_PLEX_MONO_REGULAR_TTF,
            IBM_PLEX_MONO_BOLD_TTF,
            IBM_PLEX_MONO_ITALIC_TTF,
            IBM_PLEX_MONO_BOLD_ITALIC_TTF,
        )
        .ok()
    }
    #[cfg(not(feature = "bundled-font"))]
    {
        let _ = fs;
        None
    }
}

/// Convert gameui's letter spacing (pixels) to cosmic-text's (em of the font size).
pub(crate) fn letter_spacing_em(pixels: f32, font_size: f32) -> f32 {
    if font_size > 0.0 {
        pixels / font_size
    } else {
        0.0
    }
}

/// Family name of the bundled mono faces (IBM Plex Mono).
#[cfg(feature = "bundled-font")]
pub(crate) const BUNDLED_MONO_FAMILY: &str = "IBM Plex Mono";

#[cfg(feature = "bundled-font")]
const IBM_PLEX_SANS_REGULAR_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_SANS_BOLD_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Bold.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_SANS_ITALIC_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Italic.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_SANS_BOLD_ITALIC_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-BoldItalic.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_MONO_REGULAR_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-Regular.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_MONO_BOLD_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-Bold.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_MONO_ITALIC_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-Italic.ttf");
#[cfg(feature = "bundled-font")]
const IBM_PLEX_MONO_BOLD_ITALIC_TTF: &[u8] =
    include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-BoldItalic.ttf");

#[cfg(feature = "bundled-font")]
fn load_font_family(
    fs: &FontSystemHandle,
    regular: &'static [u8],
    bold: &'static [u8],
    italic: &'static [u8],
    bold_italic: &'static [u8],
) -> Result<FontHandle, String> {
    let regular = load_font_bytes(fs, regular)?;
    let _ = load_font_bytes(fs, bold)?;
    let _ = load_font_bytes(fs, italic)?;
    let _ = load_font_bytes(fs, bold_italic)?;
    Ok(regular)
}

/// Handle to a font loaded into the shared [`FontSystem`], identified by its
/// family name.
///
/// cosmic-text's shaping selects fonts by family name only (`Family::Name`), so
/// a handle is just the family string. Obtain one from [`load_font_file`] /
/// [`load_font_bytes`] and pass it to [`TextBlock::with_font`] to shape a block
/// in that font. If two faces share a family name, the most recently loaded one
/// wins.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontHandle(pub String);

impl FontHandle {
    /// The font's family name (the cosmic-text selector).
    pub fn family(&self) -> &str {
        &self.0
    }
}

/// Horizontal alignment of multi-line text within its `max_width` layout box.
///
/// Alignment is relative to [`TextBlock::max_width`]; `Center`/`Right` only
/// produce a visible shift when `max_width` is wider than the longest line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlign {
    /// Lines flush to the **reading start** of the box — the left edge for
    /// left-to-right text, the right edge for right-to-left text (default).
    /// Follows the block's resolved [base direction](TextBlock::direction).
    #[default]
    Start,
    /// Lines are centered within `max_width`.
    Center,
    /// Lines flush to the **reading end** of the box — the right edge for
    /// left-to-right text, the left edge for right-to-left text. The
    /// direction-relative mirror of [`Start`](TextAlign::Start).
    End,
    /// Lines flush to the left edge of `max_width`, regardless of text
    /// direction (absolute).
    Left,
    /// Lines flush to the right edge of `max_width`, regardless of text
    /// direction (absolute).
    Right,
}

/// How a [`TextBlock`] breaks lines when its content is wider than
/// [`max_width`](TextBlock::max_width).
///
/// The default, [`WordOrGlyph`](WrapMode::WordOrGlyph), is exactly the implicit
/// behaviour every block had before this knob existed (cosmic-text's `Buffer`
/// default), so leaving it unset changes nothing. Use [`None`](WrapMode::None)
/// for single-line fields that should overflow (and be clipped) rather than
/// wrap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WrapMode {
    /// Never wrap; the text stays on one line and overflows `max_width`
    /// (combine with [`TextBlock::with_clip`] to hide the overflow).
    None,
    /// Break between words; a single word too long to fit still overflows.
    Word,
    /// Break anywhere between glyphs.
    Glyph,
    /// Break between words, falling back to glyph breaks for a word too long to
    /// fit on a line by itself. Matches the pre-existing implicit behaviour.
    #[default]
    WordOrGlyph,
}

impl From<WrapMode> for Wrap {
    fn from(mode: WrapMode) -> Self {
        match mode {
            WrapMode::None => Wrap::None,
            WrapMode::Word => Wrap::Word,
            WrapMode::Glyph => Wrap::Glyph,
            WrapMode::WordOrGlyph => Wrap::WordOrGlyph,
        }
    }
}

/// Base paragraph direction for a [`TextBlock`] / text field.
///
/// Bidi *reordering* of mixed-script runs is automatic in every case (cosmic-text
/// runs the Unicode bidi algorithm during shaping); this only fixes the **base**
/// direction — which edge lines start from, and how direction-neutral content
/// (digits, punctuation, an empty string, a leading Latin word in an otherwise
/// RTL UI) resolves. The default, [`Auto`](TextDirection::Auto), matches the
/// pre-existing behaviour, so leaving it unset changes nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextDirection {
    /// Auto-detect from the first strong character of each line (cosmic-text's
    /// default). Neutral-only content resolves left-to-right.
    #[default]
    Auto,
    /// Force a left-to-right base direction.
    Ltr,
    /// Force a right-to-left base direction.
    Rtl,
}

/// The zero-width strong directional mark that pins the paragraph base level when
/// prepended to a shaped string, or `""` for [`Auto`](TextDirection::Auto).
///
/// cosmic-text 0.12 exposes no API to set the base/paragraph direction — it always
/// auto-detects from the first strong character (`BidiInfo::new(line, None)`). The
/// only lever is to make that first strong character ours: U+200E LEFT-TO-RIGHT
/// MARK / U+200F RIGHT-TO-LEFT MARK. Both are zero-width and non-joining, so they
/// set the base level without altering shaping of the real runs that follow.
pub(crate) fn direction_prefix(dir: TextDirection) -> &'static str {
    match dir {
        TextDirection::Auto => "",
        TextDirection::Ltr => "\u{200E}", // LRM
        TextDirection::Rtl => "\u{200F}", // RLM
    }
}

/// Load a font from a TTF/OTF file into the shared `FontSystem`, returning a
/// [`FontHandle`] that selects it for [`TextBlock::with_font`].
///
/// After loading, drop any cached measurements ([`TextMeasurer::clear_cache`])
/// if the same family name replaced an earlier face.
pub fn load_font_file(
    fs: &FontSystemHandle,
    path: impl AsRef<Path>,
) -> std::io::Result<FontHandle> {
    let bytes = std::fs::read(path)?;
    load_font_bytes(fs, &bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Load a font from in-memory TTF/OTF bytes into the shared `FontSystem`.
///
/// Returns the family-name [`FontHandle`], or an error string if the bytes do
/// not parse into a face that exposes a family name. The family name is read
/// from the same `fontdb` that cosmic-text shapes against, so the returned
/// handle is guaranteed to resolve.
pub fn load_font_bytes(fs: &FontSystemHandle, bytes: &[u8]) -> Result<FontHandle, String> {
    let mut guard = fs.lock().expect("FontSystem poisoned");
    let db = guard.db_mut();
    let before = db.len();
    db.load_font_data(bytes.to_vec());
    // `load_font_data` appends one face per font in the data (one for a plain
    // TTF/OTF). Take the first newly added face's primary family name.
    db.faces()
        .nth(before)
        .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
        .map(FontHandle)
        .ok_or_else(|| "loaded font exposes no family name".to_string())
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct MsdfVertex {
    position: [f32; 2],
    uv: [f32; 2],
    fill: [f32; 4],
    clip: [f32; 4],
    clip_enabled: f32,
    /// Distance-ramp width of the field in atlas texels (constant per atlas).
    px_range: f32,
    /// Outline/glow color composited under the fill (a == 0 disables).
    outline: [f32; 4],
    /// Outline width in screen px (glyph grown outward by this much).
    outline_width: f32,
    /// Extra AA spread in screen px (soft shadows / glow); 0 = crisp.
    softness: f32,
}

const MSDF_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32,
    5 => Float32,
    6 => Float32x4,
    7 => Float32,
    8 => Float32,
];

/// One MSDF run (text or icons) uploaded for drawing: its vertices sit in
/// `vbo` at `offset`, and the bind groups it samples were captured at upload.
/// Drawn with [`TextRenderer::draw_prepared`] inside a render pass the caller
/// owns, so a whole paint stream can share one pass.
pub(crate) struct PreparedMsdf {
    vbo: wgpu::Buffer,
    offset: u64,
    vertices: u32,
    atlas: wgpu::BindGroup,
    uniform: wgpu::BindGroup,
    uniform_offset: u32,
}

/// GPU text renderer: owns the MSDF glyph atlas (and optional Phosphor icon
/// atlas), the shaping font system, and the wgpu pipeline/buffers that draw
/// shaped glyph quads. One instance is created per [`crate::UiRenderer`].
pub struct TextRenderer {
    font_system: FontSystemHandle,

    // MSDF glyph atlas (CPU source of truth) + its GPU mirror.
    atlas: MsdfGlyphAtlas,
    atlas_bgl: wgpu::BindGroupLayout,
    glyph_gpu: MsdfTextureGpu,

    // Phosphor icon atlas (separate ref_px, never evicted) + its GPU mirror.
    // Shares the atlas bind-group layout and the MSDF pipeline with text; only
    // the bound texture (and vertex slice) differ.
    #[cfg(feature = "phosphor-icons")]
    icon_atlas: MsdfGlyphAtlas,
    #[cfg(feature = "phosphor-icons")]
    icon_gpu: MsdfTextureGpu,

    // Ortho projection (owned, sized from `resize`), one slot per pass: the
    // projection is per pass and `Queue::write_buffer` lands at submit, so passes
    // sharing a slot would all draw with the last pass's matrix. `begin_frame`
    // resets the arena; see `render::uniform_arena`.
    uniform: UniformArena,
    /// The slot `resize` handed this pass. Bound as the dynamic offset.
    uniform_offset: u64,
    /// Logical canvas point projected onto the target's top-left corner; see
    /// [`set_view_origin`](Self::set_view_origin).
    view_origin: [f32; 2],

    pipeline: wgpu::RenderPipeline,

    vbo: wgpu::Buffer,
    vbo_capacity: u64,
    /// Bytes already used in `vbo` this frame. Each `render` pass writes at this
    /// offset and advances it, so multiple text passes within one submit (e.g.
    /// base layer + tooltip layer) occupy disjoint regions instead of all
    /// aliasing offset 0 and reading the last-written data at draw time. Reset
    /// to 0 by [`begin_frame`](Self::begin_frame) each frame.
    vbo_offset: u64,
    /// Replaced buffers retained for the renderer's lifetime. Ordered painting
    /// can grow the VBO while encoded or submitted passes still reference its
    /// predecessor. Growth is geometric, so this remains a small bounded set and
    /// avoids backend-specific completion polling before releasing resources.
    retired_vbos: Vec<wgpu::Buffer>,

    /// Stable per-font keys for the atlas, assigned on first sighting. Decouples
    /// the atlas from cosmic-text's `fontdb::ID`.
    font_keys: HashMap<fontdb::ID, u64>,
    next_font_key: u64,

    width: u32,
    height: u32,
}

impl TextRenderer {
    /// Construct a `TextRenderer` with a fresh shared `FontSystem` (loads system +
    /// bundled fonts). Use [`with_font_system`](Self::with_font_system) to share an
    /// existing one. `format` is the render target's color format.
    ///
    /// Text colours are sRGB-encoded (see [`crate::color`]) and blend in the
    /// target's storage space, so a non-sRGB `format` gives browser-matching
    /// text. [`UiRenderer`](crate::UiRenderer) arranges that for any host
    /// target; a standalone `TextRenderer` on an `*Srgb` target blends in
    /// linear light instead.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = shared_font_system();
        Self::with_font_system(device, queue, format, font_system)
    }

    /// Construct a `TextRenderer` reusing an existing shared `FontSystem`.
    pub fn with_font_system(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: FontSystemHandle,
    ) -> Self {
        let atlas = MsdfGlyphAtlas::new();

        // Uniform (group 0): ortho projection, matching the main UI pipelines.
        // One arena slot per pass, for the same reason as the vertex buffer below:
        // several passes share one submit, and a shared slot would let a later
        // pass's matrix reach the GPU first.
        let uniform = UniformArena::new(
            device,
            "msdf text uniform",
            UNIFORM_SIZE,
            wgpu::ShaderStages::VERTEX,
        );
        let uniform_bgl = uniform.layout();

        // Atlas texture (group 1): linear filtering, linear (non-sRGB) format.
        // The layout is shared by the text and icon atlas mirrors and the pipeline.
        let atlas_bgl = create_msdf_atlas_bgl(device);
        let glyph_gpu = MsdfTextureGpu::new(device, &atlas_bgl, &atlas);

        // Icon atlas: same MSDF machinery, generated at a higher reference size
        // (icons render anywhere from ~16px steppers to ~48px gallery cells) and
        // the standard distance ramp so the shared shader's AA math is unchanged.
        #[cfg(feature = "phosphor-icons")]
        let icon_atlas = MsdfGlyphAtlas::with_params(ICON_REF_PX, DEFAULT_PX_RANGE);
        #[cfg(feature = "phosphor-icons")]
        let icon_gpu = MsdfTextureGpu::new(device, &atlas_bgl, &icon_atlas);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("msdf text shader"),
            source: wgpu::ShaderSource::Wgsl(MSDF_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("msdf text pipeline layout"),
            bind_group_layouts: &[uniform_bgl, &atlas_bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("msdf text pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_msdf"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MsdfVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MSDF_VERTEX_ATTRIBS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_msdf"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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

        let vbo_capacity = (4096 * std::mem::size_of::<MsdfVertex>()) as u64;
        let vbo = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("msdf text vbo"),
            size: vbo_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            font_system,
            atlas,
            atlas_bgl,
            glyph_gpu,
            #[cfg(feature = "phosphor-icons")]
            icon_atlas,
            #[cfg(feature = "phosphor-icons")]
            icon_gpu,
            uniform,
            uniform_offset: 0,
            view_origin: [0.0, 0.0],
            pipeline,
            vbo,
            vbo_capacity,
            vbo_offset: 0,
            retired_vbos: Vec::new(),
            font_keys: HashMap::new(),
            next_font_key: 0,
            width: 1,
            height: 1,
        }
    }

    /// Get a clone of the shared font system handle.
    ///
    /// Use this to construct a `DrawList` / `TextMeasurer` that shares font state with
    /// this renderer.
    pub fn font_system_handle(&self) -> FontSystemHandle {
        Arc::clone(&self.font_system)
    }

    /// Update the viewport size used to build the ortho projection, and give this
    /// pass its own uniform slot.
    ///
    /// `width`/`height` are the **physical** render-target pixels; `scale_factor`
    /// is the logical → physical ratio (e.g. 2.0 on Retina). The ortho matrix is
    /// built from the *logical* dimensions (`physical / scale`) so text lands at
    /// the same coordinates as the geometry pipeline, which also projects in
    /// logical space.
    ///
    /// Call this once per pass, before that pass's [`render`](Self::render) /
    /// [`render_icons`](Self::render_icons): the projection is per pass, because two
    /// passes submitted together can target differently sized views and a shared
    /// uniform slot would hand both of them the last matrix written (see
    /// [`uniform_arena`](crate::render::uniform_arena)). `begin_frame` — called once
    /// per frame by [`UiRenderer::begin_frame`](crate::UiRenderer::begin_frame) —
    /// releases the slots for reuse.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) {
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        // Store the logical dimensions — ortho_matrix needs these, not physical.
        self.width = ((width as f32 / scale) as u32).max(1);
        self.height = ((height as f32 / scale) as u32).max(1);
        let (slot, _grew) = self.uniform.allocate(device);
        queue.write_buffer(
            self.uniform.buffer(),
            slot,
            bytemuck::cast_slice(&[ortho_matrix(
                self.view_origin,
                self.width as f32,
                self.height as f32,
            )]),
        );
        self.uniform_offset = slot;
    }

    /// Project the logical point `(x, y)` onto the target's top-left corner
    /// from the next [`resize`](Self::resize) on, so the pass draws a window of
    /// a larger canvas. [`UiRenderer::set_view_origin`](crate::UiRenderer::set_view_origin)
    /// calls this; set it there rather than here.
    pub fn set_view_origin(&mut self, x: f32, y: f32) {
        self.view_origin = [x, y];
    }

    /// Reset this frame's bump cursors — the vertex buffer and the uniform slots.
    /// Called once per frame (from [`UiRenderer::begin_frame`](crate::UiRenderer::begin_frame)),
    /// *not* once per pass: passes within one submission must keep their own regions,
    /// and only the frame boundary makes those regions reusable.
    pub fn begin_frame(&mut self) {
        self.vbo_offset = 0;
        self.uniform.reset();
        self.uniform_offset = 0;
    }

    /// Bytes of per-frame GPU scratch this renderer is holding: vertex data plus
    /// uniform slots. Part of `UiRenderer`'s frame-arena accounting.
    pub(crate) fn frame_arena_bytes(&self) -> u64 {
        self.vbo_offset + self.uniform.bytes_used()
    }

    /// Drop the text layouts and font metrics shared with the measurers,
    /// forcing every block to be shaped again when next drawn
    /// ([`SharedFontSystem::clear_caches`]). Loading a font through this crate
    /// already does this.
    pub fn clear_shape_cache(&mut self) {
        self.font_system
            .lock()
            .expect("FontSystem poisoned")
            .clear_caches();
    }

    /// Measure text using cosmic-text's shaping/layout path without touching GPU
    /// state: the default face on one line, as
    /// [`TextMeasurer::measure`] with no `max_width` reports it.
    pub fn measure(&mut self, text: &str, font_size: f32) -> (f32, f32) {
        let spec = LayoutSpec::plain(font_size, None);
        if text.is_empty() {
            return (0.0, spec.line_height);
        }
        let mut shared = self.font_system.lock().expect("FontSystem poisoned");
        shared.layout(&spec, text).size
    }

    /// Pre-generate the printable-ASCII glyph set into the atlas so the first
    /// frame that displays them doesn't hitch. Call once after construction.
    pub fn prewarm_ascii(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let ascii: String = (0x20u8..=0x7e).map(|c| c as char).collect();
        // Clone the handle so the guard borrows a local, not `self` (frees `self`
        // for `self.font_key`/`self.atlas`).
        let fs_handle = Arc::clone(&self.font_system);
        let mut shared = fs_handle.lock().expect("FontSystem poisoned");
        let fs = shared.font_system();
        let mut buffer = Buffer::new(fs, Metrics::new(self.atlas.ref_px(), self.atlas.ref_px()));
        buffer.set_text(
            &ascii,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(fs, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let font_key = self.font_key(glyph.font_id);
                if let Some(font) = fs.get_font(glyph.font_id, glyph.font_weight) {
                    self.atlas.glyph(font_key, glyph.glyph_id, font.data());
                }
            }
        }
        drop(shared);
        self.upload_atlas(device, queue);
    }

    fn font_key(&mut self, id: fontdb::ID) -> u64 {
        resolve_font_key(&mut self.font_keys, &mut self.next_font_key, id)
    }

    /// Pre-generate every curated [`PhosphorIcon`] into the icon atlas so the
    /// first frame that shows an icon doesn't hitch. This is intentionally opt-in;
    /// renderer construction otherwise generates icon glyphs lazily.
    #[cfg(feature = "phosphor-icons")]
    pub fn prewarm_icons(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let glyphs: Vec<IconGlyph> = PhosphorIcon::ALL.iter().filter_map(|i| i.glyph()).collect();
        self.prewarm_icon_glyphs(device, queue, &glyphs);
    }

    /// Pre-generate an explicit set of icon glyphs — the route for an
    /// application font registered with
    /// [`register_icon_font`](crate::render::register_icon_font), whose glyph set
    /// the library can't enumerate for itself.
    ///
    /// Unresolvable glyphs (unregistered font, or a font id past the end of the
    /// registry) are skipped rather than panicking.
    #[cfg(feature = "phosphor-icons")]
    pub fn prewarm_icon_glyphs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: &[IconGlyph],
    ) {
        let fonts = icon_font_snapshot();
        for g in glyphs {
            let Some(data) = fonts.get(g.font.index() as usize) else {
                continue;
            };
            self.icon_atlas
                .glyph(g.font.index() as u64, g.glyph_id, data);
        }
        self.icon_gpu
            .upload(device, queue, &self.atlas_bgl, &mut self.icon_atlas);
    }

    /// Resolve every glyph needed by an ordered paint stream before encoding its
    /// first text pass. Text runs are rendered separately to preserve submission
    /// order; without this preflight, a later run can grow the atlas after earlier
    /// passes have baked UVs against its old dimensions.
    pub(crate) fn prepare_texts(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texts: &[TextBlock],
    ) {
        if texts.is_empty() {
            return;
        }
        let _ = self.build_vertices(texts);
        self.upload_atlas(device, queue);
    }

    /// Resolve every icon glyph needed by an ordered paint stream before its first
    /// icon pass, for the same atlas-stability reason as [`prepare_texts`](Self::prepare_texts).
    #[cfg(feature = "phosphor-icons")]
    pub(crate) fn prepare_icons(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        icons: &[IconMsdf],
    ) {
        let fonts = icon_font_snapshot();
        for icon in icons {
            let Some(data) = fonts.get(icon.glyph.font.index() as usize) else {
                continue;
            };
            self.icon_atlas
                .glyph(icon.glyph.font.index() as u64, icon.glyph.glyph_id, data);
        }
        self.icon_gpu
            .upload(device, queue, &self.atlas_bgl, &mut self.icon_atlas);
    }

    /// Prepare and render a batch of MSDF icons in a pass of their own. Mirrors
    /// [`render`](Self::render) but builds each quad by fitting-and-centering the
    /// icon's glyph tile into its rect (see `fit_centered`), and binds the icon
    /// atlas instead of the glyph atlas. Shares the pipeline, ortho uniform, and
    /// vertex buffer (bump-allocated) with text.
    #[cfg(feature = "phosphor-icons")]
    pub fn render_icons(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        icons: &[IconMsdf],
    ) {
        if let Some(run) = self.upload_icons(device, queue, icons) {
            let mut pass = crate::render::load_pass(encoder, view, "msdf icon pass");
            self.draw_prepared(&mut pass, &run);
        }
    }

    /// Build and upload a batch of MSDF icons, ready for
    /// [`draw_prepared`](Self::draw_prepared). `None` when nothing draws.
    #[cfg(feature = "phosphor-icons")]
    pub(crate) fn upload_icons(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        icons: &[IconMsdf],
    ) -> Option<PreparedMsdf> {
        if icons.is_empty() {
            return None;
        }

        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_icon_render").entered();

        // One lock acquisition for the whole batch; the bytes are `'static`, so
        // MSDF generation below runs with the registry unlocked.
        let fonts = icon_font_snapshot();
        let px_range = self.icon_atlas.px_range();
        let mut verts: Vec<MsdfVertex> = Vec::with_capacity(icons.len() * 6);
        for icon in icons {
            // An icon from an unregistered font simply doesn't draw.
            let Some(data) = fonts.get(icon.glyph.font.index() as usize) else {
                continue;
            };
            let Some(tile) =
                self.icon_atlas
                    .glyph(icon.glyph.font.index() as u64, icon.glyph.glyph_id, data)
            else {
                continue;
            };
            push_icon_quad(
                &mut verts,
                &tile,
                icon,
                self.icon_atlas.width(),
                self.icon_atlas.height(),
                px_range,
            );
        }

        // Glyph generation may have dirtied / grown the icon atlas — upload first.
        self.icon_gpu
            .upload(device, queue, &self.atlas_bgl, &mut self.icon_atlas);

        if verts.is_empty() {
            return None;
        }

        let atlas = self.icon_gpu.bind_group.clone();
        Some(self.upload_vertices(device, queue, &verts, atlas))
    }

    /// Bump-allocate `verts` into this frame's vertex buffer (so the run doesn't
    /// alias earlier runs in the same submit, which would all read the last
    /// write at draw time) and capture what drawing it needs.
    fn upload_vertices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        verts: &[MsdfVertex],
        atlas: wgpu::BindGroup,
    ) -> PreparedMsdf {
        let vbytes = std::mem::size_of_val(verts) as u64;
        let offset = self.ensure_vbo_capacity(device, vbytes);
        queue.write_buffer(&self.vbo, offset, bytemuck::cast_slice(verts));
        self.vbo_offset = offset + vbytes;
        PreparedMsdf {
            vbo: self.vbo.clone(),
            offset,
            vertices: verts.len() as u32,
            atlas,
            uniform: self.uniform.bind_group().clone(),
            uniform_offset: self.uniform_offset as u32,
        }
    }

    /// Draw an uploaded run into `pass`. Everything it binds was captured at
    /// upload, so runs uploaded earlier in the frame stay drawable after the
    /// vertex buffer grows.
    pub(crate) fn draw_prepared(&self, pass: &mut wgpu::RenderPass<'_>, run: &PreparedMsdf) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &run.uniform, &[run.uniform_offset]);
        pass.set_bind_group(1, &run.atlas, &[]);
        pass.set_vertex_buffer(0, run.vbo.slice(run.offset..));
        pass.draw(0..run.vertices, 0..1);
    }

    /// Build glyph quads for all text blocks, generating any unseen glyphs into the
    /// atlas. Returns the vertex list (6 verts/glyph, triangle list).
    fn build_vertices(&mut self, texts: &[TextBlock]) -> Vec<MsdfVertex> {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_shape").entered();

        let px_range = self.atlas.px_range();
        let ref_px = self.atlas.ref_px();

        // First pass: resolve every block to a list of `GlyphPlacement`s. The
        // layouts come from the font system shared with the measurers, which
        // shapes through cosmic-text only for a layout it doesn't keep — so a
        // block measured for layout earlier this frame, or drawn on an earlier
        // one, is not shaped again. We resolve uv only *after* this pass,
        // because glyph generation can grow the atlas (changing the size uv
        // divides by) — pixel regions stay valid (top-left origin) but uv must use
        // the final size.
        //
        // The font system stays locked for the whole pass, including drawing
        // new glyphs into the atlas (~1.5 ms each, once per glyph): a placed
        // glyph borrows its layout from the shared cache, and its outline
        // comes from the font system. Measuring and drawing run on one thread,
        // so nothing waits on it; a renderer on another thread sharing this
        // handle would wait for the pass.
        let mut placements: Vec<GlyphPlacement> = Vec::new();
        let fs_handle = Arc::clone(&self.font_system);
        let mut shared = fs_handle.lock().expect("FontSystem poisoned");
        for block in texts {
            if block.content.is_empty() {
                continue;
            }
            let (layout, fs) =
                shared.layout_and_fonts(&LayoutSpec::of_block(block), &block.content);
            append_placements(
                &mut self.atlas,
                &mut self.font_keys,
                &mut self.next_font_key,
                fs,
                block,
                &block.spans,
                &layout.glyphs,
                &mut placements,
            );
        }
        drop(shared);

        // Second pass: resolve uv against the final atlas size and emit quads in
        // back-to-front sweeps so every glyph's shadow/glow sits behind ALL fills:
        //   1. shadows  2. glow  3. fill (+ outline)
        let (aw, ah) = (self.atlas.width(), self.atlas.height());
        let mut verts: Vec<MsdfVertex> = Vec::with_capacity(placements.len() * 6);

        for p in &placements {
            if let Some((color, offset, softness)) = p.shadow {
                // A shadow is a fill with widened AA; cap the blur to the field reach.
                let safe = field_reach(p.font_size, px_range, ref_px);
                push_glyph_quad(
                    &mut verts,
                    p,
                    aw,
                    ah,
                    px_range,
                    &QuadStyle {
                        fill: color,
                        outline: [0.0; 4],
                        outline_width: 0.0,
                        softness: softness.min(safe),
                        offset,
                    },
                );
            }
        }
        for p in &placements {
            if let Some((color, radius)) = p.glow {
                // The glow is a grown, soft, fill-less halo. Cap its band (width +
                // softness) to the field's valid reach so it follows the glyph instead
                // of filling the tile rectangle (graceful degradation at small sizes).
                let safe = field_reach(p.font_size, px_range, ref_px);
                let radius = radius.min(safe / 1.5);
                let softness = (radius * 0.5).max(0.5).min((safe - radius).max(0.0));
                push_glyph_quad(
                    &mut verts,
                    p,
                    aw,
                    ah,
                    px_range,
                    &QuadStyle {
                        fill: [0.0; 4],
                        outline: color,
                        outline_width: radius,
                        softness,
                        offset: [0.0, 0.0],
                    },
                );
            }
        }
        for p in &placements {
            let (outline, outline_width) = p.outline.unwrap_or(([0.0; 4], 0.0));
            // Cap the outline thickness to the field reach to avoid tile-fill artifacts.
            let safe = field_reach(p.font_size, px_range, ref_px);
            push_glyph_quad(
                &mut verts,
                p,
                aw,
                ah,
                px_range,
                &QuadStyle {
                    fill: p.fill,
                    outline,
                    outline_width: outline_width.min(safe),
                    softness: 0.0,
                    offset: [0.0, 0.0],
                },
            );
        }
        verts
    }

    /// (Re)upload the atlas pixels to the GPU if the CPU atlas changed. Recreates
    /// the texture + bind group when the atlas has grown.
    fn upload_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_atlas_upload").entered();
        self.glyph_gpu
            .upload(device, queue, &self.atlas_bgl, &mut self.atlas);
    }

    /// Prepare and render text in a pass of its own.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        texts: &[TextBlock],
    ) {
        if let Some(run) = self.upload_texts(device, queue, texts) {
            let mut pass = crate::render::load_pass(encoder, view, "msdf text pass");
            self.draw_prepared(&mut pass, &run);
        }
    }

    /// Shape and upload a batch of text, ready for
    /// [`draw_prepared`](Self::draw_prepared). `None` when nothing draws.
    pub(crate) fn upload_texts(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texts: &[TextBlock],
    ) -> Option<PreparedMsdf> {
        if texts.is_empty() {
            return None;
        }

        #[cfg(feature = "tracy")]
        let _span = tracing::info_span!("gameui_text_render").entered();

        let verts = self.build_vertices(texts);
        // Glyph generation may have dirtied / grown the atlas — upload before drawing.
        self.upload_atlas(device, queue);

        if verts.is_empty() {
            return None;
        }
        let atlas = self.glyph_gpu.bind_group.clone();
        Some(self.upload_vertices(device, queue, &verts, atlas))
    }

    /// Ensure `vbo` can hold `bytes` starting at the current frame offset, and
    /// return the byte offset to write/draw this pass at. Grows by allocating a
    /// fresh buffer when needed; earlier passes keep referencing the old buffer
    /// (held alive by the encoder), so their data stays valid.
    fn ensure_vbo_capacity(&mut self, device: &wgpu::Device, bytes: u64) -> u64 {
        let offset = self.vbo_offset;
        let needed = offset + bytes;
        if needed > self.vbo_capacity {
            self.vbo_capacity = needed.next_power_of_two();
            let replacement = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("msdf text vbo"),
                size: self.vbo_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.retired_vbos
                .push(std::mem::replace(&mut self.vbo, replacement));
        }
        offset
    }
}

/// Convert a cosmic-text `Color` (sRGB-encoded u8 RGBA) to a normalized
/// `[f32; 4]`. No decode: the renderer blends in sRGB space, exactly like the
/// colored-quad pipeline's pass-through `Vertex` colors, so a hex text colour
/// renders as that hex.
fn color_to_rgba(c: Color) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// Lay a string out for **vertical (stacked) text** by putting each grapheme
/// cluster on its own line, so cosmic-text — which has no writing-mode API and
/// only ever stacks *buffer lines* top-to-bottom — renders the clusters in a
/// single descending column. Grapheme clusters (not `char`s) keep combining
/// marks (e.g. dakuten) and ZWJ emoji sequences intact on one row.
///
/// Each inserted `'\n'` is one byte. A glyph's offset is per-buffer-line, so the
/// caller's content byte is recovered as `line_start[line_i] + glyph.start -
/// line_i` — the shaped line base, minus the `line_i` separators that precede the
/// cluster (the correction applied in `build_vertices`). See
/// [`TextBlock::with_vertical`].
pub(crate) fn vertical_stack_string(s: &str) -> String {
    s.graphemes(true).collect::<Vec<_>>().join("\n")
}

/// Stable discriminant for a cosmic-text [`Style`] so it can sit in a `Hash + Eq`
/// cache key: `Normal = 0`, `Italic = 1`, `Oblique = 2`.
pub(crate) fn style_disc(style: Style) -> u8 {
    match style {
        Style::Normal => 0,
        Style::Italic => 1,
        Style::Oblique => 2,
    }
}

/// Map a cosmic-text `fontdb::ID` to a stable atlas font key, assigning a fresh
/// one on first sighting. Free function (rather than a `&mut self` method) so the
/// `font_keys`/`next_font_key` fields can be borrowed disjointly from the rest of
/// the renderer during shaping.
fn resolve_font_key(
    font_keys: &mut HashMap<fontdb::ID, u64>,
    next_font_key: &mut u64,
    id: fontdb::ID,
) -> u64 {
    if let Some(k) = font_keys.get(&id) {
        return *k;
    }
    let k = *next_font_key;
    *next_font_key += 1;
    font_keys.insert(id, k);
    k
}

/// Resolve the fill colour for a single glyph given a set of [`TextSpan`]s.
///
/// Iterates the spans in order, tracking their cumulative byte position in the
/// concatenated text, and returns the `color` of the first span that contains
/// `byte_start`. Returns `None` when `spans` is empty or all spans have
/// `color: None` (caller should fall back to the block's global colour).
pub fn resolve_span_color(byte_start: u32, spans: &[TextSpan]) -> Option<[f32; 4]> {
    let mut offset = 0usize;
    for span in spans {
        let end = offset + span.text.len();
        if (byte_start as usize) < end {
            return span.color;
        }
        offset = end;
    }
    None
}

/// Resolve a glyph colour from sorted byte-range styles in logarithmic time.
pub fn resolve_range_color(byte_start: u32, ranges: &[TextStyleRange]) -> Option<[f32; 4]> {
    let byte = byte_start as usize;
    let candidate = ranges.partition_point(|range| range.range.start <= byte);
    candidate.checked_sub(1).and_then(|index| {
        let range = &ranges[index];
        (byte < range.range.end).then_some(range.color).flatten()
    })
}

/// Turn a block's relative glyph layout into `GlyphPlacement`s, applying the
/// block's position, color, clip and effects. Takes the atlas / font-key fields
/// by `&mut` (not `&mut self`) so the caller can hold a borrow into the shared
/// layouts simultaneously. A glyph the atlas hasn't seen — a layout shaped by a
/// measurer, or a new font — is generated from its face in `font_system`;
/// outline-less glyphs (whitespace) are skipped.
///
/// `spans` may be empty (plain mode); in that case all glyphs use the block's
/// global colour. When non-empty, per-glyph colour is resolved via
/// [`resolve_span_color`] and falls back to the block colour for spans with
/// `color: None`.
#[allow(clippy::too_many_arguments)]
fn append_placements(
    atlas: &mut MsdfGlyphAtlas,
    font_keys: &mut HashMap<fontdb::ID, u64>,
    next_font_key: &mut u64,
    font_system: &mut FontSystem,
    block: &TextBlock,
    spans: &[TextSpan],
    shaped: &[ShapedGlyph],
    out: &mut Vec<GlyphPlacement>,
) {
    let block_fill = color_to_rgba(block.color);
    let clip = block.clip.map(|c| [c.x, c.y, c.width, c.height]);
    let outline = block
        .outline
        .as_ref()
        .map(|o| (color_to_rgba(o.color), o.width_px));
    let shadow = block
        .shadow
        .as_ref()
        .map(|s| (color_to_rgba(s.color), s.offset, s.softness));
    let glow = block
        .glow
        .as_ref()
        .map(|g| (color_to_rgba(g.color), g.radius_px));

    for g in shaped {
        let font_key = resolve_font_key(font_keys, next_font_key, g.font_id);
        let tile = match atlas.cached(font_key, g.glyph_id) {
            Some(tile) => tile,
            None => font_system
                .get_font(g.font_id, g.font_weight)
                .and_then(|font| atlas.glyph(font_key, g.glyph_id, font.data())),
        };
        let Some(tile) = tile else {
            continue; // whitespace / outline-less
        };
        // Byte ranges are searched logarithmically, avoiding the old
        // glyphs-times-tokens scan. Legacy owned spans remain supported.
        let fill = if !block.style_ranges.is_empty() {
            resolve_range_color(g.byte_start, &block.style_ranges)
                .map(|mut color| {
                    for (channel, tint) in color.iter_mut().zip(block.style_range_tint) {
                        *channel = (*channel * tint).clamp(0.0, 1.0);
                    }
                    color
                })
                .unwrap_or(block_fill)
        } else if !spans.is_empty() {
            resolve_span_color(g.byte_start, spans).unwrap_or(block_fill)
        } else {
            block_fill
        };
        out.push(GlyphPlacement {
            tile,
            pen_x: block.x + g.rel_x,
            baseline_y: block.y + g.rel_y,
            font_size: g.font_size,
            clip,
            fill,
            outline,
            shadow,
            glow,
        });
    }
}

/// A glyph ready to be turned into quads, captured before uv resolution. Carries
/// the block's resolved effect parameters so the emit sweeps can build shadow /
/// glow / fill quads from one record.
struct GlyphPlacement {
    tile: GlyphTile,
    pen_x: f32,
    baseline_y: f32,
    font_size: f32,
    clip: Option<[f32; 4]>,
    fill: [f32; 4],
    /// (color, width_px)
    outline: Option<([f32; 4], f32)>,
    /// (color, offset, softness)
    shadow: Option<([f32; 4], [f32; 2], f32)>,
    /// (color, radius_px)
    glow: Option<([f32; 4], f32)>,
}

/// Maximum effect reach (screen px) a glyph's distance field supports at a given
/// font size, leaving 0.5px AA headroom. Effects (outline width, shadow/glow blur)
/// clamped to this never read past the field's valid range, so they follow the
/// glyph shape instead of filling the tile rectangle.
///
/// The field is valid for `±(px_range/2)` tile texels around the edge; one tile
/// texel maps to `font_size / ref_px` screen px.
fn field_reach(font_size: f32, px_range: f32, ref_px: f32) -> f32 {
    (0.5 * px_range * font_size / ref_px - 0.5).max(0.0)
}

/// Per-quad appearance for one emit sweep.
struct QuadStyle {
    fill: [f32; 4],
    outline: [f32; 4],
    outline_width: f32,
    softness: f32,
    /// Screen-space translation applied to the quad (drop-shadow offset).
    offset: [f32; 2],
}

/// Emit two triangles (6 verts) for one glyph's MSDF tile with the given style.
fn push_glyph_quad(
    out: &mut Vec<MsdfVertex>,
    p: &GlyphPlacement,
    atlas_w: u32,
    atlas_h: u32,
    px_range: f32,
    style: &QuadStyle,
) {
    let m = &p.tile.metrics;
    let font_size = p.font_size;
    let (ox, oy) = (style.offset[0], style.offset[1]);
    // Screen rect (y-down): top_em is above the baseline (positive), bottom_em below.
    let x0 = p.pen_x + m.left_em * font_size + ox;
    let x1 = p.pen_x + m.right_em * font_size + ox;
    let y0 = p.baseline_y - m.top_em * font_size + oy;
    let y1 = p.baseline_y - m.bottom_em * font_size + oy;

    let uv = p.tile.region.uv(atlas_w, atlas_h);
    let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);

    let (clip_rect, clip_on) = match p.clip {
        Some(c) => (c, 1.0),
        None => ([0.0; 4], 0.0),
    };

    let v = |x: f32, y: f32, u: f32, vv: f32| MsdfVertex {
        position: [x, y],
        uv: [u, vv],
        fill: style.fill,
        clip: clip_rect,
        clip_enabled: clip_on,
        px_range,
        outline: style.outline,
        outline_width: style.outline_width,
        softness: style.softness,
    };

    // TL, TR, BR / TL, BR, BL
    out.push(v(x0, y0, u0, v0));
    out.push(v(x1, y0, u1, v0));
    out.push(v(x1, y1, u1, v1));
    out.push(v(x0, y0, u0, v0));
    out.push(v(x1, y1, u1, v1));
    out.push(v(x0, y1, u0, v1));
}

/// Place a glyph tile of EM extent `w_em` x `h_em`, centered inside `rect`,
/// returning the local-space quad corners `(x0, y0, x1, y1)` (top-left,
/// bottom-right; y-down).
///
/// The scale is driven by the font **em** (1.0 em → the smaller rect dimension),
/// NOT by per-glyph contain-fit. This is the crucial difference: every icon in a
/// set shares one scale, so a short-and-wide glyph (a minus: ~0.75 x 0.06 em) and
/// a square one (a plus: ~0.75 x 0.75 em) render at consistent proportions — the
/// minus stays a short bar the width of the plus's arm span, instead of being
/// stretched to fill the cell. Contain-fitting each glyph to its own ink box (the
/// old behavior) made the minus blow out to full width in non-square cells.
///
/// The tile extent includes symmetric SDF padding, so centering the tile centers
/// the ink. Padding that overflows the rect is transparent (and clipped if a clip
/// is set), and the *visible* ink size is `ink_em * min(rect dims)` regardless of
/// padding. Pure function, unit-tested headlessly.
#[cfg(feature = "phosphor-icons")]
fn fit_centered(rect: Rect, w_em: f32, h_em: f32) -> (f32, f32, f32, f32) {
    // 1 em maps to the smaller rect dimension — a fixed reference shared by all
    // icons, so their relative sizes follow the font design.
    let em_px = rect.width.min(rect.height);
    let quad_w = w_em * em_px;
    let quad_h = h_em * em_px;
    let cx = rect.x + rect.width * 0.5;
    let cy = rect.y + rect.height * 0.5;
    (
        cx - quad_w * 0.5,
        cy - quad_h * 0.5,
        cx + quad_w * 0.5,
        cy + quad_h * 0.5,
    )
}

/// Emit two triangles (6 verts) for one icon, fitting-and-centering its glyph
/// tile into the icon's local rect and transforming the resulting corners by the
/// icon's affine (so rotation/scale work — unlike the axis-aligned text path).
#[cfg(feature = "phosphor-icons")]
fn push_icon_quad(
    out: &mut Vec<MsdfVertex>,
    tile: &GlyphTile,
    icon: &IconMsdf,
    atlas_w: u32,
    atlas_h: u32,
    px_range: f32,
) {
    let m = &tile.metrics;
    let w_em = m.right_em - m.left_em;
    let h_em = m.top_em - m.bottom_em;
    if w_em <= 0.0 || h_em <= 0.0 {
        return;
    }
    let (x0, y0, x1, y1) = fit_centered(icon.local, w_em, h_em);

    let uv = tile.region.uv(atlas_w, atlas_h);
    let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);

    let (clip_rect, clip_on) = match icon.clip {
        Some(c) => ([c.x, c.y, c.width, c.height], 1.0),
        None => ([0.0; 4], 0.0),
    };

    let t = &icon.transform;
    let tl = t.transform_point([x0, y0]);
    let tr = t.transform_point([x1, y0]);
    let br = t.transform_point([x1, y1]);
    let bl = t.transform_point([x0, y1]);

    let v = |pos: [f32; 2], u: f32, vv: f32| MsdfVertex {
        position: pos,
        uv: [u, vv],
        fill: icon.tint,
        clip: clip_rect,
        clip_enabled: clip_on,
        px_range,
        outline: [0.0; 4],
        outline_width: 0.0,
        softness: 0.0,
    };

    // TL, TR, BR / TL, BR, BL
    out.push(v(tl, u0, v0));
    out.push(v(tr, u1, v0));
    out.push(v(br, u1, v1));
    out.push(v(tl, u0, v0));
    out.push(v(br, u1, v1));
    out.push(v(bl, u0, v1));
}

/// The bind-group layout (group 1) for an MSDF atlas: a filterable 2D texture +
/// a filtering sampler. Identical for the text glyph atlas and the icon atlas,
/// so one layout is shared across both `MsdfTextureGpu` instances and the
/// pipeline.
fn create_msdf_atlas_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("msdf atlas bgl"),
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
    })
}

/// GPU mirror of an [`MsdfGlyphAtlas`]: the `Rgba8Unorm` (linear) atlas texture,
/// its filtering sampler, and the group-1 bind group consumed by the shared MSDF
/// pipeline. Extracting this keeps the (fragile) grow/upload/resize dance in one
/// place — the text glyph atlas and the Phosphor icon atlas each own one.
struct MsdfTextureGpu {
    texture: wgpu::Texture,
    /// Held only to keep it alive for `bind_group`.
    #[allow(dead_code)]
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    /// Atlas resources replaced while encoding the current frame. A newly seen
    /// glyph may grow the atlas between ordered text runs; earlier render passes
    /// still reference the previous bind group and texture until submission.
    retired: Vec<(wgpu::Texture, wgpu::Sampler, wgpu::BindGroup)>,
    /// Atlas width last uploaded to the GPU. Starts at 0 so the first `upload`
    /// always writes pixels (the texture is created empty).
    current_size: u32,
}

impl MsdfTextureGpu {
    fn new(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, atlas: &MsdfGlyphAtlas) -> Self {
        let (texture, sampler, _bgl, bind_group) =
            create_msdf_texture_with_bgl(device, bgl, atlas.width(), atlas.height());
        Self {
            texture,
            sampler,
            bind_group,
            retired: Vec::new(),
            current_size: 0,
        }
    }

    /// (Re)upload the atlas pixels if the CPU atlas changed. Recreates the texture
    /// + bind group when the atlas has grown.
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bgl: &wgpu::BindGroupLayout,
        atlas: &mut MsdfGlyphAtlas,
    ) {
        if atlas.width() != self.current_size {
            let (texture, sampler, _bgl, bind_group) =
                create_msdf_texture_with_bgl(device, bgl, atlas.width(), atlas.height());
            let old_texture = std::mem::replace(&mut self.texture, texture);
            let old_sampler = std::mem::replace(&mut self.sampler, sampler);
            let old_bind_group = std::mem::replace(&mut self.bind_group, bind_group);
            self.retired
                .push((old_texture, old_sampler, old_bind_group));
            self.current_size = atlas.width();
            let _ = atlas.take_dirty();
            self.write_pixels(queue, atlas);
        } else if atlas.take_dirty() {
            self.write_pixels(queue, atlas);
        }
    }

    fn write_pixels(&self, queue: &wgpu::Queue, atlas: &MsdfGlyphAtlas) {
        let pixels = atlas.build_pixel_buffer();
        let w = atlas.width();
        let h = atlas.height();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
}

fn create_msdf_texture_with_bgl(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::Sampler, (), wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("msdf atlas texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Linear (NOT sRGB): MSDF texels are distances, not colors.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("msdf atlas bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (texture, sampler, (), bg)
}

/// Reference size (px) at which per-font vertical metrics are sampled, then stored
/// as ratios and scaled to the actual `font_size`. Large enough that hinting /
/// rounding noise in the sampled baseline is negligible.
const VMETRICS_REF_PX: f32 = 100.0;

/// Per-font vertical metrics for **optical** vertical centring, expressed as
/// ratios of `font_size` so they apply at any size.
///
/// - `baseline_ratio` — the first baseline's offset below the text block's top
///   (`TextBlock::y`), i.e. `baseline_y / font_size` when the block top is 0. For
///   the default line box this is a touch over `1.0` (the line box leads the
///   baseline slightly).
/// - `x_ratio` — the font's **x-height** (lowercase body height) as a fraction of
///   `font_size` (`~0.5`). Used to centre labels that contain lowercase letters:
///   the lowercase body carries the visual mass of mixed-case text, so centring it
///   reads as "centred".
/// - `cap_ratio` — the font's **cap height** (`~0.7`). Used to centre labels with no
///   lowercase letters (all-caps, digits, symbols), whose mass reaches cap height —
///   centring those on the x-band would sit them low.
/// - `cjk_baseline_ratio` — the baseline offset below the block top for a **CJK**
///   line, as a fraction of `font_size`. CJK glyphs ride a taller baseline than
///   roman text, so this differs from `baseline_ratio`; it is read from the same
///   shaping pass as `cjk_center_ratio` so the two stay consistent.
/// - `cjk_center_ratio` — the **ideographic ink centre** above the *CJK* baseline
///   as a fraction of `font_size` (`~0.4`). CJK glyphs fill the em square and dip
///   slightly below the baseline, so labels containing CJK centre on this band.
///   Both CJK fields degrade to the roman baseline + cap-band centre when no CJK
///   face is available, so non-CJK setups are unchanged.
///
/// `DrawList::vcentered_text_y` picks the centre per label via
/// [`Self::visual_center_ratio`]. See [`vcentered_line_y`] for the em-box
/// (font-metric-agnostic) counterpart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontVMetrics {
    /// First baseline's offset below the block top, as a fraction of `font_size`
    /// (a touch over `1.0` for the default line box).
    pub baseline_ratio: f32,
    /// Font x-height (lowercase body height) as a fraction of `font_size` (`~0.5`).
    pub x_ratio: f32,
    /// Font cap height as a fraction of `font_size` (`~0.7`).
    pub cap_ratio: f32,
    /// Baseline offset below the block top for a CJK line, as a fraction of
    /// `font_size` (CJK rides a taller baseline than roman text).
    pub cjk_baseline_ratio: f32,
    /// Ideographic ink centre above the CJK baseline, as a fraction of `font_size`
    /// (`~0.4`).
    pub cjk_center_ratio: f32,
}

impl FontVMetrics {
    /// The optical centring band height (as a fraction of `font_size`) for a label,
    /// chosen by whether the label contains lowercase letters. Roman scripts only —
    /// CJK uses [`Self::visual_center_ratio`] directly.
    ///
    /// Lowercase roman bodies are the only glyphs whose visual mass sits at
    /// x-height; capitals, digits and symbols all reach (roughly) cap height. So a
    /// label with any lowercase letter centres on the **x-height** band, and one
    /// with none (all-caps, `"100%"`, `"OK"`) centres on the **cap-height** band.
    pub fn band_ratio(&self, has_lowercase: bool) -> f32 {
        if has_lowercase {
            self.x_ratio
        } else {
            self.cap_ratio
        }
    }

    /// How far **below the text block's top** the label's optical centre sits, as a
    /// fraction of `font_size` — the quantity `DrawList::vcentered_text_y` aligns to
    /// the span centre (`block_top + font_size · this == span_centre`). Chosen from
    /// the text:
    ///
    /// - contains **CJK** → the ideographic ink centre, measured on the CJK
    ///   baseline (`cjk_baseline_ratio − cjk_center_ratio`);
    /// - else contains **lowercase** → the x-height band centre
    ///   (`baseline_ratio − x_ratio/2`);
    /// - else (all-caps / numeric) → the cap-height band centre
    ///   (`baseline_ratio − cap_ratio/2`).
    ///
    /// CJK takes precedence over case because, when ideographs are present, their
    /// em-filling mass dominates the line's vertical placement. CJK uses its own
    /// baseline (taller than roman) so the glyph is centred where it actually
    /// renders, not where roman text would.
    pub fn visual_center_ratio(&self, text: &str) -> f32 {
        if has_cjk(text) {
            self.cjk_baseline_ratio - self.cjk_center_ratio
        } else if has_lowercase(text) {
            self.baseline_ratio - self.x_ratio / 2.0
        } else {
            self.baseline_ratio - self.cap_ratio / 2.0
        }
    }
}

/// Whether `text` contains any CJK / full-width ideographic character — the signal
/// [`FontVMetrics::visual_center_ratio`] uses to switch to ideographic centring.
///
/// These scripts (Han, Kana, Hangul, CJK punctuation, full-width forms) fill and
/// slightly overhang the em square rather than sitting in the roman x/cap band, so
/// they centre a little higher and dip below the baseline.
pub fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x3000..=0x303F |    // CJK symbols & punctuation
            0x3040..=0x30FF |    // Hiragana + Katakana
            0x3400..=0x4DBF |    // CJK Unified Ideographs Ext A
            0x4E00..=0x9FFF |    // CJK Unified Ideographs
            0xAC00..=0xD7AF |    // Hangul syllables
            0xF900..=0xFAFF |    // CJK compatibility ideographs
            0xFF00..=0xFFEF |    // Halfwidth & fullwidth forms
            0x20000..=0x2A6DF |  // CJK Unified Ideographs Ext B
            0x2A700..=0x2EBEF    // CJK Unified Ideographs Ext C–F
        )
    })
}

/// Whether `text` contains any lowercase letter — the signal
/// [`FontVMetrics::band_ratio`] uses to pick the x-height vs cap-height band.
///
/// Scripts without case (digits, symbols, CJK) report `false`, so they centre on
/// the taller cap-height band where their visual mass actually sits.
pub fn has_lowercase(text: &str) -> bool {
    text.chars().any(|c| c.is_lowercase())
}

/// Text measurement front-end: shapes through cosmic-text to report `(width,
/// height)` for layout, and the optical vertical metrics per font.
///
/// Both are kept in the font system it shares with a `TextRenderer`
/// ([`SharedFontSystem`]), so measured widths match rendered glyphs, measuring
/// the same string again is a hash lookup, a block measured for layout and
/// then drawn is shaped once, and a new measurer on the same font system
/// starts warm.
pub struct TextMeasurer {
    font_system: FontSystemHandle,
}

impl TextMeasurer {
    /// Create a measurer with its own private `FontSystem`.
    ///
    /// Prefer [`TextMeasurer::with_font_system`] when a `TextRenderer` already exists,
    /// so measured widths match rendered glyphs.
    pub fn new() -> Self {
        Self::with_font_system(shared_font_system())
    }

    /// Create a measurer that shares its `FontSystem` with another component (typically
    /// a `TextRenderer`).
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self { font_system }
    }

    /// Get a clone of the shared font system handle.
    pub fn font_system_handle(&self) -> FontSystemHandle {
        Arc::clone(&self.font_system)
    }

    /// Drop all cached measurements: the layouts and font metrics kept in the
    /// shared font system ([`SharedFontSystem::clear_caches`]). Loading a font
    /// through this crate already does this.
    pub fn clear_cache(&mut self) {
        self.font_system
            .lock()
            .expect("FontSystem poisoned")
            .clear_caches();
    }

    /// Resolve [`FontVMetrics`] for `(font, weight, style)` for optical vertical
    /// centring, kept in the shared font system. On a hit this is a hash lookup;
    /// on a miss it shapes one glyph to read the font's baseline placement and
    /// parses the resolved face for its cap height.
    ///
    /// If the face can't be resolved or parsed, the returned metrics reduce
    /// optical centring to the em-box result of [`vcentered_line_y`], so callers
    /// degrade gracefully rather than mis-centre.
    pub fn vmetrics(
        &mut self,
        font: Option<&FontHandle>,
        weight: Weight,
        style: Style,
    ) -> FontVMetrics {
        self.font_system
            .lock()
            .expect("FontSystem poisoned")
            .vmetrics(font, weight, style)
    }

    /// Measure text using glyphon's shaping/layout path, with a result cache.
    ///
    /// `max_width` constrains the shaping width; pass `None` for unconstrained
    /// single-line measurement, or `Some(w)` to let glyphon wrap the text and report
    /// the resulting multi-line height.
    ///
    /// On a cache hit this performs only a hash lookup and does not lock the
    /// `FontSystem`. On a miss it shapes the text (mutating glyphon's font system cache)
    /// and stores the result; it never touches any GPU renderer, atlas, or swash state.
    pub fn measure(&mut self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_with_font(text, font_size, max_width, None)
    }

    /// Like [`measure`](Self::measure), but shapes `text` in a specific font.
    ///
    /// Pass `None` for the default (system sans-serif). Different fonts have
    /// different glyph advances, so the font is part of the cache key — measuring
    /// the same string under two fonts caches two results.
    pub fn measure_with_font(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        font: Option<&FontHandle>,
    ) -> (f32, f32) {
        self.measure_styled(
            text,
            font_size,
            max_width,
            font,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        )
    }

    /// Like [`measure_with_font`](Self::measure_with_font), but also selects a
    /// font `weight`, `style`, and wrap policy. Bold/italic faces have different
    /// advances and the wrap policy changes line breaks, so each
    /// `(font, weight, style, wrap)` combination is a distinct cache entry — a
    /// measurement under this path matches a [`TextBlock`] rendered with the same
    /// font/weight/style/wrap.
    #[allow(clippy::too_many_arguments)]
    pub fn measure_styled(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        font: Option<&FontHandle>,
        weight: Weight,
        style: Style,
        wrap: WrapMode,
    ) -> (f32, f32) {
        self.measure_styled_with_letter_spacing(
            text, font_size, max_width, font, weight, style, wrap, 0.0,
        )
    }

    /// Like [`measure_styled`](Self::measure_styled), with additional spacing in
    /// pixels applied by cosmic-text between shaped glyphs.
    #[allow(clippy::too_many_arguments)]
    pub fn measure_styled_with_letter_spacing(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        font: Option<&FontHandle>,
        weight: Weight,
        style: Style,
        wrap: WrapMode,
        letter_spacing: f32,
    ) -> (f32, f32) {
        self.measure_keyed(
            text,
            font_size,
            max_width,
            font,
            weight,
            style,
            wrap,
            false,
            letter_spacing,
        )
    }

    /// Measure `text` laid out as **vertical (stacked) text** — one grapheme
    /// cluster per row, top-to-bottom — returning `(column_width, stacked_height)`.
    /// Matches a [`TextBlock`] rendered with [`with_vertical`](TextBlock::with_vertical)
    /// at the same `font_size` in the default font. The result is tall-and-narrow,
    /// the transpose of the horizontal measurement of the same string.
    pub fn measure_vertical(&mut self, text: &str, font_size: f32) -> (f32, f32) {
        self.measure_keyed(
            text,
            font_size,
            None,
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
            true,
            0.0,
        )
    }

    /// Measure a [`TextBlock`] **exactly as it will be laid out**, honouring its
    /// font, weight, style, wrap policy, `max_width`, and `vertical` flag.
    ///
    /// The narrower `measure*` methods above each hard-code some of those
    /// (`measure_styled` cannot do vertical text; everything below it assumes
    /// the default font at `Weight::NORMAL`/`Style::Normal`), so measuring a
    /// bold, italic, custom-font, or stacked block through them reports the
    /// wrong size. Prefer this when you have the block itself — as debug
    /// tooling and layout inspection do.
    ///
    /// An [`ellipsize`](TextBlock::ellipsize) block is measured **unwrapped on
    /// one line**: the width the content *wants* before truncation. Compare it
    /// against `max_width` to tell whether the ellipsis actually engaged.
    pub fn measure_block(&mut self, block: &TextBlock) -> (f32, f32) {
        let mut spec = LayoutSpec::of_block(block);
        if block.ellipsize && !block.vertical {
            spec.ellipsize = false;
            spec.max_width = None;
            spec.align = TextAlign::Start;
        }
        self.measure_spec(&block.content, &spec)
    }

    /// The band of real glyph **ink** a block paints, as `(top, bottom)` offsets
    /// below the block's top edge ([`TextBlock::y`]). `None` when the block inks
    /// nothing — empty or whitespace-only content.
    ///
    /// [`measure_block`](Self::measure_block) reports the *slot*: advance width
    /// by line-box height, which is what layout reserves. This reports what
    /// actually lands on screen, and the two differ by design. A line box always
    /// carries leading above the ascent and below the descent, and
    /// `DrawList::vcentered_text_y` slides that whole slot upward so the glyphs'
    /// optical centre — not the slot's — sits on the row centre. Comparing the
    /// slot against the row therefore reports a correctly centred label as
    /// overflowing by a pixel or two; comparing the ink does not.
    ///
    /// Intended for layout inspection (see [`crate::debug`]): the band is kept
    /// with the block's layout, but worked out only when asked for, never on
    /// the drawing path.
    pub fn measure_block_ink(&mut self, block: &TextBlock) -> Option<(f32, f32)> {
        if block.content.is_empty() {
            return None;
        }
        let mut shared = self.font_system.lock().expect("FontSystem poisoned");
        let (layout, fs) = shared.layout_and_fonts(&LayoutSpec::of_block(block), &block.content);
        *layout.ink.get_or_init(|| ink_band(fs, &layout.glyphs))
    }

    /// Measurement backing [`measure_styled`](Self::measure_styled)
    /// (horizontal) and [`measure_vertical`](Self::measure_vertical): plain
    /// text in `font` at the default line height.
    #[allow(clippy::too_many_arguments)]
    fn measure_keyed(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        font: Option<&FontHandle>,
        weight: Weight,
        style: Style,
        wrap: WrapMode,
        vertical: bool,
        letter_spacing: f32,
    ) -> (f32, f32) {
        let spec = LayoutSpec {
            font,
            weight,
            style,
            wrap,
            vertical,
            letter_spacing,
            ..LayoutSpec::plain(font_size, max_width)
        };
        self.measure_spec(text, &spec)
    }

    /// The size layout reserves for `text` laid out under `spec`.
    fn measure_spec(&mut self, text: &str, spec: &LayoutSpec<'_>) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, spec.line_height);
        }
        let mut shared = self.font_system.lock().expect("FontSystem poisoned");
        shared.layout(spec, text).size
    }
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a font handle into the `TextMeasurer` cache key. `None` (the default
/// font) hashes to 0; a named font hashes its family. Two different family names
/// colliding on a 64-bit hash is astronomically unlikely and only the cost is a
/// rare stale measurement, so a plain `DefaultHasher` is fine here.
pub(crate) fn family_hash(font: Option<&FontHandle>) -> u64 {
    use std::hash::{Hash, Hasher};
    match font {
        None => 0,
        Some(h) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            h.0.hash(&mut hasher);
            hasher.finish()
        }
    }
}

/// Map our [`TextAlign`] to cosmic-text's `Align`, returning `None` for the
/// default (`Left`) so callers can skip the per-line override.
pub(crate) fn cosmic_align(align: TextAlign) -> Option<CosmicAlign> {
    match align {
        // cosmic-text's default layout already flushes to the reading start
        // (left for LTR, right for RTL), so `Start` is "no override".
        TextAlign::Start => None,
        TextAlign::Center => Some(CosmicAlign::Center),
        // `End` is cosmic-text's direction-relative end alignment.
        TextAlign::End => Some(CosmicAlign::End),
        TextAlign::Left => Some(CosmicAlign::Left),
        TextAlign::Right => Some(CosmicAlign::Right),
    }
}

/// The vertical band of real glyph **ink**, as offsets below the block's top
/// edge, or `None` when the string inks nothing (empty or whitespace-only).
///
/// This is deliberately *not* the line box. A line box is a typographic slot
/// that always has leading above the ascent and below the descent, and
/// `DrawList::vcentered_text_y` shifts that whole slot upward on purpose so the
/// glyphs' optical centre — not the slot's centre — lands on the row's centre.
/// A correctly centred label therefore has a line box poking out of its row by
/// a pixel or two, which is exactly the false alarm layout inspection must not
/// raise. Measuring the ink itself is what makes "did this text paint outside
/// its box?" answerable.
///
/// Per glyph the band is `baseline − y_max` … `baseline − y_min` of the glyph's
/// outline bounding box, scaled from font units. Whitespace and other
/// outline-less glyphs have no bounding box and contribute nothing, matching the
/// renderer, which skips exactly those.
fn ink_band(font_system: &mut FontSystem, glyphs: &[ShapedGlyph]) -> Option<(f32, f32)> {
    // Group by face first: parsing a face is far more expensive than reading a
    // glyph box out of one, and a label is almost always a single face.
    let mut by_font: HashMap<(fontdb::ID, fontdb::Weight), Vec<&ShapedGlyph>> = HashMap::new();
    for glyph in glyphs {
        by_font
            .entry((glyph.font_id, glyph.font_weight))
            .or_default()
            .push(glyph);
    }

    let mut top = f32::INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for ((font_id, font_weight), glyphs) in by_font {
        let Some(font) = font_system.get_font(font_id, font_weight) else {
            continue;
        };
        let Ok(face) = ttf_parser::Face::parse(font.data(), 0) else {
            continue;
        };
        let upem = face.units_per_em() as f32;
        if upem <= 0.0 {
            continue;
        }
        for glyph in glyphs {
            let Some(bb) = face.glyph_bounding_box(ttf_parser::GlyphId(glyph.glyph_id)) else {
                continue; // whitespace / outline-less
            };
            // `rel_y` is the baseline the renderer places the quad on.
            top = top.min(glyph.rel_y - bb.y_max as f32 / upem * glyph.font_size);
            bottom = bottom.max(glyph.rel_y - bb.y_min as f32 / upem * glyph.font_size);
        }
    }

    (top <= bottom).then_some((top, bottom))
}

/// Sample a font's vertical metrics ([`FontVMetrics`]) for optical centring.
///
/// Shapes a single `'H'` at [`VMETRICS_REF_PX`] to read the first baseline's
/// offset from cosmic-text's *own* layout (so it matches how text is actually
/// shaped — no hhea-vs-OS/2 ambiguity), then resolves the shaped face and reads
/// its cap height via ttf-parser. Falls back so that, when cap height is
/// unavailable, optical centring equals em-box centring.
pub(crate) fn resolve_vmetrics(
    font_system: &mut FontSystem,
    family_name: Option<&str>,
    weight: Weight,
    style: Style,
) -> FontVMetrics {
    let ref_px = VMETRICS_REF_PX;
    let line_height = ref_px * LINE_HEIGHT_RATIO;
    let mut buffer = Buffer::new(font_system, Metrics::new(ref_px, line_height));
    buffer.set_size(Some(f32::MAX / 4.0), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(
        "H",
        &Attrs::new().family(family).weight(weight).style(style),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    // First baseline offset, from cosmic-text's layout of the reference line.
    let mut baseline_ratio = LINE_HEIGHT_RATIO / 2.0;
    let mut font_id: Option<(fontdb::ID, fontdb::Weight)> = None;
    if let Some(run) = buffer.layout_runs().next() {
        baseline_ratio = run.line_y / ref_px;
        font_id = run.glyphs.first().map(|g| (g.font_id, g.font_weight));
    }

    // x-height (centring target) and cap height (reference) from the resolved
    // face. The em-box equivalent is the fallback so optical centring degrades to
    // line-box centring when the metrics are unavailable.
    let embox_ratio = 2.0 * (baseline_ratio - LINE_HEIGHT_RATIO / 2.0);
    let face_metrics = font_id
        .and_then(|(id, weight)| font_system.get_font(id, weight))
        .and_then(|f| face_vratios(f.data()));
    let (x_ratio, cap_ratio) = match face_metrics {
        Some((x, cap)) => (x, cap),
        None => (embox_ratio, embox_ratio),
    };

    // CJK ideographic centre. Ideographs are laid out on the CJK font's *own*
    // baseline — taller than the roman one — and overhang the em square, so we
    // read BOTH the baseline and the ink centre from this same shaping pass:
    // mixing the roman baseline with CJK ink units mis-centres the glyph. Shapes a
    // representative ideograph, resolves the face cosmic-text picked for it
    // (commonly a fallback distinct from the roman face), and reads its ink centre
    // above that baseline. Falls back to the roman baseline + cap-band centre when
    // no CJK face/glyph is available, so setups without a CJK font are unchanged.
    let mut cjk_buffer = Buffer::new(font_system, Metrics::new(ref_px, line_height));
    cjk_buffer.set_size(Some(f32::MAX / 4.0), None);
    cjk_buffer.set_text(
        CJK_PROBE,
        &Attrs::new().family(family).weight(weight).style(style),
        Shaping::Advanced,
        None,
    );
    cjk_buffer.shape_until_scroll(font_system, false);
    let mut cjk_line_y = None;
    let mut cjk_font_id = None;
    if let Some(run) = cjk_buffer.layout_runs().next() {
        cjk_line_y = Some(run.line_y);
        cjk_font_id = run.glyphs.first().map(|g| (g.font_id, g.font_weight));
    }
    let cjk_face_center = cjk_font_id
        .and_then(|(id, weight)| font_system.get_font(id, weight))
        .and_then(|f| face_cjk_center(f.data()));
    let (cjk_baseline_ratio, cjk_center_ratio) = match (cjk_line_y, cjk_face_center) {
        (Some(line_y), Some(center)) => (line_y / ref_px, center),
        // No CJK face/glyph: degrade to roman baseline + cap-band centring.
        _ => (baseline_ratio, cap_ratio / 2.0),
    };

    FontVMetrics {
        baseline_ratio,
        x_ratio,
        cap_ratio,
        cjk_baseline_ratio,
        cjk_center_ratio,
    }
}

/// Representative ideograph used to probe a CJK face's ink centre. U+4E2D (中) is
/// present in every CJK font and roughly fills the ideographic square.
const CJK_PROBE: &str = "中";

/// The ideographic ink centre above the baseline, as a fraction of em, from raw
/// font bytes: `(y_max + y_min) / (2·upem)` of the probe glyph's bounding box
/// (`y_min` is negative for the part below the baseline). `None` when the face
/// can't be parsed or lacks the probe glyph.
fn face_cjk_center(data: &[u8]) -> Option<f32> {
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let upem = face.units_per_em() as f32;
    if upem <= 0.0 {
        return None;
    }
    let probe = CJK_PROBE.chars().next()?;
    let gid = face.glyph_index(probe)?;
    let bb = face.glyph_bounding_box(gid)?;
    Some((bb.y_max as f32 + bb.y_min as f32) / (2.0 * upem))
}

/// `(x_height, cap_height)` as fractions of em, from raw font bytes.
///
/// x-height: OS/2 `sxHeight` → `'x'` glyph bbox height → `0.5 × cap`.
/// cap height: OS/2 `sCapHeight` → `'H'` glyph bbox height → `0.7 × ascender`.
/// Returns `None` only if the face can't be parsed at all.
fn face_vratios(data: &[u8]) -> Option<(f32, f32)> {
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let upem = face.units_per_em() as f32;
    if upem <= 0.0 {
        return None;
    }
    let glyph_top = |c: char| -> Option<f32> {
        let gid = face.glyph_index(c)?;
        let bb = face.glyph_bounding_box(gid)?;
        (bb.y_max > 0).then_some(bb.y_max as f32 / upem)
    };

    let cap_ratio = face
        .capital_height()
        .filter(|&c| c > 0)
        .map(|c| c as f32 / upem)
        .or_else(|| glyph_top('H'))
        .or_else(|| {
            let asc = face.ascender();
            (asc > 0).then_some(0.7 * asc as f32 / upem)
        })
        .unwrap_or(0.7);

    let x_ratio = face
        .x_height()
        .filter(|&x| x > 0)
        .map(|x| x as f32 / upem)
        .or_else(|| glyph_top('x'))
        .unwrap_or(0.5 * cap_ratio);

    Some((x_ratio, cap_ratio))
}

/// Compute per-character cursor x-positions for the given text.
///
/// Returns a `Vec<(usize, f32)>` where each entry maps a byte index in `text`
/// to its x-offset (in screen pixels) from the left edge. The first entry is
/// always `(0, 0.0)` and the last is `(text.len(), total_width)`. For a single
/// line of text, these are monotonically increasing.
///
/// Use this to implement click-to-position (binary-search on the x values) and
/// to draw the text cursor / selection highlights at the correct x position
/// for a given byte offset.
///
/// `family_name` selects the font; pass `None` for the default sans-serif.
pub fn text_cursor_positions(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    family_name: Option<&str>,
) -> Vec<(usize, f32)> {
    let mut positions: Vec<(usize, f32)> = Vec::with_capacity(text.len().saturating_add(1));
    if text.is_empty() {
        positions.push((0, 0.0));
        return positions;
    }

    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(Some(max_width), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(text, &Attrs::new().family(family), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    // Each cluster boundary (byte index) maps to the glyph's x-position.
    // We iterate in layout-run order (top-to-bottom for multi-line) and
    // record the x for each glyph's start. The last position is the total
    // width of the longest line.
    positions.push((0, 0.0));

    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let start_idx = glyph.start;
            // Only record the first time we see each byte index.
            if start_idx > positions.last().map(|(i, _)| *i).unwrap_or(0) {
                positions.push((start_idx, glyph.x));
            }
        }
        // After each run, record the line end position.
        let line_end_x = run.line_w;
        // Find the last byte index of this run.
        if let Some(last_glyph) = run.glyphs.last() {
            let end_idx = last_glyph.end;
            if end_idx > positions.last().map(|(i, _)| *i).unwrap_or(0) {
                positions.push((end_idx, line_end_x));
            }
        }
    }

    // Ensure the final byte index is always present.
    let last_byte = text.len();
    if positions.last().map(|(i, _)| *i).unwrap_or(0) < last_byte {
        // Estimate: use the last position's x plus an average char width.
        let last_x = positions.last().map(|(_, x)| *x).unwrap_or(0.0);
        positions.push((last_byte, last_x));
    }

    positions
}

/// A caret-addressable position in laid-out text, keeping the **line geometry**
/// that [`text_cursor_positions`] flattens away. Used for multi-line editing:
/// vertical navigation, line-relative Home/End, per-line selection rectangles,
/// and click-to-place hit testing.
///
/// `byte` is an **absolute** byte offset into the whole text (cosmic-text's
/// per-glyph `start`/`end` are relative to their buffer line, so this struct
/// pre-adds the buffer line's starting byte). `x` is the caret x within the
/// visual line's left edge. `line` is the **visual** line ordinal (a soft-wrap
/// produces a new visual line even without a `\n`), top-to-bottom. `line_top`
/// is the y of the top of that visual line; `line_height` its height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretPos {
    /// Absolute byte offset into the whole text (buffer-line base pre-added).
    pub byte: usize,
    /// Caret x relative to the visual line's left edge.
    pub x: f32,
    /// Visual line ordinal, top-to-bottom (a soft wrap starts a new visual line).
    pub line: usize,
    /// Y of the top of this visual line.
    pub line_top: f32,
    /// Height of this visual line.
    pub line_height: f32,
}

/// Lay text out and return one [`CaretPos`] per cluster boundary, preserving
/// per-line geometry (unlike [`text_cursor_positions`], which flattens to
/// `(byte, x)`). This is the keystone for multi-line `TextInput`.
///
/// `wrap` controls line breaking; pass [`WrapMode::None`] for single-line fields
/// and [`WrapMode::WordOrGlyph`] (the default) for a textarea. `family_name`
/// selects the font (`None` → default sans-serif).
///
/// ## Byte offsets are absolute
/// cosmic-text reports `glyph.start`/`glyph.end` **relative to the buffer line**
/// (`LayoutRun.line_i`), so this function precomputes each buffer line's starting
/// byte (by scanning for `\n`) and adds it: `byte = line_start[line_i] +
/// glyph.start`. Without this, every line after the first would map to the wrong
/// position in the source string.
///
/// ## Soft-wrap boundaries
/// A soft wrap (no `\n`) yields the same byte at the end of visual line *i*
/// (`x = line_w`) and the start of line *i+1* (`x = 0`). Both entries are emitted;
/// [`caret_for_byte`] returns the first (end-of-line) match — acceptable for v1.
#[allow(clippy::too_many_arguments)]
pub fn text_caret_layout(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    wrap: WrapMode,
    family_name: Option<&str>,
    direction: TextDirection,
) -> Vec<CaretPos> {
    let mut out: Vec<CaretPos> = Vec::with_capacity(text.len().saturating_add(1));
    if text.is_empty() {
        out.push(CaretPos {
            byte: 0,
            x: 0.0,
            line: 0,
            line_top: 0.0,
            line_height,
        });
        return out;
    }

    // Optional base-direction mark. A single leading mark shifts every subsequent
    // byte (including across '\n') by exactly `prefix_len`, so the byte mapping
    // below stays uniform: `byte = prefixed_line_base + glyph.start - prefix_len`.
    // (It only pins the *first* line's visual direction; later lines auto-detect.)
    let prefix = direction_prefix(direction);
    let prefix_len = prefix.len();
    let shaped: Cow<str> = if prefix.is_empty() {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(format!("{prefix}{text}"))
    };

    // Byte offset of the start of each buffer line within the *shaped* (prefixed)
    // string — cosmic's glyph.start/.end are relative to their buffer line.
    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in shaped.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }

    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_wrap(wrap.into());
    buffer.set_size(Some(max_width), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(
        &shaped,
        &Attrs::new().family(family),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    // Map a shaped (prefixed) absolute byte back to the caller's text. The mark
    // glyph itself sits in `[0, prefix_len)`; skip it.
    let to_orig = |abs: usize| abs.saturating_sub(prefix_len);

    // Visual line ordinal: layout_runs() yields runs top-to-bottom; each run is
    // one visual line (a wrapped buffer line produces several consecutive runs).
    let mut visual_line = 0usize;
    for run in buffer.layout_runs() {
        let line_base = line_starts.get(run.line_i).copied().unwrap_or(0);
        let lt = run.line_top;
        let lh = run.line_height;

        // Leading caret at x=0 for the start of this visual line (covers blank
        // lines from "\n\n", whose run has no glyphs). Skip the zero-width
        // direction mark when picking the first real glyph's byte.
        let first_real = run
            .glyphs
            .iter()
            .find(|g| line_base + g.start >= prefix_len);
        let line_start_byte = to_orig(line_base + first_real.map(|g| g.start).unwrap_or(0));
        out.push(CaretPos {
            byte: line_start_byte,
            x: 0.0,
            line: visual_line,
            line_top: lt,
            line_height: lh,
        });

        for g in run.glyphs.iter() {
            let abs = line_base + g.start;
            if abs < prefix_len {
                continue; // the direction mark — not a caret stop
            }
            let b = to_orig(abs);
            // Record the first time we see each byte on this run.
            if out.last().map(|p| p.byte) != Some(b) {
                out.push(CaretPos {
                    byte: b,
                    x: g.x,
                    line: visual_line,
                    line_top: lt,
                    line_height: lh,
                });
            }
        }

        // Run-end caret (x = line width). For a hard newline this is the byte of
        // the '\n'; for a soft wrap it duplicates the next line's start byte.
        if let Some(last) = run.glyphs.last() {
            let end_b = to_orig(line_base + last.end);
            if out.last().map(|p| p.byte) != Some(end_b) {
                out.push(CaretPos {
                    byte: end_b,
                    x: run.line_w,
                    line: visual_line,
                    line_top: lt,
                    line_height: lh,
                });
            }
        }

        visual_line += 1;
    }

    // Ensure the final byte index is always addressable (e.g. text with no
    // trailing newline whose last run-end already covers it is a no-op).
    let last_byte = text.len();
    if out.last().map(|p| p.byte).unwrap_or(0) < last_byte {
        let lp = *out.last().unwrap();
        out.push(CaretPos {
            byte: last_byte,
            x: lp.x,
            line: lp.line,
            line_top: lp.line_top,
            line_height: lp.line_height,
        });
    }

    out
}

/// The caret position at (or just after) `byte`: the first entry whose byte is
/// `>= byte`, falling back to the last entry. Valid cursor positions always have
/// an exact match because every cluster boundary is recorded.
pub fn caret_for_byte(layout: &[CaretPos], byte: usize) -> CaretPos {
    layout
        .iter()
        .copied()
        .find(|p| p.byte >= byte)
        .or_else(|| layout.last().copied())
        .unwrap_or(CaretPos {
            byte: 0,
            x: 0.0,
            line: 0,
            line_top: 0.0,
            line_height: 0.0,
        })
}

/// Hit-test a point (relative to the text block's top-left origin) to a byte
/// offset: pick the visual line whose `[line_top, line_top+line_height)` band
/// brackets `y` (clamping above the first / below the last line), then the
/// nearest caret `x` on that line.
pub fn byte_at_point(layout: &[CaretPos], x: f32, y: f32) -> usize {
    if layout.is_empty() {
        return 0;
    }
    let first = layout.first().unwrap();
    let last = layout.last().unwrap();
    let target_line = if y < first.line_top {
        first.line
    } else if y >= last.line_top + last.line_height {
        last.line
    } else {
        layout
            .iter()
            .find(|p| y >= p.line_top && y < p.line_top + p.line_height)
            .map(|p| p.line)
            .unwrap_or(last.line)
    };

    let mut best_byte = 0usize;
    let mut best_dx = f32::MAX;
    for p in layout.iter().filter(|p| p.line == target_line) {
        let dx = (p.x - x).abs();
        if dx < best_dx {
            best_dx = dx;
            best_byte = p.byte;
        }
    }
    best_byte
}

/// Move the caret to the visual line `dir` steps away (`-1` up, `+1` down),
/// landing at the caret nearest `desired_x` on that line (sticky-column vertical
/// navigation). Returns `byte` unchanged when already at the top/bottom line.
pub fn byte_on_adjacent_line(layout: &[CaretPos], byte: usize, dir: i32, desired_x: f32) -> usize {
    if layout.is_empty() {
        return byte;
    }
    let cur = caret_for_byte(layout, byte);
    let min_line = layout.first().unwrap().line as i32;
    let max_line = layout.last().unwrap().line as i32;
    let target = cur.line as i32 + dir;
    if target < min_line || target > max_line {
        return byte;
    }
    let target = target as usize;

    let mut best_byte = byte;
    let mut best_dx = f32::MAX;
    for p in layout.iter().filter(|p| p.line == target) {
        let dx = (p.x - desired_x).abs();
        if dx < best_dx {
            best_dx = dx;
            best_byte = p.byte;
        }
    }
    best_byte
}

/// One laid-out glyph cell in **visual** order, the primitive bidi-aware editing
/// builds on. cosmic-text lays glyphs out left-to-right on screen after bidi
/// reordering, so `[x, x + w]` is always the visual cell (positive `w`) regardless
/// of direction; `byte_start`/`byte_end` are the **logical** (ascending) byte range
/// the cell covers in the caller's text (the direction-mark prefix, if any, is
/// already subtracted out). `rtl` is the glyph's own bidi level parity — it can
/// differ from neighbours within one visual line (mixed bidi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualGlyph {
    /// Logical (ascending) start byte of the cell in the caller's text.
    pub byte_start: usize,
    /// Logical (ascending) end byte of the cell in the caller's text.
    pub byte_end: usize,
    /// Visual left edge of the cell (relative to the visual line's left edge).
    pub x: f32,
    /// Visual cell width (always positive, after bidi reordering).
    pub w: f32,
    /// Visual line ordinal the cell sits on, top-to-bottom.
    pub line: usize,
    /// Y of the top of this visual line.
    pub line_top: f32,
    /// Height of this visual line.
    pub line_height: f32,
    /// Whether this glyph's own bidi level is right-to-left (may differ from
    /// neighbours within a mixed-bidi line).
    pub rtl: bool,
}

/// A selection-highlight rectangle, relative to the text block's top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelRect {
    /// Left edge, relative to the text block's top-left origin.
    pub x: f32,
    /// Top edge, relative to the text block's top-left origin.
    pub y: f32,
    /// Width of the highlight rectangle.
    pub w: f32,
    /// Height of the highlight rectangle.
    pub h: f32,
}

/// Lay text out and return one [`VisualGlyph`] per shaped glyph, in visual order,
/// carrying the bidi level and logical byte range of each cell. This is the
/// keystone for bidi-aware caret movement ([`visual_caret_neighbor`]) and
/// selection rectangles ([`selection_rects`]); both are pure functions over the
/// returned slice and need no `FontSystem`.
///
/// `direction` forces the base paragraph direction via a leading mark (see
/// [`TextDirection`]); the mark glyph is filtered out and byte offsets are mapped
/// back to the caller's text.
#[allow(clippy::too_many_arguments)]
pub fn text_visual_layout(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    wrap: WrapMode,
    family_name: Option<&str>,
    direction: TextDirection,
) -> Vec<VisualGlyph> {
    let mut out: Vec<VisualGlyph> = Vec::new();
    if text.is_empty() {
        return out;
    }

    let prefix = direction_prefix(direction);
    let prefix_len = prefix.len();
    let shaped: Cow<str> = if prefix.is_empty() {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(format!("{prefix}{text}"))
    };

    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in shaped.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }

    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_wrap(wrap.into());
    buffer.set_size(Some(max_width), None);
    let family = family_name.map(Family::Name).unwrap_or(Family::SansSerif);
    buffer.set_text(
        &shaped,
        &Attrs::new().family(family),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    let to_orig = |abs: usize| abs.saturating_sub(prefix_len);
    let mut visual_line = 0usize;
    for run in buffer.layout_runs() {
        let line_base = line_starts.get(run.line_i).copied().unwrap_or(0);
        let lt = run.line_top;
        let lh = run.line_height;
        for g in run.glyphs.iter() {
            let abs = line_base + g.start;
            if abs < prefix_len {
                continue; // the zero-width direction mark
            }
            let a = to_orig(abs);
            let b = to_orig(line_base + g.end);
            out.push(VisualGlyph {
                byte_start: a.min(b),
                byte_end: a.max(b),
                x: g.x,
                w: g.w,
                line: visual_line,
                line_top: lt,
                line_height: lh,
                rtl: g.level.is_rtl(),
            });
        }
        visual_line += 1;
    }
    out
}

/// Selection-highlight rectangles for the logical byte range `[sel_start, sel_end)`
/// over a [`text_visual_layout`]. Per visual line, the glyphs whose logical range
/// overlaps the selection are taken at their visual extents `[x, x+w]`, sorted, and
/// merged into contiguous spans — so a selection that straddles an LTR↔RTL boundary
/// yields the several disjoint rectangles it visually occupies, not one bogus span.
pub fn selection_rects(glyphs: &[VisualGlyph], sel_start: usize, sel_end: usize) -> Vec<SelRect> {
    let mut out: Vec<SelRect> = Vec::new();
    if sel_start >= sel_end || glyphs.is_empty() {
        return out;
    }
    let max_line = glyphs.iter().map(|g| g.line).max().unwrap_or(0);
    const EPS: f32 = 0.5; // merge near-touching advances into one rect
    for line in 0..=max_line {
        let mut spans: Vec<(f32, f32, f32, f32)> = glyphs
            .iter()
            .filter(|g| g.line == line && g.byte_start < sel_end && g.byte_end > sel_start)
            .map(|g| (g.x, g.x + g.w, g.line_top, g.line_height))
            .collect();
        if spans.is_empty() {
            continue;
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (lt, lh) = (spans[0].2, spans[0].3);
        let mut cur = (spans[0].0, spans[0].1);
        for &(s, e, _, _) in &spans[1..] {
            if s <= cur.1 + EPS {
                cur.1 = cur.1.max(e);
            } else {
                out.push(SelRect {
                    x: cur.0,
                    y: lt,
                    w: cur.1 - cur.0,
                    h: lh,
                });
                cur = (s, e);
            }
        }
        out.push(SelRect {
            x: cur.0,
            y: lt,
            w: cur.1 - cur.0,
            h: lh,
        });
    }
    out
}

/// The byte offset the caret lands on when moved one step in the **visual**
/// direction (`dir < 0` = screen-left, `dir > 0` = screen-right) from `cursor_byte`,
/// over a [`text_visual_layout`]. Caret stops are the visual edges of each glyph
/// cell, mapped to a byte by the glyph's own bidi level (LTR: `start` at the left
/// edge, `end` at the right; RTL: the reverse), so Left/Right always move the caret
/// the way it moves on screen even across direction runs. Wraps to the adjacent
/// visual line's extreme at a line end. Returns `cursor_byte` unchanged when there
/// is nowhere to go.
///
/// Caret **affinity** at a direction boundary (one screen x mapping to two byte
/// positions) is resolved deterministically — leftmost occurrence for a left move,
/// rightmost for a right move — rather than tracked as cursor state; pixel-perfect
/// affinity is a documented v1 limitation.
pub fn visual_caret_neighbor(glyphs: &[VisualGlyph], cursor_byte: usize, dir: i32) -> usize {
    if glyphs.is_empty() || dir == 0 {
        return cursor_byte;
    }
    let stops = caret_stops(glyphs);

    // Current caret position: among stops for this byte, take the leftmost for a
    // left move and the rightmost for a right move (affinity tie-break).
    let cur = if dir < 0 {
        stops
            .iter()
            .filter(|s| s.byte == cursor_byte)
            .min_by(|a, b| a.x.total_cmp(&b.x))
    } else {
        stops
            .iter()
            .filter(|s| s.byte == cursor_byte)
            .max_by(|a, b| a.x.total_cmp(&b.x))
    };
    let Some(&CaretStop { x: cur_x, line, .. }) = cur else {
        return cursor_byte;
    };
    const EPS: f32 = 0.01;

    // Nearest stop strictly in the visual direction on the same line.
    let same_line = if dir < 0 {
        stops
            .iter()
            .filter(|s| s.line == line && s.x < cur_x - EPS)
            .max_by(|a, b| a.x.total_cmp(&b.x))
    } else {
        stops
            .iter()
            .filter(|s| s.line == line && s.x > cur_x + EPS)
            .min_by(|a, b| a.x.total_cmp(&b.x))
    };
    if let Some(s) = same_line {
        return s.byte;
    }

    // Off the end of the line: wrap to the adjacent visual line's extreme.
    let target = line as i32 + dir.signum();
    if target < 0 {
        return cursor_byte;
    }
    let target = target as usize;
    let wrapped = if dir < 0 {
        stops
            .iter()
            .filter(|s| s.line == target)
            .max_by(|a, b| a.x.total_cmp(&b.x))
    } else {
        stops
            .iter()
            .filter(|s| s.line == target)
            .min_by(|a, b| a.x.total_cmp(&b.x))
    };
    wrapped.map(|s| s.byte).unwrap_or(cursor_byte)
}

/// A direction-aware caret stop: the byte that begins at visual position `x` on
/// visual line `line`. Each glyph cell contributes two stops (its two visual
/// edges); for an RTL cell the logical-start byte is the *right* edge and the
/// logical-end byte the *left* edge (the reverse of an LTR cell).
#[derive(Debug, Clone, Copy)]
struct CaretStop {
    byte: usize,
    x: f32,
    line: usize,
    line_top: f32,
    line_height: f32,
}

/// Build the level-aware caret stops for a visual glyph layout — the shared
/// basis for [`visual_caret_neighbor`] (visual cursor movement) and
/// [`visual_caret_pos`] (edge-correct caret rendering).
fn caret_stops(glyphs: &[VisualGlyph]) -> Vec<CaretStop> {
    let mut stops = Vec::with_capacity(glyphs.len() * 2);
    for g in glyphs {
        let (left_byte, right_byte) = if g.rtl {
            (g.byte_end, g.byte_start)
        } else {
            (g.byte_start, g.byte_end)
        };
        stops.push(CaretStop {
            byte: left_byte,
            x: g.x,
            line: g.line,
            line_top: g.line_top,
            line_height: g.line_height,
        });
        stops.push(CaretStop {
            byte: right_byte,
            x: g.x + g.w,
            line: g.line,
            line_top: g.line_top,
            line_height: g.line_height,
        });
    }
    stops
}

/// Edge-correct caret geometry for laid-out text.
///
/// `text_caret_layout`/`text_cursor_positions` place every caret at a glyph
/// cell's **left** edge (`byte = glyph.start → x = glyph.x`), which is wrong for
/// RTL glyphs — there the logical-start byte sits at the cell's *right* edge. A
/// caret drawn from those tables would land on the wrong side in RTL/bidi text.
/// This returns the visual position where `byte` *logically begins*, honouring
/// each glyph's resolved direction, so a rendered caret matches the visual
/// movement produced by [`visual_caret_neighbor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualCaret {
    /// Visual x where the byte logically begins (relative to the visual line's
    /// left edge), edge-corrected for the glyph's direction.
    pub x: f32,
    /// Visual line ordinal the caret sits on, top-to-bottom.
    pub line: usize,
    /// Y of the top of this visual line.
    pub line_top: f32,
    /// Height of this visual line.
    pub line_height: f32,
}

/// Resolve the edge-correct caret geometry for `byte` against a visual glyph
/// layout. Returns `None` when the layout is empty or no glyph boundary matches
/// `byte` (the caller should fall back to a line-start position). When a byte
/// has two stops (a direction boundary — affinity), the leftmost is chosen
/// deterministically, consistent with the soft-wrap affinity note on
/// [`text_caret_layout`].
pub fn visual_caret_pos(glyphs: &[VisualGlyph], byte: usize) -> Option<VisualCaret> {
    if glyphs.is_empty() {
        return None;
    }
    let stops = caret_stops(glyphs);
    stops
        .iter()
        .filter(|s| s.byte == byte)
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .map(|s| VisualCaret {
            x: s.x,
            line: s.line,
            line_top: s.line_top,
            line_height: s.line_height,
        })
}

/// Truncate `content` to a single line that fits within `max_width`, appending a
/// trailing `'…'`. Returns `content` unchanged when it already fits.
///
/// Shapes with no wrapping and reads the laid-out glyph positions to find the
/// byte cutoff, so it costs at most two extra shaping passes (the content and the
/// ellipsis) and only for blocks that actually overflow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ellipsize_to_width(
    fs: &mut FontSystem,
    content: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    family: Family,
    weight: Weight,
    style: Style,
    letter_spacing: f32,
) -> String {
    match ellipsis_cut(
        fs,
        content,
        font_size,
        line_height,
        max_width,
        family,
        weight,
        style,
        letter_spacing,
    ) {
        None => content.to_string(),
        Some(cut) => {
            let mut s = content[..cut].to_string();
            s.push('…');
            s
        }
    }
}

/// Where [`ellipsize_to_width`] cuts `content`: `None` when it fits in
/// `max_width`, otherwise the length in bytes of the part kept before the
/// `'…'` (trailing whitespace dropped).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ellipsis_cut(
    fs: &mut FontSystem,
    content: &str,
    font_size: f32,
    line_height: f32,
    max_width: f32,
    family: Family,
    weight: Weight,
    style: Style,
    letter_spacing: f32,
) -> Option<usize> {
    if content.is_empty() || !max_width.is_finite() || max_width <= 0.0 {
        return None;
    }
    let metrics = Metrics::new(font_size, line_height);
    let attrs = || {
        Attrs::new()
            .family(family)
            .weight(weight)
            .style(style)
            .letter_spacing(letter_spacing_em(letter_spacing, font_size))
    };

    // Shape the full content on a single line.
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    buffer.set_text(content, &attrs(), Shaping::Advanced, None);
    buffer.shape_until_scroll(fs, false);

    let full_w = buffer
        .layout_runs()
        .map(|r| r.line_w)
        .fold(0.0_f32, f32::max);
    if full_w <= max_width {
        return None;
    }

    // Width of the ellipsis at this size/family, reserved at the right edge.
    let mut ell = Buffer::new(fs, metrics);
    ell.set_wrap(Wrap::None);
    ell.set_size(None, None);
    ell.set_text("…", &attrs(), Shaping::Advanced, None);
    ell.shape_until_scroll(fs, false);
    let ellipsis_w = ell.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max);

    let budget = max_width - ellipsis_w;
    if budget <= 0.0 {
        return Some(0);
    }

    // Largest byte offset whose glyph still fits within the budget. Take the max
    // over all fitting glyphs (shaping/ligatures need not be end-ordered).
    let mut cut = 0usize;
    for run in buffer.layout_runs() {
        for g in run.glyphs {
            if g.x + g.w <= budget {
                cut = cut.max(g.end);
            }
        }
    }
    let cut = cut.min(content.len());
    Some(content[..cut].trim_end().len())
}

/// How a [`TextSpan`] is underlined.
///
/// The underline rect is emitted as a coloured soup quad at
/// [`DrawList::text`](crate::DrawList::text) time so it renders beneath the MSDF
/// glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Underline {
    /// No underline (default).
    #[default]
    None,
    /// Underline using the span's text colour — its [`color`](TextSpan::color),
    /// or the block colour when the span sets none. The common case: an
    /// underline that tracks whatever colour the text is.
    Inherit,
    /// Underline with an explicit `[r, g, b, a]` colour (`0.0..=1.0`), regardless
    /// of the glyph colour — for a contrasting underline rule.
    Color([f32; 4]),
}

impl From<[f32; 4]> for Underline {
    /// An explicit `[r, g, b, a]` colour becomes [`Underline::Color`].
    fn from(c: [f32; 4]) -> Self {
        Underline::Color(c)
    }
}

/// A colour/underline override over a half-open byte range of a [`TextBlock`]'s
/// existing [`TextBlock::content`]. Ranges avoid copying text into one `String`
/// per token and should be sorted, non-overlapping, and on UTF-8 boundaries.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyleRange {
    /// Half-open byte range in [`TextBlock::content`].
    pub range: std::ops::Range<usize>,
    /// Per-range fill colour, or `None` to inherit the block colour.
    pub color: Option<[f32; 4]>,
    /// Underline style for this range.
    pub underline: Underline,
}

/// A run of text within a [`TextBlock`] with optional per-span colour and
/// underline overrides.
///
/// **V1 constraint**: spans must share the block's global font attributes
/// (size, weight, style, family) — only colour and underline may vary. Mixed
/// font-size / weight spans require a `set_rich_text` shaping path and are
/// deferred to a future version.
#[derive(Debug, Clone, Default)]
pub struct TextSpan {
    /// The text content of this span.
    pub text: String,
    /// Per-span fill colour as `[r, g, b, a]` in `0.0..=1.0`. `None` →
    /// inherit the block's colour.
    pub color: Option<[f32; 4]>,
    /// Underline style. [`Underline::None`] (default) draws nothing;
    /// [`Underline::Inherit`] tracks the text colour; [`Underline::Color`]
    /// overrides it.
    pub underline: Underline,
}

/// Crisp outline drawn around glyphs, composited under the fill. Maps to
/// Teardown's `UiTextOutline(r, g, b, a, thickness)`.
#[derive(Clone, Copy, Debug)]
pub struct TextOutline {
    /// Outline colour.
    pub color: Color,
    /// Outline thickness in screen pixels.
    pub width_px: f32,
}

/// Drop shadow drawn offset behind the text. Maps to Teardown's
/// `UiTextShadow(r, g, b, a, distance, blur)`.
#[derive(Clone, Copy, Debug)]
pub struct TextShadow {
    /// Shadow colour.
    pub color: Color,
    /// Screen-space offset `[dx, dy]`.
    pub offset: [f32; 2],
    /// Edge softness (blur) in screen pixels.
    pub softness: f32,
}

/// Soft colored halo around glyphs (a wide, soft, fill-less outline).
#[derive(Clone, Copy, Debug)]
pub struct TextGlow {
    /// Halo colour.
    pub color: Color,
    /// Halo radius in screen pixels.
    pub radius_px: f32,
}

/// Line-box-height multiple applied to `font_size` by [`TextBlock::with_size`]
/// (and the text measurer). A single line of text is shaped into a box this tall,
/// so any vertical centring must centre *this* height — not `font_size` — or the
/// glyphs drift toward the bottom of the container.
pub const LINE_HEIGHT_RATIO: f32 = 1.25;

/// Top `y` for a single-line text block of `font_size` (shaped with the default
/// `LINE_HEIGHT_RATIO` line box, as [`TextBlock::with_size`] / `Theme::text`
/// do) so its line box is vertically centred over the span `[top, top + height]`.
///
/// Centring by `font_size` alone leaves the line box sitting low, so the visible
/// glyphs drift to the bottom on short containers (tab bars, drag-handle title
/// bars, table rows). The text is shaped into the full `font_size * LINE_HEIGHT_RATIO`
/// box, and cosmic-text centres the glyph (ascent+descent) box within that line
/// box, so centring the line box centres the glyphs exactly — no per-font metrics
/// needed.
pub fn vcentered_line_y(top: f32, height: f32, font_size: f32) -> f32 {
    top + (height - font_size * LINE_HEIGHT_RATIO) / 2.0
}

/// A block of text to render.
#[derive(Clone)]
pub struct TextBlock {
    /// The text to render. When [`spans`](Self::spans) is non-empty, this is
    /// derived from the concatenated span texts at draw time.
    pub content: String,
    /// Left edge of the block (the pen origin, in screen pixels).
    pub x: f32,
    /// Top edge of the block (in screen pixels).
    pub y: f32,
    /// Font size in pixels.
    pub font_size: f32,
    /// Line-box height in pixels (usually `font_size * LINE_HEIGHT_RATIO`).
    pub line_height: f32,
    /// Layout box width in pixels; wrapping and alignment are relative to this.
    pub max_width: f32,
    /// Additional spacing between glyphs in pixels (default `0.0`).
    pub letter_spacing: f32,
    /// Global fill colour (overridden per-run by coloured [`spans`](Self::spans)).
    pub color: Color,
    /// Optional clip rectangle; glyphs outside it are not drawn.
    pub clip: Option<Rect>,
    /// Optional crisp outline (off by default).
    pub outline: Option<TextOutline>,
    /// Optional drop shadow (off by default).
    pub shadow: Option<TextShadow>,
    /// Optional soft glow halo (off by default).
    pub glow: Option<TextGlow>,
    /// Font to shape this block in. `None` = the default system sans-serif.
    pub font: Option<FontHandle>,
    /// Horizontal alignment within `max_width` (default
    /// [`Start`](TextAlign::Start), i.e. the reading start).
    pub align: TextAlign,
    /// Base paragraph direction (default [`Auto`](TextDirection::Auto)). Bidi
    /// reordering of mixed scripts is automatic regardless; this only forces the
    /// base direction for direction-neutral content.
    pub direction: TextDirection,
    /// Single-line ellipsis mode: when `true`, the block is laid out on one line
    /// (no wrapping) and truncated with a trailing `'…'` if it would exceed
    /// `max_width`. When `false` (default) the block wraps at `max_width`.
    pub ellipsize: bool,
    /// Font weight selector (default [`Weight::NORMAL`]). Picks the matching face
    /// (e.g. [`Weight::BOLD`]) from the block's family during shaping; cosmic-text
    /// selects a real face and does **not** synthesize faux-bold when absent.
    pub weight: Weight,
    /// Font style selector (default [`Style::Normal`]). [`Style::Italic`] /
    /// [`Style::Oblique`] pick the matching face from the family when present.
    pub style: Style,
    /// Inline text spans for per-run colour and underline overrides. When
    /// non-empty, `content` is derived from the concatenation of span texts at
    /// draw time and need not be set by the caller. All spans must share the
    /// block's global font attributes (see [`TextSpan`]).
    pub spans: Vec<TextSpan>,
    /// Sorted byte-range style overrides over `content`. Unlike `spans`, these
    /// retain the original content and allocate no string per styled token.
    pub style_ranges: std::sync::Arc<Vec<TextStyleRange>>,
    /// Draw-list tint applied lazily to range colours during glyph placement.
    /// Keeping it separate preserves shared range storage under tinted scopes.
    pub style_range_tint: [f32; 4],
    /// Line-wrapping policy when the content exceeds `max_width` (default
    /// [`WrapMode::WordOrGlyph`], matching the historical implicit behaviour).
    /// Ignored in `ellipsize` mode, which always lays out on a single line.
    pub wrap: WrapMode,
    /// Vertical (stacked) text mode (default `false`). When `true`, each grapheme
    /// cluster is laid out on its own row so the label reads top-to-bottom, with
    /// glyphs centered within the column — the casual look used for Japanese game
    /// labels. This is upright stacking, **not** true CJK `vertical-rl` (no
    /// vertical glyph variants, rotated kana/punctuation, or right-to-left
    /// columns). `direction`, `wrap`, and `ellipsize` do not apply in this mode,
    /// but [`align`](Self::with_align) still positions the whole column
    /// horizontally within [`max_width`](Self::with_max_width) (`Start`/`Left`,
    /// `Center`, `End`/`Right`). See [`with_vertical`](Self::with_vertical).
    pub vertical: bool,
}

impl TextBlock {
    /// A white block at `(x, y)` with default metrics (16px, 1.25× line height,
    /// 800px max width) and no effects. Use the `with_*` builders to customize.
    pub fn new(content: impl Into<String>, x: f32, y: f32) -> Self {
        Self {
            content: content.into(),
            x,
            y,
            font_size: 16.0,
            line_height: 20.0,
            max_width: 800.0,
            letter_spacing: 0.0,
            color: Color::rgb(255, 255, 255),
            clip: None,
            outline: None,
            shadow: None,
            glow: None,
            font: None,
            align: TextAlign::default(),
            direction: TextDirection::default(),
            ellipsize: false,
            weight: Weight::NORMAL,
            style: Style::Normal,
            spans: Vec::new(),
            style_ranges: std::sync::Arc::new(Vec::new()),
            style_range_tint: [1.0; 4],
            wrap: WrapMode::default(),
            vertical: false,
        }
    }

    /// Set the font size (px); `line_height` is derived as `size *
    /// LINE_HEIGHT_RATIO`.
    pub fn with_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self.line_height = size * LINE_HEIGHT_RATIO;
        self
    }

    /// Set the line-box height in pixels (call after
    /// [`with_size`](Self::with_size), which resets it to
    /// `size × LINE_HEIGHT_RATIO`).
    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    /// Set the layout box width (px) that wrapping and alignment are relative to.
    pub fn with_max_width(mut self, width: f32) -> Self {
        self.max_width = width;
        self
    }

    /// Set additional spacing between glyphs in pixels.
    pub fn with_letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.letter_spacing = letter_spacing;
        self
    }

    /// Set the opaque fill colour from 8-bit RGB components.
    pub fn with_color(mut self, r: u8, g: u8, b: u8) -> Self {
        self.color = Color::rgb(r, g, b);
        self
    }

    /// Set the fill colour from 8-bit RGBA components (with alpha).
    pub fn with_rgba(mut self, r: u8, g: u8, b: u8, a: u8) -> Self {
        self.color = Color::rgba(r, g, b, a);
        self
    }

    /// Set the fill colour from a straight sRGB-encoded `[r, g, b, a]` in
    /// `0.0..=1.0` — the same convention as every other colour in the crate
    /// (see [`crate::color`]), so theme/[`StyleKey`](crate::StyleKey) colours
    /// pass straight through. Channels are clamped and rounded to 8 bits;
    /// alpha is kept.
    pub fn with_color_f32(mut self, color: [f32; 4]) -> Self {
        self.color = crate::color::text_color(color);
        self
    }

    /// Clip glyphs to `clip`; anything outside the rectangle is not drawn.
    pub fn with_clip(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Add a crisp outline of `width_px` screen pixels in the given color.
    pub fn with_outline(mut self, r: u8, g: u8, b: u8, a: u8, width_px: f32) -> Self {
        self.outline = Some(TextOutline {
            color: Color::rgba(r, g, b, a),
            width_px,
        });
        self
    }

    /// Add a drop shadow offset by `(dx, dy)` screen px with `softness` px blur.
    ///
    /// The `(r, g, b, a, dx, dy, softness)` shape mirrors Teardown's
    /// `UiTextShadow(r, g, b, a, distance, blur)` for a direct binding mapping.
    #[allow(clippy::too_many_arguments)]
    pub fn with_shadow(
        mut self,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
        dx: f32,
        dy: f32,
        softness: f32,
    ) -> Self {
        self.shadow = Some(TextShadow {
            color: Color::rgba(r, g, b, a),
            offset: [dx, dy],
            softness,
        });
        self
    }

    /// Add a soft glow halo of `radius_px` screen pixels.
    pub fn with_glow(mut self, r: u8, g: u8, b: u8, a: u8, radius_px: f32) -> Self {
        self.glow = Some(TextGlow {
            color: Color::rgba(r, g, b, a),
            radius_px,
        });
        self
    }

    /// Shape this block in `font` (from [`load_font_file`] / [`load_font_bytes`])
    /// instead of the default system sans-serif.
    pub fn with_font(mut self, font: FontHandle) -> Self {
        self.font = Some(font);
        self
    }

    /// Set the horizontal alignment within `max_width`. `Center`/`Right` only
    /// shift visibly when `max_width` exceeds the longest line.
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Force the base paragraph direction. Bidi reordering of mixed scripts is
    /// automatic regardless; this only pins the base direction for
    /// direction-neutral content (digits, punctuation, leading Latin in an RTL UI).
    pub fn with_direction(mut self, direction: TextDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Lay this block out as **vertical (stacked) text**: each grapheme cluster
    /// on its own row, top-to-bottom, glyphs centered within the column. This is
    /// the casual upright stacking used for Japanese game labels — *not* true CJK
    /// `vertical-rl` (no vertical glyph variants, rotated kana/punctuation, or
    /// right-to-left columns); embedded Latin stacks per-letter too.
    ///
    /// The row pitch is the block's `line_height` (set via [`with_size`](Self::with_size));
    /// a tighter `line_height` reads better for full-width kana/kanji. `direction`,
    /// `wrap`, and ellipsis do not apply in this mode, but [`with_align`](Self::with_align)
    /// still places the column horizontally within [`with_max_width`](Self::with_max_width)
    /// (e.g. `Center` to center a stacked label in a fixed-width slot).
    pub fn with_vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    /// Enable single-line ellipsis: lay the text out on one line and truncate it
    /// with a trailing `'…'` if it would exceed [`Self::max_width`]. Use this for
    /// labels that must stay inside a fixed-width box (set `max_width` to that
    /// box's inner width).
    pub fn with_ellipsis(mut self) -> Self {
        self.ellipsize = true;
        self
    }

    /// Shape this block in `font` when `font` is `Some`, leaving the current font
    /// unchanged on `None`. Lets callers thread an optional theme font through
    /// without a branch at every call site.
    pub fn with_font_opt(mut self, font: Option<FontHandle>) -> Self {
        if let Some(f) = font {
            self.font = Some(f);
        }
        self
    }

    /// Render this block bold (shorthand for `with_weight(Weight::BOLD)`).
    pub fn bold(mut self) -> Self {
        self.weight = Weight::BOLD;
        self
    }

    /// Render this block italic (shorthand for `with_style(Style::Italic)`).
    pub fn italic(mut self) -> Self {
        self.style = Style::Italic;
        self
    }

    /// Select a specific font weight (e.g. `Weight::BOLD`, `Weight(500)`).
    pub fn with_weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Select a specific font style (`Style::Normal`/`Italic`/`Oblique`).
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Replace the block's text with the given inline spans. The display
    /// `content` is derived automatically as the concatenation of all span
    /// texts at draw time. See [`TextSpan`] for the V1 constraint (all spans
    /// must share the block's global font attributes).
    pub fn with_spans(mut self, spans: Vec<TextSpan>) -> Self {
        self.spans = spans;
        self.style_ranges = std::sync::Arc::new(Vec::new());
        self
    }

    /// Apply sorted byte-range styles without replacing or copying `content`.
    /// Invalid/out-of-order ranges are ignored defensively by colour resolution;
    /// callers should produce non-overlapping ranges on UTF-8 boundaries.
    pub fn with_style_ranges(mut self, ranges: Vec<TextStyleRange>) -> Self {
        self.style_ranges = std::sync::Arc::new(ranges);
        self.spans.clear();
        self
    }

    /// Apply shared sorted byte-range styles. Cloning the `Arc` is constant time,
    /// making this suitable for retained editor caches submitted every frame.
    pub fn with_shared_style_ranges(mut self, ranges: std::sync::Arc<Vec<TextStyleRange>>) -> Self {
        self.style_ranges = ranges;
        self.spans.clear();
        self
    }

    /// Set the line-wrapping policy (default [`WrapMode::WordOrGlyph`]). Use
    /// [`WrapMode::None`] to keep the text on one line and overflow `max_width`
    /// (pair with [`with_clip`](Self::with_clip) to hide the overflow).
    pub fn with_wrap(mut self, wrap: WrapMode) -> Self {
        self.wrap = wrap;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaretPos, FontHandle, FontVMetrics, LINE_HEIGHT_RATIO, MsdfVertex, SelRect, TextAlign,
        TextBlock, TextDirection, TextMeasurer, TextRenderer, TextSpan, TextStyleRange, Underline,
        VisualGlyph, WrapMode, byte_at_point, byte_on_adjacent_line, caret_for_byte, color_to_rgba,
        cosmic_align, direction_prefix, ellipsize_to_width, field_reach, has_cjk, has_lowercase,
        load_font_bytes, resolve_range_color, resolve_span_color, selection_rects,
        shared_font_system, text_caret_layout, text_cursor_positions, text_visual_layout,
        vcentered_line_y, vertical_stack_string, visual_caret_neighbor,
    };
    use crate::shaping::{LayoutSpec, LayoutStats, shape_layout};
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Style, Weight};

    /// The shared layout cache behind a measurer.
    fn layout_stats(measurer: &TextMeasurer) -> LayoutStats {
        measurer.font_system_handle().lock().unwrap().layout_stats()
    }

    // ---- TextSpan / resolve_span_color ----

    fn red() -> [f32; 4] {
        [1.0, 0.0, 0.0, 1.0]
    }
    fn green() -> [f32; 4] {
        [0.0, 1.0, 0.0, 1.0]
    }
    fn blue() -> [f32; 4] {
        [0.0, 0.0, 1.0, 1.0]
    }

    #[test]
    fn resolve_span_color_picks_correct_span_for_each_byte() {
        // Spans: "Hello" (red) | " " (no color) | "World" (blue)
        // Bytes: 0..5            5..6              6..11
        let spans = vec![
            TextSpan {
                text: "Hello".into(),
                color: Some(red()),
                underline: Underline::None,
            },
            TextSpan {
                text: " ".into(),
                color: None,
                underline: Underline::None,
            },
            TextSpan {
                text: "World".into(),
                color: Some(blue()),
                underline: Underline::None,
            },
        ];
        // First span: bytes 0–4
        assert_eq!(resolve_span_color(0, &spans), Some(red()));
        assert_eq!(resolve_span_color(4, &spans), Some(red()));
        // Second span: byte 5, color None
        assert_eq!(resolve_span_color(5, &spans), None);
        // Third span: bytes 6–10
        assert_eq!(resolve_span_color(6, &spans), Some(blue()));
        assert_eq!(resolve_span_color(10, &spans), Some(blue()));
    }

    #[test]
    fn resolve_span_color_empty_spans_returns_none() {
        assert_eq!(resolve_span_color(0, &[]), None);
    }

    #[test]
    fn resolve_range_color_binary_searches_sorted_ranges() {
        let ranges = vec![
            super::TextStyleRange {
                range: 0..5,
                color: Some(red()),
                underline: Underline::None,
            },
            super::TextStyleRange {
                range: 8..12,
                color: Some(blue()),
                underline: Underline::None,
            },
        ];
        assert_eq!(resolve_range_color(4, &ranges), Some(red()));
        assert_eq!(resolve_range_color(5, &ranges), None);
        assert_eq!(resolve_range_color(9, &ranges), Some(blue()));
        assert_eq!(resolve_range_color(12, &ranges), None);
    }

    #[test]
    fn resolve_span_color_all_no_color_returns_none() {
        let spans = vec![
            TextSpan {
                text: "abc".into(),
                color: None,
                underline: Underline::None,
            },
            TextSpan {
                text: "def".into(),
                color: None,
                underline: Underline::None,
            },
        ];
        assert_eq!(resolve_span_color(0, &spans), None);
        assert_eq!(resolve_span_color(3, &spans), None);
    }

    #[test]
    fn resolve_span_color_multibyte_utf8_boundary() {
        // "café" is 5 bytes (c-a-f-é where é = 2 bytes)
        let spans = vec![
            TextSpan {
                text: "café".into(),
                color: Some(red()),
                underline: Underline::None,
            },
            TextSpan {
                text: "!".into(),
                color: Some(green()),
                underline: Underline::None,
            },
        ];
        // 'é' is at byte offset 3 (0xc3 0xa9), so byte 3 and 4 are in first span
        assert_eq!(resolve_span_color(3, &spans), Some(red()));
        assert_eq!(resolve_span_color(4, &spans), Some(red()));
        // '!' is at byte offset 5
        assert_eq!(resolve_span_color(5, &spans), Some(green()));
    }

    // ---- TextBlock::with_spans ----

    #[test]
    fn with_spans_derives_content_from_span_texts() {
        let block = TextBlock::new("", 0.0, 0.0).with_spans(vec![
            TextSpan {
                text: "Hello".into(),
                color: None,
                underline: Underline::None,
            },
            TextSpan {
                text: " ".into(),
                color: None,
                underline: Underline::None,
            },
            TextSpan {
                text: "World".into(),
                color: None,
                underline: Underline::None,
            },
        ]);
        // Content is derived by DrawList::text at draw time, not in the builder.
        // The builder just stores the spans; the content field is the caller's
        // responsibility or derived from spans at draw time.
        assert_eq!(block.spans.len(), 3);
        assert_eq!(block.spans[0].text, "Hello");
        assert_eq!(block.spans[2].text, "World");
    }

    #[test]
    fn with_spans_empty_vec_is_plain_mode() {
        let block = TextBlock::new("Hello", 0.0, 0.0).with_spans(vec![]);
        assert!(block.spans.is_empty());
        assert_eq!(block.content, "Hello");
    }

    #[test]
    fn color_to_rgba_normalizes_channels() {
        let c = Color::rgba(255, 128, 0, 64);
        let v = color_to_rgba(c);
        assert!((v[0] - 1.0).abs() < 1e-6);
        assert!((v[1] - 128.0 / 255.0).abs() < 1e-6);
        assert!((v[2] - 0.0).abs() < 1e-6);
        assert!((v[3] - 64.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn with_color_f32_round_trips_hex_and_keeps_alpha() {
        // A hex colour survives the f32 round-trip exactly (rounding, not the
        // truncation `as u8` would do: 0xbf/255*255 can land just below 191).
        let block = TextBlock::new("x", 0.0, 0.0)
            .with_color_f32(crate::color::rgba8([0x3e, 0xbf, 0xc6], 0.5));
        assert_eq!(block.color, Color::rgba(0x3e, 0xbf, 0xc6, 128));
        // Out-of-range channels clamp instead of wrapping.
        let block = TextBlock::new("x", 0.0, 0.0).with_color_f32([1.5, -0.2, 0.5, 1.0]);
        assert_eq!(block.color, Color::rgba(255, 0, 128, 255));
    }

    #[test]
    fn field_reach_scales_with_font_size() {
        // Reach grows linearly with font size and is zero (clamped) for tiny fonts.
        let small = field_reach(8.0, 12.0, 40.0);
        let large = field_reach(40.0, 12.0, 40.0);
        assert!(large > small);
        // At 40px with px_range 12 / ref 40: 0.5*12*40/40 - 0.5 = 5.5 px.
        assert!((large - 5.5).abs() < 1e-4);
        // Never negative.
        assert_eq!(
            field_reach(0.5, 12.0, 40.0).max(0.0),
            field_reach(0.5, 12.0, 40.0)
        );
        assert!(field_reach(0.1, 12.0, 40.0) >= 0.0);
    }

    #[test]
    fn effect_builders_are_opt_in_and_set_fields() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert!(plain.outline.is_none() && plain.shadow.is_none() && plain.glow.is_none());

        let styled = TextBlock::new("x", 0.0, 0.0)
            .with_outline(0, 0, 0, 255, 2.0)
            .with_shadow(10, 20, 30, 200, 1.0, 2.0, 1.5)
            .with_glow(80, 180, 255, 255, 3.0);
        let o = styled.outline.unwrap();
        assert_eq!(o.width_px, 2.0);
        let s = styled.shadow.unwrap();
        assert_eq!(s.offset, [1.0, 2.0]);
        assert_eq!(s.softness, 1.5);
        let g = styled.glow.unwrap();
        assert_eq!(g.radius_px, 3.0);
    }

    #[test]
    fn measures_text_with_glyphon_layout() {
        let mut measurer = TextMeasurer::new();
        let (hello_width, hello_height) = measurer.measure("Hello", 16.0, None);
        assert!(hello_width > 0.0);
        assert!(hello_height > 0.0);

        let font_size = 16.0;
        let (m_width, _) = measurer.measure("M", font_size, None);
        let approximate_width = "M".len() as f32 * font_size * 0.5;
        assert!((m_width - approximate_width).abs() > f32::EPSILON);
    }

    #[test]
    fn measure_with_max_width_wraps_to_multiple_lines() {
        let mut measurer = TextMeasurer::new();
        let long = "The quick brown fox jumps over the lazy dog repeatedly each morning.";
        let (_, h_unwrapped) = measurer.measure(long, 14.0, None);
        let (_, h_wrapped) = measurer.measure(long, 14.0, Some(80.0));
        assert!(h_wrapped > h_unwrapped);
    }

    /// Helper: number of laid-out lines = height / line_height (font_size*1.25).
    fn line_count(
        measurer: &mut TextMeasurer,
        text: &str,
        size: f32,
        w: f32,
        wrap: WrapMode,
    ) -> u32 {
        let (_, h) = measurer.measure_styled(
            text,
            size,
            Some(w),
            None,
            Weight::NORMAL,
            Style::Normal,
            wrap,
        );
        (h / (size * 1.25)).round() as u32
    }

    #[test]
    fn wrap_mode_controls_line_count() {
        let mut m = TextMeasurer::new();
        let size = 14.0;
        let w = 70.0;

        // A multi-word string narrower than its natural width: every wrapping
        // mode breaks it; `None` keeps it on one line. (Word- vs glyph-packing
        // line *counts* are font-dependent, so we don't compare those two here —
        // see the unbreakable-word case below for that distinction.)
        let words = "alpha beta gamma delta epsilon";
        assert_eq!(
            line_count(&mut m, words, size, w, WrapMode::None),
            1,
            "Wrap::None must stay on a single line",
        );
        assert!(
            line_count(&mut m, words, size, w, WrapMode::Word) > 1,
            "Word should wrap"
        );
        assert!(
            line_count(&mut m, words, size, w, WrapMode::WordOrGlyph) > 1,
            "WordOrGlyph should wrap",
        );
        assert!(
            line_count(&mut m, words, size, w, WrapMode::Glyph) > 1,
            "Glyph should wrap"
        );

        // A single word with no break opportunities: `Word` cannot break it (it
        // overflows on one line) while `Glyph`/`WordOrGlyph` break mid-word.
        // This is the font-independent Word-vs-Glyph distinction.
        let long_word = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            line_count(&mut m, long_word, size, w, WrapMode::Word),
            1,
            "Word wrap cannot split a single long word — it overflows on one line",
        );
        assert!(
            line_count(&mut m, long_word, size, w, WrapMode::Glyph) > 1,
            "Glyph wrap must break a long word across lines",
        );
        assert!(
            line_count(&mut m, long_word, size, w, WrapMode::WordOrGlyph) > 1,
            "WordOrGlyph falls back to glyph breaks for a too-long word",
        );
    }

    #[test]
    fn wrap_mode_default_is_word_or_glyph() {
        // The TextBlock default and the bare `measure` default must agree.
        assert_eq!(WrapMode::default(), WrapMode::WordOrGlyph);
        let block = TextBlock::new("x", 0.0, 0.0);
        assert_eq!(block.wrap, WrapMode::WordOrGlyph);
        let with = block.with_wrap(WrapMode::None);
        assert_eq!(with.wrap, WrapMode::None);

        // `measure` (no wrap arg) must match `measure_styled` with the default,
        // proving the convenience path forwards the same policy.
        let mut m = TextMeasurer::new();
        let text = "alpha beta gamma delta epsilon";
        let bare = m.measure(text, 14.0, Some(70.0));
        let styled = m.measure_styled(
            text,
            14.0,
            Some(70.0),
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        );
        assert_eq!(bare, styled);
    }

    #[test]
    fn a_block_is_measured_at_its_own_line_height() {
        let mut measurer = TextMeasurer::new();
        let block = TextBlock::new("one two three four five six", 0.0, 0.0)
            .with_size(14.0)
            .with_max_width(60.0);
        let (_, default_h) = measurer.measure_block(&block);
        let lines = (default_h / (14.0 * LINE_HEIGHT_RATIO)).round();
        assert!(lines >= 3.0, "wraps to several lines: {lines}");
        let (_, loose_h) = measurer.measure_block(&block.clone().with_line_height(22.0));
        assert!(
            (loose_h - lines * 22.0).abs() < 0.01,
            "{lines} lines of 22px, got {loose_h}"
        );
    }

    #[test]
    fn measuring_text_and_measuring_its_block_share_one_layout() {
        let mut measurer = TextMeasurer::new();
        let start = layout_stats(&measurer);
        let text = measurer.measure("Shared layout", 16.0, Some(800.0));
        // A default block: 800px wide, default face, line height and wrap.
        let block =
            measurer.measure_block(&TextBlock::new("Shared layout", 5.0, 9.0).with_size(16.0));
        assert_eq!(text, block);
        let stats = layout_stats(&measurer);
        assert_eq!(
            (stats.shaped - start.shaped, stats.hits - start.hits),
            (1, 1)
        );
    }

    #[test]
    fn wrap_mode_is_part_of_measure_cache_key() {
        // Same content/metrics, different wrap → distinct cached results (None
        // stays one line, Glyph wraps), so the wrap must be in the key.
        let mut m = TextMeasurer::new();
        let text = "alpha beta gamma delta epsilon";
        let (_, h_none) = m.measure_styled(
            text,
            14.0,
            Some(70.0),
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::None,
        );
        let (_, h_glyph) = m.measure_styled(
            text,
            14.0,
            Some(70.0),
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::Glyph,
        );
        assert!(
            h_glyph > h_none,
            "distinct wrap modes must not collide in the cache"
        );
        // Re-measuring None still returns the one-line height (key really splits).
        let (_, h_none2) = m.measure_styled(
            text,
            14.0,
            Some(70.0),
            None,
            Weight::NORMAL,
            Style::Normal,
            WrapMode::None,
        );
        assert_eq!(h_none, h_none2);
    }

    // ---- text_caret_layout / caret helpers ----

    /// Lay out `text` with a generous width (no wrap unless `\n`) and return the
    /// caret entries. Uses the shared font system (CPU-only — no GPU needed).
    fn caret_layout(text: &str, wrap: WrapMode, max_width: f32) -> Vec<CaretPos> {
        let fsh = shared_font_system();
        let mut shared = fsh.lock().unwrap();
        let fs = shared.font_system();
        text_caret_layout(
            fs,
            text,
            16.0,
            20.0,
            max_width,
            wrap,
            None,
            TextDirection::Auto,
        )
    }

    #[test]
    fn caret_layout_empty_text_has_single_origin_entry() {
        let layout = caret_layout("", WrapMode::None, 1000.0);
        assert_eq!(layout.len(), 1);
        assert_eq!(layout[0].byte, 0);
        assert_eq!(layout[0].x, 0.0);
        assert_eq!(layout[0].line, 0);
    }

    #[test]
    fn caret_layout_newline_splits_into_distinct_visual_lines() {
        // "ab\ncd": line 0 = "ab" (bytes 0,1,2), line 1 = "cd" (bytes 3,4,5).
        let layout = caret_layout("ab\ncd", WrapMode::None, 1000.0);
        let max_line = layout.iter().map(|p| p.line).max().unwrap();
        assert_eq!(max_line, 1, "two visual lines expected");

        // Line 1 must start at x=0 and at a byte after the newline (>=3).
        let line1: Vec<_> = layout.iter().filter(|p| p.line == 1).collect();
        assert!(!line1.is_empty());
        assert_eq!(line1[0].x, 0.0, "second line starts at x=0");
        assert!(
            line1[0].byte >= 3,
            "second line bytes are after the newline"
        );

        // Every byte index in the source is addressable, including the final one.
        assert!(layout.iter().any(|p| p.byte == "ab\ncd".len()));
        // line_top strictly increases between the two lines.
        let top0 = layout.iter().find(|p| p.line == 0).unwrap().line_top;
        let top1 = layout.iter().find(|p| p.line == 1).unwrap().line_top;
        assert!(top1 > top0, "second line sits below the first");
    }

    #[test]
    fn caret_layout_bytes_are_absolute_across_lines() {
        // The keystone correctness property: byte offsets on later lines are
        // absolute into the whole string, NOT relative to the buffer line.
        let text = "hello\nworld";
        let layout = caret_layout(text, WrapMode::None, 1000.0);
        // "world" starts at byte 6 (after "hello\n"). Some caret entry on line 1
        // must reference byte 6, and none may reference a byte < 6 there.
        let line1: Vec<_> = layout.iter().filter(|p| p.line == 1).collect();
        assert!(
            line1.iter().all(|p| p.byte >= 6),
            "line-1 bytes are absolute (>=6)"
        );
        assert!(
            line1.iter().any(|p| p.byte == 6),
            "line 1 begins at absolute byte 6"
        );
        assert!(layout.iter().any(|p| p.byte == text.len()));
    }

    #[test]
    fn caret_layout_blank_line_is_addressable() {
        // "a\n\nb": three buffer lines, the middle one empty. The blank middle
        // line must still get a caret entry at x=0.
        let layout = caret_layout("a\n\nb", WrapMode::None, 1000.0);
        let max_line = layout.iter().map(|p| p.line).max().unwrap();
        assert_eq!(max_line, 2, "three visual lines (incl. the empty middle)");
        let mid: Vec<_> = layout.iter().filter(|p| p.line == 1).collect();
        assert!(!mid.is_empty(), "blank middle line must be addressable");
        assert!(
            mid.iter().all(|p| p.x == 0.0),
            "blank line caret sits at x=0"
        );
        // The blank line's byte is the position just after the first '\n' (byte 2).
        assert!(mid.iter().any(|p| p.byte == 2));
    }

    #[test]
    fn caret_layout_long_line_wraps_into_multiple_lines() {
        // A long unbreakable run forces a glyph wrap at a narrow width → >1 line,
        // each line's carets x-monotonic increasing.
        let text = "abcdefghijklmnopqrstuvwxyz0123456789";
        let layout = caret_layout(text, WrapMode::WordOrGlyph, 60.0);
        let max_line = layout.iter().map(|p| p.line).max().unwrap();
        assert!(max_line >= 1, "narrow width must wrap the long run");
        // Within each visual line, x is non-decreasing.
        for line in 0..=max_line {
            let xs: Vec<f32> = layout
                .iter()
                .filter(|p| p.line == line)
                .map(|p| p.x)
                .collect();
            for w in xs.windows(2) {
                assert!(w[1] >= w[0] - 0.01, "x is monotonic within a line");
            }
        }
    }

    #[test]
    fn caret_for_byte_finds_exact_and_clamps() {
        let layout = caret_layout("ab\ncd", WrapMode::None, 1000.0);
        // Exact match for the first byte.
        assert_eq!(caret_for_byte(&layout, 0).byte, 0);
        // A byte past the end clamps to the last entry.
        let last = *layout.last().unwrap();
        assert_eq!(caret_for_byte(&layout, 9999).byte, last.byte);
    }

    #[test]
    fn byte_at_point_picks_line_by_y_then_nearest_x() {
        let text = "ab\ncd";
        let layout = caret_layout(text, WrapMode::None, 1000.0);
        let lh = layout[0].line_height;
        // A click well into the second line's y band, far left → its start byte.
        let y_line1 = layout.iter().find(|p| p.line == 1).unwrap().line_top + lh * 0.5;
        let b = byte_at_point(&layout, 0.0, y_line1);
        let line1_start = layout.iter().find(|p| p.line == 1).unwrap().byte;
        assert_eq!(
            b, line1_start,
            "click on line 1 left edge → line-1 start byte"
        );

        // A click above everything clamps to line 0.
        let b_top = byte_at_point(&layout, 0.0, -100.0);
        assert_eq!(caret_for_byte(&layout, b_top).line, 0);
        // A click far below clamps to the last line.
        let b_bot = byte_at_point(&layout, 1e6, 1e6);
        assert_eq!(caret_for_byte(&layout, b_bot).line, 1);
    }

    #[test]
    fn byte_on_adjacent_line_moves_with_sticky_column() {
        // Two lines of different content; moving down from line 0 at a desired x
        // lands on line 1, and moving up returns toward line 0.
        let text = "hello\nworld";
        let layout = caret_layout(text, WrapMode::None, 1000.0);
        // Start near the end of line 0 (byte 5 = the '\n' position, x≈line_w).
        let start = caret_for_byte(&layout, 5);
        let down = byte_on_adjacent_line(&layout, start.byte, 1, start.x);
        assert_eq!(
            caret_for_byte(&layout, down).line,
            1,
            "down moves to line 1"
        );
        // Moving up from there returns to line 0.
        let up = byte_on_adjacent_line(&layout, down, -1, start.x);
        assert_eq!(caret_for_byte(&layout, up).line, 0, "up returns to line 0");
        // At the top line, up is a no-op.
        let top = byte_on_adjacent_line(&layout, 0, -1, 0.0);
        assert_eq!(top, 0);
    }

    #[test]
    fn cache_returns_identical_results_on_repeat() {
        let mut measurer = TextMeasurer::new();
        let first = measurer.measure("Cached label", 16.0, None);
        // Second call must hit the cache and return the exact same dimensions.
        let second = measurer.measure("Cached label", 16.0, None);
        assert_eq!(first, second);
    }

    #[test]
    fn cache_keys_on_font_size_and_max_width() {
        let mut measurer = TextMeasurer::new();
        let small = measurer.measure("Hello", 12.0, None);
        let large = measurer.measure("Hello", 24.0, None);
        // Different font sizes are distinct cache entries with distinct metrics.
        assert!(large.0 > small.0);
        assert!(large.1 > small.1);
        // Re-measuring each still returns its own cached value.
        assert_eq!(measurer.measure("Hello", 12.0, None), small);
        assert_eq!(measurer.measure("Hello", 24.0, None), large);
    }

    #[test]
    fn positive_letter_spacing_changes_width_and_uses_distinct_cache_entry() {
        let mut measurer = TextMeasurer::new();
        let plain = TextBlock::new("Spacing", 0.0, 0.0).with_size(20.0);
        let spaced = plain.clone().with_letter_spacing(3.0);

        let plain_size = measurer.measure_block(&plain);
        assert_eq!(layout_stats(&measurer).layouts, 1);
        let spaced_size = measurer.measure_block(&spaced);
        assert_eq!(
            layout_stats(&measurer).layouts,
            2,
            "spacing must distinguish cache keys"
        );
        assert!(
            spaced_size.0 > plain_size.0,
            "positive spacing should widen text: plain={}, spaced={}",
            plain_size.0,
            spaced_size.0
        );
        assert_eq!(measurer.measure_block(&plain), plain_size);
        let stats = layout_stats(&measurer);
        assert_eq!(
            (stats.layouts, stats.hits),
            (2, 1),
            "repeat should hit plain cache entry"
        );
    }

    #[test]
    fn letter_spacing_is_in_pixels_at_any_font_size() {
        // cosmic-text takes letter spacing in em; the block's value is pixels,
        // so four glyphs with 2 px each widen by ~8 px whatever the size.
        let mut measurer = TextMeasurer::new();
        for size in [10.0, 20.0, 40.0] {
            let plain = TextBlock::new("abcd", 0.0, 0.0).with_size(size);
            let spaced = plain.clone().with_letter_spacing(2.0);
            let grown = measurer.measure_block(&spaced).0 - measurer.measure_block(&plain).0;
            assert!(
                (grown - 8.0).abs() < 1.0,
                "at {size} px, 2 px spacing over four glyphs grew the width by {grown}"
            );
        }
    }

    #[test]
    fn clear_cache_forces_remeasure_without_changing_result() {
        let mut measurer = TextMeasurer::new();
        let before = measurer.measure("Persistent", 18.0, None);
        measurer.clear_cache();
        let after = measurer.measure("Persistent", 18.0, None);
        assert_eq!(before, after);
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn bundled_sans_font_returns_family_name() {
        let fs = shared_font_system();
        let handle = super::register_bundled_fonts(&fs).expect("register bundled fonts");
        assert_eq!(handle.family(), "IBM Plex Sans");
    }

    #[test]
    fn load_font_bytes_rejects_garbage() {
        let fs = shared_font_system();
        assert!(load_font_bytes(&fs, &[0u8, 1, 2, 3, 4, 5, 6, 7]).is_err());
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn loaded_font_is_actually_selected_during_shaping() {
        // The real proof that `with_font` works: shape a string selecting the
        // loaded family and confirm cosmic-text resolved glyphs to *that* face.
        let fs = shared_font_system();
        let handle = super::register_bundled_fonts(&fs).unwrap();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let mut buffer = Buffer::new(guard, Metrics::new(20.0, 25.0));
        buffer.set_text(
            "Ag",
            &Attrs::new().family(Family::Name(handle.family())),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(guard, false);
        let font_id = buffer.layout_runs().next().unwrap().glyphs[0].font_id;
        let info = guard.db().face(font_id).expect("resolved face exists");
        let family = info.families.first().map(|(n, _)| n.as_str()).unwrap_or("");
        assert_eq!(family, handle.family());
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn measure_with_font_is_cached_per_font() {
        // Default and explicit bundled-mono measurements live under distinct
        // cache keys and each round-trips.
        let fs = shared_font_system();
        let mono = super::bundled_mono_font(&fs).unwrap();
        let mut measurer = TextMeasurer::with_font_system(fs);
        let default = measurer.measure("Hello world", 16.0, None);
        let mono_measure = measurer.measure_with_font("Hello world", 16.0, None, Some(&mono));
        assert!(default.0 > 0.0 && mono_measure.0 > 0.0);
        // Re-measuring each returns its own cached value.
        assert_eq!(measurer.measure("Hello world", 16.0, None), default);
        assert_eq!(
            measurer.measure_with_font("Hello world", 16.0, None, Some(&mono)),
            mono_measure
        );
    }

    #[test]
    fn font_and_align_defaults_and_builders() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert!(plain.font.is_none());
        assert_eq!(plain.align, TextAlign::Start);

        let styled = TextBlock::new("x", 0.0, 0.0)
            .with_font(FontHandle("IBM Plex Sans".to_string()))
            .with_align(TextAlign::Center);
        assert_eq!(styled.font.as_ref().unwrap().family(), "IBM Plex Sans");
        assert_eq!(styled.align, TextAlign::Center);
    }

    // ---- RTL / bidi display knobs ----

    #[test]
    fn direction_prefix_emits_the_right_bidi_mark() {
        assert_eq!(direction_prefix(TextDirection::Auto), "");
        assert_eq!(direction_prefix(TextDirection::Ltr), "\u{200E}"); // LRM
        assert_eq!(direction_prefix(TextDirection::Rtl), "\u{200F}"); // RLM
    }

    #[test]
    fn cosmic_align_maps_logical_and_absolute_variants() {
        use cosmic_text::Align as CA;
        // Start is cosmic-text's direction-relative default → no override.
        assert!(cosmic_align(TextAlign::Start).is_none());
        assert!(matches!(cosmic_align(TextAlign::Center), Some(CA::Center)));
        assert!(matches!(cosmic_align(TextAlign::End), Some(CA::End)));
        assert!(matches!(cosmic_align(TextAlign::Left), Some(CA::Left)));
        assert!(matches!(cosmic_align(TextAlign::Right), Some(CA::Right)));
    }

    #[test]
    fn direction_and_align_defaults_and_builders() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert_eq!(
            plain.align,
            TextAlign::Start,
            "default align is reading-start"
        );
        assert_eq!(
            plain.direction,
            TextDirection::Auto,
            "default direction is auto"
        );

        let forced = TextBlock::new("x", 0.0, 0.0)
            .with_direction(TextDirection::Rtl)
            .with_align(TextAlign::End);
        assert_eq!(forced.direction, TextDirection::Rtl);
        assert_eq!(forced.align, TextAlign::End);
    }

    #[test]
    fn forced_rtl_right_flushes_neutral_content() {
        // A forced-RTL base direction makes a line flush to the right edge (cosmic
        // lays an RTL paragraph out from `line_width`), so even Latin content is
        // pushed rightward versus the LTR default. Mirrors `build_vertices`'
        // prefix mechanism without a GPU.
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let leftmost = |guard: &mut cosmic_text::FontSystem, prefix: &str| -> f32 {
            let mut buffer = Buffer::new(guard, Metrics::new(16.0, 20.0));
            buffer.set_size(Some(400.0), None);
            buffer.set_text(
                &format!("{prefix}short"),
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(guard, false);
            // First glyph that carries ink (skip the zero-width mark at index 0).
            buffer
                .layout_runs()
                .next()
                .unwrap()
                .glyphs
                .iter()
                .map(|g| g.x)
                .fold(f32::MAX, f32::min)
        };
        let ltr = leftmost(guard, direction_prefix(TextDirection::Ltr));
        let rtl = leftmost(guard, direction_prefix(TextDirection::Rtl));
        assert!(
            rtl > ltr + 100.0,
            "forced RTL should right-flush: rtl {rtl} vs ltr {ltr}"
        );
    }

    // ---- Bidi editing primitives (pure, synthetic glyphs) ----

    /// A synthetic bidi line: logical "ab" (LTR) followed by "גד" (RTL, 2-byte
    /// chars). Visually: a@0 b@10 then the RTL run reversed — ד@20 ג@30 — each 10px.
    /// Byte layout: a=0..1, b=1..2, ג=2..4, ד=4..6.
    fn bidi_line() -> Vec<VisualGlyph> {
        let vg = |byte_start, byte_end, x, rtl| VisualGlyph {
            byte_start,
            byte_end,
            x,
            w: 10.0,
            line: 0,
            line_top: 0.0,
            line_height: 20.0,
            rtl,
        };
        vec![
            vg(0, 1, 0.0, false),  // a
            vg(1, 2, 10.0, false), // b
            vg(4, 6, 20.0, true),  // ד (logical-last, visually-left of the RTL run)
            vg(2, 4, 30.0, true),  // ג
        ]
    }

    #[test]
    fn selection_rects_split_across_a_bidi_boundary() {
        // Selecting logical [1,4) covers b (LTR, x 10..20) and ג (RTL, x 30..40),
        // skipping ד (x 20..30) which is outside the range → two disjoint rects.
        let rects = selection_rects(&bidi_line(), 1, 4);
        assert_eq!(
            rects.len(),
            2,
            "bidi-straddling selection is two visual spans"
        );
        let mut xs: Vec<f32> = rects.iter().map(|r| r.x).collect();
        xs.sort_by(f32::total_cmp);
        assert!((xs[0] - 10.0).abs() < 0.6, "first span starts at b: {xs:?}");
        assert!(
            (xs[1] - 30.0).abs() < 0.6,
            "second span starts at ג: {xs:?}"
        );
    }

    #[test]
    fn selection_rects_contiguous_run_is_one_rect() {
        // Selecting just "ab" (logical 0..2) is one merged visual span [0,20].
        let rects = selection_rects(&bidi_line(), 0, 2);
        assert_eq!(rects.len(), 1);
        let r = rects[0];
        assert!(
            r.x.abs() < 0.6 && (r.w - 20.0).abs() < 0.6,
            "merged ab span: {r:?}"
        );
    }

    #[test]
    fn selection_rects_empty_when_degenerate() {
        assert!(
            selection_rects(&bidi_line(), 3, 3).is_empty(),
            "empty range"
        );
        assert!(selection_rects(&[], 0, 5).is_empty(), "no glyphs");
    }

    #[test]
    fn visual_caret_moves_left_to_right_on_screen() {
        let line = bidi_line();
        // LTR portion: stepping right increases byte (0→1→2 at x 0,10,20).
        assert_eq!(visual_caret_neighbor(&line, 0, 1), 1, "a→b");
        assert_eq!(visual_caret_neighbor(&line, 1, 1), 2, "b→ab/RTL seam");
        // RTL interior: stepping right *decreases* logical byte (visual right in an
        // RTL run is logically backward): ד-left=6 → ג-left=4.
        assert_eq!(visual_caret_neighbor(&line, 6, 1), 4, "ד→ג moving right");
        // Leftward is the mirror.
        assert_eq!(visual_caret_neighbor(&line, 1, -1), 0, "b→a moving left");
    }

    #[test]
    fn visual_caret_pure_ltr_is_logical() {
        let vg = |byte_start, byte_end, x| VisualGlyph {
            byte_start,
            byte_end,
            x,
            w: 10.0,
            line: 0,
            line_top: 0.0,
            line_height: 20.0,
            rtl: false,
        };
        let line = vec![vg(0, 1, 0.0), vg(1, 2, 10.0), vg(2, 3, 20.0)]; // "abc"
        assert_eq!(visual_caret_neighbor(&line, 0, 1), 1);
        assert_eq!(visual_caret_neighbor(&line, 1, 1), 2);
        assert_eq!(visual_caret_neighbor(&line, 2, -1), 1);
        // No-op at the visual extremes (single line, nowhere to wrap).
        assert_eq!(visual_caret_neighbor(&line, 0, -1), 0);
        assert_eq!(visual_caret_neighbor(&line, 3, 1), 3);
    }

    #[test]
    fn text_visual_layout_tags_bidi_levels() {
        // Real shaping of a Latin+Hebrew string: cosmic assigns bidi levels per
        // glyph regardless of whether a Hebrew face is installed, so the rtl flags
        // are deterministic. 'a' is LTR, the Hebrew letters are RTL.
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let glyphs = text_visual_layout(
            guard,
            "aאב",
            16.0,
            20.0,
            400.0,
            WrapMode::None,
            None,
            TextDirection::Auto,
        );
        assert!(!glyphs.is_empty(), "shaped some glyphs");
        assert!(glyphs.iter().any(|g| !g.rtl), "the Latin 'a' is LTR");
        assert!(glyphs.iter().any(|g| g.rtl), "the Hebrew letters are RTL");
        // Byte offsets stay within the source string (prefix-adjusted, here no prefix).
        assert!(glyphs.iter().all(|g| g.byte_end <= "aאב".len()));
    }

    #[test]
    fn text_visual_layout_strips_forced_direction_mark() {
        // Forcing a direction prepends a zero-width mark; it must not appear as a
        // glyph nor shift the reported byte offsets.
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let glyphs = text_visual_layout(
            guard,
            "hi",
            16.0,
            20.0,
            400.0,
            WrapMode::None,
            None,
            TextDirection::Rtl,
        );
        assert!(
            glyphs.iter().all(|g| g.byte_end <= 2),
            "byte offsets map back to 'hi', not the prefixed string: {glyphs:?}"
        );
    }

    // `SelRect` is part of the public surface; touch it so the import is used in
    // builds that compile only a subset of tests.
    #[allow(dead_code)]
    fn _selrect_is_constructible() -> SelRect {
        SelRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }
    }

    #[test]
    fn alignment_shifts_leftmost_glyph_within_max_width() {
        // Mirrors the shaping in `build_vertices`: a short line in a wide box
        // moves rightward under Center then Right. Asserting on cosmic-text's
        // per-glyph x (which the renderer adds to `block.x`) keeps this GPU-free.
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let mut leftmost = |align: TextAlign| -> f32 {
            let mut buffer = Buffer::new(guard, Metrics::new(16.0, 20.0));
            buffer.set_size(Some(400.0), None);
            buffer.set_text(
                "short",
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            if let Some(a) = cosmic_align(align) {
                for line in buffer.lines.iter_mut() {
                    line.set_align(Some(a));
                }
            }
            buffer.shape_until_scroll(guard, false);
            buffer.layout_runs().next().unwrap().glyphs[0].x
        };
        let left = leftmost(TextAlign::Left);
        let center = leftmost(TextAlign::Center);
        let right = leftmost(TextAlign::Right);
        assert!(center > left, "center {center} should exceed left {left}");
        assert!(
            right > center,
            "right {right} should exceed center {center}"
        );
    }

    #[test]
    fn with_ellipsis_is_opt_in() {
        assert!(!TextBlock::new("x", 0.0, 0.0).ellipsize);
        assert!(TextBlock::new("x", 0.0, 0.0).with_ellipsis().ellipsize);
    }

    #[test]
    fn weight_and_style_defaults_and_builders() {
        let plain = TextBlock::new("x", 0.0, 0.0);
        assert_eq!(plain.weight, Weight::NORMAL);
        assert_eq!(plain.style, Style::Normal);

        assert_eq!(TextBlock::new("x", 0.0, 0.0).bold().weight, Weight::BOLD);
        assert_eq!(TextBlock::new("x", 0.0, 0.0).italic().style, Style::Italic);
        assert_eq!(
            TextBlock::new("x", 0.0, 0.0)
                .with_weight(Weight(500))
                .weight,
            Weight(500)
        );
        assert_eq!(
            TextBlock::new("x", 0.0, 0.0)
                .with_style(Style::Oblique)
                .style,
            Style::Oblique
        );
    }

    #[test]
    fn with_font_opt_only_applies_some() {
        let none = TextBlock::new("x", 0.0, 0.0).with_font_opt(None);
        assert!(none.font.is_none());
        let some = TextBlock::new("x", 0.0, 0.0)
            .with_font_opt(Some(FontHandle("IBM Plex Sans".to_string())));
        assert_eq!(some.font.as_ref().unwrap().family(), "IBM Plex Sans");
        // Some over an existing font replaces it; None leaves it untouched.
        let kept = TextBlock::new("x", 0.0, 0.0)
            .with_font(FontHandle("A".into()))
            .with_font_opt(None);
        assert_eq!(kept.font.as_ref().unwrap().family(), "A");
    }

    #[test]
    fn style_disc_is_stable() {
        assert_eq!(super::style_disc(Style::Normal), 0);
        assert_eq!(super::style_disc(Style::Italic), 1);
        assert_eq!(super::style_disc(Style::Oblique), 2);
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn bold_measures_wider_than_regular() {
        // The regular + bold IBM Plex Sans faces share one family; weight selects
        // between them at shape time. GPU-free — measurer only.
        let fs = shared_font_system();
        let regular = super::register_bundled_fonts(&fs).unwrap();
        let mut m = TextMeasurer::with_font_system(fs);
        let text = "The quick brown fox jumps";
        let (rw, _) = m.measure_styled(
            text,
            18.0,
            None,
            Some(&regular),
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        );
        let (bw, _) = m.measure_styled(
            text,
            18.0,
            None,
            Some(&regular),
            Weight::BOLD,
            Style::Normal,
            WrapMode::default(),
        );
        assert!(rw > 0.0 && bw > 0.0);
        assert!(bw > rw, "bold width {bw} should exceed regular {rw}");
        // Distinct cache entries keyed by weight: re-measuring regular still
        // returns the regular width (proves weight is part of the key).
        let (rw2, _) = m.measure_styled(
            text,
            18.0,
            None,
            Some(&regular),
            Weight::NORMAL,
            Style::Normal,
            WrapMode::default(),
        );
        assert_eq!(rw2, rw);
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn bundled_font_is_default_sans_serif() {
        // `shared_font_system` registers bundled IBM Plex Sans and makes it the
        // default sans-serif, so `Family::SansSerif` resolves to that family.
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();

        let mut buffer = Buffer::new(guard, Metrics::new(18.0, 22.0));
        buffer.set_text(
            "Ag",
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(guard, false);
        let font_id = buffer.layout_runs().next().unwrap().glyphs[0].font_id;
        let fam = guard
            .db()
            .face(font_id)
            .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
            .unwrap_or_default();
        assert_eq!(
            fam, "IBM Plex Sans",
            "default sans-serif should be the bundled Plex Sans family"
        );

        // Bold weight selects a heavier face from the same bundled family.
        let mut bold = Buffer::new(guard, Metrics::new(18.0, 22.0));
        bold.set_text(
            "Ag",
            &Attrs::new().family(Family::SansSerif).weight(Weight::BOLD),
            Shaping::Advanced,
            None,
        );
        bold.shape_until_scroll(guard, false);
        let bold_id = bold.layout_runs().next().unwrap().glyphs[0].font_id;
        let bold_weight = guard.db().face(bold_id).map(|f| f.weight.0).unwrap_or(0);
        assert!(
            bold_weight >= 600,
            "bold weight should select a bold face (>=600), got {bold_weight}"
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn bundled_mono_font_loads_nothing_into_a_font_system_that_has_it() {
        let fs = shared_font_system();
        let faces = || fs.lock().unwrap().db().faces().count();
        let before = faces();
        for _ in 0..3 {
            assert_eq!(
                super::bundled_mono_font(&fs).unwrap().family(),
                super::BUNDLED_MONO_FAMILY
            );
        }
        assert_eq!(faces(), before);
        assert_eq!(
            crate::Theme::default().mono_font.unwrap().family(),
            super::BUNDLED_MONO_FAMILY
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn bundled_mono_font_selects_the_technical_companion_family() {
        let fs = shared_font_system();
        let mono = super::bundled_mono_font(&fs).expect("bundled mono is available");
        assert_eq!(mono.family(), "IBM Plex Mono");

        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let mut buffer = Buffer::new(guard, Metrics::new(18.0, 22.0));
        buffer.set_text(
            "x=42",
            &Attrs::new().family(Family::Name(mono.family())),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(guard, false);
        let font_id = buffer.layout_runs().next().unwrap().glyphs[0].font_id;
        let face = guard.db().face(font_id).expect("resolved face exists");
        let family = face.families.first().map(|(name, _)| name.as_str());
        assert_eq!(family, Some("IBM Plex Mono"));
    }

    #[test]
    fn ellipsize_leaves_fitting_text_unchanged() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        // A wide budget the short string easily fits within.
        let out = ellipsize_to_width(
            guard,
            "short",
            16.0,
            20.0,
            1000.0,
            Family::SansSerif,
            Weight::NORMAL,
            Style::Normal,
            0.0,
        );
        assert_eq!(out, "short");
    }

    #[test]
    fn ellipsize_truncates_overflowing_text_with_ellipsis() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let long = "a_very_long_object_name_that_will_not_fit";
        let max_width = 80.0;
        let out = ellipsize_to_width(
            guard,
            long,
            14.0,
            18.0,
            max_width,
            Family::SansSerif,
            Weight::NORMAL,
            Style::Normal,
            0.0,
        );
        assert_ne!(out, long, "overflowing text should be truncated");
        assert!(
            out.ends_with('…'),
            "truncated text should end with an ellipsis"
        );
        assert!(out.chars().count() < long.chars().count());
        // The truncated line (incl. the ellipsis) must fit the budget.
        let (w, _) = shape_layout(guard, &LayoutSpec::plain(14.0, None), &out).size;
        assert!(w <= max_width, "ellipsized width {w} must fit {max_width}");
    }

    #[test]
    fn ellipsize_degenerate_budget_returns_just_ellipsis() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        // A budget too small for even one glyph + the ellipsis.
        let out = ellipsize_to_width(
            guard,
            "anything",
            14.0,
            18.0,
            2.0,
            Family::SansSerif,
            Weight::NORMAL,
            Style::Normal,
            0.0,
        );
        assert_eq!(out, "…");
    }

    // ---- text_cursor_positions tests ----

    #[test]
    fn cursor_positions_empty_text() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let pos = text_cursor_positions(guard, "", 16.0, 20.0, 800.0, None);
        assert_eq!(pos, &[(0, 0.0)]);
    }

    #[test]
    fn cursor_positions_has_origin_first() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let pos = text_cursor_positions(guard, "Hi", 16.0, 20.0, 800.0, None);
        assert_eq!(pos.first(), Some(&(0, 0.0)));
    }

    #[test]
    fn cursor_positions_last_is_text_len() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let text = "Hello";
        let pos = text_cursor_positions(guard, text, 16.0, 20.0, 800.0, None);
        assert_eq!(pos.last().map(|(i, _)| *i), Some(text.len()));
    }

    #[test]
    fn cursor_positions_monotonically_increasing() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let text = "The quick brown fox";
        let pos = text_cursor_positions(guard, text, 16.0, 20.0, 800.0, None);
        for pair in pos.windows(2) {
            assert!(
                pair[0].1 <= pair[1].1,
                "position regressed: {:?} -> {:?}",
                pair[0],
                pair[1]
            );
            assert!(
                pair[0].0 <= pair[1].0,
                "byte index regressed: {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn cursor_positions_last_matches_measure_width() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        let text = "Hello World";
        let font_size = 16.0;
        let max_width = 800.0;
        let pos = text_cursor_positions(guard, text, font_size, font_size * 1.25, max_width, None);

        let (total_w, _) = shape_layout(guard, &LayoutSpec::plain(font_size, None), text).size;
        let final_x = pos.last().map(|(_, x)| *x).unwrap_or(0.0);
        // The final x-position should approximate the measured width.
        assert!(
            (final_x - total_w).abs() < 2.0,
            "final x {final_x} differs from measured width {total_w} by >2px"
        );
    }

    #[test]
    fn cursor_positions_multibyte_utf8() {
        let fs = shared_font_system();
        let mut shared = fs.lock().unwrap();
        let guard = shared.font_system();
        // "é" is 2 bytes (U+00E9), "あ" is 3 bytes (U+3042).
        // Positions should be recorded at the correct *byte* boundaries:
        //   "éXあ" → bytes: [0..2) = é, [2..3) = X, [3..6) = あ
        let text = "éXあ";
        let pos = text_cursor_positions(guard, text, 16.0, 20.0, 800.0, None);

        // We should have a position for byte 0, byte 2 (after é), byte 3 (after X),
        // and byte 6 (end of あ).
        let indices: Vec<usize> = pos.iter().map(|(i, _)| *i).collect();
        assert!(
            indices.contains(&0),
            "should have position at byte 0, got {indices:?}"
        );
        assert!(
            indices.contains(&2),
            "should have position at byte 2 (after é), got {indices:?}"
        );
        assert!(
            indices.contains(&3),
            "should have position at byte 3 (after X), got {indices:?}"
        );
        assert!(
            indices.contains(&6),
            "should have position at byte 6 (end), got {indices:?}"
        );
        assert_eq!(pos.last().map(|(i, _)| *i), Some(6));
    }

    // ----- Shaped-text cache (GPU-gated, like the chrome parity test) -----

    /// Build a headless `TextRenderer` + a bundled Plex Sans handle, or `None` if
    /// no GPU adapter is available (so the `#[ignore]`d tests below no-op safely).
    fn headless_renderer() -> Option<(wgpu::Device, wgpu::Queue, TextRenderer, FontHandle)> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("text-cache test device"),
                ..Default::default()
            },
            None,
        ))
        .ok()?;
        let fs = shared_font_system();
        let font = super::register_bundled_fonts(&fs).expect("register bundled fonts");
        let renderer = TextRenderer::with_font_system(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            fs,
        );
        Some((device, queue, renderer, font))
    }

    /// Layouts kept in the renderer's shared font system.
    fn cache_total(r: &TextRenderer) -> usize {
        r.font_system.lock().unwrap().layout_stats().layouts
    }

    /// Reinterpret a vertex slice as raw bytes for exact-equality comparison
    /// (`MsdfVertex` is `Pod` but not `PartialEq`).
    fn vbytes(v: &[MsdfVertex]) -> &[u8] {
        bytemuck::cast_slice(v)
    }

    fn label(text: &str, font: &FontHandle) -> TextBlock {
        TextBlock::new(text, 40.0, 40.0)
            .with_size(18.0)
            .with_font(font.clone())
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn shape_cache_reuses_layout_and_matches_cold() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let blocks = [label("Cached label", &font)];

        // Cold: shapes and records exactly one entry.
        let cold = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1, "one shaping entry recorded");
        assert!(!cold.is_empty(), "non-empty glyph geometry");

        // Warm: served from the cache, byte-identical output, still one entry.
        let warm = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1, "warm frame adds no new entries");
        assert_eq!(vbytes(&cold), vbytes(&warm), "cache must not change pixels");
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn moving_block_reuses_cache_and_shifts() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let a = r.build_vertices(&[label("Move me", &font)]);
        let mut moved = label("Move me", &font);
        moved.x += 100.0;
        moved.y += 50.0;
        let b = r.build_vertices(&[moved]);

        // Same key + content => still a single cache entry (a hit, not a re-shape).
        assert_eq!(cache_total(&r), 1, "moved block reuses the cached layout");
        assert_eq!(a.len(), b.len());
        for (va, vb) in a.iter().zip(b.iter()) {
            assert!((vb.position[0] - va.position[0] - 100.0).abs() < 1e-3);
            assert!((vb.position[1] - va.position[1] - 50.0).abs() < 1e-3);
            // Everything but position is unchanged: compare uv + fill.
            assert_eq!(va.uv, vb.uv);
            assert_eq!(va.fill, vb.fill);
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn distinct_attributes_create_distinct_entries() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Each variant differs in exactly one keyed attribute, so each is a miss.
        let base = label("Same", &font);
        let mut bigger = label("Same", &font);
        bigger.font_size = 24.0;
        let mut narrower = label("Same", &font);
        narrower.max_width = 20.0;
        let mut centered = label("Same", &font);
        centered.align = TextAlign::Center;
        let mut clipped_ellipsis = label("Same", &font);
        clipped_ellipsis.ellipsize = true;
        let other_content = label("Different", &font);
        let default_font = label("Same", &font).with_max_width(800.0);
        let mut default_font = default_font;
        default_font.font = None; // None vs Noto => different family hash
        let bold_variant = label("Same", &font).bold();
        let italic_variant = label("Same", &font).italic();

        r.build_vertices(&[
            base,
            bigger,
            narrower,
            centered,
            clipped_ellipsis,
            other_content,
            default_font,
            bold_variant,
            italic_variant,
        ]);
        assert_eq!(
            cache_total(&r),
            9,
            "each distinct (content|size|width|align|ellipsize|font|weight|style) is its own entry"
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn direction_is_part_of_the_shape_cache_key() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Same content/metrics, three base directions → three distinct entries.
        let auto = label("dir", &font);
        let ltr = label("dir", &font).with_direction(TextDirection::Ltr);
        let rtl = label("dir", &font).with_direction(TextDirection::Rtl);
        r.build_vertices(&[auto, ltr, rtl]);
        assert_eq!(
            cache_total(&r),
            3,
            "each base direction caches and shapes independently"
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn span_colours_survive_the_direction_prefix() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // A two-span string: the direction prefix shifts cosmic's byte offsets,
        // but `byte_start` is corrected back so per-span colour resolution is
        // unaffected. The set of emitted fills must match between Auto and Rtl.
        let spans = vec![
            TextSpan {
                text: "AB".into(),
                color: Some(red()),
                underline: Underline::None,
            },
            TextSpan {
                text: "cd".into(),
                color: Some(blue()),
                underline: Underline::None,
            },
        ];
        let auto = label("ABcd", &font).with_spans(spans.clone());
        let rtl = label("ABcd", &font)
            .with_spans(spans)
            .with_direction(TextDirection::Rtl);
        let va = r.build_vertices(&[auto]);
        let vb = r.build_vertices(&[rtl]);

        let fills = |v: &[MsdfVertex]| -> std::collections::BTreeSet<[u32; 4]> {
            v.iter().map(|x| x.fill.map(f32::to_bits)).collect()
        };
        let fa = fills(&va);
        assert!(
            fa.contains(&red().map(f32::to_bits)) && fa.contains(&blue().map(f32::to_bits)),
            "both span colours present in the LTR baseline"
        );
        assert_eq!(
            fa,
            fills(&vb),
            "forcing RTL must not corrupt per-span colour mapping"
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn a_block_measured_for_layout_is_drawn_without_shaping_it_again() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let mut measurer = TextMeasurer::with_font_system(std::sync::Arc::clone(&r.font_system));
        let block = label("Measured, then drawn", &font)
            .with_max_width(90.0)
            .with_align(TextAlign::Center);
        let start = layout_stats(&measurer);
        let size = measurer.measure_block(&block);
        assert!(size.1 > block.line_height, "wraps: {size:?}");
        let measured = layout_stats(&measurer);
        assert_eq!(
            (measured.shaped - start.shaped, measured.hits - start.hits),
            (1, 0)
        );

        let verts = r.build_vertices(std::slice::from_ref(&block));
        assert!(!verts.is_empty());
        let drawn = layout_stats(&measurer);
        assert_eq!(
            (drawn.shaped - start.shaped, drawn.hits - start.hits),
            (1, 1),
            "the renderer used the measured layout"
        );

        // And it draws exactly what shaping it afresh draws.
        r.clear_shape_cache();
        assert_eq!(vbytes(&verts), vbytes(&r.build_vertices(&[block])));
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn clear_shape_cache_forces_reshape_with_identical_result() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let blocks = [label("Reshape", &font)];
        let before = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1);

        r.clear_shape_cache();
        assert_eq!(cache_total(&r), 0, "clear empties the cache");

        let after = r.build_vertices(&blocks);
        assert_eq!(cache_total(&r), 1, "re-shaped and re-cached");
        assert_eq!(vbytes(&before), vbytes(&after), "re-shape is identical");
    }

    // ---- per-glyph byte mapping (style ranges / syntax highlighting) ----

    /// A glyph's `byte_start` must address the **whole block content**, not its
    /// buffer line. cosmic-text reports `glyph.start` relative to the original
    /// text *line*, so the horizontal shaping path must rebase it by the line's
    /// starting byte — exactly what `text_caret_layout`, `text_visual_layout`,
    /// and the vertical shaping branch already do. Without the rebase, every
    /// line after the first resolves style ranges against earlier lines' bytes
    /// and syntax highlighting drifts off the text it describes.
    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn style_ranges_map_to_the_correct_glyphs_across_lines() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Two lines; the second starts at byte 4 ("RED\n"). Only the first
        // line's bytes carry a style range, so only "RED" may render red. The
        // block colour is white — any red on line 2 is the bug.
        let content = "RED\nBLUE";
        let red = [1.0, 0.0, 0.0, 1.0];
        let block = label(content, &font)
            .with_wrap(WrapMode::None)
            .with_style_ranges(vec![TextStyleRange {
                range: 0..3,
                color: Some(red),
                underline: Underline::None,
            }]);
        let verts = r.build_vertices(&[block]);

        // Fills are the last back-to-front sweep, one 6-vert quad per glyph.
        // With the line-rebase fix exactly "RED" (3 glyphs) is red; the bug
        // mapped "BLUE"'s line-relative bytes (0..3) into the same range and
        // turned B/L/U red too (6 red quads).
        let red_quads: Vec<_> = verts
            .as_chunks::<6>()
            .0
            .iter()
            .filter(|q| q[0].fill == red)
            .collect();
        assert_eq!(
            red_quads.len(),
            3,
            "only the 3 glyphs of the styled range may resolve to the range colour"
        );
        let first_line_bottom = 40.0 + 18.0 * super::LINE_HEIGHT_RATIO; // block.y + one line box
        for quad in &red_quads {
            let y = quad[0].position[1];
            assert!(
                y < first_line_bottom,
                "red glyph quad at y={y} is below the first line — \
                 style bytes leaked across lines"
            );
        }
    }

    /// Same mapping with an explicit base direction: the 3-byte zero-width LRM
    /// sits once at the head of the shaped string. Line bases are computed over
    /// the *shaped* text (mark included), and the prefix is subtracted once from
    /// the absolute byte — so line 2's glyphs must land on caller bytes 8.. and
    /// stay white; the styled range still colours only line 1's "RED".
    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn style_ranges_stay_aligned_with_a_direction_prefix() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let content = "RED\nBLUE";
        let red = [1.0, 0.0, 0.0, 1.0];
        let block = label(content, &font)
            .with_wrap(WrapMode::None)
            .with_direction(TextDirection::Ltr)
            .with_style_ranges(vec![TextStyleRange {
                range: 0..3,
                color: Some(red),
                underline: Underline::None,
            }]);
        let verts = r.build_vertices(&[block]);
        assert_eq!(
            verts
                .as_chunks::<6>()
                .0
                .iter()
                .filter(|q| q[0].fill == red)
                .count(),
            3
        );
    }

    /// Soft-wrap mapping: a narrow `max_width` splits "REDBLUE" into several
    /// visual rows of the *same* buffer line. Every row's glyphs must keep
    /// addressing their true content bytes — the styled "RED" prefix stays red
    /// and the rest falls back to the block colour, never the reverse.
    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn style_ranges_survive_soft_wrapping() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Wide-ish glyphs at 18px: force a wrap after ~3 glyphs.
        let content = "REDBLUE";
        let red = [1.0, 0.0, 0.0, 1.0];
        let block = label(content, &font)
            .with_max_width(40.0)
            .with_style_ranges(vec![TextStyleRange {
                range: 0..3,
                color: Some(red),
                underline: Underline::None,
            }]);
        let verts = r.build_vertices(&[block]);
        let quads: Vec<_> = verts.as_chunks::<6>().0.iter().collect();
        assert!(quads.len() >= 7, "every letter produced a quad");
        // The red ones are exactly the 3 "R E D" glyphs, and they sit on the
        // top row (smallest y).
        let mut reds: Vec<[f32; 2]> = quads
            .iter()
            .filter(|q| q[0].fill == red)
            .map(|q| q[0].position)
            .collect();
        reds.sort_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])));
        assert_eq!(reds.len(), 3);
        let top_y = quads
            .iter()
            .map(|q| q[0].position[1])
            .fold(f32::MAX, f32::min);
        for p in &reds {
            assert!(
                (p[1] - top_y).abs() < 1.0,
                "red quad at {:?} left the top row — wrap rows mis-resolve ranges",
                p
            );
        }
    }

    // ---- optical vertical centring metrics ----

    #[test]
    fn has_lowercase_detects_case() {
        assert!(has_lowercase("Apply"));
        assert!(has_lowercase("a"));
        // No lowercase letters: all-caps, digits, symbols, and (case-less) CJK.
        assert!(!has_lowercase("OK"));
        assert!(!has_lowercase("100%"));
        assert!(!has_lowercase(""));
        assert!(!has_lowercase("漢字"));
    }

    #[test]
    fn vmetrics_default_font_in_plausible_ranges() {
        let mut m = TextMeasurer::new();
        let v = m.vmetrics(None, Weight::NORMAL, Style::Normal);
        // IBM Plex Sans stays in these broad optical-centering-safe ranges.
        assert!(
            v.baseline_ratio > 0.9 && v.baseline_ratio < 1.1,
            "baseline {v:?}"
        );
        assert!(v.x_ratio > 0.4 && v.x_ratio < 0.65, "x-height {v:?}");
        assert!(v.cap_ratio > 0.6 && v.cap_ratio < 0.85, "cap {v:?}");
        // The lowercase body is shorter than the caps.
        assert!(
            v.x_ratio < v.cap_ratio,
            "x-height must be below cap height {v:?}"
        );
        // CJK centre sits above the baseline. It is either measured from a real
        // CJK face (a touch above the cap-band centre) or, with no CJK font
        // installed, falls back exactly to the cap-band centre.
        assert!(
            v.cjk_center_ratio > 0.2 && v.cjk_center_ratio < 0.6,
            "cjk {v:?}"
        );
        // CJK baseline is at least as far down as the roman one (CJK rides taller).
        assert!(
            v.cjk_baseline_ratio >= v.baseline_ratio - 0.05,
            "cjk baseline {v:?}"
        );
    }

    #[test]
    fn has_cjk_detects_ideographs() {
        assert!(has_cjk("中"));
        assert!(has_cjk("こんにちは"));
        assert!(has_cjk("한글"));
        assert!(has_cjk("Tab 中")); // mixed
        assert!(!has_cjk("Apply"));
        assert!(!has_cjk("OK"));
        assert!(!has_cjk("100%"));
        assert!(!has_cjk(""));
    }

    #[test]
    fn visual_center_picks_band_by_script_then_case() {
        let mut m = TextMeasurer::new();
        let v = m.vmetrics(None, Weight::NORMAL, Style::Normal);
        // Roman: centre measured down from block top on the roman baseline.
        assert_eq!(
            v.visual_center_ratio("Apply"),
            v.baseline_ratio - v.x_ratio / 2.0
        );
        assert_eq!(
            v.visual_center_ratio("OK"),
            v.baseline_ratio - v.cap_ratio / 2.0
        );
        // CJK wins over case and uses its OWN baseline, not the roman one.
        let cjk_center = v.cjk_baseline_ratio - v.cjk_center_ratio;
        assert_eq!(v.visual_center_ratio("中"), cjk_center);
        assert_eq!(v.visual_center_ratio("Tab 中"), cjk_center);
    }

    #[test]
    fn vmetrics_is_deterministic_across_calls() {
        let mut m = TextMeasurer::new();
        let a = m.vmetrics(None, Weight::NORMAL, Style::Normal);
        let b = m.vmetrics(None, Weight::NORMAL, Style::Normal); // cache hit
        assert_eq!(a, b);
    }

    #[test]
    fn band_ratio_picks_band_by_case() {
        let mut m = TextMeasurer::new();
        let v = m.vmetrics(None, Weight::NORMAL, Style::Normal);
        // Lowercase present → x-height band; absent → cap-height band.
        assert_eq!(v.band_ratio(true), v.x_ratio);
        assert_eq!(v.band_ratio(false), v.cap_ratio);
        assert_eq!(v.band_ratio(has_lowercase("Apply")), v.x_ratio);
        assert_eq!(v.band_ratio(has_lowercase("OK")), v.cap_ratio);
    }

    // ---- Ink band ----

    #[cfg(feature = "bundled-font")]
    #[test]
    fn ink_band_sits_inside_the_line_box() {
        let mut m = TextMeasurer::new();
        let block = TextBlock::new("Medium", 0.0, 0.0).with_size(16.0);
        let (_, line_h) = m.measure_block(&block);
        let (top, bottom) = m.measure_block_ink(&block).expect("glyphs ink");
        assert!(
            top > 0.0 && bottom < line_h,
            "ink {top}..{bottom} should sit strictly inside the {line_h}px line box"
        );
        // Leading above the ascent is what makes the line box taller than the
        // ink, and it is exactly what `vcentered_text_y` compensates for.
        assert!(
            bottom - top < line_h,
            "ink height {} should be under the line box {line_h}",
            bottom - top
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn a_descender_reaches_below_a_baseline_only_glyph() {
        let mut m = TextMeasurer::new();
        let no_desc = TextBlock::new("mow", 0.0, 0.0).with_size(16.0);
        let desc = TextBlock::new("mop", 0.0, 0.0).with_size(16.0);
        let (_, b_no) = m.measure_block_ink(&no_desc).expect("inks");
        let (_, b_yes) = m.measure_block_ink(&desc).expect("inks");
        assert!(
            b_yes > b_no,
            "the 'p' descender should extend the band: {b_no} vs {b_yes}"
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn an_ascender_reaches_above_an_x_height_glyph() {
        let mut m = TextMeasurer::new();
        let low = TextBlock::new("nom", 0.0, 0.0).with_size(16.0);
        let tall = TextBlock::new("nomd", 0.0, 0.0).with_size(16.0);
        let (t_low, _) = m.measure_block_ink(&low).expect("inks");
        let (t_tall, _) = m.measure_block_ink(&tall).expect("inks");
        assert!(
            t_tall < t_low,
            "the 'd' ascender should raise the band top: {t_low} vs {t_tall}"
        );
    }

    #[test]
    fn text_without_outlines_inks_nothing() {
        let mut m = TextMeasurer::new();
        assert_eq!(
            m.measure_block_ink(&TextBlock::new("", 0.0, 0.0).with_size(16.0)),
            None
        );
        assert_eq!(
            m.measure_block_ink(&TextBlock::new("   ", 0.0, 0.0).with_size(16.0)),
            None,
            "spaces have no outline, so there is no ink band to report"
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn ink_band_scales_with_font_size() {
        let mut m = TextMeasurer::new();
        let small = m
            .measure_block_ink(&TextBlock::new("Medium", 0.0, 0.0).with_size(10.0))
            .expect("inks");
        let large = m
            .measure_block_ink(&TextBlock::new("Medium", 0.0, 0.0).with_size(20.0))
            .expect("inks");
        let ratio = (large.1 - large.0) / (small.1 - small.0);
        assert!(
            (ratio - 2.0).abs() < 0.05,
            "doubling the font size should double the ink height (got {ratio}x)"
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn a_wrapped_block_inks_across_every_line() {
        let mut m = TextMeasurer::new();
        let one = TextBlock::new("Medium", 0.0, 0.0).with_size(16.0);
        let mut two = TextBlock::new("Medium medium", 0.0, 0.0).with_size(16.0);
        two.max_width = 50.0; // forces a wrap
        let (_, h_two) = m.measure_block(&two);
        assert!(h_two > m.measure_block(&one).1, "precondition: it wrapped");
        let single = m.measure_block_ink(&one).expect("inks");
        let wrapped = m.measure_block_ink(&two).expect("inks");
        assert!(
            wrapped.1 - wrapped.0 > single.1 - single.0,
            "a wrapped block's ink must span its lines: {single:?} vs {wrapped:?}"
        );
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn ink_band_is_cached_per_key() {
        let mut m = TextMeasurer::new();
        let block = TextBlock::new("Medium", 0.0, 0.0).with_size(16.0);
        let a = m.measure_block_ink(&block);
        let b = m.measure_block_ink(&block); // cache hit
        assert_eq!(a, b);
        // A different size is a different key, so it must re-measure, not reuse.
        let bigger = TextBlock::new("Medium", 0.0, 0.0).with_size(32.0);
        assert_ne!(a, m.measure_block_ink(&bigger));
    }

    #[test]
    fn embox_fallback_matches_line_box_centring() {
        // When a font lacks cap/x metrics, `resolve_vmetrics` stores the em-box
        // equivalent so optical centring degrades to `vcentered_line_y`. Construct
        // that fallback explicitly and verify the identity holds.
        let baseline_ratio = 1.013_f32;
        let embox = 2.0 * (baseline_ratio - LINE_HEIGHT_RATIO / 2.0);
        let v = FontVMetrics {
            baseline_ratio,
            x_ratio: embox,
            cap_ratio: embox,
            cjk_baseline_ratio: baseline_ratio,
            cjk_center_ratio: embox / 2.0,
        };
        let (top, height, fs) = (10.0_f32, 40.0_f32, 16.0_f32);
        // Optical y using the (fallback) band.
        let optical = top + height / 2.0 - fs * v.visual_center_ratio("Apply");
        let line_box = vcentered_line_y(top, height, fs);
        assert!(
            (optical - line_box).abs() < 1e-4,
            "optical {optical} vs line-box {line_box}"
        );
    }

    // ---- Vertical (stacked) text ----

    #[test]
    fn vertical_stack_string_one_cluster_per_line() {
        // Three kana → three lines joined by '\n', clusters intact and in order.
        assert_eq!(vertical_stack_string("あいう"), "あ\nい\nう");
        // Empty stays empty; a single cluster has no separator.
        assert_eq!(vertical_stack_string(""), "");
        assert_eq!(vertical_stack_string("あ"), "あ");
    }

    #[test]
    fn vertical_stack_string_keeps_grapheme_clusters_intact() {
        // A base + combining dakuten is one grapheme cluster — it must NOT be split
        // across rows (no newline injected between base and mark).
        let combining = "\u{304B}\u{3099}"; // か + combining ゛ = が
        assert_eq!(vertical_stack_string(combining), combining);
        // A ZWJ emoji sequence (family) is a single cluster too.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(vertical_stack_string(family), family);
        // Mixed: each of two clusters on its own line, the combining one whole.
        assert_eq!(
            vertical_stack_string(&format!("{combining}A")),
            format!("{combining}\nA")
        );
    }

    #[test]
    fn vertical_measure_is_tall_and_narrow() {
        // The vertical column of N clusters is ~N rows tall and one glyph wide —
        // the transpose of the same string measured horizontally (wide and short).
        let mut m = TextMeasurer::new();
        let text = "あいうえお"; // 5 clusters
        let (vw, vh) = m.measure_vertical(text, 24.0);
        let (hw, hh) = m.measure(text, 24.0, None);

        assert!(
            vh > vw,
            "vertical column should be taller than wide: {vw}x{vh}"
        );
        assert!(
            hw > hh,
            "horizontal run should be wider than tall: {hw}x{hh}"
        );
        // Stacked height ≈ 5 rows; clearly taller than the single-line height.
        assert!(
            vh > hh * 4.0,
            "5-cluster column ({vh}) should dwarf one line ({hh})"
        );
        // Column width ≈ one glyph: far narrower than the 5-glyph horizontal run.
        assert!(
            vw < hw * 0.5,
            "column width ({vw}) should be a fraction of the run width ({hw})"
        );
    }

    #[test]
    fn vertical_measure_height_scales_with_cluster_count() {
        // Each extra cluster adds one row of `line_height` (= size * ratio).
        let mut m = TextMeasurer::new();
        let (_, h3) = m.measure_vertical("あいう", 20.0);
        let (_, h6) = m.measure_vertical("あいうえおか", 20.0);
        let row = 20.0 * LINE_HEIGHT_RATIO;
        assert!(
            (h6 - h3 - 3.0 * row).abs() < row * 0.5,
            "doubling 3→6 clusters should add ~3 rows ({row} each): {h3} -> {h6}"
        );
    }

    #[test]
    fn vertical_and_horizontal_measure_cache_independently() {
        // The orientation is part of the measure cache key, so the two never
        // collide on the same (text, size).
        let mut m = TextMeasurer::new();
        let v = m.measure_vertical("漢字", 18.0);
        let h = m.measure("漢字", 18.0, None);
        assert_ne!(v, h, "vertical and horizontal dims must differ for CJK");
        // Re-measuring returns the cached value unchanged.
        assert_eq!(m.measure_vertical("漢字", 18.0), v);
        assert_eq!(m.measure("漢字", 18.0, None), h);
    }

    // GPU-gated: exercise the real `build_vertices` vertical layout via the shape
    // cache (stacking, in-column centering, and the byte→content offset map).

    /// Pull the cached `ShapedGlyph`s for a block's content, sorted by `rel_y`
    /// then `rel_x` (visual top→bottom, left→right within a row).
    #[cfg(test)]
    fn cached_vertical_glyphs(r: &TextRenderer, content: &str) -> Vec<(f32, f32, u32)> {
        let shared = r.font_system.lock().unwrap();
        let mut out: Vec<(f32, f32, u32)> = shared
            .kept_layouts(content)
            .flat_map(|layout| layout.glyphs.iter())
            .map(|g| (g.rel_x, g.rel_y, g.byte_start))
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        out
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn vertical_stacks_glyphs_top_to_bottom() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        let content = "あいう";
        let block = TextBlock::new(content, 0.0, 0.0)
            .with_size(24.0)
            .with_font(font)
            .with_vertical();
        r.build_vertices(&[block]);

        let glyphs = cached_vertical_glyphs(&r, content);
        assert_eq!(glyphs.len(), 3, "three kana → three glyphs");
        // Strictly descending rows.
        assert!(glyphs[0].1 < glyphs[1].1 && glyphs[1].1 < glyphs[2].1);
        // Byte offsets map back to the caller's content (3-byte kana: 0,3,6) —
        // guards the `glyph.start - run.line_i` correction.
        assert_eq!(glyphs[0].2, 0);
        assert_eq!(glyphs[1].2, 3);
        assert_eq!(glyphs[2].2, 6);
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn vertical_centers_narrow_glyph_in_column() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // A wide full-width kanji over a thin Latin 'l': the narrow row gets pushed
        // right so it centers under the wide one (manual in-column centering).
        let content = "漢l";
        let block = TextBlock::new(content, 0.0, 0.0)
            .with_size(28.0)
            .with_font(font)
            .with_vertical();
        r.build_vertices(&[block]);

        let glyphs = cached_vertical_glyphs(&r, content);
        assert_eq!(glyphs.len(), 2);
        let (wide_x, _, _) = glyphs[0]; // top row = 漢
        let (thin_x, _, _) = glyphs[1]; // bottom row = l
        assert!(
            thin_x > wide_x,
            "narrow glyph ({thin_x}) should be centered right of the wide one ({wide_x})"
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter (DISPLAY=:0)"]
    fn vertical_align_center_offsets_column_within_max_width() {
        let Some((_d, _q, mut r, font)) = headless_renderer() else {
            return;
        };
        // Full-width kana (equal advance) so per-row centering is zero — any x
        // offset is purely the column being centred within `max_width`.
        let content = "あいう";
        let max_width = 120.0;
        let block = TextBlock::new(content, 0.0, 0.0)
            .with_size(24.0)
            .with_font(font)
            .with_max_width(max_width)
            .with_align(TextAlign::Center)
            .with_vertical();
        r.build_vertices(&[block]);

        let glyphs = cached_vertical_glyphs(&r, content);
        assert_eq!(glyphs.len(), 3);
        let min_x = glyphs.iter().fold(f32::MAX, |m, g| m.min(g.0));
        let max_x = glyphs.iter().fold(0.0f32, |m, g| m.max(g.0));
        // Clearly inset from the left (a left-flushed column starts at ~0) and
        // still within the box — i.e. the column sits centred, not flush-left.
        assert!(
            (30.0..70.0).contains(&min_x),
            "centred column should be inset ~half the slack from the left (min_x={min_x})"
        );
        assert!(
            max_x < max_width,
            "column stays within max_width (max_x={max_x})"
        );
    }
}

#[cfg(all(test, feature = "phosphor-icons"))]
mod icon_tests {
    use super::*;

    #[test]
    fn fit_centered_em_full_glyph_maps_to_smaller_dimension() {
        // A full-em glyph in a wide rect: 1 em → height (the smaller dim), so the
        // quad is a height-sized square centered on x.
        let rect = Rect::new(0.0, 0.0, 100.0, 20.0);
        let (x0, y0, x1, y1) = fit_centered(rect, 1.0, 1.0);
        assert!(
            (y0 - 0.0).abs() < 1e-4 && (y1 - 20.0).abs() < 1e-4,
            "fills height"
        );
        assert!(((x1 - x0) - 20.0).abs() < 1e-4, "quad is em(=height)-sized");
        assert!((x0 - 40.0).abs() < 1e-4, "h-centered: left margin {x0}");
        assert!((x1 - 60.0).abs() < 1e-4, "right edge {x1}");
    }

    #[test]
    fn fit_centered_em_full_glyph_in_tall_rect() {
        let rect = Rect::new(0.0, 0.0, 20.0, 100.0);
        let (x0, y0, x1, y1) = fit_centered(rect, 1.0, 1.0);
        assert!(
            (x0 - 0.0).abs() < 1e-4 && (x1 - 20.0).abs() < 1e-4,
            "fills width"
        );
        assert!(((y1 - y0) - 20.0).abs() < 1e-4, "em(=width)-sized");
        assert!(
            (y0 - 40.0).abs() < 1e-4 && (y1 - 60.0).abs() < 1e-4,
            "v-centered"
        );
    }

    #[test]
    fn fit_centered_uses_shared_em_scale_not_per_glyph_fit() {
        // The whole point of the fix: a tall glyph and a short-wide "minus-like"
        // glyph in the SAME cell share one scale (1 em → cell size). The minus
        // stays a short bar — it does NOT stretch to fill the cell width.
        let rect = Rect::new(0.0, 0.0, 40.0, 40.0);
        let (px0, py0, px1, py1) = fit_centered(rect, 0.75, 0.75); // plus-like
        let (mx0, my0, mx1, my1) = fit_centered(rect, 0.75, 0.06); // minus-like
        // Identical em scale → identical width for equal w_em.
        assert!(
            ((px1 - px0) - (mx1 - mx0)).abs() < 1e-4,
            "same width: shared scale"
        );
        assert!((px1 - px0 - 30.0).abs() < 1e-4, "0.75 em * 40 = 30");
        // The minus is short, not stretched to the cell.
        assert!(((my1 - my0) - 2.4).abs() < 1e-4, "minus stays a 0.06em bar");
        // Both centered.
        assert!(
            (px0 - 5.0).abs() < 1e-4 && (mx0 - 5.0).abs() < 1e-4,
            "h-centered"
        );
        assert!(((py0 + py1) * 0.5 - 20.0).abs() < 1e-4, "plus v-centered");
        assert!(((my0 + my1) * 0.5 - 20.0).abs() < 1e-4, "minus v-centered");
    }

    #[test]
    fn icon_atlas_generates_and_caches_a_tile() {
        let mut atlas = MsdfGlyphAtlas::with_params(ICON_REF_PX, DEFAULT_PX_RANGE);
        let g = PhosphorIcon::Plus.glyph().expect("Plus resolves");
        let data = icon_font_snapshot()[g.font.index() as usize];
        let key = g.font.index() as u64;

        let t1 = atlas
            .glyph(key, g.glyph_id, data)
            .expect("Plus generates a tile");
        assert!(t1.region.w > 0 && t1.region.h > 0);
        // A real icon has horizontal and vertical extent.
        assert!(t1.metrics.right_em > t1.metrics.left_em);
        assert!(t1.metrics.top_em > t1.metrics.bottom_em);

        // Cached: same tile, no new packing.
        let t2 = atlas.glyph(key, g.glyph_id, data).expect("cached");
        assert_eq!(t1, t2);
    }

    /// Two icon fonts share one atlas, so the resolution `render_icons` performs
    /// — index the font snapshot by `IconGlyph::font`, key the atlas by the same
    /// id — must give the *same glyph index* in different fonts different tiles.
    /// Getting this wrong would silently draw font A's art for font B's icon.
    #[cfg(all(feature = "phosphor-icons", feature = "bundled-font"))]
    #[test]
    fn two_icon_fonts_resolve_to_distinct_tiles_for_the_same_glyph_index() {
        use crate::render::register_icon_font;

        let other = register_icon_font(
            "text-icons-under-test",
            include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf"),
        )
        .expect("IBM Plex Sans parses as a face");
        let phosphor = PhosphorIcon::Gear.glyph().expect("gear resolves");
        // Deliberately the *same* glyph index in the other font — the font id is
        // the only thing that may distinguish these two.
        let twin = IconGlyph {
            font: other,
            glyph_id: phosphor.glyph_id,
        };

        let fonts = icon_font_snapshot();
        let mut atlas = MsdfGlyphAtlas::with_params(ICON_REF_PX, DEFAULT_PX_RANGE);
        let a = atlas
            .glyph(
                phosphor.font.index() as u64,
                phosphor.glyph_id,
                fonts[phosphor.font.index() as usize],
            )
            .expect("phosphor tile");
        let b = atlas
            .glyph(
                twin.font.index() as u64,
                twin.glyph_id,
                fonts[twin.font.index() as usize],
            )
            .expect("other-font tile");
        assert_ne!(
            a.region, b.region,
            "same glyph index in two fonts must pack to two tiles"
        );
    }

    #[test]
    fn push_icon_quad_emits_six_verts_with_tint_and_clip() {
        use crate::affine::Affine2;
        let mut atlas = MsdfGlyphAtlas::with_params(ICON_REF_PX, DEFAULT_PX_RANGE);
        let g = PhosphorIcon::Check.glyph().unwrap();
        let data = icon_font_snapshot()[g.font.index() as usize];
        let tile = atlas
            .glyph(g.font.index() as u64, g.glyph_id, data)
            .unwrap();

        let icon = IconMsdf {
            local: Rect::new(0.0, 0.0, 32.0, 32.0),
            transform: Affine2::translation(100.0, 50.0),
            glyph: g,
            tint: [0.2, 0.4, 0.6, 1.0],
            clip: Some(Rect::new(0.0, 0.0, 200.0, 200.0)),
        };
        let mut out = Vec::new();
        push_icon_quad(
            &mut out,
            &tile,
            &icon,
            atlas.width(),
            atlas.height(),
            atlas.px_range(),
        );
        assert_eq!(out.len(), 6, "two triangles");
        assert_eq!(out[0].fill, [0.2, 0.4, 0.6, 1.0]);
        assert_eq!(out[0].clip_enabled, 1.0);
        // The translate transform shifts the whole quad by (100, 50). The icon is
        // centred in its 32x32 local rect (centre (16,16)), so the quad centroid
        // lands at (116, 66) in world space regardless of the glyph's tile size.
        let cx = out.iter().map(|v| v.position[0]).sum::<f32>() / out.len() as f32;
        let cy = out.iter().map(|v| v.position[1]).sum::<f32>() / out.len() as f32;
        assert!((cx - 116.0).abs() < 1e-2, "centroid x {cx}");
        assert!((cy - 66.0).abs() < 1e-2, "centroid y {cy}");
    }
}
