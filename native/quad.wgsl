@group(0) @binding(0) var<uniform> viewport: vec4f;
struct Vertex { @builtin(position) position: vec4f, @location(0) color: vec4f }
@vertex fn vertex_main(@builtin(vertex_index) id: u32,
    @location(0) bounds: vec4f, @location(1) color: vec4f) -> Vertex {
    let corners = array<vec2f, 6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let p = bounds.xy + corners[id] * bounds.zw;
    return Vertex(vec4f(p.x / viewport.x * 2 - 1, 1 - p.y / viewport.y * 2, 0, 1), color);
}
@fragment fn fragment_main(in: Vertex) -> @location(0) vec4f { return in.color; }
