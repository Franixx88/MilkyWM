//! Winit backend — runs the compositor inside an existing desktop window.
//!
//! Rendering pipeline per frame (smithay 0.7)
//! ────────────────────────────────────────────
//!  1. Dispatch winit events via WinitEventLoop calloop source → input handlers.
//!  2. Timer fires → `backend.bind()` → (GlowRenderer, framebuffer).
//!  3. `damage_tracker.render_output` with `MilkyRenderElement`:
//!       • `StarfieldElement` (pushed last → drawn first → below windows)
//!       • Wayland window surfaces (pushed first → drawn last → on top)
//!  4. Force alpha=1 everywhere so Hyprland sees an opaque window.
//!  5. `GlesSpaceRenderer::draw_orbital_overlay` — rings + halos (if Visible).
//!  6. `GlesSpaceRenderer::draw_galaxy_view`     — workspaces (if Galaxy).
//!  7. `backend.submit(damage)` → swap buffers.
use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            glow::GlowRenderer,
            Renderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
    utils::Transform,
};
use glow::HasContext;
use tracing::{error, info, warn};

use smithay::xwayland::{XWayland, XWaylandEvent, X11Wm};

use crate::{
    backend::Backend,
    orbital::SwitcherState,
    render::{
        frame::{build_frame_elements, update_thumbnails_if_needed},
        gles::GlesSpaceRenderer,
    },
    state::MilkyState,
};

const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_millis(1000 / TARGET_FPS);

// ---------------------------------------------------------------------------
// Backend state
// ---------------------------------------------------------------------------

/// All winit-specific state. Owned by `Backend::Winit(_)` inside `MilkyState`.
pub struct Winit {
    pub backend: WinitGraphicsBackend<GlowRenderer>,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    pub space_gl: Option<GlesSpaceRenderer>,
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

pub fn init_winit(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    // smithay 0.7: init returns (WinitGraphicsBackend<R>, WinitEventLoop)
    let (backend, winit_evt) =
        winit::init::<GlowRenderer>().map_err(|e| anyhow::anyhow!("{e:?}"))?;

    let mode = Mode {
        size: backend.window_size(),
        refresh: TARGET_FPS as i32 * 1000,
    };
    let output = Output::new(
        "milkywm-winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MilkyWM".into(),
            model: "Winit".into(),
            serial_number: "".to_string(),
        },
    );
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    state.sync_screen_size();

    let damage_tracker = OutputDamageTracker::from_output(&output);

    info!("Winit backend initialised — {}x{}", mode.size.w, mode.size.h);

    state.backend = Some(Backend::Winit(Winit {
        backend,
        output,
        damage_tracker,
        space_gl: None,
    }));

    // ---- XWayland -------------------------------------------------------
    init_xwayland(event_loop, state)?;

    // ---- WinitEventLoop as calloop source --------------------------------
    event_loop
        .handle()
        .insert_source(winit_evt, move |ev, _meta, state: &mut MilkyState| {
            state.with_backend(|backend, state| {
                if let Backend::Winit(winit) = backend {
                    handle_winit_event(ev, state, winit);
                }
            });
        })
        .map_err(|e| anyhow::anyhow!("insert winit source: {e}"))?;

