use glam::Vec2;

/// A single star in the background starfield.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Star {
    /// Position in world-space (normalised 0..1 across each axis,
    /// then scaled to canvas size at render time).
    pub pos: Vec2,
    /// Brightness (0.0 … 1.0).
    pub brightness: f32,
    /// Radius in screen pixels.
    pub radius: f32,
    /// Twinkle phase offset (radians).
    pub phase: f32,
}

// ---------------------------------------------------------------------------

/// Procedurally generates the starfield that fills the space background.
///
/// Uses a simple LCG so the field is deterministic across frames and sessions
/// (same seed → same sky), which avoids distracting flicker on startup.
pub struct Starfield {
    pub stars: Vec<Star>,
    pub time: f32,
}

impl Starfield {
    /// Generate `count` stars using the given seed.
    pub fn new(count: usize, seed: u64) -> Self {
        let mut rng = LcgRng::new(seed);
        let stars = (0..count)
            .map(|_| Star {
                pos: Vec2::new(rng.next_f32(), rng.next_f32()),
                brightness: rng.next_f32() * 0.6 + 0.2,
                radius: rng.next_f32() * 1.5 + 0.5,
                phase: rng.next_f32() * std::f32::consts::TAU,
            })
            .collect();

        Self { stars, time: 0.0 }
    }

    /// Advance the twinkle animation.
    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
    }

    /// Effective brightness of a star at the current time (with twinkle).
    #[allow(dead_code)]
    pub fn star_brightness(&self, star: &Star) -> f32 {
        let twinkle = (self.time * 1.5 + star.phase).sin() * 0.12;
        (star.brightness + twinkle).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Minimal LCG random number generator
// (no external crate needed for something this simple)
// ---------------------------------------------------------------------------

struct LcgRng(u64);

impl LcgRng {
    fn new(seed: u64) -> Self { Self(seed) }

    fn next_u64(&mut self) -> u64 {
        // Knuth multiplicative LCG
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 33) as f32 / (u32::MAX as f32)
    }
}
