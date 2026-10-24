use crate::{CapacityExceeded, Rect};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrokeAlignment {
    #[default]
    Inside,
    Center,
    Outside,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerStyle {
    pub radius: f32,
    /// Figma-style arc/cubic smoothing in the range `0.0..=1.0`.
    /// Effective smoothing decreases when the radius leaves insufficient room.
    pub smoothing: f32,
}

impl CornerStyle {
    pub const fn new(radius: f32, smoothing: f32) -> Self {
        Self { radius, smoothing }
    }

    /// Designer-facing percentage, clamped to 0..=100 when rendered.
    pub const fn from_percent(radius: f32, percent: f32) -> Self {
        Self::new(radius, percent / 100.)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DropShadow {
    /// Linear, premultiplied RGBA.
    pub color: [f32; 4],
    pub offset: [f32; 2],
    /// Gaussian standard deviation in logical pixels. The shadow edge is the
    /// shape convolved with this Gaussian (half opacity on the spread outline).
    pub blur: f32,
    pub spread: f32,
}

impl DropShadow {
    /// Shadow from a CSS/Figma-style blur radius, which is twice the Gaussian
    /// standard deviation, so design-tool values render at their authored size.
    pub fn from_blur_radius(color: [f32; 4], offset: [f32; 2], radius: f32, spread: f32) -> Self {
        Self {
            color,
            offset,
            blur: radius.max(0.) * 0.5,
            spread,
        }
    }
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
    /// `0.0` is a circular rounded corner and `1.0` is maximally smoothed.
    pub corner_smoothing: f32,
    pub _padding: f32,
    /// Linear, premultiplied RGBA used inside an outline.
    pub inner_color: [f32; 4],
    /// Mathematical shape bounds. `bounds` may be larger to rasterize a shadow.
    pub shape_bounds: Rect,
    pub shadow_color: [f32; 4],
    pub shadow_offset: [f32; 2],
    pub shadow_blur: f32,
    pub shadow_spread: f32,
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            bounds: Rect::default(),
            color: [0.; 4],
            clip: Rect::new(-1.0e6, -1.0e6, 2.0e6, 2.0e6),
            radius: 0.,
            stroke_width: -1.,
            corner_smoothing: 0.,
            _padding: 0.,
            inner_color: [0.; 4],
            shape_bounds: Rect::default(),
            shadow_color: [0.; 4],
            shadow_offset: [0.; 2],
            shadow_blur: 0.,
            shadow_spread: 0.,
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
        self.fill_styled(bounds, color, CornerStyle::new(radius, 0.), None, clip)
    }

    pub fn fill_styled(
        &mut self,
        bounds: Rect,
        color: [f32; 4],
        corner: CornerStyle,
        shadow: Option<DropShadow>,
        clip: Rect,
    ) -> Result<(), CapacityExceeded> {
        if bounds.width <= 0. || bounds.height <= 0. {
            return Ok(());
        }
        let shadow = shadow.unwrap_or_default();
        let reach = shadow.blur.max(0.) * 3. + shadow.spread.max(0.);
        let left = reach + (-shadow.offset[0]).max(0.);
        let right = reach + shadow.offset[0].max(0.);
        let top = reach + (-shadow.offset[1]).max(0.);
        let bottom = reach + shadow.offset[1].max(0.);
        let draw_bounds = Rect::new(
            bounds.x - left,
            bounds.y - top,
            bounds.width + left + right,
            bounds.height + top + bottom,
        );
        let slot = self.storage.get_mut(self.len).ok_or(CapacityExceeded)?;
        *slot = Visual {
            bounds: draw_bounds,
            shape_bounds: bounds,
            color,
            clip,
            radius: corner.radius,
            corner_smoothing: corner.smoothing.clamp(0., 1.),
            stroke_width: -1.,
            _padding: 0.,
            inner_color: [0.; 4],
            shadow_color: shadow.color,
            shadow_offset: shadow.offset,
            shadow_blur: shadow.blur.max(0.),
            shadow_spread: shadow.spread,
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
        self.outline_styled(
            bounds,
            stroke,
            inner_color,
            CornerStyle::new(radius, 0.),
            width,
            alignment,
            clip,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn outline_styled(
        &mut self,
        bounds: Rect,
        stroke: [f32; 4],
        inner_color: [f32; 4],
        corner: CornerStyle,
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
            shape_bounds: bounds,
            color: stroke,
            clip,
            radius: corner.radius + expansion,
            corner_smoothing: corner.smoothing.clamp(0., 1.),
            stroke_width: width,
            _padding: 0.,
            inner_color,
            shadow_color: [0.; 4],
            shadow_offset: [0.; 2],
            shadow_blur: 0.,
            shadow_spread: 0.,
        };
        self.len += 1;
        Ok(())
    }

    /// Append already-built commands, e.g. replaying a retained presentation.
    pub fn extend_from(&mut self, visuals: &[Visual]) -> Result<(), CapacityExceeded> {
        let end = self.len + visuals.len();
        self.storage
            .get_mut(self.len..end)
            .ok_or(CapacityExceeded)?
            .copy_from_slice(visuals);
        self.len = end;
        Ok(())
    }

    /// Multiply every premultiplied color emitted since `start` by `opacity`.
    pub fn fade_since(&mut self, start: usize, opacity: f32) {
        let opacity = opacity.clamp(0., 1.);
        for visual in self.storage[start.min(self.len)..self.len].iter_mut() {
            for color in [
                &mut visual.color,
                &mut visual.inner_color,
                &mut visual.shadow_color,
            ] {
                *color = color.map(|channel| channel * opacity);
            }
        }
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
    fn fade_scales_only_later_commands_and_replay_is_bounded() {
        let mut storage = [Visual::default(); 3];
        let mut list = VisualList::new(&mut storage);
        let clip = Rect::new(0., 0., 100., 100.);
        let bounds = Rect::new(0., 0., 10., 10.);
        list.fill(bounds, [1.; 4], 0., clip).unwrap();
        list.outline(bounds, [1.; 4], [0.5; 4], 0., 1., clip)
            .unwrap();
        list.fade_since(1, 0.5);
        assert_eq!(list.commands()[0].color, [1.; 4]);
        assert_eq!(list.commands()[1].color, [0.5; 4]);
        assert_eq!(list.commands()[1].inner_color, [0.25; 4]);
        let replay = [list.commands()[0]; 2];
        assert_eq!(list.extend_from(&replay), Err(CapacityExceeded));
        assert_eq!(list.len(), 2);
        list.extend_from(&replay[..1]).unwrap();
        assert_eq!(list.len(), 3);
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

    #[test]
    fn styled_fill_clamps_smoothing_and_expands_for_gaussian_shadow() {
        let mut storage = [Visual::default(); 1];
        let mut list = VisualList::new(&mut storage);
        let shape = Rect::new(20., 30., 100., 50.);
        list.fill_styled(
            shape,
            [1.; 4],
            CornerStyle::new(12., 2.),
            Some(DropShadow {
                color: [0., 0., 0., 0.4],
                offset: [4., 6.],
                blur: 8.,
                spread: 2.,
            }),
            Rect::new(0., 0., 200., 200.),
        )
        .unwrap();
        let visual = list.commands()[0];
        assert_eq!(visual.corner_smoothing, 1.);
        assert_eq!(visual.shape_bounds, shape);
        assert_eq!(visual.bounds, Rect::new(-6., 4., 156., 108.));
        assert_eq!(visual.shadow_blur, 8.);
    }
}
