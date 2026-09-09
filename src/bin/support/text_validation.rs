//! Deterministic GPU checks using original rectangle/hole font geometry.
use best_gui::text::{Font, TextError, TextRenderer, TextSection};
use std::error::Error;

pub fn run(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), Box<dyn Error>> {
    let bytes = include_bytes!("../../../tests/assets/test-outline.ttf");
    let mut font = Font::new(bytes.to_vec(), 0)?;
    let run = font.shape("O")?;
    let mut renderer = TextRenderer::new(device, wgpu::TextureFormat::Rgba8Unorm, 2)?;
    renderer.sync_font(device, &font);
    for dpi in [1u32, 2] {
        let sections = [
            TextSection {
                run: &run,
                baseline: [16.25, 100.25],
                size: 96.,
                color: [1.; 4],
                clip: None,
            },
            TextSection {
                run: &run,
                baseline: [140.3, 40.3],
                size: 12.,
                color: [1.; 4],
                clip: None,
            },
        ];
        renderer.prepare(queue, &sections, [256., 128.], dpi as f32)?;
        let size = wgpu::Extent3d {
            width: 256 * dpi,
            height: 128 * dpi,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text regression target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text regression readback"),
            size: (size.width * size.height * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text regression pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
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
                    bytes_per_row: Some(size.width * 4),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let pixels = readback.slice(..).get_mapped_range()?;
        let alpha = |x: u32, y: u32| pixels[((y * dpi * size.width + x * dpi) * 4 + 3) as usize];
        for (name, x, y, filled) in [
            ("large stroke", 30, 50, true),
            ("large hole", 64, 52, false),
            ("outside", 10, 50, false),
            ("small stroke", 142, 34, true),
            ("small hole", 146, 34, false),
        ] {
            let a = alpha(x, y);
            if (filled && a < 240) || (!filled && a > 2) {
                return Err(format!("text {name} at {dpi}x: unexpected coverage {a}").into());
            }
        }
        let partial = pixels
            .chunks_exact(4)
            .filter(|p| p[3] > 2 && p[3] < 253)
            .count();
        if partial < 20 {
            return Err("missing text antialiasing".into());
        }
        println!(
            "Windfoil regression {dpi}x: small/large strokes, holes, outside and {partial} AA pixels verified"
        );
        drop(pixels);
        readback.unmap();
    }
    let extra = font.shape("f")?;
    let section = TextSection {
        run: &extra,
        baseline: [0., 20.],
        size: 16.,
        color: [1.; 4],
        clip: None,
    };
    assert!(matches!(
        renderer.prepare(queue, &[section], [256., 128.], 1.),
        Err(TextError::AtlasOutOfDate)
    ));
    let too_many = font.shape("OOO")?;
    assert!(matches!(
        renderer.prepare(
            queue,
            &[TextSection {
                run: &too_many,
                baseline: [0., 20.],
                size: 16.,
                color: [1.; 4],
                clip: None,
            }],
            [256., 128.],
            1.
        ),
        Err(TextError::Capacity)
    ));
    let other = Font::new(bytes.to_vec(), 0)?.shape("O")?;
    assert!(matches!(
        renderer.prepare(
            queue,
            &[TextSection {
                run: &other,
                baseline: [0., 20.],
                size: 16.,
                color: [1.; 4],
                clip: None,
            }],
            [256., 128.],
            1.
        ),
        Err(TextError::WrongFont)
    ));
    println!("Windfoil regression: stale atlas, wrong font and capacity errors verified");
    Ok(())
}
