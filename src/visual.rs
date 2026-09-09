use crate::{CapacityExceeded, Rect};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrokeAlignment {
    #[default]
    Inside,
    Center,
    Outside,
}

/// A clipped, antialiased rounded rectangle in logical pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visual {
    pub bounds: Rect,
    /// Linear, premultiplied RGBA.
    pub color: [f32; 4],
    pub clip: Rect,
    /// `radius` is clamped to half the smaller extent by the shader.
    pub radius: f32,
    /// Negative values select a fill. Non-negative values select an outline whose
    /// interior is composited with `inner_color`.
    pub stroke_width: f32,
    pub _padding: [f32; 2],
    /// Linear, premultiplied RGBA used inside an outline.
    pub inner_color: [f32; 4],
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            bounds: Rect::default(),
            color: [0.; 4],
            clip: Rect::new(-1.0e6, -1.0e6, 2.0e6, 2.0e6),
            radius: 0.,
            stroke_width: -1.,
            _padding: [0.; 2],
            inner_color: [0.; 4],
        }
    }
}

/// Fixed-capacity visual command list. It preserves painter's order and never allocates.
pub struct VisualList<'a> {
    storage: &'a mut [Visual],
    len: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct ListCheckpoint(usize);

impl<'a> VisualList<'a> {
    pub(crate) fn checkpoint(&self) -> ListCheckpoint {
        ListCheckpoint(self.len)
    }

    pub(crate) fn rollback(&mut self, checkpoint: ListCheckpoint) {
        self.len = checkpoint.0;
    }

    pub fn new(storage: &'a mut [Visual]) -> Self {
        Self { storage, len: 0 }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn fill(
        &mut self,
        bounds: Rect,
        color: [f32; 4],
        radius: f32,
        clip: Rect,
    ) -> Result<(), CapacityExceeded> {
        if bounds.width <= 0. || bounds.height <= 0. {
            return Ok(());
        }
        let slot = self.storage.get_mut(self.len).ok_or(CapacityExceeded)?;
        *slot = Visual {
            bounds,
            color,
            clip,
            radius,
            stroke_width: -1.,
            _padding: [0.; 2],
            inner_color: [0.; 4],
        };
        self.len += 1;
        Ok(())
    }

    /// Draw an outline and its interior as one atomic command. A transparent
    /// `inner_color` leaves the content below the outline unchanged.
    pub fn outline(
        &mut self,
        bounds: Rect,
        stroke: [f32; 4],
        inner_color: [f32; 4],
        radius: f32,
        width: f32,
        clip: Rect,
    ) -> Result<(), CapacityExceeded> {
        self.outline_aligned(
            bounds,
            stroke,
            inner_color,
            radius,
            width,
            StrokeAlignment::Inside,
            clip,
        )
    }

    /// Draws an outline inside, centered on, or outside the supplied bounds.
    /// The radius describes the supplied bounds; it expands with center/outside strokes.
    #[allow(clippy::too_many_arguments)]
    pub fn outline_aligned(
        &mut self,
        bounds: Rect,
        stroke: [f32; 4],
        inner_color: [f32; 4],
        radius: f32,
        width: f32,
        alignment: StrokeAlignment,
        clip: Rect,
    ) -> Result<(), CapacityExceeded> {
        if bounds.width <= 0. || bounds.height <= 0. {
            return Ok(());
        }
        let width = width.max(0.);
        let expansion = match alignment {
            StrokeAlignment::Inside => 0.,
            StrokeAlignment::Center => width * 0.5,
            StrokeAlignment::Outside => width,
        };
        let bounds = Rect::new(
            bounds.x - expansion,
            bounds.y - expansion,
            bounds.width + expansion * 2.,
            bounds.height + expansion * 2.,
        );
        let slot = self.storage.get_mut(self.len).ok_or(CapacityExceeded)?;
        *slot = Visual {
            bounds,
            color: stroke,
            clip,
            radius: radius + expansion,
            stroke_width: width,
            _padding: [0.; 2],
            inner_color,
        };
        self.len += 1;
        Ok(())
    }

    pub fn commands(&self) -> &[Visual] {
        &self.storage[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.storage.len()
    }

    pub fn remaining_capacity(&self) -> usize {
        self.storage.len() - self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_is_one_atomic_command_and_clamps_its_width_in_the_shader() {
        let mut storage = [Visual::default(); 1];
        let mut list = VisualList::new(&mut storage);
        let clip = Rect::new(0., 0., 100., 100.);
        list.outline(
            Rect::new(10., 20., 30., 12.),
            [1.; 4],
            [0.; 4],
            6.,
            20.,
            clip,
        )
        .unwrap();
        assert_eq!(list.commands().len(), 1);
        assert_eq!(list.commands()[0].bounds.x, 10.);
        assert_eq!(list.commands()[0].stroke_width, 20.);
        assert_eq!(list.commands()[0].inner_color, [0.; 4]);
        assert_eq!(list.remaining_capacity(), 0);
    }

    #[test]
    fn failed_outline_does_not_change_the_list() {
        let mut storage = [];
        let mut list = VisualList::new(&mut storage);
        assert_eq!(
            list.outline(
                Rect::new(0., 0., 10., 10.),
                [1.; 4],
                [0.; 4],
                2.,
                1.,
                Rect::new(0., 0., 10., 10.)
            ),
            Err(CapacityExceeded)
        );
        assert!(list.is_empty());
    }

    #[test]
    fn outline_alignment_expands_bounds_and_radius() {
        let mut storage = [Visual::default(); 3];
        let mut list = VisualList::new(&mut storage);
        let bounds = Rect::new(10., 20., 30., 40.);
        let clip = Rect::new(0., 0., 100., 100.);
        for alignment in [
            StrokeAlignment::Inside,
            StrokeAlignment::Center,
            StrokeAlignment::Outside,
        ] {
            list.outline_aligned(bounds, [1.; 4], [0.; 4], 5., 4., alignment, clip)
                .unwrap();
        }
        assert_eq!(list.commands()[0].bounds, bounds);
        assert_eq!(list.commands()[1].bounds, Rect::new(8., 18., 34., 44.));
        assert_eq!(list.commands()[1].radius, 7.);
        assert_eq!(list.commands()[2].bounds, Rect::new(6., 16., 38., 48.));
        assert_eq!(list.commands()[2].radius, 9.);
    }
}
