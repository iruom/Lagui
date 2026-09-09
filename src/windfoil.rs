//! General-purpose analytic vector rendering backed by the Windfoil shader.
//!
//! Paths live in caller-defined local coordinates. Closed paths can be used for
//! icons and other filled vector art; open quadratic/cubic paths can be stroked
//! for node-editor connections.

use crate::Rect;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
    vec,
    vec::Vec,
};
use wgpu::util::DeviceExt;

pub type Point = [f32; 2];
type Curve = [Point; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindfoilError {
    EmptyPath,
    InvalidGeometry,
    Capacity,
    WrongAtlas,
    AtlasOutOfDate,
}
impl core::fmt::Display for WindfoilError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "windfoil: {self:?}")
    }
}
impl std::error::Error for WindfoilError {}

#[derive(Clone, Copy)]
enum Segment {
    Line(Point, Point),
    Quad(Point, Point, Point),
    Cubic(Point, Point, Point, Point),
}

/// Allocation-backed path builder intended for retained vector assets.
#[derive(Default)]
pub struct Path {
    segments: Vec<Segment>,
    current: Option<Point>,
    start: Option<Point>,
}
impl Path {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn move_to(&mut self, p: Point) -> &mut Self {
        self.current = Some(p);
        self.start = Some(p);
        self
    }
    pub fn line_to(&mut self, p: Point) -> &mut Self {
        if let Some(a) = self.current {
            if a != p {
                self.segments.push(Segment::Line(a, p));
            }
        } else {
            self.start = Some(p);
        }
        self.current = Some(p);
        self
    }
    pub fn quadratic_to(&mut self, control: Point, p: Point) -> &mut Self {
        if let Some(a) = self.current {
            self.segments.push(Segment::Quad(a, control, p));
        } else {
            self.start = Some(p);
        }
        self.current = Some(p);
        self
    }
    pub fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) -> &mut Self {
        if let Some(a) = self.current {
            self.segments.push(Segment::Cubic(a, c1, c2, p));
        } else {
            self.start = Some(p);
        }
        self.current = Some(p);
        self
    }
    pub fn close(&mut self) -> &mut Self {
        if let (Some(a), Some(b)) = (self.current, self.start) {
            if a != b {
                self.segments.push(Segment::Line(a, b));
            }
            self.current = Some(b);
        }
        self
    }
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Shape {
    atlas: u64,
    bounds: [f32; 4],
    band_start: u32,
    bands: u32,
    band_h: f32,
    inv_h: f32,
}
impl Shape {
    pub fn bounds(self) -> [f32; 4] {
        self.bounds
    }
}

/// CPU-side atlas shared by any number of filled paths and stroked curves.
pub struct Atlas {
    id: u64,
    curves: Vec<Curve>,
    rows: Vec<[u32; 5]>,
}
impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}
impl Atlas {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            curves: Vec::new(),
            rows: Vec::new(),
        }
    }
    pub fn curve_count(&self) -> usize {
        self.curves.len()
    }
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
    pub fn add_fill(&mut self, path: &Path) -> Result<Shape, WindfoilError> {
        let mut curves = Vec::new();
        for &s in &path.segments {
            segment_quadratics(s, &mut curves);
        }
        self.insert(curves)
    }
    /// Converts a centerline to a round-capped/round-joined filled outline.
    /// Cubics are approximated by quadratics; their offset curves remain quadratic.
    pub fn add_stroke(&mut self, path: &Path, width: f32) -> Result<Shape, WindfoilError> {
        if !width.is_finite() || width <= 0. {
            return Err(WindfoilError::InvalidGeometry);
        }
        let mut center = Vec::new();
        for &s in &path.segments {
            segment_quadratics(s, &mut center);
        }
        if center.is_empty() {
            return Err(WindfoilError::EmptyPath);
        }
        let mut outline = Vec::new();
        let mut run_start = 0;
        for i in 1..=center.len() {
            if i == center.len() || center[i - 1][2] != center[i][0] {
                stroke_quadratics(&center[run_start..i], width * 0.5, &mut outline);
                run_start = i;
            }
        }
        self.insert(outline)
    }
    fn insert(&mut self, curves: Vec<Curve>) -> Result<Shape, WindfoilError> {
        if curves.is_empty() {
            return Err(WindfoilError::EmptyPath);
        }
        if curves.iter().flatten().flatten().any(|v| !v.is_finite()) {
            return Err(WindfoilError::InvalidGeometry);
        }
        let pieces = monotone(&curves);
        let bounds = bounds(&pieces);
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            return Err(WindfoilError::EmptyPath);
        }
        let (mut cs, mut rows, band_h, inv_h) = bands(&pieces, bounds);
        if self.curves.len() + cs.len() > 2_097_152 || self.rows.len() + rows.len() > 262_144 {
            return Err(WindfoilError::Capacity);
        }
        let shape = Shape {
            atlas: self.id,
            bounds,
            band_start: self.rows.len() as u32,
            bands: rows.len() as u32,
            band_h,
            inv_h,
        };
        let base = self.curves.len() as u32;
        for r in &mut rows {
            r[0] += base;
        }
        self.curves.append(&mut cs);
        self.rows.append(&mut rows);
        Ok(shape)
    }
}

