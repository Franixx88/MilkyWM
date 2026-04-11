use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

/// MilkyWM configuration, loaded from `~/.config/milkywm/config.toml`.
///
/// All fields have sensible defaults so the compositor works out of the box
/// with no config file present.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ---- Output defaults --------------------------------------------------
    /// Fallback screen width used before any output is detected.
    pub default_width: u32,
    /// Fallback screen height used before any output is detected.
    pub default_height: u32,

    // ---- Input ------------------------------------------------------------
    pub seat_name: String,

    // ---- Orbital switcher ------------------------------------------------
    /// Key that opens the orbital switcher (XKB keysym name).
    pub switcher_key: String,
    /// Animation speed multiplier (1.0 = default, higher = faster).
    pub animation_speed: f32,

    // ---- Starfield --------------------------------------------------------
    /// Number of stars in the background.
    pub star_count: usize,
    /// RNG seed for the starfield (same seed = same sky every launch).
    pub star_seed: u64,

    // ---- Appearance -------------------------------------------------------
    /// Corner radius for planet thumbnails in pixels.
    pub planet_corner_radius: f32,
    /// Width of the planet border ring in pixels.
    pub planet_border_width: f32,
    /// Radius of the sun corona glow in world-space units.
    pub sun_glow_radius: f32,
    /// Whether to show orbit ring guides when the switcher is open.
    pub show_orbit_rings: bool,

    // ---- Gaps & borders (normal work mode) --------------------------------
    /// Gap between tiled windows in pixels (future tiling mode).
    pub gap: u32,
    /// Border width for the focused window.
    pub border_width: u32,
    /// Border colour for the focused window (hex, e.g. "#7AADFF").
    pub border_color_focused: String,
    /// Border colour for unfocused windows.
    pub border_color_unfocused: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_width: 1920,
            default_height: 1080,
            seat_name: "seat0".into(),
            switcher_key: "Super_L".into(),
            animation_speed: 1.0,
            star_count: 400,
            star_seed: MILKY_WAY_SEED,
            planet_corner_radius: 12.0,
            planet_border_width: 2.0,
            sun_glow_radius: 200.0,
            show_orbit_rings: true,
            gap: 8,
            border_width: 2,
            border_color_focused: "#7AADFF".into(),
            border_color_unfocused: "#2A3A55".into(),
        }
    }
}

// "MILKYWAY" in ASCII bytes — Easter egg seed for the starfield
const MILKY_WAY_SEED: u64 = 0x4D494C4B_59574159;

impl Config {
    /// Load config from the standard location, falling back to defaults.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!("Could not load config ({e}), using defaults");
                Self::default()
            }
        }
    }

    fn try_load() -> anyhow::Result<Self> {
        let path = config_path();
        let text = std::fs::read_to_string(&path)?;
        let cfg: Config = toml::from_str(&text)?;
        Ok(cfg)
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config")
        });
    base.join("milkywm").join("config.toml")
}

pub mod watcher;
