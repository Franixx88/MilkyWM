//! Thumbnail cache — renders each window's content into a small offscreen texture.
//!
//! # How it works
//!
//!  1. `ThumbnailCache::update()` is called once per frame for each planet window.
//!  2. It binds an offscreen `GlesTexture` as an FBO, renders the window's surface
//!     tree into it via `OutputDamageTracker::render_output`, then unbinds.
//!  3. The resulting `GlesTexture` is available via `get()` for use in the
//!     orbital overlay shader.

use smithay::{
    backend::renderer::{
        Offscreen, Bind,
        damage::OutputDamageTracker,
        element::{AsRenderElements, surface::WaylandSurfaceRenderElement},
        gles::GlesTexture,
        glow::GlowRenderer,
    },
    desktop::Window,
    reexports::drm::buffer::DrmFourcc,
    utils::{Buffer as BufferCoord, Physical, Scale, Size, Transform},
};
use tracing::warn;

/// Pixel dimensions of each thumbnail texture.
pub const THUMB_W: u32 = 256;
pub const THUMB_H: u32 = 160;

// ---------------------------------------------------------------------------

/// Cached thumbnail for one window.
struct Entry {
    window: Window,
    texture: GlesTexture,
    damage_tracker: OutputDamageTracker,
}

/// Per-frame offscreen thumbnail renderer.
///
/// Kept inside `GlesSpaceRenderer` so lifetime matches the GL context.
pub struct ThumbnailCache {
    entries: Vec<Entry>,
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Update (or create) the thumbnail for `window`.
    ///
    /// Must be called with the GlowRenderer **before** the main framebuffer is
    /// bound, so we can freely bind/unbind our own FBOs.
    pub fn update(&mut self, renderer: &mut GlowRenderer, window: &Window) {
        let thumb_size = Size::<i32, BufferCoord>::from((THUMB_W as i32, THUMB_H as i32));

        // Find or create entry.
        let idx = self.entries.iter().position(|e| &e.window == window);
        let entry = if let Some(i) = idx {
            &mut self.entries[i]
        } else {
            // Create a new offscreen texture for this window.
            let texture = match renderer.create_buffer(DrmFourcc::Argb8888, thumb_size) {
                Ok(t) => t,
                Err(e) => {
                    warn!("Failed to create thumbnail texture: {e:?}");
                    return;
                }
            };
            let phys_size = Size::<i32, Physical>::from((THUMB_W as i32, THUMB_H as i32));
            let damage_tracker = OutputDamageTracker::new(
                phys_size,
                Scale::from(1.0_f64),
                Transform::Normal,
            );
            self.entries.push(Entry { window: window.clone(), texture, damage_tracker });
            self.entries.last_mut().unwrap()
        };

        // Render window contents into the thumbnail texture.
        // We scale window content to fit the thumbnail dimensions.
        let win_geo = window.geometry();
        let scale_x = THUMB_W as f64 / win_geo.size.w.max(1) as f64;
        let scale_y = THUMB_H as f64 / win_geo.size.h.max(1) as f64;
        let scale = Scale::from(scale_x.min(scale_y));

        // Get render elements for this window.
        let elements: Vec<WaylandSurfaceRenderElement<GlowRenderer>> =
            window.render_elements(renderer, (0, 0).into(), scale, 1.0_f32);

        // Bind the thumbnail texture as the render target.
        let mut framebuffer = match renderer.bind(&mut entry.texture) {
            Ok(fb) => fb,
            Err(e) => {
                warn!("Failed to bind thumbnail FBO: {e:?}");
                return;
            }
        };

        // Render into FBO (age=0 → always full repaint).
        if let Err(e) = entry.damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            0,
            &elements,
            [0.05_f32, 0.05, 0.1, 1.0],
        ) {
            warn!("Thumbnail render_output failed: {e:?}");
        }
        // framebuffer dropped here — unbinds the thumbnail FBO.
    }

    /// Get the texture for `window`, if it has been rendered at least once.
    pub fn get(&self, window: &Window) -> Option<&GlesTexture> {
        self.entries
            .iter()
            .find(|e| &e.window == window)
            .map(|e| &e.texture)
    }

    /// Remove stale entries for windows that are no longer in the planet list.
    pub fn retain(&mut self, windows: &[&Window]) {
        self.entries.retain(|e| windows.contains(&&e.window));
    }
}
