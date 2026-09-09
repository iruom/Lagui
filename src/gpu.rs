//! Optional wgpu adapter. The host owns its window, device, queue and presentation.
//! Queue writes must be followed by encoding/submitting this batch before preparing
//! another one. All resources and the queue must belong to the same device.
use crate::visual::Visual;
use crate::{Quad, Rect};
use std::ops::Range;

pub const CAPACITY: usize = 96;

#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    Capacity,
    InvalidViewport,
}

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

/// Rounded and clipped UI primitive renderer used by the full studio demo.
pub struct VisualRenderer {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    viewport: wgpu::Buffer,
    binding: wgpu::BindGroup,
    bytes: std::vec::Vec<u8>,
    capacity: usize,
    count: u32,
}

impl VisualRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Result<Self, PrepareError> {
        if capacity == 0 || capacity > 65_536 {
            return Err(PrepareError::Capacity);
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lagui rounded visuals"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../native/visual.wgsl").into()),
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
            bind_group_layouts: &[Some(&layout)],
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
                    array_stride: 80,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
                        4 => Float32x4
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
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("visual instances"),
            size: (capacity * 80) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            instances,
            viewport,
            binding,
            bytes: std::vec::Vec::with_capacity(capacity * 80),
            capacity,
            count: 0,
        })
    }

    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        visuals: &[Visual],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), PrepareError> {
        self.count = 0;
        self.bytes.clear();
        if visuals.len() > self.capacity {
            return Err(PrepareError::Capacity);
        }
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            return Err(PrepareError::InvalidViewport);
        }
        for visual in visuals {
            let values = [
                visual.bounds.x,
                visual.bounds.y,
                visual.bounds.width,
                visual.bounds.height,
                visual.color[0],
                visual.color[1],
                visual.color[2],
                visual.color[3],
                visual.clip.x,
                visual.clip.y,
                visual.clip.width,
                visual.clip.height,
                visual.radius,
                scale_factor,
                visual.stroke_width,
                0.,
                visual.inner_color[0],
                visual.inner_color[1],
                visual.inner_color[2],
                visual.inner_color[3],
            ];
            if values.iter().any(|v| !v.is_finite()) {
                return Err(PrepareError::InvalidViewport);
            }
            for value in values {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut screen = [0; 16];
        screen[..4].copy_from_slice(&viewport[0].to_le_bytes());
        screen[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        queue.write_buffer(&self.viewport, 0, &screen);
        if !self.bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.bytes);
        }
        self.count = visuals.len() as u32;
        Ok(())
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.count as usize);
    }

    /// Draws a prepared subrange, allowing other renderers to preserve painter's order.
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<usize>) {
        let start = range.start.min(self.count as usize);
        let end = range.end.min(self.count as usize);
        if start >= end {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, start as u32..end as u32);
    }

    pub fn prepared_len(&self) -> usize {
        self.count as usize
    }
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    viewport: wgpu::Buffer,
    binding: wgpu::BindGroup,
    bytes: [u8; CAPACITY * 32],
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
            pipeline,
            instances,
            viewport,
            binding,
            bytes: [0; CAPACITY * 32],
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
        if quads.len() > CAPACITY {
            return Err(PrepareError::Capacity);
        }
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
