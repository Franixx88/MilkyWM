pub mod gles;
pub mod space;
pub mod thumbnail;

use crate::config::Config;
use space::Starfield;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::glow::GlowRenderer;
use smithay::desktop::space::SpaceRenderElements;

use gles::StarfieldElement;

smithay::backend::renderer::element::render_elements! {
    /// All render elements used by the MilkyWM compositor.
    ///
    /// Z-order (render_output_internal iterates the slice in reverse):
    ///   Border    — first in vec → drawn last → on top of everything
    ///   Space     — window surfaces in the middle
    ///   Starfield — last in vec  → drawn first → below all windows
    pub MilkyRenderElement<=GlowRenderer>;
    Border    = SolidColorRenderElement,
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
    pub const SPACE_BLACK: [f32; 4]      = [0.00, 0.00, 0.03, 1.00];
    #[allow(dead_code)]
    pub const STAR_WHITE: [f32; 4]       = [0.85, 0.90, 1.00, 1.00];
    pub const ORBIT_RING: [f32; 4]       = [0.20, 0.30, 0.50, 0.15];
    pub const SUN_INNER: [f32; 4]        = [1.00, 0.92, 0.60, 0.90];
    pub const SUN_OUTER: [f32; 4]        = [1.00, 0.50, 0.10, 0.00];
    pub const PLANET_BORDER: [f32; 4]    = [0.40, 0.60, 1.00, 0.80];
    pub const PLANET_HOVER: [f32; 4]     = [0.80, 0.90, 1.00, 1.00];
    /// Border around the focused (active) window.
    pub const WIN_BORDER_FOCUSED: [f32; 4]   = [0.35, 0.65, 1.00, 1.00];
    /// Border around unfocused windows — dimmer, transparent.
    pub const WIN_BORDER_UNFOCUSED: [f32; 4] = [0.20, 0.28, 0.42, 0.55];
    /// Width of window border in physical pixels.
    pub const WIN_BORDER_WIDTH: i32 = 2;
}
