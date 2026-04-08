use smithay::{
    delegate_compositor, delegate_data_device, delegate_output,
    delegate_seat, delegate_shm, delegate_xdg_shell,
    desktop::{Space, Window},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, LoopHandle, LoopSignal},
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Point},
    wayland::{
        compositor::CompositorState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};
use tracing::info;

use crate::{config::Config, orbital::OrbitalSwitcher, render::SpaceRenderer};

/// Global compositor state — owns everything.
pub struct MilkyState {
    // ---- Wayland core -----------------------------------------------------
    pub display_handle: DisplayHandle,
    pub socket_name: String,
    pub loop_handle: LoopHandle<'static, MilkyState>,
    pub loop_signal: LoopSignal,

    // ---- Smithay protocol states ------------------------------------------
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub seat_state: SeatState<MilkyState>,
    pub seat: Seat<MilkyState>,

    // ---- Desktop ----------------------------------------------------------
    /// The smithay Space — tracks window positions in logical space.
    pub space: Space<Window>,

    // ---- MilkyWM-specific -------------------------------------------------
    /// The orbital switcher: manages the solar-system overlay.
    pub orbital: OrbitalSwitcher,

    /// The space renderer: draws the starfield background and window textures.
    pub renderer: SpaceRenderer,

    pub config: Config,
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

        // --- Wayland socket ------------------------------------------------
        let source = ListeningSocketSource::new_auto()?;
        let socket_name = source.socket_name().to_string_lossy().into_owned();
        loop_handle.insert_source(source, |client_stream, _, state: &mut MilkyState| {
            state.display_handle
                .insert_client(client_stream, std::sync::Arc::new(()))
                .expect("failed to insert client");
        })?;

        // --- Protocol states -----------------------------------------------
        let compositor_state = CompositorState::new::<MilkyState>(&dh);
        let xdg_shell_state = XdgShellState::new::<MilkyState>(&dh);
        let shm_state = ShmState::new::<MilkyState>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<MilkyState>(&dh);
        let data_device_state = DataDeviceState::new::<MilkyState>(&dh);

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, config.seat_name.clone());

        let space = Space::default();
        let orbital = OrbitalSwitcher::new(&config);
        let renderer = SpaceRenderer::new(&config);

        info!("MilkyState initialised");

        Ok(Self {
            display_handle: dh,
            socket_name,
            loop_handle,
            loop_signal,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            data_device_state,
            seat_state,
            seat,
            space,
            orbital,
            renderer,
            config,
        })
    }

    /// Called once per event-loop iteration (idle callback).
    pub fn on_idle(&mut self) {
        // Advance orbital animations
        self.orbital.tick();
        // Render frame
        self.renderer.render_frame(&self.space, &self.orbital, &self.config);
        // Flush pending Wayland events to clients
        self.display_handle.flush_clients();
    }
}

// ---------------------------------------------------------------------------
// Smithay delegate macros — wire protocol handlers to MilkyState
// ---------------------------------------------------------------------------

delegate_compositor!(MilkyState);
delegate_xdg_shell!(MilkyState);
delegate_shm!(MilkyState);
delegate_output!(MilkyState);
delegate_seat!(MilkyState);
delegate_data_device!(MilkyState);
