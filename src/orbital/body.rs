use glam::Vec2;
use smithay::desktop::Window;

const ORBIT_CAPACITY: usize = 7;
pub const ORBIT_BASE_RADIUS: f32 = 320.0;
pub const ORBIT_STEP: f32 = 180.0;

// ---------------------------------------------------------------------------

/// A window treated as a celestial body in the orbital switcher.
#[derive(Debug, Clone)]
pub struct Planet {
    pub window: Window,

    /// Angle on the orbit (radians, 0 = 12 o'clock).
    pub angle: f32,

    /// Concentric orbit ring index (0 = innermost).
    pub orbit_index: usize,

    /// Visual scale factor (1.0 normal, 1.25 when hovered).
    pub scale: f32,
    pub scale_target: f32,

    /// Opacity fade-in (0 → 1 over ~0.5 s on creation).
    pub alpha: f32,

    /// Extra angular offset added on switcher open, decays to 0 → "spin-in" effect.
    ///
    /// When the switcher opens each planet starts offset by `spin_offset` radians
    /// and the offset eases out to 0, so planets appear to fly into their final positions.
    pub spin_offset: f32,

    /// Radial offset from nominal orbit radius; decays to 0 → "zoom-in" entrance.
    pub radial_offset: f32,
}

impl Planet {
    pub fn new(window: Window, angle: f32, orbit_index: usize) -> Self {
        Self {
            window,
            angle,
            orbit_index,
            scale: 0.0,
            scale_target: 1.0,
            alpha: 0.0,
            spin_offset: 0.0,
            radial_offset: 0.0,
        }
    }

    /// Trigger entry animation when the orbital switcher is opened.
    ///
    /// Each planet gets a random-ish spin and radial offset based on its slot,
    /// which then eases back to zero.
    pub fn trigger_entry(&mut self) {
        // Deterministic pseudo-random per-planet variation using angle as seed.
        let sign = if (self.orbit_index + self.angle.to_bits() as usize) % 2 == 0 { 1.0_f32 } else { -1.0_f32 };
        self.spin_offset   = sign * (0.4 + (self.angle * 0.3).abs().min(0.6));
        self.radial_offset = 180.0 + self.orbit_index as f32 * 60.0;
    }

    pub fn orbit_radius(&self) -> f32 {
        ORBIT_BASE_RADIUS + self.orbit_index as f32 * ORBIT_STEP
    }

    /// World-space position relative to the workspace origin.
    pub fn world_pos(&self) -> Vec2 {
        let r = self.orbit_radius() + self.radial_offset;
        let a = self.angle + self.spin_offset;
        Vec2::new(r * a.sin(), -r * a.cos())
    }

    pub fn tick(&mut self, dt: f32) {
        // Scale ease-out
        self.scale += (self.scale_target - self.scale) * (1.0 - (-8.0 * dt).exp());
        // Fade-in
        self.alpha = (self.alpha + dt * 2.0).min(1.0);
        // Spin and radial offset decay (faster ease-out than scale)
        let k = 1.0 - (-6.0 * dt).exp();
        self.spin_offset   += (0.0 - self.spin_offset)   * k;
        self.radial_offset += (0.0 - self.radial_offset) * k;
    }

    pub fn visual_diameter(&self) -> f32 {
        64.0 * self.scale
    }
}

// ---------------------------------------------------------------------------

/// Distributes planets evenly across concentric orbit rings.
pub fn assign_orbits(planets: &mut Vec<Planet>) {
    let n = planets.len();
    if n == 0 { return; }

    for (i, planet) in planets.iter_mut().enumerate() {
        let orbit_index = i / ORBIT_CAPACITY;
        let slot = i % ORBIT_CAPACITY;

        let ring_start = orbit_index * ORBIT_CAPACITY;
        let ring_end   = ((orbit_index + 1) * ORBIT_CAPACITY).min(n);
        let ring_count = ring_end - ring_start;

        planet.orbit_index = orbit_index;
        planet.angle       = (std::f32::consts::TAU / ring_count as f32) * slot as f32;
    }
}
