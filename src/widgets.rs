//! Themeable visual primitives. Applications keep ownership of state and text.

use crate::{
    CapacityExceeded, Rect,
    ui::{RegistrationError, Semantics, UiRuntimeStorage, WidgetId},
    visual::{StrokeAlignment, VisualList},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetState {
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
    pub selected: bool,
    pub disabled: bool,
}

/// Failure to build a complete widget for a frame.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetError {
    Registration(RegistrationError),
    VisualCapacityExceeded,
}

impl core::fmt::Display for WidgetError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Registration(error) => error.fmt(formatter),
            Self::VisualCapacityExceeded => formatter.write_str("visual capacity was exceeded"),
        }
    }
}

impl core::error::Error for WidgetError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Registration(error) => Some(error),
            Self::VisualCapacityExceeded => None,
        }
    }
}

impl From<RegistrationError> for WidgetError {
    fn from(value: RegistrationError) -> Self {
        Self::Registration(value)
    }
}

/// Geometry and semantic identity shared by standard widgets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetSpec {
    pub id: WidgetId,
    pub bounds: Rect,
    pub clip: Rect,
    pub semantics: Semantics,
}

/// Append-only construction context for application-defined widgets.
///
/// Use [`Self::transaction`] around every logical widget. If registration or
/// painting fails, all regions and visual commands appended by that closure are
/// rolled back. Only append operations are exposed so earlier widgets cannot be
/// accidentally cleared or rewritten from inside a transaction.
pub struct WidgetContext<'a, 'storage, const N: usize> {
    ui: &'a mut UiRuntimeStorage<N>,
    visuals: &'a mut VisualList<'storage>,
}

impl<'a, 'storage, const N: usize> WidgetContext<'a, 'storage, N> {
    pub fn new(ui: &'a mut UiRuntimeStorage<N>, visuals: &'a mut VisualList<'storage>) -> Self {
        Self { ui, visuals }
    }

    /// Runs one atomic widget build. Nested transactions are supported.
    pub fn transaction<T, E>(
        &mut self,
        build: impl FnOnce(&mut Self) -> Result<T, E>,
    ) -> Result<T, E> {
        let ui_checkpoint = self.ui.checkpoint();
        let visual_checkpoint = self.visuals.checkpoint();
        match build(self) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.ui.rollback(ui_checkpoint);
                self.visuals.rollback(visual_checkpoint);
                Err(error)
            }
        }
    }

    pub fn register(
        &mut self,
        spec: WidgetSpec,
        focusable: bool,
    ) -> Result<WidgetState, RegistrationError> {
        register(
            self.ui,
            spec.id,
            spec.bounds,
            spec.clip,
            focusable,
            spec.semantics,
        )
    }

    pub fn fill(
        &mut self,
        bounds: Rect,
        color: [f32; 4],
        radius: f32,
        clip: Rect,
    ) -> Result<(), CapacityExceeded> {
        self.visuals.fill(bounds, color, radius, clip)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn outline(
        &mut self,
        bounds: Rect,
        stroke: [f32; 4],
        inner_color: [f32; 4],
        radius: f32,
        width: f32,
        alignment: StrokeAlignment,
        clip: Rect,
    ) -> Result<(), CapacityExceeded> {
        self.visuals
            .outline_aligned(bounds, stroke, inner_color, radius, width, alignment, clip)
    }

    pub fn remaining_ui_capacity(&self) -> usize {
        self.ui.remaining_capacity()
    }

    pub fn remaining_visual_capacity(&self) -> usize {
        self.visuals.remaining_capacity()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub surface: [f32; 4],
    pub surface_hovered: [f32; 4],
    pub surface_active: [f32; 4],
    pub border: [f32; 4],
    pub accent: [f32; 4],
    pub disabled: [f32; 4],
    pub radius: f32,
    pub focus_width: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            surface: [0.047, 0.057, 0.077, 1.],
            surface_hovered: [0.072, 0.084, 0.112, 1.],
            surface_active: [0.33, 0.22, 0.94, 1.],
            border: [0.105, 0.126, 0.165, 1.],
            accent: [0.33, 0.22, 0.94, 1.],
            disabled: [0.04, 0.045, 0.055, 0.65],
            radius: 7.,
            focus_width: 2.,
        }
    }
}

