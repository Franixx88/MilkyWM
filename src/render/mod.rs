pub mod gles;
pub mod space;
pub mod thumbnail;

use crate::config::Config;
use space::Starfield;

pub struct SpaceRenderer {
    pub starfield: Starfield,
}

impl SpaceRenderer {
    pub fn new(config: &Config) -> Self {
        Self { starfield: Starfield::new(config.star_count, config.star_seed) }
    }
}

pub mod palette {
    #[allow(dead_code)]
    pub const SPACE_BLACK: [f32; 4]   = [0.0,  0.0,  0.03, 1.0];
    #[allow(dead_code)]
    pub const STAR_WHITE: [f32; 4]    = [0.85, 0.90, 1.00, 1.0];
    pub const ORBIT_RING: [f32; 4]    = [0.20, 0.30, 0.50, 0.15];
    pub const SUN_INNER: [f32; 4]     = [1.00, 0.92, 0.60, 0.90];
    pub const SUN_OUTER: [f32; 4]     = [1.00, 0.50, 0.10, 0.00];
    pub const PLANET_BORDER: [f32; 4] = [0.40, 0.60, 1.00, 0.80];
    pub const PLANET_HOVER: [f32; 4]  = [0.80, 0.90, 1.00, 1.00];
}
