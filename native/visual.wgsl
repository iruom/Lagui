@group(0) @binding(0) var<uniform> viewport: vec4f;

struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) logical: vec2f,
    @location(1) local: vec2f,
    @location(2) @interpolate(flat) extent: vec2f,
    @location(3) @interpolate(flat) color: vec4f,
    @location(4) @interpolate(flat) clip: vec4f,
    @location(5) @interpolate(flat) style: vec3f,
    @location(6) @interpolate(flat) inner_color: vec4f,
    @location(7) @interpolate(flat) smoothing: f32,
    @location(8) @interpolate(flat) shape_bounds: vec4f,
    @location(9) @interpolate(flat) shadow_color: vec4f,
    @location(10) @interpolate(flat) shadow: vec4f,
}

@vertex fn vertex_main(
    @builtin(vertex_index) id: u32,
    @location(0) bounds: vec4f,
    @location(1) color: vec4f,
    @location(2) clip: vec4f,
    @location(3) style: vec4f,
    @location(4) inner_color: vec4f,
    @location(5) shape_bounds: vec4f,
    @location(6) shadow_color: vec4f,
    @location(7) shadow: vec4f,
) -> Vertex {
    let corners = array<vec2f, 6>(
        vec2f(0,0), vec2f(1,0), vec2f(0,1),
        vec2f(0,1), vec2f(1,0), vec2f(1,1));
    let corner = corners[id];
    // Include one physical pixel outside the mathematical bounds so both sides
    // of the antialiasing ramp are rasterized.
    let margin = 1 / style.y;
    let logical = bounds.xy - margin + corner * (bounds.zw + 2 * margin);
    return Vertex(
        vec4f(logical.x / viewport.x * 2 - 1, 1 - logical.y / viewport.y * 2, 0, 1),
        logical, logical - shape_bounds.xy, shape_bounds.zw, color, clip, style.xyz,
        inner_color, style.w, shape_bounds, shadow_color, shadow);
}

fn rounded_distance(centered: vec2f, extent: vec2f, radius_value: f32, smoothing_value: f32) -> f32 {
    let radius = clamp(radius_value, 0, min(extent.x, extent.y) * 0.5);
    let smoothing = clamp(smoothing_value, 0, 1);
    let corner_extent = min(vec2f(radius * (1 + smoothing)), extent * 0.5);
    if radius <= 0.0001 || corner_extent.x <= 0.0001 || corner_extent.y <= 0.0001 {
        let q = abs(centered) - extent * 0.5;
        return length(max(q, vec2f(0))) + min(max(q.x, q.y), 0);
    }
    let q = abs(centered) - (extent * 0.5 - corner_extent);
    let exponent = mix(2, 6, smoothing);
    let normalized = max(q, vec2f(0)) / corner_extent;
    let superellipse = pow(pow(normalized.x, exponent) + pow(normalized.y, exponent), 1 / exponent);
    let scale = min(corner_extent.x, corner_extent.y);
    return superellipse * scale + min(max(q.x, q.y), 0) - scale;
}

@fragment fn fragment_main(in: Vertex) -> @location(0) vec4f {
    if in.logical.x < in.clip.x || in.logical.y < in.clip.y
        || in.logical.x >= in.clip.x + in.clip.z
        || in.logical.y >= in.clip.y + in.clip.w {
        discard;
    }
    let centered = in.local - in.extent * 0.5;
    let outer_distance = rounded_distance(centered, in.extent, in.style.x, in.smoothing);
    let outer_coverage = clamp(0.5 - outer_distance * in.style.y, 0, 1);
    let shadow_centered = (in.logical - in.shadow.xy) - (in.shape_bounds.xy + in.shape_bounds.zw * 0.5);
    let shadow_distance = rounded_distance(shadow_centered, in.shape_bounds.zw, in.style.x, in.smoothing) - in.shadow.w;
    var shadow_coverage = clamp(0.5 - shadow_distance * in.style.y, 0, 1);
    if in.shadow.z > 0.0001 && shadow_distance > 0 {
        let gaussian = shadow_distance / in.shadow.z;
        shadow_coverage = exp(-0.5 * gaussian * gaussian);
    }
    let shadow_layer = in.shadow_color * shadow_coverage;
    if in.style.z < 0 {
        let base = in.color * outer_coverage;
        return base + shadow_layer * (1 - base.a);
    }

    let width = min(in.style.z, min(in.extent.x, in.extent.y) * 0.5);
    let inner_extent = in.extent - 2 * width;
    var inner_coverage = 0.0;
    if inner_extent.x > 0 && inner_extent.y > 0 {
        let inner_distance = rounded_distance(centered, inner_extent, max(in.style.x - width, 0), in.smoothing);
        inner_coverage = clamp(0.5 - inner_distance * in.style.y, 0, 1);
    }
    let base = in.color * (outer_coverage - inner_coverage) + in.inner_color * inner_coverage;
    return base + shadow_layer * (1 - base.a);
}
