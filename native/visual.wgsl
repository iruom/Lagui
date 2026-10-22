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
            let inner_distance = rounded_distance(centered, inner_extent, max(radius - width, 0), smoothing);
            inner_coverage = clamp(0.5 - inner_distance * scale, 0, 1);
        }
        base = in.color * (outer_coverage - inner_coverage) + in.inner_color * inner_coverage;
    }
    // Flat per instance, so shadowless visuals skip the second distance field.
    if !has_shadow(in.shadow_color) {
        return base;
    }
    let shadow_centered = (in.logical - in.shadow.xy) - (in.shape_bounds.xy + in.shape_bounds.zw * 0.5);
    let shadow_distance = rounded_distance(shadow_centered, in.shape_bounds.zw, radius, smoothing) - in.shadow.w;
    var shadow_coverage = clamp(0.5 - shadow_distance * scale, 0, 1);
    if in.shadow.z > 0.0001 && shadow_distance > 0 {
        let gaussian = shadow_distance / in.shadow.z;
        shadow_coverage = exp(-0.5 * gaussian * gaussian);
    }
    return base + in.shadow_color * shadow_coverage * (1 - base.a);
}
