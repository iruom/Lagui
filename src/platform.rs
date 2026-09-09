//! Backend-neutral contracts between the GUI core and an operating system.
//!
//! Implement these traits in an application adapter (for example with winit,
//! AccessKit, or native platform APIs). Keeping them here avoids coupling the
//! allocation-free core to a particular event loop or clipboard package.

use crate::{
    Rect,
    ui::{Semantics, WidgetId},
};

/// Mouse cursor requested by the widget under the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorIcon {
    #[default]
    Default,
    Pointer,
    Text,
    Crosshair,
    Move,
    ResizeHorizontal,
    ResizeVertical,
}

/// Clipboard service owned by the host application.
pub trait Clipboard {
    type Error;

    fn read_text(&mut self, out: &mut [u8]) -> Result<usize, Self::Error>;
    fn write_text(&mut self, text: &str) -> Result<(), Self::Error>;
}

/// Requests which cannot be fulfilled inside the platform-independent core.
pub trait Platform {
    fn request_redraw(&mut self);
    fn set_cursor(&mut self, cursor: CursorIcon);
    fn set_ime_allowed(&mut self, allowed: bool);
    fn set_ime_cursor_area(&mut self, area: Rect);
}

/// A sink used to rebuild the native accessibility tree each frame.
pub trait AccessibilitySink {
    type Error;

    fn begin(&mut self) -> Result<(), Self::Error>;
    fn node(&mut self, id: WidgetId, bounds: Rect, semantics: Semantics)
    -> Result<(), Self::Error>;
    fn focus(&mut self, id: Option<WidgetId>) -> Result<(), Self::Error>;
    fn end(&mut self) -> Result<(), Self::Error>;
}

/// Exports a frame's semantic nodes to a native accessibility adapter.
pub fn export_accessibility<S: AccessibilitySink, const N: usize>(
    ui: &crate::ui::UiRuntimeStorage<N>,
    sink: &mut S,
) -> Result<(), S::Error> {
    sink.begin()?;
    for (id, bounds, semantics) in ui.semantics() {
        sink.node(id, bounds, semantics)?;
    }
    sink.focus(ui.focused_id())?;
    sink.end()
}
