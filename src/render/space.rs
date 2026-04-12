use glam::Vec2;

/// A single star in the background starfield.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Star {
    /// Position (normalised 0..1 on each axis).
    pub pos: Vec2,
    /// Brightness (0.0 … 1.0).
    pub brightness: f32,
    /// Base point-sprite radius in pixels.
    pub radius: f32,
    /// Twinkle phase offset (radians).
    pub phase: f32,
}

// ---------------------------------------------------------------------------

/// One parallax layer of stars.
///
/// Each layer has a different depth — far layers move slower with the camera
/// and contain smaller, dimmer stars; near layers move faster and are bigger.
#[derive(Debug, Clone)]
pub struct StarLayer {
    pub stars: Vec<Star>,
    /// How much the camera offset affects this layer's scroll (0 = fixed, 1 = full speed).
    pub parallax_factor: f32,
    /// Point-sprite size multiplier.
    pub size_scale: f32,
}

/// Three-layer parallax starfield.
pub struct Starfield {
    /// Layer 0 — distant, slow, many small dim stars.
    pub far:    StarLayer,
    /// Layer 1 — mid-distance, medium everything.
    pub mid:    StarLayer,
    /// Layer 2 — close, fast, few large bright stars.
    pub near:   StarLayer,
    /// Global animation time (seconds).
    pub time:   f32,
}

impl Starfield {
    pub fn new(total_count: usize, seed: u64) -> Self {
        let mut rng = LcgRng::new(seed);

        // Split count across layers: far gets most stars, near gets fewest.
        let far_n  = total_count * 6 / 10;
        let mid_n  = total_count * 3 / 10;
        let near_n = total_count - far_n - mid_n;

        Self {
            far: StarLayer {
                stars: gen_stars(&mut rng, far_n, 0.15, 0.55, 0.5, 1.2),
                parallax_factor: 0.002,
                size_scale: 0.7,
            },
            mid: StarLayer {
                stars: gen_stars(&mut rng, mid_n, 0.25, 0.75, 0.8, 2.0),
                parallax_factor: 0.006,
                size_scale: 1.0,
            },
            near: StarLayer {
                stars: gen_stars(&mut rng, near_n, 0.50, 1.00, 1.2, 3.0),
                parallax_factor: 0.014,
                size_scale: 1.6,
            },
            time: 0.0,
        }
    }

    /// Advance the twinkle / shimmer animation.
    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
    }

    /// All three layers in near-to-far draw order (far first → near on top).
    pub fn layers(&self) -> [&StarLayer; 3] {
        [&self.far, &self.mid, &self.near]
    }

    #[allow(dead_code)]
    pub fn star_brightness(&self, star: &Star) -> f32 {
        let twinkle = (self.time * 1.5 + star.phase).sin() * 0.12;
        (star.brightness + twinkle).clamp(0.0, 1.0)
    }
}

fn gen_stars(
    rng: &mut LcgRng,
    count: usize,
    brightness_min: f32,
    brightness_max: f32,
    radius_min: f32,
    radius_max: f32,
) -> Vec<Star> {
    (0..count)
        .map(|_| Star {
            pos:        Vec2::new(rng.next_f32(), rng.next_f32()),
            brightness: rng.next_f32() * (brightness_max - brightness_min) + brightness_min,
            radius:     rng.next_f32() * (radius_max - radius_min) + radius_min,
            phase:      rng.next_f32() * std::f32::consts::TAU,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Minimal LCG random number generator
// ---------------------------------------------------------------------------

struct LcgRng(u64);

impl LcgRng {
    fn new(seed: u64) -> Self { Self(seed) }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 33) as f32 / (u32::MAX as f32)
    }
}
