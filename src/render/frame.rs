//! Shared per-frame element builder used by both backends.
//!
//! Produces the `Vec<MilkyRenderElement>` passed to `render_output` /
//! `render_frame`, plus a helper for keeping planet thumbnails up to date.
use smithay::{
    backend::renderer::{
        element::{solid::SolidColorRenderElement, Id, Kind},
        glow::GlowRenderer,
        utils::CommitCounter,
    },
    desktop::Window,
    output::Output,
    utils::{Physical, Rectangle, Size},
    wayland::seat::WaylandFocus,
};

use crate::{
    orbital::SwitcherState,
    render::{build_cursor_elements, gles::GlesSpaceRenderer, palette, MilkyRenderElement},
    state::MilkyState,
};

/// Update planet thumbnails for the active workspace when the switcher is open.
///
/// Must be called *before* the main framebuffer is bound, so the thumbnail
/// renderer can freely bind and unbind its own FBOs.
pub fn update_thumbnails_if_needed(
    state: &MilkyState,
    renderer: &mut GlowRenderer,
    space_gl: &mut GlesSpaceRenderer,
) {
    let switcher = state.orbital.state;
    if switcher != SwitcherState::Visible && switcher != SwitcherState::Galaxy {
        return;
    }
    let planets: Vec<Window> = state
        .orbital
        .active_ws()
        .planets
        .iter()
        .map(|p| p.window.clone())
        .collect();
    for window in &planets {
        space_gl.thumbnails.update(renderer, window);
    }
    let refs: Vec<&Window> = planets.iter().collect();
    space_gl.thumbnails.retain(&refs);
}

/// Build the per-frame element list in z-order.
///
/// `render_output` iterates the slice in reverse, so the resulting order on
/// screen is (top → bottom): cursor, window borders, window surfaces, starfield.
pub fn build_frame_elements(
    state: &mut MilkyState,
    renderer: &mut GlowRenderer,
    output: &Output,
    space_gl: &mut GlesSpaceRenderer,
    phys_size: Size<i32, Physical>,
) -> Vec<MilkyRenderElement> {
    let mut elements: Vec<MilkyRenderElement> = Vec::new();

    // ── 0. Cursor (topmost) ────────────────────────────────────────────────
    let cx = state.cursor_pos.x as i32;
    let cy = state.cursor_pos.y as i32;
    elements.extend(build_cursor_elements(cx, cy, &state.cursor_status));

    // ── 1. Window borders (drawn on top of window surfaces) ────────────────
    let focused_surface = state.seat.get_keyboard().and_then(|kb| kb.current_focus());
    let scale = output.current_scale().fractional_scale();
    let bw = palette::WIN_BORDER_WIDTH;

    for window in state.space.elements().cloned().collect::<Vec<_>>() {
        let Some(geo) = state.space.element_geometry(&window) else { continue };
        let is_focused = focused_surface
            .as_ref()
            .map_or(false, |fs| window.wl_surface().as_deref() == Some(fs));
        let color: [f32; 4] = if is_focused {
            palette::WIN_BORDER_FOCUSED
        } else {
            palette::WIN_BORDER_UNFOCUSED
        };

        let phys = geo.to_physical_precise_round(scale);
        let x: i32 = phys.loc.x;
        let y: i32 = phys.loc.y;
        let w: i32 = phys.size.w.max(2 * bw);
        let h: i32 = phys.size.h.max(2 * bw);

        let commit = CommitCounter::default();
        for rect in [
            Rectangle::new((x,        y       ).into(), (w,  bw     ).into()), // top
            Rectangle::new((x,        y+h-bw  ).into(), (w,  bw     ).into()), // bottom
            Rectangle::new((x,        y+bw    ).into(), (bw, h-2*bw ).into()), // left
            Rectangle::new((x+w-bw,   y+bw    ).into(), (bw, h-2*bw ).into()), // right
        ] {
            elements.push(MilkyRenderElement::Border(
                SolidColorRenderElement::new(Id::new(), rect, commit, color, Kind::Unspecified),
            ));
        }
    }

    // ── 2. Window surfaces ─────────────────────────────────────────────────
    let space_elements = state
        .space
        .render_elements_for_output::<GlowRenderer>(renderer, output, 1.0_f32)
        .unwrap_or_default();
    elements.extend(space_elements.into_iter().map(MilkyRenderElement::Space));

    // ── 3. Starfield (bottom layer — last in vec = drawn first) ────────────
    let cam = state.orbital.camera.position;
    elements.push(MilkyRenderElement::Starfield(
        space_gl.make_starfield_element(
            &state.renderer.starfield,
            cam.x,
            cam.y,
            phys_size.w,
            phys_size.h,
        ),
    ));

    elements
}
