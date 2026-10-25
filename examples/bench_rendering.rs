//! Offscreen renderer measurements. Run once per backend with LAGUI_BACKEND.
use lagui::{
    Rect,
    gpu::{Shared, VisualRenderer, stats::GpuTimer},
    text::{Cull, Font, TextRenderer, TextSection},
    visual::{Visual, VisualList},
    windfoil::{Atlas, Instance, Path, Pipeline, Renderer},
};
use std::time::Instant;

fn percentiles(values: &mut [f64]) -> [f64; 4] {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return [0.; 4];
    }
    [0.50, 0.95, 0.99, 1.].map(|p| values[((values.len() - 1) as f64 * p).round() as usize])
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    pollster::block_on(run())
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let backend = std::env::var("LAGUI_BACKEND").unwrap_or_else(|_| "dx12".into());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: match backend.as_str() {
            "dx12" => wgpu::Backends::DX12,
            "vulkan" => wgpu::Backends::VULKAN,
            "metal" => wgpu::Backends::METAL,
            _ => return Err("unsupported LAGUI_BACKEND".into()),
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await?;
    let features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        })
        .await?;
    let info = adapter.get_info();
    println!(
        "{{\"environment\":{{\"backend\":\"{:?}\",\"gpu\":\"{}\",\"driver\":\"{}\",\"driver_info\":\"{}\",\"os\":\"{}\",\"size\":[1024,768],\"dpi\":1,\"present_mode\":\"offscreen\",\"timestamps\":{}}}}}",
        info.backend,
        info.name,
        info.driver,
        info.driver_info,
        std::env::consts::OS,
        !features.is_empty()
    );
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render benchmark"),
        size: wgpu::Extent3d {
            width: 1024,
            height: 768,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut font = Font::new(
        include_bytes!("../tests/assets/test-outline.ttf").to_vec(),
        0,
    )?;
    let long = font.shape(&"O".repeat(10000))?;
    let label = font.shape("OOOO")?;
    let short_label = font.shape("OO")?;
    let shared = Shared::new(&device);
    for directional in [false, true] {
        let pipeline = Pipeline::with_band_direction(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            &shared,
            directional,
        );
        for unified in [false, true] {
            for scene in ["text", "controls", "graph", "mixed", "list"] {
                if std::env::var("LAGUI_BENCH_SCENE").is_ok_and(|filter| filter != scene) {
                    continue;
                }
                let motion = std::env::var("LAGUI_BENCH_MOTION").unwrap_or_else(|_| {
                    if scene == "list" {
                        "scroll".into()
                    } else {
                        "static".into()
                    }
                });
                let mut transforms = lagui::gpu::Transforms::new(&device, &shared);
                let mut batch = lagui::gpu::outlines::OutlineBatch::new(&device, &pipeline)?;
                let mut emoji =
                    lagui::gpu::emoji::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
                let mut font_system =
                    glyphon::FontSystem::new_with_fonts([glyphon::fontdb::Source::Binary(
                        std::sync::Arc::new(
                            include_bytes!("../tests/assets/test-outline.ttf").to_vec(),
                        ),
                    )]);
                let mut swash = glyphon::SwashCache::new();
                let emoji_layer = emoji
                    .cache(&queue, &mut font_system, &mut swash, "O")?
                    .ok_or("benchmark raster glyph missing")?
                    .layer;
                let mut pulled = lagui::gpu::unified::UnifiedRenderer::new(
                    &device,
                    wgpu::TextureFormat::Rgba8Unorm,
                    &pipeline,
                    &emoji,
                )?;
                let mut text = TextRenderer::with_pipeline(&device, &pipeline, 1)?;
                text.sync_font(&device, &font)?;
                let mut visuals = VisualRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1)?;
                let mut atlas = Atlas::new();
                let mut path = Path::new();
                path.move_to([0., 0.])
                    .cubic_to([30., 0.], [50., 50.], [80., 50.]);
                let shape = atlas.add_stroke(&path, 2.)?;
                let mut vectors = Renderer::with_pipeline(&device, &pipeline, 1)?;
                vectors.sync_atlas(&device, &atlas)?;
                let mut timer = GpuTimer::new(&device, &queue, 2, 1);
                let mut gpu = Vec::new();
                let mut cpu = Vec::new();
                let mut bytes = 0u64;
                let mut writes = 0u64;
                let mut storage = Vec::new();
                let mut items = Vec::new();
                for frame in 0..240 {
                    let start = Instant::now();
                    let mut sections = Vec::new();
                    storage.clear();
                    items.clear();
                    let offset = (frame % 60) as f32;
                    transforms.begin();
                    let transform = lagui::gpu::Transform2D {
                        offset: if motion == "pan" {
                            [offset, offset * 0.5]
                        } else {
                            [0.; 2]
                        },
                        scale: if motion == "zoom" {
                            0.75 + offset / 120.
                        } else {
                            1.
                        },
                    };
                    let layer = transforms.push(transform)?;
                    transforms.commit(&queue);
                    let mut emoji_quads = Vec::new();
                    let clip = Rect::new(0., 0., 1024., 768.);
                    let rows = if scene == "text" {
                        1
                    } else if scene == "graph" {
                        1000
                    } else if scene == "list" {
                        10000
                    } else {
                        500
                    };
                    storage.resize(rows, Visual::default());
                    let mut list = VisualList::new(&mut storage);
                    for i in 0..rows {
                        let x = (i % 20) as f32 * 50.
                            + if motion == "drag" && i == 0 {
                                offset
                            } else {
                                0.
                            };
                        let y =
                            (i / 20) as f32 * 28. - if motion == "scroll" { offset } else { 0. };
                        list.with_transform(layer, |list| {
                            list.fill(
                                Rect::new(x, y, 48., 24.),
                                if motion == "hover" && i == frame % rows {
                                    [0.2, 0.2, 0.2, 1.]
                                } else {
                                    [0.1, 0.1, 0.1, 1.]
                                },
                                3.,
                                clip,
                            )
                        })?;
                        if scene == "mixed" {
                            emoji_quads.push(lagui::gpu::emoji::EmojiQuad {
                                bounds: [x + 32., y + 2., 16., 16.],
                                clip: [0., 0., 1024., 768.],
                                layer: emoji_layer,
                            });
                        }
                        sections.push(TextSection {
                            run: if scene == "text" {
                                &long
                            } else if motion == "typing" && frame % 40 < 20 {
                                &short_label
                            } else {
                                &label
                            },
                            baseline: [x + 2., y + 18.],
                            size: 12.,
                            color: [1.; 4],
                            clip: Some(clip),
                            cull: if motion == "pan" || motion == "zoom" {
                                Cull::Guard { margin: 1024. }
                            } else {
                                Cull::Clip
                            },
                            transform: layer,
                        });
                        if scene == "graph" {
                            items.push(Instance {
                                shape,
                                origin: [x, y],
                                scale: [1.; 2],
                                color: [1.; 4],
                                clip: Some(clip),
                                transform: layer,
                            });
                        }
                    }
                    visuals.prepare_transformed(
                        &queue,
                        list.commands(),
                        [1024., 768.],
                        1.,
                        &transforms,
                    )?;
                    emoji.prepare(&queue, &emoji_quads, [1024., 768.])?;
                    let ranges = if unified {
                        text.prepare_cpu(&font, &sections, [1024., 768.], 1., Some(&transforms))?
                    } else {
                        text.prepare_transformed(&queue, &sections, [1024., 768.], 1., &transforms)?
                    }
                    .to_vec();
                    if unified {
                        vectors.prepare_cpu(&atlas, &items, [1024., 768.], 1.)?;
                        let sources = [
                            text.prepared_source(&font)?,
                            vectors.prepared_source(&atlas)?,
                        ];
                        let glyphs = ranges.last().map_or(0, |r| r.end);
                        let order = [(1, 0..items.len() as u32), (0, 0..glyphs)];
                        batch.prepare(
                            &queue,
                            &sources,
                            &order,
                            [1024., 768.],
                            1.,
                            Some(&transforms),
                        )?;
                        use lagui::gpu::unified::Item;
                        let mut order = Vec::new();
                        if scene == "mixed" {
                            for (i, range) in ranges.iter().enumerate() {
                                order.push(Item::Visual(i as u32..i as u32 + 1));
                                order.push(Item::Outline(
                                    range.start + items.len() as u32
                                        ..range.end + items.len() as u32,
                                ));
                                order.push(Item::Emoji(i as u32..i as u32 + 1));
                            }
                        } else {
                            order.push(Item::Visual(0..visuals.prepared_len() as u32));
                            order.push(Item::Outline(0..batch.prepared_len()));
                        }
                        pulled.prepare(
                            &queue,
                            &visuals,
                            &batch,
                            &emoji,
                            &order,
                            Some(&transforms),
                        )?;
                    } else {
                        vectors.prepare_transformed(
                            &queue,
                            &items,
                            [1024., 768.],
                            1.,
                            &transforms,
                        )?;
                    }
                    let mut encoder = device.create_command_encoder(&Default::default());
                    timer.begin();
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("benchmark UI"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                depth_slice: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            timestamp_writes: timer.writes(),
                            ..Default::default()
                        });
                        if unified {
                            pulled.draw_range(&mut pass, 0..pulled.prepared_len());
                        } else if scene == "mixed" {
                            for (i, range) in ranges.iter().enumerate() {
                                visuals.draw_range(&mut pass, i..i + 1);
                                text.draw_range(&mut pass, range.clone());
                                emoji.draw_range(&mut pass, i as u32..i as u32 + 1);
                            }
                        } else {
                            visuals.draw(&mut pass);
                            vectors.draw(&mut pass);
                            text.draw(&mut pass);
                        }
                    }
                    timer.resolve(&mut encoder);
                    queue.submit([encoder.finish()]);
                    timer.submitted();
                    device.poll(wgpu::PollType::Poll)?;
                    if frame == 59 {
                        device.poll(wgpu::PollType::wait_indefinitely())?;
                        timer.collect(&mut gpu);
                        gpu.clear();
                    } else if frame >= 60 {
                        cpu.push(start.elapsed().as_secs_f64() * 1e6);
                        timer.collect(&mut gpu);
                        for stats in [
                            visuals.upload_stats(),
                            text.upload_stats(),
                            if unified {
                                batch.upload_stats()
                            } else {
                                vectors.upload_stats()
                            },
                            if unified {
                                pulled.upload_stats()
                            } else {
                                Default::default()
                            },
                        ] {
                            bytes += stats.bytes;
                            writes += stats.writes;
                        }
                    }
                }
                device.poll(wgpu::PollType::wait_indefinitely())?;
                timer.collect(&mut gpu);
                let cpu_n = cpu.len();
                let gpu_n = gpu.len();
                let c = percentiles(&mut cpu);
                let g = percentiles(&mut gpu);
                println!(
                    "{{\"scene\":\"{scene}\",\"bidirectional\":{directional},\"unified\":{unified},\"motion\":\"{motion}\",\"cpu_samples\":{cpu_n},\"gpu_samples\":{gpu_n},\"cpu_us\":{{\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{}}},\"gpu_ms\":{{\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{}}},\"uploaded_bytes\":{bytes},\"write_calls\":{writes}}}",
                    c[0], c[1], c[2], c[3], g[0], g[1], g[2], g[3]
                );
            }
        }
    }
    Ok(())
}
