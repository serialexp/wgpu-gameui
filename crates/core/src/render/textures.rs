//! Where each sprite's pixels live on the GPU.
//!
//! By default a sprite gets a texture of its own: it can be as large as the GPU
//! allows, and unloading it frees its memory at once. Drawing it costs a texture
//! switch, though, so many small sprites drawn together (icons, thumbnails) are
//! cheaper packed into a shared [`SpriteAtlas`]. Sharing is opt-in: the
//! application creates an atlas with the largest size it wants
//! ([`UiRenderer::create_atlas`](crate::UiRenderer::create_atlas)) and names it
//! when loading each sprite that belongs there ([`Placement::Atlas`]).
//!
//! The renderer draws sprites that share a texture in one call where paint
//! order allows: a list's plan groups them by texture
//! (`docs/design/draw-call-batching.md`), and a list filled through the public
//! arrays draws runs of consecutive ones ([`batches`]). It counts each draw of
//! small sprites in a texture of their own
//! ([`RenderStats::small_texture_batches`](crate::RenderStats)), so a frame
//! full of them is reported as a sign they belong in an atlas.
//!
//! A sprite of its own can have its pixels replaced in place
//! ([`Textures::update_rgba`]), keeping its id and texture: a picture that
//! changes every frame costs an upload, not a new texture. YUV sprites
//! ([`Textures::insert_yuv`], see [`super::yuv`]) always have textures of their
//! own and are drawn through a pipeline of their own.
//!
//! Sprite ids are global across all textures; names are too.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use super::atlas::{AtlasRegion, AtlasSlot, SpriteAtlas};
use super::yuv::{YuvColour, YuvPlane, YuvPlanes, chroma_size};

/// Opaque handle to a loaded sprite, wherever its pixels live. Stable until the
/// sprite is unloaded.
pub type SpriteId = u32;

/// A shared atlas created by
/// [`UiRenderer::create_atlas`](crate::UiRenderer::create_atlas).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AtlasId(u32);

/// Where a sprite's pixels go when it is loaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// A texture of its own, up to the GPU's largest texture size.
    #[default]
    Own,
    /// Packed into a shared atlas, drawn together with the atlas' other sprites.
    Atlas(AtlasId),
}

/// A sprite no larger than this on either side counts as small: drawn from a
/// texture of its own, it costs a texture switch for little pixel work, and it
/// would fit an atlas easily.
pub const SMALL_SPRITE_EDGE: u32 = 256;

/// Why a sprite could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpriteError {
    /// A sprite needs at least one pixel on each side.
    Empty,
    /// The pixel buffer isn't `width * height * 4` bytes of RGBA8.
    PixelCount {
        /// `width * height * 4`.
        expected: usize,
        /// The buffer's length.
        got: usize,
    },
    /// Larger than its texture may be: the GPU's limit for a texture of its own,
    /// or the atlas' maximum size (less the edge it keeps around each sprite).
    TooLarge {
        /// The sprite's width in pixels.
        width: u32,
        /// The sprite's height in pixels.
        height: u32,
        /// The longest side the texture allows.
        max: u32,
    },
    /// The atlas is at its maximum size and has no room left for the sprite.
    AtlasFull {
        /// The atlas that is full.
        atlas: AtlasId,
        /// The sprite's width in pixels.
        width: u32,
        /// The sprite's height in pixels.
        height: u32,
    },
    /// No atlas with this id was created.
    UnknownAtlas(AtlasId),
    /// No sprite with this id is loaded.
    UnknownSprite(SpriteId),
    /// The sprite is packed into an atlas, which is uploaded whole: only a
    /// sprite with a texture of its own can be updated in place.
    InAtlas(SpriteId),
    /// The sprite holds the other kind of pixels: RGBA given for a YUV sprite,
    /// or YUV planes for an RGBA one.
    WrongFormat(SpriteId),
    /// The sprite shows a caller's GPU texture: draw into that texture instead.
    External(SpriteId),
    /// A YUV plane's rows are shorter than the picture needs.
    PlaneStride {
        /// The plane.
        plane: YuvPlane,
        /// The stride given.
        stride: u32,
        /// The plane's width, the least its stride may be.
        width: u32,
    },
    /// A YUV plane holds too few or too many bytes for its rows: it needs
    /// every row at its stride, the last with or without its padding.
    PlaneSize {
        /// The plane.
        plane: YuvPlane,
        /// Its bytes with every row at its stride.
        expected: usize,
        /// The bytes given.
        got: usize,
    },
}

