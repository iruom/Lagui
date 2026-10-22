#![cfg(feature = "portable")]
//! Antialiasing contracts checked against independent formulas on real GPU output.
mod support;
use lagui::{
    Rect,
    gpu::VisualRenderer,
    text::{Font, TextCache, TextRenderer, TextSection},
    visual::{CornerStyle, DropShadow, StrokeAlignment, Visual, VisualList},
};

const SIZE: [u32; 2] = [64, 64];
const VIEWPORT: [f32; 2] = [64., 64.];
const CLIP: Rect = Rect::new(0., 0., 64., 64.);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn alpha(pixels: &[u8], x: u32, y: u32) -> f32 {
    f32::from(pixels[((y * SIZE[0] + x) * 4 + 3) as usize]) / 255.
}

/// erf by numerical integration, independent of the shader's approximation.
fn erf(x: f64) -> f64 {
    let steps = 4000;
    let h = x / f64::from(steps);
    let sum: f64 = (0..steps)
        .map(|i| (-(h * (f64::from(i) + 0.5)).powi(2)).exp())
        .sum();
    sum * h * 2. / std::f64::consts::PI.sqrt()
}

#[test]
fn drop_shadow_edges_follow_the_gaussian_convolution() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let sigma = 4.;
    let mut storage = [Visual::default(); 1];
    let mut list = VisualList::new(&mut storage);
    // A transparent shape leaves only its shadow; the offset keeps the shadow's
    // left edge (x = 28) away from the shape.
    list.fill_styled(
        Rect::new(8., 0., 16., 64.),
        [0.; 4],
        CornerStyle::default(),
        Some(DropShadow {
            color: [0., 0., 0., 1.],
            offset: [20., 0.],
            blur: sigma,
            spread: 0.,
        }),
        CLIP,
    )
    .unwrap();
    let mut renderer = VisualRenderer::new(&device, FORMAT, 1).unwrap();
    renderer.prepare(&queue, &storage, VIEWPORT, 1.).unwrap();
    let pixels = support::render(&device, &queue, SIZE, |pass| renderer.draw(pass));
    let sigma = (f64::from(sigma).powi(2) + 1. / 12.).sqrt();
    let phi = |d: f64| 0.5 + 0.5 * erf(d / (sigma * std::f64::consts::SQRT_2));
    for x in 18..54 {
        // A sharp box blurs separably: both of its x edges (28 and 44) contribute.
        let center = f64::from(x) + 0.5;
        let expected = phi(center - 28.) - phi(center - 44.);
        let actual = f64::from(alpha(&pixels, x, 32));
        assert!(
            (actual - expected).abs() <= 1.5 / 255.,
            "x {x}: {actual} versus {expected}"
        );
    }
    // CSS/Figma blur radii are twice the standard deviation.
    let css = DropShadow::from_blur_radius([0., 0., 0., 1.], [0.; 2], 8., 1.);
    assert_eq!((css.blur, css.spread), (4., 1.));
}

/// Brute-force 2D convolution of a rounded box with a Gaussian.
fn rounded_box_blur(point: [f64; 2], half: [f64; 2], radius: f64, sigma: f64) -> f64 {
    let inside = |x: f64, y: f64| {
        let q = [x.abs() - half[0] + radius, y.abs() - half[1] + radius];
        let outside = (q[0].max(0.).powi(2) + q[1].max(0.).powi(2)).sqrt();
        outside + q[0].max(q[1]).min(0.) - radius <= 0.
    };
    let step = 0.25;
    let reach = (sigma * 4. / step) as i32;
    let mut sum = 0.;
    for i in -reach..=reach {
        for j in -reach..=reach {
            let (dx, dy) = (f64::from(i) * step, f64::from(j) * step);
            if inside(point[0] + dx, point[1] + dy) {
                sum += (-(dx * dx + dy * dy) / (2. * sigma * sigma)).exp();
            }
        }
    }
    sum * step * step / (2. * std::f64::consts::PI * sigma * sigma)
}

#[test]
fn rounded_shadow_corners_match_a_brute_force_convolution() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let mut storage = [Visual::default(); 1];
    let mut list = VisualList::new(&mut storage);
    list.fill_styled(
        Rect::new(16., 16., 32., 32.),
        [0.; 4],
        CornerStyle::new(10., 0.),
        Some(DropShadow {
            color: [0., 0., 0., 1.],
            offset: [0.; 2],
            blur: 3.,
            spread: 0.,
        }),
        CLIP,
    )
    .unwrap();
    let mut renderer = VisualRenderer::new(&device, FORMAT, 1).unwrap();
    renderer.prepare(&queue, &storage, VIEWPORT, 1.).unwrap();
    let pixels = support::render(&device, &queue, SIZE, |pass| renderer.draw(pass));
    let sigma = (9f64 + 1. / 12.).sqrt();
    for (x, y) in [
        (10, 10),
        (14, 14),
        (16, 16),
        (18, 18),
        (20, 12),
        (12, 32),
        (40, 52),
    ] {
        let point = [f64::from(x) + 0.5 - 32., f64::from(y) + 0.5 - 32.];
        let expected = rounded_box_blur(point, [16., 16.], 10., sigma);
        let actual = f64::from(alpha(&pixels, x, y));
        assert!(
            (actual - expected).abs() <= 0.02,
            "({x}, {y}): {actual} versus {expected}"
        );
    }
}

