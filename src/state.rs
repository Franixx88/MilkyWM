use smithay::{
    delegate_compositor, delegate_data_device, delegate_fractional_scale,
    delegate_layer_shell, delegate_output,
    delegate_seat, delegate_shm, delegate_viewporter, delegate_xdg_decoration,
    delegate_xdg_shell, delegate_xwayland_shell,
    desktop::{Space, Window},
    input::{Seat, SeatState, pointer::CursorImageStatus},
    output::Output,
    reexports::{
        calloop::{EventLoop, LoopHandle, LoopSignal},
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Point},
    wayland::{
        compositor::CompositorState,
        fractional_scale::FractionalScaleManagerState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
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
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub viewporter_state: ViewporterState,
    pub fractional_scale_state: FractionalScaleManagerState,
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
    /// Last known cursor position in logical output coordinates.
    pub cursor_pos: Point<f64, Logical>,
    /// Current cursor image as requested by the focused client (or default).
    pub cursor_status: CursorImageStatus,
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
        let xdg_decoration_state = XdgDecorationState::new::<MilkyState>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<MilkyState>(&dh);
        let viewporter_state = ViewporterState::new::<MilkyState>(&dh);
        let fractional_scale_state = FractionalScaleManagerState::new::<MilkyState>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<MilkyState>(&dh);
        let shm_state = ShmState::new::<MilkyState>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<MilkyState>(&dh);
        let data_device_state = DataDeviceState::new::<MilkyState>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, config.seat_name.clone());

        // Register keyboard and pointer capabilities so get_keyboard() / get_pointer()
        // return Some(…) instead of None. Without these calls ALL input events are
        // silently dropped — keyboard shortcuts, terminal launch, pointer forwarding.
        seat.add_keyboard(smithay::input::keyboard::XkbConfig::default(), 200, 25)
            .expect("failed to add keyboard to seat");
        seat.add_pointer();

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
            xdg_decoration_state,
            layer_shell_state,
            viewporter_state,
            fractional_scale_state,
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
            cursor_pos: Point::default(),
            cursor_status: CursorImageStatus::default_named(),
        })
    }

    pub fn flush_clients(&mut self) {
        let _ = self.display_handle.flush_clients();
    }

    /// The primary output — the first one mapped in the compositor's Space.
    ///
    /// MilkyWM's orbital layout is single-output: workspaces and tiling live
    /// on one output at a time. Secondary outputs receive the same scene but
    /// don't affect geometry decisions.
    pub fn primary_output(&self) -> Option<Output> {
        self.space.outputs().next().cloned()
    }

    /// Current screen rectangle in logical pixels, derived from the primary
    /// output's current mode. Falls back to (0,0,0,0) if no output is mapped.
    pub fn screen_rect(&self) -> Rect {
        let Some(output) = self.primary_output() else {
            return Rect::new(0, 0, 0, 0);
        };
        let Some(mode) = output.current_mode() else {
            return Rect::new(0, 0, 0, 0);
        };
        let scale = output.current_scale().fractional_scale();
        let w = (mode.size.w as f64 / scale).round() as i32;
        let h = (mode.size.h as f64 / scale).round() as i32;
        Rect::new(0, 0, w, h)
    }

    /// Rectangle available for tiling — the primary output minus layer-shell
    /// exclusive zones (panels, docks like waybar).
    pub fn tiling_rect(&self) -> Rect {
        let full = self.screen_rect();
        let Some(output) = self.primary_output() else { return full };
        let zone = smithay::desktop::layer_map_for_output(&output).non_exclusive_zone();
        Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h)
    }

    /// Push the primary output's logical size into the orbital camera.
    /// GL shaders use `screen_size` directly, so it must track the primary
    /// output when it is added, resized, or removed.
    pub fn sync_screen_size(&mut self) {
        let r = self.screen_rect();
        self.orbital.camera.screen_size = glam::Vec2::new(r.w as f32, r.h as f32);
    }

    /// Re-tile the active workspace using the current tiling rectangle.
    pub fn re_tile(&mut self) {
        let screen = self.tiling_rect();
        let ws = self.orbital.active_ws().clone();
        crate::compositor::apply_layout(&mut self.space, &ws, screen);
    }
}

delegate_compositor!(MilkyState);
delegate_xdg_shell!(MilkyState);
delegate_xdg_decoration!(MilkyState);
delegate_layer_shell!(MilkyState);
delegate_viewporter!(MilkyState);
delegate_fractional_scale!(MilkyState);
delegate_shm!(MilkyState);
delegate_output!(MilkyState);
delegate_seat!(MilkyState);
delegate_data_device!(MilkyState);
delegate_xwayland_shell!(MilkyState);