impl std::fmt::Display for SpriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpriteError::Empty => write!(f, "a sprite needs at least one pixel on each side"),
            SpriteError::PixelCount { expected, got } => {
                write!(f, "sprite pixels are {got} bytes, expected {expected}")
            }
            SpriteError::TooLarge { width, height, max } => {
                write!(
                    f,
                    "sprite {width}x{height} is larger than its texture allows ({max})"
                )
            }
            SpriteError::AtlasFull {
                atlas,
                width,
                height,
            } => write!(
                f,
                "atlas {} has no room for a {width}x{height} sprite",
                atlas.0
            ),
            SpriteError::UnknownAtlas(atlas) => write!(f, "no atlas {}", atlas.0),
            SpriteError::UnknownSprite(sprite) => write!(f, "no sprite {sprite}"),
            SpriteError::InAtlas(sprite) => write!(
                f,
                "sprite {sprite} is in an atlas, so it can't be updated in place"
            ),
            SpriteError::WrongFormat(sprite) => {
                write!(f, "sprite {sprite} holds the other kind of pixels")
            }
            SpriteError::External(sprite) => write!(
                f,
                "sprite {sprite} shows a caller's GPU texture, so it has no pixels to update"
            ),
            SpriteError::PlaneStride {
                plane,
                stride,
                width,
            } => write!(
                f,
                "YUV plane {plane:?} has a stride of {stride}, less than its width {width}"
            ),
            SpriteError::PlaneSize {
                plane,
                expected,
                got,
            } => write!(f, "YUV plane {plane:?} is {got} bytes, expected {expected}"),
        }
    }
}

impl std::error::Error for SpriteError {}

/// The texture a resolved sprite is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    Own(SpriteId),
    Atlas(AtlasId),
    /// A YUV sprite's planes, drawn through the YUV pipeline.
    Yuv(SpriteId),
}

impl Binding {
    /// A number of its own for each texture, to tell batches apart.
    pub(crate) fn code(self) -> u64 {
        match self {
            Binding::Own(id) => u64::from(id) << 2,
            Binding::Atlas(AtlasId(n)) => u64::from(n) << 2 | 1,
            Binding::Yuv(id) => u64::from(id) << 2 | 2,
        }
    }
}

/// What drawing a sprite needs: its texture, where it is in it, and how large
/// that texture is (to turn the region into UVs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Resolved {
    pub binding: Binding,
    pub region: AtlasRegion,
    pub texture: (u32, u32),
}

impl Resolved {
    /// Whether this is a small sprite drawn from a texture of its own.
    pub fn small_own(&self) -> bool {
        matches!(self.binding, Binding::Own(_))
            && self.region.w <= SMALL_SPRITE_EDGE
            && self.region.h <= SMALL_SPRITE_EDGE
    }
}

/// Texture uploads made by [`Textures::flush`]: whole atlases, and sprites of
/// their own (RGBA or YUV).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Uploads {
    pub atlases: usize,
    pub atlas_bytes: u64,
    pub own: usize,
    pub own_bytes: u64,
    /// Textures of their own created: one per sprite, at its first upload.
    pub created: usize,
}

/// What sprite textures are sampled through: the sampler, and the bind group
/// layouts of the RGBA and the YUV pipelines.
pub(crate) struct Sampling<'a> {
    pub sampler: &'a wgpu::Sampler,
    pub rgba: &'a wgpu::BindGroupLayout,
    pub yuv: &'a wgpu::BindGroupLayout,
}

