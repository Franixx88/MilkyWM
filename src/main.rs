mod compositor;
mod config;
mod orbital;
mod render;
mod state;

use std::time::Duration;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use crate::config::Config;
use crate::state::MilkyState;

fn main() -> anyhow::Result<()> {
    // --- Logging -----------------------------------------------------------
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()
            .add_directive("milkywm=debug".parse()?))
        .init();

    info!("MilkyWM starting — welcome to the cosmos");

    // --- Config ------------------------------------------------------------
    let config = Config::load();
    info!("Config loaded: {:?}", config);

    // --- Wayland display & event loop --------------------------------------
    let mut event_loop: EventLoop<MilkyState> = EventLoop::try_new()?;
    let mut display: Display<MilkyState> = Display::new()?;

    // --- Compositor state --------------------------------------------------
    let mut state = MilkyState::new(&mut event_loop, &mut display, config)?;

    // Listen on $WAYLAND_DISPLAY (e.g. wayland-1)
    let socket_name = state.socket_name.clone();
    info!("Listening on Wayland socket: {}", socket_name);
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    // --- Main event loop ---------------------------------------------------
    event_loop.run(
        Some(Duration::from_millis(16)), // ~60 fps tick
        &mut state,
        |state| {
            // Called after each event batch — drive rendering
            state.on_idle();
        },
    )?;

    Ok(())
}
