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
    // geometry capacities. 16 steps keep error below 0.1 logical px at 4K.
    var lo = 0.0;
    var hi = 1.0;
    for (var iteration = 0; iteration < 16; iteration++) {
        let t = (lo + hi) * 0.5;
        if shoulder(t, p, a, b, end).x > u { lo = t; } else { hi = t; }
    }
    let t = (lo + hi) * 0.5;
    let h = 1 - t;
    let point = shoulder(t, p, a, b, end);
    let derivative = 3 * vec2f(
        -h*h*a - 2*h*t*b + t*t*(end.x - (p-a-b)),
        t*t*end.y);
    // Normalized implicit distance gives a one-physical-pixel AA transition;
    // unlike an unnormalized superellipse it does not thicken the corner AA.
    return (point.y - v) * abs(derivative.x) / max(length(derivative), 0.0001);
}
