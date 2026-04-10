pub mod body;
pub mod camera;
pub mod layout;
pub mod workspace;

use glam::Vec2;
use smithay::desktop::Window;

pub use body::{Planet, assign_orbits};
pub use camera::{Camera, ZoomLevel};
pub use layout::{LayoutMode, Rect};
pub use workspace::Workspace;

use crate::config::Config;

/// Horizontal spacing between workspaces on the infinite canvas.
pub const WORKSPACE_SPACING: f32 = 2800.0;

// ---------------------------------------------------------------------------

/// State of the orbital switcher / camera view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitcherState {
    /// Work mode — camera zoomed into the active workspace's sun.
    Hidden,
    /// Super held — orbital system visible, user is selecting a planet within
    /// the active workspace.
    Visible,
    /// Galaxy view — camera pulled far back; shows all workspaces as planet
    /// icons; user can fly to another workspace.
    Galaxy,
}

// ---------------------------------------------------------------------------

/// The orbital switcher — core unique feature of MilkyWM.
///
/// Conceptual model
/// ────────────────
///  - The infinite 2D canvas holds multiple **workspaces** (solar systems).
///  - Each workspace has one **sun** (focused window) + **planets** (others).
///  - A **camera** with three zoom levels navigates the canvas:
///      Work   → zoomed into active workspace's sun.
///      System → pulled back to see the full solar system of the active ws.
///      Galaxy → very far back; all workspaces visible as planet icons.
///  - Keyboard shortcuts let the user switch layouts, create / switch
///    workspaces, and navigate planets within a workspace.
pub struct OrbitalSwitcher {
    /// All workspaces (solar systems) on the infinite canvas.
    pub workspaces: Vec<Workspace>,

    /// Index of the currently active workspace.
    pub active: usize,

    /// Camera controlling the current view.
    pub camera: Camera,

    /// Current overlay / view state.
    pub state: SwitcherState,

    /// In Visible state: index of highlighted planet in the active workspace.
    pub hovered_planet: Option<usize>,

    /// In Galaxy state: index of highlighted workspace.
    pub hovered_ws: Option<usize>,

    last_tick: std::time::Instant,
}

