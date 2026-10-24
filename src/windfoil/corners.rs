//! Vector counterpart of native/corners.wgsl's arc and cubic shoulders.
use crate::visual::CornerStyle;
use std::vec::Vec;

impl CornerStyle {
    /// Cubics for a right-angle corner in inward coordinates, from `[p, 0]`
    /// to `[0, p]`. `budget` is the available run on each adjacent edge.
    /// The same radius and smoothing reduction as the native visual shader apply.
    pub fn cubic_corner(self, budget: f32) -> Vec<[[f32; 2]; 4]> {
        let radius = self.radius.clamp(0., budget.max(0.));
        if radius <= 0.0001 {
            return Vec::new();
        }
        let smoothing = self
            .smoothing
            .clamp(0., 1.)
            .min((budget / radius - 1.).max(0.));
        let smoothing = if smoothing <= 0.0001 { 0. } else { smoothing };
        let p = radius * (1. + smoothing);
        let beta = smoothing * core::f32::consts::FRAC_PI_4;
        let (sin, cos) = beta.sin_cos();
        let end = [radius * (1. - sin), radius * (1. - cos)];
        let mirror = [end[1], end[0]];
        let mut curves = Vec::new();
        if smoothing > 0. {
            let tangent_run = end[1] / beta.tan();
            let b = (p - end[0] - tangent_run) / 3.;
            let a = 2. * b;
            curves.push([[p, 0.], [p - a, 0.], [p - a - b, 0.], end]);
        }
        let angle = core::f32::consts::FRAC_PI_2 - 2. * beta;
        if angle > 0. {
            let handle = 4. / 3. * (angle * 0.25).tan() * radius;
            curves.push([
                end,
                [end[0] - cos * handle, end[1] + sin * handle],
                [mirror[0] + sin * handle, mirror[1] - cos * handle],
                mirror,
            ]);
        }
        if smoothing > 0. {
            let first = curves[0];
            curves.push([
                mirror,
                [first[2][1], first[2][0]],
                [first[1][1], first[1][0]],
                [0., p],
            ]);
        }
        curves
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_corners_retain_shoulders_and_reduce_smoothing_only_when_space_requires_it() {
        for (percent, count, run) in [(0., 1, 16.), (60., 3, 25.6), (100., 2, 32.)] {
            let curves = CornerStyle::from_percent(16., percent).cubic_corner(40.);
            assert_eq!(curves.len(), count);
            assert!((curves[0][0][0] - run).abs() < 0.0001);
            assert_eq!(curves.last().unwrap()[3], [0., run]);
            assert!(curves.windows(2).all(|pair| pair[0][3] == pair[1][0]));
            assert!(
                curves
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|value| value.is_finite())
            );
        }
        assert_eq!(
            CornerStyle::from_percent(16., 100.).cubic_corner(16.),
            CornerStyle::from_percent(16., 0.).cubic_corner(16.)
        );
    }
}
