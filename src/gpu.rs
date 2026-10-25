//! Optional wgpu adapter. The host owns its window, device, queue and presentation.
//! Queue writes must be followed by encoding/submitting this batch before preparing
//! another one. All resources and the queue must belong to the same device.
use crate::visual::Visual;
use crate::{Quad, Rect};
use std::ops::Range;

/// Color-emoji rasterization and GPU compositing.
#[cfg(feature = "portable")]
pub mod emoji;
pub(crate) mod instances;
pub mod outlines;
pub mod retained;
pub mod stats;
#[cfg(feature = "portable")]
pub mod unified;
pub use retained::RetainedTarget;
mod transforms;
pub use transforms::{Shared, Transform2D, TransformId, Transforms};

pub use instances::Damage;

pub const CAPACITY: usize = 96;

/// Built-in shader sources for composing Lagui GPU render passes.
///
/// Applications can use these without copying Lagui-owned implementation
/// assets into their own repository.
pub mod shaders {
    /// Shared arc/cubic corner distance for host-owned gradient renderers.
    pub const CORNERS: &str = include_str!("../native/corners.wgsl");
    pub const EMOJI: &str = include_str!("../native/emoji.wgsl");
    pub const PREVIEW_3D: &str = include_str!("../native/preview3d.wgsl");
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareError {
    Capacity,
    InvalidViewport,
}

impl core::fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Capacity => "GPU renderer capacity was exceeded",
            Self::InvalidViewport => "GPU renderer received an invalid viewport or visual",
        })
    }
}

impl std::error::Error for PrepareError {}

/// A logical GUI region reserved for application-owned GPU rendering.
///
/// The host records its custom render pass between GUI layers (or after the
/// GUI) and applies this viewport/scissor pair. Coordinates are converted to
/// physical pixels and clipped to both the widget clip and render target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuCanvas {
    pub bounds: Rect,
    pub clip: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalCanvas {
    pub viewport: [f32; 4],
    pub scissor: [u32; 4],
}

impl GpuCanvas {
    pub const fn new(bounds: Rect, clip: Rect) -> Self {
        Self { bounds, clip }
    }

    pub fn physical(self, scale: f32, target: [u32; 2]) -> Option<PhysicalCanvas> {
        if !scale.is_finite() || scale <= 0. || target[0] == 0 || target[1] == 0 {
            return None;
        }
        let left = self.bounds.x.max(self.clip.x).max(0.);
        let top = self.bounds.y.max(self.clip.y).max(0.);
        let right = (self.bounds.x + self.bounds.width)
            .min(self.clip.x + self.clip.width)
            .min(target[0] as f32 / scale);
        let bottom = (self.bounds.y + self.bounds.height)
            .min(self.clip.y + self.clip.height)
            .min(target[1] as f32 / scale);
        if ![left, top, right, bottom].iter().all(|v| v.is_finite())
            || right <= left
            || bottom <= top
        {
            return None;
        }
        let x = (left * scale).floor().max(0.) as u32;
        let y = (top * scale).floor().max(0.) as u32;
        let right_px = (right * scale).ceil().min(target[0] as f32) as u32;
        let bottom_px = (bottom * scale).ceil().min(target[1] as f32) as u32;
        Some(PhysicalCanvas {
            viewport: [
                self.bounds.x * scale,
                self.bounds.y * scale,
                self.bounds.width * scale,
                self.bounds.height * scale,
            ],
            scissor: [x, y, right_px - x, bottom_px - y],
        })
    }

    pub fn apply(self, pass: &mut wgpu::RenderPass<'_>, scale: f32, target: [u32; 2]) -> bool {
        let Some(region) = self.physical(scale, target) else {
            return false;
        };
        pass.set_viewport(
            region.viewport[0],
            region.viewport[1],
            region.viewport[2],
            region.viewport[3],
            0.,
            1.,
        );
        pass.set_scissor_rect(
            region.scissor[0],
            region.scissor[1],
            region.scissor[2],
            region.scissor[3],
        );
        true
    }
}

