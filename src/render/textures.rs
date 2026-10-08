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
//! The renderer draws runs of consecutive sprites that share a texture in one
//! call ([`batches`]), and counts each run of a small sprite in a texture of its
//! own ([`RenderStats::small_texture_batches`](crate::RenderStats)), so a frame
//! full of them is reported as a sign they belong in an atlas.
//!
//! Sprite ids are global across all textures; names are too.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use super::atlas::{AtlasRegion, AtlasSlot, SpriteAtlas};

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
        }
    }
}

impl std::error::Error for SpriteError {}

/// The texture a resolved sprite is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    Own(SpriteId),
    Atlas(AtlasId),
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
/// their own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Uploads {
    pub atlases: usize,
    pub atlas_bytes: u64,
    pub own: usize,
    pub own_bytes: u64,
}

/// One uploaded texture and the bind group that samples it.
struct Gpu {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

enum Home {
    /// A texture of its own. `pixels` waits for the first flush, which creates
    /// `gpu` and drops them: the GPU copy is the only one kept.
    Own {
        pixels: Option<Vec<u8>>,
        gpu: Option<Gpu>,
    },
    Atlas {
        atlas: AtlasId,
        slot: AtlasSlot,
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
    /// Sprites of their own waiting for their first flush.
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
                    if width > self.max_dimension || height > self.max_dimension {
                        return Err(SpriteError::TooLarge {
                            width,
                            height,
                            max: self.max_dimension,
                        });
                    }
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
        if let Some(previous) = name.and_then(|name| self.names.get(name).copied()) {
            self.remove(previous);
        }
        let own = matches!(home, Home::Own { .. });
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
        Some(match sprite.home {
            Home::Own { .. } => Resolved {
                binding: Binding::Own(id),
                region: AtlasRegion {
                    x: 0,
                    y: 0,
                    w: sprite.width,
                    h: sprite.height,
                },
                texture: (sprite.width, sprite.height),
            },
            Home::Atlas { atlas, slot } => {
                let shared = &self.atlases[atlas.0 as usize];
                Resolved {
                    binding: Binding::Atlas(atlas),
                    region: shared.atlas.region(slot)?,
                    texture: (shared.atlas.width(), shared.atlas.height()),
                }
            }
        })
    }

    /// The bind group sampling `binding`'s texture, once it has been uploaded.
    pub fn bind_group(&self, binding: Binding) -> Option<&wgpu::BindGroup> {
        let gpu = match binding {
            Binding::Own(id) => match &self.sprite(id)?.home {
                Home::Own { gpu, .. } => gpu.as_ref(),
                Home::Atlas { .. } => None,
            },
            Binding::Atlas(atlas) => self.atlases.get(atlas.0 as usize)?.gpu.as_ref(),
        };
        gpu.map(|gpu| &gpu.bind_group)
    }

    /// Upload what changed since the last flush: new sprites of their own, and
    /// atlases that gained, lost or moved sprites (all of an atlas' pixels,
    /// into a new texture if it grew).
    pub fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
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
                shared.gpu = Some(Gpu::new(device, layout, sampler, size, "ui sprite atlas"));
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
            if let Home::Own { pixels, gpu } = &mut sprite.home
                && let Some(pixels) = pixels.take()
            {
                let created = Gpu::new(device, layout, sampler, size, "ui sprite texture");
                created.write(queue, size, &pixels);
                *gpu = Some(created);
                uploads.own += 1;
                uploads.own_bytes += pixels.len() as u64;
            }
        }
        uploads
    }

    fn sprite(&self, id: SpriteId) -> Option<&Sprite> {
        self.sprites.get(id as usize)?.as_ref()
    }
}

impl Gpu {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        (width, height): (u32, u32),
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self {
            texture,
            bind_group,
        }
    }

    fn write(&self, queue: &wgpu::Queue, (width, height): (u32, u32), pixels: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Split draws, in paint order, into runs that sample the same texture: each
/// run is one draw call, so order is kept and a texture switch only happens
/// where the texture changes.
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
