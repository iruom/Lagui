//! General-purpose analytic vector rendering backed by the Windfoil shader.
//!
//! Paths live in caller-defined local coordinates. Closed paths can be used for
//! icons and other filled vector art; open quadratic/cubic paths can be stroked
//! for node-editor connections.

mod corners;
mod gpu;

use crate::{Rect, gpu::Damage};
pub(crate) use gpu::{Analytic, CoreError, Placement, STRIDE};
pub use gpu::{PERCEPTUAL_COVERAGE_GAMMA, Pipeline};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
    vec,
    vec::Vec,
};

pub type Point = [f32; 2];
type Curve = [Point; 3];

/// A row-local copy retains the first row of its original monotone piece.
/// A pixel spanning several rows integrates that piece once, at its first
/// overlapping row, without changing winding multiplicity or source geometry.
#[derive(Clone, Copy)]
pub(crate) struct BandedCurve {
    pub points: Curve,
    pub first_row: u32,
}

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
    curves: Vec<BandedCurve>,
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
        // Curve addresses are u32; band addresses are encoded as f32 in instances.
        // These are format limits, not arbitrary allocation capacities.
        if self
            .curves
            .len()
            .checked_add(cs.len())
            .is_none_or(|len| len > u32::MAX as usize)
            || self
                .rows
                .len()
                .checked_add(rows.len())
                .is_none_or(|len| len > (1 << 24))
        {
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
        for curve in &mut cs {
            curve.first_row += shape.band_start;
        }
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
    // Subdivide in higher precision: at ordinary window coordinates, f32 ULPs
    // exceed the approximation tolerance and otherwise cause runaway splitting.
    cubic_precise(p.map(|point| point.map(f64::from)), out, depth);
}
fn cubic_precise(p: [[f64; 2]; 4], out: &mut Vec<Curve>, depth: u32) {
    let midpoint = |a: [f64; 2], b: [f64; 2]| [a[0] * 0.5 + b[0] * 0.5, a[1] * 0.5 + b[1] * 0.5];
    let q = [
        (3. * p[1][0] + 3. * p[2][0] - p[0][0] - p[3][0]) * 0.25,
        (3. * p[1][1] + 3. * p[2][1] - p[0][1] - p[3][1]) * 0.25,
    ];
    let mut e = 0f64;
    for (c, end) in [(p[1], p[0]), (p[2], p[3])] {
        let d = [
            c[0] - (end[0] + 2. / 3. * (q[0] - end[0])),
            c[1] - (end[1] + 2. / 3. * (q[1] - end[1])),
        ];
        e = e.max(d[0] * d[0] + d[1] * d[1]);
    }
    if e <= 1e-8 || depth >= 16 {
        out.push([p[0], q, p[3]].map(|point| point.map(|value| value as f32)));
        return;
    }
    let a = midpoint(p[0], p[1]);
    let b = midpoint(p[1], p[2]);
    let c = midpoint(p[2], p[3]);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let f = midpoint(d, e);
    cubic_precise([p[0], a, d, f], out, depth + 1);
    cubic_precise([f, e, c, p[3]], out, depth + 1);
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
fn bands(pieces: &[Curve], b: [f32; 4]) -> (Vec<BandedCurve>, Vec<[u32; 5]>, f32, f32) {
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
                cs.push(BandedCurve {
                    points: q,
                    first_row: band_index(
                        q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                        b[1],
                        inv,
                        n,
                    ) as u32,
                })
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

#[derive(Clone)]
pub struct Instance {
    pub transform: crate::gpu::TransformId,
    pub shape: Shape,
    pub origin: Point,
    pub scale: Point,
    pub color: [f32; 4],
    pub clip: Option<Rect>,
}

/// Vector shapes from one [`Atlas`], drawn through the shared Windfoil [`Pipeline`].
pub struct Renderer {
    device: wgpu::Device,
    core: Analytic,
    cpu_source: Option<(u64, usize, usize)>,
}

impl From<CoreError> for WindfoilError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Capacity => Self::Capacity,
            CoreError::InvalidGeometry => Self::InvalidGeometry,
        }
    }
}