impl OrbitalSwitcher {
    pub fn new(config: &Config) -> Self {
        // Start with a single workspace at the world origin.
        let workspaces = vec![Workspace::new(0, Vec2::ZERO)];
        Self {
            workspaces,
            active: 0,
            camera: Camera::new(config.default_width, config.default_height),
            state: SwitcherState::Hidden,
            hovered_planet: None,
            hovered_ws: None,
            last_tick: std::time::Instant::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Convenience accessors
    // -----------------------------------------------------------------------

    pub fn active_ws(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub fn active_ws_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    /// The sun of the active workspace.
    pub fn sun(&self) -> Option<&Window> {
        self.workspaces[self.active].sun.as_ref()
    }

    /// Planets of the active workspace.
    pub fn planets(&self) -> &[Planet] {
        &self.workspaces[self.active].planets
    }

    // -----------------------------------------------------------------------
    // Window management
    // -----------------------------------------------------------------------

    /// Add a new window to the active workspace.
    pub fn add_window(&mut self, window: Window) {
        self.workspaces[self.active].add_window(window);
    }

    /// Remove a window from whatever workspace it belongs to.
    pub fn remove_window(&mut self, window: &Window) {
        for ws in &mut self.workspaces {
            if ws.remove_window(window) {
                return;
            }
        }
    }

    /// Find which workspace contains `window`.
    pub fn workspace_of(&self, window: &Window) -> Option<usize> {
        self.workspaces.iter().position(|ws| ws.contains(window))
    }

    /// Promote `window` to the sun of its workspace.
    /// If the window is in a different workspace, that workspace becomes active
    /// and the camera flies to it.
    pub fn set_sun(&mut self, window: Window) {
        // Find the workspace that owns this window.
        let ws_idx = self.workspaces.iter().position(|ws| ws.contains(&window));

        if let Some(idx) = ws_idx {
            if idx != self.active {
                self.active = idx;
                let pos = self.workspaces[idx].world_pos;
                self.camera.fly_to(pos, ZoomLevel::Work.scale());
            }
            self.workspaces[idx].set_sun(window);
        } else {
            // Window not in any workspace — add to active and make it sun.
            self.workspaces[self.active].set_sun(window);
        }

        // Fly camera to active workspace origin in Work mode.
        let pos = self.workspaces[self.active].world_pos;
        self.camera.set_zoom(ZoomLevel::Work, Some(pos));
    }

    // -----------------------------------------------------------------------
    // Workspace management
    // -----------------------------------------------------------------------

    /// Create a new empty workspace to the right of the last one.
    /// Returns the index of the new workspace.
    pub fn new_workspace(&mut self) -> usize {
        let last_pos = self.workspaces.last().map(|ws| ws.world_pos).unwrap_or(Vec2::ZERO);
        let id = self.workspaces.len();
        let pos = Vec2::new(last_pos.x + WORKSPACE_SPACING, 0.0);
        self.workspaces.push(Workspace::new(id, pos));
        id
    }

    /// Switch to the workspace at `idx`, animating the camera.
    pub fn switch_workspace(&mut self, idx: usize) {
        if idx >= self.workspaces.len() {
            return;
        }
        self.active = idx;
        let pos = self.workspaces[idx].world_pos;
        self.camera.fly_to(pos, ZoomLevel::Work.scale());
        self.state = SwitcherState::Hidden;
        self.hovered_ws = None;
        self.hovered_planet = None;
    }

    pub fn next_workspace(&mut self) {
        let next = (self.active + 1) % self.workspaces.len();
        self.switch_workspace(next);
    }

    pub fn prev_workspace(&mut self) {
        let prev = if self.active == 0 {
            self.workspaces.len() - 1
        } else {
            self.active - 1
        };
        self.switch_workspace(prev);
    }

    // -----------------------------------------------------------------------
    // Layout control
    // -----------------------------------------------------------------------

    pub fn set_layout(&mut self, mode: LayoutMode) {
        self.workspaces[self.active].layout = mode;
    }

    // -----------------------------------------------------------------------
    // Orbital switcher (System view) — planet navigation within active ws
    // -----------------------------------------------------------------------

    /// Called when Super is pressed — show the orbital overlay (System view).
    pub fn open(&mut self) {
        self.state = SwitcherState::Visible;
        let pos = self.workspaces[self.active].world_pos;
        self.camera.set_zoom(ZoomLevel::System, Some(pos));
        let has_planets = !self.workspaces[self.active].planets.is_empty();
        self.hovered_planet = if has_planets { Some(0) } else { None };
        self.update_hovered_scale();
    }

    /// Called when Super is released without selecting a planet.
    pub fn close(&mut self) {
        self.state = SwitcherState::Hidden;
        let pos = self.workspaces[self.active].world_pos;
        self.camera.set_zoom(ZoomLevel::Work, Some(pos));
        self.hovered_planet = None;
        self.update_hovered_scale();
    }

    /// Confirm selection of the hovered planet and close the switcher.
    pub fn confirm_selection(&mut self) {
        if let Some(idx) = self.hovered_planet {
            let ws = &mut self.workspaces[self.active];
            if idx < ws.planets.len() {
                let planet = ws.planets.remove(idx);
                let new_sun = planet.window.clone();
                if let Some(old_sun) = ws.sun.take() {
                    ws.planets.push(Planet::new(old_sun, 0.0, 0));
                }
                ws.sun = Some(new_sun);
                assign_orbits(&mut ws.planets);
            }
        }
        self.close();
    }

    pub fn highlight_next(&mut self) {
        let n = self.workspaces[self.active].planets.len();
        if n == 0 { return; }
        self.hovered_planet = Some(match self.hovered_planet {
            None    => 0,
            Some(i) => (i + 1) % n,
        });
        self.update_hovered_scale();
    }

    pub fn highlight_prev(&mut self) {
        let n = self.workspaces[self.active].planets.len();
        if n == 0 { return; }
        self.hovered_planet = Some(match self.hovered_planet {
            None    => n - 1,
            Some(0) => n - 1,
            Some(i) => i - 1,
        });
        self.update_hovered_scale();
    }

    /// Try to select a planet by screen-space click position.
    pub fn pick(&mut self, screen_pos: Vec2) -> bool {
        let world_pos = self.camera.screen_to_world(screen_pos);
        let ws = &self.workspaces[self.active];
        for (i, planet) in ws.planets.iter().enumerate() {
            let d = (world_pos - planet.world_pos()).length();
            if d <= planet.visual_diameter() * 0.5 {
                self.hovered_planet = Some(i);
                self.update_hovered_scale();
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Galaxy view — workspace navigation
    // -----------------------------------------------------------------------

    /// Enter Galaxy view: camera pulls far back to show all workspaces.
    pub fn enter_galaxy(&mut self) {
        self.state = SwitcherState::Galaxy;
        // Centre on the midpoint of all workspaces.
        let centroid = self.workspace_centroid();
        // Zoom to fit: ensure all workspaces are visible.
        let fit_zoom = self.galaxy_fit_zoom();
        self.camera.fly_to(centroid, fit_zoom);
        self.hovered_ws = Some(self.active);
    }

    /// Exit Galaxy view, flying back to the (possibly changed) active workspace.
    pub fn exit_galaxy(&mut self) {
        self.state = SwitcherState::Hidden;
        let pos = self.workspaces[self.active].world_pos;
        self.camera.fly_to(pos, ZoomLevel::Work.scale());
        self.hovered_ws = None;
    }

    /// In Galaxy view: highlight the next workspace.
    pub fn highlight_next_ws(&mut self) {
        let n = self.workspaces.len();
        if n == 0 { return; }
        self.hovered_ws = Some(match self.hovered_ws {
            None    => 0,
            Some(i) => (i + 1) % n,
        });
    }

    /// In Galaxy view: highlight the previous workspace.
    pub fn highlight_prev_ws(&mut self) {
        let n = self.workspaces.len();
        if n == 0 { return; }
        self.hovered_ws = Some(match self.hovered_ws {
            None    => n - 1,
            Some(0) => n - 1,
            Some(i) => i - 1,
        });
    }

    /// In Galaxy view: confirm selection of hovered workspace and fly to it.
    pub fn confirm_ws_selection(&mut self) {
        if let Some(idx) = self.hovered_ws {
            self.switch_workspace(idx);
        } else {
            self.exit_galaxy();
        }
    }

    // -----------------------------------------------------------------------
    // Per-frame tick
    // -----------------------------------------------------------------------

    pub fn tick(&mut self) {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;

        self.camera.tick(dt);
        for ws in &mut self.workspaces {
            for planet in &mut ws.planets {
                planet.tick(dt);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn update_hovered_scale(&mut self) {
        let hovered = self.hovered_planet;
        let ws = &mut self.workspaces[self.active];
        for (i, planet) in ws.planets.iter_mut().enumerate() {
            planet.scale_target = if Some(i) == hovered { 1.25 } else { 1.0 };
        }
    }

    fn workspace_centroid(&self) -> Vec2 {
        if self.workspaces.is_empty() {
            return Vec2::ZERO;
        }
        let sum: Vec2 = self.workspaces.iter().map(|ws| ws.world_pos).sum();
        sum / self.workspaces.len() as f32
    }

    /// Compute a zoom level that fits all workspaces on screen.
    fn galaxy_fit_zoom(&self) -> f32 {
        if self.workspaces.len() <= 1 {
            return ZoomLevel::Galaxy.scale();
        }
        let n = self.workspaces.len() as f32;
        // Each workspace is `WORKSPACE_SPACING` apart; we want all to fit in
        // ~80 % of the screen width.
        let total_span = WORKSPACE_SPACING * (n - 1.0);
        let screen_w = self.camera.screen_size.x;
        let zoom = (screen_w * 0.8) / total_span.max(1.0);
        zoom.clamp(ZoomLevel::Galaxy.scale(), ZoomLevel::System.scale())
    }
}
