/// Backend abstraction for MilkyWM.
///
/// MilkyWM supports two backends:
///  - `winit`  — runs inside an existing X11/Wayland window. Used during
///               development; no root privileges required.
///  - `drm`    — takes over the GPU directly via DRM/KMS. Used in production
///               (started from a TTY or display manager).
///
/// The backend is selected at runtime:
///  - If the env var `MILKYWM_BACKEND=drm` is set  → DRM backend.
///  - If `DISPLAY` or `WAYLAND_DISPLAY` is set      → winit backend.
///  - Otherwise                                      → DRM backend.
pub mod winit;
pub mod drm;

use smithay::reexports::calloop::EventLoop;

use crate::state::MilkyState;

/// Initialise the appropriate backend and attach it to the event loop.
/// Returns the name of the backend that was selected.
pub fn init(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<&'static str> {
    let use_winit = std::env::var("MILKYWM_BACKEND")
        .map(|v| v == "winit")
        .unwrap_or_else(|_| {
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
        });

    if use_winit {
        winit::init_winit(event_loop, state)?;
        Ok("winit")
    } else {
        drm::init_drm(event_loop, state)?;
        Ok("drm")
    }
}
