#include <metal_stdlib>
using namespace metal;
struct Quad { float4 bounds; float4 color; };
struct Vertex { float4 position [[position]]; float4 color; };
vertex Vertex quad_vertex(uint id [[vertex_id]], uint instance [[instance_id]],
                          constant Quad *quads [[buffer(0)]],
                          constant float2 &viewport [[buffer(1)]]) {
    constexpr float2 corners[] = { {0,0}, {1,0}, {0,1}, {0,1}, {1,0}, {1,1} };
    Quad q = quads[instance];
    float2 p = q.bounds.xy + corners[id] * q.bounds.zw;
    return { float4(p.x / viewport.x * 2 - 1, 1 - p.y / viewport.y * 2, 0, 1), q.color };
}
fragment float4 quad_fragment(Vertex in [[stage_in]]) { return in.color; }
