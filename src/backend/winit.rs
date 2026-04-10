/// Winit backend — runs the compositor inside an existing desktop window.
///
/// Rendering pipeline per frame (smithay 0.7)
/// ────────────────────────────────────────────
///  1. Dispatch winit events via WinitEventLoop calloop source → input handlers.
///  2. Timer fires → `backend.bind()` → (GlowRenderer, framebuffer).
///  3. `GlesSpaceRenderer::draw_starfield`  — clear + star point-sprites.
///  4. `damage_tracker.render_output`       — Wayland window surfaces.
///  5. `GlesSpaceRenderer::draw_orbital_overlay` — rings + halos (if Visible).
///  6. `GlesSpaceRenderer::draw_galaxy_view`     — workspaces (if Galaxy).
///  7. `backend.submit(damage)` → swap buffers.
use std::{cell::RefCell, rc::Rc, time::Duration};

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            glow::GlowRenderer,
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
use tracing::{error, info, warn};

use crate::{
    orbital::SwitcherState,
    render::gles::GlesSpaceRenderer,
    state::MilkyState,
};

const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_millis(1000 / TARGET_FPS);

// ---------------------------------------------------------------------------

pub fn init_winit(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    // smithay 0.7: init returns (WinitGraphicsBackend<R>, WinitEventLoop)
    let (backend, winit_evt) =
        winit::init::<GlowRenderer>().map_err(|e| anyhow::anyhow!("{e:?}"))?;

    // Wrap backend in Rc<RefCell> so both the winit source and the timer
    // closure can share it without moving it into either one.
    let backend = Rc::new(RefCell::new(backend));

    let mode = Mode {
        size: backend.borrow().window_size(),
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

    {
        let sz = backend.borrow().window_size();
        state.orbital.camera.screen_size = glam::Vec2::new(sz.w as f32, sz.h as f32);
    }

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut space_gl: Option<GlesSpaceRenderer> = None;

    info!("Winit backend initialised — {}x{}", mode.size.w, mode.size.h);

    // ---- WinitEventLoop as calloop source (handles PumpStatus internally) --
    let backend_evt = Rc::clone(&backend);
    let output_evt = output.clone();
    event_loop
        .handle()
        .insert_source(winit_evt, move |ev, _meta, state| {
            handle_winit_event(ev, state, &output_evt, &backend_evt.borrow());
        })
        .map_err(|e| anyhow::anyhow!("insert winit source: {e}"))?;

    // ---- Frame timer -------------------------------------------------------
    let backend_timer = Rc::clone(&backend);
    event_loop
        .handle()
        .insert_source(
            Timer::from_duration(FRAME_DURATION),
            move |_, _, state| {
                // Lazy-init custom GL renderer on first frame.
                if space_gl.is_none() {
                    let mut b = backend_timer.borrow_mut();
                    match GlesSpaceRenderer::init(b.renderer(), &state.renderer.starfield) {
                        Ok(r) => {
                            info!("GlesSpaceRenderer ready");
                            space_gl = Some(r);
                        }
                        Err(e) => {
                            error!("GlesSpaceRenderer init failed: {e:?}");
                            state.loop_signal.stop();
                            return TimeoutAction::Drop;
                        }
                    }
                }

                if let Err(e) = render_frame(
                    &mut backend_timer.borrow_mut(),
                    state,
                    &output,
                    &mut damage_tracker,
                    space_gl.as_ref().unwrap(),
                ) {
                    warn!("Render error: {e:?}");
                }

                TimeoutAction::ToDuration(FRAME_DURATION)
            },
        )
        .map_err(|e| anyhow::anyhow!("insert timer source: {}", e.error))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn handle_winit_event(
    event: WinitEvent,
    state: &mut MilkyState,
    output: &Output,
    backend: &WinitGraphicsBackend<GlowRenderer>,
) {
    use smithay::backend::input::{Event, InputEvent, KeyboardKeyEvent};

    match event {
        WinitEvent::CloseRequested => {
            info!("Winit window closed — stopping");
            state.loop_signal.stop();
        }

        WinitEvent::Resized { size, scale_factor } => {
            output.change_current_state(
                Some(Mode {
                    size,
                    refresh: TARGET_FPS as i32 * 1000,
                }),
                None,
                Some(Scale::Fractional(scale_factor)),
                None,
            );
            state.orbital.camera.screen_size =
                glam::Vec2::new(size.w as f32, size.h as f32);

            // Re-tile with new screen dimensions.
            let screen = state.screen_rect();
            let ws = state.orbital.active_ws().clone();
            crate::compositor::apply_layout(&mut state.space, &ws, screen);
        }

        WinitEvent::Input(InputEvent::Keyboard { event }) => {
            use smithay::input::keyboard::{keysyms, FilterResult};

            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            if let Some(kb) = state.seat.get_keyboard() {
                let time = event.time_msec();
                kb.input::<(), _>(
                    state,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |milky, _mods, handle| {
                        use smithay::backend::input::KeyState;
                        let pressed = event.state() == KeyState::Pressed;

                        match handle.modified_sym().raw() {
                            // ---- Super: toggle orbital switcher (System view) ----
                            keysyms::KEY_Super_L | keysyms::KEY_Super_R => {
                                if pressed {
                                    if milky.orbital.state == SwitcherState::Hidden {
                                        milky.orbital.open();
                                    }
                                } else {
                                    if milky.orbital.state == SwitcherState::Visible {
                                        milky.orbital.close();
                                    }
                                }
                            }

                            // ---- Tab: navigate planets / workspaces ----
                            keysyms::KEY_Tab if pressed => {
                                match milky.orbital.state {
                                    SwitcherState::Visible => milky.orbital.highlight_next(),
                                    SwitcherState::Galaxy  => milky.orbital.highlight_next_ws(),
                                    SwitcherState::Hidden  => {}
                                }
                            }

                            // ---- Return: confirm selection ----
                            keysyms::KEY_Return if pressed => {
                                match milky.orbital.state {
                                    SwitcherState::Visible => milky.orbital.confirm_selection(),
                                    SwitcherState::Galaxy  => {
                                        milky.orbital.confirm_ws_selection();
                                        // Re-tile after workspace switch.
                                        let screen = milky.screen_rect();
                                        let ws = milky.orbital.active_ws().clone();
                                        crate::compositor::apply_layout(&mut milky.space, &ws, screen);
                                    }
                                    SwitcherState::Hidden  => {}
                                }
                            }

                            // ---- G: toggle Galaxy view (while Super held or free) ----
                            keysyms::KEY_g | keysyms::KEY_G if pressed => {
                                match milky.orbital.state {
                                    SwitcherState::Galaxy => milky.orbital.exit_galaxy(),
                                    _ => milky.orbital.enter_galaxy(),
                                }
                            }

                            // ---- N: new workspace ----
                            keysyms::KEY_n | keysyms::KEY_N if pressed => {
                                milky.orbital.new_workspace();
                            }

                            // ---- ] or Right: next workspace ----
                            keysyms::KEY_bracketright | keysyms::KEY_Right if pressed => {
                                milky.orbital.next_workspace();
                                let screen = milky.screen_rect();
                                let ws = milky.orbital.active_ws().clone();
                                crate::compositor::apply_layout(&mut milky.space, &ws, screen);
                            }

                            // ---- [ or Left: prev workspace ----
                            keysyms::KEY_bracketleft | keysyms::KEY_Left if pressed => {
                                milky.orbital.prev_workspace();
                                let screen = milky.screen_rect();
                                let ws = milky.orbital.active_ws().clone();
                                crate::compositor::apply_layout(&mut milky.space, &ws, screen);
                            }

                            // ---- Layout shortcuts ----
                            keysyms::KEY_h | keysyms::KEY_H if pressed => {
                                milky.orbital.set_layout(crate::orbital::LayoutMode::HorizSplit);
                                let screen = milky.screen_rect();
                                let ws = milky.orbital.active_ws().clone();
                                crate::compositor::apply_layout(&mut milky.space, &ws, screen);
                            }
                            keysyms::KEY_v | keysyms::KEY_V if pressed => {
                                milky.orbital.set_layout(crate::orbital::LayoutMode::VertSplit);
                                let screen = milky.screen_rect();
                                let ws = milky.orbital.active_ws().clone();
                                crate::compositor::apply_layout(&mut milky.space, &ws, screen);
                            }
                            keysyms::KEY_m | keysyms::KEY_M if pressed => {
                                milky.orbital.set_layout(crate::orbital::LayoutMode::Monocle);
                                let screen = milky.screen_rect();
                                let ws = milky.orbital.active_ws().clone();
                                crate::compositor::apply_layout(&mut milky.space, &ws, screen);
                            }

                            _ => {}
                        }
                        FilterResult::Forward
                    },
                );
            }
        }

        WinitEvent::Input(InputEvent::PointerButton { event }) => {
            use smithay::backend::input::{ButtonState, PointerButtonEvent};
            if event.state() == ButtonState::Pressed {
                match state.orbital.state {
                    SwitcherState::Visible => {
                        if let Some(pos) = state.seat.get_pointer().map(|p| p.current_location()) {
                            state.orbital.pick(glam::Vec2::new(pos.x as f32, pos.y as f32));
                            state.orbital.confirm_selection();
                        }
                    }
                    SwitcherState::Galaxy => {
                        // Click on a workspace planet in galaxy view — pick by proximity.
                        if let Some(pos) = state.seat.get_pointer().map(|p| p.current_location()) {
                            let screen_pos = glam::Vec2::new(pos.x as f32, pos.y as f32);
                            let world_pos = state.orbital.camera.screen_to_world(screen_pos);
                            let mut picked = None;
                            for (i, ws) in state.orbital.workspaces.iter().enumerate() {
                                if (world_pos - ws.world_pos).length() < 80.0 {
                                    picked = Some(i);
                                    break;
                                }
                            }
                            if let Some(idx) = picked {
                                state.orbital.switch_workspace(idx);
                                let screen = state.screen_rect();
                                let ws = state.orbital.active_ws().clone();
                                crate::compositor::apply_layout(&mut state.space, &ws, screen);
                            }
                        }
                    }
                    SwitcherState::Hidden => {}
                }
            }
        }

        WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
            use smithay::backend::input::AbsolutePositionEvent;
            // position_transformed expects logical size; for winit with scale 1
            // the logical size equals the physical window size.
            let phys = backend.window_size();
            let logical = smithay::utils::Size::<i32, smithay::utils::Logical>::from(
                (phys.w, phys.h),
            );
            let pos = event.position_transformed(logical);
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            if let Some(ptr) = state.seat.get_pointer() {
                ptr.motion(
                    state,
                    None,
                    &smithay::input::pointer::MotionEvent {
                        location: pos.into(),
                        serial,
                        time: event.time_msec(),
                    },
                );
            }
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_frame(
    backend: &mut WinitGraphicsBackend<GlowRenderer>,
    state: &mut MilkyState,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    space_gl: &GlesSpaceRenderer,
) -> anyhow::Result<()> {
    state.orbital.tick();
    state.renderer.starfield.tick(1.0 / TARGET_FPS as f32);

    // Query these before bind() to avoid borrow conflicts.
    let size = backend.window_size();
    let buffer_age = backend.buffer_age().unwrap_or(0) as usize;
    let cam_x = state.orbital.camera.position.x;
    let cam_y = state.orbital.camera.position.y;
    let switcher_state = state.orbital.state;

    // smithay 0.7: bind() returns (renderer, framebuffer).
    // All rendering happens inside this block so framebuffer is dropped
    // before we call submit() below.
    let render_result = {
        let (renderer, mut framebuffer) = backend
            .bind()
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        // 1. Starfield — clears framebuffer + draws point-sprite stars.
        space_gl.draw_starfield(renderer, size, &state.renderer.starfield, cam_x, cam_y)?;

        // 2. Wayland window surfaces (1 generic: the renderer R).
        let elements = state
            .space
            .render_elements_for_output::<GlowRenderer>(renderer, output, 1.0_f32)
            .unwrap_or_default();

        // 3. Damage-tracked render — transparent clear so the starfield shows through.
        let result = damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            buffer_age,
            &elements,
            [0.0_f32, 0.0, 0.0, 0.0],
        );

        // 4. Orbital overlay (System view) or Galaxy view on top.
        match switcher_state {
            SwitcherState::Visible => {
                space_gl.draw_orbital_overlay(renderer, size, &state.orbital)?;
            }
            SwitcherState::Galaxy => {
                space_gl.draw_galaxy_view(renderer, size, &state.orbital)?;
            }
            SwitcherState::Hidden => {}
        }

        result
    }; // framebuffer dropped here

    // 5. Submit frame.
    match render_result {
        Ok(res) => {
            let damage = res.damage.map(|d| d.as_slice().to_vec());
            backend
                .submit(damage.as_deref())
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        Err(e) => {
            backend.submit(None).ok();
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
    for window in state.space.elements().cloned().collect::<Vec<_>>() {
        window.send_frame(output, now, Some(FRAME_DURATION), |_, _| {
            Some(output.clone())
        });
    }

    Ok(())
}