/// One uploaded texture and the bind group that samples it.
struct Gpu {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// A YUV sprite's three plane textures and the bind group that samples them
/// with its colour conversion.
struct YuvGpu {
    planes: [wgpu::Texture; 3],
    bind_group: wgpu::BindGroup,
}

enum Home {
    /// A texture of its own. `pixels` waits for the next flush (the first, or
    /// one after an update), which creates `gpu` if needed, writes them and
    /// drops them: the GPU copy is the only one kept.
    Own {
        pixels: Option<Vec<u8>>,
        gpu: Option<Gpu>,
    },
    /// Three plane textures of its own; `planes` waits for the next flush like
    /// an RGBA sprite's pixels.
    Yuv {
        planes: Option<YuvPlanes<'static>>,
        colour: YuvColour,
        /// Boxed: three textures and a bind group would make every sprite
        /// the size of a YUV one.
        gpu: Option<Box<YuvGpu>>,
    },
    Atlas {
        atlas: AtlasId,
        slot: AtlasSlot,
    },
    /// Caller-rendered texture. The bind group retains its view without uploads.
    External {
        bind_group: wgpu::BindGroup,
    },
}

struct Sprite {
    name: Option<String>,
    width: u32,
    height: u32,
    home: Home,
}

struct Shared {
    atlas: SpriteAtlas,
    /// Created at the first flush with a sprite in the atlas, and again each
    /// time the atlas grows.
    gpu: Option<Gpu>,
    gpu_size: (u32, u32),
}

/// Every loaded sprite and shared atlas, and their GPU textures.
pub(crate) struct Textures {
    /// The GPU's largest texture edge.
    max_dimension: u32,
    /// Indexed by [`SpriteId`]; `None` is an unloaded sprite's slot, reused
    /// through `free`.
    sprites: Vec<Option<Sprite>>,
    free: Vec<SpriteId>,
    names: HashMap<String, SpriteId>,
    atlases: Vec<Shared>,
    /// Sprites of their own waiting for the next flush: their first upload,
    /// or an update. May hold an id twice, or one since unloaded; the flush
    /// skips a sprite with nothing waiting.
    pending: Vec<SpriteId>,
}

impl Textures {
    pub fn new(max_dimension: u32) -> Self {
        Self {
            max_dimension: max_dimension.max(1),
            sprites: Vec::new(),
            free: Vec::new(),
            names: HashMap::new(),
            atlases: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// The GPU's largest texture edge, which also bounds an atlas.
    pub fn max_dimension(&self) -> u32 {
        self.max_dimension
    }

    /// A new, empty shared atlas that may grow to `max_size` on each side
    /// (capped at the GPU's limit). No GPU memory is used until a sprite goes
    /// into it.
    pub fn create_atlas(&mut self, max_size: u32) -> AtlasId {
        let id = AtlasId(self.atlases.len() as u32);
        self.atlases.push(Shared {
            atlas: SpriteAtlas::with_max_size(max_size.min(self.max_dimension)),
            gpu: None,
            gpu_size: (0, 0),
        });
        id
    }

    /// The atlas' current texture size, or `None` for an unknown id.
    pub fn atlas_size(&self, id: AtlasId) -> Option<(u32, u32)> {
        let shared = self.atlases.get(id.0 as usize)?;
        Some((shared.atlas.width(), shared.atlas.height()))
    }

    /// Repack the atlas' live sprites (see [`SpriteAtlas::compact`]). Returns
    /// `false` for an unknown id, or when the repacked sprites wouldn't fit
    /// (the atlas then stays as it was).
    pub fn compact(&mut self, id: AtlasId) -> bool {
        self.atlases
            .get_mut(id.0 as usize)
            .is_some_and(|shared| shared.atlas.compact())
    }

    /// Load a `width`×`height` RGBA8 sprite where `placement` says. A `name`
    /// already in use moves to the new sprite, and the sprite that had it is
    /// unloaded (once the new one has loaded: a failed load changes nothing).
    pub fn insert<'a>(
        &mut self,
        name: Option<&str>,
        width: u32,
        height: u32,
        pixels: impl Into<Cow<'a, [u8]>>,
        placement: Placement,
    ) -> Result<SpriteId, SpriteError> {
        let pixels = pixels.into();
        if width == 0 || height == 0 {
            return Err(SpriteError::Empty);
        }
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            return Err(SpriteError::PixelCount {
                expected,
                got: pixels.len(),
            });
        }
        let home =
            match placement {
                Placement::Own => {
                    self.check_own_size(width, height)?;
                    Home::Own {
                        // Owned pixels move in: a screenful isn't copied.
                        pixels: Some(pixels.into_owned()),
                        gpu: None,
                    }
                }
                Placement::Atlas(atlas) => {
                    let shared = self
                        .atlases
                        .get_mut(atlas.0 as usize)
                        .ok_or(SpriteError::UnknownAtlas(atlas))?;
                    if !shared.atlas.could_fit(width, height) {
                        return Err(SpriteError::TooLarge {
                            width,
                            height,
                            max: shared.atlas.max_size().saturating_sub(2),
                        });
                    }
                    let slot = shared.atlas.insert(width, height, &pixels).ok_or(
                        SpriteError::AtlasFull {
                            atlas,
                            width,
                            height,
                        },
                    )?;
                    Home::Atlas { atlas, slot }
                }
            };
        Ok(self.add(name, width, height, home))
    }

