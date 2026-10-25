//! The single Windfoil GPU path. Glyph runs and vector shapes are both encoded
//! as instances of this pipeline; only their outline sources differ.
use crate::{
    Rect,
    gpu::{
        Damage,
        instances::{self, CapacityError, InstanceBuffer},
    },
};
use std::{ops::Range, vec::Vec};
use wgpu::util::DeviceExt;

/// Bytes per instance: origin/scale, outline bounds, clip, band rows, band size, color.
pub(crate) const STRIDE: usize = 80;

/// Default coverage gamma: edges blend as an sRGB compositor would over a
/// contrasting background, matching design tools and native text weight.
pub const PERCEPTUAL_COVERAGE_GAMMA: f32 = 2.2;

/// Compiled Windfoil shader and layout for one render-target format.
///
/// Create it once per device/format and pass it to every text and vector
/// renderer, so adding a font or atlas never recompiles the shader and
/// consecutive draws reuse the bound pipeline.
#[derive(Clone)]
pub struct Pipeline {
    pub(crate) layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    shared: crate::gpu::Shared,
}

impl Pipeline {
    pub fn shared(&self) -> &crate::gpu::Shared {
        &self.shared
    }
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self::with_shared(device, format, &crate::gpu::Shared::new(device))
    }
    pub fn with_shared(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        shared: &crate::gpu::Shared,
    ) -> Self {
        Self::with_band_direction(device, format, shared, true)
    }
    /// Compare directional integration on the same source data and device.
    pub fn with_band_direction(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        shared: &crate::gpu::Shared,
        bidirectional: bool,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Windfoil analytic coverage"),
            source: wgpu::ShaderSource::Wgsl(
                std::format!(
                    "{}{}",
                    include_str!("../../native/transforms.wgsl"),
                    include_str!("../../native/windfoil.wgsl")
                )
                .into(),
            ),
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
                    min_binding_size: wgpu::BufferSize::new([16, 32, 28][i]),
                },
                count: None,
            });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Windfoil layout"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Windfoil pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(&shared.transforms)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Windfoil pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: STRIDE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4,
                        3 => Uint32x2, 4 => Float32x2, 5 => Float16x4, 6 => Uint32, 7 => Float32
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("bidirectional", if bidirectional { 1. } else { 0. })],
                    ..Default::default()
                },
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
        Self {
            layout,
            pipeline,
            shared: shared.clone(),
        }
    }
}

/// One outline instance in logical pixels. `bounds` and band data address the
/// uploaded outline; `origin + local * scale` maps outline units to the screen.
pub(crate) struct Placement {
    pub origin: [f32; 2],
    pub scale: [f32; 2],
    pub bounds: [f32; 4],
    pub band_start: u32,
    pub bands: u32,
    pub band_h: f32,
    pub inv_h: f32,
    pub color: [f32; 4],
    pub clip: Rect,
    pub transform: crate::gpu::TransformId,
    pub baseline: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreError {
    Capacity,
    InvalidGeometry,
}

impl From<CapacityError> for CoreError {
    fn from(_: CapacityError) -> Self {
        Self::Capacity
    }
}

/// Retained instance storage plus one uploaded outline source.
pub(crate) struct Analytic {
    pipeline: Pipeline,
    viewport: wgpu::Buffer,
    binding: wgpu::BindGroup,
    instances: InstanceBuffer,
    /// (source id, curve count, row count) of the uploaded outline buffers.
    uploaded: (u64, usize, usize),
    atlas_pending_bytes: u64,
    upload_stats: crate::gpu::stats::UploadStats,
    coverage_gamma: f32,
    transform_binding: wgpu::BindGroup,
    transformed: bool,
}

fn storage(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn bind(
    device: &wgpu::Device,
    pipeline: &Pipeline,
    viewport: &wgpu::Buffer,
    curves: &wgpu::Buffer,
    rows: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Windfoil outline data"),
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: curves.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: rows.as_entire_binding(),
            },
        ],
    })
}

impl Analytic {
    pub fn new(
        device: &wgpu::Device,
        pipeline: &Pipeline,
        label: &'static str,
        capacity: usize,
    ) -> Result<Self, CapacityError> {
        if capacity == 0 {
            return Err(CapacityError);
        }
        let instances = InstanceBuffer::new(device, label, STRIDE, capacity)?;
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Windfoil viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let curves = storage(device, "empty Windfoil curves", &[0; 32]);
        let rows = storage(device, "empty Windfoil rows", &[0; 28]);
        let binding = bind(device, pipeline, &viewport, &curves, &rows);
        Ok(Self {
            pipeline: pipeline.clone(),
            transform_binding: pipeline.shared.identity.clone(),
            transformed: false,
            viewport,
            binding,
            instances,
            uploaded: (0, 0, 0),
            atlas_pending_bytes: 0,
            upload_stats: Default::default(),
            coverage_gamma: PERCEPTUAL_COVERAGE_GAMMA,
        })
    }