/// Bytes per prepared visual: shape and clip rectangles, half-float colors and
/// shadow, then corner parameters. Draw bounds are derived on the GPU.
const VISUAL_STRIDE: usize = 80;

/// Rounded and clipped UI primitive renderer used by the full studio demo.
///
/// Prepared instances are retained on the GPU; each prepare uploads only the
/// visuals whose encoding changed and reports the changed region as [`Damage`].
pub struct VisualRenderer {
    pipeline: wgpu::RenderPipeline,
    instances: instances::InstanceBuffer,
    viewport: wgpu::Buffer,
    binding: wgpu::BindGroup,
    transform_binding: wgpu::BindGroup,
    identity_binding: wgpu::BindGroup,
    transformed: bool,
}

impl VisualRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Result<Self, PrepareError> {
        Self::with_shared(device, format, capacity, &Shared::new(device))
    }

    pub fn with_shared(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        capacity: usize,
        shared: &Shared,
    ) -> Result<Self, PrepareError> {
        let instances =
            instances::InstanceBuffer::new(device, "visual instances", VISUAL_STRIDE, capacity)
                .map_err(|_| PrepareError::Capacity)?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lagui rounded visuals"),
            source: wgpu::ShaderSource::Wgsl(
                std::format!(
                    "{}{}{}",
                    include_str!("../native/transforms.wgsl"),
                    shaders::CORNERS,
                    include_str!("../native/visual.wgsl")
                )
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("visual viewport layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("visual viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("visual viewport binding"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("visual pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(&shared.transforms)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("visual pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VISUAL_STRIDE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float16x4, 3 => Float16x4,
                        4 => Float16x4, 5 => Float16x4, 6 => Float32x3, 7 => Uint32
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            instances,
            viewport,
            binding,
            transform_binding: shared.identity.clone(),
            identity_binding: shared.identity.clone(),
            transformed: false,
        })
    }

    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        visuals: &[Visual],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), PrepareError> {
        self.prepare_impl(queue, visuals, viewport, scale_factor, None)
    }

    pub fn prepare_transformed(
        &mut self,
        queue: &wgpu::Queue,
        visuals: &[Visual],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: &Transforms,
    ) -> Result<(), PrepareError> {
        self.transform_binding = transforms.binding.clone();
        self.prepare_impl(queue, visuals, viewport, scale_factor, Some(transforms))
    }

    fn prepare_impl(
        &mut self,
        queue: &wgpu::Queue,
        visuals: &[Visual],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: Option<&Transforms>,
    ) -> Result<(), PrepareError> {
        if visuals.iter().any(|v| {
            transforms.map_or(v.transform != TransformId::IDENTITY, |t| {
                t.get(v.transform).is_none()
            })
        }) {
            self.instances.abandon();
            return Err(PrepareError::InvalidViewport);
        }
        if self.transformed != transforms.is_some()
            || transforms.is_some_and(|t| t.previous(TransformId::IDENTITY).is_none())
        {
            self.instances.abandon();
        }
        self.transformed = transforms.is_some();
        self.transform_binding =
            transforms.map_or_else(|| self.identity_binding.clone(), |t| t.binding.clone());
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            self.instances.abandon();
            return Err(PrepareError::InvalidViewport);
        }
        if let Err(error) = self.encode(visuals) {
            self.instances.abandon();
            return Err(error);
        }
        let mut uniforms = [0; 16];
        uniforms[..4].copy_from_slice(&viewport[0].to_le_bytes());
        uniforms[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        uniforms[8..12].copy_from_slice(&scale_factor.to_le_bytes());
        let margin = 1. / scale_factor;
        self.instances
            .commit_bounds(queue, &self.viewport, uniforms, |bytes, old| {
                let id = TransformId(u32::from_le_bytes(bytes[76..80].try_into().unwrap()));
                let transform = transforms
                    .and_then(|t| if old { t.previous(id) } else { t.get(id) })
                    .unwrap_or_default();
                visual_damage_transformed(
                    bytes,
                    margin,
                    transform,
                    transforms.map(|t| {
                        t.clip(
                            id,
                            Rect::new(
                                instances::read_f32(bytes, 4),
                                instances::read_f32(bytes, 5),
                                instances::read_f32(bytes, 6),
                                instances::read_f32(bytes, 7),
                            ),
                            old,
                        )
                    }),
                )
            });
        Ok(())
    }

    fn encode(&mut self, visuals: &[Visual]) -> Result<(), PrepareError> {
        self.instances
            .reserve(visuals.len())
            .map_err(|_| PrepareError::Capacity)?;
        let bytes = self.instances.begin();
        for visual in visuals {
            let shape = visual.shape_bounds;
            let clip = visual.clip;
            let rects = [
                shape.x,
                shape.y,
                shape.width,
                shape.height,
                clip.x,
                clip.y,
                clip.width,
                clip.height,
            ];
            let params = [visual.radius, visual.stroke_width, visual.corner_smoothing];
            let shadow = [
                visual.shadow_offset[0],
                visual.shadow_offset[1],
                visual.shadow_blur,
                visual.shadow_spread,
            ];
            if rects
                .iter()
                .chain(&params)
                .chain(&visual.color)
                .chain(&visual.inner_color)
                .chain(&visual.shadow_color)
                .chain(&shadow)
                .any(|v| !v.is_finite())
            {
                return Err(PrepareError::InvalidViewport);
            }
            instances::push_f32(bytes, &rects);
            instances::push_f16x4(bytes, visual.color);
            instances::push_f16x4(bytes, visual.inner_color);
            instances::push_f16x4(bytes, visual.shadow_color);
            instances::push_f16x4(bytes, shadow);
            instances::push_f32(bytes, &params);
            instances::push_u32(bytes, &[visual.transform.0]);
        }
        Ok(())
    }

    /// Region changed by the latest prepare, for hosts that redraw partially.
    pub fn upload_stats(&self) -> crate::gpu::stats::UploadStats {
        self.instances.stats()
    }

    pub fn damage(&self) -> Damage {
        self.instances.damage()
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.instances.count() as usize);
    }

    /// Draws a prepared subrange, allowing other renderers to preserve painter's order.
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<usize>) {
        let count = self.instances.count() as usize;
        let start = range.start.min(count);
        let end = range.end.min(count);
        if start >= end {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_bind_group(1, &self.transform_binding, &[]);
        pass.set_vertex_buffer(0, self.instances.buffer().slice(..));
        pass.draw(0..6, start as u32..end as u32);
    }

    pub(crate) fn encoded(&self) -> &[u8] {
        self.instances.encoded()
    }

    pub fn prepared_len(&self) -> usize {
        self.instances.count() as usize
    }
}