fn midpoint(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}
fn cubic(p: [Point; 4], out: &mut Vec<Curve>, depth: u32) {
    let q = [
        (3. * p[1][0] + 3. * p[2][0] - p[0][0] - p[3][0]) * 0.25,
        (3. * p[1][1] + 3. * p[2][1] - p[0][1] - p[3][1]) * 0.25,
    ];
    let mut e = 0f32;
    for (c, end) in [(p[1], p[0]), (p[2], p[3])] {
        let d = [
            c[0] - (end[0] + 2. / 3. * (q[0] - end[0])),
            c[1] - (end[1] + 2. / 3. * (q[1] - end[1])),
        ];
        e = e.max(d[0] * d[0] + d[1] * d[1]);
    }
    if e <= 1e-8 || depth >= 16 {
        out.push([p[0], q, p[3]]);
        return;
    }
    let a = midpoint(p[0], p[1]);
    let b = midpoint(p[1], p[2]);
    let c = midpoint(p[2], p[3]);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let f = midpoint(d, e);
    cubic([p[0], a, d, f], out, depth + 1);
    cubic([f, e, c, p[3]], out, depth + 1);
}
fn segment_quadratics(s: Segment, out: &mut Vec<Curve>) {
    match s {
        Segment::Line(a, b) => out.push([a, b, b]),
        Segment::Quad(a, b, c) => out.push([a, b, c]),
        Segment::Cubic(a, b, c, d) => cubic([a, b, c, d], out, 0),
    }
}
fn normal(v: Point) -> Point {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    [-v[1] / len, v[0] / len]
}
fn unit(v: Point) -> Point {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    [v[0] / len, v[1] / len]
}

