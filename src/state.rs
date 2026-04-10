use smithay::{
    delegate_compositor, delegate_data_device, delegate_output,
    delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xwayland_shell,
    desktop::{Space, Window},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, LoopHandle, LoopSignal},
        wayland_server::{Display, DisplayHandle},
    },
    wayland::{
        compositor::CompositorState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Wm, XWayland},
};
use tracing::info;

use crate::{config::Config, orbital::{OrbitalSwitcher, Rect}, render::SpaceRenderer};

pub use crate::compositor::ClientData;

#[allow(dead_code)]
pub struct MilkyState {
    pub display_handle: DisplayHandle,
    pub socket_name: String,
    pub loop_handle: LoopHandle<'static, MilkyState>,
    pub loop_signal: LoopSignal,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xwayland_shell_state: XWaylandShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub seat_state: SeatState<MilkyState>,
    pub seat: Seat<MilkyState>,
    pub space: Space<Window>,
    pub orbital: OrbitalSwitcher,
    pub renderer: SpaceRenderer,
    pub config: Config,
    /// The running XWayland instance (kept alive for the lifetime of the compositor).
    pub xwayland: Option<XWayland>,
    /// The X11 window manager, available once XWayland is ready.
    pub xwm: Option<X11Wm>,
}

impl MilkyState {
    pub fn new(
        event_loop: &mut EventLoop<'static, MilkyState>,
        display: &mut Display<MilkyState>,
        config: Config,
    ) -> anyhow::Result<Self> {
        let dh = display.handle();
        let loop_handle = event_loop.handle();
        let loop_signal = event_loop.get_signal();

        let source = ListeningSocketSource::new_auto()?;
        let socket_name = source.socket_name().to_string_lossy().into_owned();

        loop_handle.insert_source(source, |client_stream, _, state: &mut MilkyState| {
            state
                .display_handle
                .insert_client(
                    client_stream,
                    std::sync::Arc::new(ClientData::default()),
                )
                .expect("failed to insert Wayland client");
        })?;

        let compositor_state = CompositorState::new::<MilkyState>(&dh);
        let xdg_shell_state = XdgShellState::new::<MilkyState>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<MilkyState>(&dh);
        let shm_state = ShmState::new::<MilkyState>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<MilkyState>(&dh);
        let data_device_state = DataDeviceState::new::<MilkyState>(&dh);

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, config.seat_name.clone());

        let space = Space::default();
        let orbital = OrbitalSwitcher::new(&config);
        let renderer = SpaceRenderer::new(&config);

        info!("MilkyState initialised — socket: {}", socket_name);

        Ok(Self {
            display_handle: dh,
            socket_name,
            loop_handle,
            loop_signal,
            compositor_state,
            xdg_shell_state,
            xwayland_shell_state,
            shm_state,
            output_manager_state,
            data_device_state,
            seat_state,
            seat,
            space,
            orbital,
            renderer,
            config,
            xwayland: None,
            xwm: None,
        })
    }

    pub fn flush_clients(&mut self) {
        let _ = self.display_handle.flush_clients();
    }

    /// Current screen rectangle in logical pixels, derived from camera screen size.
    pub fn screen_rect(&self) -> Rect {
        let sz = self.orbital.camera.screen_size;
        Rect::new(0, 0, sz.x as i32, sz.y as i32)
    }
}

delegate_compositor!(MilkyState);
delegate_xdg_shell!(MilkyState);
delegate_shm!(MilkyState);
delegate_output!(MilkyState);
delegate_seat!(MilkyState);
delegate_data_device!(MilkyState);
delegate_xwayland_shell!(MilkyState);
