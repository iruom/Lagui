struct Scene { time: f32, aspect: f32, _pad: vec2<f32> }
@group(0) @binding(0) var<uniform> scene: Scene;

struct Out { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32> }

const P = array<vec3<f32>, 8>(
    vec3(-1.,-1.,-1.), vec3(1.,-1.,-1.), vec3(1.,1.,-1.), vec3(-1.,1.,-1.),
    vec3(-1.,-1., 1.), vec3(1.,-1., 1.), vec3(1.,1., 1.), vec3(-1.,1., 1.)
);
const I = array<u32, 36>(
    4,5,6, 4,6,7, 1,0,3, 1,3,2, 0,4,7, 0,7,3,
    5,1,2, 5,2,6, 3,7,6, 3,6,2, 0,1,5, 0,5,4
);

@vertex fn vertex_main(@builtin(vertex_index) vi: u32) -> Out {
    var p = P[I[vi]];
    let cy = cos(scene.time); let sy = sin(scene.time);
    let cx = cos(scene.time * .63); let sx = sin(scene.time * .63);
    p = vec3(cy*p.x + sy*p.z, p.y, -sy*p.x + cy*p.z);
    p = vec3(p.x, cx*p.y - sx*p.z, sx*p.y + cx*p.z);
    var out: Out;
    let z = p.z + 4.2;
    out.position = vec4(p.x / max(scene.aspect, .01) * 1.7, p.y * 1.7, z - 2.2, z);
    out.normal = normalize(p);
    return out;
}

@fragment fn fragment_main(in: Out) -> @location(0) vec4<f32> {
    let light = max(dot(normalize(in.normal), normalize(vec3(.4,.8,.6))), 0.);
    let color = vec3(.20,.10,.82) + light * vec3(.34,.28,.18);
    return vec4(color, 1.);
}