    pub fn reserve(&mut self, required: usize) -> Result<(), CapacityError> {
        self.instances.reserve(required)
    }

    /// Takes effect on the next prepare. Values at or below 1 keep linear coverage.
    pub fn set_coverage_gamma(&mut self, gamma: f32) {
        self.coverage_gamma = if gamma.is_finite() { gamma.max(1.) } else { 1. };
    }

    pub fn capacity(&self) -> usize {
        self.instances.capacity()
    }

    pub fn uploaded(&self) -> (u64, usize, usize) {
        self.uploaded
    }

    /// Replaces the immutable outline buffers when the source revision changed.
    /// Submitted frames retain their previous buffers through wgpu lifetimes.
    pub fn sync(
        &mut self,
        device: &wgpu::Device,
        id: u64,
        curves: &[[[f32; 2]; 3]],
        rows: &[[u32; 5]],
    ) -> Result<(), CapacityError> {
        let revision = (id, curves.len(), rows.len());
        if revision == self.uploaded {
            return Ok(());
        }
        let limit = device
            .limits()
            .max_buffer_size
            .min(device.limits().max_storage_buffer_binding_size);
        if curves.len().max(1) as u64 > limit / 32 || rows.len().max(1) as u64 > limit / 20 {
            return Err(CapacityError);
        }
        let mut curve_bytes = Vec::with_capacity(curves.len().max(1) * 32);
        for curve in curves {
            for point in curve {
                instances::push_f32(&mut curve_bytes, point);
            }
            curve_bytes.extend_from_slice(&[0; 8]);
        }
        curve_bytes.resize(curve_bytes.len().max(32), 0);
        // Keep the existing right-hand run contiguous. A left-hand run only
        // needs indices, so curve control points are never duplicated here.
        let mut packed_rows = Vec::with_capacity(rows.len() * 7);
        let mut indices = Vec::new();
        for row in rows {
            let start = row[0] as usize;
            let count = row[1] as usize;
            let left_start = rows
                .len()
                .checked_mul(7)
                .and_then(|v| v.checked_add(indices.len()))
                .and_then(|v| u32::try_from(v).ok())
                .ok_or(CapacityError)?;
            let mut order: Vec<usize> = (start..start + count).collect();
            order.sort_by(|&a, &b| {
                let a = curves[a];
                let b = curves[b];
                a[0][0].min(a[2][0]).total_cmp(&b[0][0].min(b[2][0]))
            });
            let split = if count > 4 {
                let mut centers: Vec<f32> = curves[start..start + count]
                    .iter()
                    .map(|q| (q[0][0] + q[2][0]) * 0.5)
                    .collect();
                centers.sort_by(f32::total_cmp);
                indices.extend(order.into_iter().map(|v| v as u32));
                centers[count / 2]
            } else {
                f32::NEG_INFINITY
            };
            packed_rows.extend_from_slice(row);
            packed_rows.extend_from_slice(&[left_start, split.to_bits()]);
        }
        packed_rows.extend(indices);
        if packed_rows.len().max(7) as u64 > limit / 4 {
            return Err(CapacityError);
        }
        let mut row_bytes = Vec::with_capacity(packed_rows.len().max(7) * 4);
        instances::push_u32(&mut row_bytes, &packed_rows);
        row_bytes.resize(row_bytes.len().max(28), 0);
        let curves = storage(device, "Windfoil curves", &curve_bytes);
        let rows = storage(device, "Windfoil rows", &row_bytes);
        self.binding = bind(device, &self.pipeline, &self.viewport, &curves, &rows);
        self.uploaded = revision;
        self.atlas_pending_bytes += (curve_bytes.len() + row_bytes.len()) as u64;
        self.instances.abandon();
        Ok(())
    }

    pub fn begin(&mut self) {
        self.instances.begin();
    }

    pub fn push(&mut self, placement: &Placement) -> Result<(), CoreError> {
        let p = placement;
        let floats = [
            p.origin[0],
            p.origin[1],
            p.scale[0],
            p.scale[1],
            p.bounds[0],
            p.bounds[1],
            p.bounds[2],
            p.bounds[3],
            p.clip.x,
            p.clip.y,
            p.clip.width,
            p.clip.height,
        ];
        if floats
            .iter()
            .chain(&p.color)
            .chain(&[p.band_h, p.inv_h])
            .any(|v| !v.is_finite())
            || p.scale[0] == 0.
            || p.scale[1] == 0.
        {
            return Err(CoreError::InvalidGeometry);
        }
        if self.instances.staged_len() >= self.instances.capacity() {
            return Err(CoreError::Capacity);
        }
        let bytes = self.instances.staged();
        instances::push_f32(bytes, &floats);
        instances::push_u32(
            bytes,
            &[
                p.band_start,
                p.bands | if p.baseline.is_some() { 0x8000_0000 } else { 0 },
            ],
        );
        instances::push_f32(bytes, &[p.band_h, p.inv_h]);
        instances::push_f16x4(bytes, p.color);
        instances::push_u32(bytes, &[p.transform.0]);
        instances::push_f32(bytes, &[p.baseline.unwrap_or(0.)]);
        Ok(())
    }

