pub mod space;

use smithay::desktop::{Space, Window};

use crate::{config::Config, orbital::OrbitalSwitcher};
use space::Starfield;

/// High-level renderer for MilkyWM.
///
/// Rendering pipeline each frame
/// ──────────────────────────────
///  1. Clear to deep-space black.
///  2. Draw starfield (parallax-scrolls with camera position).
///  3. If switcher is visible:
///     a. Draw orbit rings (faint ellipses).
///     b. Draw planet thumbnails with glow + scale animation.
///     c. Draw the sun (current window) with a warm corona effect.
///  4. Draw window surfaces (via smithay's renderer).
///  5. Draw cursor.
///
/// NOTE: The actual GL/wgpu draw calls are wired up when the output backend
/// is initialised (DRM or winit).  This struct holds the render-side state
/// that is backend-agnostic.
pub struct SpaceRenderer {
    pub starfield: Starfield,
}

impl SpaceRenderer {
    pub fn new(config: &Config) -> Self {
        Self {
            starfield: Starfield::new(config.star_count, config.star_seed),
        }
    }

    /// Called once per frame from `MilkyState::on_idle`.
    ///
    /// For now this drives the starfield animation; actual GPU draw calls will
    /// be dispatched from the backend-specific render loop added later.
    pub fn render_frame(
        &mut self,
        _space: &Space<Window>,
        orbital: &OrbitalSwitcher,
        _config: &Config,
    ) {
        // Advance starfield twinkle
        let dt = 1.0 / 60.0; // placeholder until we wire up real delta-time
        self.starfield.tick(dt);

        // The actual draw order (to be implemented per backend):
        //
        //  draw_background()         — fill with #000008 (near-black space)
        //  draw_starfield()          — scatter star quads with brightness
        //  if orbital.state == Visible {
        //      draw_orbit_rings()    — faint circles at each orbit radius
        //      draw_planets()        — window thumbnails as planet discs
        //      draw_sun_corona()     — warm glow around the focused window
        //  }
        //  draw_windows()            — render actual Wayland surfaces
        //  draw_cursor()             — hardware or software cursor
        let _ = orbital; // suppress unused warning until implemented
    }
}

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

/// The MilkyWM colour palette (linear sRGB, alpha = 1.0 unless noted).
pub mod palette {
    /// Deep-space background.
    pub const SPACE_BLACK: [f32; 4] = [0.0, 0.0, 0.03, 1.0];

    /// Faint star colour (white-blue tint).
    pub const STAR_WHITE: [f32; 4] = [0.85, 0.90, 1.00, 1.0];

    /// Orbit ring stroke (very faint).
    pub const ORBIT_RING: [f32; 4] = [0.20, 0.30, 0.50, 0.15];

    /// Sun corona inner glow (warm yellow).
    pub const SUN_INNER: [f32; 4] = [1.00, 0.92, 0.60, 0.9];

    /// Sun corona outer glow (orange fade).
    pub const SUN_OUTER: [f32; 4] = [1.00, 0.50, 0.10, 0.0];

    /// Planet thumbnail border.
    pub const PLANET_BORDER: [f32; 4] = [0.40, 0.60, 1.00, 0.8];

    /// Highlighted planet border.
    pub const PLANET_HOVER: [f32; 4] = [0.80, 0.90, 1.00, 1.0];
}
