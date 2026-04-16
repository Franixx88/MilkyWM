use std::os::unix::io::OwnedFd;

use smithay::reexports::wayland_server::Resource;
use smithay::{
    desktop::Window,
    utils::{Logical, Rectangle},
    wayland::{
        selection::SelectionTarget,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Wm,
        xwm::{Reorder, ResizeEdge, X11Surface, XwmId},
    },
};
use tracing::{debug, warn};

use crate::state::MilkyState;

// ---------------------------------------------------------------------------
// XWaylandShellHandler
// ---------------------------------------------------------------------------

impl XWaylandShellHandler for MilkyState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm_id: XwmId,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        window: X11Surface,
    ) {
        debug!(
            "XWayland surface associated: wl_surface={:?} x11={:?}",
            surface.id(),
            window.window_id()
        );
        // The actual window mapping happens in map_window_request / mapped_override_redirect_window.
    }
}

// ---------------------------------------------------------------------------
// XwmHandler
// ---------------------------------------------------------------------------

impl smithay::xwayland::XwmHandler for MilkyState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("XWM not initialised")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!("X11 new_window: {:?}", window.window_id());
        // Nothing needed — we wait for map_window_request.
        let _ = window;
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!("X11 new_override_redirect: {:?}", window.window_id());
        let _ = window;
    }

    /// Called when an X11 app wants to be shown (equivalent to new_toplevel for XDG).
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!("X11 map_window_request: {:?}", window.window_id());

        // Set the window geometry to current tiling size so it tiles properly.
        let screen = self.tiling_rect();
        if let Err(e) = window.set_mapped(true) {
            warn!("X11 set_mapped failed: {e:?}");
        }
        if let Err(e) = window.configure(Rectangle::new(
            (0, 0).into(),
            (screen.w, screen.h).into(),
        )) {
            warn!("X11 configure failed: {e:?}");
        }

        // Wrap in a smithay Window and add to the compositor.
        let w = Window::new_x11_window(window);
        self.space.map_element(w.clone(), (0, 0), false);
        self.orbital.add_window(w);

        // Re-tile.
        self.re_tile();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!("X11 mapped_override_redirect: {:?}", window.window_id());
        // Override-redirect windows (menus, tooltips) — just place them at reported geometry.
        let geo = window.geometry();
        let w = Window::new_x11_window(window);
        self.space.map_element(w, (geo.loc.x, geo.loc.y), false);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!("X11 unmapped: {:?}", window.window_id());
        // Find matching Window in space and remove it.
        let found = self
            .space
            .elements()
            .find(|w| {
                w.x11_surface()
                    .map(|s| s.window_id() == window.window_id())
                    .unwrap_or(false)
            })
            .cloned();

        if let Some(w) = found {
            self.orbital.remove_window(&w);
            self.space.unmap_elem(&w);
            self.re_tile();
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!("X11 destroyed: {:?}", window.window_id());
        // Same cleanup as unmap — in case we didn't get an unmap event.
        let found = self
            .space
            .elements()
            .find(|w| {
                w.x11_surface()
                    .map(|s| s.window_id() == window.window_id())
                    .unwrap_or(false)
            })
            .cloned();

        if let Some(w) = found {
            self.orbital.remove_window(&w);
            self.space.unmap_elem(&w);
            self.re_tile();
        }
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // Acknowledge the request but enforce our own tiling geometry.
        // We reply with the current tile rect for this window.
        let screen = self.tiling_rect();
        let ws = self.orbital.active_ws();
        let tile = ws
            .tile_rects(screen)
            .into_iter()
            .find(|(win, _)| {
                win.x11_surface()
                    .map(|s| s.window_id() == window.window_id())
                    .unwrap_or(false)
            })
            .map(|(_, r)| r);

        let rect = if let Some(r) = tile {
            Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
        } else {
            // Fallback: honour client request.
            let cur = window.geometry();
            Rectangle::new(
                (x.unwrap_or(cur.loc.x), y.unwrap_or(cur.loc.y)).into(),
                (
                    w.map(|v| v as i32).unwrap_or(cur.size.w),
                    h.map(|v| v as i32).unwrap_or(cur.size.h),
                )
                    .into(),
            )
        };

        if let Err(e) = window.configure(rect) {
            warn!("X11 configure_request reply failed: {e:?}");
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // No-op — we manage positions ourselves.
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _resize_edge: ResizeEdge,
    ) {
        // Not implemented — tiling layout handles sizing.
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {
        // Not implemented — tiling layout handles positioning.
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        _selection: SelectionTarget,
        _mime_type: String,
        _fd: OwnedFd,
    ) {
        // Clipboard selection forwarding — stub for now.
    }
}
