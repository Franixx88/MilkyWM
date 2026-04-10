use glam::Vec2;
use smithay::desktop::Window;

use super::body::{Planet, assign_orbits};
use super::layout::{LayoutMode, Rect, compute_tiles};

/// A workspace — one "solar system" positioned somewhere on the infinite canvas.
///
/// Each workspace has:
///  - a **sun**: the currently focused window (renders full-screen in Work mode)
///  - **planets**: all other windows in this workspace, arranged on orbits
///  - a **layout**: how windows are tiled when more than one is visible
///  - a **world_pos**: where this workspace sits on the 2D infinite canvas
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: usize,
    /// Position of this workspace's origin in world-space.
    pub world_pos: Vec2,
    /// The focused window for this workspace.
    pub sun: Option<Window>,
    /// Orbiting windows.
    pub planets: Vec<Planet>,
    /// Tiling layout used when in Work/System mode.
    pub layout: LayoutMode,
    /// Optional human-readable label (shown in Galaxy view).
    pub label: Option<String>,
}

impl Workspace {
    pub fn new(id: usize, world_pos: Vec2) -> Self {
        Self {
            id,
            world_pos,
            sun: None,
            planets: Vec::new(),
            layout: LayoutMode::Monocle,
            label: None,
        }
    }

    /// Total number of windows in this workspace.
    pub fn window_count(&self) -> usize {
        self.sun.is_some() as usize + self.planets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sun.is_none() && self.planets.is_empty()
    }

    /// Add a window. If no sun yet, it becomes the sun; otherwise it's a planet.
    pub fn add_window(&mut self, window: Window) {
        if self.sun.is_none() {
            self.sun = Some(window);
        } else {
            self.planets.push(Planet::new(window, 0.0, 0));
            assign_orbits(&mut self.planets);
        }
    }

    /// Remove a window. If the sun was removed, the first planet is promoted.
    /// Returns `true` if the window was found and removed.
    pub fn remove_window(&mut self, window: &Window) -> bool {
        if self.sun.as_ref() == Some(window) {
            if !self.planets.is_empty() {
                let promoted = self.planets.remove(0);
                self.sun = Some(promoted.window);
                assign_orbits(&mut self.planets);
            } else {
                self.sun = None;
            }
            true
        } else if let Some(idx) = self.planets.iter().position(|p| &p.window == window) {
            self.planets.remove(idx);
            assign_orbits(&mut self.planets);
            true
        } else {
            false
        }
    }

    /// Promote `window` to sun, demoting the previous sun to a planet.
    pub fn set_sun(&mut self, window: Window) {
        if self.sun.as_ref() == Some(&window) {
            return;
        }
        if let Some(old_sun) = self.sun.take() {
            self.planets.push(Planet::new(old_sun, 0.0, 0));
        }
        self.planets.retain(|p| p.window != window);
        self.sun = Some(window);
        assign_orbits(&mut self.planets);
    }

    /// Check whether this workspace contains the given window.
    pub fn contains(&self, window: &Window) -> bool {
        self.sun.as_ref() == Some(window) || self.planets.iter().any(|p| &p.window == window)
    }

    /// Compute tile rectangles for all windows in this workspace.
    ///
    /// The sun (if present) is always the first tile. Planets follow in order.
    /// Returns `(window, rect)` pairs ready to send to `space.map_element`.
    pub fn tile_rects(&self, screen: Rect) -> Vec<(Window, Rect)> {
        let mut windows: Vec<Window> = Vec::new();
        if let Some(sun) = &self.sun {
            windows.push(sun.clone());
        }
        for planet in &self.planets {
            windows.push(planet.window.clone());
        }

        let count = windows.len();
        let tiles = compute_tiles(count, screen, self.layout);

        windows.into_iter().zip(tiles).collect()
    }
}
