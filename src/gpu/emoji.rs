use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};
use std::{borrow::ToOwned, collections::HashMap, string::String, vec, vec::Vec};

pub const RASTER_SIZE: u32 = 256;
const LAYERS: u32 = 64;
const CAPACITY: usize = 256;

#[derive(Clone, Copy)]
pub struct EmojiQuad {
    pub bounds: [f32; 4],
    pub clip: [f32; 4],
    pub layer: u32,
}

#[derive(Clone, Copy)]
pub struct CachedEmoji {
    pub layer: u32,
    pub advance: f32,
}

pub struct Renderer {
    texture: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    binding: wgpu::BindGroup,
    viewport: wgpu::Buffer,
    instances: wgpu::Buffer,
    bytes: Vec<u8>,
    count: u32,
    cached: HashMap<String, CachedEmoji>,
    next_layer: u32,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fixed 256px emoji atlas"),
            size: wgpu::Extent3d {
                width: RASTER_SIZE,
                height: RASTER_SIZE,
                depth_or_array_layers: LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emoji viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fixed emoji layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fixed emoji binding"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: viewport.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fixed emoji shader"),
            source: wgpu::ShaderSource::Wgsl(crate::gpu::shaders::EMOJI.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fixed emoji pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fixed emoji pipeline"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 48, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Uint32] })] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fragment_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: None,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emoji quads"),
            size: (CAPACITY * 48) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            texture,
            pipeline,
            binding,
            viewport,
            instances,
            bytes: Vec::with_capacity(CAPACITY * 48),
            count: 0,
            cached: HashMap::new(),
            next_layer: 0,
        }
    }

    pub fn cache(
        &mut self,
        queue: &wgpu::Queue,
        fonts: &mut FontSystem,
        swash: &mut SwashCache,
        text: &str,
    ) -> Option<CachedEmoji> {
        if let Some(entry) = self.cached.get(text) {
            return Some(*entry);
        }
        if self.next_layer >= LAYERS {
            return None;
        }
        let mut buffer = Buffer::new(fonts, Metrics::new(RASTER_SIZE as f32, RASTER_SIZE as f32));
        buffer.set_size(None, Some(RASTER_SIZE as f32));
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        let mut pixels = vec![0u8; (RASTER_SIZE * RASTER_SIZE * 4) as usize];
        let mut advance = 1.;
        for run in buffer.layout_runs() {
            advance = run.line_w / RASTER_SIZE as f32;
            for glyph in run.glyphs {
                let physical = glyph.physical((0., 0.), 1.);
                let Some(image) = swash.get_image_uncached(fonts, physical.cache_key) else {
                    continue;
                };
                let ox = physical.x + image.placement.left;
                let oy = (run.line_y.round() as i32 + physical.y) - image.placement.top;
                for y in 0..image.placement.height as i32 {
                    for x in 0..image.placement.width as i32 {
                        let dx = ox + x;
                        let dy = oy + y;
                        if !(0..RASTER_SIZE as i32).contains(&dx)
                            || !(0..RASTER_SIZE as i32).contains(&dy)
                        {
                            continue;
                        }
                        let dst = ((dy as u32 * RASTER_SIZE + dx as u32) * 4) as usize;
                        match image.content {
                            SwashContent::Color => pixels[dst..dst + 4].copy_from_slice(
                                &image.data[((y as u32 * image.placement.width + x as u32) * 4)
                                    as usize..][..4],
                            ),
                            _ => {
                                let a = image.data
                                    [(y as u32 * image.placement.width + x as u32) as usize];
                                pixels[dst..dst + 4].copy_from_slice(&[255, 255, 255, a]);
                            }
                        }
                    }
                }
            }
        }
        let layer = self.next_layer;
        self.next_layer += 1;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(RASTER_SIZE * 4),
                rows_per_image: Some(RASTER_SIZE),
            },
            wgpu::Extent3d {
                width: RASTER_SIZE,
                height: RASTER_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let entry = CachedEmoji { layer, advance };
        self.cached.insert(text.to_owned(), entry);
        Some(entry)
    }

    pub fn prepare(&mut self, queue: &wgpu::Queue, quads: &[EmojiQuad], viewport: [f32; 2]) {
        self.bytes.clear();
        for q in quads.iter().take(CAPACITY) {
            for v in q.bounds.into_iter().chain(q.clip) {
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
            self.bytes.extend_from_slice(&q.layer.to_le_bytes());
            self.bytes.extend_from_slice(&[0; 12]);
        }
        let mut screen = [0; 16];
        screen[..4].copy_from_slice(&viewport[0].to_le_bytes());
        screen[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        queue.write_buffer(&self.viewport, 0, &screen);
        if !self.bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.bytes);
        }
        self.count = quads.len().min(CAPACITY) as u32;
    }

    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>) {
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
}