    // ---- Frame timer -----------------------------------------------------
    event_loop
        .handle()
        .insert_source(
            Timer::from_duration(FRAME_DURATION),
            move |_, _, state: &mut MilkyState| {
                state.with_backend(|backend, state| {
                    if let Backend::Winit(winit) = backend {
                        if let Err(e) = render_frame(winit, state) {
                            warn!("Render error: {e:?}");
                        }
                    }
                });
                TimeoutAction::ToDuration(FRAME_DURATION)
            },
        )
        .map_err(|e| anyhow::anyhow!("insert timer source: {}", e.error))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// XWayland initialisation (shared with the DRM backend)
// ---------------------------------------------------------------------------

pub fn init_xwayland_pub(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    init_xwayland(event_loop, state)
}

fn init_xwayland(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    use std::process::Stdio;

    let (xwayland, client) = XWayland::spawn(
        &state.display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    )
    .map_err(|e| anyhow::anyhow!("XWayland spawn failed: {e:?}"))?;

    let dh = state.display_handle.clone();
    let handle = event_loop.handle();

    event_loop
        .handle()
        .insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                info!("XWayland ready on DISPLAY=:{display_number}");
                std::env::set_var("DISPLAY", format!(":{display_number}"));

                match X11Wm::start_wm(handle.clone(), &dh, x11_socket, client.clone()) {
                    Ok(wm) => {
                        state.xwm = Some(wm);
                        info!("X11 window manager attached");
                    }
                    Err(e) => {
                        tracing::error!("Failed to start X11 WM: {e:?}");
                    }
                }
            }
            XWaylandEvent::Error => {
                tracing::error!("XWayland exited with error");
                state.xwm = None;
            }
        })
        .map_err(|e| anyhow::anyhow!("insert XWayland source: {}", e.error))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn handle_winit_event(
    event: WinitEvent,
    state: &mut MilkyState,
    winit: &mut Winit,
) {
    use smithay::backend::input::{Event, InputEvent, KeyboardKeyEvent};

    match event {
        WinitEvent::CloseRequested => {
            info!("Winit window closed — stopping");
            state.loop_signal.stop();
        }

        WinitEvent::Resized { size, scale_factor } => {
            winit.output.change_current_state(
                Some(Mode {
                    size,
                    refresh: TARGET_FPS as i32 * 1000,
                }),
                None,
                Some(Scale::Fractional(scale_factor)),
                None,
            );
            state.sync_screen_size();
            state.re_tile();
        }

        WinitEvent::Input(InputEvent::Keyboard { event }) => {
            use smithay::backend::input::KeyState;

            let key_code = event.key_code();
            let key_state = event.state();
            let time = event.time_msec();
            let pressed = key_state == KeyState::Pressed;
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();

            if let Some(kb) = state.seat.get_keyboard() {
                kb.input::<(), _>(state, key_code, key_state, serial, time, |milky, mods, handle| {
                    crate::input::handle_shortcut(milky, handle.modified_sym().raw(), pressed, mods)
                });
            }
        }

        WinitEvent::Input(InputEvent::PointerButton { event }) => {
            use smithay::backend::input::PointerButtonEvent;
            crate::input::dispatch_pointer_button(
                state,
                event.button_code(),
                event.state(),
                event.time_msec(),
            );
        }

        WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
            use smithay::backend::input::AbsolutePositionEvent;

            let phys = winit.backend.window_size();
            let logical = smithay::utils::Size::<i32, smithay::utils::Logical>::from(
                (phys.w, phys.h),
            );
            let pos = event.position_transformed(logical);
            crate::input::dispatch_cursor_motion(state, pos, event.time_msec());
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_frame(
    winit: &mut Winit,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    state.orbital.tick();
    state.renderer.starfield.tick(1.0 / TARGET_FPS as f32);

    // Lazy-init custom GL renderer on first frame.
    if winit.space_gl.is_none() {
        match GlesSpaceRenderer::init(winit.backend.renderer(), &state.renderer.starfield) {
            Ok(r) => {
                info!("GlesSpaceRenderer ready");
                winit.space_gl = Some(r);
            }
            Err(e) => {
                error!("GlesSpaceRenderer init failed: {e:?}");
                state.loop_signal.stop();
                return Err(anyhow::anyhow!("GlesSpaceRenderer init: {e:?}"));
            }
        }
    }
    let space_gl = winit.space_gl.as_mut().unwrap();

    // Query these before bind() to avoid borrow conflicts.
    let buffer_age = winit.backend.buffer_age().unwrap_or(0);
    let size = winit.backend.window_size();
    let switcher_state = state.orbital.state;

    // Update planet thumbnails before binding the main framebuffer.
    {
        let renderer = winit.backend.renderer();
        update_thumbnails_if_needed(state, renderer, space_gl);
    }

    let render_result = {
        let (renderer, mut framebuffer) = winit.backend
            .bind()
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let elements = build_frame_elements(state, renderer, &winit.output, space_gl, size);

        // Pass 1 — damage-tracked surfaces + starfield.
        let result = winit.damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            buffer_age,
            &elements,
            [0.0_f32, 0.0, 0.03, 1.0],
        );

        // Pass 2 — force alpha=1 so nested compositors see opaque window.
        {
            let mut frame = renderer
                .render(&mut framebuffer, size, Transform::Normal)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            frame
                .with_context(|gl: &std::sync::Arc<glow::Context>| unsafe {
                    gl.color_mask(false, false, false, true);
                    gl.clear_color(0.0, 0.0, 0.0, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                    gl.color_mask(true, true, true, true);
                })
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }

        // Pass 3 — orbital / galaxy overlay.
        if switcher_state != SwitcherState::Hidden {
            let mut frame = renderer
                .render(&mut framebuffer, size, Transform::Normal)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            match switcher_state {
                SwitcherState::Visible => {
                    space_gl.draw_orbital_overlay(&mut frame, size, &state.orbital)?;
                }
                SwitcherState::Galaxy => {
                    space_gl.draw_galaxy_view(&mut frame, size, &state.orbital)?;
                }
                SwitcherState::Hidden => {}
            }
        }

        result
    };

    // Submit frame.
    match render_result {
        Ok(res) => {
            let damage = res.damage.map(|d| d.as_slice().to_vec());
            winit.backend
                .submit(damage.as_deref())
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        Err(e) => {
            winit.backend.submit(None).ok();
            return Err(anyhow::anyhow!("render_output: {e:?}"));
        }
    }

    // Notify clients that the frame was presented.
    let now = Duration::from_millis(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    );
    let output = winit.output.clone();
    for window in state.space.elements().cloned().collect::<Vec<_>>() {
        window.send_frame(&output, now, Some(FRAME_DURATION), |_, _| {
            Some(output.clone())
        });
    }

    Ok(())
}
