use crate::windfoil::BandedCurve;
use std::{vec, vec::Vec};
use ttf_parser::OutlineBuilder;

pub type Point = [f32; 2];
pub type Curve = [Point; 3];
pub const CUBIC_TOLERANCE: f32 = 1. / 8192.;
fn mix(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

pub struct Outline {
    pub curves: Vec<Curve>,
    pub failed: bool,
    scale: f32,
    current: Point,
    start: Point,
}
impl Outline {
    pub fn new(units: u16) -> Self {
        Self {
            curves: Vec::new(),
            failed: false,
            scale: 1. / units as f32,
            current: [0.; 2],
            start: [0.; 2],
        }
    }
    fn point(&self, x: f32, y: f32) -> Point {
        [x * self.scale, y * self.scale]
    }
    fn push(&mut self, curve: Curve) {
        if self.curves.len() >= u32::MAX as usize || curve.iter().flatten().any(|v| !v.is_finite())
        {
            self.failed = true;
        } else {
            self.curves.push(curve);
        }
    }
    fn cubic(&mut self, p: [Point; 4], depth: u32) {
        if self.failed {
            return;
        }
        let q = [
            (3. * p[1][0] + 3. * p[2][0] - p[0][0] - p[3][0]) * 0.25,
            (3. * p[1][1] + 3. * p[2][1] - p[0][1] - p[3][1]) * 0.25,
        ];
        // Compare the degree-elevated quadratic's control polygon. Its maximum
        // control-point distance bounds the error everywhere on the curve.
        let mut error2 = 0f32;
        for (control, end) in [(p[1], p[0]), (p[2], p[3])] {
            let d = [
                control[0] - (end[0] + 2. / 3. * (q[0] - end[0])),
                control[1] - (end[1] + 2. / 3. * (q[1] - end[1])),
            ];
            error2 = error2.max(d[0] * d[0] + d[1] * d[1]);
        }
        if error2 <= CUBIC_TOLERANCE * CUBIC_TOLERANCE {
            self.push([p[0], q, p[3]]);
            return;
        }
        if depth >= 16 {
            self.failed = true;
            return;
        }
        let a = mix(p[0], p[1]);
        let b = mix(p[1], p[2]);
        let c = mix(p[2], p[3]);
        let d = mix(a, b);
        let e = mix(b, c);
        let f = mix(d, e);
        self.cubic([p[0], a, d, f], depth + 1);
        self.cubic([f, e, c, p[3]], depth + 1);
    }
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.current = self.point(x, y);
        self.start = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let end = self.point(x, y);
        if end != self.current {
            self.push([self.current, end, end]);
        }
        self.current = end;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let end = self.point(x, y);
        self.push([self.current, self.point(x1, y1), end]);
        self.current = end;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let end = self.point(x, y);
        self.cubic(
            [self.current, self.point(x1, y1), self.point(x2, y2), end],
            0,
        );
        self.current = end;
    }
    fn close(&mut self) {
        if self.current != self.start {
            self.push([self.current, self.start, self.start]);
        }
        self.current = self.start;
    }
}

