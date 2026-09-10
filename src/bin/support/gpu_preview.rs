use lagui::gpu::GpuCanvas;

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    binding: wgpu::BindGroup,
    depth: wgpu::Texture,
    size: [u32; 2],
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lagui 3D preview"),
            source: wgpu::ShaderSource::Wgsl(lagui::gpu::shaders::PREVIEW_3D.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D preview scene layout"),
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
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D preview scene"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D preview binding"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D preview pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D preview pipeline"),
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
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniform,
            binding,
            depth: depth(device, size),
            size,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.size != size && size[0] > 0 && size[1] > 0 {
            self.depth = depth(device, size);
            self.size = size;
        }
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        queue: &wgpu::Queue,
        canvas: GpuCanvas,
        scale: f32,
        time: f32,
    ) {
        let Some(region) = canvas.physical(scale, self.size) else {
            return;
        };
        let aspect = region.viewport[2] / region.viewport[3].max(1.);
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&time.to_le_bytes());
        bytes[4..8].copy_from_slice(&aspect.to_le_bytes());
        queue.write_buffer(&self.uniform, 0, &bytes);
        let depth_view = self.depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("embedded 3D GPU canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if canvas.apply(&mut pass, scale, self.size) {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.binding, &[]);
            pass.draw(0..36, 0..1);
        }
    }
}

fn depth(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("3D preview depth"),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}
