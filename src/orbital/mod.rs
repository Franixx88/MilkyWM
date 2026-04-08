pub mod body;
pub mod camera;

use glam::Vec2;
use smithay::desktop::Window;

pub use body::{Planet, assign_orbits};
pub use camera::{Camera, ZoomLevel};

use crate::config::Config;

/// Whether the orbital switcher overlay is currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitcherState {
    /// Normal work mode — camera is zoomed in on the sun.
    Hidden,
    /// Super held — orbital system visible, user is choosing a planet.
    Visible,
}

// ---------------------------------------------------------------------------

/// The orbital switcher — the core unique feature of MilkyWM.
///
/// Conceptual model
/// ────────────────
///  - One window is the **sun**: it sits at world-origin and is the focused window.
///  - All other windows are **planets**: arranged on concentric orbit rings.
///  - The **camera** determines the current view.
///    · In `Work` mode it is zoomed in and the sun fills the screen.
///    · When Super is held the camera pulls back to `System` zoom, revealing
///      the full solar system.
///  - Selecting a planet triggers a camera fly-to, followed by making that
///    window the new sun.
pub struct OrbitalSwitcher {
    /// The currently focused window (displayed at world origin as the sun).
    pub sun: Option<Window>,

    /// All other open windows, arranged as planets.
    pub planets: Vec<Planet>,

    /// Camera controlling the view.
    pub camera: Camera,

    /// Whether the overlay is currently shown.
    pub state: SwitcherState,

    /// Index of the currently highlighted planet (keyboard navigation).
    pub hovered: Option<usize>,

    /// Fixed timestep accumulator (seconds).
    last_tick: std::time::Instant,
}

impl OrbitalSwitcher {
    pub fn new(config: &Config) -> Self {
        Self {
            sun: None,
            planets: Vec::new(),
            camera: Camera::new(config.default_width, config.default_height),
            state: SwitcherState::Hidden,
            hovered: None,
            last_tick: std::time::Instant::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Window management
    // -----------------------------------------------------------------------

    /// Add a new window. It becomes a planet (or the sun if none yet).
    pub fn add_window(&mut self, window: Window) {
        if self.sun.is_none() {
            self.sun = Some(window);
        } else {
            let planet = Planet::new(window, 0.0, 0);
            self.planets.push(planet);
            assign_orbits(&mut self.planets);
        }
    }

    /// Remove a window (closed / destroyed).
    pub fn remove_window(&mut self, window: &Window) {
        // Was it the sun?
        if self.sun.as_ref() == Some(window) {
            // Promote the first planet to sun, if any.
            if !self.planets.is_empty() {
                let promoted = self.planets.remove(0);
                self.sun = Some(promoted.window);
                assign_orbits(&mut self.planets);
            } else {
                self.sun = None;
            }
        } else {
            self.planets.retain(|p| &p.window != window);
            assign_orbits(&mut self.planets);
        }
    }

    /// Make a window the new sun (called on focus change or planet selection).
    pub fn set_sun(&mut self, window: Window) {
        // If the window is already the sun, nothing to do.
        if self.sun.as_ref() == Some(&window) {
            return;
        }

        // Demote current sun to a planet.
        if let Some(old_sun) = self.sun.take() {
            let planet = Planet::new(old_sun, 0.0, 0);
            self.planets.push(planet);
        }

        // Remove the window from planets.
        self.planets.retain(|p| p.window != window);

        // Promote.
        self.sun = Some(window);
        assign_orbits(&mut self.planets);

        // Fly camera back to origin (new sun).
        self.camera.set_zoom(ZoomLevel::Work, Some(Vec2::ZERO));
    }

    // -----------------------------------------------------------------------
    // Switcher activation
    // -----------------------------------------------------------------------

    /// Called when Super is pressed — show the orbital overlay.
    pub fn open(&mut self) {
        self.state = SwitcherState::Visible;
        self.camera.set_zoom(ZoomLevel::System, Some(Vec2::ZERO));
        self.hovered = if self.planets.is_empty() { None } else { Some(0) };
    }

    /// Called when Super is released without selecting a planet.
    pub fn close(&mut self) {
        self.state = SwitcherState::Hidden;
        self.camera.set_zoom(ZoomLevel::Work, Some(Vec2::ZERO));
        self.hovered = None;
    }

    /// Confirm selection of the hovered planet and close the switcher.
    pub fn confirm_selection(&mut self) {
        if let Some(idx) = self.hovered {
            if idx < self.planets.len() {
                let planet = self.planets.remove(idx);
                let new_sun = planet.window.clone();
                // Demote current sun
                if let Some(old_sun) = self.sun.take() {
                    self.planets.push(Planet::new(old_sun, 0.0, 0));
                }
                self.sun = Some(new_sun);
                assign_orbits(&mut self.planets);
            }
        }
        self.close();
    }

    /// Move highlight to the next planet (wraps around).
    pub fn highlight_next(&mut self) {
        if self.planets.is_empty() { return; }
        self.hovered = Some(match self.hovered {
            None => 0,
            Some(i) => (i + 1) % self.planets.len(),
        });
        self.update_hovered_scale();
    }

    /// Move highlight to the previous planet (wraps around).
    pub fn highlight_prev(&mut self) {
        if self.planets.is_empty() { return; }
        self.hovered = Some(match self.hovered {
            None => self.planets.len() - 1,
            Some(0) => self.planets.len() - 1,
            Some(i) => i - 1,
        });
        self.update_hovered_scale();
    }

    /// Try to select a planet by screen-space click position.
    pub fn pick(&mut self, screen_pos: Vec2) -> bool {
        let world_pos = self.camera.screen_to_world(screen_pos);
        for (i, planet) in self.planets.iter().enumerate() {
            let d = (world_pos - planet.world_pos()).length();
            if d <= planet.visual_diameter() * 0.5 {
                self.hovered = Some(i);
                self.update_hovered_scale();
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Per-frame tick
    // -----------------------------------------------------------------------

    pub fn tick(&mut self) {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;

        self.camera.tick(dt);
        for planet in &mut self.planets {
            planet.tick(dt);
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn update_hovered_scale(&mut self) {
        for (i, planet) in self.planets.iter_mut().enumerate() {
            planet.scale_target = if Some(i) == self.hovered { 1.25 } else { 1.0 };
        }
    }
}
