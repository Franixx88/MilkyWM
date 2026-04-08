use glam::Vec2;
use smithay::desktop::Window;

/// How far a planet is from the sun (in world-space units).
///
/// Windows are placed on one of several concentric orbits so that the
/// display doesn't become cluttered when many windows are open.
///
///  Orbit 0 (inner) — up to `ORBIT_CAPACITY` planets
///  Orbit 1         — next `ORBIT_CAPACITY` planets
///  …
const ORBIT_CAPACITY: usize = 7;

/// Base radius of the first orbit in world-space units.
pub const ORBIT_BASE_RADIUS: f32 = 320.0;

/// Each additional orbit is this many units further out.
pub const ORBIT_STEP: f32 = 180.0;

// ---------------------------------------------------------------------------

/// A window treated as a celestial body in the orbital switcher.
#[derive(Debug, Clone)]
pub struct Planet {
    /// The underlying Wayland window.
    pub window: Window,

    /// Angle on the orbit, in radians. 0 = top (12 o'clock).
    pub angle: f32,

    /// Which concentric orbit ring this planet is on (0 = innermost).
    pub orbit_index: usize,

    /// Visual scale factor (1.0 = normal planet size).
    /// Animated to 1.2 when hovered.
    pub scale: f32,

    /// Target scale — `scale` lerps toward this each tick.
    pub scale_target: f32,

    /// Alpha (0.0 … 1.0) — planets fade in when added.
    pub alpha: f32,
}

impl Planet {
    pub fn new(window: Window, angle: f32, orbit_index: usize) -> Self {
        Self {
            window,
            angle,
            orbit_index,
            scale: 0.0,       // starts invisible, fades in
            scale_target: 1.0,
            alpha: 0.0,
        }
    }

    /// World-space radius of this planet's orbit.
    pub fn orbit_radius(&self) -> f32 {
        ORBIT_BASE_RADIUS + self.orbit_index as f32 * ORBIT_STEP
    }

    /// World-space position of the planet's centre relative to the sun.
    pub fn world_pos(&self) -> Vec2 {
        let r = self.orbit_radius();
        Vec2::new(r * self.angle.sin(), -r * self.angle.cos())
    }

    /// Advance per-frame animations.
    pub fn tick(&mut self, dt: f32) {
        // Smooth scale transition (ease-out)
        self.scale += (self.scale_target - self.scale) * (1.0 - (-8.0 * dt).exp());
        // Fade in
        self.alpha = (self.alpha + dt * 2.0).min(1.0);
    }

    /// Visual diameter in world-space units.
    pub fn visual_diameter(&self) -> f32 {
        64.0 * self.scale
    }
}

// ---------------------------------------------------------------------------

/// Assigns each new planet its angle and orbit ring so that planets are
/// distributed evenly around all orbit rings.
pub fn assign_orbits(planets: &mut Vec<Planet>) {
    let n = planets.len();
    if n == 0 {
        return;
    }

    for (i, planet) in planets.iter_mut().enumerate() {
        let orbit_index = i / ORBIT_CAPACITY;
        let slot = i % ORBIT_CAPACITY;

        // How many planets share this orbit ring?
        let ring_start = orbit_index * ORBIT_CAPACITY;
        let ring_end = ((orbit_index + 1) * ORBIT_CAPACITY).min(n);
        let ring_count = ring_end - ring_start;

        let angle = (std::f32::consts::TAU / ring_count as f32) * slot as f32;

        planet.orbit_index = orbit_index;
        planet.angle = angle;
    }
}
