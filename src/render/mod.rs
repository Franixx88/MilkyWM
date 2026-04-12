pub mod gles;
pub mod space;
pub mod thumbnail;

use crate::config::Config;
use space::Starfield;

use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::glow::GlowRenderer;
use smithay::desktop::space::SpaceRenderElements;

use gles::StarfieldElement;

smithay::backend::renderer::element::render_elements! {
    /// All render elements used by the MilkyWM compositor.
    ///
    /// `Space` wraps whatever `render_elements_for_output` returns (window surfaces +
    /// layer-shell surfaces).  `Starfield` is pushed **last** so that
    /// `render_output_internal` (which iterates in reverse) draws it first, placing
    /// the stars visually below all Wayland windows.
    pub MilkyRenderElement<=GlowRenderer>;
    Space     = SpaceRenderElements<GlowRenderer, WaylandSurfaceRenderElement<GlowRenderer>>,
    Starfield = StarfieldElement,
}

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
