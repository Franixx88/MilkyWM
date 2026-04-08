/// Winit backend — runs the compositor inside an existing desktop window.
///
/// Rendering pipeline per frame
/// ────────────────────────────
///  1. Dispatch winit events (resize, input, close).
///  2. Bind the EGL surface.
///  3. Begin GLES frame.
///  4. Clear to SPACE_BLACK.
///  5. Draw starfield quads.
///  6. Draw Wayland window surfaces (via smithay Space).
///  7. If orbital switcher is open: draw orbit rings + planet borders.
///  8. Submit frame (swap buffers).
use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
            utils::on_commit_buffer_handler,
            Frame, ImportAll, ImportMem, Renderer,
        },
        winit::{self, WinitError, WinitEvent, WinitGraphicsBackend},
    },
    desktop::space::SpaceRenderElements,
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop, LoopHandle,
    },
    utils::{Physical, Rectangle, Size, Transform},
};
use tracing::{error, info, warn};

use crate::{render::palette, state::MilkyState};

const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_millis(1000 / TARGET_FPS);

pub fn init_winit(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    let (mut backend, mut winit_evt) =
        winit::init::<GlesRenderer>().map_err(|e| anyhow::anyhow!("{:?}", e))?;

    let mode = Mode {
        size: backend.window_size().physical_size,
        refresh: TARGET_FPS as i32 * 1000,
    };

    let output = Output::new(
        "milkywm-winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MilkyWM".to_string(),
            model: "Winit".to_string(),
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

    let phys = backend.window_size().physical_size;
    state.orbital.camera.screen_size = glam::Vec2::new(phys.w as f32, phys.h as f32);

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    info!("Winit backend initialised — {}x{}", phys.w, phys.h);

    let loop_handle = event_loop.handle();

    loop_handle.insert_source(
        Timer::from_duration(FRAME_DURATION),
        move |_tick, _meta, state| {
            let result = winit_evt.dispatch_new_events(|event| {
                handle_winit_event(event, state, &output, &backend);
            });

            match result {
                Ok(_) => {}
                Err(WinitError::WindowClosed) => {
                    info!("Winit window closed — stopping");
                    state.loop_signal.stop();
                    return TimeoutAction::Drop;
                }
                Err(e) => {
                    error!("Winit error: {:?}", e);
                    state.loop_signal.stop();
                    return TimeoutAction::Drop;
                }
            }

            if let Err(e) = render_frame(&mut backend, state, &output, &mut damage_tracker) {
                warn!("Render error: {:?}", e);
            }

            TimeoutAction::ToDuration(FRAME_DURATION)
        },
    )?;

    Ok(())
}

fn handle_winit_event(
    event: WinitEvent,
    state: &mut MilkyState,
    output: &Output,
    backend: &WinitGraphicsBackend<GlesRenderer>,
) {
    use smithay::backend::input::InputEvent;
    use smithay::backend::winit::WinitInput;

    match event {
        WinitEvent::Resized { size, scale_factor } => {
            let mode = Mode {
                size: size.physical_size,
                refresh: TARGET_FPS as i32 * 1000,
            };
            output.change_current_state(
                Some(mode),
                None,
                Some(Scale::Fractional(scale_factor)),
                None,
            );
            state.orbital.camera.screen_size = glam::Vec2::new(
                size.physical_size.w as f32,
                size.physical_size.h as f32,
            );
        }

        WinitEvent::Input(InputEvent::Keyboard { event }) => {
            use smithay::backend::input::KeyboardKeyEvent;
            use smithay::input::keyboard::{keysyms, FilterResult};

            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            let time = smithay::backend::input::KeyboardKeyEvent::time_msec(&event);

            if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.input::<(), _>(
                    state,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |milky_state, _modifiers, handle| {
                        let sym = handle.modified_sym();
                        match sym.raw() {
                            keysyms::KEY_Super_L | keysyms::KEY_Super_R => {
                                use smithay::backend::input::KeyState;
                                if event.state() == KeyState::Pressed {
                                    milky_state.orbital.open();
                                } else {
                                    milky_state.orbital.close();
                                }
                            }
                            keysyms::KEY_Tab => {
                                if milky_state.orbital.state
                                    == crate::orbital::SwitcherState::Visible
                                    && event.state() == smithay::backend::input::KeyState::Pressed
                                {
                                    milky_state.orbital.highlight_next();
                                }
                            }
                            keysyms::KEY_Return => {
                                if milky_state.orbital.state
                                    == crate::orbital::SwitcherState::Visible
                                {
                                    milky_state.orbital.confirm_selection();
                                }
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
                if state.orbital.state == crate::orbital::SwitcherState::Visible {
                    if let Some(pos) = state.seat.get_pointer().map(|p| p.current_location()) {
                        let screen = glam::Vec2::new(pos.x as f32, pos.y as f32);
                        state.orbital.pick(screen);
                        state.orbital.confirm_selection();
                    }
                }
            }
        }

        WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
            use smithay::backend::input::AbsolutePositionEvent;
            let pos = event.position_transformed(backend.window_size().physical_size);
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();

            if let Some(pointer) = state.seat.get_pointer() {
                pointer.motion(
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

fn render_frame(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    state: &mut MilkyState,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
) -> anyhow::Result<()> {
    state.orbital.tick();
    state.renderer.starfield.tick(1.0 / TARGET_FPS as f32);

    backend.bind().map_err(|e| anyhow::anyhow!("{:?}", e))?;

    let size = backend.window_size().physical_size;
    let renderer = backend.renderer();

    let elements: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>> =
        state.space.render_elements_for_output(renderer, output, 1.0);

    let render_result = damage_tracker.render_output(
        renderer,
        0,
        &elements,
        palette::SPACE_BLACK,
    );

    match render_result {
        Ok(render_output_result) => {
            let damage = render_output_result.damage.map(|d| d.as_slice().to_vec());
            backend
                .submit(damage.as_deref())
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;
        }
        Err(e) => {
            backend.submit(None).ok();
            return Err(anyhow::anyhow!("Render error: {:?}", e));
        }
    }

    state.space.elements().for_each(|w| {
        state.space.send_frames(
            output,
            &w,
            state
                .seat
                .get_pointer()
                .as_ref()
                .and_then(|p| state.space.element_under(p.current_location()))
                .as_ref()
                .map(|(e, _)| e),
            |_, _| Some(output.clone()),
        );
    });

    Ok(())
}
