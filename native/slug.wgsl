// Slug root eligibility, ray coverage and weighted combination adapted from
// Eric Lengyel, SlugPixelShader.hlsl, Copyright 2017, MIT License.
// See third_party/slug/LICENSE-MIT and NOTICE.
// This adapter uses 32-bit storage buffers, not Slug's packed 2D textures.
struct Curve { p01: vec4f, p2: vec4f }
@group(0) @binding(0) var<uniform> viewport: vec4f;
@group(0) @binding(1) var<storage, read> curves: array<Curve>;
@group(0) @binding(2) var<storage, read> bands: array<vec2u>;
@group(0) @binding(3) var<storage, read> indices: array<u32>;
struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) color: vec4f,
    @location(2) @interpolate(flat) transform: vec4f,
    @location(3) @interpolate(flat) glyph_data: vec4u,
    @location(4) @interpolate(flat) clip: vec4f,
    @location(5) logical: vec2f,
}
@vertex fn vertex_main(@builtin(vertex_index) id:u32,
    @location(0) rect:vec4f, @location(1) uv:vec4f, @location(2) color:vec4f,
    @location(3) transform:vec4f, @location(4) glyph_data:vec4u,
    @location(5) clip:vec4f)->Vertex {
    let corners=array<vec2f,6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let c=corners[id];let p=rect.xy+c*rect.zw;
    return Vertex(vec4f(p.x/viewport.x*2-1,1-p.y/viewport.y*2,0,1),uv.xy+c*uv.zw,color,transform,glyph_data,clip,p);
}
fn root_code(y:vec3f)->u32 {
    let shift=(bitcast<u32>(y.x)>>31u)|((bitcast<u32>(y.y)>>30u)&2u)|((bitcast<u32>(y.z)>>29u)&4u);
    return (0x2e74u>>shift)&0x0101u;
}
fn solve(p0:vec2f,p1:vec2f,p2:vec2f)->vec2f {
    let a=p0-2*p1+p2;let b=p0-p1;
    var t:vec2f;
    if abs(a.y)<1.0/65536.0 && abs(b.y)>1e-12 {
        t=vec2f(p0.y/(2*b.y));
    } else {
        let d=sqrt(max(b.y*b.y-a.y*p0.y,0));
        t=vec2f(b.y-d,b.y+d)/a.y;
    }
    return (a.x*t-2*b.x)*t+p0.x;
}
fn ray(band:vec2u,uv:vec2f,pixels:vec2f,vertical:bool)->vec2f {
    var coverage=0.0;var weight=0.0;
    for(var i=0u;i<band.y;i++) {
        let curve=curves[indices[band.x+i]];
        var p0=curve.p01.xy-uv;var p1=curve.p01.zw-uv;var p2=curve.p2.xy-uv;
        var scale=pixels.x;
        if vertical {p0=p0.yx;p1=p1.yx;p2=p2.yx;scale=pixels.y;}
        if max(max(p0.x,p1.x),p2.x)*scale < -0.5 {break;}
        let code=root_code(vec3f(p0.y,p1.y,p2.y));
        if code!=0u {
            let r=solve(p0,p1,p2)*scale;
            if (code&1u)!=0u {coverage+=clamp(r.x+0.5,0,1);weight=max(weight,clamp(1-abs(r.x)*2,0,1));}
            if code>1u {coverage-=clamp(r.y+0.5,0,1);weight=max(weight,clamp(1-abs(r.y)*2,0,1));}
        }
    }
    if vertical {coverage = -coverage;}
    return vec2f(coverage,weight);
}
@fragment fn fragment_main(v:Vertex)->@location(0) vec4f {
    let logical=v.logical;
    if logical.x<v.clip.x || logical.y<v.clip.y || logical.x>=v.clip.x+v.clip.z || logical.y>=v.clip.y+v.clip.w {discard;}
    let pixels=1.0/max(fwidth(v.uv),vec2f(1e-9));
    let index=vec2u(clamp(v.uv*v.transform.xy+v.transform.zw,vec2f(0),vec2f(v.glyph_data.yz)-1));
    let h=ray(bands[v.glyph_data.x+index.y],v.uv,pixels,false);
    let y=ray(bands[v.glyph_data.x+v.glyph_data.z+index.x],v.uv,pixels,true);
    let coverage=clamp(max(abs(h.x*h.y+y.x*y.y)/max(h.y+y.y,1.0/65536.0),min(abs(h.x),abs(y.x))),0,1);
    return v.color*coverage;
}
