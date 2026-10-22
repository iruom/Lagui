// Windfoil box-filter coverage, adapted to Lagui's retained text pipeline.
// Algorithm and shader structure: Copyright 2026 Matt DesLauriers, Apache-2.0.
// https://github.com/texel-org/windfoil
// Curves are stored as padded structs here; the upstream implementation uses vec2 arrays.
// Instances carry an affine placement; the vertex stage derives the screen quad.

struct Curve { p01: vec4f, p2: vec4f }
@group(0) @binding(0) var<uniform> viewport: vec4f;
@group(0) @binding(1) var<storage, read> curves: array<Curve>;
@group(0) @binding(2) var<storage, read> rows: array<u32>;

const ROW_STRIDE: u32 = 5u;
const ROW_DENSITY: u32 = 2u;
const ROW_XMIN: u32 = 3u;
const ROW_XMAX: u32 = 4u;
const SORT_MIN: u32 = 4u;
const GUARD_PX: f32 = 3.7;

struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) rc: vec2f,
    @location(1) @interpolate(flat) color: vec4f,
    // First row index and row count of this outline's horizontal bands.
    @location(2) @interpolate(flat) row_range: vec2u,
    // Band height and its reciprocal in outline units.
    @location(3) @interpolate(flat) band_size: vec2f,
    @location(4) @interpolate(flat) bbox: vec4f,
}

// viewport.xy: logical size. viewport.z: physical pixels per logical pixel.
@vertex fn vertex_main(
    @builtin(vertex_index) id: u32,
    // origin.xy and scale.zw map outline units to logical pixels.
    @location(0) transform: vec4f,
    @location(1) bbox: vec4f,
    @location(2) clip: vec4f,
    @location(3) row_range: vec2u,
    @location(4) band_size: vec2f,
    @location(5) color: vec4f,
) -> Vertex {
    let corners = array<vec2f, 6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let origin = transform.xy;
    let scale = transform.zw;
    // One physical pixel of padding keeps the box-filter footprint inside the quad.
    let margin = 1 / viewport.z;
    let a = origin + bbox.xy * scale;
    let b = origin + bbox.zw * scale;
    let quad_lo = min(a, b) - margin;
    let quad_hi = max(a, b) + margin;
    // Clip the quad instead of discarding: the same pixel centers are covered.
    let lo = max(quad_lo, clip.xy);
    let hi = max(min(quad_hi, clip.xy + clip.zw), lo);
    let p = mix(lo, hi, corners[id]);
    let rc = (p - origin) / scale;
    return Vertex(vec4f(p.x/viewport.x*2-1, 1-p.y/viewport.y*2, 0, 1), rc, color, row_range, band_size, bbox);
}

fn mono_root(a2: f32, a1: f32, a0: f32, e1: f32, v: f32, rising: bool) -> f32 {
    if rising {
        if a0 >= v { return 0.0; }
        if e1 <= v { return 1.0; }
    } else {
        if a0 <= v { return 0.0; }
        if e1 >= v { return 1.0; }
    }
    let c = a0 - v;
    if abs(a2) < 1e-12 * max(abs(a1), 1.0) { return clamp(-c/a1, 0.0, 1.0); }
    let disc = max(a1*a1 - 4.0*a2*c, 0.0);
    let root = sqrt(disc);
    let q = -0.5 * (a1 + select(-root, root, a1 >= 0.0));
    let use_r1 = (a1 < 0.0) == rising;
    let num = select(c, q, use_r1);
    let den = select(q, a2, use_r1);
    let valid = den != 0.0;
    return clamp(select(0.0, num/select(1.0, den, valid), valid), 0.0, 1.0);
}

fn integrate_inside(a2: vec2f, a1: vec2f, x0: f32, ta: f32, tb: f32, hx: f32) -> f32 {
    if tb <= ta { return 0.0; }
    let tm = 0.5*(ta+tb);
    let d = 0.5*(tb-ta);
    let x_mid = (a2.x*tm+a1.x)*tm+x0+hx;
    let derivative = 2.0*a2*tm+a1;
    return 2.0*d*x_mid*derivative.y + (2.0*d*d*d/3.0)*(a2.x*derivative.y+2.0*a2.y*derivative.x);
}

fn integrate_piece(q1: vec2f, q2: vec2f, q3: vec2f, lo: f32, hi: f32, hx: f32) -> f32 {
    let a2 = q1-2.0*q2+q3;
    let a1 = 2.0*(q2-q1);
    let yr = q3.y >= q1.y;
    let xr = q3.x >= q1.x;
    let y0 = mono_root(a2.y,a1.y,q1.y,q3.y,select(hi,lo,yr),yr);
    let y1 = mono_root(a2.y,a1.y,q1.y,q3.y,select(lo,hi,yr),yr);
    if y1 <= y0 { return 0.0; }
    let xl = mono_root(a2.x,a1.x,q1.x,q3.x,-hx,xr);
    let xh = mono_root(a2.x,a1.x,q1.x,q3.x, hx,xr);
    let c0 = clamp(select(xh,xl,xr),y0,y1);
    let c1 = clamp(select(xl,xh,xr),y0,y1);
    let t1 = min(c0,c1);
    let t2 = max(c0,c1);
    var acc = integrate_inside(a2,a1,q1.x,t1,t2,hx);
    let right = select(vec2f(y0,t1),vec2f(t2,y1),xr);
    let d = max(right.y-right.x,0.0);
    let tm = 0.5*(right.x+right.y);
    acc += d*(2.0*a2.y*tm+a1.y)*(2.0*hx);
    return acc;
}

