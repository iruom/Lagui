// Figma's published circular-arc + cubic-shoulder construction:
// https://www.figma.com/blog/desperately-seeking-squircles/
// The two shoulders mirror across the corner diagonal. We fold the query
// into one shoulder, so every corner uses the same geometry.
fn shoulder(t: f32, p: f32, a: f32, b: f32, end: vec2f) -> vec2f {
    let h = 1 - t;
    return vec2f(
        h*h*h*p + 3*h*h*t*(p-a) + 3*h*t*t*(p-a-b) + t*t*t*end.x,
        t*t*t*end.y);
}

fn rounded_distance(centered: vec2f, extent: vec2f, radius_value: f32, smoothing_value: f32) -> f32 {
    let budget = min(extent.x, extent.y) * 0.5;
    let radius = clamp(radius_value, 0, budget);
    let q = abs(centered) - extent * 0.5 + vec2f(radius);
    // Retain the exact circular SDF for unsmoothed corners and pills.
    if radius <= 0.0001 {
        let square = abs(centered) - extent * 0.5;
        return length(max(square, vec2f(0))) + min(max(square.x, square.y), 0);
    }
    // Like Figma, reduce effective smoothing when adjacent corners lack room.
    let smoothing = min(clamp(smoothing_value, 0, 1), max(budget / radius - 1, 0));
    if smoothing <= 0.0001 {
        return length(max(q, vec2f(0))) + min(max(q.x, q.y), 0) - radius;
    }
    let inward = extent * 0.5 - abs(centered);
    let u = max(inward.x, inward.y);
    let v = min(inward.x, inward.y);
    let p = radius * (1 + smoothing);
    if u >= p { return -v; }
    let beta = smoothing * 0.7853981633974483;
    // Circular endpoint and tangent determine the cubic control points.
    let end = radius * vec2f(1 - sin(beta), 1 - cos(beta));
    if u <= end.x {
        return length(inward - vec2f(radius)) - radius;
    }
    let tangent_run = end.y / tan(beta);
    let b = (p - end.x - tangent_run) / 3;
    let a = 2 * b;
    // Monotone x permits a bounded numerical solve without tessellation or
    // geometry capacities. Bisection on x finds the curve point above the query;
    // its side gives the sign. Newton steps then move to the nearest curve point,
    // so the result is a Euclidean distance: offsets (strokes, spread) keep a
    // constant width through the corner, and the AA ramp stays one pixel wide.
    var lo = 0.0;
    var hi = 1.0;
    for (var iteration = 0; iteration < 12; iteration++) {
        let t = (lo + hi) * 0.5;
        if shoulder(t, p, a, b, end).x > u { lo = t; } else { hi = t; }
    }
    var t = (lo + hi) * 0.5;
    let outside = shoulder(t, p, a, b, end).y > v;
    let query = vec2f(u, v);
    for (var iteration = 0; iteration < 4; iteration++) {
        let h = 1 - t;
        let offset = shoulder(t, p, a, b, end) - query;
        let d1 = 3 * vec2f(-h*h*a - 2*h*t*b + t*t*(end.x - (p-a-b)), t*t*end.y);
        let d2 = 6 * vec2f(h*(a - b) + t*(end.x - p + a + 2*b), t*end.y);
        let slope = dot(d1, d1) + dot(offset, d2);
        if abs(slope) > 1e-8 {
            t = clamp(t - dot(offset, d1) / slope, 0, 1);
        }
    }
    var nearest = length(shoulder(t, p, a, b, end) - query);
    // The circular arc between the diagonal and the shoulder may be closer.
    let to_center = query - vec2f(radius);
    let center_distance = length(to_center);
    if center_distance > 0.0001 {
        let foot = vec2f(radius) + to_center * (radius / center_distance);
        if foot.x <= end.x {
            nearest = min(nearest, abs(center_distance - radius));
        }
    }
    if outside {
        return nearest;
    }
    // Inside, the straight edge beyond the shoulder (v = 0, x >= p) may be closer.
    return -min(nearest, length(vec2f(max(p - u, 0), v)));
}