/// Adapted from Blend2D's `PathStroker::offset_quad_simple` (Zlib license).
/// Both sides remain quadratic instead of becoming a flattened line mesh.
fn offset_quad(q: Curve, r: f32) -> (Curve, Curve) {
    let v0 = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
    let v1 = [q[2][0] - q[1][0], q[2][1] - q[1][1]];
    let n0 = normal(if v0 == [0., 0.] { v1 } else { v0 });
    let n2 = normal(if v1 == [0., 0.] { v0 } else { v1 });
    let m = [n0[0] + n2[0], n0[1] + n2[1]];
    let m2 = m[0] * m[0] + m[1] * m[1];
    let k = if m2 > 1e-12 {
        [m[0] * (2. * r / m2), m[1] * (2. * r / m2)]
    } else {
        [n2[0] * r, n2[1] * r]
    };
    let s = [n0[0] * r, n0[1] * r];
    let e = [n2[0] * r, n2[1] * r];
    (
        [
            [q[0][0] + s[0], q[0][1] + s[1]],
            [q[1][0] + k[0], q[1][1] + k[1]],
            [q[2][0] + e[0], q[2][1] + e[1]],
        ],
        [
            [q[0][0] - s[0], q[0][1] - s[1]],
            [q[1][0] - k[0], q[1][1] - k[1]],
            [q[2][0] - e[0], q[2][1] - e[1]],
        ],
    )
}
fn round_cap(p: Point, from: Point, to: Point, forward: Point, out: &mut Vec<Curve>) {
    let tip = [p[0] + forward[0], p[1] + forward[1]];
    out.push([from, [from[0] + forward[0], from[1] + forward[1]], tip]);
    out.push([tip, [to[0] + forward[0], to[1] + forward[1]], to]);
}
fn stroke_quadratics(center: &[Curve], r: f32, out: &mut Vec<Curve>) {
    if center.is_empty() {
        return;
    }
    let mut left = Vec::with_capacity(center.len());
    let mut right = Vec::with_capacity(center.len());
    for &q in center {
        let (a, b) = offset_quad(q, r);
        left.push(a);
        right.push(b)
    }
    // Independently calculated normals can differ by a few ULPs. Keep the
    // winding contour exactly closed instead of leaving ray-producing seams.
    for i in 1..left.len() {
        let p = midpoint(left[i - 1][2], left[i][0]);
        left[i - 1][2] = p;
        left[i][0] = p;
        let p = midpoint(right[i - 1][2], right[i][0]);
        right[i - 1][2] = p;
        right[i][0] = p;
    }
    out.extend(left.iter().copied());
    let q = *center.last().unwrap();
    let end_tangent = [q[2][0] - q[1][0], q[2][1] - q[1][1]];
    let t = unit(if end_tangent == [0., 0.] {
        [q[1][0] - q[0][0], q[1][1] - q[0][1]]
    } else {
        end_tangent
    });
    round_cap(
        q[2],
        left.last().unwrap()[2],
        right.last().unwrap()[2],
        [t[0] * r, t[1] * r],
        out,
    );
    out.extend(right.iter().rev().map(|q| [q[2], q[1], q[0]]));
    let q = center[0];
    let start_tangent = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
    let t = unit(if start_tangent == [0., 0.] {
        [q[2][0] - q[1][0], q[2][1] - q[1][1]]
    } else {
        start_tangent
    });
    round_cap(q[0], right[0][0], left[0][0], [-t[0] * r, -t[1] * r], out);
}
fn extremum(p0: f32, p1: f32, p2: f32) -> Option<f32> {
    let a = p0 - 2. * p1 + p2;
    if a == 0. {
        None
    } else {
        let t = (p0 - p1) / a;
        (t > 0. && t < 1.).then_some(t)
    }
}
fn subdivide(q: Curve, t: f32) -> [Curve; 2] {
    let l = |a: Point, b: Point| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let a = l(q[0], q[1]);
    let b = l(q[1], q[2]);
    let m = l(a, b);
    [[q[0], a, m], [m, b, q[2]]]
}
fn monotone(curves: &[Curve]) -> Vec<Curve> {
    let mut o = Vec::new();
    for &q in curves {
        let mut ts = [
            extremum(q[0][0], q[1][0], q[2][0]).unwrap_or(2.),
            extremum(q[0][1], q[1][1], q[2][1]).unwrap_or(2.),
        ];
        ts.sort_by(f32::total_cmp);
        let (mut rest, mut used) = (q, 0.);
        for t in ts.into_iter().filter(|t| *t < 1.) {
            if t <= used {
                continue;
            }
            let local = (t - used) / (1. - used);
            let [l, r] = subdivide(rest, local);
            o.push(l);
            rest = r;
            used = t;
        }
        o.push(rest)
    }
    o
}
fn bounds(cs: &[Curve]) -> [f32; 4] {
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for p in cs.iter().flatten() {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    b
}
fn band_index(y: f32, y0: f32, inv: f32, n: usize) -> usize {
    if inv <= 0. {
        0
    } else {
        ((y - y0) * inv).floor().clamp(0., (n - 1) as f32) as usize
    }
}
fn mono_root(a: f64, b: f64, e0: f64, e1: f64, value: f64, rising: bool) -> f64 {
    if if rising { e0 >= value } else { e0 <= value } {
        return 0.;
    }
    if if rising { e1 <= value } else { e1 >= value } {
        return 1.;
    }
    let c = e0 - value;
    if a.abs() < 1e-12 * b.abs().max(1.) {
        return (-c / b).clamp(0., 1.);
    }
    let root = (b * b - 4. * a * c).max(0.).sqrt();
    let q = -0.5 * (b + if b >= 0. { root } else { -root });
    let r1 = q / a;
    let r2 = if q != 0. { c / q } else { 0. };
    let wanted = if rising { 1. } else { -1. };
    (if (2. * a * r1 + b) * wanted >= 0. {
        r1
    } else {
        r2
    })
    .clamp(0., 1.)
}
fn winding_area(pieces: &[Curve], bucket: &[usize], x0: f32, lo: f32, hi: f32) -> f64 {
    let mut area = 0.;
    for &i in bucket {
        let q = pieces[i];
        let low = (lo as f64).max(q[0][1].min(q[2][1]) as f64);
        let high = (hi as f64).min(q[0][1].max(q[2][1]) as f64);
        if high <= low {
            continue;
        }
        let rising = q[2][1] >= q[0][1];
        let ay = (q[0][1] - 2. * q[1][1] + q[2][1]) as f64;
        let by = (2. * (q[1][1] - q[0][1])) as f64;
        let ta = mono_root(
            ay,
            by,
            q[0][1] as f64,
            q[2][1] as f64,
            if rising { low } else { high },
            rising,
        );
        let tb = mono_root(
            ay,
            by,
            q[0][1] as f64,
            q[2][1] as f64,
            if rising { high } else { low },
            rising,
        );
        if tb <= ta {
            continue;
        }
        let ax = (q[0][0] - 2. * q[1][0] + q[2][0]) as f64;
        let bx = (2. * (q[1][0] - q[0][0])) as f64;
        let cx = (q[0][0] - x0) as f64;
        let (c3, c2, c1, c0) = (
            2. * ax * ay,
            ax * by + 2. * bx * ay,
            bx * by + 2. * cx * ay,
            cx * by,
        );
        let f = |t: f64| ((c3 / 4. * t + c2 / 3.) * t + c1 / 2.) * t * t + c0 * t;
        area += f(tb) - f(ta);
    }
    area
}
fn bands(pieces: &[Curve], b: [f32; 4]) -> (Vec<Curve>, Vec<[u32; 5]>, f32, f32) {
    let n = pieces.len().div_ceil(10).clamp(1, 64);
    let inv = if n > 1 { n as f32 / (b[3] - b[1]) } else { 0. };
    let h = if n > 1 { 1. / inv } else { b[3] - b[1] };
    let mut buckets = vec![Vec::new(); n];
    for (i, q) in pieces.iter().enumerate() {
        let lo = band_index(
            q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
            b[1],
            inv,
            n,
        );
        let hi = band_index(
            q.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max),
            b[1],
            inv,
            n,
        );
        for bucket in &mut buckets[lo..=hi] {
            bucket.push(i)
        }
    }
    let mut cs = Vec::new();
    let mut rows = Vec::new();
    let x_left = pieces
        .iter()
        .flatten()
        .map(|p| p[0])
        .fold(f32::INFINITY, f32::min);
    for (band, mut bucket) in buckets.into_iter().enumerate() {
        let xmax = |i: usize| {
            pieces[i]
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        if bucket.len() > 4 {
            bucket.sort_by(|a, b| xmax(*b).total_cmp(&xmax(*a)))
        }
        let start = cs.len() as u32;
        let (mut xmin, mut xmaxv) = (f32::INFINITY, f32::NEG_INFINITY);
        for &i in &bucket {
            let q = pieces[i];
            xmin = xmin.min(q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min));
            xmaxv = xmaxv.max(q.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max));
            if q[0][1] != q[1][1] || q[0][1] != q[2][1] {
                cs.push(q)
            }
        }
        let lo = b[1] + band as f32 * h;
        let hi = b[1] + (band + 1) as f32 * h;
        let density =
            winding_area(pieces, &bucket, x_left, lo, hi) as f32 / (h * (xmaxv - xmin)).max(1e-30);
        rows.push([
            start,
            cs.len() as u32 - start,
            density.to_bits(),
            xmin.to_bits(),
            xmaxv.to_bits(),
        ]);
    }
    (cs, rows, h, inv)
}