/// Registers one logical widget and returns the interaction state needed to
/// paint it. This keeps hit testing, disabled behavior and semantic metadata on
/// the same code path instead of requiring every application to duplicate it.
pub fn register<const N: usize>(
    ui: &mut UiRuntimeStorage<N>,
    id: WidgetId,
    bounds: Rect,
    clip: Rect,
    focusable: bool,
    semantics: Semantics,
) -> Result<WidgetState, RegistrationError> {
    ui.register_semantic_unique(id, bounds, clip, focusable, semantics)?;
    Ok(WidgetState {
        hovered: ui.hovered(id),
        active: ui.active(id),
        focused: ui.focused(id),
        selected: semantics.selected,
        disabled: semantics.disabled,
    })
}

/// Returns whether a pointer response activated this widget.
pub const fn clicked(response: crate::ui::UiResponse, id: WidgetId) -> bool {
    response.clicked && matches!(response.target, Some(target) if target.0 == id.0)
}

/// Registers and paints a standard button atomically.
///
/// Text rendering remains application-owned; `semantics.label` supplies the
/// accessible label. No UI region or visual command is appended when capacity
/// or ID validation fails.
pub fn button<const N: usize>(
    ui: &mut UiRuntimeStorage<N>,
    visuals: &mut VisualList<'_>,
    spec: WidgetSpec,
    theme: Theme,
) -> Result<WidgetState, WidgetError> {
    let mut context = WidgetContext::new(ui, visuals);
    context.transaction(|context| {
        let state = context.register(spec, true)?;
        paint_control(context, spec.bounds, spec.clip, state, theme)?;
        Ok(state)
    })
}

/// Registers and paints a standard switch atomically.
pub fn toggle_switch<const N: usize>(
    ui: &mut UiRuntimeStorage<N>,
    visuals: &mut VisualList<'_>,
    spec: WidgetSpec,
    on: bool,
    theme: Theme,
) -> Result<WidgetState, WidgetError> {
    let mut context = WidgetContext::new(ui, visuals);
    context.transaction(|context| {
        let state = context.register(spec, true)?;
        paint_switch(context, spec.bounds, spec.clip, on, state, theme)?;
        Ok(state)
    })
}

fn paint_control<const N: usize>(
    context: &mut WidgetContext<'_, '_, N>,
    bounds: Rect,
    clip: Rect,
    state: WidgetState,
    theme: Theme,
) -> Result<(), WidgetError> {
    let fill = if state.disabled {
        theme.disabled
    } else if state.active || state.selected {
        theme.surface_active
    } else if state.hovered {
        theme.surface_hovered
    } else {
        theme.surface
    };
    let (stroke, width) = if state.focused {
        (theme.accent, theme.focus_width.max(1.))
    } else {
        (theme.border, 1.)
    };
    context
        .outline(
            bounds,
            stroke,
            fill,
            theme.radius,
            width,
            StrokeAlignment::Inside,
            clip,
        )
        .map_err(|_| WidgetError::VisualCapacityExceeded)
}

fn paint_switch<const N: usize>(
    context: &mut WidgetContext<'_, '_, N>,
    bounds: Rect,
    clip: Rect,
    on: bool,
    state: WidgetState,
    theme: Theme,
) -> Result<(), WidgetError> {
    let track = if on {
        theme.accent
    } else if state.disabled {
        theme.disabled
    } else {
        theme.surface_hovered
    };
    context
        .fill(bounds, track, bounds.height * 0.5, clip)
        .map_err(|_| WidgetError::VisualCapacityExceeded)?;
    let size = (bounds.height - 4.).max(0.);
    let x = if on {
        bounds.x + bounds.width - size - 2.
    } else {
        bounds.x + 2.
    };
    context
        .fill(
            Rect::new(x, bounds.y + 2., size, size),
            [0.88, 0.91, 0.98, 1.],
            size * 0.5,
            clip,
        )
        .map_err(|_| WidgetError::VisualCapacityExceeded)?;
    if state.focused {
        context
            .outline(
                bounds,
                theme.accent,
                [0.; 4],
                bounds.height * 0.5,
                theme.focus_width,
                StrokeAlignment::Inside,
                clip,
            )
            .map_err(|_| WidgetError::VisualCapacityExceeded)?;
    }
    Ok(())
}

