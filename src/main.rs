mod backend;
mod compositor;
mod config;
mod orbital;
mod render;
mod state;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::MilkyState;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("milkywm=debug,smithay=warn")),
        )
        .init();

    info!("MilkyWM — welcome to the cosmos");

    let config = Config::load();

    let mut event_loop: EventLoop<MilkyState> = EventLoop::try_new()?;
    let mut display: Display<MilkyState> = Display::new()?;

    let mut state = MilkyState::new(&mut event_loop, &mut display, config)?;

    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    info!("Wayland socket: {}", state.socket_name);

    let backend_name = backend::init(&mut event_loop, &mut state)?;
    info!("Backend: {backend_name}");

    event_loop.run(
        None,
        &mut state,
        |state| {
            state.flush_clients();
        },
    )?;

    info!("MilkyWM exiting — safe travels");
    Ok(())
}