pub fn bounds(curves: &[Curve]) -> [f32; 4] {
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for p in curves.iter().flatten() {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    b
}

const TARGET_PER_BAND: usize = 10;
const MAX_BANDS: usize = 64;
pub const BAND_SORT_MIN: usize = 4;

fn extremum_t(p0: f32, p1: f32, p2: f32) -> Option<f32> {
    let a = p0 - 2. * p1 + p2;
    if a == 0. {
        return None;
    }
    let t = (p0 - p1) / a;
    (t > 0. && t < 1.).then_some(t)
}

fn subdivide(q: Curve, t: f32) -> [Curve; 2] {
    let lerp = |a: Point, b: Point| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let p01 = lerp(q[0], q[1]);
    let p12 = lerp(q[1], q[2]);
    let midpoint = lerp(p01, p12);
    [[q[0], p01, midpoint], [midpoint, p12, q[2]]]
}

/// Split quadratics at both component extrema, as required by Windfoil's
/// closed-form box integral. Every resulting piece is monotone in x and y.
pub fn monotone(curves: &[Curve]) -> Vec<Curve> {
    let mut result = Vec::with_capacity(curves.len() * 2);
    for &curve in curves {
        let tx = extremum_t(curve[0][0], curve[1][0], curve[2][0]);
        let ty = extremum_t(curve[0][1], curve[1][1], curve[2][1]);
        let mut splits = [tx.unwrap_or(2.), ty.unwrap_or(2.)];
        splits.sort_by(f32::total_cmp);
        let mut rest = curve;
        let mut consumed = 0.;
        for t in splits.into_iter().filter(|t| *t < 1.) {
            if t <= consumed {
                continue;
            }
            let local = (t - consumed) / (1. - consumed);
            if local > 0. && local < 1. {
                let [left, right] = subdivide(rest, local);
                result.push(left);
                rest = right;
                consumed = t;
            }
        }
        result.push(rest);
    }
    result
}

fn band_index(y: f32, y0: f32, inv_h: f32, count: usize) -> usize {
    if inv_h <= 0. {
        0
    } else {
        ((y - y0) * inv_h).floor().clamp(0., (count - 1) as f32) as usize
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
    let t = if (2. * a * r1 + b) * wanted >= 0. {
        r1
    } else {
        r2
    };
    t.clamp(0., 1.)
}

fn winding_area(pieces: &[Curve], bucket: &[usize], x0: f32, lo: f32, hi: f32) -> f64 {
    let mut area = 0.;
    for &i in bucket {
        let q = pieces[i];
        let low = (lo as f64).max((q[0][1].min(q[2][1])) as f64);
        let high = (hi as f64).min((q[0][1].max(q[2][1])) as f64);
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
        let c3 = 2. * ax * ay;
        let c2 = ax * by + 2. * bx * ay;
        let c1 = bx * by + 2. * cx * ay;
        let c0 = cx * by;
        let antiderivative = |t: f64| ((c3 / 4. * t + c2 / 3.) * t + c1 / 2.) * t * t + c0 * t;
        area += antiderivative(tb) - antiderivative(ta);
    }
    area
}

/// Windfoil row table. Curves are duplicated into the horizontal rows they
/// touch; each row is [curve start, count, density bits, x-min bits, x-max bits].
pub fn windfoil_bands(
    pieces: &[Curve],
    bounds: [f32; 4],
) -> (Vec<BandedCurve>, Vec<[u32; 5]>, f32, f32) {
    let count = if pieces.len() <= TARGET_PER_BAND {
        1
    } else {
        pieces.len().div_ceil(TARGET_PER_BAND).min(MAX_BANDS)
    };
    let inv_h = if count > 1 {
        count as f32 / (bounds[3] - bounds[1])
    } else {
        0.
    };
    let band_h = if count > 1 {
        1. / inv_h
    } else {
        bounds[3] - bounds[1]
    };
    let mut buckets = vec![Vec::new(); count];
    let mut x_left = f32::INFINITY;
    for (i, q) in pieces.iter().enumerate() {
        let y_min = q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let y_max = q.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
        let lo = band_index(y_min, bounds[1], inv_h, count);
        let hi = band_index(y_max, bounds[1], inv_h, count);
        for bucket in &mut buckets[lo..=hi] {
            bucket.push(i);
        }
        x_left = x_left.min(q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min));
    }
    let mut curves = Vec::new();
    let mut rows = Vec::with_capacity(count);
    for (band, mut bucket) in buckets.into_iter().enumerate() {
        let x_max = |i: usize| {
            pieces[i]
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        if bucket.len() > BAND_SORT_MIN {
            bucket.sort_by(|a, b| x_max(*b).total_cmp(&x_max(*a)));
        }
        let start = curves.len() as u32;
        let mut bx_min = 3e38f32;
        let mut bx_max = -3e38f32;
        for &i in &bucket {
            let q = pieces[i];
            bx_min = bx_min.min(q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min));
            bx_max = bx_max.max(q.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max));
            if q[0][1] != q[1][1] || q[0][1] != q[2][1] {
                curves.push(BandedCurve {
                    points: q,
                    first_row: band_index(
                        q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                        bounds[1],
                        inv_h,
                        count,
                    ) as u32,
                });
            }
        }
        let lo = bounds[1] + band as f32 * band_h;
        let hi = bounds[1] + (band + 1) as f32 * band_h;
        let area = winding_area(pieces, &bucket, x_left, lo, hi) as f32;
        let density = area / (band_h * (bx_max - bx_min)).max(1e-30);
        rows.push([
            start,
            (curves.len() as u32) - start,
            density.to_bits(),
            bx_min.to_bits(),
            bx_max.to_bits(),
        ]);
    }
    (curves, rows, band_h, inv_h)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cubic_is_subdivided_with_continuous_endpoints() {
        let mut o = Outline::new(1);
        o.move_to(0., 0.);
        o.curve_to(0., 1., 1., -1., 1., 0.);
        assert!(!o.failed);
        assert!(o.curves.len() > 4);
        assert_eq!(o.curves[0][0], [0., 0.]);
        assert_eq!(o.curves.last().unwrap()[2], [1., 0.]);
        for pair in o.curves.windows(2) {
            assert_eq!(pair[0][2], pair[1][0]);
        }
        // Nearest reference cubic point: x is monotone for this fixture.
        for c in &o.curves {
            for i in 0..=20 {
                let t = i as f32 / 20.;
                let p = [
                    (1. - t).powi(2) * c[0][0] + 2. * t * (1. - t) * c[1][0] + t * t * c[2][0],
                    (1. - t).powi(2) * c[0][1] + 2. * t * (1. - t) * c[1][1] + t * t * c[2][1],
                ];
                let (mut lo, mut hi) = (0f32, 1f32);
                for _ in 0..24 {
                    let m = (lo + hi) * 0.5;
                    let x = 3. * m * m - 2. * m * m * m;
                    if x < p[0] {
                        lo = m;
                    } else {
                        hi = m;
                    }
                }
                let s = (lo + hi) * 0.5;
                let y = 3. * s * (1. - s) * (1. - 2. * s);
                assert!((p[1] - y).abs() < 0.001);
            }
        }
    }
    #[test]
    fn windfoil_bands_split_and_file_monotone_curves() {
        let mut o = Outline::new(1);
        o.move_to(0., 0.);
        o.line_to(1., 0.);
        o.line_to(1., 1.);
        o.line_to(0., 1.);
        o.close();
        let pieces = monotone(&o.curves);
        let (banded, rows, _, _) = windfoil_bands(&pieces, bounds(&pieces));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1] as usize, banded.len());
        assert_eq!(banded.len(), 2); // horizontal edges carry no winding
    }
}
