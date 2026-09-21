//! Core drawing types - vertices, draw commands, and the DrawList.

use crate::affine::Affine2;
use crate::chrome::{Background, Edge, EdgeWidths, GradientAxis, QuadStyle};
use crate::layout::Rect;
use crate::render::SpriteId;
#[cfg(feature = "phosphor-icons")]
use crate::render::{IconGlyph, PhosphorIcon};
use crate::shadow::{BoxShadow, CornerRadii, ShadowInstance};
use crate::text::{FontHandle, FontSystemHandle, FontVMetrics, TextBlock, TextMeasurer, Underline};

const ANALYTIC_KIND_CHROME: u32 = 0;
const ANALYTIC_KIND_SHADOW: u32 = 1;

pub(crate) const ROUNDED_RECT_CORNER_SEGMENTS: usize = 8;

/// Monotonic source of per-`DrawList` identity. Each `DrawList` gets a unique,
/// never-reused id at construction so the renderer can detect the "freshly
/// constructed every frame" footgun (a per-frame-new list can never warm its
/// text-measure cache — see [`crate::render`]'s stale-list detector).
static NEXT_DRAW_LIST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Allocate the next unique `DrawList` id.
fn next_draw_list_id() -> u64 {
    NEXT_DRAW_LIST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A colored vertex for triangle-based rendering.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// World-space position `[x, y]`.
    pub position: [f32; 2],
    /// RGBA color (tint already applied).
    pub color: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `clip_enabled > 0.5`).
    pub clip: [f32; 4],
    /// `1.0` enables `clip`, `0.0` draws unclipped.
    pub clip_enabled: f32,
}

impl Vertex {
    /// Build an unclipped vertex at `(x, y)` with the given color.
    pub fn new(x: f32, y: f32, color: [f32; 4]) -> Self {
        Self {
            position: [x, y],
            color,
            clip: [0.0; 4],
            clip_enabled: 0.0,
        }
    }

    /// Attach a clip rect, enabling clipping for this vertex; `None` leaves it
    /// unclipped.
    pub fn with_clip(mut self, clip: Option<Rect>) -> Self {
        if let Some(clip) = clip {
            self.clip = [clip.x, clip.y, clip.width, clip.height];
            self.clip_enabled = 1.0;
        }
        self
    }
}

/// A textured quad command (e.g. an icon from a texture atlas).
///
/// Carries pre-transformed corners in TL, TR, BR, BL order so rotated/scaled
/// sprites tessellate correctly.
///
/// `sprite` is the resolved atlas handle. When `None`, the renderer falls back
/// to looking up `icon_key` in the atlas at render time (slightly slower; one
/// `HashMap<String, SpriteId>` lookup per icon per frame). Prefer resolving the
/// sprite once at registration via [`DrawList::icon_sprite`].
#[derive(Clone, Debug)]
pub struct IconDraw {
    /// Pre-transformed corners, TL/TR/BR/BL.
    pub corners: [[f32; 2]; 4],
    /// Pre-resolved atlas handle, if known.
    pub sprite: Option<SpriteId>,
    /// Name fallback for late-resolved sprites.
    pub icon_key: String,
    /// Multiplied with sampled atlas color. Default white.
    pub tint: [f32; 4],
    /// Optional clip rect; `None` draws unclipped.
    pub clip: Option<Rect>,
    /// Optional normalized source sub-rect `[u0, v0, u1, v1]` (0..1 within the
    /// sprite) for cropped draws. `None` draws the whole sprite. Resolved
    /// against the atlas region at render time.
    pub src: Option<[f32; 4]>,
    /// Tile-wrap flag: when `true`, `src` is a UV span in *tile units*
    /// (u1/v1 may exceed 1) and the fragment shader repeats the source
    /// region modulo its size, cropping at the draw's edges. Written only by
    /// [`DrawList::image_tiled`].
    pub wrap: bool,
}

/// A single instanced "chrome" rect (button background + border) for the SDF
/// rounded-rect pipeline.
///
/// Field layout matches the per-instance vertex attributes in `ui.wgsl`
/// (`vs_chrome`), so a `&[ChromeInstance]` uploads straight to the instance
/// buffer with no repacking. All geometry is computed in the fragment shader
/// from these values, which is why thousands of identical-shape buttons collapse
/// to one base mesh + N small records instead of re-tessellating ~80 verts each.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChromeInstance {
    /// Forward affine linear part `[a, b, c, d]`.
    pub linear: [f32; 4],
    /// `[tx, ty, clip_enabled, horizontal_gradient]`.
    pub translation: [f32; 4],
    /// Local-space rect `[x, y, width, height]`.
    pub rect: [f32; 4],
    /// Fill (background) color at the gradient's start edge, tint applied.
    pub bg: [f32; 4],
    /// Fill color at the gradient's end edge; equal to `bg` for a solid fill.
    pub bg2: [f32; 4],
    /// Shared border color, tint applied.
    pub border: [f32; 4],
    /// Outer radii in top-left, top-right, bottom-right, bottom-left order.
    pub radii: [f32; 4],
    /// Inward border widths in top, right, bottom, left order.
    pub widths: [f32; 4],
    /// World-space clip rect `[x, y, width, height]`.
    pub clip: [f32; 4],
    /// Compatibility metadata `[uniform_radius, uniform_width, clip_enabled, _]`.
    pub params: [f32; 4],
}

/// One ordered analytic GPU instance. The ten `vec4` payload slots are followed
/// by an explicit integer tag and padding, keeping one upload-ready record for
/// both chrome and shadows without per-frame conversion.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AnalyticInstance {
    /// Kind-specific payload, uploaded directly as ten `vec4<f32>` attributes.
    pub payload: [[f32; 4]; 10],
    /// Flat shader tag: zero for chrome, one for shadow.
    pub kind: u32,
    /// Explicit alignment padding; keeps the record stride vertex-buffer safe.
    pub padding: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<AnalyticInstance>() == 176);
const _: () = assert!(std::mem::offset_of!(AnalyticInstance, kind) == 160);

impl AnalyticInstance {
    fn chrome(value: ChromeInstance) -> Self {
        Self {
            payload: [
                value.linear,
                value.translation,
                value.rect,
                value.bg,
                value.bg2,
                value.border,
                value.radii,
                value.widths,
                value.clip,
                value.params,
            ],
            kind: ANALYTIC_KIND_CHROME,
            padding: [0; 3],
        }
    }

    fn shadow(value: ShadowInstance) -> Self {
        Self {
            payload: [
                value.linear,
                value.translation,
                value.raster_rect,
                value.shadow_rect,
                value.element_rect,
                value.color,
                value.shadow_radii,
                value.element_radii,
                value.clip,
                value.params,
            ],
            kind: ANALYTIC_KIND_SHADOW,
            padding: [0; 3],
        }
    }

    /// Decode this record as chrome, or return `None` for another kind.
    pub fn as_chrome(&self) -> Option<ChromeInstance> {
        (self.kind == ANALYTIC_KIND_CHROME).then(|| ChromeInstance {
            linear: self.payload[0],
            translation: self.payload[1],
            rect: self.payload[2],
            bg: self.payload[3],
            bg2: self.payload[4],
            border: self.payload[5],
            radii: self.payload[6],
            widths: self.payload[7],
            clip: self.payload[8],
            params: self.payload[9],
        })
    }

    /// Decode this record as a shadow, or return `None` for another kind.
    pub fn as_shadow(&self) -> Option<ShadowInstance> {
        (self.kind == ANALYTIC_KIND_SHADOW).then(|| ShadowInstance {
            linear: self.payload[0],
            translation: self.payload[1],
            raster_rect: self.payload[2],
            shadow_rect: self.payload[3],
            element_rect: self.payload[4],
            color: self.payload[5],
            shadow_radii: self.payload[6],
            element_radii: self.payload[7],
            clip: self.payload[8],
            params: self.payload[9],
        })
    }
}

/// A single instanced circle (filled disc or ring outline) for the SDF circle
/// pipeline.
///
/// Field layout matches the per-instance vertex attributes in `ui.wgsl`
/// (`vs_circle`), so a `&[CircleInstance]` uploads straight to the instance
/// buffer. The fragment computes the disc/ring from a signed distance, so a
/// smooth anti-aliased circle of any size is one base mesh + one small record
/// instead of a re-tessellated 16-64-segment fan every frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CircleInstance {
    /// `[center_x, center_y, radius, thickness]` (transform already baked in).
    /// `thickness <= 0` is a filled disc; `> 0` is a ring centered on `radius`.
    pub center: [f32; 4],
    /// Color, tint already applied.
    pub color: [f32; 4],
    /// Clip rect `[x, y, w, h]` (ignored unless `params[0] > 0.5`).
    pub clip: [f32; 4],
    /// `[clip_enabled, _pad, _pad, _pad]`.
    pub params: [f32; 4],
}

/// One entry in a [`DrawList`]'s ordered paint stream.
///
/// The colored-quad stage is no longer a single soup draw: chrome rects are
/// instanced and must interleave with surrounding soup geometry in submission
/// order (a hover overlay quad drawn *over* a button, a panel *under* it). The
/// renderer walks these in order, drawing each soup index sub-range with the
/// color pipeline and each chrome instance sub-range with the instanced chrome
/// pipeline. When no `chrome_rect` is ever called the stream stays empty and the
/// renderer keeps its original single-draw fast path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintCmd {
    /// Draw soup index positions `start..end` (absolute into `indices`).
    Soup { indices: std::ops::Range<u32> },
    /// Draw ordered heterogeneous chrome/shadow instances `start..end`.
    Analytic { instances: std::ops::Range<u32> },
    /// Draw circle instances `start..end` (into `circle_instances`).
    Circle { instances: std::ops::Range<u32> },
    /// Draw nine-slice payloads `start..end`.
    NineSlice { draws: std::ops::Range<u32> },
    /// Draw atlas icon/image payloads `start..end`.
    Icon { draws: std::ops::Range<u32> },
    /// Draw MSDF icon payloads `start..end`.
    #[cfg(feature = "phosphor-icons")]
    IconMsdf { draws: std::ops::Range<u32> },
    /// Draw text payloads `start..end`.
    Text { draws: std::ops::Range<u32> },
}

/// Opaque handle to a registered nine-slice resource.
pub type NineSliceId = u32;

/// A nine-slice textured panel draw command.
///
/// Carries the local-space rect plus the affine transform that maps local
/// space to world (screen) space. Tessellation computes the 9 sub-rect
/// corners in local space and runs each through `transform`.
#[derive(Clone, Debug)]
pub struct NineSliceDraw {
    /// Local-space rect (pre-transform).
    pub local: Rect,
    /// Affine to apply to each corner during tessellation.
    pub transform: Affine2,
    /// Pre-resolved nine-slice handle.
    pub nine_slice: Option<NineSliceId>,
    /// Name fallback for late resolution.
    pub texture_key: String,
    /// Multiplied with sampled color. Default white.
    pub tint: [f32; 4],
    /// Optional clip rect; `None` draws unclipped.
    pub clip: Option<Rect>,
}

/// A vector icon drawn through the MSDF icon atlas (Phosphor).
///
/// Like [`NineSliceDraw`], this stores the **local rect + transform** rather than
/// pre-baked corners: the renderer fits-and-centers the glyph tile inside `local`
/// (so placement is bearing-independent) and then transforms the resulting quad
/// corners, giving rotation/scale support for free.
#[cfg(feature = "phosphor-icons")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconMsdf {
    /// Local-space rect (pre-transform) the icon is fit-centered into.
    pub local: Rect,
    /// Affine applied to each fitted quad corner during tessellation.
    pub transform: Affine2,
    /// Glyph resolved at push time — which icon font, and which glyph in it.
    /// Resolving here (rather than at render time) keeps the render pass free of
    /// registry lookups and lets a command outlive the enum it came from.
    pub glyph: IconGlyph,
    /// Multiplied with the sampled field's fill color. Default white.
    pub tint: [f32; 4],
    /// Optional clip rect; `None` draws unclipped.
    pub clip: Option<Rect>,
}

/// Per-buffer element counts — a snapshot of how much geometry a [`DrawList`]
/// held at one instant, or (as a difference of two snapshots) how much a
/// [`DebugScope`] emitted.
///
/// Every geometry buffer on `DrawList` is append-only between [`DrawList::clear`]
/// calls, so a pair of these snapshots delimits a contiguous, stable range in
/// each buffer. That is what lets debug scopes be recorded without touching a
/// single primitive method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrimCounts {
    /// Soup vertices ([`DrawList::vertices`]).
    pub vertices: usize,
    /// Soup indices ([`DrawList::indices`]).
    pub indices: usize,
    /// Text blocks ([`DrawList::texts`]).
    pub texts: usize,
    /// Atlas icon draws ([`DrawList::icons`]).
    pub icons: usize,
    /// Nine-slice draws ([`DrawList::nine_slices`]).
    pub nine_slices: usize,
    /// MSDF vector icon draws ([`DrawList::icons_msdf`]).
    #[cfg(feature = "phosphor-icons")]
    pub icons_msdf: usize,
    /// Instanced chrome rects ([`DrawList::chrome_instances`]).
    pub chrome_instances: usize,
    /// Instanced circles ([`DrawList::circle_instances`]).
    pub circle_instances: usize,
    /// Analytic shadows ([`DrawList::shadow_instances`]).
    pub shadow_instances: usize,
    /// Primitives silently dropped by a non-positive size/radius/thickness
    /// guard. These leave **no trace in any buffer**, so this counter is the
    /// only evidence that an element collapsed — see
    /// [`DrawList::dropped_degenerate`].
    pub dropped_degenerate: usize,
}

impl PrimCounts {
    /// Total number of drawn primitives, counting each soup *triangle* once
    /// (soup vertices are shared, so raw vertex count overstates the work).
    pub fn total(&self) -> usize {
        let soup_tris = self.indices / 3;
        #[cfg(feature = "phosphor-icons")]
        let msdf = self.icons_msdf;
        #[cfg(not(feature = "phosphor-icons"))]
        let msdf = 0;
        soup_tris
            + self.texts
            + self.icons
            + self.nine_slices
            + msdf
            + self.chrome_instances
            + self.circle_instances
            + self.shadow_instances
    }

    /// Element-wise `self - earlier`, saturating at zero. Use this to turn a
    /// scope's `(start, end)` snapshot pair into "what this scope emitted".
    pub fn since(&self, earlier: PrimCounts) -> PrimCounts {
        PrimCounts {
            vertices: self.vertices.saturating_sub(earlier.vertices),
            indices: self.indices.saturating_sub(earlier.indices),
            texts: self.texts.saturating_sub(earlier.texts),
            icons: self.icons.saturating_sub(earlier.icons),
            nine_slices: self.nine_slices.saturating_sub(earlier.nine_slices),
            #[cfg(feature = "phosphor-icons")]
            icons_msdf: self.icons_msdf.saturating_sub(earlier.icons_msdf),
            chrome_instances: self
                .chrome_instances
                .saturating_sub(earlier.chrome_instances),
            circle_instances: self
                .circle_instances
                .saturating_sub(earlier.circle_instances),
            shadow_instances: self
                .shadow_instances
                .saturating_sub(earlier.shadow_instances),
            dropped_degenerate: self
                .dropped_degenerate
                .saturating_sub(earlier.dropped_degenerate),
        }
    }

    /// True when no primitives at all fall in this span.
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// A named region of a [`DrawList`], recorded by
/// [`push_debug_scope`](DrawList::push_debug_scope) /
/// [`pop_debug_scope`](DrawList::pop_debug_scope).
///
/// Scopes are pure instrumentation — they emit no geometry and cost nothing
/// unless pushed. Each one owns the contiguous span of primitives emitted
/// between its push and its pop, which is what
/// [`DebugReport`](crate::debug::DebugReport) turns into a named, nested tree
/// of bounding boxes.
///
/// **Widget implementors** should use
/// [`push_debug_scope_rect`](DrawList::push_debug_scope_rect) and declare the
/// `Rect` the widget was handed: without a declared rect a scope's bounds are
/// derived from what it painted, so it can never be found to have overflowed,
/// and a widget that collapsed to nothing is indistinguishable from one that was
/// never drawn. This crate's own widgets all do it, so applications get those
/// checks for free and never supply geometry themselves — for an application, a
/// scope is only ever a label ([`UiContext::debug_scope`](crate::UiContext::debug_scope)).
#[derive(Clone, Debug, PartialEq)]
pub struct DebugScope {
    /// Caller-supplied label, e.g. `"settings_window/ok_button"`.
    pub name: String,
    /// Index of the enclosing scope in [`DrawList::debug_scopes`], if nested.
    pub parent: Option<usize>,
    /// Nesting depth (0 for a top-level scope).
    pub depth: usize,
    /// The box this scope said it would paint into, in **world space** (the
    /// active transform is applied at push time, as [`DrawList::push_clip`]
    /// does). `None` when opened via [`DrawList::push_debug_scope`].
    pub declared: Option<Rect>,
    /// The clip rect in force when the scope was pushed, in world space.
    /// Recorded directly rather than reconstructed from per-primitive clip
    /// data, so it stays correct for primitives that carry no clip of their own.
    pub clip: Option<Rect>,
    /// Buffer lengths when the scope was pushed.
    pub start: PrimCounts,
    /// Buffer lengths when the scope was popped. Equal to `start` until then;
    /// a scope left unpopped at report time is treated as closing at the end of
    /// the list.
    pub end: PrimCounts,
    /// False until [`pop_debug_scope`](DrawList::pop_debug_scope) closes it.
    pub closed: bool,
}

impl DebugScope {
    /// Primitives emitted inside this scope (including nested child scopes).
    pub fn counts(&self) -> PrimCounts {
        self.end.since(self.start)
    }
}

/// Draw list for collecting render commands.
///
/// Owns a transform stack and a tint stack: every primitive method consults
/// the top of both stacks at push time so widgets that already take absolute
/// `Rect`s remain transform-aware without code changes.
pub struct DrawList {
    /// Soup vertex buffer for color-stage geometry.
    pub vertices: Vec<Vertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
    /// Queued text blocks, rendered by the text pass.
    pub texts: Vec<TextBlock>,
    /// Queued textured-atlas icon draws.
    pub icons: Vec<IconDraw>,
    /// Queued nine-slice panel draws.
    pub nine_slices: Vec<NineSliceDraw>,
    /// MSDF vector icons (Phosphor), rendered by the text renderer's icon pass.
    #[cfg(feature = "phosphor-icons")]
    pub icons_msdf: Vec<IconMsdf>,
    /// Ordered upload-ready chrome and shadow records. Their source-over order
    /// is their vector order, so arbitrary alternation remains one GPU draw.
    pub analytic_instances: Vec<AnalyticInstance>,
    analytic_chrome_count: usize,
    analytic_shadow_count: usize,
    /// Instanced circles (filled discs + ring outlines). Drawn by the circle
    /// SDF pipeline; interleaved with soup/chrome via `DrawList::paint_cmds`.
    pub circle_instances: Vec<CircleInstance>,
    /// Ordered color-stage command stream (soup runs interleaved with chrome
    /// instance runs). Empty unless [`DrawList::chrome_rect`] was used, in which
    /// case the renderer falls back to a single soup draw.
    pub(crate) paint_cmds: Vec<PaintCmd>,
    /// Count of soup index positions already committed to a `Soup` command. Soup
    /// appended after the last command is the implicit trailing run.
    pub(crate) soup_committed_indices: u32,
    pub(crate) text_measurer: TextMeasurer,
    clip_stack: Vec<Rect>,
    /// World-space rects of clips pushed via
    /// [`push_clip_viewport`](DrawList::push_clip_viewport). See
    /// [`viewport_clips`](DrawList::viewport_clips).
    viewport_clips: Vec<Rect>,
    transform_stack: Vec<Affine2>,
    tint_stack: Vec<[f32; 4]>,
    /// Recorded debug scopes, in push order. Empty (and never allocated) unless
    /// [`push_debug_scope`](Self::push_debug_scope) is used, so instrumentation
    /// costs nothing when it is off.
    debug_scopes: Vec<DebugScope>,
    /// Indices into `debug_scopes` for the currently-open scopes.
    debug_scope_stack: Vec<usize>,
    /// Running count of primitives rejected by a non-positive size/radius/
    /// thickness guard. See [`DrawList::dropped_degenerate`].
    dropped_degenerate: u32,
    /// Logged-once flag for "tried to draw rotated text" — glyphon does not
    /// support rotation, so we silently render axis-aligned.
    text_rotation_warned: bool,
    /// Unique identity (see [`next_draw_list_id`]). Stable for this list's whole
    /// lifetime and never reused; lets the renderer tell a reused list from a
    /// per-frame-fresh one. `clear()` keeps it (the list is the same object).
    id: u64,
}

impl Default for DrawList {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texts: Vec::new(),
            icons: Vec::new(),
            nine_slices: Vec::new(),
            #[cfg(feature = "phosphor-icons")]
            icons_msdf: Vec::new(),
            analytic_instances: Vec::new(),
            analytic_chrome_count: 0,
            analytic_shadow_count: 0,
            circle_instances: Vec::new(),
            paint_cmds: Vec::new(),
            soup_committed_indices: 0,
            text_measurer: TextMeasurer::default(),
            clip_stack: Vec::new(),
            viewport_clips: Vec::new(),
            transform_stack: vec![Affine2::IDENTITY],
            tint_stack: vec![[1.0, 1.0, 1.0, 1.0]],
            debug_scopes: Vec::new(),
            debug_scope_stack: Vec::new(),
            dropped_degenerate: 0,
            text_rotation_warned: false,
            id: next_draw_list_id(),
        }
    }
}