/// Logical region one encoded visual can touch: its shape grown by the shadow
/// reach and AA margin, then clipped. Mirrors `vertex_main` in visual.wgsl.
fn visual_damage_transformed(
    bytes: &[u8],
    margin: f32,
    transform: Transform2D,
    transformed_clip: Option<Option<Rect>>,
) -> Option<Rect> {
    let shape = transform.rect(Rect::new(
        instances::read_f32(bytes, 0),
        instances::read_f32(bytes, 1),
        instances::read_f32(bytes, 2),
        instances::read_f32(bytes, 3),
    ));
    let clip = Rect::new(
        instances::read_f32(bytes, 4),
        instances::read_f32(bytes, 5),
        instances::read_f32(bytes, 6),
        instances::read_f32(bytes, 7),
    );
    let [left, right, top, bottom] = if bytes[48..56].iter().any(|byte| *byte != 0) {
        let [offset_x, offset_y, blur, spread] =
            core::array::from_fn(|i| instances::read_f16(bytes, 56 + i * 2) * transform.scale);
        let reach = blur.max(0.) * 3. + spread.max(0.);
        [
            reach + (-offset_x).max(0.),
            reach + offset_x.max(0.),
            reach + (-offset_y).max(0.),
            reach + offset_y.max(0.),
        ]
    } else {
        [0.; 4]
    };
    Rect::new(
        shape.x - left - margin,
        shape.y - top - margin,
        shape.width + left + right + 2. * margin,
        shape.height + top + bottom + 2. * margin,
    )
    .intersection(match transformed_clip {
        Some(r) => r?,
        None => clip,
    })
}

