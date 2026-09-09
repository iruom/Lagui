use lagui::{
    Rect,
    ui::{Role, Semantics, UiRuntimeStorage, WidgetId},
    visual::{Visual, VisualList},
    widgets::{self, Theme, WidgetContext, WidgetError, WidgetSpec},
};

#[test]
fn downstream_application_can_build_and_activate_a_button() {
    let id = WidgetId(42);
    let bounds = Rect::new(10., 10., 100., 32.);
    let mut ui = UiRuntimeStorage::<4>::default();
    let mut storage = [Visual::default(); 4];

    ui.begin_frame();
    let mut visuals = VisualList::new(&mut storage);
    let state = widgets::button(
        &mut ui,
        &mut visuals,
        WidgetSpec {
            id,
            bounds,
            clip: bounds,
            semantics: Semantics {
                role: Role::Button,
                label: "Save",
                ..Semantics::default()
            },
        },
        Theme::default(),
    )
    .unwrap();
    assert!(!state.disabled);
    assert_eq!(visuals.len(), 1);
    ui.end_frame();

    ui.pointer_down([20., 20.]);
    let released = ui.pointer_up([20., 20.]);
    assert!(widgets::clicked(released, id));
}

#[test]
fn custom_widget_transaction_rolls_back_every_append() {
    let bounds = Rect::new(0., 0., 20., 20.);
    let mut ui = UiRuntimeStorage::<2>::default();
    let mut storage = [Visual::default(); 1];
    ui.begin_frame();
    let mut visuals = VisualList::new(&mut storage);
    let mut context = WidgetContext::new(&mut ui, &mut visuals);

    let result: Result<(), WidgetError> = context.transaction(|context| {
        context.register(
            WidgetSpec {
                id: WidgetId(1),
                bounds,
                clip: bounds,
                semantics: Semantics::default(),
            },
            true,
        )?;
        context
            .fill(bounds, [1.; 4], 0., bounds)
            .map_err(|_| WidgetError::VisualCapacityExceeded)?;
        context
            .fill(bounds, [0.; 4], 0., bounds)
            .map_err(|_| WidgetError::VisualCapacityExceeded)?;
        Ok(())
    });

    assert_eq!(result, Err(WidgetError::VisualCapacityExceeded));
    assert_eq!(context.remaining_ui_capacity(), 2);
    assert_eq!(context.remaining_visual_capacity(), 1);
}