    pub fn binding(&self) -> &wgpu::BindGroup {
        &self.binding
    }

    pub fn encoded(&self) -> &[u8] {
        self.instances.encoded()
    }
    pub fn append_encoded(&mut self, bytes: &[u8], band_offset: u32) -> Result<(), CoreError> {
        for instance in bytes.chunks_exact(STRIDE) {
            let start = self.instances.staged().len();
            self.instances.staged().extend_from_slice(instance);
            let offset = u32::from_le_bytes(instance[48..52].try_into().unwrap())
                .checked_add(band_offset)
                .ok_or(CoreError::Capacity)?;
            self.instances.staged()[start + 48..start + 52].copy_from_slice(&offset.to_le_bytes());
        }
        Ok(())
    }

    pub fn abandon(&mut self) {
        self.instances.abandon();
    }

    pub fn commit_transformed(
        &mut self,
        queue: &wgpu::Queue,
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: Option<&crate::gpu::Transforms>,
    ) {
        if self.transformed != transforms.is_some()
            || transforms.is_some_and(|t| t.previous(crate::gpu::TransformId::IDENTITY).is_none())
        {
            // Preserve staged instances while invalidating only residency.
            self.instances.invalidate();
        }
        self.transformed = transforms.is_some();
        self.transform_binding = transforms.map_or_else(
            || self.pipeline.shared.identity.clone(),
            |t| t.binding.clone(),
        );
        let mut uniforms = [0; 16];
        uniforms[..4].copy_from_slice(&viewport[0].to_le_bytes());
        uniforms[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        uniforms[8..12].copy_from_slice(&scale_factor.to_le_bytes());
        uniforms[12..16].copy_from_slice(&self.coverage_gamma.to_le_bytes());
        let margin = 1. / scale_factor;
        self.instances
            .commit_bounds(queue, &self.viewport, uniforms, |bytes, old| {
                let id =
                    crate::gpu::TransformId(u32::from_le_bytes(bytes[72..76].try_into().unwrap()));
                let t = transforms
                    .and_then(|t| if old { t.previous(id) } else { t.get(id) })
                    .unwrap_or_default();
                damage_transformed(
                    bytes,
                    margin,
                    t,
                    scale_factor,
                    transforms.map(|t| {
                        t.clip(
                            id,
                            Rect::new(
                                instances::read_f32(bytes, 8),
                                instances::read_f32(bytes, 9),
                                instances::read_f32(bytes, 10),
                                instances::read_f32(bytes, 11),
                            ),
                            old,
                        )
                    }),
                )
            });
        self.upload_stats = self.instances.stats();
        self.upload_stats.bytes += self.atlas_pending_bytes;
        self.atlas_pending_bytes = 0;
    }

    pub fn upload_stats(&self) -> crate::gpu::stats::UploadStats {
        self.upload_stats
    }

    pub fn damage(&self) -> Damage {
        self.instances.damage()
    }

    pub fn count(&self) -> u32 {
        self.instances.count()
    }

    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        let count = self.instances.count();
        let start = range.start.min(count);
        let end = range.end.min(count);
        if start >= end {
            return;
        }
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_bind_group(1, &self.transform_binding, &[]);
        pass.set_vertex_buffer(0, self.instances.buffer().slice(..));
        pass.draw(0..6, start..end);
    }
}

/// Logical region one encoded instance can touch. Mirrors `vertex_main`.
fn damage_transformed(
    bytes: &[u8],
    margin: f32,
    t: crate::gpu::Transform2D,
    dpi: f32,
    clip: Option<Option<Rect>>,
) -> Option<Rect> {
    let f = |index| instances::read_f32(bytes, index);
    let origin = t.point([f(0), f(1)]);
    let snap = if u32::from_le_bytes(bytes[52..56].try_into().unwrap()) & 0x8000_0000 != 0 {
        let baseline = f(19) * t.scale + t.offset[1];
        (baseline * dpi).round() / dpi - baseline
    } else {
        0.
    };
    let x = [
        origin[0] + f(4) * f(2) * t.scale,
        origin[0] + f(6) * f(2) * t.scale,
    ];
    let y = [
        origin[1] + snap + f(5) * f(3) * t.scale,
        origin[1] + snap + f(7) * f(3) * t.scale,
    ];
    let left = x[0].min(x[1]) - margin;
    let top = y[0].min(y[1]) - margin;
    Rect::new(
        left,
        top,
        (x[1] - x[0]).abs() + 2. * margin,
        (y[1] - y[0]).abs() + 2. * margin,
    )
    .intersection(match clip {
        Some(r) => r?,
        None => Rect::new(f(8), f(9), f(10), f(11)),
    })
}
