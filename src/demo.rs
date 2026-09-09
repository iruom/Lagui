use crate::{CapacityExceeded, DrawList, Rect};

const BUTTON: Rect = Rect::new(40., 150., 168., 48.);
const SLIDER: Rect = Rect::new(40., 258., 260., 36.);

#[derive(Default, Clone, Copy, Debug, PartialEq)]
enum Capture {
    #[default]
    None,
    Button,
    Slider,
}

/// Small input/state fixture, independent of the OS and renderer.
pub struct Demo {
    pub value: f32,
    pub alternate: bool,
    capture: Capture,
    hover: bool,
    pointer: [f32; 2],
}
impl Default for Demo {
    fn default() -> Self {
        Self {
            value: 0.55,
            alternate: false,
            capture: Capture::None,
            hover: false,
            pointer: [-20., -20.],
        }
    }
}
impl Demo {
    /// 0 move, 1 press, 2 release, 3 cancel/focus loss, 4 space, 5/6 arrows.
    pub fn input(&mut self, kind: u32, x: f32, y: f32) {
        if kind <= 2 {
            self.pointer = [x, y];
            self.hover = BUTTON.contains(x, y);
        }
        match kind {
            1 => {
                self.capture = if BUTTON.contains(x, y) {
                    Capture::Button
                } else if SLIDER.contains(x, y) {
                    Capture::Slider
                } else {
                    Capture::None
                }
            }
            2 => {
                if self.capture == Capture::Button && BUTTON.contains(x, y) {
                    self.alternate = !self.alternate;
                }
            }
            3 => {
                self.capture = Capture::None;
                self.hover = false;
                self.pointer = [-20.; 2];
            }
            4 => self.alternate = !self.alternate,
            5 => self.value = (self.value - 0.05).max(0.),
            6 => self.value = (self.value + 0.05).min(1.),
            _ => {}
        }
        if kind <= 2 && self.capture == Capture::Slider {
            self.value = ((x - SLIDER.x) / SLIDER.width).clamp(0., 1.);
        }
        if kind == 2 {
            self.capture = Capture::None;
        }
    }

    pub fn paint(
        &self,
        list: &mut DrawList<'_>,
        width: f32,
        height: f32,
    ) -> Result<(), CapacityExceeded> {
        let accent = if self.alternate {
            [0.6, 0.24, 0.95, 1.]
        } else {
            [0.2, 0.85, 0.48, 1.]
        };
        list.rectangle(Rect::new(0., 0., width, height), [0.014, 0.019, 0.025, 1.])?;
        list.rectangle(Rect::new(0., 0., 340., height), [0.025, 0.032, 0.043, 1.])?;
        list.rectangle(Rect::new(40., 40., 28., 5.), accent)?;
        list.rectangle(
            BUTTON,
            if self.hover || self.capture == Capture::Button {
                [0.35, 0.42, 0.5, 1.]
            } else {
                [0.12, 0.16, 0.2, 1.]
            },
        )?;
        list.rectangle(Rect::new(40., 272., 260., 6.), [0.09, 0.12, 0.16, 1.])?;
        list.rectangle(Rect::new(40., 272., 260. * self.value, 6.), accent)?;
        list.rectangle(
            Rect::new(34. + 260. * self.value, 262., 12., 26.),
            [0.85, 0.92, 0.96, 1.],
        )?;
        let area_width = (width - 420.).max(40.);
        let area_height = (height - 250.).max(40.);
        for i in 0..24 {
            let factor =
                (((i * 17 + 5) % 23) as f32 / 23. * 0.7 + 0.3) * (0.15 + self.value * 0.85);
            let h = area_height * factor;
            list.rectangle(
                Rect::new(
                    380. + i as f32 * area_width / 24.,
                    height - 80. - h,
                    (area_width / 24. - 5.).max(1.),
                    h,
                ),
                accent,
            )?;
        }
        list.rectangle(
            Rect::new(self.pointer[0] - 3., self.pointer[1] - 3., 6., 6.),
            [1., 1., 1., 1.],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_requires_matching_press_and_release() {
        let mut ui = Demo::default();
        ui.input(2, 60., 170.);
        assert!(!ui.alternate);
        ui.input(1, 60., 170.);
        ui.input(2, 500., 170.);
        assert!(!ui.alternate);
        ui.input(1, 60., 170.);
        ui.input(2, 60., 170.);
        assert!(ui.alternate);
    }
    #[test]
    fn slider_capture_clamps_and_focus_loss_cancels() {
        let mut ui = Demo::default();
        ui.input(1, 150., 275.);
        ui.input(0, 900., 275.);
        assert_eq!(ui.value, 1.);
        ui.input(3, 0., 0.);
        ui.input(0, -100., 275.);
        assert_eq!(ui.value, 1.);
    }
}