/// Sum of alpha sampled one physical pixel apart along a ray, i.e. the stroke
/// width measured along that ray in pixel-step units.
fn ray_ink(pixels: &[u8], start: [u32; 2], step: [i32; 2], count: u32) -> f32 {
    (0..count)
        .map(|i| {
            let x = start[0] as i32 + step[0] * i as i32;
            let y = start[1] as i32 + step[1] * i as i32;
            alpha(pixels, x as u32, y as u32)
        })
        .sum()
}

#[test]
fn smoothed_corner_strokes_keep_their_width() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let width = 4.;
    let mut renderer = VisualRenderer::new(&device, FORMAT, 1).unwrap();
    for smoothing in [0., 60., 100.] {
        let mut storage = [Visual::default(); 1];
        let mut list = VisualList::new(&mut storage);
        list.outline_styled(
            Rect::new(4., 4., 56., 56.),
            [1.; 4],
            [0.; 4],
            CornerStyle::from_percent(16., smoothing),
            width,
            StrokeAlignment::Inside,
            CLIP,
        )
        .unwrap();
        renderer.prepare(&queue, &storage, VIEWPORT, 1.).unwrap();
        let pixels = support::render(&device, &queue, SIZE, |pass| renderer.draw(pass));
        let side = ray_ink(&pixels, [0, 32], [1, 0], 16);
        assert!((side - width).abs() < 0.05, "{smoothing}%: side {side}");
        // The diagonal is the corner's symmetry axis and therefore its normal;
        // a pixel step along it advances sqrt(2) along the normal.
        let diagonal = ray_ink(&pixels, [0, 0], [1, 1], 24) * std::f32::consts::SQRT_2;
        assert!(
            (diagonal - width).abs() < 0.35,
            "{smoothing}%: corner width {diagonal}"
        );
    }
}

fn glyph(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut TextRenderer,
    cache: &TextCache,
    baseline: [f32; 2],
    color: [f32; 4],
) -> Vec<u8> {
    renderer.sync_font(device, cache.font()).unwrap();
    renderer
        .prepare(
            queue,
            &[TextSection {
                run: cache.get("O").unwrap(),
                baseline,
                size: 40.,
                color,
                clip: None,
            }],
            VIEWPORT,
            1.,
        )
        .unwrap();
    support::render(device, queue, SIZE, |pass| renderer.draw(pass))
}

#[test]
fn text_coverage_is_perceptual_for_dark_and_light_text() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let font = Font::new(include_bytes!("assets/test-outline.ttf").to_vec(), 0).unwrap();
    let mut cache = TextCache::new(font);
    cache.cache("O").unwrap();
    let mut renderer = TextRenderer::new(&device, FORMAT, 1).unwrap();
    let white = [1.; 4];
    let black = [0., 0., 0., 1.];
    renderer.set_coverage_gamma(1.);
    // A fractional x origin puts whole glyph edges on partially covered pixels.
    let linear = glyph(&device, &queue, &mut renderer, &cache, [8.4, 48.], white);
    let linear_black = glyph(&device, &queue, &mut renderer, &cache, [8.4, 48.], black);
    let alphas = |pixels: &[u8]| {
        pixels
            .iter()
            .skip(3)
            .step_by(4)
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(alphas(&linear), alphas(&linear_black));

    renderer.set_coverage_gamma(lagui::windfoil::PERCEPTUAL_COVERAGE_GAMMA);
    let light = glyph(&device, &queue, &mut renderer, &cache, [8.4, 48.], white);
    let dark = glyph(&device, &queue, &mut renderer, &cache, [8.4, 48.], black);
    let gamma = 2.2f32;
    let mut partial = 0;
    for ((c, l), d) in alphas(&linear)
        .into_iter()
        .zip(alphas(&light))
        .zip(alphas(&dark))
    {
        let c = f32::from(c) / 255.;
        // Gamma-space blending over the opposite luminance, solved for alpha.
        let light_expected = c.powf(gamma);
        let dark_expected = 1. - (1. - c).powf(gamma);
        assert!((f32::from(l) / 255. - light_expected).abs() <= 2.5 / 255.);
        assert!((f32::from(d) / 255. - dark_expected).abs() <= 2.5 / 255.);
        if c > 0.1 && c < 0.9 {
            partial += 1;
            assert!(d > l, "dark text edges are heavier than light text edges");
        }
    }
    assert!(partial > 8, "the glyph must have antialiased edge pixels");
}

#[test]
fn text_baselines_snap_to_physical_pixel_rows() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let font = Font::new(include_bytes!("assets/test-outline.ttf").to_vec(), 0).unwrap();
    let mut cache = TextCache::new(font);
    cache.cache("O").unwrap();
    let mut renderer = TextRenderer::new(&device, FORMAT, 1).unwrap();
    let white = [1.; 4];
    let base = glyph(&device, &queue, &mut renderer, &cache, [8., 48.], white);
    let near = glyph(&device, &queue, &mut renderer, &cache, [8., 48.3], white);
    let next = glyph(&device, &queue, &mut renderer, &cache, [8., 48.6], white);
    let row = glyph(&device, &queue, &mut renderer, &cache, [8., 49.], white);
    assert_eq!(base, near);
    assert_eq!(next, row);
    assert_ne!(base, row);
    // Horizontal placement keeps subpixel precision.
    let shifted = glyph(&device, &queue, &mut renderer, &cache, [8.3, 48.], white);
    assert_ne!(base, shifted);
}