pub struct Instance {
    pub shape: Shape,
    pub origin: Point,
    pub scale: Point,
    pub color: [f32; 4],
    pub clip: Option<Rect>,
}
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    viewport: wgpu::Buffer,
    instances: wgpu::Buffer,
    binding: wgpu::BindGroup,
    bytes: Vec<u8>,
    capacity: usize,
    count: u32,
    uploaded: (u64, usize, usize),
}
fn storage(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    u: &wgpu::Buffer,
    c: &wgpu::Buffer,
    r: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Windfoil vector data"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: u.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: c.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: r.as_entire_binding(),
            },
        ],
    })
}
impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Result<Self, WindfoilError> {
        if capacity == 0 || capacity > 16384 {
            return Err(WindfoilError::Capacity);
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Windfoil vectors"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../native/windfoil.wgsl").into()),
        });
        let entries: [wgpu::BindGroupLayoutEntry; 3] =
            core::array::from_fn(|i| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: if i == 0 {
                    wgpu::ShaderStages::VERTEX_FRAGMENT
                } else {
                    wgpu::ShaderStages::FRAGMENT
                },
                ty: wgpu::BindingType::Buffer {
                    ty: if i == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage { read_only: true }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new([16, 32, 20][i]),
                },
                count: None,
            });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Windfoil vector layout"),
            entries: &entries,
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Windfoil vector pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("Windfoil vector pipeline"),layout:Some(&pl),vertex:wgpu::VertexState{module:&shader,entry_point:Some("vertex_main"),compilation_options:Default::default(),buffers:&[Some(wgpu::VertexBufferLayout{array_stride:96,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4]})]},fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fragment_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),primitive:Default::default(),depth_stencil:None,multisample:Default::default(),multiview_mask:None,cache:None});
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vector viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vector instances"),
            size: (capacity * 96) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let c = storage(device, "empty vector curves", &[0; 32]);
        let r = storage(device, "empty vector rows", &[0; 20]);
        let binding = bind(device, &layout, &viewport, &c, &r);
        Ok(Self {
            pipeline,
            layout,
            viewport,
            instances,
            binding,
            bytes: Vec::with_capacity(capacity * 96),
            capacity,
            count: 0,
            uploaded: (0, 0, 0),
        })
    }
    pub fn sync_atlas(&mut self, device: &wgpu::Device, atlas: &Atlas) {
        let rev = (atlas.id, atlas.curves.len(), atlas.rows.len());
        if rev == self.uploaded {
            return;
        }
        let mut cb = Vec::with_capacity(atlas.curves.len() * 32);
        for c in &atlas.curves {
            for p in c {
                for v in p {
                    cb.extend_from_slice(&v.to_le_bytes())
                }
            }
            cb.extend_from_slice(&[0; 8])
        }
        if cb.is_empty() {
            cb.resize(32, 0)
        }
        let mut rb = Vec::with_capacity(atlas.rows.len() * 20);
        for r in &atlas.rows {
            for v in r {
                rb.extend_from_slice(&v.to_le_bytes())
            }
        }
        if rb.is_empty() {
            rb.resize(20, 0)
        }
        let c = storage(device, "vector curves", &cb);
        let r = storage(device, "vector rows", &rb);
        self.binding = bind(device, &self.layout, &self.viewport, &c, &r);
        self.uploaded = rev;
        self.count = 0;
    }
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        items: &[Instance],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), WindfoilError> {
        self.bytes.clear();
        self.count = 0;
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            return Err(WindfoilError::InvalidGeometry);
        }
        for item in items {
            if item.shape.atlas != self.uploaded.0 {
                return Err(WindfoilError::WrongAtlas);
            }
            if item.shape.band_start as usize + item.shape.bands as usize > self.uploaded.2 {
                return Err(WindfoilError::AtlasOutOfDate);
            }
            if self.bytes.len() / 96 >= self.capacity {
                return Err(WindfoilError::Capacity);
            }
            if item
                .origin
                .iter()
                .chain(item.scale.iter())
                .chain(item.color.iter())
                .any(|v| !v.is_finite())
                || item.scale[0] == 0.
                || item.scale[1] == 0.
            {
                return Err(WindfoilError::InvalidGeometry);
            }
            let b = item.shape.bounds;
            let margin = 1. / scale_factor;
            let pad = [margin / item.scale[0].abs(), margin / item.scale[1].abs()];
            let x0 = item.origin[0] + b[0] * item.scale[0];
            let x1 = item.origin[0] + b[2] * item.scale[0];
            let y0 = item.origin[1] + b[1] * item.scale[1];
            let y1 = item.origin[1] + b[3] * item.scale[1];
            let rect = [
                x0.min(x1) - margin,
                y0.min(y1) - margin,
                (x1 - x0).abs() + 2. * margin,
                (y1 - y0).abs() + 2. * margin,
            ];
            let local = [
                if item.scale[0] > 0. {
                    b[0] - pad[0]
                } else {
                    b[2] + pad[0]
                },
                if item.scale[1] > 0. {
                    b[1] - pad[1]
                } else {
                    b[3] + pad[1]
                },
                (b[2] - b[0] + 2. * pad[0]) * item.scale[0].signum(),
                (b[3] - b[1] + 2. * pad[1]) * item.scale[1].signum(),
            ];
            let clip = item
                .clip
                .unwrap_or(Rect::new(0., 0., viewport[0], viewport[1]));
            let vals = [
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                local[0],
                local[1],
                local[2],
                local[3],
                item.color[0],
                item.color[1],
                item.color[2],
                item.color[3],
                item.shape.band_start as f32,
                item.shape.bands as f32,
                item.shape.band_h,
                item.shape.inv_h,
                b[0],
                b[1],
                b[2],
                b[3],
                clip.x,
                clip.y,
                clip.width,
                clip.height,
            ];
            if vals.iter().any(|v| !v.is_finite()) {
                return Err(WindfoilError::InvalidGeometry);
            }
            for v in vals {
                self.bytes.extend_from_slice(&v.to_le_bytes())
            }
        }
        let mut screen = [0; 16];
        screen[..4].copy_from_slice(&viewport[0].to_le_bytes());
        screen[4..8].copy_from_slice(&viewport[1].to_le_bytes());
        screen[8..12].copy_from_slice(&scale_factor.to_le_bytes());
        queue.write_buffer(&self.viewport, 0, &screen);
        if !self.bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.bytes)
        }
        self.count = (self.bytes.len() / 96) as u32;
        Ok(())
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.count)
    }
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        let a = range.start.min(self.count);
        let b = range.end.min(self.count);
        if a >= b {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, a..b)
    }
    pub fn prepared_len(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_icon_and_node_curve() {
        let mut atlas = Atlas::new();
        let mut icon = Path::new();
        icon.move_to([0., 0.])
            .line_to([1., 0.])
            .line_to([0.5, 1.])
            .close();
        let shape = atlas.add_fill(&icon).unwrap();
        assert!(shape.bands > 0);
        assert_ne!(f32::from_bits(atlas.rows[0][2]), 0.0);
        let mut edge = Path::new();
        edge.move_to([0., 0.])
            .cubic_to([40., 0.], [60., 100.], [100., 100.]);
        let stroke = atlas.add_stroke(&edge, 3.).unwrap();
        assert!(stroke.bounds[2] > 100.);
        assert!(atlas.curve_count() > 3);
    }

    #[test]
    fn stroke_caps_extend_outward_and_contour_is_closed() {
        let center = [[[0., 0.], [10., 0.], [10., 0.]]];
        let mut outline = Vec::new();
        stroke_quadratics(&center, 1., &mut outline);
        assert!(outline.windows(2).all(|pair| pair[0][2] == pair[1][0]));
        assert_eq!(outline.last().unwrap()[2], outline[0][0]);
        let b = bounds(&outline);
        assert!((b[0] + 1.).abs() < 1e-6);
        assert!((b[2] - 11.).abs() < 1e-6);
    }
}
