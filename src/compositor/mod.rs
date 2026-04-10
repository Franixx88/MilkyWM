pub mod xwayland;

use smithay::{
    desktop::{Space, Window},
    input::{SeatHandler, SeatState},
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_seat::WlSeat, wl_surface::WlSurface},
        Resource,
    },
    utils::{Logical, Point, Serial, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{
            data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler},
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use tracing::debug;

use crate::orbital::{Rect, Workspace};
use crate::state::MilkyState;

// ---------------------------------------------------------------------------
// Layout helper
// ---------------------------------------------------------------------------

/// Re-tile all windows in a workspace and push XDG configure + Space positions.
///
/// `screen` is the current output rectangle in logical pixels.
pub fn apply_layout(space: &mut Space<Window>, ws: &Workspace, screen: Rect) {
    for (window, rect) in ws.tile_rects(screen) {
        // Tell the client its new size.
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| {
                s.size = Some(Size::<i32, Logical>::from((rect.w, rect.h)));
            });
            toplevel.send_configure();
        }
        // Move the window in the compositor space.
        space.map_element(
            window,
            Point::<i32, Logical>::from((rect.x, rect.y)),
            false,
        );
    }
}

// ---------------------------------------------------------------------------
// CompositorHandler
// ---------------------------------------------------------------------------

impl CompositorHandler for MilkyState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a smithay::reexports::wayland_server::Client,
    ) -> &'a CompositorClientState {
        &client.get_data::<ClientData>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<MilkyState>(surface);
        debug!("surface commit: {:?}", surface.id());
    }
}

// ---------------------------------------------------------------------------
// XdgShellHandler
// ---------------------------------------------------------------------------

impl XdgShellHandler for MilkyState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        debug!("new toplevel — adding to orbital system");
        // Map at origin initially; apply_layout will reposition.
        self.space.map_element(window.clone(), (0, 0), false);
        self.orbital.add_window(window);

        // Re-tile the active workspace with the new window included.
        let screen = self.screen_rect();
        let ws = self.orbital.active_ws().clone();
        apply_layout(&mut self.space, &ws, screen);
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let window = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface.wl_surface()))
            .cloned();
        if let Some(window) = window {
            debug!("toplevel destroyed — removing from orbital system");
            self.orbital.remove_window(&window);
            self.space.unmap_elem(&window);

            // Re-tile after removal.
            let screen = self.screen_rect();
            let ws = self.orbital.active_ws().clone();
            apply_layout(&mut self.space, &ws, screen);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}
}

// ---------------------------------------------------------------------------
// ShmHandler
// ---------------------------------------------------------------------------

impl ShmHandler for MilkyState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

// ---------------------------------------------------------------------------
// SeatHandler
// ---------------------------------------------------------------------------

impl SeatHandler for MilkyState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<MilkyState> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        focused: Option<&WlSurface>,
    ) {
        if let Some(surface) = focused {
            let window = self
                .space
                .elements()
                .find(|w| w.wl_surface().as_deref() == Some(surface))
                .cloned();
            if let Some(window) = window {
                self.orbital.set_sun(window);
            }
        }
    }

    fn cursor_image(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }
}

// ---------------------------------------------------------------------------
// DataDevice (clipboard / DnD)
// ---------------------------------------------------------------------------

impl SelectionHandler for MilkyState {
    type SelectionUserData = ();
}

/// Smithay 0.7: WaylandDndGrabHandler replaces Client/ServerDndGrabHandler.
impl WaylandDndGrabHandler for MilkyState {}

impl smithay::input::dnd::DndGrabHandler for MilkyState {}

impl DataDeviceHandler for MilkyState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

// ---------------------------------------------------------------------------
// Per-client userdata
// ---------------------------------------------------------------------------

/// Userdata attached to every connected Wayland client.
#[derive(Default)]
pub struct ClientData {
    pub compositor_state: CompositorClientState,
}

// ---------------------------------------------------------------------------
// BufferHandler (required by delegate_shm)
// ---------------------------------------------------------------------------

impl BufferHandler for MilkyState {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

// ---------------------------------------------------------------------------
// OutputHandler (required by delegate_output)
// ---------------------------------------------------------------------------

impl OutputHandler for MilkyState {}

// ---------------------------------------------------------------------------

impl smithay::reexports::wayland_server::backend::ClientData for ClientData {
    fn initialized(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
    ) {
    }
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}