impl Renderer {
    /// Compiles a private pipeline. Prefer [`Self::with_pipeline`] when the
    /// application already owns one for this format.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Result<Self, WindfoilError> {
        Self::with_pipeline(device, &Pipeline::new(device, format), capacity)
    }

    pub fn with_pipeline(
        device: &wgpu::Device,
        pipeline: &Pipeline,
        capacity: usize,
    ) -> Result<Self, WindfoilError> {
        Ok(Self {
            device: device.clone(),
            cpu_source: None,
            core: Analytic::new(device, pipeline, "vector instances", capacity)
                .map_err(|_| WindfoilError::Capacity)?,
        })
    }

    /// Grow instance storage while preserving the compiled pipeline and atlas.
    /// Submitted frames retain their old buffer through wgpu's resource lifetime.
    pub fn reserve(
        &mut self,
        _device: &wgpu::Device,
        required: usize,
    ) -> Result<(), WindfoilError> {
        self.core
            .reserve(required)
            .map_err(|_| WindfoilError::Capacity)
    }

    pub fn sync_atlas(
        &mut self,
        device: &wgpu::Device,
        atlas: &Atlas,
    ) -> Result<(), WindfoilError> {
        self.core
            .sync(device, atlas.id, &atlas.curves, &atlas.rows)
            .map_err(|_| WindfoilError::Capacity)
    }

    /// Encodes all instances; only instances that changed since the previous
    /// prepare are uploaded. See [`Self::damage`].
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        items: &[Instance],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), WindfoilError> {
        self.prepare_impl(queue, items, viewport, scale_factor, None)
    }
    pub fn prepare_transformed(
        &mut self,
        queue: &wgpu::Queue,
        items: &[Instance],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: &crate::gpu::Transforms,
    ) -> Result<(), WindfoilError> {
        self.prepare_impl(queue, items, viewport, scale_factor, Some(transforms))
    }
    fn prepare_impl(
        &mut self,
        queue: &wgpu::Queue,
        items: &[Instance],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: Option<&crate::gpu::Transforms>,
    ) -> Result<(), WindfoilError> {
        if items.iter().any(|i| {
            transforms.map_or(i.transform != crate::gpu::TransformId::IDENTITY, |t| {
                t.get(i.transform).is_none()
            })
        }) {
            self.core.abandon();
            return Err(WindfoilError::InvalidGeometry);
        }
        let result = self.encode(items, viewport, scale_factor, None);
        match result {
            Ok(()) => self
                .core
                .commit_transformed(queue, viewport, scale_factor, transforms),
            Err(_) => self.core.abandon(),
        }
        result
    }

    fn encode(
        &mut self,
        items: &[Instance],
        viewport: [f32; 2],
        scale_factor: f32,
        source: Option<(u64, usize, usize)>,
    ) -> Result<(), WindfoilError> {
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            return Err(WindfoilError::InvalidGeometry);
        }
        let device = self.device.clone();
        self.reserve(&device, items.len())?;
        let (atlas, _, rows) = source.unwrap_or_else(|| self.core.uploaded());
        self.core.begin();
        for item in items {
            if item.shape.atlas != atlas {
                return Err(WindfoilError::WrongAtlas);
            }
            if item.shape.band_start as usize + item.shape.bands as usize > rows {
                return Err(WindfoilError::AtlasOutOfDate);
            }
            self.core.push(&Placement {
                transform: item.transform,
                baseline: None,
                origin: item.origin,
                scale: item.scale,
                bounds: item.shape.bounds,
                band_start: item.shape.band_start,
                bands: item.shape.bands,
                band_h: item.shape.band_h,
                inv_h: item.shape.inv_h,
                color: item.color,
                clip: item
                    .clip
                    .unwrap_or(Rect::new(0., 0., viewport[0], viewport[1])),
            })?;
        }
        Ok(())
    }

    pub fn prepare_cpu(
        &mut self,
        atlas: &Atlas,
        items: &[Instance],
        viewport: [f32; 2],
        dpi: f32,
    ) -> Result<(), WindfoilError> {
        let source = (atlas.id, atlas.curves.len(), atlas.rows.len());
        self.cpu_source = None;
        if let Err(error) = self.encode(items, viewport, dpi, Some(source)) {
            self.core.abandon();
            return Err(error);
        }
        self.cpu_source = Some(source);
        Ok(())
    }
    pub fn prepared_source<'a>(
        &'a self,
        atlas: &'a Atlas,
    ) -> Result<crate::gpu::outlines::PreparedOutline<'a>, WindfoilError> {
        if self.cpu_source != Some((atlas.id, atlas.curves.len(), atlas.rows.len())) {
            return Err(WindfoilError::AtlasOutOfDate);
        }
        Ok(crate::gpu::outlines::PreparedOutline::new(
            1,
            atlas.id,
            &atlas.curves,
            &atlas.rows,
            self.core.encoded(),
        ))
    }

    /// Edge coverage gamma, [`PERCEPTUAL_COVERAGE_GAMMA`] by default; `1.0`
    /// restores linear coverage. Takes effect on the next prepare.
    pub fn set_coverage_gamma(&mut self, gamma: f32) {
        self.core.set_coverage_gamma(gamma);
    }

    /// Region changed by the latest prepare, for hosts that redraw partially.
    pub fn upload_stats(&self) -> crate::gpu::stats::UploadStats {
        self.core.upload_stats()
    }

    pub fn damage(&self) -> Damage {
        self.core.damage()
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.core.count())
    }

    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        self.core.draw_range(pass, range)
    }

    /// Allocated instance slots, owned by this renderer's GPU buffer.
    pub fn capacity(&self) -> usize {
        self.core.capacity()
    }

    pub fn prepared_len(&self) -> u32 {
        self.core.count()
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
    fn atlas_grows_past_previous_curve_and_row_limits() {
        let mut atlas = Atlas::new();
        atlas.curves.resize(
            2_097_152,
            BandedCurve {
                points: [[0.; 2]; 3],
                first_row: 0,
            },
        );
        atlas.rows.resize(262_144, [0; 5]);
        let mut path = Path::new();
        path.move_to([0., 0.])
            .line_to([1., 0.])
            .line_to([0., 1.])
            .close();
        let shape = atlas.add_fill(&path).unwrap();
        assert_eq!(shape.band_start, 262_144);
        assert!(atlas.curve_count() > 2_097_152);
        assert!(atlas.row_count() > 262_144);
        assert!(atlas.rows[shape.band_start as usize][0] >= 2_097_152);
    }

    #[test]
    fn cubic_subdivision_is_stable_when_window_coordinates_change() {
        let source = [[0., 0.], [0., 12.], [12., 12.], [12., 0.]];
        let mut local = Vec::new();
        cubic(source, &mut local, 0);
        for origin in [640., 1024., 1280., 1536., 1920., 2560., 4096.] {
            let shifted = source.map(|p| [p[0] + origin, p[1] + origin]);
            let mut translated = Vec::new();
            cubic(shifted, &mut translated, 0);
            assert!(
                translated.len() <= local.len() * 2,
                "origin {origin}: {} curves versus {} locally",
                translated.len(),
                local.len()
            );
        }
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
