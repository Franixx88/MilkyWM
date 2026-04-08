use std::time::Duration;
use smithay::{
    backend::{
        renderer::{ damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement, gles::GlesRenderer, Frame, Renderer },
        winit::{self, WinitError, WinitEvent, WinitGraphicsBackend},
    },
    desktop::space::SpaceRenderElements,
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::{ timer::{TimeoutAction, Timer}, EventLoop },
    utils::Transform,
};
use tracing::{error, info, warn};
use crate::{ orbital::SwitcherState, render::gles::GlesSpaceRenderer, state::MilkyState };

const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_millis(1000 / TARGET_FPS);

pub fn init_winit(event_loop: &mut EventLoop<'static, MilkyState>, state: &mut MilkyState) -> anyhow::Result<()> {
    let (mut backend, mut winit_evt) = winit::init::<GlesRenderer>().map_err(|e| anyhow::anyhow!("{:?}", e))?;
    let mode = Mode { size: backend.window_size().physical_size, refresh: TARGET_FPS as i32 * 1000 };
    let output = Output::new("milkywm-winit".to_string(), PhysicalProperties { size: (0,0).into(), subpixel: Subpixel::Unknown, make: "MilkyWM".into(), model: "Winit".into() });
    output.change_current_state(Some(mode), Some(Transform::Normal), Some(Scale::Integer(1)), Some((0,0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    let phys = backend.window_size().physical_size;
    state.orbital.camera.screen_size = glam::Vec2::new(phys.w as f32, phys.h as f32);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut space_gl: Option<GlesSpaceRenderer> = None;
    info!("Winit backend initialised — {}x{}", phys.w, phys.h);
    event_loop.handle().insert_source(Timer::from_duration(FRAME_DURATION), move |_, _, state| {
        let result = winit_evt.dispatch_new_events(|ev| handle_winit_event(ev, state, &output, &backend));
        match result {
            Ok(_) => {}
            Err(WinitError::WindowClosed) => { info!("Winit window closed"); state.loop_signal.stop(); return TimeoutAction::Drop; }
            Err(e) => { error!("Winit: {e:?}"); state.loop_signal.stop(); return TimeoutAction::Drop; }
        }
        if space_gl.is_none() {
            match GlesSpaceRenderer::init(backend.renderer(), &state.renderer.starfield) {
                Ok(r)  => { info!("GlesSpaceRenderer ready"); space_gl = Some(r); }
                Err(e) => { error!("GlesSpaceRenderer: {e:?}"); state.loop_signal.stop(); return TimeoutAction::Drop; }
            }
        }
        if let Err(e) = render_frame(&mut backend, state, &output, &mut damage_tracker, space_gl.as_ref().unwrap()) { warn!("{e:?}"); }
        TimeoutAction::ToDuration(FRAME_DURATION)
    })?;
    Ok(())
}

fn handle_winit_event(event: WinitEvent, state: &mut MilkyState, output: &Output, backend: &WinitGraphicsBackend<GlesRenderer>) {
    use smithay::backend::input::InputEvent;
    match event {
        WinitEvent::Resized { size, scale_factor } => {
            output.change_current_state(Some(Mode { size: size.physical_size, refresh: TARGET_FPS as i32 * 1000 }), None, Some(Scale::Fractional(scale_factor)), None);
            state.orbital.camera.screen_size = glam::Vec2::new(size.physical_size.w as f32, size.physical_size.h as f32);
        }
        WinitEvent::Input(InputEvent::Keyboard { event }) => {
            use smithay::backend::input::KeyboardKeyEvent;
            use smithay::input::keyboard::{keysyms, FilterResult};
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            if let Some(kb) = state.seat.get_keyboard() {
                kb.input::<(), _>(state, event.key_code(), event.state(), serial, KeyboardKeyEvent::time_msec(&event), |m, _, h| {
                    match h.modified_sym().raw() {
                        keysyms::KEY_Super_L | keysyms::KEY_Super_R => { use smithay::backend::input::KeyState; if event.state()==KeyState::Pressed { m.orbital.open(); } else { m.orbital.close(); } }
                        keysyms::KEY_Tab    => { if m.orbital.state==SwitcherState::Visible && event.state()==smithay::backend::input::KeyState::Pressed { m.orbital.highlight_next(); } }
                        keysyms::KEY_Return => { if m.orbital.state==SwitcherState::Visible { m.orbital.confirm_selection(); } }
                        _ => {}
                    }
                    FilterResult::Forward
                });
            }
        }
        WinitEvent::Input(InputEvent::PointerButton { event }) => {
            use smithay::backend::input::{ButtonState, PointerButtonEvent};
            if event.state()==ButtonState::Pressed && state.orbital.state==SwitcherState::Visible {
                if let Some(pos) = state.seat.get_pointer().map(|p| p.current_location()) {
                    state.orbital.pick(glam::Vec2::new(pos.x as f32, pos.y as f32));
                    state.orbital.confirm_selection();
                }
            }
        }
        WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
            use smithay::backend::input::AbsolutePositionEvent;
            let pos = event.position_transformed(backend.window_size().physical_size);
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            if let Some(ptr) = state.seat.get_pointer() {
                ptr.motion(state, None, &smithay::input::pointer::MotionEvent { location: pos.into(), serial, time: event.time_msec() });
            }
        }
        _ => {}
    }
}

fn render_frame(backend: &mut WinitGraphicsBackend<GlesRenderer>, state: &mut MilkyState, output: &Output, dt: &mut OutputDamageTracker, gl: &GlesSpaceRenderer) -> anyhow::Result<()> {
    state.orbital.tick();
    state.renderer.starfield.tick(1.0 / TARGET_FPS as f32);
    backend.bind().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let size = backend.window_size().physical_size;
    let cam  = &state.orbital.camera;
    gl.draw_starfield(backend.renderer(), size, &state.renderer.starfield, cam.position.x, cam.position.y)?;
    let elements: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>> =
        state.space.render_elements_for_output(backend.renderer(), output, 1.0);
    let res = dt.render_output(backend.renderer(), 0, &elements, [0.0, 0.0, 0.0, 0.0]);
    if state.orbital.state == SwitcherState::Visible { gl.draw_orbital_overlay(backend.renderer(), size, &state.orbital)?; }
    match res {
        Ok(r)  => { let d = r.damage.map(|d| d.as_slice().to_vec()); backend.submit(d.as_deref()).map_err(|e| anyhow::anyhow!("{e:?}"))?; }
        Err(e) => { backend.submit(None).ok(); return Err(anyhow::anyhow!("{e:?}")); }
    }
    state.space.elements().for_each(|w| {
        state.space.send_frames(output, &w,
            state.seat.get_pointer().as_ref().and_then(|p| state.space.element_under(p.current_location())).as_ref().map(|(e,_)| e),
            |_,_| Some(output.clone()));
    });
    Ok(())
}