pub struct Renderer {
    device: wgpu::Device,
    capacity: usize,
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    viewport: wgpu::Buffer,
    binding: wgpu::BindGroup,
    bytes: std::vec::Vec<u8>,
    count: u32,
}

impl Renderer {
    /// Accepts an existing device, allowing the application to share GPU resources.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lagui rectangles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../native/quad.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport binding"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rectangle layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rectangle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rectangle instances"),
            size: (CAPACITY * 32) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            device: device.clone(),
            capacity: CAPACITY,
            pipeline,
            instances,
            viewport,
            binding,
            bytes: std::vec::Vec::with_capacity(CAPACITY * 32),
            count: 0,
        }
    }

    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        quads: &[Quad],
        viewport: [f32; 2],
    ) -> Result<(), PrepareError> {
        self.count = 0;
        let required = quads.len().checked_mul(32).ok_or(PrepareError::Capacity)?;
        if quads.len() > u32::MAX as usize || required as u64 > self.device.limits().max_buffer_size
        {
            return Err(PrepareError::Capacity);
        }
        if quads.len() > self.capacity {
            let limit = (self.device.limits().max_buffer_size / 32)
                .min(u64::from(u32::MAX))
                .min((usize::MAX / 32) as u64) as usize;
            self.capacity = quads
                .len()
                .checked_next_power_of_two()
                .unwrap_or(limit)
                .min(limit);
            self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("grown rectangle instances"),
                size: (self.capacity * 32) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.bytes.resize(required, 0);
        if viewport.iter().any(|n| !n.is_finite() || *n <= 0.) {
            return Err(PrepareError::InvalidViewport);
        }
        // Explicit wire encoding keeps the core free of unsafe casts and dependencies.
        for (q, bytes) in quads.iter().zip(self.bytes.chunks_exact_mut(32)) {
            let values = [
                q.bounds.x,
                q.bounds.y,
                q.bounds.width,
                q.bounds.height,
                q.color[0],
                q.color[1],
                q.color[2],
                q.color[3],
            ];
            if values.iter().any(|v| !v.is_finite()) {
                return Err(PrepareError::InvalidViewport);
            }
            for (value, slot) in values.iter().zip(bytes.chunks_exact_mut(4)) {
                slot.copy_from_slice(&value.to_le_bytes());
            }
        }
        let mut screen = [0; 16];
        screen[..4].copy_from_slice(&viewport[0].to_le_bytes());
        screen[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        queue.write_buffer(&self.viewport, 0, &screen);
        if !quads.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.bytes[..quads.len() * 32]);
        }
        self.count = quads.len() as u32;
        Ok(())
    }

    /// Draw into the host's existing render pass, preserving its load/store policy.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

#[cfg(test)]
mod canvas_tests {
    use super::*;

    #[test]
    fn canvas_intersects_clip_and_target_in_physical_pixels() {
        let canvas = GpuCanvas::new(
            Rect::new(10., 20., 100., 80.),
            Rect::new(30., 0., 100., 70.),
        );
        let physical = canvas.physical(2., [180, 200]).unwrap();
        assert_eq!(physical.viewport, [20., 40., 200., 160.]);
        assert_eq!(physical.scissor, [60, 40, 120, 100]);
    }

    #[test]
    fn empty_or_invalid_canvas_is_rejected() {
        let canvas = GpuCanvas::new(Rect::new(0., 0., 0., 10.), Rect::new(0., 0., 10., 10.));
        assert!(canvas.physical(1., [100, 100]).is_none());
        assert!(canvas.physical(f32::NAN, [100, 100]).is_none());
    }
}
