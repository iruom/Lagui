//! macOS-specific surface synchronization for interactive window resizing.

/// Toggle Core Animation transaction presents for a wgpu surface.
///
/// wgpu snapshots this Metal-layer property when acquiring a surface texture,
/// so the caller must change it before the next frame is acquired. Reconfigure
/// the surface after changing it to keep the surface state synchronized with
/// the backend, matching the egui fix this is based on.
pub fn set_present_with_transaction(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    enabled: bool,
) {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `surface` is a live wgpu surface and the Metal HAL cast is
        // only attempted on macOS. `as_hal` returns None for a non-Metal
        // surface, so the operation is a no-op for any unexpected backend.
        let is_metal = unsafe {
            surface
                .as_hal::<wgpu::hal::api::Metal>()
                .map(|hal_surface| {
                    hal_surface
                        .render_layer()
                        .lock()
                        .setPresentsWithTransaction(enabled);
                })
                .is_some()
        };
        if is_metal {
            surface.configure(device, config);
        }
    }

    #[cfg(not(target_os = "macos"))]
    let _ = (surface, device, config, enabled);
}
