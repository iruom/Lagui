//! Presentation coordinates shared by CPU bounds and GPU rendering.
use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2D {
    pub offset: [f32; 2],
    pub scale: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    pub const IDENTITY: Self = Self {
        offset: [0.; 2],
        scale: 1.,
    };
    pub fn valid(self) -> bool {
        self.scale.is_finite() && self.scale > 0. && self.offset.iter().all(|v| v.is_finite())
    }
    pub fn point(self, p: [f32; 2]) -> [f32; 2] {
        [
            p[0] * self.scale + self.offset[0],
            p[1] * self.scale + self.offset[1],
        ]
    }
    pub fn rect(self, r: Rect) -> Rect {
        let p = self.point([r.x, r.y]);
        Rect::new(p[0], p[1], r.width * self.scale, r.height * self.scale)
    }
    pub fn inverse_point(self, p: [f32; 2]) -> [f32; 2] {
        [
            (p[0] - self.offset[0]) / self.scale,
            (p[1] - self.offset[1]) / self.scale,
        ]
    }
    pub fn inverse_rect(self, r: Rect) -> Rect {
        let p = self.inverse_point([r.x, r.y]);
        Rect::new(p[0], p[1], r.width / self.scale, r.height / self.scale)
    }
}

/// Frame-local index into a host's transform table; zero always means identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TransformId(pub(crate) u32);
impl TransformId {
    pub const IDENTITY: Self = Self(0);
}

/// A local clip follows geometry; the containing panel clip stays on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformEntry {
    pub transform: Transform2D,
    pub screen_clip: Rect,
    pub local_clip: bool,
}
impl TransformEntry {
    pub const IDENTITY: Self = Self {
        transform: Transform2D::IDENTITY,
        screen_clip: Rect::new(-1e20, -1e20, 2e20, 2e20),
        local_clip: false,
    };
    pub fn clip(self, clip: Rect) -> Option<Rect> {
        if self.local_clip {
            clip.intersection(self.transform.inverse_rect(self.screen_clip))
                .map(|r| self.transform.rect(r))
        } else {
            clip.intersection(self.screen_clip)
        }
    }
}

/// Reused CPU presentation output, not an editing or camera-state owner.
#[cfg(feature = "alloc")]
pub struct TransformList {
    pub(crate) entries: alloc::vec::Vec<TransformEntry>,
}
#[cfg(feature = "alloc")]
impl Default for TransformList {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(feature = "alloc")]
impl TransformList {
    pub fn new() -> Self {
        Self {
            entries: alloc::vec![TransformEntry::IDENTITY],
        }
    }
    pub fn begin(&mut self) {
        self.entries.clear();
        self.entries.push(TransformEntry::IDENTITY);
    }
    pub fn push_local(&mut self, transform: Transform2D, screen_clip: Rect) -> Option<TransformId> {
        if !transform.valid()
            || [
                screen_clip.x,
                screen_clip.y,
                screen_clip.width,
                screen_clip.height,
            ]
            .iter()
            .any(|v| !v.is_finite())
        {
            return None;
        }
        let entry = TransformEntry {
            transform,
            screen_clip,
            local_clip: true,
        };
        if let Some(index) = self.entries.iter().position(|v| *v == entry) {
            return Some(TransformId(index as u32));
        }
        let id = u32::try_from(self.entries.len()).ok()?;
        self.entries.push(entry);
        Some(TransformId(id))
    }
}
