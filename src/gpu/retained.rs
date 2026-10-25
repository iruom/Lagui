//! Persistent color target. A partial frame must clear its scissor before
//! replaying painter order; Load alone would accumulate translucent layers.
use super::{Damage, GpuCanvas};
use crate::Rect;

pub struct RetainedTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    clear: [wgpu::RenderPipeline; 2],
    binding: wgpu::BindGroup,
    valid: bool,
}
impl RetainedTarget {
    pub fn new(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Lagui retained color"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lagui retained blit"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array<vec2f,3>(vec2f(-1,-1),vec2f(3,-1),vec2f(-1,3));
    return vec4f(p[i],0,1);
}
@fragment fn transparent() -> @location(0) vec4f { return vec4f(0); }
@fragment fn black() -> @location(0) vec4f { return vec4f(0,0,0,1); }
@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return textureLoad(source, vec2i(p.xy), 0);
}
"#,
            )),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Lagui retained blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make_pipeline = |entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Lagui retained blit"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline("fs");
        let clear_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let clear_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(r#"
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4f { let p=array<vec2f,3>(vec2f(-1,-1),vec2f(3,-1),vec2f(-1,3)); return vec4f(p[i],0,1); }
@fragment fn transparent()->@location(0) vec4f { return vec4f(0); }
@fragment fn black()->@location(0) vec4f { return vec4f(0,0,0,1); }
"#)) });
        let clear = ["transparent", "black"].map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Lagui damage clear"),
                layout: Some(&clear_layout),
                vertex: wgpu::VertexState {
                    module: &clear_shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &clear_shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Self {
            texture,
            view,
            pipeline,
            clear,
            binding,
            valid: false,
        }
    }
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
    pub fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }
    pub fn invalidate(&mut self) {
        self.valid = false;
    }
    pub fn damage(&self, damage: Damage) -> Damage {
        if self.valid { damage } else { Damage::Full }
    }
    pub fn mark_rendered(&mut self) {
        self.valid = true;
    }
    pub fn scissor(&self, damage: Damage, density: f32) -> Option<[u32; 4]> {
        let screen = Rect::new(
            0.,
            0.,
            self.texture.width() as f32 / density,
            self.texture.height() as f32 / density,
        );
        let rect = match self.damage(damage) {
            Damage::None => return None,
            Damage::Region(r) => r,
            Damage::Full => screen,
        };
        GpuCanvas::new(rect, screen)
            .physical(density, self.size())
            .map(|p| p.scissor)
    }
    pub fn clear_region(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scissor: [u32; 4],
        opaque: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lagui damage background"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
        pass.set_pipeline(&self.clear[usize::from(opaque)]);
        // This entry point does not read the retained texture (which is also the attachment).
        pass.draw(0..3, 0..1);
    }
    pub fn blit(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lagui retained present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Expand damage through backdrop readers until no dependency adds a region.
/// Call with each reader's output bounds and its sampling footprint (including blur).
pub fn backdrop_damage(mut damage: Damage, readers: &[(Rect, Rect)]) -> Damage {
    loop {
        let previous = damage;
        for &(output, footprint) in readers {
            if matches!(damage, Damage::Region(r) if r.intersects(footprint)) {
                damage = damage.union(Damage::Region(output));
            }
        }
        if damage == previous {
            return damage;
        }
    }
}
