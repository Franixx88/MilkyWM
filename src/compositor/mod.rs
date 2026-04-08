/// Compositor protocol handlers.
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

impl XdgShellHandler for MilkyState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        debug!("new toplevel window — adding to orbital system");
        self.space.map_element(window.clone(), (0, 0), false);
        self.orbital.add_window(window);
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let window = self.space.elements().find(|w| {
            w.wl_surface().as_deref() == Some(surface.wl_surface())
        }).cloned();

        if let Some(window) = window {
            debug!("toplevel destroyed — removing from orbital system");
            self.orbital.remove_window(&window);
            self.space.unmap_elem(&window);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}
}

impl ShmHandler for MilkyState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

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
    ) {}
}

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

/// Userdata attached to every connected Wayland client.
#[derive(Default)]
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
