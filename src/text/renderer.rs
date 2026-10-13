use super::{Font, TextError, TextRun};
use crate::Rect;
use std::ops::Range;
use std::vec::Vec;
use wgpu::util::DeviceExt;

/// Logical pixel coordinates; baseline is the left/baseline origin of the run.
/// Colors are linear premultiplied RGBA. Runs may be reused at arbitrary sizes.
pub struct TextSection<'a> {
    pub run: &'a TextRun,
    pub baseline: [f32; 2],
    pub size: f32,
    pub color: [f32; 4],
    /// Optional scissor rectangle in logical pixels.
    pub clip: Option<Rect>,
}

pub struct TextRenderer {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    viewport: wgpu::Buffer,
    instances: wgpu::Buffer,
    binding: wgpu::BindGroup,
    bytes: Vec<u8>,
    capacity: usize,
    count: u32,
    uploaded: (u64, usize, usize),
}
fn storage(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}
fn binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    curves: &wgpu::Buffer,
    bands: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Windfoil data"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: curves.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: bands.as_entire_binding(),
            },
        ],
    })
}
impl TextRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        max_glyphs: usize,
    ) -> Result<Self, TextError> {
        if max_glyphs == 0
            || max_glyphs as u64
                > (device.limits().max_buffer_size / 96)
                    .min(u64::from(u32::MAX))
                    .min((usize::MAX / 96) as u64)
        {
            return Err(TextError::Capacity);
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Windfoil analytic text"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../native/windfoil.wgsl").into()),
        });
        let entries: [wgpu::BindGroupLayoutEntry; 3] =
            core::array::from_fn(|i| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: if i == 0 {
                    wgpu::ShaderStages::VERTEX_FRAGMENT
                } else {
                    wgpu::ShaderStages::FRAGMENT
                },
                ty: wgpu::BindingType::Buffer {
                    ty: if i == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage { read_only: true }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new([16, 32, 20][i]),
                },
                count: None,
            });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Windfoil layout"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Windfoil pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("Windfoil pipeline"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vertex_main"),compilation_options:Default::default(),
                buffers:&[Some(wgpu::VertexBufferLayout {array_stride:96,step_mode:wgpu::VertexStepMode::Instance,
                    attributes:&wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4]})]},
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fragment_main"),compilation_options:Default::default(),
                targets:&[Some(wgpu::ColorTargetState {format,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),
            primitive:Default::default(),depth_stencil:None,multisample:Default::default(),multiview_mask:None,cache:None,
        });
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text instances"),
            size: (max_glyphs * 96) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let curves = storage(device, "empty curves", &[0; 32]);
        let bands = storage(device, "empty bands", &[0; 20]);
        let binding = binding(device, &layout, &viewport, &curves, &bands);
        Ok(Self {
            device: device.clone(),
            pipeline,
            layout,
            viewport,
            instances,
            binding,
            bytes: Vec::with_capacity(max_glyphs * 96),
            capacity: max_glyphs,
            count: 0,
            uploaded: (0, 0, 0),
        })
    }

    /// Grow instance storage while preserving the compiled pipeline and atlas.
    /// Submitted frames retain their old buffer through wgpu's resource lifetime.
    pub fn reserve(&mut self, device: &wgpu::Device, required: usize) -> Result<(), TextError> {
        if required <= self.capacity {
            return Ok(());
        }
        let limit = (device.limits().max_buffer_size / 96)
            .min(u64::from(u32::MAX))
            .min((usize::MAX / 96) as u64) as usize;
        if required > limit {
            return Err(TextError::Capacity);
        }
        let capacity = required
            .checked_next_power_of_two()
            .unwrap_or(limit)
            .min(limit);
        self.instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grown analytic instances"),
            size: (capacity * 96) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bytes.reserve(capacity * 96 - self.bytes.len());
        self.capacity = capacity;
        self.count = 0;
        Ok(())
    }

    /// Upload new outlines only after shaping has introduced new glyphs.
    /// Replaces the immutable atlas buffers; existing submitted frames retain theirs.
    pub fn sync_font(&mut self, device: &wgpu::Device, font: &Font) -> Result<(), TextError> {
        let revision = (font.id, font.curves.len(), font.bands.len());
        if revision == self.uploaded {
            return Ok(());
        }
        for (count, stride) in [
            (font.curves.len().max(1), 32usize),
            (font.bands.len().max(1), 20usize),
        ] {
            let bytes = count.checked_mul(stride).ok_or(TextError::Capacity)? as u64;
            if bytes > device.limits().max_buffer_size
                || bytes > device.limits().max_storage_buffer_binding_size
            {
                return Err(TextError::Capacity);
            }
        }
        let mut curve_bytes = Vec::with_capacity(font.curves.len() * 32);
        for c in &font.curves {
            for p in c {
                for v in p {
                    curve_bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
            curve_bytes.extend_from_slice(&[0; 8]);
        }
        if curve_bytes.is_empty() {
            curve_bytes.resize(32, 0);
        }
        let mut band_bytes = Vec::with_capacity(font.bands.len() * 20);
        for b in &font.bands {
            for v in b {
                band_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        if band_bytes.is_empty() {
            band_bytes.resize(20, 0);
        }
        let curves = storage(device, "glyph quadratic curves", &curve_bytes);
        let bands = storage(device, "glyph bands", &band_bytes);
        self.binding = binding(device, &self.layout, &self.viewport, &curves, &bands);
        self.uploaded = revision;
        self.count = 0;
        Ok(())
    }

    /// No shaping or font parsing here. Reuses CPU storage; wgpu's own allocation
    /// behavior is outside this guarantee. Submit before preparing the next batch.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), TextError> {
        self.count = 0;
        self.bytes.clear();
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            return Err(TextError::InvalidGeometry);
        }
        let required = sections.iter().try_fold(0usize, |count, section| {
            count
                .checked_add(section.run.glyph_count())
                .ok_or(TextError::Capacity)
        })?;
        let device = self.device.clone();
        self.reserve(&device, required)?;
        let margin = 1. / scale_factor;
        for section in sections {
            if section.run.font_id != self.uploaded.0 {
                return Err(TextError::WrongFont);
            }
            if !section.size.is_finite()
                || section.size <= 0.
                || section
                    .baseline
                    .iter()
                    .chain(section.color.iter())
                    .any(|v| !v.is_finite())
            {
                return Err(TextError::InvalidGeometry);
            }
            let clip = section
                .clip
                .unwrap_or(Rect::new(0., 0., viewport[0], viewport[1]));
            let clip_values = [clip.x, clip.y, clip.width, clip.height];
            if clip_values.iter().any(|v| !v.is_finite()) {
                return Err(TextError::InvalidGeometry);
            }
            for positioned in &section.run.glyphs {
                if self.bytes.len() / 96 >= self.capacity {
                    return Err(TextError::Capacity);
                }
                let g = positioned.glyph;
                let b = g.bounds;
                // Reject runs shaped after the currently uploaded atlas revision.
                if g.band_start as usize + g.bands as usize > self.uploaded.2 {
                    return Err(TextError::AtlasOutOfDate);
                }
                let size = section.size;
                let pad = margin / size;
                let values = [
                    section.baseline[0] + (positioned.position[0] + b[0]) * size - margin,
                    section.baseline[1] - (positioned.position[1] + b[3]) * size - margin,
                    (b[2] - b[0]) * size + 2. * margin,
                    (b[3] - b[1]) * size + 2. * margin,
                    b[0] - pad,
                    b[3] + pad,
                    b[2] - b[0] + 2. * pad,
                    -(b[3] - b[1] + 2. * pad),
                    section.color[0],
                    section.color[1],
                    section.color[2],
                    section.color[3],
                    g.band_start as f32,
                    g.bands as f32,
                    g.band_h,
                    g.inv_h,
                    b[0],
                    b[1],
                    b[2],
                    b[3],
                ];
                if values.iter().any(|v| !v.is_finite()) {
                    return Err(TextError::InvalidGeometry);
                }
                for value in values {
                    self.bytes.extend_from_slice(&value.to_le_bytes());
                }
                for value in clip_values {
                    self.bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        let mut screen = [0; 16];
        screen[..4].copy_from_slice(&viewport[0].to_le_bytes());
        screen[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        screen[8..12].copy_from_slice(&scale_factor.to_le_bytes());
        queue.write_buffer(&self.viewport, 0, &screen);
        if !self.bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.bytes);
        }
        self.count = (self.bytes.len() / 96) as u32;
        Ok(())
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.count);
    }

    /// Draws a prepared glyph subrange so text can be interleaved with other primitives.
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        let start = range.start.min(self.count);
        let end = range.end.min(self.count);
        if start >= end {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, start..end);
    }

    pub fn prepared_len(&self) -> u32 {
        self.count
    }
}