impl DrawList {
    /// Create an empty draw list with its own (freshly-scanned) font system.
    /// Prefer [`DrawList::with_font_system`] to share a font system and avoid a
    /// per-instance font-database scan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a `DrawList` whose measurer shares the given `FontSystem`.
    ///
    /// Use this together with `TextRenderer::font_system_handle()` so measured text
    /// widths match what gets rendered to screen.
    ///
    /// IMPORTANT: this builds every field explicitly rather than via
    /// `..Self::default()`. Struct-update syntax would fully evaluate
    /// `Self::default()` first — which constructs a throwaway
    /// `TextMeasurer::default()` whose `FontSystem::new()` scans the entire
    /// system font database (multiple milliseconds) — only to immediately
    /// overwrite and drop it. Callers that build a `DrawList` per frame would
    /// pay that font-DB scan every frame. Constructing fields directly with the
    /// caller-supplied (shared) font system avoids the wasted scan entirely.
    pub fn with_font_system(font_system: FontSystemHandle) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texts: Vec::new(),
            icons: Vec::new(),
            nine_slices: Vec::new(),
            #[cfg(feature = "phosphor-icons")]
            icons_msdf: Vec::new(),
            analytic_instances: Vec::new(),
            analytic_chrome_count: 0,
            analytic_shadow_count: 0,
            circle_instances: Vec::new(),
            paint_cmds: Vec::new(),
            soup_committed_indices: 0,
            text_measurer: TextMeasurer::with_font_system(font_system),
            clip_stack: Vec::new(),
            viewport_clips: Vec::new(),
            transform_stack: vec![Affine2::IDENTITY],
            tint_stack: vec![[1.0, 1.0, 1.0, 1.0]],
            debug_scopes: Vec::new(),
            debug_scope_stack: Vec::new(),
            dropped_degenerate: 0,
            text_rotation_warned: false,
            id: next_draw_list_id(),
        }
    }

    /// This list's unique, lifetime-stable identity (preserved across
    /// [`clear`](Self::clear)). Two distinct `DrawList`s never share an id, and
    /// ids are never reused — so a caller that builds a fresh `DrawList` every
    /// frame yields a different id each frame, which the renderer uses to flag the
    /// "cache can never warm" footgun.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Clear all queued geometry/commands and reset the clip, transform, tint,
    /// and debug-scope stacks to their base state, ready to reuse for the next
    /// frame. The shared font system / measurer is retained.
    pub fn clear(&mut self) {
        self.debug_scopes.clear();
        self.debug_scope_stack.clear();
        self.dropped_degenerate = 0;
        self.vertices.clear();
        self.indices.clear();
        self.texts.clear();
        self.icons.clear();
        self.nine_slices.clear();
        #[cfg(feature = "phosphor-icons")]
        self.icons_msdf.clear();
        self.analytic_instances.clear();
        self.analytic_chrome_count = 0;
        self.analytic_shadow_count = 0;
        self.circle_instances.clear();
        self.paint_cmds.clear();
        self.soup_committed_indices = 0;
        self.clip_stack.clear();
        self.viewport_clips.clear();
        self.transform_stack.clear();
        self.transform_stack.push(Affine2::IDENTITY);
        self.tint_stack.clear();
        self.tint_stack.push([1.0, 1.0, 1.0, 1.0]);
    }

    /// Measure text using glyphon's shaping/layout path.
    ///
    /// Pass `max_width = None` for unconstrained single-line measurement, or
    /// `Some(w)` to let glyphon wrap and report the resulting multi-line height.
    pub fn measure_text(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.text_measurer.measure(text, font_size, max_width)
    }

    /// Measure a queued [`TextBlock`] exactly as it will be laid out — font,
    /// weight, style, wrap, and vertical stacking included. See
    /// [`TextMeasurer::measure_block`]; prefer this over
    /// [`measure_text`](Self::measure_text), which assumes the default font at
    /// normal weight and style.
    pub fn measure_block(&mut self, block: &TextBlock) -> (f32, f32) {
        self.text_measurer.measure_block(block)
    }

    /// Borrow the narrow CPU text-measurement service used by contextual widget
    /// measurement. It shares font state and caches with this draw list.
    pub(crate) fn text_measurer_mut(&mut self) -> &mut TextMeasurer {
        &mut self.text_measurer
    }

    /// The band of real glyph ink a queued [`TextBlock`] paints, as `(top,
    /// bottom)` offsets below its top edge. `None` when it inks nothing.
    ///
    /// See [`TextMeasurer::measure_block_ink`] — this is the *painted* extent,
    /// where [`measure_block`](Self::measure_block) is the *reserved* one, and a
    /// vertically centred label deliberately makes those two disagree.
    pub fn measure_block_ink(&mut self, block: &TextBlock) -> Option<(f32, f32)> {
        self.text_measurer.measure_block_ink(block)
    }

    /// Per-font vertical metrics for optical (cap-height) centring, for the given
    /// font at Normal weight/style (the only combination widget labels centre).
    /// Cached per font. Exposed mainly so debug tooling can draw the band; most
    /// callers want [`Self::vcentered_text_y`].
    pub fn font_vmetrics(&mut self, font: Option<&FontHandle>) -> FontVMetrics {
        self.text_measurer.vmetrics(
            font,
            cosmic_text::Weight::NORMAL,
            cosmic_text::Style::Normal,
        )
    }

    /// Top `y` for a single-line text block of `font_size` so the label `text` is
    /// *optically* centred over the span `[top, top + height]`, using the font's
    /// real metrics.
    ///
    /// This is the font-aware counterpart of [`crate::text::vcentered_line_y`]:
    /// where that centres the em box (ascent+descent, biased low by the empty
    /// descent space), this centres the band the label's visual mass occupies.
    /// Which band that is depends on the text — labels with lowercase letters
    /// centre on the **x-height** body, all-caps/numeric labels on the taller
    /// **cap-height** band, and labels containing **CJK** on the ideographic ink
    /// centre (see [`FontVMetrics::visual_center_ratio`](crate::FontVMetrics::visual_center_ratio)) — so text reads as
    /// centred across scripts and case. Pass the same `font` the block renders
    /// with. Degrades to em-box centring when the metrics are unavailable.
    pub fn vcentered_text_y(
        &mut self,
        top: f32,
        height: f32,
        font_size: f32,
        font: Option<&FontHandle>,
        text: &str,
    ) -> f32 {
        let m = self.font_vmetrics(font);
        top + height / 2.0 - font_size * m.visual_center_ratio(text)
    }

    /// Compute per-character cursor x-positions for the given text.
    ///
    /// Returns a `Vec<(usize, f32)>` mapping byte indices in `text` to their
    /// x-offset (pixels) from the left edge. Use this for click-to-position
    /// cursor placement and selection highlight rendering.
    ///
    /// `max_width` constrains the layout width (glyphon may wrap). Pass
    /// `font_size * 0.0` for single-line mode with effectively infinite width.
    pub fn text_cursor_positions(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> Vec<(usize, f32)> {
        let handle = self.text_measurer.font_system_handle();
        let mut fs = handle.lock().expect("FontSystem poisoned");
        let mw = max_width.unwrap_or(f32::MAX / 4.0);
        let lh = font_size * 1.25;
        crate::text::text_cursor_positions(&mut fs, text, font_size, lh, mw, None)
    }

    /// Line-aware caret layout for the given text — the multi-line counterpart of
    /// [`Self::text_cursor_positions`]. Returns one [`crate::text::CaretPos`] per
    /// cluster boundary, preserving per-visual-line geometry (line index, top y,
    /// height) needed for vertical navigation, per-line selection, and
    /// click-to-place hit testing in a multi-line `TextInput`.
    ///
    /// `wrap` controls line breaking (use [`crate::text::WrapMode::None`] for
    /// single-line, [`crate::text::WrapMode::WordOrGlyph`] for a textarea).
    /// `max_width` constrains the layout width; pass `None` for effectively
    /// infinite width (single-line).
    pub fn text_caret_layout(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        wrap: crate::text::WrapMode,
        direction: crate::text::TextDirection,
    ) -> Vec<crate::text::CaretPos> {
        let handle = self.text_measurer.font_system_handle();
        let mut fs = handle.lock().expect("FontSystem poisoned");
        let mw = max_width.unwrap_or(f32::MAX / 4.0);
        let lh = font_size * 1.25;
        crate::text::text_caret_layout(&mut fs, text, font_size, lh, mw, wrap, None, direction)
    }

    /// Visual-order glyph layout for bidi-aware editing — the source for
    /// [`crate::text::selection_rects`] and [`crate::text::visual_caret_neighbor`].
    /// Same shaping parameters as [`Self::text_caret_layout`].
    pub fn text_visual_layout(
        &mut self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
        wrap: crate::text::WrapMode,
        direction: crate::text::TextDirection,
    ) -> Vec<crate::text::VisualGlyph> {
        let handle = self.text_measurer.font_system_handle();
        let mut fs = handle.lock().expect("FontSystem poisoned");
        let mw = max_width.unwrap_or(f32::MAX / 4.0);
        let lh = font_size * 1.25;
        crate::text::text_visual_layout(&mut fs, text, font_size, lh, mw, wrap, None, direction)
    }

    // ---- Debug scopes ----

    /// Current buffer lengths, as a [`PrimCounts`] snapshot.
    pub fn prim_counts(&self) -> PrimCounts {
        PrimCounts {
            vertices: self.vertices.len(),
            indices: self.indices.len(),
            texts: self.texts.len(),
            icons: self.icons.len(),
            nine_slices: self.nine_slices.len(),
            #[cfg(feature = "phosphor-icons")]
            icons_msdf: self.icons_msdf.len(),
            chrome_instances: self.analytic_chrome_count,
            circle_instances: self.circle_instances.len(),
            shadow_instances: self.analytic_shadow_count,
            dropped_degenerate: self.dropped_degenerate as usize,
        }
    }

    /// Open a named debug scope. Everything drawn until the matching
    /// [`pop_debug_scope`](Self::pop_debug_scope) is attributed to `name` in the
    /// [`DebugReport`](crate::debug::DebugReport).
    ///
    /// Emits no geometry and does not affect rendering in any way. Scopes nest.
    ///
    /// This is the labelling form. If you are **implementing a widget**, use
    /// [`push_debug_scope_rect`](Self::push_debug_scope_rect) instead and declare
    /// the `Rect` you were handed.
    pub fn push_debug_scope(&mut self, name: impl Into<String>) {
        self.open_debug_scope(name.into(), None);
    }

    /// Open a named debug scope declaring the box it was allocated — **the entry
    /// point for widget implementors**.
    ///
    /// Pass the `Rect` the widget received from layout (or, for a widget that
    /// stores its own geometry, the rect it derives from its fields). That is the
    /// one fact the draw list cannot recover on its own: it records what was
    /// painted, never what was assigned. Declaring it is what lets the report
    /// tell "painted outside its box" and "drew nothing at all" apart from a
    /// region that simply had little to draw.
    ///
    /// Applications never need this — every widget in this crate declares its
    /// own allocation, so a caller only ever supplies a *label* via
    /// [`push_debug_scope`](Self::push_debug_scope). Downstream custom widgets
    /// should call this for the same reason the built-ins do.
    ///
    /// `rect` is in local space and transformed to its world-space AABB by the
    /// active transform, matching [`push_clip`](Self::push_clip).
    pub fn push_debug_scope_rect(&mut self, name: impl Into<String>, rect: Rect) {
        let world = self.current_transform().transform_rect_aabb(rect);
        self.open_debug_scope(name.into(), Some(world));
    }

    fn open_debug_scope(&mut self, name: String, declared: Option<Rect>) {
        let start = self.prim_counts();
        let parent = self.debug_scope_stack.last().copied();
        let depth = self.debug_scope_stack.len();
        let index = self.debug_scopes.len();
        self.debug_scopes.push(DebugScope {
            name,
            parent,
            depth,
            declared,
            clip: self.current_clip(),
            start,
            end: start,
            closed: false,
        });
        self.debug_scope_stack.push(index);
    }

    /// Close the innermost open debug scope. No-op when none is open, so an
    /// unbalanced pop cannot corrupt the record (it just loses attribution).
    pub fn pop_debug_scope(&mut self) {
        let Some(index) = self.debug_scope_stack.pop() else {
            return;
        };
        let end = self.prim_counts();
        let scope = &mut self.debug_scopes[index];
        scope.end = end;
        scope.closed = true;
    }

    /// All debug scopes recorded on this list, in push order. Parent indices in
    /// [`DebugScope::parent`] refer into this slice.
    pub fn debug_scopes(&self) -> &[DebugScope] {
        &self.debug_scopes
    }

    /// How many primitives were rejected by a non-positive size, radius, or
    /// thickness guard since the last [`clear`](Self::clear).
    ///
    /// This is the crate's only signal for a whole class of layout bug. Every
    /// primitive returns early on a degenerate rect — `quad` bails on
    /// `width <= 0.0`, and so on — which means an element whose size was
    /// computed as, say, `rect.width - padding * 2.0` and came out negative
    /// **vanishes leaving nothing at all in any buffer**. No amount of
    /// inspecting the geometry can distinguish that from an element that was
    /// never meant to be drawn; a non-zero count here can.
    ///
    /// Note that a legitimately hidden element (an `if` that chose not to draw)
    /// does *not* increment this — only one that asked to draw something
    /// impossible.
    pub fn dropped_degenerate(&self) -> u32 {
        self.dropped_degenerate
    }

    /// Number of debug scopes currently open. Callers that scope by depth (as
    /// [`UiContext`](crate::UiContext) does) snapshot this and
    /// [`truncate_debug_scopes`](Self::truncate_debug_scopes) back to it.
    pub fn debug_scope_depth(&self) -> usize {
        self.debug_scope_stack.len()
    }

    /// Close open debug scopes until only `depth` remain (no-op if already at
    /// or below `depth`).
    pub fn truncate_debug_scopes(&mut self, depth: usize) {
        while self.debug_scope_stack.len() > depth {
            self.pop_debug_scope();
        }
    }

    // ---- Clip stack ----

    /// Push a clipping rectangle. Nested clips are intersected with the current clip.
    ///
    /// **Note:** when the active transform has rotation or shear, the rect is
    /// transformed to its AABB before being intersected; clipping is therefore
    /// approximate (over-clips along the diagonal) under rotation. Document the
    /// limitation rather than silently drawing wrong.
    pub fn push_clip(&mut self, rect: Rect) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            log::warn!(
                "push_clip: degenerate input rect ({w:.1}×{h:.1} at ({x:.0},{y:.0})) — \
                 all content under this clip will be invisible. Check that the \
                 widget's padding doesn't exceed its size.",
                w = rect.width,
                h = rect.height,
                x = rect.x,
                y = rect.y,
            );
        }
        let world_rect = self.current_transform().transform_rect_aabb(rect);
        let clip = match self.current_clip() {
            Some(current) => current
                .intersection(world_rect)
                .unwrap_or_else(|| Rect::new(world_rect.x, world_rect.y, 0.0, 0.0)),
            None => world_rect,
        };
        self.clip_stack.push(clip);
    }

    /// Push a clipping rectangle **without** intersecting the parent clip — the
    /// new clip *replaces* whatever was active (Teardown's `UiClipRect`/`UiWindow`
    /// with `inherit = false`). The rect is still transformed to its world-space
    /// AABB by the active transform, with the same rotation caveat as
    /// [`push_clip`](Self::push_clip).
    pub fn push_clip_exact(&mut self, rect: Rect) {
        let world_rect = self.current_transform().transform_rect_aabb(rect);
        self.clip_stack.push(world_rect);
    }

    /// Push a clip that is a **viewport**: a deliberately small window onto
    /// content that is expected to be larger than it — a scroll view, a
    /// dropdown's option list, a horizontally-scrolled text field.
    ///
    /// Geometrically identical to [`push_clip`](Self::push_clip). The only
    /// difference is intent, and intent is exactly what the debug report cannot
    /// infer: a clip that removes an element entirely is a layout bug when the
    /// clip is a hard boundary (a window, a panel) and *the whole point* when it
    /// is a viewport. Recording which is which is what lets
    /// [`DebugReport`](crate::debug::DebugReport) flag the first and stay quiet
    /// about the second.
    ///
    /// The recorded rect is the effective (already intersected, world-space)
    /// clip, and the marking is sticky for the subtree: anything drawn while
    /// this clip is active — including under further nested clips — counts as
    /// living inside a viewport.
    pub fn push_clip_viewport(&mut self, rect: Rect) {
        self.push_clip(rect);
        if let Some(clip) = self.current_clip() {
            self.viewport_clips.push(clip);
        }
    }

    /// Effective world-space rects of every viewport clip pushed this frame, in
    /// push order. Read by the debug report; see
    /// [`push_clip_viewport`](Self::push_clip_viewport).
    pub fn viewport_clips(&self) -> &[Rect] {
        &self.viewport_clips
    }

    /// Pop the current clipping rectangle.
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    /// Number of clips currently on the stack. Used to scope clips to a
    /// push/pop frame (record the depth on push, [`truncate_clip`](Self::truncate_clip) back on pop).
    pub fn clip_len(&self) -> usize {
        self.clip_stack.len()
    }

    /// Drop clips until the stack is `len` deep (no-op if already ≤ `len`).
    pub fn truncate_clip(&mut self, len: usize) {
        self.clip_stack.truncate(len);
    }

    /// Return the active clipping rectangle (in world / screen space).
    pub fn current_clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    // ---- Transform stack ----

    /// Push the current transform onto the stack (the new top is a clone of
    /// the old top, matching Teardown's `UiPush`).
    pub fn push_transform(&mut self) {
        let top = *self.transform_stack.last().unwrap_or(&Affine2::IDENTITY);
        self.transform_stack.push(top);
    }

    /// Pop the top transform. Refuses to pop below 1 entry (the implicit
    /// identity at the base of the stack).
    pub fn pop_transform(&mut self) {
        if self.transform_stack.len() > 1 {
            self.transform_stack.pop();
        }
    }

    /// Return the current (top) transform.
    pub fn current_transform(&self) -> Affine2 {
        *self.transform_stack.last().unwrap_or(&Affine2::IDENTITY)
    }

    /// Post-multiply the current transform by a translation.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.compose_top(&Affine2::translation(dx, dy));
    }

    /// Post-multiply the current transform by a rotation about the local origin.
    pub fn rotate(&mut self, angle_radians: f32) {
        self.compose_top(&Affine2::rotation(angle_radians));
    }

    /// Post-multiply the current transform by a non-uniform scale.
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.compose_top(&Affine2::scale(sx, sy));
    }

    fn compose_top(&mut self, m: &Affine2) {
        if let Some(top) = self.transform_stack.last_mut() {
            *top = top.compose(m);
        }
    }

    // ---- Tint stack ----

    /// Push the current tint onto the stack (clone of top).
    pub fn push_tint(&mut self) {
        let top = *self.tint_stack.last().unwrap_or(&[1.0, 1.0, 1.0, 1.0]);
        self.tint_stack.push(top);
    }

    /// Pop the top tint. Refuses to pop below 1 entry.
    pub fn pop_tint(&mut self) {
        if self.tint_stack.len() > 1 {
            self.tint_stack.pop();
        }
    }

    /// Replace the current tint (Teardown's `UiColor` semantics).
    pub fn set_tint(&mut self, rgba: [f32; 4]) {
        if let Some(top) = self.tint_stack.last_mut() {
            *top = rgba;
        }
    }

    /// Multiply the current tint by `rgba` (Teardown's `UiColorFilter` semantics).
    pub fn multiply_tint(&mut self, rgba: [f32; 4]) {
        if let Some(top) = self.tint_stack.last_mut() {
            top[0] *= rgba[0];
            top[1] *= rgba[1];
            top[2] *= rgba[2];
            top[3] *= rgba[3];
        }
    }

    /// Return the current (top) tint.
    pub fn current_tint(&self) -> [f32; 4] {
        *self.tint_stack.last().unwrap_or(&[1.0, 1.0, 1.0, 1.0])
    }

    /// Combine an input color with the current tint.
    fn apply_tint(&self, color: [f32; 4]) -> [f32; 4] {
        let t = self.current_tint();
        [
            color[0] * t[0],
            color[1] * t[1],
            color[2] * t[2],
            color[3] * t[3],
        ]
    }

    /// Build a colored vertex by transforming local position through the current
    /// affine and multiplying the input color by the current tint.
    fn vertex(&self, x: f32, y: f32, color: [f32; 4]) -> Vertex {
        let world = self.current_transform().transform_point([x, y]);
        let tinted = self.apply_tint(color);
        Vertex::new(world[0], world[1], tinted).with_clip(self.current_clip())
    }

    // ---- Primitives ----

    /// Add a single triangle.
    pub fn triangle(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), color: [f32; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.push(self.vertex(p0.0, p0.1, color));
        self.vertices.push(self.vertex(p1.0, p1.1, color));
        self.vertices.push(self.vertex(p2.0, p2.1, color));
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Add a single triangle with a distinct color per corner, in `p0`, `p1`,
    /// `p2` order — the GPU interpolates linearly across the face. Like
    /// [`Self::quad_gradient`], this reproduces any *linear* color ramp over
    /// the triangle exactly (used for fills whose alpha follows an axis, e.g.
    /// the curve editor's under-curve fade). Always soup geometry.
    pub fn triangle_gradient(
        &mut self,
        p0: (f32, f32),
        p1: (f32, f32),
        p2: (f32, f32),
        c0: [f32; 4],
        c1: [f32; 4],
        c2: [f32; 4],
    ) {
        let base = self.vertices.len() as u32;
        self.vertices.push(self.vertex(p0.0, p0.1, c0));
        self.vertices.push(self.vertex(p1.0, p1.1, c1));
        self.vertices.push(self.vertex(p2.0, p2.1, c2));
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Add a filled rectangle.
    ///
    /// Fast path (translation-only transform): records a single fill-only SDF
    /// chrome instance (radius 0) instead of two soup triangles — so thousands of
    /// rects collapse to small per-instance records the renderer rasterizes,
    /// with no per-frame soup re-tessellation/re-upload. Under any rotation/scale/
    /// shear it falls back to soup geometry (`DrawList::quad_soup`) so the rect
    /// still transforms correctly.
    pub fn quad(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
        if width <= 0.0 || height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        if self.current_transform().is_translate_only() {
            self.fill_rect_instance(Rect::new(x, y, width, height), 0.0, color);
        } else {
            self.quad_soup(x, y, width, height, color);
        }
    }

    /// Tessellate a filled rectangle into the vertex soup (2 triangles, 4
    /// vertices). The fallback path for [`DrawList::quad`] under non-translation
    /// transforms, and the building block for the other soup primitives.
    fn quad_soup(&mut self, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
        let x0 = x;
        let y0 = y;
        let x1 = x + width;
        let y1 = y + height;
        let base = self.vertices.len() as u32;

        self.vertices.push(self.vertex(x0, y0, color));
        self.vertices.push(self.vertex(x1, y0, color));
        self.vertices.push(self.vertex(x1, y1, color));
        self.vertices.push(self.vertex(x0, y1, color));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    /// Add a filled rectangle with a distinct color per corner, in
    /// `[top_left, top_right, bottom_right, bottom_left]` order — the GPU
    /// interpolates linearly across the two triangles, giving a gradient fill
    /// (used by the color picker's SV square / hue / alpha bars).
    ///
    /// Always soup geometry (a gradient can't use the instanced-chrome fast
    /// path), and like `quad_soup` it honors the current
    /// transform + tint via `vertex`. No-op on non-positive size.
    pub fn quad_gradient(&mut self, rect: Rect, colors: [[f32; 4]; 4]) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;
        let base = self.vertices.len() as u32;

        // Same winding as `quad_soup`: TL, TR, BR, BL.
        self.vertices.push(self.vertex(x0, y0, colors[0]));
        self.vertices.push(self.vertex(x1, y0, colors[1]));
        self.vertices.push(self.vertex(x1, y1, colors[2]));
        self.vertices.push(self.vertex(x0, y1, colors[3]));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    /// Fill `rect` with a linear gradient from `start` to `end` along `angle`
    /// (radians; `0` = left→right, `π/2` = top→bottom, increasing clockwise in
    /// screen space where +y points down).
    ///
    /// Exact for any angle: a linear color ramp is an affine function of
    /// position, which the GPU's bilinear corner interpolation reproduces
    /// precisely — so this is just [`quad_gradient`](Self::quad_gradient) with
    /// the four corner colors projected onto the gradient axis. No-op on
    /// non-positive size. For the cardinal directions prefer the cheaper
    /// [`horizontal_gradient`](Self::horizontal_gradient) /
    /// [`vertical_gradient`](Self::vertical_gradient).
    pub fn linear_gradient(&mut self, rect: Rect, start: [f32; 4], end: [f32; 4], angle: f32) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        let (s, c) = angle.sin_cos();
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;
        // Project each corner (TL, TR, BR, BL) onto the gradient direction, then
        // normalize to [0,1] across the projected extent → per-corner colors.
        let proj = [
            x0 * c + y0 * s,
            x1 * c + y0 * s,
            x1 * c + y1 * s,
            x0 * c + y1 * s,
        ];
        let min = proj.iter().copied().fold(f32::INFINITY, f32::min);
        let max = proj.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let span = (max - min).max(f32::EPSILON);
        let colors = proj.map(|p| lerp_color(start, end, (p - min) / span));
        self.quad_gradient(rect, colors);
    }

    /// Fill `rect` with a horizontal gradient (`left` edge → `right` edge).
    pub fn horizontal_gradient(&mut self, rect: Rect, left: [f32; 4], right: [f32; 4]) {
        self.quad_gradient(rect, [left, right, right, left]);
    }

    /// Fill `rect` with a vertical gradient (`top` edge → `bottom` edge).
    pub fn vertical_gradient(&mut self, rect: Rect, top: [f32; 4], bottom: [f32; 4]) {
        self.quad_gradient(rect, [top, top, bottom, bottom]);
    }

    /// Fill `rect` with a radial gradient: `inner` at the center fading to
    /// `outer` toward the edges, as a triangle fan of `segments` wedges (clamped
    /// to ≥ 3). The fan radius reaches the rect's farthest corner so the whole
    /// rect is filled, and the geometry is clipped to `rect`. Honors the current
    /// transform/tint/clip like the other soup primitives. No-op on non-positive
    /// size.
    ///
    /// The fade is circular (equal in x and y), so for a non-square rect the
    /// iso-color rings are circles centered in the rect, not ellipses.
    pub fn radial_gradient(&mut self, rect: Rect, inner: [f32; 4], outer: [f32; 4], segments: u32) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        let segments = segments.max(3);
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        // Circumradius whose inscribed circle (apothem) still reaches the
        // farthest corner, so the N-gon fully covers the rect before clipping.
        let half_diag = (rect.width * rect.width + rect.height * rect.height).sqrt() * 0.5;
        let r = half_diag / (std::f32::consts::PI / segments as f32).cos();

        self.push_clip(rect);
        let base = self.vertices.len() as u32;
        self.vertices.push(self.vertex(cx, cy, inner));
        for i in 0..segments {
            let theta = std::f32::consts::TAU * (i as f32) / (segments as f32);
            let (s, c) = theta.sin_cos();
            self.vertices
                .push(self.vertex(cx + r * c, cy + r * s, outer));
        }
        for i in 0..segments {
            let a = base + 1 + i;
            let b = base + 1 + ((i + 1) % segments);
            self.indices.extend_from_slice(&[base, a, b]);
        }
        self.pop_clip();
    }

    /// Add a thick line segment as a quad.
    pub fn line(&mut self, p0: [f32; 2], p1: [f32; 2], thickness: f32, color: [f32; 4]) {
        let dx = p1[0] - p0[0];
        let dy = p1[1] - p0[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON || thickness <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }

        let half = thickness * 0.5;
        let ox = -dy / len * half;
        let oy = dx / len * half;
        let base = self.vertices.len() as u32;

        // Compute offsets in local space; transform happens inside `vertex()`.
        self.vertices
            .push(self.vertex(p0[0] + ox, p0[1] + oy, color));
        self.vertices
            .push(self.vertex(p1[0] + ox, p1[1] + oy, color));
        self.vertices
            .push(self.vertex(p1[0] - ox, p1[1] - oy, color));
        self.vertices
            .push(self.vertex(p0[0] - ox, p0[1] - oy, color));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
    }

    /// Add connected thick line segments without joins or caps.
    pub fn polyline(&mut self, points: &[[f32; 2]], thickness: f32, color: [f32; 4]) {
        for segment in points.windows(2) {
            self.line(segment[0], segment[1], thickness, color);
        }
    }

    /// Add a rounded rectangle. Geometry is built in local space and
    /// transformed at vertex push time, so a rotated transform produces a
    /// rotated rounded rect.
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
        if radius <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            self.quad(rect.x, rect.y, rect.width, rect.height, color);
            return;
        }

        // Fast path: one fill-only SDF instance (the shader clamps the radius and
        // rasterizes anti-aliased corners) instead of 5 strip quads + 4×8 corner
        // triangles into the soup. Falls back to tessellation under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.fill_rect_instance(rect, radius, color);
            return;
        }

        let radius = radius.min(rect.width * 0.5).min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;

        // Center quad — fully inset rect, untouched by corner arcs.
        self.quad(
            x0 + radius,
            y0 + radius,
            rect.width - radius * 2.0,
            rect.height - radius * 2.0,
            color,
        );
        // Top side strip
        self.quad(x0 + radius, y0, rect.width - radius * 2.0, radius, color);
        // Bottom side strip
        self.quad(
            x0 + radius,
            y1 - radius,
            rect.width - radius * 2.0,
            radius,
            color,
        );
        // Left side strip
        self.quad(x0, y0 + radius, radius, rect.height - radius * 2.0, color);
        // Right side strip
        self.quad(
            x1 - radius,
            y0 + radius,
            radius,
            rect.height - radius * 2.0,
            color,
        );

        self.rounded_corner(
            (x0 + radius, y0 + radius),
            radius,
            std::f32::consts::PI,
            std::f32::consts::PI * 1.5,
            color,
        );
        self.rounded_corner(
            (x1 - radius, y0 + radius),
            radius,
            std::f32::consts::PI * 1.5,
            std::f32::consts::TAU,
            color,
        );
        self.rounded_corner(
            (x1 - radius, y1 - radius),
            radius,
            0.0,
            std::f32::consts::FRAC_PI_2,
            color,
        );
        self.rounded_corner(
            (x0 + radius, y1 - radius),
            radius,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            color,
        );
    }

    fn rounded_corner(
        &mut self,
        center: (f32, f32),
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        color: [f32; 4],
    ) {
        for i in 0..ROUNDED_RECT_CORNER_SEGMENTS {
            let t0 = i as f32 / ROUNDED_RECT_CORNER_SEGMENTS as f32;
            let t1 = (i + 1) as f32 / ROUNDED_RECT_CORNER_SEGMENTS as f32;
            let a0 = start_angle + (end_angle - start_angle) * t0;
            let a1 = start_angle + (end_angle - start_angle) * t1;
            let p0 = (center.0 + a0.cos() * radius, center.1 + a0.sin() * radius);
            let p1 = (center.0 + a1.cos() * radius, center.1 + a1.sin() * radius);
            self.triangle(center, p0, p1, color);
        }
    }

    /// Add a filled convex polygon using fan triangulation from centroid.
    /// Points should be in order (clockwise or counter-clockwise).
    pub fn filled_polygon(&mut self, points: &[(f32, f32)], color: [f32; 4]) {
        if points.len() < 3 {
            return;
        }

        // Calculate centroid
        let mut cx = 0.0;
        let mut cy = 0.0;
        for &(x, y) in points {
            cx += x;
            cy += y;
        }
        cx /= points.len() as f32;
        cy /= points.len() as f32;

        // Fan triangulation: create triangle from centroid to each edge
        for i in 0..points.len() {
            let p0 = points[i];
            let p1 = points[(i + 1) % points.len()];
            self.triangle((cx, cy), p0, p1, color);
        }
    }

    /// Add a rectangle outline of the given `thickness`, drawn flush *inside*
    /// `rect` (the outer edge of the border coincides with `rect`, the border
    /// grows inward). Mirrors Teardown's `UiRectOutline(w, h, thickness)`.
    ///
    /// Built from four edge quads so it transforms (rotation/scale/clip/tint)
    /// exactly like [`DrawList::quad`].
    pub fn rect_outline(&mut self, rect: Rect, thickness: f32, color: [f32; 4]) {
        if thickness <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        // Fast path: one outline-only SDF instance (radius 0, transparent fill)
        // instead of four edge quads. Falls back to soup under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.stroke_rect_instance(rect, 0.0, thickness, color);
            return;
        }
        // Clamp so an over-thick border degenerates to a filled rect instead of
        // overlapping itself / inverting the inner strips.
        let t = thickness.min(rect.width * 0.5).min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;

        // Top and bottom run the full width.
        self.quad(x0, y0, rect.width, t, color);
        self.quad(x0, rect.y + rect.height - t, rect.width, t, color);

        // Left and right fill only the gap between the top/bottom strips so the
        // corners are not double-covered.
        let inner_h = rect.height - 2.0 * t;
        if inner_h > 0.0 {
            self.quad(x0, y0 + t, t, inner_h, color);
            self.quad(x1 - t, y0 + t, t, inner_h, color);
        }
    }

    /// Add a rounded-rectangle outline of the given `thickness`, tracing the
    /// same boundary as [`DrawList::rounded_rect`] (outer edge flush with
    /// `rect`, border grows inward). Mirrors Teardown's
    /// `UiRoundedRectOutline(w, h, radius, thickness)`.
    pub fn rounded_rect_outline(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        if thickness <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        if radius <= 0.0 {
            self.rect_outline(rect, thickness, color);
            return;
        }

        // Fast path: one outline-only SDF instance (rounded, transparent fill)
        // instead of two edge quads + four corner arcs. The shader clamps radius
        // and thickness. Falls back to soup tessellation under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.stroke_rect_instance(rect, radius, thickness, color);
            return;
        }

        let radius = radius.min(rect.width * 0.5).min(rect.height * 0.5);
        let t = thickness
            .min(radius)
            .min(rect.width * 0.5)
            .min(rect.height * 0.5);
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.width;
        let y1 = rect.y + rect.height;

        // Straight edges between the corner tangent points, inset inward by `t`.
        let span_w = rect.width - radius * 2.0;
        let span_h = rect.height - radius * 2.0;
        if span_w > 0.0 {
            self.quad(x0 + radius, y0, span_w, t, color); // top
            self.quad(x0 + radius, y1 - t, span_w, t, color); // bottom
        }
        if span_h > 0.0 {
            self.quad(x0, y0 + radius, t, span_h, color); // left
            self.quad(x1 - t, y0 + radius, t, span_h, color); // right
        }

        // Corner arcs (outer radius = `radius`, inner = radius - t) using the
        // same angular ranges as `rounded_rect` so the stroke follows the fill.
        let inner = (radius - t).max(0.0);
        let seg = ROUNDED_RECT_CORNER_SEGMENTS;
        self.stroked_arc(
            (x0 + radius, y0 + radius),
            inner,
            radius,
            std::f32::consts::PI,
            std::f32::consts::PI * 1.5,
            seg,
            color,
        );
        self.stroked_arc(
            (x1 - radius, y0 + radius),
            inner,
            radius,
            std::f32::consts::PI * 1.5,
            std::f32::consts::TAU,
            seg,
            color,
        );
        self.stroked_arc(
            (x1 - radius, y1 - radius),
            inner,
            radius,
            0.0,
            std::f32::consts::FRAC_PI_2,
            seg,
            color,
        );
        self.stroked_arc(
            (x0 + radius, y1 - radius),
            inner,
            radius,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            seg,
            color,
        );
    }

    /// Draw a rounded-rect chrome panel. This compatibility wrapper records one
    /// full-affine composable quad with uniform radii and border widths.
    pub fn chrome_rect(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        bg: [f32; 4],
        border: [f32; 4],
    ) {
        self.chrome_rect_gradient(rect, radius, thickness, bg, bg, border);
    }

    /// Draw vertically graded rounded chrome. This compatibility wrapper uses
    /// the composable quad instance path.
    pub fn chrome_rect_gradient(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        bg: [f32; 4],
        bg2: [f32; 4],
        border: [f32; 4],
    ) {
        if !self.current_transform().is_translate_only() {
            if radius > 0.0 {
                self.rounded_rect(rect, radius, bg);
                self.vertical_gradient(rect, bg, bg2);
            } else {
                self.quad(rect.x, rect.y, rect.width, rect.height, bg);
                self.vertical_gradient(rect, bg, bg2);
            }
            if thickness > 0.0 {
                self.rounded_rect_outline(rect, radius, thickness, border);
            }
            return;
        }
        self.push_chrome_instance(
            rect,
            Background::LinearGradient {
                start: bg,
                end: bg2,
                axis: GradientAxis::Vertical,
            },
            EdgeWidths::uniform(thickness),
            border,
            CornerRadii::uniform(radius),
        );
    }

    /// Paint a background and border as one fixed-size retained chrome instance.
    pub fn paint_quad(&mut self, rect: Rect, style: QuadStyle) {
        self.push_chrome_instance(
            rect,
            style.background,
            style.border_widths,
            style.border_color,
            style.corner_radii,
        );
    }

    /// Paint only a quad background, retaining its per-corner outer shape.
    pub fn paint_quad_background(
        &mut self,
        rect: Rect,
        background: Background,
        corner_radii: CornerRadii,
    ) {
        self.push_chrome_instance(
            rect,
            background,
            EdgeWidths::default(),
            [0.0; 4],
            corner_radii,
        );
    }

    /// Paint only an inward-growing quad border.
    pub fn paint_quad_border(
        &mut self,
        rect: Rect,
        widths: EdgeWidths,
        color: [f32; 4],
        corner_radii: CornerRadii,
    ) {
        self.push_chrome_instance(
            rect,
            Background::Solid([color[0], color[1], color[2], 0.0]),
            widths,
            color,
            corner_radii,
        );
    }

    /// Paint one structural line flush inside `edge` of `rect`.
    pub fn edge_line(&mut self, rect: Rect, edge: Edge, thickness: f32, color: [f32; 4]) {
        if thickness <= 0.0 || !thickness.is_finite() {
            self.dropped_degenerate += 1;
            return;
        }
        let line = match edge {
            Edge::Top => Rect::new(rect.x, rect.y, rect.width, thickness.min(rect.height)),
            Edge::Right => Rect::new(
                rect.right() - thickness.min(rect.width),
                rect.y,
                thickness.min(rect.width),
                rect.height,
            ),
            Edge::Bottom => Rect::new(
                rect.x,
                rect.bottom() - thickness.min(rect.height),
                rect.width,
                thickness.min(rect.height),
            ),
            Edge::Left => Rect::new(rect.x, rect.y, thickness.min(rect.width), rect.height),
        };
        self.quad(line.x, line.y, line.width, line.height, color);
    }

    fn push_chrome_instance(
        &mut self,
        rect: Rect,
        background: Background,
        widths: EdgeWidths,
        border: [f32; 4],
        radii: CornerRadii,
    ) {
        let m = self.current_transform();
        let mut values = [0.0; 30];
        values[..4].copy_from_slice(&[rect.x, rect.y, rect.width, rect.height]);
        values[4..8].copy_from_slice(&widths.as_array());
        values[8..12].copy_from_slice(&radii.as_array());
        values[12..16].copy_from_slice(&border);
        values[16..22].copy_from_slice(&[m.a, m.b, m.c, m.d, m.tx, m.ty]);
        let (bg, bg2, horizontal) = match background {
            Background::Solid(color) => (color, color, 0.0),
            Background::LinearGradient { start, end, axis } => {
                (start, end, (axis == GradientAxis::Horizontal) as u8 as f32)
            }
        };
        values[22..26].copy_from_slice(&bg);
        values[26..30].copy_from_slice(&bg2);
        if rect.is_empty()
            || values.iter().any(|value| !value.is_finite())
            || m.try_inverse().is_none()
        {
            self.dropped_degenerate += 1;
            return;
        }

        let current_clip = self.current_clip();
        self.flush_soup();
        let (clip, clip_enabled) = current_clip
            .map(|c| ([c.x, c.y, c.width, c.height], 1.0))
            .unwrap_or(([0.0; 4], 0.0));
        let instance = ChromeInstance {
            linear: if m.is_translate_only() {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [m.a, m.b, m.c, m.d]
            },
            translation: if m.is_translate_only() {
                [0.0, 0.0, clip_enabled, horizontal]
            } else {
                [m.tx, m.ty, clip_enabled, horizontal]
            },
            rect: if m.is_translate_only() {
                [rect.x + m.tx, rect.y + m.ty, rect.width, rect.height]
            } else {
                [rect.x, rect.y, rect.width, rect.height]
            },
            bg: self.apply_tint(bg),
            bg2: self.apply_tint(bg2),
            border: self.apply_tint(border),
            radii: normalize_shadow_radii(
                radii.as_array().map(|r| r.max(0.0)),
                rect.width,
                rect.height,
            ),
            widths: widths.as_array().map(|width| width.max(0.0)),
            clip,
            params: [
                radii.top_left.max(0.0),
                widths.top.max(0.0),
                clip_enabled,
                0.0,
            ],
        };
        let idx = self.analytic_instances.len() as u32;
        self.analytic_instances
            .push(AnalyticInstance::chrome(instance));
        self.analytic_chrome_count += 1;
        self.push_paint_cmd(PaintCmd::Analytic {
            instances: idx..idx + 1,
        });
    }

    /// Record a fill-only SDF rect instance. Backs the fast path of rectangle
    /// and rounded-rectangle fills.
    fn fill_rect_instance(&mut self, rect: Rect, radius: f32, color: [f32; 4]) {
        self.push_chrome_instance(
            rect,
            Background::Solid(color),
            EdgeWidths::default(),
            color,
            CornerRadii::uniform(radius),
        );
    }

    /// Record an outline-only SDF rect instance. Backs rectangle outline helpers.
    fn stroke_rect_instance(&mut self, rect: Rect, radius: f32, thickness: f32, color: [f32; 4]) {
        self.paint_quad_border(
            rect,
            EdgeWidths::uniform(thickness),
            color,
            CornerRadii::uniform(radius),
        );
    }

    /// Record one SDF circle instance for a translation-only circle. Same
    /// ordering contract as [`DrawList::push_chrome_instance`] but into the
    /// circle instance buffer / `Circle` runs. `thickness <= 0` is a filled
    /// disc; `> 0` is a ring centered on `radius`. The caller must already have
    /// checked `is_translate_only()`.
    fn push_circle_instance(
        &mut self,
        center: (f32, f32),
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        self.flush_soup();

        let m = self.current_transform();
        let (clip, clip_enabled) = match self.current_clip() {
            Some(c) => ([c.x, c.y, c.width, c.height], 1.0),
            None => ([0.0; 4], 0.0),
        };
        let inst = CircleInstance {
            center: [center.0 + m.tx, center.1 + m.ty, radius, thickness],
            color: self.apply_tint(color),
            clip,
            params: [clip_enabled, 0.0, 0.0, 0.0],
        };
        let idx = self.circle_instances.len() as u32;
        self.circle_instances.push(inst);

        match self.paint_cmds.last_mut() {
            Some(PaintCmd::Circle { instances }) if instances.end == idx => {
                instances.end = idx + 1;
            }
            _ => self.paint_cmds.push(PaintCmd::Circle {
                instances: idx..idx + 1,
            }),
        }
    }

    /// Paint one non-inset shadow around `border_box`.
    pub fn box_shadow_outset(
        &mut self,
        border_box: Rect,
        border_radii: CornerRadii,
        shadow: BoxShadow,
    ) {
        if !shadow.inset {
            self.push_box_shadow(border_box, border_radii, shadow, false);
        }
    }

    /// Paint one inset shadow inside the explicitly supplied padding box.
    pub fn box_shadow_inset(
        &mut self,
        padding_box: Rect,
        padding_radii: CornerRadii,
        shadow: BoxShadow,
    ) {
        if shadow.inset {
            self.push_box_shadow(padding_box, padding_radii, shadow, true);
        }
    }

    /// Paint matching outset declarations in reverse CSS declaration order.
    pub fn box_shadows_outset(
        &mut self,
        border_box: Rect,
        border_radii: CornerRadii,
        declarations: &[BoxShadow],
    ) {
        for shadow in declarations.iter().rev() {
            self.box_shadow_outset(border_box, border_radii, *shadow);
        }
    }

    /// Paint matching inset declarations in reverse CSS declaration order.
    pub fn box_shadows_inset(
        &mut self,
        padding_box: Rect,
        padding_radii: CornerRadii,
        declarations: &[BoxShadow],
    ) {
        for shadow in declarations.iter().rev() {
            self.box_shadow_inset(padding_box, padding_radii, *shadow);
        }
    }

    fn push_box_shadow(
        &mut self,
        element: Rect,
        radii: CornerRadii,
        shadow: BoxShadow,
        inset: bool,
    ) {
        if element.is_empty() || shadow.color[3] <= 0.0 {
            return;
        }
        let transform = self.current_transform();
        let values = [
            element.x,
            element.y,
            element.width,
            element.height,
            radii.top_left,
            radii.top_right,
            radii.bottom_right,
            radii.bottom_left,
            shadow.offset[0],
            shadow.offset[1],
            shadow.blur,
            shadow.spread,
            shadow.color[0],
            shadow.color[1],
            shadow.color[2],
            shadow.color[3],
            transform.a,
            transform.b,
            transform.c,
            transform.d,
            transform.tx,
            transform.ty,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            self.dropped_degenerate += 1;
            return;
        }

        let blur = shadow.blur.max(0.0);
        let sigma = blur * 0.5;
        let delta = if inset { -shadow.spread } else { shadow.spread };
        let mut width = element.width + 2.0 * delta;
        let mut height = element.height + 2.0 * delta;
        let mut x = element.x + shadow.offset[0] - delta;
        let mut y = element.y + shadow.offset[1] - delta;
        let collapsed = width <= 0.0 || height <= 0.0;
        if collapsed && !inset {
            self.dropped_degenerate += 1;
            return;
        }
        if collapsed {
            x += width * 0.5;
            y += height * 0.5;
            width = 0.0;
            height = 0.0;
        }

        let element_radii = normalize_shadow_radii(radii.as_array(), element.width, element.height);
        let adjusted = element_radii.map(|r| css_spread_radius(r, delta));
        let shadow_radii = normalize_shadow_radii(adjusted, width.max(0.0), height.max(0.0));
        let shadow_rect = Rect::new(x, y, width, height);

        // Translation is the dominant widget transform. Its inverse linear part
        // is exactly identity, so avoid determinant/division and five roots per
        // shadow while producing the same instance bytes and raster bounds.
        let (sigma_y, sigma_x_conditional, beta, coverage_pad, blur_pad) =
            if transform.is_translate_only() {
                (sigma, sigma, 0.0, [0.5, 0.5], [3.0 * sigma, 3.0 * sigma])
            } else {
                // CSS blurs after transforming the element, so its Gaussian is
                // isotropic in screen space. Pull sigma² I back through the
                // affine. For A^-1 = B, local covariance is sigma² B Bᵀ. The
                // shader uses its y marginal and x|y conditional.
                let Some(inverse) = transform.try_inverse() else {
                    self.dropped_degenerate += 1;
                    return;
                };
                let row_x_norm = inverse.a.hypot(inverse.b);
                let row_y_norm = inverse.c.hypot(inverse.d);
                let sigma_y = sigma * row_y_norm;
                let covariance_xy = sigma * sigma * (inverse.a * inverse.c + inverse.b * inverse.d);
                let beta = if sigma_y > 1.0e-5 {
                    covariance_xy / (sigma_y * sigma_y)
                } else {
                    0.0
                };
                let sigma_x_conditional = if sigma > 1.0e-5 {
                    let variance_x = sigma * sigma * row_x_norm * row_x_norm;
                    (variance_x - beta * covariance_xy).max(0.0).sqrt()
                } else {
                    0.0
                };
                (
                    sigma_y,
                    sigma_x_conditional,
                    beta,
                    [
                        0.5 * (inverse.a.abs() + inverse.b.abs()),
                        0.5 * (inverse.c.abs() + inverse.d.abs()),
                    ],
                    [3.0 * sigma * row_x_norm, 3.0 * sigma * row_y_norm],
                )
            };

        // A centered one-screen-pixel square pulled through the inverse has
        // local half-extents equal to half its row L1 norms. The blur support
        // adds the 3-sigma ellipse AABB (row L2 norms).
        let raster_base = if inset { element } else { shadow_rect };
        let raster_pad = if inset {
            coverage_pad
        } else {
            [coverage_pad[0] + blur_pad[0], coverage_pad[1] + blur_pad[1]]
        };
        let raster = Rect::new(
            raster_base.x - raster_pad[0],
            raster_base.y - raster_pad[1],
            raster_base.width + 2.0 * raster_pad[0],
            raster_base.height + 2.0 * raster_pad[1],
        );
        let current_clip = self.current_clip();
        if let Some(clip) = current_clip {
            let raster_world = if transform.is_translate_only() {
                Rect::new(
                    raster.x + transform.tx,
                    raster.y + transform.ty,
                    raster.width,
                    raster.height,
                )
            } else {
                transform.transform_rect_aabb(raster)
            };
            if clip.intersection(raster_world).is_none() {
                return;
            }
        }

        self.flush_soup();
        let (clip, clip_enabled) = current_clip
            .map(|c| ([c.x, c.y, c.width, c.height], 1.0))
            .unwrap_or(([0.0; 4], 0.0));
        let instance = ShadowInstance {
            linear: [transform.a, transform.b, transform.c, transform.d],
            translation: [transform.tx, transform.ty, clip_enabled, inset as u8 as f32],
            raster_rect: [raster.x, raster.y, raster.width, raster.height],
            shadow_rect: [
                shadow_rect.x,
                shadow_rect.y,
                shadow_rect.width,
                shadow_rect.height,
            ],
            element_rect: [element.x, element.y, element.width, element.height],
            color: self.apply_tint(shadow.color),
            shadow_radii,
            element_radii,
            clip,
            params: [sigma_y, collapsed as u8 as f32, sigma_x_conditional, beta],
        };
        let idx = self.analytic_instances.len() as u32;
        self.analytic_instances
            .push(AnalyticInstance::shadow(instance));
        self.analytic_shadow_count += 1;
        self.push_paint_cmd(PaintCmd::Analytic {
            instances: idx..idx + 1,
        });
    }

    /// Ordered commands consumed by the renderer and debug report.
    pub(crate) fn paint_commands(&self) -> &[PaintCmd] {
        &self.paint_cmds
    }

    /// Number of chrome records in the heterogeneous analytic stream.
    pub fn chrome_instance_count(&self) -> usize {
        self.analytic_chrome_count
    }

    /// Iterate chrome payloads without maintaining a duplicate production vector.
    pub fn chrome_instances(&self) -> impl Iterator<Item = ChromeInstance> + '_ {
        self.analytic_instances
            .iter()
            .filter_map(AnalyticInstance::as_chrome)
    }

    /// Return the `index`th chrome payload.
    pub fn chrome_instance(&self, index: usize) -> Option<ChromeInstance> {
        self.chrome_instances().nth(index)
    }

    /// Reserved record capacity shared by chrome and shadows.
    pub fn analytic_instance_capacity(&self) -> usize {
        self.analytic_instances.capacity()
    }

    /// Reserve shared chrome/shadow record capacity.
    pub fn reserve_analytic_instances(&mut self, additional: usize) {
        self.analytic_instances.reserve(additional);
    }

    /// Number of shadow records in the heterogeneous analytic stream.
    pub fn shadow_instance_count(&self) -> usize {
        self.analytic_shadow_count
    }

    /// Iterate shadow payloads without maintaining a duplicate production vector.
    pub fn shadow_instances(&self) -> impl Iterator<Item = ShadowInstance> + '_ {
        self.analytic_instances
            .iter()
            .filter_map(AnalyticInstance::as_shadow)
    }

    /// Return the `index`th shadow payload.
    pub fn shadow_instance(&self, index: usize) -> Option<ShadowInstance> {
        self.shadow_instances().nth(index)
    }

    /// Index range for soup not yet represented by an explicit command.
    ///
    /// The renderer submits this once after the command stream. Keeping the
    /// mapping here prevents non-rendering consumers from duplicating the
    /// implicit-tail rule.
    pub(crate) fn trailing_soup_range(&self) -> std::ops::Range<u32> {
        self.soup_committed_indices..self.indices.len() as u32
    }

    fn push_paint_cmd(&mut self, cmd: PaintCmd) {
        match (self.paint_cmds.last_mut(), &cmd) {
            (Some(PaintCmd::Soup { indices: a }), PaintCmd::Soup { indices: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            (Some(PaintCmd::Analytic { instances: a }), PaintCmd::Analytic { instances: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            (Some(PaintCmd::Circle { instances: a }), PaintCmd::Circle { instances: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            (Some(PaintCmd::NineSlice { draws: a }), PaintCmd::NineSlice { draws: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            (Some(PaintCmd::Icon { draws: a }), PaintCmd::Icon { draws: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            #[cfg(feature = "phosphor-icons")]
            (Some(PaintCmd::IconMsdf { draws: a }), PaintCmd::IconMsdf { draws: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            (Some(PaintCmd::Text { draws: a }), PaintCmd::Text { draws: b })
                if a.end == b.start =>
            {
                a.end = b.end
            }
            _ => self.paint_cmds.push(cmd),
        }
    }

    /// Commit soup geometry appended since the last command into a `Soup`
    /// command, so a following command draws after it. No-op if nothing
    /// new was appended.
    fn flush_soup(&mut self) {
        let total = self.indices.len() as u32;
        if total > self.soup_committed_indices {
            self.push_paint_cmd(PaintCmd::Soup {
                indices: self.soup_committed_indices..total,
            });
            self.soup_committed_indices = total;
        }
    }

    /// Emit a thick arc band between `inner` and `outer` radius from
    /// `start_angle` to `end_angle` as a strip of `segments` quads (two
    /// triangles each).
    #[allow(clippy::too_many_arguments)]
    fn stroked_arc(
        &mut self,
        center: (f32, f32),
        inner: f32,
        outer: f32,
        start_angle: f32,
        end_angle: f32,
        segments: usize,
        color: [f32; 4],
    ) {
        for i in 0..segments {
            let t0 = i as f32 / segments as f32;
            let t1 = (i + 1) as f32 / segments as f32;
            let a0 = start_angle + (end_angle - start_angle) * t0;
            let a1 = start_angle + (end_angle - start_angle) * t1;
            let (c0, s0) = (a0.cos(), a0.sin());
            let (c1, s1) = (a1.cos(), a1.sin());
            let i0 = (center.0 + c0 * inner, center.1 + s0 * inner);
            let o0 = (center.0 + c0 * outer, center.1 + s0 * outer);
            let i1 = (center.0 + c1 * inner, center.1 + s1 * inner);
            let o1 = (center.0 + c1 * outer, center.1 + s1 * outer);
            self.triangle(i0, o0, o1, color);
            self.triangle(i0, o1, i1, color);
        }
    }

    /// Number of segments to approximate a circle of the given radius — enough
    /// for the curve to read as smooth without exploding vertex counts.
    fn circle_segments(radius: f32) -> usize {
        ((radius * 0.5).ceil() as usize).clamp(16, 64)
    }

    /// Add a filled circle, centered at `center`. Mirrors Teardown's
    /// `UiCircle(radius)`. Built as a triangle fan so it transforms like the
    /// other primitives.
    pub fn circle(&mut self, center: (f32, f32), radius: f32, color: [f32; 4]) {
        if radius <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        // Fast path: one SDF disc instance (smooth at any radius) instead of a
        // 16-64-segment fan. Falls back to the fan under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.push_circle_instance(center, radius, 0.0, color);
            return;
        }
        let segs = Self::circle_segments(radius);
        for i in 0..segs {
            let a0 = std::f32::consts::TAU * i as f32 / segs as f32;
            let a1 = std::f32::consts::TAU * (i + 1) as f32 / segs as f32;
            let p0 = (center.0 + a0.cos() * radius, center.1 + a0.sin() * radius);
            let p1 = (center.0 + a1.cos() * radius, center.1 + a1.sin() * radius);
            self.triangle(center, p0, p1, color);
        }
    }

    /// Add a circle outline of the given `thickness`, centered on the path at
    /// `radius` (the band spans `radius ± thickness/2`). Mirrors Teardown's
    /// `UiCircleOutline(radius, thickness)`.
    pub fn circle_outline(
        &mut self,
        center: (f32, f32),
        radius: f32,
        thickness: f32,
        color: [f32; 4],
    ) {
        if radius <= 0.0 || thickness <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        // Fast path: one SDF ring instance instead of a stroked-arc band.
        // Falls back to the band tessellation under rotation/scale.
        if self.current_transform().is_translate_only() {
            self.push_circle_instance(center, radius, thickness, color);
            return;
        }
        let half = thickness * 0.5;
        let inner = (radius - half).max(0.0);
        let outer = radius + half;
        let segs = Self::circle_segments(outer);
        self.stroked_arc(
            center,
            inner,
            outer,
            0.0,
            std::f32::consts::TAU,
            segs,
            color,
        );
    }

    /// Add text. The block's origin is transformed through the current
    /// affine; uniform scale (the geometric mean of the X and Y axis basis
    /// lengths, i.e. `sqrt(|det|)`) is applied to font_size, line_height and
    /// max_width. Under non-uniform scale this picks the "average" zoom so a
    /// 2x-by-1x stretch becomes ~1.41x text rather than picking only one axis.
    /// Rotation/shear is **not supported** by the text pipeline (glyphs are
    /// emitted as axis-aligned MSDF quads) — when the transform has any
    /// rotation we log a one-shot warning and render axis-aligned.
    pub fn text(&mut self, mut block: TextBlock) {
        let m = self.current_transform();
        if !m.is_axis_aligned() && !self.text_rotation_warned {
            log::warn!(
                "wgpu-gameui: TextBlock pushed under a rotated/sheared transform — \
                 text will render axis-aligned (MSDF text pipeline limitation)"
            );
            self.text_rotation_warned = true;
        }

        // When span mode is active, derive the display content from the
        // concatenated span texts so the shape cache and cursor-position calls
        // below all operate on the same string.
        if !block.spans.is_empty() {
            block.content = block.spans.iter().map(|s| s.text.as_str()).collect();
        }

        // Apply tint to the block colour and to per-span colour/underline
        // overrides. Tint is colour-only, so we do it before the position
        // transform (order w.r.t. position doesn't matter here, but doing it
        // early lets the underline quads below use the already-tinted colour).
        let tint = self.current_tint();
        if tint != [1.0, 1.0, 1.0, 1.0] {
            // cosmic_text::Color is RGBA8; multiply per-channel via the public accessors.
            let r = block.color.r() as f32 / 255.0;
            let g = block.color.g() as f32 / 255.0;
            let b = block.color.b() as f32 / 255.0;
            let a = block.color.a() as f32 / 255.0;
            let nr = (r * tint[0]).clamp(0.0, 1.0);
            let ng = (g * tint[1]).clamp(0.0, 1.0);
            let nb = (b * tint[2]).clamp(0.0, 1.0);
            let na = (a * tint[3]).clamp(0.0, 1.0);
            block.color = cosmic_text::Color::rgba(
                (nr * 255.0).round() as u8,
                (ng * 255.0).round() as u8,
                (nb * 255.0).round() as u8,
                (na * 255.0).round() as u8,
            );
            // Tint per-span/range colour and underline overrides with the same factor.
            for span in &mut block.spans {
                if let Some(c) = &mut span.color {
                    c[0] = (c[0] * tint[0]).clamp(0.0, 1.0);
                    c[1] = (c[1] * tint[1]).clamp(0.0, 1.0);
                    c[2] = (c[2] * tint[2]).clamp(0.0, 1.0);
                    c[3] = (c[3] * tint[3]).clamp(0.0, 1.0);
                }
                // Only an explicit underline colour needs tinting here; an
                // inheriting underline reads the already-tinted span/block colour
                // at emission time.
                if let Underline::Color(c) = &mut span.underline {
                    c[0] = (c[0] * tint[0]).clamp(0.0, 1.0);
                    c[1] = (c[1] * tint[1]).clamp(0.0, 1.0);
                    c[2] = (c[2] * tint[2]).clamp(0.0, 1.0);
                    c[3] = (c[3] * tint[3]).clamp(0.0, 1.0);
                }
            }
            // Range styles are shared by retained editors; preserve that Arc and
            // defer tint multiplication to glyph placement instead of cloning it.
            block.style_range_tint = tint;
        }

        // Emit underline rects for spans that have `underline` set, BEFORE
        // transforming block.x/block.y. We use the original (pre-transform,
        // pre-scale) font_size and position, so that `self.quad()` can apply
        // the active transform uniformly — matching exactly what the text
        // pipeline does. Soup geometry draws before text glyphs, so the
        // underlines naturally appear beneath the MSDF rendering.
        if block
            .spans
            .iter()
            .any(|s| !matches!(s.underline, Underline::None))
        {
            let positions =
                self.text_cursor_positions(&block.content, block.font_size, Some(block.max_width));
            // Sit the underline just below the baseline so it clears the letter
            // bottoms. `baseline_ratio` (~1.0 of the em) locates the baseline
            // below the block top; the old flat `0.9` sat *above* it, cutting
            // through the glyph bottoms. The small extra gap drops it into the
            // descender zone, font-metric-relative so it scales with any face.
            let vm = self.font_vmetrics(block.font.as_ref());
            let underline_y = block.y + block.font_size * (vm.baseline_ratio + 0.12);
            let thickness = (block.font_size * 0.07).max(1.0);
            // The block colour (already tinted above), as the fallback for an
            // inheriting underline on a span with no colour of its own.
            let block_rgba = [
                block.color.r() as f32 / 255.0,
                block.color.g() as f32 / 255.0,
                block.color.b() as f32 / 255.0,
                block.color.a() as f32 / 255.0,
            ];
            let mut span_byte = 0usize;
            for span in &block.spans {
                // Inherit → the span's text colour (or the block colour); Color →
                // the explicit (tinted) override; None → no underline.
                let ul_color = match span.underline {
                    Underline::None => None,
                    Underline::Inherit => Some(span.color.unwrap_or(block_rgba)),
                    Underline::Color(c) => Some(c),
                };
                if let Some(ul_color) = ul_color {
                    let x_start = span_cursor_x(&positions, span_byte);
                    let end_byte = span_byte + span.text.len();
                    let x_end = span_cursor_x(&positions, end_byte);
                    if x_end > x_start {
                        self.quad(
                            block.x + x_start,
                            underline_y,
                            x_end - x_start,
                            thickness,
                            ul_color,
                        );
                    }
                }
                span_byte += span.text.len();
            }
        }

        // Transform origin.
        let origin = m.transform_point([block.x, block.y]);
        block.x = origin[0];
        block.y = origin[1];

        // Apply uniform-ish scale: geometric mean of the two basis lengths,
        // which equals sqrt(|det|). This handles non-uniform axis-aligned
        // scale gracefully (picks the average zoom instead of dropping a
        // dimension).
        let scale = m.uniform_scale();
        if scale > 0.0 && (scale - 1.0).abs() > 1e-6 {
            block.font_size *= scale;
            block.line_height *= scale;
            block.max_width *= scale;
        }

        if let Some(clip) = self.current_clip() {
            let natural_bounds = Rect::new(block.x, block.y, block.max_width, 2000.0);
            let text_bounds = block.clip.unwrap_or(natural_bounds);
            block.clip = text_bounds
                .intersection(clip)
                .or_else(|| Some(Rect::new(clip.x, clip.y, 0.0, 0.0)));
        }
        self.flush_soup();
        let start = self.texts.len() as u32;
        self.texts.push(block);
        self.push_paint_cmd(PaintCmd::Text {
            draws: start..start + 1,
        });
    }

    /// Add a vector icon from any registered icon font, fit-centered into `rect`
    /// and rendered crisp at any size through the MSDF icon atlas. `tint`
    /// multiplies the fill (use `[1.0; 4]` for the icon's natural color). Honors
    /// the current transform, tint stack, and clip. No-op for a zero-area rect.
    ///
    /// Resolve the [`IconGlyph`] once at startup (see
    /// [`icon_glyph`](crate::render::icon_glyph)) and keep it — it is `Copy`.
    #[cfg(feature = "phosphor-icons")]
    pub fn icon_msdf(&mut self, rect: Rect, glyph: IconGlyph, tint: [f32; 4]) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            self.dropped_degenerate += 1;
            return;
        }
        self.flush_soup();
        let start = self.icons_msdf.len() as u32;
        self.icons_msdf.push(IconMsdf {
            local: rect,
            transform: self.current_transform(),
            glyph,
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
        });
        self.push_paint_cmd(PaintCmd::IconMsdf {
            draws: start..start + 1,
        });
    }

    /// [`icon_msdf`](Self::icon_msdf) for the built-in Phosphor set — resolves
    /// the enum to its glyph at push time. No-op if the glyph is unresolvable
    /// (which the library's tests rule out for the curated set).
    #[cfg(feature = "phosphor-icons")]
    pub fn phosphor_icon(&mut self, rect: Rect, icon: PhosphorIcon, tint: [f32; 4]) {
        if let Some(glyph) = icon.glyph() {
            self.icon_msdf(rect, glyph, tint);
        }
    }

    /// Add a textured icon by name. The renderer will resolve `icon_key` against
    /// its `SpriteAtlas` at render time.
    pub fn icon(&mut self, icon_key: &str, x: f32, y: f32, width: f32, height: f32) {
        let corners = self
            .current_transform()
            .transform_rect_corners(Rect::new(x, y, width, height));
        self.flush_soup();
        let start = self.icons.len() as u32;
        self.icons.push(IconDraw {
            corners,
            sprite: None,
            icon_key: icon_key.to_string(),
            tint: self.current_tint(),
            clip: self.current_clip(),
            src: None,
            wrap: false,
        });
        self.push_paint_cmd(PaintCmd::Icon {
            draws: start..start + 1,
        });
    }

    /// Add a textured icon by pre-resolved sprite handle, with optional tint.
    /// Cheaper than [`DrawList::icon`] — no per-frame name lookup.
    pub fn icon_sprite(
        &mut self,
        sprite: SpriteId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        tint: [f32; 4],
    ) {
        let corners = self
            .current_transform()
            .transform_rect_corners(Rect::new(x, y, width, height));
        self.flush_soup();
        let start = self.icons.len() as u32;
        self.icons.push(IconDraw {
            corners,
            sprite: Some(sprite),
            icon_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
            src: None,
            wrap: false,
        });
        self.push_paint_cmd(PaintCmd::Icon {
            draws: start..start + 1,
        });
    }

    /// Draw a loaded image sprite filling `dest`, tinted by `tint`. Equivalent
    /// to [`DrawList::icon_sprite`] with a `Rect` destination. Backs Teardown's
    /// `UiImage(path)` / `UiFillImage`.
    pub fn image(&mut self, sprite: SpriteId, dest: Rect, tint: [f32; 4]) {
        self.push_image(sprite, dest, None, tint);
    }

    /// Draw a cropped region of a loaded image sprite into `dest`. `src_uv` is a
    /// normalized `[u0, v0, u1, v1]` sub-rect (0..1) within the sprite. Backs
    /// Teardown's `UiImage(path, x0, y0, x1, y1)`.
    pub fn image_cropped(
        &mut self,
        sprite: SpriteId,
        dest: Rect,
        src_uv: [f32; 4],
        tint: [f32; 4],
    ) {
        self.push_image(sprite, dest, Some(src_uv), tint);
    }

    /// Draw a loaded image sprite **tiled** across `dest` at its natural size:
    /// the source repeats edge-to-edge (u1/v1 of `src_uv` are the tile count
    /// along each axis, may exceed 1) and a partial tile at the right/bottom
    /// edge is cropped to the draw. Repetition happens in the fragment shader
    /// (region-relative `fract`), so one instance covers any destination — a
    /// fullscreen backdrop costs one instance, not one per tile.
    pub fn image_tiled(
        &mut self,
        sprite: SpriteId,
        dest: Rect,
        tile_span_uv: [f32; 4],
        tint: [f32; 4],
    ) {
        let corners = self.current_transform().transform_rect_corners(dest);
        self.flush_soup();
        let start = self.icons.len() as u32;
        self.icons.push(IconDraw {
            corners,
            sprite: Some(sprite),
            icon_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
            src: Some(tile_span_uv),
            wrap: true,
        });
        self.push_paint_cmd(PaintCmd::Icon {
            draws: start..start + 1,
        });
    }

    fn push_image(&mut self, sprite: SpriteId, dest: Rect, src: Option<[f32; 4]>, tint: [f32; 4]) {
        let corners = self.current_transform().transform_rect_corners(dest);
        self.flush_soup();
        let start = self.icons.len() as u32;
        self.icons.push(IconDraw {
            corners,
            sprite: Some(sprite),
            icon_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
            src,
            wrap: false,
        });
        self.push_paint_cmd(PaintCmd::Icon {
            draws: start..start + 1,
        });
    }

    /// Add a nine-slice textured panel by name.
    pub fn nine_slice(&mut self, x: f32, y: f32, width: f32, height: f32, texture_key: &str) {
        self.flush_soup();
        let start = self.nine_slices.len() as u32;
        self.nine_slices.push(NineSliceDraw {
            local: Rect::new(x, y, width, height),
            transform: self.current_transform(),
            nine_slice: None,
            texture_key: texture_key.to_string(),
            tint: self.current_tint(),
            clip: self.current_clip(),
        });
        self.push_paint_cmd(PaintCmd::NineSlice {
            draws: start..start + 1,
        });
    }

    /// Add a nine-slice panel by pre-resolved handle.
    pub fn nine_slice_id(
        &mut self,
        id: NineSliceId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        tint: [f32; 4],
    ) {
        self.flush_soup();
        let start = self.nine_slices.len() as u32;
        self.nine_slices.push(NineSliceDraw {
            local: Rect::new(x, y, width, height),
            transform: self.current_transform(),
            nine_slice: Some(id),
            texture_key: String::new(),
            tint: self.apply_tint(tint),
            clip: self.current_clip(),
        });
        self.push_paint_cmd(PaintCmd::NineSlice {
            draws: start..start + 1,
        });
    }
}

/// Return the x-pixel offset of the cursor at `byte_pos` in the positions
/// table returned by [`DrawList::text_cursor_positions`]. Falls back to `0.0`
/// if the byte position is not present (shouldn't happen for well-formed span
/// data, but the function is cheap enough to not warrant a panic).
fn span_cursor_x(positions: &[(usize, f32)], byte_pos: usize) -> f32 {
    positions
        .iter()
        .find(|(b, _)| *b == byte_pos)
        .map(|(_, x)| *x)
        .unwrap_or(0.0)
}

/// Component-wise linear interpolation between two RGBA colors at `t ∈ [0,1]`.
fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

fn css_spread_radius(radius: f32, spread: f32) -> f32 {
    let radius = radius.max(0.0);
    if spread > 0.0 && radius < spread {
        let ratio = radius / spread - 1.0;
        (radius + spread * (1.0 + ratio * ratio * ratio)).max(0.0)
    } else {
        (radius + spread).max(0.0)
    }
}

fn normalize_shadow_radii(mut radii: [f32; 4], width: f32, height: f32) -> [f32; 4] {
    for radius in &mut radii {
        *radius = radius.max(0.0);
    }
    let mut factor: f32 = 1.0;
    for (extent, sum) in [
        (width, radii[0] + radii[1]),
        (width, radii[3] + radii[2]),
        (height, radii[0] + radii[3]),
        (height, radii[1] + radii[2]),
    ] {
        if sum > 0.0 {
            factor = factor.min((extent.max(0.0) / sum).min(1.0));
        }
    }
    radii.map(|radius| radius * factor)
}

#[cfg(test)]
mod tests {
    use crate::affine::Affine2;
    use crate::chrome::{Background, Edge, EdgeWidths, GradientAxis, QuadStyle};
    use crate::layout::Rect;
    use crate::shadow::{BoxShadow, CornerRadii};

    use super::{DrawList, PaintCmd, PrimCounts, css_spread_radius, normalize_shadow_radii};

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn rounded_rect_emits_chrome_instance() {
        // Translate-only rounded fills record one SDF chrome instance (radius in
        // params[0], thickness 0 = fill), not tessellated soup.
        let mut list = DrawList::new();
        list.rounded_rect(Rect::new(0.0, 0.0, 100.0, 40.0), 6.0, [1.0, 1.0, 1.0, 1.0]);

        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instance_count(), 1);
        let inst = list.chrome_instance(0).unwrap();
        assert_eq!(inst.rect, [0.0, 0.0, 100.0, 40.0]);
        assert!(approx(inst.params[0], 6.0)); // radius
        assert!(approx(inst.params[1], 0.0)); // thickness (fill)
    }

    #[test]
    fn rect_outline_emits_stroke_instance() {
        let mut list = DrawList::new();
        list.rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 2.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instance_count(), 1);
        let inst = list.chrome_instance(0).unwrap();
        assert!(approx(inst.params[0], 0.0)); // radius (square corners)
        assert!(approx(inst.params[1], 2.0)); // thickness
        assert!(approx(inst.bg[3], 0.0)); // transparent fill: only the border band draws
    }

    #[test]
    fn rect_outline_degenerates_to_two_quads_when_thick() {
        // Under rotation the outline falls back to the soup tessellator, where a
        // thickness >= half height collapses the inner strip to top+bottom only.
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        list.rect_outline(Rect::new(0.0, 0.0, 100.0, 10.0), 50.0, [1.0; 4]);
        assert!(list.chrome_instance_count() == 0);
        assert_eq!(list.vertices.len(), 8); // two quads
    }

    #[test]
    fn rect_outline_zero_thickness_draws_nothing() {
        let mut list = DrawList::new();
        list.rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 0.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert!(list.chrome_instance_count() == 0);
    }

    #[test]
    fn rounded_rect_outline_emits_stroke_instance() {
        let mut list = DrawList::new();
        list.rounded_rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 8.0, 2.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.chrome_instance_count(), 1);
        let inst = list.chrome_instance(0).unwrap();
        assert!(approx(inst.params[0], 8.0)); // radius
        assert!(approx(inst.params[1], 2.0)); // thickness
        assert!(approx(inst.bg[3], 0.0)); // transparent fill
    }

    #[test]
    fn rounded_rect_outline_zero_radius_falls_back_to_rect_outline() {
        let mut rounded = DrawList::new();
        rounded.rounded_rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 0.0, 2.0, [1.0; 4]);
        let mut plain = DrawList::new();
        plain.rect_outline(Rect::new(0.0, 0.0, 100.0, 40.0), 2.0, [1.0; 4]);
        // Both produce an identical square-cornered stroke instance.
        assert_eq!(rounded.chrome_instance_count(), 1);
        assert_eq!(rounded.analytic_instances, plain.analytic_instances);
    }

    #[test]
    fn circle_emits_instance() {
        // Translate-only circle fills record one SDF circle instance.
        let mut list = DrawList::new();
        list.circle((50.0, 50.0), 20.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(list.circle_instances.len(), 1);
        let inst = list.circle_instances[0];
        assert_eq!(inst.center, [50.0, 50.0, 20.0, 0.0]); // cx, cy, radius, thickness(fill)
    }

    #[test]
    fn circle_emits_fan_within_radius_under_rotation() {
        // Under rotation the circle falls back to the soup fan. Centered at the
        // origin (rotation's fixed point) so the within-radius check still holds.
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        let r = 20.0;
        list.circle((0.0, 0.0), r, [1.0; 4]);
        assert!(list.circle_instances.is_empty());
        assert!(!list.vertices.is_empty());
        for v in &list.vertices {
            let d = (v.position[0] * v.position[0] + v.position[1] * v.position[1]).sqrt();
            assert!(d <= r + 1e-3);
        }
    }

    #[test]
    fn circle_outline_band_spans_radius_under_rotation() {
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        let (r, t) = (20.0, 4.0);
        list.circle_outline((0.0, 0.0), r, t, [1.0; 4]);
        // Vertices sit on the inner or outer ring: distance in [r - t/2, r + t/2].
        assert!(list.circle_instances.is_empty());
        let lo = r - t * 0.5 - 1e-3;
        let hi = r + t * 0.5 + 1e-3;
        for v in &list.vertices {
            let d = (v.position[0] * v.position[0] + v.position[1] * v.position[1]).sqrt();
            assert!(
                d >= lo && d <= hi,
                "vertex dist {d} outside band [{lo},{hi}]"
            );
        }
    }

    #[test]
    fn circle_outline_emits_instance() {
        let mut list = DrawList::new();
        list.circle_outline((0.0, 0.0), 20.0, 4.0, [1.0; 4]);
        assert_eq!(list.circle_instances.len(), 1);
        let inst = list.circle_instances[0];
        assert_eq!(inst.center, [0.0, 0.0, 20.0, 4.0]); // thickness carries the band width
    }

    #[test]
    fn circle_zero_radius_draws_nothing() {
        let mut list = DrawList::new();
        list.circle((0.0, 0.0), 0.0, [1.0; 4]);
        list.circle_outline((0.0, 0.0), 0.0, 2.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert!(list.circle_instances.is_empty());
    }

    #[test]
    fn outline_primitives_respect_transform() {
        // Translate-only outlines bake the translated rect into the instance.
        let mut list = DrawList::new();
        list.translate(100.0, 50.0);
        list.rect_outline(Rect::new(0.0, 0.0, 10.0, 10.0), 2.0, [1.0; 4]);
        assert_eq!(
            list.chrome_instance(0).unwrap().rect,
            [100.0, 50.0, 10.0, 10.0]
        );
    }

    #[test]
    fn line_emits_quad_geometry() {
        let mut list = DrawList::new();
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0, 1.0, 1.0, 1.0]);

        assert_eq!(list.vertices.len(), 4);
        assert_eq!(list.indices.len(), 6);
    }

    #[test]
    fn icon_helper_pushes_one_command() {
        let mut list = DrawList::new();
        list.icon("foo", 1.0, 2.0, 16.0, 16.0);
        assert_eq!(list.icons.len(), 1);
        assert_eq!(list.icons[0].icon_key, "foo");
        assert_eq!(list.icons[0].sprite, None);
        assert_eq!(list.icons[0].tint, [1.0, 1.0, 1.0, 1.0]);
        // TL and BR corners under identity match input rect.
        assert_eq!(list.icons[0].corners[0], [1.0, 2.0]);
        assert_eq!(list.icons[0].corners[2], [17.0, 18.0]);
    }

    #[test]
    fn icon_sprite_helper_resolves_id_and_tint() {
        let mut list = DrawList::new();
        list.icon_sprite(7, 0.0, 0.0, 24.0, 24.0, [0.5, 0.6, 0.7, 1.0]);
        assert_eq!(list.icons.len(), 1);
        assert_eq!(list.icons[0].sprite, Some(7));
        assert_eq!(list.icons[0].tint, [0.5, 0.6, 0.7, 1.0]);
        assert!(list.icons[0].icon_key.is_empty());
        assert_eq!(list.icons[0].src, None);
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn icon_msdf_records_glyph_tint_transform_and_clip() {
        use crate::render::PhosphorIcon;
        let mut list = DrawList::new();
        list.push_transform();
        list.translate(40.0, 60.0);
        list.set_tint([1.0, 1.0, 1.0, 0.5]);
        list.phosphor_icon(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            PhosphorIcon::Plus,
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(list.icons_msdf.len(), 1);
        let rec = list.icons_msdf[0];
        // Glyph resolved to a real (non-notdef) id in the Phosphor font.
        assert_eq!(rec.glyph.font, crate::render::IconFontId::PHOSPHOR);
        assert_ne!(rec.glyph.glyph_id, 0);
        // Tint is multiplied by the active tint stack (alpha 1.0 * 0.5).
        assert_eq!(rec.tint, [1.0, 0.0, 0.0, 0.5]);
        // The translate transform is carried (origin maps to (40, 60)).
        let o = rec.transform.transform_point([0.0, 0.0]);
        assert!((o[0] - 40.0).abs() < 1e-4 && (o[1] - 60.0).abs() < 1e-4);
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn icon_msdf_skips_zero_rect() {
        use crate::render::PhosphorIcon;
        let mut list = DrawList::new();
        list.phosphor_icon(Rect::new(0.0, 0.0, 0.0, 20.0), PhosphorIcon::X, [1.0; 4]);
        assert!(list.icons_msdf.is_empty());
    }

    /// The generic verb and the Phosphor convenience wrapper must produce the
    /// same record — the wrapper is a resolution shortcut, not a second path.
    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn phosphor_icon_matches_the_generic_verb() {
        use crate::render::PhosphorIcon;
        let glyph = PhosphorIcon::Gear.glyph().expect("gear resolves");
        let rect = Rect::new(3.0, 4.0, 18.0, 18.0);

        let mut a = DrawList::new();
        a.phosphor_icon(rect, PhosphorIcon::Gear, [1.0; 4]);
        let mut b = DrawList::new();
        b.icon_msdf(rect, glyph, [1.0; 4]);
        assert_eq!(a.icons_msdf, b.icons_msdf);
    }

    #[test]
    fn image_and_image_cropped_set_src() {
        let mut list = DrawList::new();
        list.image(3, Rect::new(0.0, 0.0, 32.0, 32.0), [1.0, 1.0, 1.0, 1.0]);
        list.image_cropped(
            4,
            Rect::new(0.0, 0.0, 16.0, 16.0),
            [0.0, 0.0, 0.5, 0.5],
            [1.0, 1.0, 1.0, 1.0],
        );
        assert_eq!(list.icons.len(), 2);
        assert_eq!(list.icons[0].sprite, Some(3));
        assert_eq!(list.icons[0].src, None);
        assert_eq!(list.icons[1].sprite, Some(4));
        assert_eq!(list.icons[1].src, Some([0.0, 0.0, 0.5, 0.5]));
    }

    #[test]
    fn clip_stack_marks_emitted_commands() {
        let mut list = DrawList::new();
        let clip = Rect::new(10.0, 20.0, 30.0, 40.0);

        list.push_clip(clip);
        // Translate-only quads record chrome instances carrying the active clip.
        list.quad(0.0, 0.0, 100.0, 100.0, [1.0, 1.0, 1.0, 1.0]);
        list.text(crate::text::TextBlock::new("clipped", 0.0, 0.0));
        list.icon("icon", 0.0, 0.0, 10.0, 10.0);
        list.pop_clip();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0, 1.0, 1.0, 1.0]);

        assert_eq!(list.chrome_instance(0).unwrap().params[2], 1.0); // clip_enabled
        assert_eq!(
            list.chrome_instance(0).unwrap().clip,
            [10.0, 20.0, 30.0, 40.0]
        );
        assert_eq!(list.texts[0].clip, Some(Rect::new(10.0, 20.0, 30.0, 40.0)));
        assert_eq!(list.icons[0].clip, Some(clip));
        assert_eq!(list.icons[0].icon_key, "icon");
        assert_eq!(list.icons[0].tint, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(list.chrome_instance(1).unwrap().params[2], 0.0); // clip disabled
    }

    #[test]
    fn push_clip_intersects_but_exact_replaces() {
        let mut list = DrawList::new();
        // Parent clip.
        list.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
        // Intersecting child: a larger rect is clipped down to the parent.
        list.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(list.current_clip(), Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
        list.pop_clip();
        // Exact child: replaces the parent, even when larger.
        list.push_clip_exact(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(list.current_clip(), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn clip_len_and_truncate_scope() {
        let mut list = DrawList::new();
        assert_eq!(list.clip_len(), 0);
        let base = list.clip_len();
        list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
        list.push_clip_exact(Rect::new(0.0, 0.0, 5.0, 5.0));
        assert_eq!(list.clip_len(), 2);
        list.truncate_clip(base);
        assert_eq!(list.clip_len(), 0);
        assert_eq!(list.current_clip(), None);
    }

    #[test]
    fn byte_range_styles_keep_original_content_and_shared_storage() {
        let ranges = std::sync::Arc::new(vec![crate::TextStyleRange {
            range: 0..5,
            color: Some([1.0, 0.0, 0.0, 1.0]),
            underline: crate::Underline::None,
        }]);
        let mut list = DrawList::new();
        list.text(
            crate::TextBlock::new("local café", 0.0, 0.0).with_shared_style_ranges(ranges.clone()),
        );
        assert_eq!(list.texts[0].content, "local café");
        assert!(std::sync::Arc::ptr_eq(&list.texts[0].style_ranges, &ranges));
    }

    #[test]
    fn underline_inherit_uses_span_then_block_colour() {
        use crate::text::{TextBlock, TextSpan, Underline};

        // Three underlined spans: Inherit-with-span-colour, Inherit-without
        // (falls back to block colour), and an explicit Colour override. Each
        // underline lands as a thin translate-only quad in chrome_instances,
        // carrying its resolved colour in `bg`.
        let span_red = [1.0, 0.0, 0.0, 1.0];
        let override_yellow = [1.0, 0.9, 0.0, 1.0];
        let mut list = DrawList::with_font_system(crate::shared_font_system());
        list.text(
            TextBlock::new("", 18.0, 10.0)
                .with_size(24.0)
                .with_color(0, 200, 0) // block green
                .with_spans(vec![
                    TextSpan {
                        text: "red ".into(),
                        color: Some(span_red),
                        underline: Underline::Inherit,
                    },
                    TextSpan {
                        text: "green ".into(),
                        color: None,
                        underline: Underline::Inherit,
                    },
                    TextSpan {
                        text: "yellow".into(),
                        color: None,
                        underline: Underline::Color(override_yellow),
                    },
                ]),
        );

        let block_green = [0.0, 200.0 / 255.0, 0.0, 1.0];
        let has_colour = |want: [f32; 4]| {
            list.chrome_instances().any(|c| {
                c.bg.iter()
                    .zip(want.iter())
                    .all(|(a, b)| (a - b).abs() < 1e-3)
            })
        };
        assert!(
            has_colour(span_red),
            "inherit underline should use span colour"
        );
        assert!(
            has_colour(block_green),
            "inherit underline w/o span colour should fall back to block colour"
        );
        assert!(
            has_colour(override_yellow),
            "explicit Colour underline should use the override"
        );
    }

    // ---- Transform/tint stack tests ----

    #[test]
    fn quad_under_translate() {
        // Translate-only quads bake the translated rect into a chrome instance.
        let mut list = DrawList::new();
        list.translate(100.0, 50.0);
        list.quad(0.0, 0.0, 10.0, 20.0, [1.0; 4]);
        assert!(list.vertices.is_empty());
        assert_eq!(
            list.chrome_instance(0).unwrap().rect,
            [100.0, 50.0, 10.0, 20.0]
        );
    }

    #[test]
    fn quad_gradient_assigns_corner_colors() {
        let mut list = DrawList::new();
        let tl = [1.0, 0.0, 0.0, 1.0];
        let tr = [0.0, 1.0, 0.0, 1.0];
        let br = [0.0, 0.0, 1.0, 1.0];
        let bl = [1.0, 1.0, 0.0, 1.0];
        list.quad_gradient(Rect::new(0.0, 0.0, 10.0, 20.0), [tl, tr, br, bl]);
        // Four soup vertices in TL, TR, BR, BL order, each carrying its color.
        assert_eq!(list.vertices.len(), 4);
        assert_eq!(list.vertices[0].position, [0.0, 0.0]);
        assert_eq!(list.vertices[0].color, tl);
        assert_eq!(list.vertices[1].position, [10.0, 0.0]);
        assert_eq!(list.vertices[1].color, tr);
        assert_eq!(list.vertices[2].position, [10.0, 20.0]);
        assert_eq!(list.vertices[2].color, br);
        assert_eq!(list.vertices[3].position, [0.0, 20.0]);
        assert_eq!(list.vertices[3].color, bl);
        // Two triangles → 6 indices.
        assert_eq!(list.indices, vec![0, 1, 2, 2, 3, 0]);
    }

    #[test]
    fn quad_gradient_zero_size_is_noop() {
        let mut list = DrawList::new();
        list.quad_gradient(Rect::new(0.0, 0.0, 0.0, 20.0), [[1.0; 4]; 4]);
        list.quad_gradient(Rect::new(0.0, 0.0, 20.0, 0.0), [[1.0; 4]; 4]);
        assert!(list.vertices.is_empty());
        assert!(list.indices.is_empty());
    }

    #[test]
    fn triangle_gradient_assigns_corner_colors() {
        let mut list = DrawList::new();
        let c0 = [1.0, 0.0, 0.0, 0.3];
        let c1 = [0.0, 1.0, 0.0, 0.3];
        let c2 = [0.0, 0.0, 1.0, 0.05];
        list.triangle_gradient((0.0, 0.0), (10.0, 0.0), (5.0, 8.0), c0, c1, c2);
        // Three soup vertices in argument order, each carrying its color, one
        // triangle of indices.
        assert_eq!(list.vertices.len(), 3);
        assert_eq!(list.vertices[0].position, [0.0, 0.0]);
        assert_eq!(list.vertices[0].color, c0);
        assert_eq!(list.vertices[1].position, [10.0, 0.0]);
        assert_eq!(list.vertices[1].color, c1);
        assert_eq!(list.vertices[2].position, [5.0, 8.0]);
        assert_eq!(list.vertices[2].color, c2);
        assert_eq!(list.indices, vec![0, 1, 2]);
    }

    #[test]
    fn triangle_gradient_matches_triangle_for_flat_colors() {
        let mut g = DrawList::new();
        let mut f = DrawList::new();
        let c = [0.2, 0.5, 0.9, 0.5];
        g.triangle_gradient((0.0, 0.0), (9.0, 1.0), (4.0, 7.0), c, c, c);
        f.triangle((0.0, 0.0), (9.0, 1.0), (4.0, 7.0), c);
        assert_eq!(g.vertices, f.vertices);
        assert_eq!(g.indices, f.indices);
    }

    #[test]
    fn horizontal_gradient_sets_left_right_corners() {
        let mut list = DrawList::new();
        let left = [1.0, 0.0, 0.0, 1.0];
        let right = [0.0, 0.0, 1.0, 1.0];
        list.horizontal_gradient(Rect::new(0.0, 0.0, 10.0, 20.0), left, right);
        // TL=left, TR=right, BR=right, BL=left.
        assert_eq!(list.vertices[0].color, left);
        assert_eq!(list.vertices[1].color, right);
        assert_eq!(list.vertices[2].color, right);
        assert_eq!(list.vertices[3].color, left);
    }

    #[test]
    fn vertical_gradient_sets_top_bottom_corners() {
        let mut list = DrawList::new();
        let top = [1.0, 0.0, 0.0, 1.0];
        let bottom = [0.0, 0.0, 1.0, 1.0];
        list.vertical_gradient(Rect::new(0.0, 0.0, 10.0, 20.0), top, bottom);
        // TL=top, TR=top, BR=bottom, BL=bottom.
        assert_eq!(list.vertices[0].color, top);
        assert_eq!(list.vertices[1].color, top);
        assert_eq!(list.vertices[2].color, bottom);
        assert_eq!(list.vertices[3].color, bottom);
    }

    #[test]
    fn linear_gradient_angle_zero_matches_horizontal() {
        let start = [1.0, 0.0, 0.0, 1.0];
        let end = [0.0, 1.0, 0.0, 1.0];
        let rect = Rect::new(3.0, 5.0, 10.0, 20.0);

        let mut a = DrawList::new();
        a.linear_gradient(rect, start, end, 0.0);
        let mut b = DrawList::new();
        b.horizontal_gradient(rect, start, end);

        for (va, vb) in a.vertices.iter().zip(b.vertices.iter()) {
            for k in 0..4 {
                assert!(
                    (va.color[k] - vb.color[k]).abs() < 1e-5,
                    "angle 0 should equal horizontal gradient"
                );
            }
        }
    }

    #[test]
    fn linear_gradient_quarter_turn_matches_vertical() {
        let start = [1.0, 0.0, 0.0, 1.0];
        let end = [0.0, 1.0, 0.0, 1.0];
        let rect = Rect::new(3.0, 5.0, 10.0, 20.0);

        let mut a = DrawList::new();
        a.linear_gradient(rect, start, end, std::f32::consts::FRAC_PI_2);
        let mut b = DrawList::new();
        b.vertical_gradient(rect, start, end);

        for (va, vb) in a.vertices.iter().zip(b.vertices.iter()) {
            for k in 0..4 {
                assert!(
                    (va.color[k] - vb.color[k]).abs() < 1e-5,
                    "angle π/2 should equal vertical gradient"
                );
            }
        }
    }

    #[test]
    fn linear_gradient_zero_size_is_noop() {
        let mut list = DrawList::new();
        list.linear_gradient(Rect::new(0.0, 0.0, 0.0, 20.0), [1.0; 4], [0.0; 4], 0.7);
        assert!(list.vertices.is_empty());
        assert!(list.indices.is_empty());
    }

    #[test]
    fn radial_gradient_builds_fan_with_center_and_ring() {
        let mut list = DrawList::new();
        let inner = [1.0, 1.0, 1.0, 1.0];
        let outer = [0.0, 0.0, 0.0, 0.0];
        let segments = 8;
        list.radial_gradient(Rect::new(0.0, 0.0, 40.0, 40.0), inner, outer, segments);

        // 1 center vertex + `segments` ring vertices.
        assert_eq!(list.vertices.len() as u32, 1 + segments);
        // Center color = inner; ring colors = outer.
        assert_eq!(list.vertices[0].color, inner);
        for v in &list.vertices[1..] {
            assert_eq!(v.color, outer);
        }
        // One triangle per wedge → 3 * segments indices, every triangle shares
        // the center vertex (index 0).
        assert_eq!(list.indices.len() as u32, 3 * segments);
        for tri in list.indices.chunks(3) {
            assert_eq!(tri[0], 0, "each wedge fans from the center vertex");
        }
    }

    #[test]
    fn radial_gradient_clamps_segments_and_zero_size_is_noop() {
        let mut list = DrawList::new();
        // segments < 3 is clamped up to 3 (a triangle).
        list.radial_gradient(Rect::new(0.0, 0.0, 10.0, 10.0), [1.0; 4], [0.0; 4], 1);
        assert_eq!(list.vertices.len(), 1 + 3);

        let mut empty = DrawList::new();
        empty.radial_gradient(Rect::new(0.0, 0.0, 0.0, 10.0), [1.0; 4], [0.0; 4], 16);
        assert!(empty.vertices.is_empty());
        assert!(empty.indices.is_empty());
    }

    #[test]
    fn quad_under_translate_then_scale() {
        let mut list = DrawList::new();
        list.translate(10.0, 20.0);
        list.scale(2.0, 3.0);
        list.quad(0.0, 0.0, 5.0, 5.0, [1.0; 4]);
        // local (0,0) -> scale -> (0,0) -> translate -> (10,20)
        assert_eq!(list.vertices[0].position, [10.0, 20.0]);
        // local (5,5) -> scale -> (10,15) -> translate -> (20,35)
        assert_eq!(list.vertices[2].position, [20.0, 35.0]);
    }

    #[test]
    fn rounded_rect_under_rotation_is_not_axis_aligned() {
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4); // 45 degrees
        list.rounded_rect(Rect::new(10.0, 10.0, 20.0, 20.0), 4.0, [1.0; 4]);

        // After 45° rotation, no two distinct vertices should share an x or y
        // by accident (other than coincidentally). Check that at least some
        // vertices have non-zero Y *and* non-zero X — i.e. the geometry isn't
        // collapsed into an axis-aligned box.
        let mut has_offdiag = false;
        for v in &list.vertices {
            if v.position[0].abs() > 0.001 && v.position[1].abs() > 0.001 {
                // Distance from origin should match local distance from origin
                // (rotation is rigid). For the corner at local (30,30), that
                // distance is sqrt(1800) ~= 42.43.
                let d = (v.position[0] * v.position[0] + v.position[1] * v.position[1]).sqrt();
                if d > 5.0 {
                    has_offdiag = true;
                }
            }
        }
        assert!(
            has_offdiag,
            "rotated rounded rect should have off-axis vertices"
        );
    }

    #[test]
    fn color_multiplies_with_tint() {
        let mut list = DrawList::new();
        list.set_tint([0.5, 0.5, 0.5, 1.0]);
        list.quad(0.0, 0.0, 10.0, 10.0, [0.4, 0.6, 0.8, 1.0]);
        // Tint is baked into the chrome instance's bg color (fill: border == bg).
        let inst = list.chrome_instance(0).unwrap();
        assert!(approx(inst.bg[0], 0.2));
        assert!(approx(inst.bg[1], 0.3));
        assert!(approx(inst.bg[2], 0.4));
        assert!(approx(inst.bg[3], 1.0));
    }

    #[test]
    fn push_pop_restores_transform() {
        let mut list = DrawList::new();
        list.translate(10.0, 20.0);
        list.push_transform();
        list.translate(5.0, 5.0);
        assert_eq!(list.current_transform(), Affine2::translation(15.0, 25.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::translation(10.0, 20.0));
    }

    #[test]
    fn push_pop_restores_tint() {
        let mut list = DrawList::new();
        list.set_tint([0.5, 0.5, 0.5, 1.0]);
        list.push_tint();
        list.multiply_tint([0.5, 0.5, 0.5, 1.0]);
        assert_eq!(list.current_tint(), [0.25, 0.25, 0.25, 1.0]);
        list.pop_tint();
        assert_eq!(list.current_tint(), [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn pop_when_at_base_does_not_underflow() {
        let mut list = DrawList::new();
        list.pop_transform();
        list.pop_transform();
        list.pop_tint();
        list.pop_tint();
        assert_eq!(list.current_transform(), Affine2::IDENTITY);
        assert_eq!(list.current_tint(), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn nested_push_pop_balances() {
        let mut list = DrawList::new();
        list.push_transform();
        list.translate(1.0, 0.0);
        list.push_transform();
        list.translate(2.0, 0.0);
        list.push_transform();
        list.translate(4.0, 0.0);
        assert_eq!(list.current_transform(), Affine2::translation(7.0, 0.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::translation(3.0, 0.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::translation(1.0, 0.0));
        list.pop_transform();
        assert_eq!(list.current_transform(), Affine2::IDENTITY);
    }

    #[test]
    fn icon_corners_transformed_under_scale() {
        let mut list = DrawList::new();
        list.scale(2.0, 2.0);
        list.icon("foo", 5.0, 5.0, 10.0, 10.0);
        let c = list.icons[0].corners;
        assert_eq!(c[0], [10.0, 10.0]);
        assert_eq!(c[2], [30.0, 30.0]);
    }

    // ---- chrome_rect (instanced SDF) tests ----

    #[test]
    fn chrome_rect_fast_path_records_one_instance() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(10.0, 20.0, 80.0, 30.0),
            6.0,
            2.0,
            [0.1, 0.2, 0.3, 1.0],
            [0.4, 0.5, 0.6, 1.0],
        );
        // One instance, one Chrome command, no soup geometry.
        assert_eq!(list.chrome_instance_count(), 1);
        assert_eq!(
            list.paint_cmds,
            vec![super::PaintCmd::Analytic { instances: 0..1 }]
        );
        assert!(list.vertices.is_empty());
        let inst = list.chrome_instance(0).unwrap();
        assert_eq!(inst.rect, [10.0, 20.0, 80.0, 30.0]);
        assert_eq!(inst.bg, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(inst.border, [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(inst.params, [6.0, 2.0, 0.0, 0.0]); // radius, thickness, no clip
    }

    #[test]
    fn chrome_rect_bakes_translation_into_world_rect() {
        let mut list = DrawList::new();
        list.translate(100.0, 50.0);
        list.chrome_rect(
            Rect::new(5.0, 5.0, 20.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        assert_eq!(
            list.chrome_instance(0).unwrap().rect,
            [105.0, 55.0, 20.0, 10.0]
        );
    }

    #[test]
    fn chrome_rect_consecutive_calls_batch_into_one_run() {
        let mut list = DrawList::new();
        for i in 0..4 {
            list.chrome_rect(
                Rect::new(i as f32 * 10.0, 0.0, 8.0, 8.0),
                4.0,
                1.0,
                [1.0; 4],
                [0.0; 4],
            );
        }
        assert_eq!(list.chrome_instance_count(), 4);
        // All four collapse into a single contiguous Chrome run.
        assert_eq!(
            list.paint_cmds,
            vec![super::PaintCmd::Analytic { instances: 0..4 }]
        );
    }

    #[test]
    fn chrome_rect_interleaves_with_soup_in_order() {
        let mut list = DrawList::new();
        // soup, chrome, soup, chrome. `line` stays in the soup (it is not
        // instanced), so it produces genuine Soup runs to interleave with chrome.
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0; 4]); // 6 indices
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0; 4]); // 6 more indices
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        assert_eq!(
            list.paint_cmds,
            vec![
                super::PaintCmd::Soup { indices: 0..6 },
                super::PaintCmd::Analytic { instances: 0..1 },
                super::PaintCmd::Soup { indices: 6..12 },
                super::PaintCmd::Analytic { instances: 1..2 },
            ]
        );
        // Trailing soup (after the last command) is implicit: committed cursor
        // sits at the last flush, anything past it is the trailing run.
        assert_eq!(list.soup_committed_indices, 12);
        assert_eq!(list.indices.len(), 12);
    }

    #[test]
    fn chrome_rect_trailing_soup_left_uncommitted() {
        let mut list = DrawList::new();
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        list.line([0.0, 0.0], [10.0, 0.0], 2.0, [1.0; 4]); // soup after chrome
        // The trailing line is NOT in a command; the renderer draws
        // indices[committed..total] as the trailing run.
        assert_eq!(
            list.paint_cmds,
            vec![super::PaintCmd::Analytic { instances: 0..1 }]
        );
        assert_eq!(list.soup_committed_indices, 0);
        assert_eq!(list.indices.len(), 6);
    }

    #[test]
    fn chrome_rect_falls_back_to_soup_under_rotation() {
        let mut list = DrawList::new();
        list.rotate(std::f32::consts::FRAC_PI_4);
        list.chrome_rect(
            Rect::new(0.0, 0.0, 40.0, 20.0),
            6.0,
            2.0,
            [1.0; 4],
            [0.5; 4],
        );
        // No instance recorded; geometry went into the soup, transformed.
        assert!(list.chrome_instance_count() == 0);
        assert!(list.paint_cmds.is_empty());
        assert!(!list.vertices.is_empty());
    }

    #[test]
    fn chrome_rect_applies_tint() {
        let mut list = DrawList::new();
        list.set_tint([0.5, 0.5, 0.5, 1.0]);
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            1.0,
            [0.4, 0.6, 0.8, 1.0],
            [0.2, 0.2, 0.2, 1.0],
        );
        let inst = list.chrome_instance(0).unwrap();
        assert!(approx(inst.bg[0], 0.2) && approx(inst.bg[1], 0.3) && approx(inst.bg[2], 0.4));
        assert!(approx(inst.border[0], 0.1));
    }

    #[test]
    fn paint_quad_records_affine_gradient_radii_widths_tint_and_order() {
        let mut list = DrawList::new();
        list.line([0.0, 0.0], [4.0, 0.0], 1.0, [1.0; 4]);
        list.set_tint([0.5, 0.25, 1.0, 0.5]);
        list.rotate(0.25);
        list.paint_quad(
            Rect::new(2.0, 3.0, 40.0, 20.0),
            QuadStyle {
                background: Background::LinearGradient {
                    start: [0.8, 0.4, 0.2, 1.0],
                    end: [0.2, 0.4, 0.8, 0.5],
                    axis: GradientAxis::Horizontal,
                },
                border_widths: EdgeWidths::new(1.0, 2.0, 3.0, 4.0),
                border_color: [0.6, 0.8, 0.2, 1.0],
                corner_radii: CornerRadii::new(2.0, 4.0, 6.0, 8.0),
            },
        );
        let inst = list.chrome_instance(0).unwrap();
        assert_eq!(inst.rect, [2.0, 3.0, 40.0, 20.0]);
        assert_eq!(inst.widths, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(inst.radii, [2.0, 4.0, 6.0, 8.0]);
        assert_eq!(inst.translation[3], 1.0);
        assert_eq!(inst.bg, [0.4, 0.1, 0.2, 0.5]);
        assert_eq!(inst.border, [0.3, 0.2, 0.2, 0.5]);
        assert_eq!(
            list.paint_cmds,
            vec![
                PaintCmd::Soup { indices: 0..6 },
                PaintCmd::Analytic { instances: 0..1 },
            ]
        );
    }

    #[test]
    fn paint_quad_helpers_and_edge_line_are_fixed_size() {
        let mut list = DrawList::new();
        list.reserve_analytic_instances(4);
        let retained_capacity = list.analytic_instance_capacity();
        list.paint_quad_background(
            Rect::new(0.0, 0.0, 20.0, 10.0),
            Background::Solid([1.0; 4]),
            CornerRadii::new(1.0, 2.0, 3.0, 4.0),
        );
        list.paint_quad_border(
            Rect::new(0.0, 0.0, 20.0, 10.0),
            EdgeWidths::new(1.0, 2.0, 3.0, 4.0),
            [0.5; 4],
            CornerRadii::uniform(4.0),
        );
        list.edge_line(Rect::new(5.0, 6.0, 20.0, 10.0), Edge::Right, 2.0, [1.0; 4]);
        assert_eq!(list.chrome_instance_count(), 3);
        assert_eq!(list.chrome_instance(0).unwrap().widths, [0.0; 4]);
        assert_eq!(
            list.chrome_instance(1).unwrap().widths,
            [1.0, 2.0, 3.0, 4.0]
        );
        assert_eq!(
            list.chrome_instance(2).unwrap().rect,
            [23.0, 6.0, 2.0, 10.0]
        );
        assert_eq!(list.analytic_instance_capacity(), retained_capacity);
        list.clear();
        assert_eq!(list.analytic_instance_capacity(), retained_capacity);
    }

    #[test]
    fn chrome_rect_records_active_clip() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(5.0, 6.0, 30.0, 40.0));
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        let inst = list.chrome_instance(0).unwrap();
        assert_eq!(inst.clip, [5.0, 6.0, 30.0, 40.0]);
        assert_eq!(inst.params[2], 1.0); // clip_enabled
    }

    #[test]
    fn chrome_rect_zero_size_draws_nothing() {
        let mut list = DrawList::new();
        list.chrome_rect(Rect::new(0.0, 0.0, 0.0, 10.0), 4.0, 1.0, [1.0; 4], [0.0; 4]);
        assert!(list.chrome_instance_count() == 0);
        assert!(list.paint_cmds.is_empty());
    }

    #[test]
    fn clear_resets_chrome_state() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        assert!(list.chrome_instance_count() != 0);
        assert!(!list.paint_cmds.is_empty());
        list.clear();
        assert!(list.chrome_instance_count() == 0);
        assert!(list.paint_cmds.is_empty());
        assert_eq!(list.soup_committed_indices, 0);
    }

    // ---- Debug scopes ----

    #[test]
    fn debug_scope_records_span_of_emitted_primitives() {
        let mut list = DrawList::new();
        list.chrome_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.push_debug_scope("inner");
        list.chrome_rect(Rect::new(0.0, 0.0, 5.0, 5.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.chrome_rect(Rect::new(5.0, 0.0, 5.0, 5.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.pop_debug_scope();
        list.chrome_rect(Rect::new(0.0, 0.0, 3.0, 3.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);

        let scopes = list.debug_scopes();
        assert_eq!(scopes.len(), 1);
        let s = &scopes[0];
        assert_eq!(s.name, "inner");
        assert!(s.closed);
        // The scope owns exactly the two chrome rects drawn between push and pop.
        assert_eq!(s.start.chrome_instances, 1);
        assert_eq!(s.end.chrome_instances, 3);
        assert_eq!(s.counts().chrome_instances, 2);
        assert_eq!(s.counts().total(), 2);
    }

    #[test]
    fn debug_scopes_nest_with_parent_and_depth() {
        let mut list = DrawList::new();
        list.push_debug_scope("window");
        list.push_debug_scope("row");
        list.push_debug_scope("button");
        list.pop_debug_scope();
        list.pop_debug_scope();
        list.pop_debug_scope();

        let s = list.debug_scopes();
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].parent, s[0].depth), (None, 0));
        assert_eq!((s[1].parent, s[1].depth), (Some(0), 1));
        assert_eq!((s[2].parent, s[2].depth), (Some(1), 2));
        assert!(s.iter().all(|sc| sc.closed));
        assert_eq!(list.debug_scope_depth(), 0);
    }

    #[test]
    fn sibling_scopes_get_the_same_parent() {
        let mut list = DrawList::new();
        list.push_debug_scope("panel");
        list.push_debug_scope("a");
        list.pop_debug_scope();
        list.push_debug_scope("b");
        list.pop_debug_scope();
        list.pop_debug_scope();

        let s = list.debug_scopes();
        assert_eq!(s[1].parent, Some(0));
        assert_eq!(s[2].parent, Some(0));
        assert_eq!(s[1].depth, 1);
        assert_eq!(s[2].depth, 1);
    }

    #[test]
    fn declared_rect_is_transformed_to_world_space() {
        let mut list = DrawList::new();
        list.push_transform();
        list.translate(100.0, 50.0);
        list.push_debug_scope_rect("moved", Rect::new(10.0, 10.0, 20.0, 20.0));
        list.pop_debug_scope();
        list.pop_transform();

        let declared = list.debug_scopes()[0].declared.expect("declared rect");
        assert_eq!(
            (declared.x, declared.y, declared.width, declared.height),
            (110.0, 60.0, 20.0, 20.0)
        );
    }

    #[test]
    fn scope_records_active_clip() {
        let mut list = DrawList::new();
        list.push_clip(Rect::new(5.0, 6.0, 30.0, 40.0));
        list.push_debug_scope("clipped");
        list.pop_debug_scope();
        list.pop_clip();
        list.push_debug_scope("unclipped");
        list.pop_debug_scope();

        let s = list.debug_scopes();
        assert_eq!(s[0].clip, Some(Rect::new(5.0, 6.0, 30.0, 40.0)));
        assert_eq!(s[1].clip, None);
    }

    #[test]
    fn unpopped_scope_stays_open() {
        let mut list = DrawList::new();
        list.push_debug_scope("leaked");
        list.chrome_rect(Rect::new(0.0, 0.0, 4.0, 4.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);

        let s = &list.debug_scopes()[0];
        assert!(
            !s.closed,
            "an unpopped scope must be flagged, not silently closed"
        );
        assert_eq!(s.end, s.start, "end is only stamped on pop");
        assert_eq!(list.debug_scope_depth(), 1);
    }

    #[test]
    fn pop_without_push_is_a_noop() {
        let mut list = DrawList::new();
        list.pop_debug_scope();
        list.pop_debug_scope();
        assert!(list.debug_scopes().is_empty());
        assert_eq!(list.debug_scope_depth(), 0);
    }

    #[test]
    fn truncate_debug_scopes_closes_back_to_depth() {
        let mut list = DrawList::new();
        list.push_debug_scope("a");
        list.push_debug_scope("b");
        list.push_debug_scope("c");
        list.truncate_debug_scopes(1);

        assert_eq!(list.debug_scope_depth(), 1);
        let s = list.debug_scopes();
        assert!(!s[0].closed, "the scope at the retained depth stays open");
        assert!(s[1].closed);
        assert!(s[2].closed);
    }

    #[test]
    fn clear_resets_debug_scopes() {
        let mut list = DrawList::new();
        list.push_debug_scope("stale");
        list.chrome_rect(Rect::new(0.0, 0.0, 4.0, 4.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.pop_debug_scope();
        assert_eq!(list.debug_scopes().len(), 1);

        list.clear();
        assert!(list.debug_scopes().is_empty());
        assert_eq!(list.debug_scope_depth(), 0);
    }

    #[test]
    fn scopes_cost_nothing_when_unused() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert!(list.debug_scopes().is_empty());
        assert_eq!(list.debug_scope_depth(), 0);
    }

    #[test]
    fn adjacent_scopes_own_disjoint_ranges_despite_command_run_merging() {
        // `push_chrome_instance` MERGES consecutive chrome draws into one
        // `PaintCmd::Chrome` run by mutating the last command in place, so
        // `paint_cmds` is NOT append-only and must never be spanned. The
        // instance buffers themselves are, which is what scopes rely on.
        let mut list = DrawList::new();
        list.push_debug_scope("a");
        list.chrome_rect(Rect::new(0.0, 0.0, 5.0, 5.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.pop_debug_scope();
        list.push_debug_scope("b");
        list.chrome_rect(Rect::new(5.0, 0.0, 5.0, 5.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.pop_debug_scope();

        // One merged draw command spanning both scopes...
        assert_eq!(
            list.paint_cmds.len(),
            1,
            "runs merge across the scope boundary"
        );
        // ...but the scopes still own disjoint, correct instance ranges.
        let s = list.debug_scopes();
        assert_eq!(
            (s[0].start.chrome_instances, s[0].end.chrome_instances),
            (0, 1)
        );
        assert_eq!(
            (s[1].start.chrome_instances, s[1].end.chrome_instances),
            (1, 2)
        );
    }

    // ---- Degenerate-drop counter ----

    #[test]
    fn degenerate_quad_is_counted_not_silently_lost() {
        let mut list = DrawList::new();
        // The classic bug: padding ate the whole width.
        list.quad(10.0, 10.0, -4.0, 20.0, [1.0; 4]);
        assert!(list.vertices.is_empty(), "nothing is drawn");
        assert!(list.chrome_instance_count() == 0);
        assert_eq!(list.dropped_degenerate(), 1, "but the drop is recorded");
    }

    #[test]
    fn degenerate_drops_counted_across_primitive_kinds() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, 0.0, 10.0, [1.0; 4]);
        list.chrome_rect(Rect::new(0.0, 0.0, 10.0, 0.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.rect_outline(Rect::new(0.0, 0.0, 10.0, 10.0), 0.0, [1.0; 4]);
        list.circle((5.0, 5.0), 0.0, [1.0; 4]);
        list.line([0.0, 0.0], [10.0, 0.0], -1.0, [1.0; 4]);
        assert_eq!(list.dropped_degenerate(), 5);
    }

    #[test]
    fn rounded_rect_fallback_counts_the_drop_only_once() {
        // `rounded_rect` delegates a degenerate rect to `quad`; only the
        // delegate may count it, or one mistake reads as two.
        let mut list = DrawList::new();
        list.rounded_rect(Rect::new(0.0, 0.0, -5.0, 10.0), 4.0, [1.0; 4]);
        assert_eq!(list.dropped_degenerate(), 1);
    }

    #[test]
    fn healthy_primitives_do_not_count_as_drops() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        list.chrome_rect(Rect::new(0.0, 0.0, 8.0, 8.0), 2.0, 1.0, [1.0; 4], [0.0; 4]);
        list.circle((5.0, 5.0), 3.0, [1.0; 4]);
        assert_eq!(list.dropped_degenerate(), 0);
    }

    #[test]
    fn scope_spans_the_drop_counter() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, -1.0, 5.0, [1.0; 4]); // outside any scope
        list.push_debug_scope("toolbar");
        list.quad(0.0, 0.0, -1.0, 5.0, [1.0; 4]);
        list.quad(0.0, 0.0, 5.0, -1.0, [1.0; 4]);
        list.pop_debug_scope();

        let s = &list.debug_scopes()[0];
        assert_eq!(s.counts().dropped_degenerate, 2, "attributed to the scope");
        assert!(
            s.counts().is_empty(),
            "a scope that only dropped draws nothing"
        );
        assert_eq!(
            list.dropped_degenerate(),
            3,
            "list-wide total includes the loose one"
        );
    }

    #[test]
    fn clear_resets_the_drop_counter() {
        let mut list = DrawList::new();
        list.quad(0.0, 0.0, -1.0, 5.0, [1.0; 4]);
        assert_eq!(list.dropped_degenerate(), 1);
        list.clear();
        assert_eq!(list.dropped_degenerate(), 0);
    }

    #[test]
    fn prim_counts_since_saturates_and_totals_triangles() {
        let a = PrimCounts {
            indices: 6,
            texts: 1,
            ..PrimCounts::default()
        };
        let b = PrimCounts {
            indices: 12,
            texts: 3,
            chrome_instances: 2,
            ..PrimCounts::default()
        };
        let d = b.since(a);
        assert_eq!(d.indices, 6);
        assert_eq!(d.texts, 2);
        assert_eq!(d.chrome_instances, 2);
        // 6 indices = 2 triangles, + 2 texts + 2 chrome
        assert_eq!(d.total(), 6);
        // reversed subtraction saturates rather than underflowing
        assert!(a.since(b).is_empty());
    }

    #[test]
    fn nine_slice_carries_transform() {
        let mut list = DrawList::new();
        list.translate(50.0, 60.0);
        list.scale(2.0, 2.0);
        list.nine_slice_id(0, 0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        let n = &list.nine_slices[0];
        // Local (0,0) -> world (50,60); local (10,10) -> (70,80).
        let tl = n.transform.transform_point([0.0, 0.0]);
        let br = n.transform.transform_point([10.0, 10.0]);
        assert!(approx(tl[0], 50.0) && approx(tl[1], 60.0));
        assert!(approx(br[0], 70.0) && approx(br[1], 80.0));
    }

    #[test]
    fn css_spread_radius_matches_positive_and_negative_numeric_cases() {
        assert!(approx(css_spread_radius(12.0, 4.0), 16.0));
        assert!(approx(css_spread_radius(2.0, 4.0), 5.5));
        assert!(approx(css_spread_radius(0.0, 4.0), 0.0));
        assert!(approx(css_spread_radius(12.0, -4.0), 8.0));
        assert!(approx(css_spread_radius(2.0, -4.0), 0.0));
    }

    #[test]
    fn shadow_radius_overlap_normalization_scales_asymmetric_corners() {
        let expanded = [2.0, 12.0, 4.0, 8.0].map(|radius| css_spread_radius(radius, 4.0));
        assert_eq!(expanded, [5.5, 16.0, 8.0, 12.0]);
        assert_eq!(normalize_shadow_radii(expanded, 48.0, 28.0), expanded);

        let radii = normalize_shadow_radii([80.0, 40.0, 20.0, 10.0], 100.0, 40.0);
        // The left-side overlap (80 + 10 over a 40px height) is limiting, so
        // the CSS overlap factor is 4/9 and applies uniformly to every corner.
        let expected = [320.0 / 9.0, 160.0 / 9.0, 80.0 / 9.0, 40.0 / 9.0];
        for (actual, expected) in radii.into_iter().zip(expected) {
            assert!(approx(actual, expected), "{actual} != {expected}");
        }

        let contracted = [
            css_spread_radius(radii[0], -8.0),
            css_spread_radius(radii[1], -8.0),
            css_spread_radius(radii[2], -8.0),
            css_spread_radius(radii[3], -8.0),
        ];
        let contracted = normalize_shadow_radii(contracted, 84.0, 24.0);
        // After -8 spread the left-side sum is limiting; 27/31 overlap scale.
        let expected = [24.0, 264.0 / 31.0, 24.0 / 31.0, 0.0];
        for (actual, expected) in contracted.into_iter().zip(expected) {
            assert!(approx(actual, expected), "{actual} != {expected}");
        }
    }

    #[test]
    fn analytic_shadow_normalizes_geometry_and_affine() {
        let mut dl = DrawList::new();
        dl.push_transform();
        dl.translate(7.0, 9.0);
        dl.rotate(0.25);
        dl.box_shadow_outset(
            Rect::new(10.0, 20.0, 100.0, 40.0),
            CornerRadii::new(80.0, 40.0, 20.0, 10.0),
            BoxShadow {
                offset: [3.0, -2.0],
                blur: 8.0,
                spread: 4.0,
                color: [0.2, 0.3, 0.4, 0.5],
                inset: false,
            },
        );
        let s = dl.shadow_instance(0).unwrap();
        assert_eq!(s.shadow_rect, [9.0, 14.0, 108.0, 48.0]);
        // Rotation preserves an isotropic sigma=blur/2 kernel.
        assert!(approx(s.params[0], 4.0));
        assert!(approx(s.params[2], 4.0));
        assert!(approx(s.params[3], 0.0));
        assert_eq!(s.translation[3], 0.0);
        assert_eq!(
            s.linear,
            [
                0.25_f32.cos(),
                -0.25_f32.sin(),
                0.25_f32.sin(),
                0.25_f32.cos()
            ]
        );
        assert!(s.shadow_radii[0] > s.shadow_radii[3]);
        assert_eq!(dl.prim_counts().shadow_instances, 1);
    }

    #[test]
    fn analytic_shadow_pulls_screen_gaussian_through_nonuniform_scale() {
        let mut dl = DrawList::new();
        dl.scale(2.0, 0.5);
        dl.box_shadow_outset(
            Rect::new(10.0, 20.0, 100.0, 40.0),
            CornerRadii::uniform(4.0),
            BoxShadow {
                blur: 8.0,
                color: [0.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
        );
        let s = dl.shadow_instance(0).unwrap();
        // A^-1=diag(1/2,2): a screen-isotropic sigma=4 becomes local
        // sigma_x|y=2 and sigma_y=8, with no conditional mean shift.
        assert!(approx(s.params[0], 8.0));
        assert!(approx(s.params[2], 2.0));
        assert!(approx(s.params[3], 0.0));
        // Three local sigmas plus the inverse image of one screen pixel.
        assert_eq!(s.raster_rect, [3.75, -5.0, 112.5, 90.0]);
    }

    #[test]
    fn analytic_inset_raster_is_conservatively_inflated() {
        let mut dl = DrawList::new();
        dl.scale(2.0, 0.5);
        dl.box_shadow_inset(
            Rect::new(10.0, 20.0, 100.0, 40.0),
            CornerRadii::uniform(4.0),
            BoxShadow {
                blur: 8.0,
                color: [0.0, 0.0, 0.0, 1.0],
                inset: true,
                ..Default::default()
            },
        );
        // The inverse image of a centered one-screen-pixel square has local
        // half-extents (0.25, 1). Fragment SDF coverage still clips the inset.
        assert_eq!(
            dl.shadow_instance(0).unwrap().raster_rect,
            [9.75, 19.0, 100.5, 42.0]
        );
    }

    #[test]
    fn analytic_shadow_pulls_screen_gaussian_through_shear() {
        let mut dl = DrawList::new();
        dl.compose_top(&Affine2::new(1.0, 0.5, 0.0, 0.0, 1.0, 0.0));
        dl.box_shadow_outset(
            Rect::new(0.0, 0.0, 20.0, 10.0),
            CornerRadii::default(),
            BoxShadow {
                blur: 8.0,
                color: [0.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
        );
        let params = dl.shadow_instance(0).unwrap().params;
        assert!(approx(params[0], 4.0));
        assert!(approx(params[2], 4.0));
        assert!(approx(params[3], -0.5));
    }

    #[test]
    fn analytic_shadow_inset_collapse_and_css_order() {
        let mut dl = DrawList::new();
        let outset = BoxShadow {
            color: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        };
        let inset_a = BoxShadow {
            spread: 100.0,
            color: [0.0, 1.0, 0.0, 1.0],
            inset: true,
            ..Default::default()
        };
        let inset_b = BoxShadow {
            color: [0.0, 0.0, 1.0, 1.0],
            inset: true,
            ..Default::default()
        };
        dl.box_shadows_inset(
            Rect::new(0.0, 0.0, 20.0, 10.0),
            CornerRadii::uniform(2.0),
            &[outset, inset_a, inset_b],
        );
        assert_eq!(dl.shadow_instance_count(), 2);
        assert_eq!(dl.shadow_instance(0).unwrap().color, inset_b.color);
        assert_eq!(dl.shadow_instance(1).unwrap().color, inset_a.color);
        assert_eq!(dl.shadow_instance(1).unwrap().shadow_rect[2..], [0.0, 0.0]);
        assert_eq!(dl.shadow_instance(1).unwrap().params[1], 1.0);
        assert!(
            matches!(&dl.paint_cmds[0], PaintCmd::Analytic { instances } if instances == &(0..2))
        );
    }

    #[test]
    fn analytic_shadow_culls_clip_and_diagnoses_invalid() {
        let mut dl = DrawList::new();
        dl.push_clip(Rect::new(200.0, 200.0, 20.0, 20.0));
        let visible = BoxShadow {
            color: [0.0, 0.0, 0.0, 1.0],
            ..Default::default()
        };
        dl.box_shadow_outset(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            CornerRadii::default(),
            visible,
        );
        assert!(dl.shadow_instance_count() == 0);
        dl.pop_clip();
        dl.box_shadow_outset(
            Rect::new(f32::NAN, 0.0, 10.0, 10.0),
            CornerRadii::default(),
            visible,
        );
        assert_eq!(dl.dropped_degenerate(), 1);
    }

    #[test]
    fn clear_retains_shadow_capacity_and_resets_count() {
        let mut dl = DrawList::new();
        dl.reserve_analytic_instances(8);
        let capacity = dl.analytic_instance_capacity();
        dl.box_shadow_outset(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            CornerRadii::default(),
            BoxShadow {
                color: [0.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
        );
        dl.clear();
        assert!(dl.shadow_instance_count() == 0);
        assert_eq!(dl.analytic_instance_capacity(), capacity);
        assert_eq!(dl.prim_counts().shadow_instances, 0);
    }

    #[test]
    fn alternating_chrome_and_shadow_share_one_ordered_run() {
        let mut d = DrawList::new();
        for i in 0..10_000 {
            d.box_shadow_outset(
                Rect::new(i as f32, 0.0, 1.0, 1.0),
                CornerRadii::default(),
                BoxShadow {
                    color: [0.0, 0.0, 0.0, 1.0],
                    ..Default::default()
                },
            );
            d.quad(i as f32, 0.0, 1.0, 1.0, [1.0; 4]);
        }
        assert_eq!(d.analytic_instances.len(), 20_000);
        assert_eq!(d.shadow_instance_count(), 10_000);
        assert_eq!(d.chrome_instance_count(), 10_000);
        assert_eq!(
            d.paint_cmds,
            vec![PaintCmd::Analytic {
                instances: 0..20_000
            }]
        );
        for (index, instance) in d.analytic_instances.iter().enumerate() {
            assert_eq!(instance.kind, (index % 2 == 0) as u32);
        }
    }

    #[test]
    fn paint_stream_records_and_coalesces_all_payload_kinds() {
        let mut d = DrawList::new();
        d.nine_slice_id(7, 0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        d.nine_slice_id(7, 10.0, 0.0, 10.0, 10.0, [1.0; 4]);
        d.icon("first", 0.0, 0.0, 8.0, 8.0);
        d.icon("second", 8.0, 0.0, 8.0, 8.0);
        d.triangle((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), [1.0; 4]);
        d.quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]);

        assert!(matches!(&d.paint_cmds[0], PaintCmd::NineSlice { draws } if draws == &(0..2)));
        assert!(matches!(&d.paint_cmds[1], PaintCmd::Icon { draws } if draws == &(0..2)));
        assert!(matches!(&d.paint_cmds[2], PaintCmd::Soup { indices } if indices == &(0..3)));
        assert!(
            matches!(&d.paint_cmds[3], PaintCmd::Analytic { instances } if instances == &(0..1))
        );
    }
}
