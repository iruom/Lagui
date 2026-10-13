#![cfg(feature = "portable")]
//! GPU contract for Figma-style corners. Reference contours are sampled as paths,
//! independently of the shader's implicit-distance/bisection implementation.
use lagui::{
    Rect,
    gpu::VisualRenderer,
    visual::{CornerStyle, Visual, VisualList},
};

fn reference_corner(radius: f64, smoothing: f64, budget: f64) -> Vec<[f64; 2]> {
    let r = radius.min(budget);
    let s = smoothing.min((budget / r - 1.).max(0.));
    let p = r * (1. + s);
    let arc_angle = std::f64::consts::FRAC_PI_2 * (1. - s);
    let arc_run = (arc_angle / 2.).sin() * r * 2_f64.sqrt();
    let beta = s * std::f64::consts::FRAC_PI_4;
    let tangent = r * (beta / 2.).tan();
    let c = tangent * beta.cos();
    let d = tangent * beta.sin();
    let b = (p - arc_run - c - d) / 3.;
    let a = 2. * b;
    let mut points = Vec::new();
    if s > 0. {
        for i in 0..=64 {
            let t = f64::from(i) / 64.;
            let h = 1. - t;
            points.push([
                p - (3. * h * h * t * a + 3. * h * t * t * (a + b) + t * t * t * (a + b + c)),
                t * t * t * d,
            ]);
        }
    }
    for i in 0..=64 {
        let angle = beta + arc_angle * f64::from(i) / 64.;
        points.push([r * (1. - angle.sin()), r * (1. - angle.cos())]);
    }
    if s > 0. {
        for i in (0..=64).rev() {
            let t = f64::from(i) / 64.;
            let h = 1. - t;
            points.push([
                t * t * t * d,
                p - (3. * h * h * t * a + 3. * h * t * t * (a + b) + t * t * t * (a + b + c)),
            ]);
        }
    }
    points
}

fn segment_distance(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let edge = [b[0] - a[0], b[1] - a[1]];
    let length = edge[0] * edge[0] + edge[1] * edge[1];
    let t = if length > 0. {
        ((point[0] - a[0]) * edge[0] + (point[1] - a[1]) * edge[1]) / length
    } else {
        0.
    }
    .clamp(0., 1.);
    (point[0] - a[0] - t * edge[0]).hypot(point[1] - a[1] - t * edge[1])
}

#[test]
fn gpu_smoothing_matches_arc_cubic_contours_at_zero_sixty_and_one_hundred_percent() {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
        else {
            assert!(
                std::env::var_os("LAGUI_REQUIRE_GPU").is_none(),
                "GPU adapter required"
            );
            eprintln!("SKIP ui_corner_smoothing: no GPU adapter");
            return;
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap();
        let mut renderer =
            VisualRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
        for (radius, percent) in [(16., 0.), (16., 60.), (16., 100.), (32., 100.), (40., 100.)] {
            for scale in [1., 2.] {
                let width = (128. * scale) as u32;
                let height = (96. * scale) as u32;
                let stride = (width * 4).div_ceil(256) * 256;
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("corner smoothing regression"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let mut storage = [Visual::default(); 1];
                let mut list = VisualList::new(&mut storage);
                list.fill_styled(
                    Rect::new(8., 8., 112., 80.),
                    [1.; 4],
                    CornerStyle::from_percent(radius, percent),
                    None,
                    Rect::new(0., 0., 128., 96.),
                )
                .unwrap();
                renderer
                    .prepare(&queue, &storage, [128., 96.], scale)
                    .unwrap();
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(stride * height),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let view = texture.create_view(&Default::default());
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
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(stride),
                            rows_per_image: Some(height),
                        },
                    },
                    texture.size(),
                );
                queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    tx.send(r).unwrap();
                });
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                rx.recv().unwrap().unwrap();
                let pixels = buffer.slice(..).get_mapped_range().unwrap();
                let mut contour =
                    reference_corner(f64::from(radius), f64::from(percent) / 100., 40.);
                contour.insert(0, [56., 0.]);
                contour.push([0., 40.]);
                contour.push([56., 40.]);
                for y in 0..(40. * scale) as u32 {
                    for x in 0..(56. * scale) as u32 {
                        let point = [
                            (f64::from(x) + 0.5) / f64::from(scale),
                            (f64::from(y) + 0.5) / f64::from(scale),
                        ];
                        let mut inside = false;
                        let mut distance = f64::INFINITY;
                        for (a, b) in contour
                            .iter()
                            .zip(contour.iter().cycle().skip(1))
                            .take(contour.len())
                        {
                            distance = distance.min(segment_distance(point, *a, *b));
                            if (a[1] > point[1]) != (b[1] > point[1])
                                && point[0]
                                    < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
                            {
                                inside = !inside;
                            }
                        }
                        let px = (120. * scale) as u32 - 1 - x;
                        let py = (8. * scale) as u32 + y;
                        let alpha = pixels[(py * stride + px * 4 + 3) as usize];
                        if distance * f64::from(scale) < 0.8 {
                            let signed = if inside { -distance } else { distance };
                            let expected = ((0.5 - signed * f64::from(scale)).clamp(0., 1.) * 255.)
                                .round() as u8;
                            assert!(
                                alpha.abs_diff(expected) <= 24,
                                "AA mismatch r={radius}, smoothing={percent}%, scale={scale}, point={point:?}: {alpha} vs {expected}"
                            );
                            continue;
                        }
                        assert_eq!(
                            alpha > 127,
                            inside,
                            "r={radius}, smoothing={percent}%, scale={scale}, point={point:?}, alpha={alpha}"
                        );
                    }
                }
            }
        }
    });
}
