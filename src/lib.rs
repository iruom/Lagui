//! Lagui is a native, immediate-mode-friendly GUI toolkit with an
//! `no_std` interaction and layout core with optional allocation-backed UI storage.
//!
//! Start with [`ui::UiRuntime`] for input and semantics, [`layout`] for
//! geometry, [`visual::VisualList`] for painting, and [`platform`] to connect
//! the core to an operating system. GPU and text support are feature-gated.
#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "renderer-wgpu")]
extern crate std;

/// Logical pixels, with the origin at the top left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    /// Returns whether two finite, positive-area rectangles overlap.
    pub fn intersects(self, other: Self) -> bool {
        let values = [
            self.x,
            self.y,
            self.width,
            self.height,
            other.x,
            other.y,
            other.width,
            other.height,
        ];
        if values.iter().any(|value| !value.is_finite())
            || self.width <= 0.
            || self.height <= 0.
            || other.width <= 0.
            || other.height <= 0.
        {
            return false;
        }
        let self_right = self.x + self.width;
        let self_bottom = self.y + self.height;
        let other_right = other.x + other.width;
        let other_bottom = other.y + other.height;
        if [self_right, self_bottom, other_right, other_bottom]
            .iter()
            .any(|value| !value.is_finite())
        {
            return false;
        }
        self.x < other_right
            && other.x < self_right
            && self.y < other_bottom
            && other.y < self_bottom
    }

    /// Finite visible bounds shared by hit testing and semantic export.
    pub fn intersection(self, other: Self) -> Option<Self> {
        if !self.intersects(other) {
            return None;
        }
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Some(Self::new(
            x,
            y,
            (self.x + self.width).min(other.x + other.width) - x,
            (self.y + self.height).min(other.y + other.height) - y,
        ))
    }
}

/// Linear, premultiplied RGBA. The backend uses an sRGB render target.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quad {
    pub bounds: Rect,
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityExceeded;

impl core::fmt::Display for CapacityExceeded {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("caller-provided capacity was exceeded")
    }
}

impl core::error::Error for CapacityExceeded {}

/// Caller-owned contiguous storage; preserves painter's order and never allocates.
pub struct DrawList<'a> {
    storage: &'a mut [Quad],
    len: usize,
}

impl<'a> DrawList<'a> {
    pub fn new(storage: &'a mut [Quad]) -> Self {
        Self { storage, len: 0 }
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn rectangle(&mut self, bounds: Rect, color: [f32; 4]) -> Result<(), CapacityExceeded> {
        let slot = self.storage.get_mut(self.len).ok_or(CapacityExceeded)?;
        *slot = Quad { bounds, color };
        self.len += 1;
        Ok(())
    }
    pub fn commands(&self) -> &[Quad] {
        &self.storage[..self.len]
    }
}

pub mod demo;
pub mod frame;
pub mod layout;
pub mod overlay;
pub mod panel;
pub mod platform;
pub mod text_edit;
pub mod transform;
pub mod ui;
pub mod viewport;
pub mod visual;
pub mod widgets;

#[cfg(feature = "renderer-wgpu")]
pub mod gpu;

/// General-purpose analytic vector paths (icons and stroked Bézier curves).
#[cfg(feature = "renderer-wgpu")]
pub mod windfoil;

#[cfg(feature = "text")]
pub mod text;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_preserves_commands_and_storage_can_be_reused() {
        let mut storage = [Quad::default(); 1];
        let mut list = DrawList::new(&mut storage);
        let rect = Rect::new(1., 2., 3., 4.);
        assert_eq!(list.rectangle(rect, [1.; 4]), Ok(()));
        assert_eq!(
            list.rectangle(Rect::default(), [0.; 4]),
            Err(CapacityExceeded)
        );
        assert_eq!(list.commands()[0].bounds, rect);
        list.clear();
        assert!(list.commands().is_empty());
        assert_eq!(list.rectangle(rect, [0.; 4]), Ok(()));
        let mut empty = [];
        assert_eq!(
            DrawList::new(&mut empty).rectangle(rect, [0.; 4]),
            Err(CapacityExceeded)
        );
    }
    #[test]
    fn hit_test_uses_half_open_edges() {
        let r = Rect::new(10., 20., 30., 40.);
        assert!(r.contains(10., 20.));
        assert!(!r.contains(40., 20.));
        assert!(!r.contains(f32::NAN, 20.));
    }

    #[test]
    fn rectangle_intersection_rejects_edges_and_invalid_extents() {
        let r = Rect::new(10., 20., 30., 40.);
        assert!(r.intersects(Rect::new(39., 59., 2., 2.)));
        assert!(!r.intersects(Rect::new(40., 20., 1., 1.)));
        assert!(!r.intersects(Rect::new(10., 20., 0., 1.)));
        assert!(!r.intersects(Rect::new(f32::NAN, 20., 1., 1.)));
    }
}