/// Paints the background, border and clearly visible keyboard focus ring for a control.
pub fn control(
    visuals: &mut VisualList<'_>,
    bounds: Rect,
    clip: Rect,
    state: WidgetState,
    theme: Theme,
) -> Result<(), CapacityExceeded> {
    let fill = if state.disabled {
        theme.disabled
    } else if state.active || state.selected {
        theme.surface_active
    } else if state.hovered {
        theme.surface_hovered
    } else {
        theme.surface
    };
    if state.focused {
        let inset = theme.focus_width.max(1.);
        visuals.outline(bounds, theme.accent, fill, theme.radius, inset, clip)
    } else {
        visuals.outline(bounds, theme.border, fill, theme.radius, 1., clip)
    }
}

/// Standard switch track and thumb. Text remains owned by the application.
pub fn switch(
    visuals: &mut VisualList<'_>,
    bounds: Rect,
    clip: Rect,
    on: bool,
    state: WidgetState,
    theme: Theme,
) -> Result<(), CapacityExceeded> {
    let track = if on {
        theme.accent
    } else if state.disabled {
        theme.disabled
    } else {
        theme.surface_hovered
    };
    visuals.fill(bounds, track, bounds.height * 0.5, clip)?;
    let size = (bounds.height - 4.).max(0.);
    let x = if on {
        bounds.x + bounds.width - size - 2.
    } else {
        bounds.x + 2.
    };
    visuals.fill(
        Rect::new(x, bounds.y + 2., size, size),
        [0.88, 0.91, 0.98, 1.],
        size * 0.5,
        clip,
    )?;
    if state.focused {
        visuals.outline(
            bounds,
            theme.accent,
            [0.; 4],
            bounds.height * 0.5,
            theme.focus_width,
            clip,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::Visual;
    #[test]
    fn focused_control_is_one_atomic_outline() {
        let mut storage = [Visual::default(); 1];
        let mut list = VisualList::new(&mut storage);
        control(
            &mut list,
            Rect::new(0., 0., 20., 20.),
            Rect::new(0., 0., 20., 20.),
            WidgetState {
                focused: true,
                ..WidgetState::default()
            },
            Theme::default(),
        )
        .unwrap();
        assert_eq!(list.commands()[0].stroke_width, 2.);
    }

    #[test]
    fn registration_derives_visual_state_and_enforces_unique_ids() {
        let mut ui = crate::ui::UiRuntimeStorage::<2>::default();
        ui.begin_frame();
        ui.set_focus(Some(WidgetId(4)));
        let state = register(
            &mut ui,
            WidgetId(4),
            Rect::new(0., 0., 20., 20.),
            Rect::new(0., 0., 20., 20.),
            true,
            Semantics {
                selected: true,
                ..Semantics::default()
            },
        )
        .unwrap();
        assert!(state.focused);
        assert!(state.selected);
        assert!(matches!(
            register(
                &mut ui,
                WidgetId(4),
                Rect::new(0., 0., 20., 20.),
                Rect::new(0., 0., 20., 20.),
                true,
                Semantics::default()
            ),
            Err(RegistrationError::DuplicateId(WidgetId(4)))
        ));
    }

    #[test]
    fn button_failure_is_atomic_across_ui_and_visual_lists() {
        let mut ui = crate::ui::UiRuntimeStorage::<1>::default();
        let mut empty = [];
        let mut visuals = VisualList::new(&mut empty);
        ui.begin_frame();
        let result = button(
            &mut ui,
            &mut visuals,
            WidgetSpec {
                id: WidgetId(9),
                bounds: Rect::new(0., 0., 20., 20.),
                clip: Rect::new(0., 0., 20., 20.),
                semantics: Semantics::default(),
            },
            Theme::default(),
        );
        assert_eq!(result, Err(WidgetError::VisualCapacityExceeded));
        assert!(ui.is_empty());
        assert!(visuals.is_empty());
    }
}
