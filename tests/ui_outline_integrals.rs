#![cfg(feature = "portable")]
//! Independent polygon/pixel clipping oracle for footprints spanning several
//! outline bands. No shader roots, band lists, or ownership rules are reused.
mod support;
use lagui::{
    Rect,
    gpu::{Shared, outlines::OutlineBatch},
    transform::TransformId,
    windfoil::{Atlas, Instance, Path, Pipeline, Renderer},
};

type Point = [f64; 2];

fn clipped_area(polygon: &[Point], low: Point, high: Point) -> f64 {
    let mut vertices = polygon.to_vec();
    for (axis, bound, greater) in [
        (0, low[0], true),
        (0, high[0], false),
        (1, low[1], true),
        (1, high[1], false),
    ] {
        let mut result = Vec::new();
        if vertices.is_empty() {
            return 0.;
        }
        let inside = |p: Point| {
            if greater {
                p[axis] >= bound
            } else {
                p[axis] <= bound
            }
        };
        let mut previous = *vertices.last().unwrap();
        for &current in &vertices {
            if inside(previous) != inside(current) {
                let t = (bound - previous[axis]) / (current[axis] - previous[axis]);
                result.push([
                    previous[0] + t * (current[0] - previous[0]),
                    previous[1] + t * (current[1] - previous[1]),
                ]);
            }
            if inside(current) {
                result.push(current);
            }
            previous = current;
        }
        vertices = result;
    }
    vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
        .sum::<f64>()
        * 0.5
}

#[test]
fn multi_band_footprints_match_polygon_area_with_holes_and_repeated_contours() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let outer = vec![[0., 0.], [48., 0.], [48., 48.], [0., 48.]];
    let hole = vec![[11., 9.], [11., 39.], [37., 39.], [37., 9.]];
    let diagonal = vec![[0., 0.], [48., 17.], [31., 48.], [3., 37.]];
    let cases = [
        vec![outer.clone(), hole],
        vec![outer.clone(), outer],
        vec![diagonal],
    ];
    let clip = Rect::new(9., 12., 47., 40.);
    for polygons in cases {
        let mut path = Path::new();
        for polygon in &polygons {
            path.move_to([polygon[0][0] as f32, polygon[0][1] as f32]);
            // Splitting each edge deliberately creates many bands without
            // changing the oracle polygon, including exact boundary endpoints.
            for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
                for i in 1..=16 {
                    let t = f64::from(i) / 16.;
                    path.line_to([
                        (a[0] + t * (b[0] - a[0])) as f32,
                        (a[1] + t * (b[1] - a[1])) as f32,
                    ]);
                }
            }
            path.close();
        }
        let mut atlas = Atlas::new();
        let mut prefix = Path::new();
        prefix
            .move_to([0., 0.])
            .line_to([1., 0.])
            .line_to([0., 1.])
            .close();
        atlas.add_fill(&prefix).unwrap();
        let shape = atlas.add_fill(&path).unwrap();
        for direction in [false, true] {
            let pipeline = Pipeline::with_band_direction(
                &device,
                wgpu::TextureFormat::Rgba8Unorm,
                &Shared::new(&device),
                direction,
            );
            let mut renderer = Renderer::with_pipeline(&device, &pipeline, 1).unwrap();
            renderer.sync_atlas(&device, &atlas).unwrap();
            renderer.set_coverage_gamma(1.);
            let mut batch = OutlineBatch::new(&device, &pipeline).unwrap();
            for scale in [[0.73, 0.13], [0.27, 0.31], [-0.73, 0.13], [0.73, -0.13]] {
                let origin = [
                    if scale[0] > 0. { 7.35 } else { 55.35 },
                    if scale[1] > 0. { 13.2 } else { 40.2 },
                ];
                let item = Instance {
                    shape,
                    origin,
                    scale,
                    // Mid-grey leaves perceptual coverage linear in both the
                    // ordinary renderer and the shared text specialization.
                    color: [0.5, 0.5, 0.5, 1.],
                    clip: Some(clip),
                    transform: TransformId::IDENTITY,
                };
                renderer.prepare_cpu(&atlas, &[item], [64.; 2], 1.).unwrap();
                let sources = [renderer.prepared_source(&atlas).unwrap()];
                batch
                    .prepare(&queue, &sources, &[(0, 0..1)], [64.; 2], 1., None)
                    .unwrap();
                let pixels = support::render(&device, &queue, [64; 2], |pass| {
                    batch.draw_range(pass, 0..1)
                });
                let screen: Vec<Vec<Point>> = polygons
                    .iter()
                    .map(|polygon| {
                        polygon
                            .iter()
                            .map(|p| {
                                [
                                    f64::from(origin[0]) + p[0] * f64::from(scale[0]),
                                    f64::from(origin[1]) + p[1] * f64::from(scale[1]),
                                ]
                            })
                            .collect()
                    })
                    .collect();
                for y in 0..64 {
                    for x in 0..64 {
                        let expected = if clip.contains(x as f32 + 0.5, y as f32 + 0.5) {
                            screen
                                .iter()
                                .map(|p| {
                                    clipped_area(
                                        p,
                                        [x as f64, y as f64],
                                        [x as f64 + 1., y as f64 + 1.],
                                    )
                                })
                                .sum::<f64>()
                                .abs()
                                .min(1.)
                        } else {
                            0.
                        };
                        let actual = f64::from(pixels[(y * 64 + x) * 4 + 3]) / 255.;
                        assert!(
                            (actual - expected).abs() <= 2. / 255.,
                            "direction={direction} scale={scale:?} ({x},{y}): {actual} != {expected}"
                        );
                    }
                }
            }
        }
    }
}
