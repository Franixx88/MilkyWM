//! Config hot-reload via inotify.
//!
//! Watches `~/.config/milkywm/config.toml` for writes and re-applies
//! the changed values to the live compositor state without a restart.
use calloop::{generic::Generic, Interest, Mode, PostAction};
use inotify::{Inotify, WatchMask};
use smithay::reexports::calloop::EventLoop;
use tracing::{info, warn};

use crate::state::MilkyState;

/// Register an inotify watcher on the config file with the calloop event loop.
///
/// On every IN_MODIFY / IN_CLOSE_WRITE event the config is reloaded and the
/// relevant parts of `MilkyState` are updated in-place.
pub fn init(event_loop: &mut EventLoop<'static, MilkyState>) -> anyhow::Result<()> {
    let path = super::config_path();

    let inotify = Inotify::init()?;

    // Watch the *directory* instead of the file so we catch atomic writes
    // (editor swaps temp file into place — the file wd would be invalidated).
    let watch_path = path.parent().unwrap_or(std::path::Path::new(".")).to_owned();
    // Make sure the directory exists; if not, silently skip hot-reload.
    if !watch_path.exists() {
        warn!("Config dir {:?} does not exist — hot-reload disabled", watch_path);
        return Ok(());
    }

    inotify
        .watches()
        .add(&watch_path, WatchMask::MODIFY | WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO)?;

    let source = Generic::new(inotify, Interest::READ, Mode::Level);
    event_loop
        .handle()
        .insert_source(source, move |_readiness, inotify, state| {
            // calloop wraps the fd in `NoIoDrop` (no `DerefMut`), so we drain
            // the inotify fd via rustix rather than calling `read_events()`.
            use std::os::unix::io::AsRawFd;
            use std::os::fd::FromRawFd;
            let raw = inotify.as_raw_fd();
            // SAFETY: we only read from the fd and immediately forget the handle.
            let mut f = unsafe { std::fs::File::from_raw_fd(raw) };
            let mut buf = [0u8; 4096];
            use std::io::Read;
            let _ = f.read(&mut buf);
            std::mem::forget(f);

            apply_reload(state);
            Ok(PostAction::Continue)
        })
        .map_err(|e| anyhow::anyhow!("insert config watcher: {e}"))?;

    info!("Config hot-reload watching {:?}", watch_path);
    Ok(())
}

fn apply_reload(state: &mut MilkyState) {
    let new_cfg = super::Config::load();
    info!("Config reloaded");

    // Update fields that can be changed live.  Fields like `seat_name` or
    // `default_width/height` require a restart and are intentionally ignored.
    state.config.animation_speed     = new_cfg.animation_speed;
    state.config.star_count          = new_cfg.star_count;
    state.config.star_seed           = new_cfg.star_seed;
    state.config.planet_corner_radius = new_cfg.planet_corner_radius;
    state.config.planet_border_width  = new_cfg.planet_border_width;
    state.config.sun_glow_radius      = new_cfg.sun_glow_radius;
    state.config.show_orbit_rings     = new_cfg.show_orbit_rings;
    state.config.gap                  = new_cfg.gap;
    state.config.border_width         = new_cfg.border_width;
    state.config.border_color_focused   = new_cfg.border_color_focused;
    state.config.border_color_unfocused = new_cfg.border_color_unfocused;

    // Re-tile the active workspace so gap changes take effect immediately.
    let screen = state.screen_rect();
    let ws = state.orbital.active_ws().clone();
    crate::compositor::apply_layout(&mut state.space, &ws, screen);
}