    /// Load a `width`×`height` YUV 4:2:0 sprite, in textures of its own, whose
    /// codes turn into colour as `colour` says. Names work as for
    /// [`insert`](Self::insert).
    pub fn insert_yuv(
        &mut self,
        name: Option<&str>,
        width: u32,
        height: u32,
        planes: YuvPlanes<'_>,
        colour: YuvColour,
    ) -> Result<SpriteId, SpriteError> {
        if width == 0 || height == 0 {
            return Err(SpriteError::Empty);
        }
        self.check_own_size(width, height)?;
        check_planes(width, height, &planes)?;
        let home = Home::Yuv {
            planes: Some(planes.into_owned()),
            colour,
            gpu: None,
        };
        Ok(self.add(name, width, height, home))
    }

    /// Replace an RGBA sprite's pixels, keeping its id and its texture. Only a
    /// sprite with a texture of its own can be updated, and only at its size.
    /// The pixels are uploaded at the next flush; updating again before then
    /// replaces them, so only the latest is uploaded.
    pub fn update_rgba<'a>(
        &mut self,
        id: SpriteId,
        pixels: impl Into<Cow<'a, [u8]>>,
    ) -> Result<(), SpriteError> {
        let sprite = self.sprite_mut(id)?;
        let expected = sprite.width as usize * sprite.height as usize * 4;
        let Home::Own {
            pixels: waiting, ..
        } = &mut sprite.home
        else {
            return Err(match sprite.home {
                Home::Atlas { .. } => SpriteError::InAtlas(id),
                Home::External { .. } => SpriteError::External(id),
                _ => SpriteError::WrongFormat(id),
            });
        };
        let pixels = pixels.into();
        if pixels.len() != expected {
            return Err(SpriteError::PixelCount {
                expected,
                got: pixels.len(),
            });
        }
        let queued = waiting.replace(pixels.into_owned()).is_some();
        if !queued {
            self.pending.push(id);
        }
        Ok(())
    }

    /// Replace a YUV sprite's planes, keeping its id and its textures. Uploaded
    /// at the next flush, like [`update_rgba`](Self::update_rgba).
    pub fn update_yuv(&mut self, id: SpriteId, planes: YuvPlanes<'_>) -> Result<(), SpriteError> {
        let sprite = self.sprite_mut(id)?;
        let (width, height) = (sprite.width, sprite.height);
        let Home::Yuv {
            planes: waiting, ..
        } = &mut sprite.home
        else {
            return Err(match sprite.home {
                Home::External { .. } => SpriteError::External(id),
                _ => SpriteError::WrongFormat(id),
            });
        };
        check_planes(width, height, &planes)?;
        let queued = waiting.replace(planes.into_owned()).is_some();
        if !queued {
            self.pending.push(id);
        }
        Ok(())
    }

    /// Refuse a texture of its own larger than the GPU allows.
    fn check_own_size(&self, width: u32, height: u32) -> Result<(), SpriteError> {
        if width > self.max_dimension || height > self.max_dimension {
            return Err(SpriteError::TooLarge {
                width,
                height,
                max: self.max_dimension,
            });
        }
        Ok(())
    }

