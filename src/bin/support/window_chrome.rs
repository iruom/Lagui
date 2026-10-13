#[cfg(target_os = "macos")]
pub fn position_traffic_lights(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    unsafe extern "C" {
        fn bg_install_traffic_light_layout(
            view: *mut core::ffi::c_void,
            left: f64,
            header_height: f64,
        );
        fn bg_install_titlebar_drag_region(
            view: *mut core::ffi::c_void,
            height: f64,
            menu_left: f64,
            menu_width: f64,
            right_width: f64,
        );
    }

    let Ok(handle) = window.window_handle() else {
        return;
    };
    if let RawWindowHandle::AppKit(appkit) = handle.as_raw() {
        // SAFETY: winit owns a live NSView for the lifetime of `window`; this
        // runs on the application thread and AppKit owns the standard buttons.
        // Shift the standard controls into Studio's 54 px application header.
        unsafe {
            bg_install_traffic_light_layout(appkit.ns_view.as_ptr(), 14., 54.);
            bg_install_titlebar_drag_region(appkit.ns_view.as_ptr(), 54., 296., 174., 110.);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn position_traffic_lights(_window: &winit::window::Window) {}
