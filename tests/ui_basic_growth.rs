#![cfg(feature = "portable")]
//! The simple renderer must draw instances beyond its former 96-quad ceiling.
mod support;
use lagui::{Quad, Rect, gpu::Renderer};

#[test]
fn basic_renderer_grows_draws_last_quad_and_reuses_after_shrink() {
    pollster::block_on(async {
        let Some((device, queue)) = support::device() else {
            return;
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("basic renderer growth"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for count in [513, 1] {
            let mut quads = vec![Quad::default(); count];
            quads[count - 1] = Quad {
                bounds: Rect::new(0., 0., 1., 1.),
                color: [0., 1., 0., 1.],
            };
            renderer.prepare(&queue, &quads, [1., 1.]).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            let view = texture.create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                renderer.draw(&mut pass);
            }
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                texture.size(),
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    tx.send(result).unwrap();
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            {
                let pixels = readback.slice(..).get_mapped_range().unwrap();
                assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
            }
            readback.unmap();
        }
        assert!(renderer.prepare(&queue, &[], [f32::NAN, 1.]).is_err());
        assert!(scope.pop().await.is_none());
    });
}