    /// Give a loaded sprite an id (and `name`, taking it from the sprite that
    /// had it), and queue a texture of its own for its first upload.
    fn add(&mut self, name: Option<&str>, width: u32, height: u32, home: Home) -> SpriteId {
        if let Some(previous) = name.and_then(|name| self.names.get(name).copied()) {
            self.remove(previous);
        }
        let own = !matches!(home, Home::Atlas { .. });
        let sprite = Sprite {
            name: name.map(str::to_owned),
            width,
            height,
            home,
        };
        let id = match self.free.pop() {
            Some(id) => {
                self.sprites[id as usize] = Some(sprite);
                id
            }
            None => {
                self.sprites.push(Some(sprite));
                (self.sprites.len() - 1) as SpriteId
            }
        };
        if let Some(name) = name {
            self.names.insert(name.to_owned(), id);
        }
        if own {
            self.pending.push(id);
        }
        id
    }

    /// Register a caller-rendered view with no pending pixel upload. Replacing
    /// an existing name keeps its SpriteId stable, including across resizes.
    pub fn external(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        bind_group: wgpu::BindGroup,
    ) -> Result<SpriteId, SpriteError> {
        if width == 0 || height == 0 {
            return Err(SpriteError::Empty);
        }
        self.check_own_size(width, height)?;
        let sprite = Sprite {
            name: Some(name.to_owned()),
            width,
            height,
            home: Home::External { bind_group },
        };
        if let Some(id) = self.names.get(name).copied() {
            if let Some(old) = self.sprites[id as usize].take()
                && let Home::Atlas { atlas, slot } = old.home
            {
                self.atlases[atlas.0 as usize].atlas.remove(slot);
            }
            self.sprites[id as usize] = Some(sprite);
            return Ok(id);
        }
        let id = if let Some(id) = self.free.pop() {
            self.sprites[id as usize] = Some(sprite);
            id
        } else {
            self.sprites.push(Some(sprite));
            (self.sprites.len() - 1) as SpriteId
        };
        self.names.insert(name.to_owned(), id);
        Ok(id)
    }

    /// Unload a sprite: a texture of its own is freed (once the GPU has
    /// finished any work already submitted with it), an atlas slot is
    /// reclaimed. Returns `false` for an unknown or already unloaded id.
    pub fn remove(&mut self, id: SpriteId) -> bool {
        let Some(sprite) = self.sprites.get_mut(id as usize).and_then(Option::take) else {
            return false;
        };
        if let Some(name) = &sprite.name {
            self.names.remove(name);
        }
        if let Home::Atlas { atlas, slot } = sprite.home {
            self.atlases[atlas.0 as usize].atlas.remove(slot);
        }
        self.free.push(id);
        true
    }

    pub fn id_for(&self, name: &str) -> Option<SpriteId> {
        self.names.get(name).copied()
    }

    pub fn name(&self, id: SpriteId) -> Option<&str> {
        self.sprite(id)?.name.as_deref()
    }

    /// Where to sample `id` from, or `None` for an unknown sprite.
    pub fn resolve(&self, id: SpriteId) -> Option<Resolved> {
        let sprite = self.sprite(id)?;
        let binding = match sprite.home {
            Home::Own { .. } | Home::External { .. } => Binding::Own(id),
            Home::Yuv { .. } => Binding::Yuv(id),
            Home::Atlas { atlas, slot } => {
                let shared = &self.atlases[atlas.0 as usize];
                return Some(Resolved {
                    binding: Binding::Atlas(atlas),
                    region: shared.atlas.region(slot)?,
                    texture: (shared.atlas.width(), shared.atlas.height()),
                });
            }
        };
        // A texture of its own is the whole sprite.
        Some(Resolved {
            binding,
            region: AtlasRegion {
                x: 0,
                y: 0,
                w: sprite.width,
                h: sprite.height,
            },
            texture: (sprite.width, sprite.height),
        })
    }