fn clipped_dy(a: f32, b: f32, lo: f32, hi: f32) -> f32 {
    return clamp(b,lo,hi)-clamp(a,lo,hi);
}

fn integrate_band(start: u32, count: u32, rc: vec2f, lo: f32, hi: f32, sx: f32) -> f32 {
    var acc = 0.0;
    let hx = sx*0.5;
    let sorted = count > SORT_MIN;
    let ulp = max(abs(rc.x),abs(rc.y))*1.2e-7;
    for (var i=0u; i<count; i=i+1u) {
        let q = curves[start+i];
        let q1 = q.p01.xy-rc;
        let q3 = q.p2.xy-rc;
        let xmax = max(q1.x,q3.x);
        if xmax <= -hx {
            if sorted { break; }
            continue;
        }
        let pylo = min(q1.y,q3.y);
        let pyhi = max(q1.y,q3.y);
        let wlo = max(lo,pylo);
        let whi = min(hi,pyhi);
        if whi <= wlo { continue; }
        let xmin = min(q1.x,q3.x);
        if xmin >= hx {
            acc += sx*clipped_dy(q1.y,q3.y,lo,hi);
            continue;
        }
        if xmax-xmin+(pyhi-pylo) <= ulp*16.0 {
            acc += (clamp((q1.x+q3.x)*0.5,-hx,hx)+hx)*clipped_dy(q1.y,q3.y,lo,hi);
            continue;
        }
        acc += integrate_piece(q1,q.p01.zw-rc,q3,wlo,whi,hx);
    }
    return acc;
}

fn band_index(dy: f32, inv_h: f32, count: u32) -> u32 {
    return u32(clamp(floor(dy*inv_h),0.0,f32(count)-1.0));
}
fn band_edges(base: f32, row: u32, h: f32) -> vec2f {
    return base+vec2f(f32(row),f32(row)+1.0)*h;
}
fn overlap(a0:f32,a1:f32,b0:f32,b1:f32)->f32 { return max(min(a1,b1)-max(a0,b0),0.0); }

fn integrate_face(rows_range: vec2u, band: vec2f, y0: f32, rc: vec2f, pixel: vec2f) -> f32 {
    let base=rows_range.x; let count=rows_range.y; let sy=pixel.y*0.5;
    let dy0=y0-rc.y;
    let first=band_index(-dy0-sy,band.y,count);
    let last=band_index(-dy0+sy,band.y,count);
    var area=0.0;
    for(var r=first;r<=last;r=r+1u) {
        var lo=-sy; var hi=sy;
        if count>1u { let edge=band_edges(dy0,r,band.x); lo=max(lo,edge.x); hi=min(hi,edge.y); }
        if hi<=lo { continue; }
        let index=(base+r)*ROW_STRIDE;
        area += integrate_band(rows[index],rows[index+1u],rc,lo,hi,pixel.x);
    }
    return area;
}

fn profile_face(rows_range:vec2u,band:vec2f,bbox:vec4f,rc:vec2f,pixel:vec2f)->f32 {
    let lo=rc-pixel*0.5; let hi=rc+pixel*0.5;
    if overlap(lo.x,hi.x,bbox.x,bbox.z)<=0.0 { return 0.0; }
    let base=rows_range.x; let count=rows_range.y;
    let first=band_index(lo.y-bbox.y,band.y,count);
    let last=band_index(hi.y-bbox.y,band.y,count);
    var ink=0.0;
    for(var r=first;r<=last;r=r+1u) {
        let index=(base+r)*ROW_STRIDE;
        let edge=band_edges(bbox.y,r,band.x);
        let oy=overlap(lo.y,hi.y,edge.x,edge.y);
        let ox=overlap(lo.x,hi.x,bitcast<f32>(rows[index+ROW_XMIN]),bitcast<f32>(rows[index+ROW_XMAX]));
        ink += bitcast<f32>(rows[index+ROW_DENSITY])*oy*ox;
    }
    return ink;
}

@fragment fn fragment_main(v:Vertex)->@location(0) vec4f {
    let pixel=max(fwidth(v.rc),vec2f(1e-9));
    let extent=v.bbox.zw-v.bbox.xy;
    var area: f32;
    if all(pixel*GUARD_PX>=extent) {
        area=profile_face(v.row_range,v.band_size,v.bbox,v.rc,pixel);
    } else {
        area=integrate_face(v.row_range,v.band_size,v.bbox.y,v.rc,pixel);
    }
    let coverage=clamp(abs(area)/(pixel.x*pixel.y),0.0,1.0);
    return v.color*coverage;
}
