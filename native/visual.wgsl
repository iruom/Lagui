// x, y: logical viewport size. z: physical pixels per logical pixel.
@group(0) @binding(0) var<uniform> viewport: vec4f;

struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) logical: vec2f,
    @location(1) local: vec2f,
    @location(2) @interpolate(flat) extent: vec2f,
    @location(3) @interpolate(flat) color: vec4f,
    @location(4) @interpolate(flat) params: vec3f,
    @location(5) @interpolate(flat) inner_color: vec4f,
    @location(6) @interpolate(flat) shape_bounds: vec4f,
    @location(7) @interpolate(flat) shadow_color: vec4f,
    @location(8) @interpolate(flat) shadow: vec4f,
}

// Abramowitz-Stegun 7.1.26; absolute error below 1.5e-7.
fn erf_approx(x: f32) -> f32 {
    let a = abs(x);
    let t = 1 / (1 + 0.3275911 * a);
    let poly = ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t;
    return sign(x) * (1 - poly * exp(-a * a));
}

// Blur of the box |x| <= half.x along x at height y, for a box whose corners
// are circular arcs of radius `corner`.
fn box_shadow_x(x: f32, y: f32, sigma: f32, corner: f32, half: vec2f) -> f32 {
    let delta = min(half.y - corner - abs(y), 0);
    let curved = half.x - corner + sqrt(max(0, corner * corner - delta * delta));
    let k = 0.70710678 / sigma;
    return 0.5 * (erf_approx((x + curved) * k) - erf_approx((x - curved) * k));
}

// Rounded box convolved with an isotropic Gaussian (Evan Wallace's method):
// exact erf along x, Gaussian-weighted samples along y. A box without rounded
// corners is separable, so both axes are exact. Smoothed corners are treated as
// circular here; the difference is far below the blur scale.
fn rounded_box_shadow(point: vec2f, half: vec2f, corner: f32, sigma: f32) -> f32 {
    let k = 0.70710678 / sigma;
    if corner <= 0.0001 {
        let x = 0.5 * (erf_approx((point.x + half.x) * k) - erf_approx((point.x - half.x) * k));
        let y = 0.5 * (erf_approx((point.y + half.y) * k) - erf_approx((point.y - half.y) * k));
        return x * y;
    }
    let low = point.y - half.y;
    let high = point.y + half.y;
    let start = clamp(-3 * sigma, low, high);
    let stop = clamp(3 * sigma, low, high);
    let samples = 6;
    let step = (stop - start) / f32(samples);
    var value = 0.0;
    for (var i = 0; i < samples; i++) {
        let y = start + step * (f32(i) + 0.5);
        let weight = exp(-0.5 * (y / sigma) * (y / sigma)) * 0.39894228 / sigma;
        value += box_shadow_x(point.x, point.y - y, sigma, corner, half) * weight * step;
    }
    return value;
}

fn has_shadow(color: vec4f) -> bool {
    return any(color != vec4f(0));
}

@vertex fn vertex_main(
    @builtin(vertex_index) id: u32,
    @location(0) shape_bounds: vec4f,
    @location(1) clip: vec4f,
    @location(2) color: vec4f,
    @location(3) inner_color: vec4f,
    @location(4) shadow_color: vec4f,
    @location(5) shadow: vec4f,
    // radius, stroke width (negative fills), corner smoothing
    @location(6) params: vec3f,
) -> Vertex {
    let corners = array<vec2f, 6>(
        vec2f(0,0), vec2f(1,0), vec2f(0,1),
        vec2f(0,1), vec2f(1,0), vec2f(1,1));
    // Grow by the Gaussian reach only when a shadow can contribute, then by one
    // physical pixel so both sides of the antialiasing ramp are rasterized.
    var grow_min = vec2f(0);
    var grow_max = vec2f(0);
    if has_shadow(shadow_color) {
        let reach = max(shadow.z, 0) * 3 + max(shadow.w, 0);
        grow_min = vec2f(reach) + max(-shadow.xy, vec2f(0));
        grow_max = vec2f(reach) + max(shadow.xy, vec2f(0));
    }
    let margin = 1 / viewport.z;
    // Clipping the quad here replaces a per-fragment discard: the rasterizer
    // samples the same pixel centers, but clipped-away pixels never shade.
    let lo = max(shape_bounds.xy - grow_min - margin, clip.xy);
    let hi = max(min(shape_bounds.xy + shape_bounds.zw + grow_max + margin, clip.xy + clip.zw), lo);
    let logical = mix(lo, hi, corners[id]);
    return Vertex(
        vec4f(logical.x / viewport.x * 2 - 1, 1 - logical.y / viewport.y * 2, 0, 1),
        logical, logical - shape_bounds.xy, shape_bounds.zw, color, params,
        inner_color, shape_bounds, shadow_color, shadow);
}

@fragment fn fragment_main(in: Vertex) -> @location(0) vec4f {
    let scale = viewport.z;
    let radius = in.params.x;
    let stroke = in.params.y;
    let smoothing = in.params.z;
    let centered = in.local - in.extent * 0.5;
    let outer_distance = rounded_distance(centered, in.extent, radius, smoothing);
    let outer_coverage = clamp(0.5 - outer_distance * scale, 0, 1);
    var base = in.color * outer_coverage;
    if stroke >= 0 {
        let width = min(stroke, min(in.extent.x, in.extent.y) * 0.5);
        let inner_extent = in.extent - 2 * width;
        var inner_coverage = 0.0;
        if inner_extent.x > 0 && inner_extent.y > 0 {
            // The interior is the outline offset inward by `width`, so the stroke
            // keeps a constant width around smoothed corners as well as circular ones.
            inner_coverage = clamp(0.5 - (outer_distance + width) * scale, 0, 1);
        }
        base = in.color * (outer_coverage - inner_coverage) + in.inner_color * inner_coverage;
    }
    // Flat per instance, so shadowless visuals skip the second distance field.
    if !has_shadow(in.shadow_color) {
        return base;
    }
    let shadow_center = in.shape_bounds.xy + in.shape_bounds.zw * 0.5 + in.shadow.xy;
    // The spread outline: half extent and corner radius both grow by `spread`.
    let half = max(in.shape_bounds.zw * 0.5 + in.shadow.w, vec2f(0));
    // CSS/Figma spread radius: grows with the spread, but a sharp or small corner
    // stays proportionally sharp instead of becoming a spread-sized arc.
    let spread = in.shadow.w;
    let base_radius = clamp(radius, 0, min(in.shape_bounds.z, in.shape_bounds.w) * 0.5);
    var grown = base_radius + spread;
    if spread > 0 && base_radius < spread {
        let ratio = base_radius / spread - 1;
        grown = base_radius + spread * (1 + ratio * ratio * ratio);
    }
    let corner = clamp(grown, 0, min(half.x, half.y));
    // A one-pixel box filter (sigma = 1/sqrt 12 px) is folded into the Gaussian,
    // so an unblurred shadow keeps an antialiased edge.
    let pixel_sigma = 0.28867513 / scale;
    let sigma = sqrt(in.shadow.z * in.shadow.z + pixel_sigma * pixel_sigma);
    let shadow_coverage = rounded_box_shadow(in.logical - shadow_center, half, corner, sigma);
    return base + in.shadow_color * shadow_coverage * (1 - base.a);
}