    /// The bind group sampling `binding`'s texture, once it has been uploaded.
    pub fn bind_group(&self, binding: Binding) -> Option<&wgpu::BindGroup> {
        match binding {
            Binding::Own(id) => match &self.sprite(id)?.home {
                Home::Own { gpu, .. } => gpu.as_ref().map(|gpu| &gpu.bind_group),
                Home::External { bind_group } => Some(bind_group),
                _ => None,
            },
            Binding::Yuv(id) => match &self.sprite(id)?.home {
                Home::Yuv { gpu, .. } => gpu.as_ref().map(|gpu| &gpu.bind_group),
                _ => None,
            },
            Binding::Atlas(atlas) => self
                .atlases
                .get(atlas.0 as usize)?
                .gpu
                .as_ref()
                .map(|gpu| &gpu.bind_group),
        }
    }

    /// Upload what changed since the last flush: new and updated sprites of
    /// their own, and atlases that gained, lost or moved sprites (all of an
    /// atlas' pixels, into a new texture if it grew).
    pub fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sampling: &Sampling<'_>,
    ) -> Uploads {
        let mut uploads = Uploads::default();
        for shared in &mut self.atlases {
            let size = (shared.atlas.width(), shared.atlas.height());
            if shared.atlas.is_empty() {
                // Nothing to show: hold no memory for it until a sprite comes.
                // A run already prepared keeps its own reference to the texture.
                shared.gpu = None;
                shared.atlas.take_dirty();
                continue;
            }
            let grew = shared.gpu.is_none() || shared.gpu_size != size;
            if grew {
                shared.gpu = Some(Gpu::new(device, sampling, size, "ui sprite atlas"));
                shared.gpu_size = size;
            }
            if shared.atlas.take_dirty() || grew {
                let gpu = shared.gpu.as_ref().expect("created above");
                gpu.write(queue, size, &shared.atlas.build_pixel_buffer());
                uploads.atlases += 1;
                uploads.atlas_bytes += u64::from(size.0) * u64::from(size.1) * 4;
            }
        }
        for id in std::mem::take(&mut self.pending) {
            // Unloaded (and perhaps reused for an atlas sprite) since it was queued.
            let Some(Some(sprite)) = self.sprites.get_mut(id as usize) else {
                continue;
            };
            let size = (sprite.width, sprite.height);
            let created = match &mut sprite.home {
                Home::Own { pixels, gpu } => {
                    let Some(pixels) = pixels.take() else {
                        continue;
                    };
                    let created = gpu.is_none();
                    gpu.get_or_insert_with(|| {
                        Gpu::new(device, sampling, size, "ui sprite texture")
                    })
                    .write(queue, size, &pixels);
                    uploads.own_bytes += pixels.len() as u64;
                    created
                }
                Home::Yuv {
                    planes,
                    colour,
                    gpu,
                } => {
                    let Some(planes) = planes.take() else {
                        continue;
                    };
                    let created = gpu.is_none();
                    gpu.get_or_insert_with(|| {
                        Box::new(YuvGpu::new(device, queue, sampling, size, *colour))
                    })
                    .write(queue, size, &planes);
                    uploads.own_bytes += planes.len() as u64;
                    created
                }
                Home::Atlas { .. } | Home::External { .. } => continue,
            };
            uploads.own += 1;
            uploads.created += usize::from(created);
        }
        uploads
    }

    fn sprite(&self, id: SpriteId) -> Option<&Sprite> {
        self.sprites.get(id as usize)?.as_ref()
    }

    fn sprite_mut(&mut self, id: SpriteId) -> Result<&mut Sprite, SpriteError> {
        self.sprites
            .get_mut(id as usize)
            .and_then(Option::as_mut)
            .ok_or(SpriteError::UnknownSprite(id))
    }
}

