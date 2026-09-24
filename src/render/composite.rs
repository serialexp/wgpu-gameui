//! How the UI reaches the host's render target, and the offscreen layer used
//! when it can't draw there directly.
//!
//! The crate's colours are sRGB-encoded and must be blended in sRGB space to
//! look like the browser designs they come from (see [`crate::color`]). A
//! target that *stores* sRGB-encoded bytes (`Rgba8Unorm`, `Bgra8Unorm`, …) does
//! exactly that with plain alpha blending, so the UI draws **directly** into it.
//!
//! A target that stores linear light — an `*Srgb` format (the GPU encodes on
//! write and blends in linear) or a float format — can't. For those the UI
//! draws into an [`Offscreen`] layer in a non-sRGB format, then one composite
//! pass decodes it and blends it over the host's contents. Opaque UI pixels come
//! out exactly as authored; translucent UI over the host's own scene mixes in
//! linear light only at that last step.

use crate::color::srgb_to_linear;

const SHADER: &str = include_str!("composite.wgsl");

/// Where the UI's pipelines draw for a given host target format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetPlan {
    /// The format the UI pipelines are built for.
    pub work_format: wgpu::TextureFormat,
    /// Whether the UI draws into an [`Offscreen`] layer and composites (the
    /// host stores linear light) rather than straight into the host target.
    pub offscreen: bool,
}

impl TargetPlan {
    pub(crate) fn for_host(host: wgpu::TextureFormat) -> Self {
        if host.is_srgb() {
            Self {
                work_format: host.remove_srgb_suffix(),
                offscreen: true,
            }
        } else if stores_linear_float(host) {
            Self {
                work_format: wgpu::TextureFormat::Rgba8Unorm,
                offscreen: true,
            }
        } else {
            Self {
                work_format: host,
                offscreen: false,
            }
        }
    }

    /// Whether the host target holds linear light (so values written to it, and
    /// its clear colour, must be linear).
    pub(crate) fn host_is_linear(self) -> bool {
        self.offscreen
    }

    /// The `wgpu::Color` that clears the host target to the sRGB-encoded
    /// `color` (e.g. `theme.background`).
    pub(crate) fn clear_color(self, color: [f32; 4]) -> wgpu::Color {
        let [r, g, b, a] = if self.host_is_linear() {
            srgb_to_linear(color)
        } else {
            color
        };
        wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: a as f64,
        }
    }
}

/// Renderable float formats hold linear light (HDR / scRGB swapchains).
fn stores_linear_float(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float
            | wgpu::TextureFormat::Rgba32Float
            | wgpu::TextureFormat::Rg11b10Ufloat
    )
}

/// A viewport-sized layer, reused across frames and re-created on resize.
struct Layer {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

/// The offscreen UI layer plus the pass that composites it onto the host.
pub(crate) struct Offscreen {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    work_format: wgpu::TextureFormat,
    layer: Option<Layer>,
}

impl Offscreen {
    pub(crate) fn new(
        device: &wgpu::Device,
        work_format: wgpu::TextureFormat,
        host_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui composite shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui composite bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui composite pipeline layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui composite pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_composite"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_composite"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: host_format,
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
        Self {
            pipeline,
            bgl,
            work_format,
            layer: None,
        }
    }

    /// Record a clear of the layer (re-creating it at `size` if needed) and
    /// return the view the UI should draw into.
    pub(crate) fn begin(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
    ) -> wgpu::TextureView {
        let size = (size.0.max(1), size.1.max(1));
        if self.layer.as_ref().is_none_or(|l| l.size != size) {
            self.layer = Some(self.create_layer(device, size));
        }
        let layer = self.layer.as_ref().expect("layer just ensured");
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui offscreen clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &layer.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        layer.view.clone()
    }

    /// Record the composite of the layer over `host`.
    pub(crate) fn composite(&self, encoder: &mut wgpu::CommandEncoder, host: &wgpu::TextureView) {
        let Some(layer) = self.layer.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: host,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn create_layer(&self, device: &wgpu::Device, size: (u32, u32)) -> Layer {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui offscreen layer"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.work_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui composite bg"),
            layout: &self.bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Layer {
            view,
            bind_group,
            size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat as F;

    #[test]
    fn unorm_hosts_draw_directly() {
        for f in [F::Rgba8Unorm, F::Bgra8Unorm, F::Rgb10a2Unorm] {
            let plan = TargetPlan::for_host(f);
            assert_eq!(plan.work_format, f);
            assert!(!plan.offscreen, "{f:?}");
        }
    }

    #[test]
    fn srgb_hosts_draw_offscreen_in_the_matching_unorm_format() {
        assert_eq!(
            TargetPlan::for_host(F::Bgra8UnormSrgb),
            TargetPlan {
                work_format: F::Bgra8Unorm,
                offscreen: true
            }
        );
        assert_eq!(
            TargetPlan::for_host(F::Rgba8UnormSrgb).work_format,
            F::Rgba8Unorm
        );
    }

    #[test]
    fn float_hosts_draw_offscreen() {
        let plan = TargetPlan::for_host(F::Rgba16Float);
        assert!(plan.offscreen);
        assert_eq!(plan.work_format, F::Rgba8Unorm);
    }

    #[test]
    fn clear_color_is_linear_only_for_linear_hosts() {
        let bg = crate::color::hex(0x0a0d0f);
        let direct = TargetPlan::for_host(F::Bgra8Unorm).clear_color(bg);
        assert_eq!(direct.r, bg[0] as f64);
        let linear = TargetPlan::for_host(F::Bgra8UnormSrgb).clear_color(bg);
        assert!((linear.r - srgb_to_linear(bg)[0] as f64).abs() < 1e-9);
        assert!(linear.r < direct.r);
        assert_eq!(linear.a, 1.0);
    }
}
