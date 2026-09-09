@group(0) @binding(0) var<uniform> viewport: vec4f;
@group(0) @binding(1) var atlas: texture_2d_array<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f, @location(1) @interpolate(flat) layer: u32 }

@vertex fn vertex_main(@builtin(vertex_index) id: u32, @location(0) bounds: vec4f,
    @location(1) clip: vec4f, @location(2) layer: u32) -> Out {
    let corners = array<vec2f,6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let uv = corners[id];
    let raw = bounds.xy + uv * bounds.zw;
    let lo = clip.xy; let hi = clip.xy + clip.zw;
    let p = clamp(raw, lo, hi);
    let clipped_uv = select(uv, (p - bounds.xy) / bounds.zw, all(bounds.zw > vec2f(0)));
    return Out(vec4f(p.x/viewport.x*2-1, 1-p.y/viewport.y*2, 0, 1), clipped_uv, layer);
}
@fragment fn fragment_main(in: Out) -> @location(0) vec4f {
    return textureSample(atlas, atlas_sampler, in.uv, i32(in.layer));
}