/// Refuse planes that aren't the sizes a `width`×`height` 4:2:0 picture has:
/// each row at least the plane's width, and every row there at its stride,
/// the last with or without its padding.
fn check_planes(width: u32, height: u32, planes: &YuvPlanes<'_>) -> Result<(), SpriteError> {
    let (cw, ch) = chroma_size(width, height);
    for (plane, bytes, stride, (width, rows)) in [
        (YuvPlane::Y, &planes.y, planes.y_stride, (width, height)),
        (YuvPlane::U, &planes.u, planes.chroma_stride, (cw, ch)),
        (YuvPlane::V, &planes.v, planes.chroma_stride, (cw, ch)),
    ] {
        if stride < width {
            return Err(SpriteError::PlaneStride {
                plane,
                stride,
                width,
            });
        }
        let padded = stride as usize * rows as usize;
        let least = padded - (stride - width) as usize;
        if !(least..=padded).contains(&bytes.len()) {
            return Err(SpriteError::PlaneSize {
                plane,
                expected: padded,
                got: bytes.len(),
            });
        }
    }
    Ok(())
}

impl Gpu {
    fn new(device: &wgpu::Device, sampling: &Sampling<'_>, size: (u32, u32), label: &str) -> Self {
        let texture = create_texture(device, size, wgpu::TextureFormat::Rgba8Unorm, label);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: sampling.rgba,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampling.sampler),
                },
            ],
        });
        Self {
            texture,
            bind_group,
        }
    }

    fn write(&self, queue: &wgpu::Queue, (width, height): (u32, u32), pixels: &[u8]) {
        write_texture(queue, &self.texture, (width, height), 4 * width, pixels);
    }
}

impl YuvGpu {
    /// The plane textures for a `size` picture, and a bind group that samples
    /// them with `colour`'s conversion (written once: it doesn't change).
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sampling: &Sampling<'_>,
        (width, height): (u32, u32),
        colour: YuvColour,
    ) -> Self {
        let chroma = chroma_size(width, height);
        let planes = [
            ((width, height), "ui yuv sprite y"),
            (chroma, "ui yuv sprite u"),
            (chroma, "ui yuv sprite v"),
        ]
        .map(|(size, label)| create_texture(device, size, wgpu::TextureFormat::R8Unorm, label));
        let views = planes
            .each_ref()
            .map(|plane| plane.create_view(&wgpu::TextureViewDescriptor::default()));
        // `YuvSprite` in `ui.wgsl`: the colour conversion's three rows, then
        // what turns the picture's UV into the colour planes' (rounded up at
        // an odd size, they reach half a pixel past the picture).
        let [r, g, b] = colour.to_rgb().rows;
        let chroma_scale = [
            width as f32 / (2 * chroma.0) as f32,
            height as f32 / (2 * chroma.1) as f32,
            0.0,
            0.0,
        ];
        let uniform = [r, g, b, chroma_scale];
        let conversion = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui yuv sprite colour"),
            size: std::mem::size_of_val(&uniform) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&conversion, 0, bytemuck::cast_slice(&uniform));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui yuv sprite"),
            layout: sampling.yuv,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampling.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: conversion.as_entire_binding(),
                },
            ],
        });
        Self { planes, bind_group }
    }

    fn write(&self, queue: &wgpu::Queue, (width, height): (u32, u32), planes: &YuvPlanes<'_>) {
        let chroma = chroma_size(width, height);
        let [y, u, v] = &self.planes;
        write_texture(queue, y, (width, height), planes.y_stride, &planes.y);
        write_texture(queue, u, chroma, planes.chroma_stride, &planes.u);
        write_texture(queue, v, chroma, planes.chroma_stride, &planes.v);
    }
}

/// A sampled texture the CPU writes into.
fn create_texture(
    device: &wgpu::Device,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Write a whole `width`×`height` texture from rows `stride` bytes apart
/// (`write_texture`, unlike a buffer copy, needs no particular stride).
fn write_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
    stride: u32,
    pixels: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Split draws, in paint order, into runs that sample the same texture: each
/// run is one draw call, so order is kept and a texture switch only happens
/// where the texture changes. Used for lists without a paint stream; a list
/// with one is grouped by its plan.
pub(crate) fn batches(bindings: &[Binding]) -> impl Iterator<Item = (Binding, Range<usize>)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        let &binding = bindings.get(start)?;
        let end = start
            + bindings[start..]
                .iter()
                .take_while(|&&other| other == binding)
                .count();
        let batch = (binding, start..end);
        start = end;
        Some(batch)
    })
}

#[cfg(test)]
#[path = "textures_tests.rs"]
mod tests;
