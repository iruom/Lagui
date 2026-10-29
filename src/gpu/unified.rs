//! Experimental vertex-pulling UI pipeline. Compare this against split pipelines
//! on the same scene before choosing it; it is never selected implicitly.
use super::{
    PrepareError, Shared, Transforms, VisualRenderer, emoji,
    instances::{self, InstanceBuffer},
    outlines::OutlineBatch,
};
use crate::windfoil::Pipeline;
use std::{ops::Range, string::String};

pub enum Item {
    Visual(Range<u32>),
    Outline(Range<u32>),
    Emoji(Range<u32>),
}

fn helpers(source: &str, prefix: &str, name: &str) -> String {
    let mut source = source
        .replace("Vertex", name)
        .replace("struct Out ", &std::format!("struct {name} "))
        .replace("-> Out {", &std::format!("-> {name} {{"))
        .replace("return Out(", &std::format!("return {name}("))
        .replace("in: Out)", &std::format!("in: {name})"))
        .replace(
            "@vertex fn vertex_main",
            &std::format!("fn {prefix}_vertex"),
        )
        .replace(
            "@fragment fn fragment_main",
            &std::format!("fn {prefix}_fragment"),
        );
    let start = source.find(&std::format!("fn {prefix}_vertex(")).unwrap();
    let end = start + source[start..].find(") -> ").unwrap();
    let mut parameters = source[start..end].replace("@builtin(vertex_index) ", "");
    for i in 0..16 {
        parameters = parameters.replace(&std::format!("@location({i}) "), "");
    }
    source.replace_range(start..end, &parameters);
    source.replace("@location(0) vec4f", "vec4f")
}
fn shader() -> String {
    let visual = helpers(
        include_str!("../../native/visual.wgsl"),
        "visual",
        "VisualVertex",
    )
    .lines()
    .filter(|line| !line.starts_with("@group"))
    .collect::<std::vec::Vec<_>>()
    .join("\n");
    let outline = helpers(
        include_str!("../../native/windfoil.wgsl"),
        "outline",
        "OutlineVertex",
    );
    let emoji = helpers(
        include_str!("../../native/emoji.wgsl"),
        "emoji",
        "EmojiVertex",
    )
    .replace("@group(0)", "@group(2)")
    .replace("viewport", "emoji_viewport");
    std::format!(
        "diagnostic(off, derivative_uniformity);\n{}\n{}\n{outline}\n{visual}\n{emoji}\n{}",
        include_str!("../../native/transforms.wgsl"),
        super::shaders::CORNERS,
        include_str!("../../native/unified.wgsl")
    )
}

pub struct UnifiedRenderer {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    records: InstanceBuffer,
    indices: InstanceBuffer,
    uniform: wgpu::Buffer,
    binding: wgpu::BindGroup,
    outline: Option<wgpu::BindGroup>,
    emoji: Option<wgpu::BindGroup>,
    transforms: wgpu::BindGroup,
    shared: Shared,
}
impl UnifiedRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        outline: &Pipeline,
        emoji: &emoji::Renderer,
    ) -> Result<Self, PrepareError> {
        let shared = outline.shared().clone();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("unified records"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(4),
                },
                count: None,
            }),
        });
        let records = InstanceBuffer::new_storage(device, "unified records", 4, 1)
            .map_err(|_| PrepareError::Capacity)?;
        let indices = InstanceBuffer::new_storage(device, "unified indices", 4, 1)
            .map_err(|_| PrepareError::Capacity)?;
        let binding = bind(device, &layout, &records, &indices);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("experimental unified UI"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                Some(&outline.layout),
                Some(&shared.transforms),
                Some(&emoji.layout),
                Some(&layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("experimental unified UI"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("reuse_footprint", 1.)],
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
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unified diff metadata"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            device: device.clone(),
            pipeline,
            layout,
            records,
            indices,
            uniform,
            binding,
            outline: None,
            emoji: None,
            transforms: shared.identity.clone(),
            shared,
        })
    }
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        visuals: &VisualRenderer,
        outlines: &OutlineBatch,
        emoji: &emoji::Renderer,
        items: &[Item],
        transforms: Option<&Transforms>,
    ) -> Result<(), PrepareError> {
        self.records.begin();
        self.indices.begin();
        let result = (|| {
            for item in items {
                let (kind, range, source, stride) = match item {
                    Item::Visual(r) => (0, r, visuals.encoded(), 80),
                    Item::Outline(r) => (1, r, outlines.encoded(), 80),
                    Item::Emoji(r) => (2, r, emoji.encoded(), 48),
                };
                if range.end < range.start || range.end as usize > source.len() / stride {
                    return Err(PrepareError::InvalidViewport);
                }
                for instance in source[range.start as usize * stride..range.end as usize * stride]
                    .chunks_exact(stride)
                {
                    let offset = self.records.staged_len();
                    if offset >= 1 << 28 {
                        return Err(PrepareError::Capacity);
                    }
                    instances::push_u32(self.indices.staged(), &[(kind << 28) | offset as u32]);
                    self.records.staged().extend_from_slice(instance);
                }
            }
            let old = (self.records.capacity(), self.indices.capacity());
            self.records
                .reserve(self.records.staged_len())
                .map_err(|_| PrepareError::Capacity)?;
            self.indices
                .reserve(self.indices.staged_len())
                .map_err(|_| PrepareError::Capacity)?;
            if old != (self.records.capacity(), self.indices.capacity()) {
                self.binding = bind(&self.device, &self.layout, &self.records, &self.indices);
            }
            self.records
                .commit_bounds(queue, &self.uniform, [0; 16], |_, _| None);
            self.indices
                .commit_bounds(queue, &self.uniform, [0; 16], |_, _| None);
            Ok(())
        })();
        if result.is_err() {
            self.records.abandon();
            self.indices.abandon();
            return result;
        }
        self.outline = Some(outlines.binding().clone());
        self.emoji = Some(emoji.binding.clone());
        self.transforms =
            transforms.map_or_else(|| self.shared.identity.clone(), |t| t.binding.clone());
        Ok(())
    }
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        let (Some(outline), Some(emoji)) = (&self.outline, &self.emoji) else {
            return;
        };
        let start = range.start.min(self.indices.count());
        let end = range.end.min(self.indices.count());
        if start >= end {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, outline, &[]);
        pass.set_bind_group(1, &self.transforms, &[]);
        pass.set_bind_group(2, emoji, &[]);
        pass.set_bind_group(3, &self.binding, &[]);
        pass.draw(0..6, start..end);
    }
    pub fn prepared_len(&self) -> u32 {
        self.indices.count()
    }
    pub fn upload_stats(&self) -> super::stats::UploadStats {
        let a = self.records.stats();
        let b = self.indices.stats();
        super::stats::UploadStats {
            bytes: a.bytes + b.bytes,
            writes: a.writes + b.writes,
            instances: b.instances,
            diff_upload_us: a.diff_upload_us + b.diff_upload_us,
        }
    }
}
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    records: &InstanceBuffer,
    indices: &InstanceBuffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("unified records"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: records.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: indices.buffer().as_entire_binding(),
            },
        ],
    })
}
