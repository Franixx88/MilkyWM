use glam::{Vec2, Mat3};

/// Camera zoom levels used during orbital-switcher transitions.
///
///  `Work`   — zoomed in on the sun window; fills the screen.
///  `System` — zoomed out to show the full solar system.
///  `Galaxy` — zoomed way out to show all window groups (future feature).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomLevel {
    Work,
    System,
    Galaxy,
}

impl ZoomLevel {
    /// World → screen scale factor for each zoom level.
    pub fn scale(self) -> f32 {
        match self {
            ZoomLevel::Work   => 1.0,
            ZoomLevel::System => 0.35,
            ZoomLevel::Galaxy => 0.10,
        }
    }
}

// ---------------------------------------------------------------------------

/// Camera that maps world-space to screen-space.
///
/// All values are smoothly interpolated each tick for buttery animations.
#[derive(Debug, Clone)]
pub struct Camera {
    /// Current look-at position in world-space.
    pub position: Vec2,
    /// Current zoom (world → screen scale factor).
    pub zoom: f32,

    /// Where the camera is animating toward.
    pub target_position: Vec2,
    pub target_zoom: f32,

    /// Screen dimensions in physical pixels (updated on output change).
    pub screen_size: Vec2,

    /// Current conceptual zoom level (for state-machine logic).
    pub level: ZoomLevel,
}

impl Camera {
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        Self {
            position: Vec2::ZERO,
            zoom: ZoomLevel::Work.scale(),
            target_position: Vec2::ZERO,
            target_zoom: ZoomLevel::Work.scale(),
            screen_size: Vec2::new(screen_width as f32, screen_height as f32),
            level: ZoomLevel::Work,
        }
    }

    /// Animate camera toward its targets (call once per frame).
    pub fn tick(&mut self, dt: f32) {
        // Exponential ease-out — feels physically natural
        let k = 1.0 - (-10.0 * dt).exp();
        self.position += (self.target_position - self.position) * k;
        self.zoom     += (self.target_zoom     - self.zoom)     * k;
    }

    /// Set new zoom level and optionally a new look-at position.
    pub fn set_zoom(&mut self, level: ZoomLevel, look_at: Option<Vec2>) {
        self.level = level;
        self.target_zoom = level.scale();
        if let Some(pos) = look_at {
            self.target_position = pos;
        }
    }

    /// Fly immediately to a world-space position (no animation).
    pub fn snap_to(&mut self, position: Vec2, zoom: f32) {
        self.position = position;
        self.target_position = position;
        self.zoom = zoom;
        self.target_zoom = zoom;
    }

    /// Returns true while the camera is still animating.
    pub fn is_animating(&self) -> bool {
        (self.position - self.target_position).length() > 0.5
            || (self.zoom - self.target_zoom).abs() > 0.001
    }

    /// World-space → screen-space transform matrix (for the renderer).
    ///
    /// Applies:  translate(screen_centre)  ·  scale(zoom)  ·  translate(-camera_pos)
    pub fn world_to_screen(&self) -> Mat3 {
        let half = self.screen_size * 0.5;
        Mat3::from_translation(half)
            * Mat3::from_scale(Vec2::splat(self.zoom))
            * Mat3::from_translation(-self.position)
    }

    /// Convert a screen-space point to world-space (e.g. for mouse picking).
    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        let half = self.screen_size * 0.5;
        (screen_pos - half) / self.zoom + self.position
    }
}
