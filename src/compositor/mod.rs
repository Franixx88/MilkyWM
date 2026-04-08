/// Compositor protocol handlers.
///
/// Each `impl` block here satisfies a smithay handler trait and is
/// connected to [`crate::state::MilkyState`] via the `delegate_*!` macros
/// declared in `state.rs`.
use smithay::{
    desktop::Window,
    input::{
        pointer::{
            AxisFrame, ButtonEvent, GrabStartData, MotionEvent, PointerGrab,
            PointerInnerHandle, RelativeMotionEvent,
        },
        touch::TouchTarget,
        SeatHandler, SeatState,
    },
    reexports::wayland_server::{
        protocol::{wl_seat::WlSeat, wl_surface::WlSurface},
        Resource,
    },
    utils::{Logical, Point, Serial},
    wayland::{
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
            XdgShellState, XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
    },
};
use tracing::debug;

use crate::state::MilkyState;

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
        &client.get_data::<crate::ClientData>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        smithay::desktop::utils::on_commit_buffer_handler::<MilkyState>(surface);
        debug!("surface commit: {:?}", surface.id());
    }
}

// ---------------------------------------------------------------------------
// XdgShellHandler  — maps new toplevel windows into the orbital system
// ---------------------------------------------------------------------------

impl XdgShellHandler for MilkyState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        debug!("new toplevel window — adding to orbital system");

        // Place the window in the smithay Space at origin; the renderer will
        // position it visually in world-space via the orbital camera transform.
        self.space.map_element(window.clone(), (0, 0), false);

        // Register the new window as a planet in the orbital switcher.
        self.orbital.add_window(window);
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Popups are rendered on top of their parent; no orbital placement needed.
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // Find and remove the window from both Space and OrbitalSwitcher.
        let window = self.space.elements().find(|w| {
            w.wl_surface().as_deref() == Some(surface.wl_surface())
        }).cloned();

        if let Some(window) = window {
            debug!("toplevel destroyed — removing from orbital system");
            self.orbital.remove_window(&window);
            self.space.unmap_elem(&window);
        }
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: WlSeat,
        _serial: Serial,
    ) {
        // Popup grabs — left as future work.
    }
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
            // Promote the focused window to "sun" in the orbital system.
            if let Some(window) = self.space.elements().find(|w| {
                w.wl_surface().as_deref() == Some(surface)
            }).cloned() {
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

impl DataDeviceHandler for MilkyState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for MilkyState {}
impl ServerDndGrabHandler for MilkyState {}

// ---------------------------------------------------------------------------
// Per-client data stored in the Wayland client userdata slot
// ---------------------------------------------------------------------------

/// Userdata attached to every connected Wayland client.
pub struct ClientData {
    pub compositor_state: CompositorClientState,
}

impl smithay::reexports::wayland_server::backend::ClientData for ClientData {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {}
}
