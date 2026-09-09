use crate::Rect;

/// Backend-independent gesture after an OS adapter has converted coordinates
/// to logical pixels and pinch magnification to a positive relative factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportGesture {
    Pan { delta: [f32; 2] },
    Zoom { anchor: [f32; 2], factor: f32 },
}

/// Reusable camera for an unbounded 2D editor surface.
///
/// World `(0, 0)` maps to the center of `view` plus `offset`. Scale has no
/// application-defined minimum or maximum; only non-finite and non-positive
/// values are rejected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport2D {
    pub offset: [f32; 2],
    pub scale: f32,
}

impl Default for Viewport2D {
    fn default() -> Self {
        Self {
            offset: [0.; 2],
            scale: 1.,
        }
    }
}

impl Viewport2D {
    pub fn apply(&mut self, view: Rect, gesture: ViewportGesture) -> bool {
        match gesture {
            ViewportGesture::Pan { delta } => self.pan(delta),
            ViewportGesture::Zoom { anchor, factor } => self.zoom_at(view, anchor, factor),
        }
    }

    pub fn center(view: Rect) -> [f32; 2] {
        [view.x + view.width * 0.5, view.y + view.height * 0.5]
    }

    pub fn origin(self, view: Rect) -> [f32; 2] {
        let center = Self::center(view);
        [center[0] + self.offset[0], center[1] + self.offset[1]]
    }

    pub fn world_to_screen(self, view: Rect, world: [f32; 2]) -> [f32; 2] {
        let origin = self.origin(view);
        [
            origin[0] + world[0] * self.scale,
            origin[1] + world[1] * self.scale,
        ]
    }

    pub fn screen_to_world(self, view: Rect, screen: [f32; 2]) -> [f32; 2] {
        let origin = self.origin(view);
        [
            (screen[0] - origin[0]) / self.scale,
            (screen[1] - origin[1]) / self.scale,
        ]
    }

    pub fn pan(&mut self, screen_delta: [f32; 2]) -> bool {
        if screen_delta.iter().any(|v| !v.is_finite()) {
            return false;
        }
        self.offset[0] += screen_delta[0];
        self.offset[1] += screen_delta[1];
        self.offset.iter().all(|v| v.is_finite())
    }

    /// Changes scale while keeping the world position below `anchor` fixed.
    pub fn set_scale_at(&mut self, view: Rect, anchor: [f32; 2], scale: f32) -> bool {
        if !scale.is_finite() || scale <= 0. || !self.scale.is_finite() || self.scale <= 0. {
            return false;
        }
        let center = Self::center(view);
        let ratio = scale / self.scale;
        let offset = [
            self.offset[0] + (anchor[0] - center[0] - self.offset[0]) * (1. - ratio),
            self.offset[1] + (anchor[1] - center[1] - self.offset[1]) * (1. - ratio),
        ];
        if offset.iter().any(|v| !v.is_finite()) {
            return false;
        }
        self.offset = offset;
        self.scale = scale;
        true
    }

    /// Applies a relative magnification factor at a screen-space anchor.
    pub fn zoom_at(&mut self, view: Rect, anchor: [f32; 2], factor: f32) -> bool {
        if !factor.is_finite() || factor <= 0. {
            return false;
        }
        self.set_scale_at(view, anchor, self.scale * factor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_preserves_the_anchored_world_point_without_fixed_limits() {
        let view = Rect::new(100., 50., 800., 600.);
        let anchor = [731., 192.];
        let mut camera = Viewport2D::default();
        let world = camera.screen_to_world(view, anchor);
        assert!(camera.zoom_at(view, anchor, 10_000.));
        let after = camera.screen_to_world(view, anchor);
        assert!((world[0] - after[0]).abs() < 0.001);
        assert!((world[1] - after[1]).abs() < 0.001);
        assert_eq!(camera.scale, 10_000.);
    }

    #[test]
    fn pan_is_in_screen_pixels_at_every_scale() {
        let mut camera = Viewport2D {
            scale: 25.,
            ..Default::default()
        };
        assert!(camera.pan([12., -8.]));
        assert_eq!(camera.offset, [12., -8.]);
    }
}
