//! DRM/KMS backend — runs MilkyWM as a standalone compositor on a TTY.
//!
//! Pipeline
//! ────────
//!  libseat session  →  udev device enumeration  →  DRM device (GPU)
//!  →  GBM allocator  →  EGL display  →  GlowRenderer
//!  →  DrmCompositor per connector  →  page-flip events  →  render loop
//!  →  libinput for keyboard / pointer input
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use libc::dev_t;
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            compositor::{DrmCompositor, FrameFlags},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
        },
        egl::EGLDisplay,
        input::{
            AbsolutePositionEvent, ButtonState, Event, InputEvent, KeyboardKeyEvent,
            PointerButtonEvent, PointerMotionEvent,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{solid::SolidColorRenderElement, Id, Kind},
            glow::GlowRenderer,
            utils::CommitCounter,
        },
        session::{libseat::LibSeatSession, Session},
        udev::{UdevBackend, UdevEvent},
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        drm::control::{
            connector::{self, State as ConnectorState},
            crtc,
            Device as DrmControlDevice,
            ModeTypeFlags,
        },
    },
    utils::{DeviceFd, Rectangle, Transform},
    wayland::seat::WaylandFocus,
};
use tracing::{error, info, warn};

use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
use smithay::reexports::rustix::fs::OFlags;

use crate::{
    orbital::SwitcherState,
    render::{gles::GlesSpaceRenderer, palette, MilkyRenderElement},
    state::MilkyState,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;

/// State for one connector/CRTC surface.
struct OutputSurface {
    output: Output,
    compositor: GbmDrmCompositor,
    space_gl: Option<GlesSpaceRenderer>,
}

/// State for one GPU/DRM device.
struct GpuDevice {
    drm: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    renderer: GlowRenderer,
    /// Active surfaces keyed by CRTC handle.
    surfaces: HashMap<crtc::Handle, OutputSurface>,
    node: DrmNode,
}

struct UdevData {
    session: LibSeatSession,
    devices: HashMap<DrmNode, GpuDevice>,
    loop_handle: LoopHandle<'static, MilkyState>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn init_drm(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<()> {
    // ── libseat session ───────────────────────────────────────────────────
    let (session, notifier) = LibSeatSession::new()
        .map_err(|e| anyhow::anyhow!("libseat: {e:?}"))?;

    let seat_name = session.seat();
    info!("DRM backend — seat: {seat_name}");

    event_loop
        .handle()
        .insert_source(notifier, |_, _, _| {})
        .map_err(|e| anyhow::anyhow!("session notifier: {e}"))?;

    // ── libinput ──────────────────────────────────────────────────────────
    let libinput_iface = LibinputSessionInterface::from(session.clone());
    let mut libinput_ctx =
        smithay::reexports::input::Libinput::new_with_udev(libinput_iface);
    libinput_ctx
        .udev_assign_seat(&seat_name)
        .map_err(|_| anyhow::anyhow!("libinput: udev_assign_seat failed"))?;

    let libinput_backend = LibinputInputBackend::new(libinput_ctx);
    event_loop
        .handle()
        .insert_source(libinput_backend, |event, _, state: &mut MilkyState| {
            handle_input_event(event, state);
        })
        .map_err(|e| anyhow::anyhow!("libinput source: {e}"))?;

    // ── udev device discovery ─────────────────────────────────────────────
    let udev_backend = UdevBackend::new(&seat_name)
        .map_err(|e| anyhow::anyhow!("udev backend: {e:?}"))?;

    let udev_data = Arc::new(Mutex::new(UdevData {
        session: session.clone(),
        devices: HashMap::new(),
        loop_handle: event_loop.handle(),
    }));

    // Enumerate devices already present.
    for (device_id, path) in udev_backend.device_list() {
        let mut ud = udev_data.lock().unwrap();
        let handle = ud.loop_handle.clone();
        if let Err(e) = device_added(&mut ud, device_id, path, state, handle) {
            warn!("Device init {path:?}: {e:#}");
        }
    }

    // Hotplug events.
    let ud_hotplug = Arc::clone(&udev_data);
    event_loop
        .handle()
        .insert_source(udev_backend, move |event, _, state: &mut MilkyState| {
            let mut ud = ud_hotplug.lock().unwrap();
            match event {
                UdevEvent::Added { device_id, path } => {
                    let handle = ud.loop_handle.clone();
                    if let Err(e) = device_added(&mut ud, device_id, &path, state, handle) {
                        warn!("Hotplug add {path:?}: {e:#}");
                    }
                }
                UdevEvent::Removed { device_id: _ } => {}
                _ => {}
            }
        })
        .map_err(|e| anyhow::anyhow!("udev source: {e}"))?;

    // ── 60 fps render timer ───────────────────────────────────────────────
    const FRAME_DUR: Duration = Duration::from_millis(1000 / 60);
    let ud_timer = Arc::clone(&udev_data);
    event_loop
        .handle()
        .insert_source(
            Timer::from_duration(FRAME_DUR),
            move |_, _, state: &mut MilkyState| {
                let mut ud = ud_timer.lock().unwrap();
                render_all(&mut ud, state);
                TimeoutAction::ToDuration(FRAME_DUR)
            },
        )
        .map_err(|e| anyhow::anyhow!("frame timer: {}", e.error))?;

    // ── XWayland ─────────────────────────────────────────────────────────
    crate::backend::winit::init_xwayland_pub(event_loop, state)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Device management
// ---------------------------------------------------------------------------

fn device_added(
    ud: &mut UdevData,
    _device_id: dev_t,
    path: &Path,
    state: &mut MilkyState,
    handle: LoopHandle<'static, MilkyState>,
) -> anyhow::Result<()> {
    let node = match DrmNode::from_path(path) {
        Ok(n) => n,
        Err(_) => return Ok(()), // not a DRM node
    };

    // Only handle primary or render nodes.
    if node.ty() != NodeType::Primary && node.ty() != NodeType::Render {
        return Ok(());
    }

    // Open via session (grants DRM master).
    let owned_fd = ud
        .session
        .open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|e| anyhow::anyhow!("session open {path:?}: {e:?}"))?;

    let device_fd = DrmDeviceFd::new(DeviceFd::from(owned_fd));

    let (drm, drm_notifier) =
        DrmDevice::new(device_fd.clone(), true)
            .map_err(|e| anyhow::anyhow!("DrmDevice: {e:?}"))?;

    let gbm = GbmDevice::new(device_fd.clone())
        .map_err(|e| anyhow::anyhow!("GbmDevice: {e:?}"))?;

    let egl = unsafe { EGLDisplay::new(gbm.clone()) }
        .map_err(|e| anyhow::anyhow!("EGLDisplay: {e:?}"))?;
    let ctx = smithay::backend::egl::EGLContext::new(&egl)
        .map_err(|e| anyhow::anyhow!("EGLContext: {e:?}"))?;
    let renderer = unsafe {
        GlowRenderer::new(ctx).map_err(|e| anyhow::anyhow!("GlowRenderer: {e:?}"))?
    };

    // Register DRM VBlank/flip events.
    let drm_node_log = node;
    handle
        .insert_source(drm_notifier, move |event, _, _state| {
            if let DrmEvent::VBlank(crtc) = event {
                tracing::trace!("VBlank {drm_node_log:?} crtc {crtc:?}");
            }
        })
        .map_err(|e| anyhow::anyhow!("drm notifier: {e}"))?;

    let mut gpu = GpuDevice { drm, gbm, renderer, surfaces: HashMap::new(), node };
    setup_outputs(&mut gpu, state)?;
    ud.devices.insert(node, gpu);

    info!("DRM device added: {path:?} ({node:?})");
    Ok(())
}

// ---------------------------------------------------------------------------
// Output / surface setup
// ---------------------------------------------------------------------------

fn setup_outputs(gpu: &mut GpuDevice, state: &mut MilkyState) -> anyhow::Result<()> {
    let resources = gpu
        .drm
        .resource_handles()
        .map_err(|e| anyhow::anyhow!("resource_handles: {e:?}"))?;

    for conn_handle in resources.connectors().to_vec() {
        let conn_info = match gpu.drm.get_connector(conn_handle, false) {
            Ok(i) => i,
            Err(_) => continue,
        };
        if conn_info.state() != ConnectorState::Connected {
            continue;
        }

        let mode = conn_info
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| conn_info.modes().first())
            .copied();
        let mode = match mode {
            Some(m) => m,
            None => { warn!("Connector {conn_handle:?}: no modes"); continue; }
        };

        let crtc = match pick_crtc(&gpu.drm, &resources, &conn_info) {
            Some(c) => c,
            None => { warn!("Connector {conn_handle:?}: no free CRTC"); continue; }
        };

        if let Err(e) = add_surface(gpu, state, conn_handle, crtc, mode) {
            warn!("Output setup {conn_handle:?}: {e:#}");
        }
    }
    Ok(())
}

fn pick_crtc(
    drm: &DrmDevice,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    conn: &connector::Info,
) -> Option<crtc::Handle> {
    // Try current encoder's CRTC first.
    if let Some(enc_handle) = conn.current_encoder() {
        if let Ok(enc) = drm.get_encoder(enc_handle) {
            if let Some(c) = enc.crtc() {
                return Some(c);
            }
        }
    }
    // Otherwise iterate encoders and find a compatible free CRTC.
    for enc_handle in conn.encoders() {
        if let Ok(enc) = drm.get_encoder(*enc_handle) {
            let possible = enc.possible_crtcs();
            for crtc in resources.filter_crtcs(possible) {
                return Some(crtc);
            }
        }
    }
    None
}

fn add_surface(
    gpu: &mut GpuDevice,
    state: &mut MilkyState,
    conn: connector::Handle,
    crtc: crtc::Handle,
    drm_mode: smithay::reexports::drm::control::Mode,
) -> anyhow::Result<()> {
    let (w, h) = drm_mode.size();
    let refresh = drm_mode.vrefresh() as i32 * 1000;

    let output = Output::new(
        format!("drm-{crtc:?}"),
        PhysicalProperties {
            size: (w as i32, h as i32).into(),
            subpixel: Subpixel::Unknown,
            make: "MilkyWM".into(),
            model: "DRM".into(),
            serial_number: String::new(),
        },
    );
    let out_mode = OutputMode { size: (w as i32, h as i32).into(), refresh };
    output.change_current_state(
        Some(out_mode),
        Some(Transform::Normal),
        Some(Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output.set_preferred(out_mode);
    state.space.map_output(&output, (0, 0));
    state.orbital.camera.screen_size = glam::Vec2::new(w as f32, h as f32);

    // DRM surface (KMS plane assignment).
    let surface = gpu
        .drm
        .create_surface(crtc, drm_mode, &[conn])
        .map_err(|e| anyhow::anyhow!("DrmSurface: {e:?}"))?;

    let allocator = GbmAllocator::new(
        gpu.gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gpu.gbm.clone(), NodeFilter::from(gpu.node));
    let renderer_formats = gpu.renderer.dmabuf_formats();
    let cursor_size = gpu.drm.cursor_size();

    let compositor = DrmCompositor::new(
        &output,
        surface,
        None,
        allocator,
        exporter,
        [Fourcc::Argb8888, Fourcc::Xrgb8888],
        renderer_formats,
        cursor_size,
        Some(gpu.gbm.clone()),
    )
    .map_err(|e| anyhow::anyhow!("DrmCompositor: {e:?}"))?;

    gpu.surfaces.insert(crtc, OutputSurface { output, compositor, space_gl: None });
    info!("Surface ready: {w}x{h}@{}", drm_mode.vrefresh());
    Ok(())
}

// ---------------------------------------------------------------------------
// Render loop
// ---------------------------------------------------------------------------

fn render_all(ud: &mut UdevData, state: &mut MilkyState) {
    state.orbital.tick();
    state.renderer.starfield.tick(1.0 / 60.0);

    let switcher_state = state.orbital.state;
    let cam = state.orbital.camera.position;

    // Collect (node, crtc) pairs to avoid borrow issues.
    let keys: Vec<(DrmNode, crtc::Handle)> = ud
        .devices
        .iter()
        .flat_map(|(node, gpu)| gpu.surfaces.keys().map(move |c| (*node, *c)))
        .collect();

    for (node, crtc) in keys {
        let gpu = match ud.devices.get_mut(&node) {
            Some(g) => g,
            None => continue,
        };
        let surface = match gpu.surfaces.get_mut(&crtc) {
            Some(s) => s,
            None => continue,
        };

        // Lazy-init GlesSpaceRenderer.
        if surface.space_gl.is_none() {
            match GlesSpaceRenderer::init(&mut gpu.renderer, &state.renderer.starfield) {
                Ok(r) => surface.space_gl = Some(r),
                Err(e) => { error!("GlesSpaceRenderer: {e:?}"); continue; }
            }
        }
        let space_gl = surface.space_gl.as_mut().unwrap();

        if let Err(e) = render_surface(
            &mut gpu.renderer,
            &mut surface.compositor,
            space_gl,
            &surface.output,
            state,
            switcher_state,
            cam,
        ) {
            warn!("render_surface {crtc:?}: {e:#}");
        }
    }
}

fn render_surface(
    renderer: &mut GlowRenderer,
    compositor: &mut GbmDrmCompositor,
    space_gl: &mut GlesSpaceRenderer,
    output: &Output,
    state: &mut MilkyState,
    switcher_state: SwitcherState,
    cam: glam::Vec2,
) -> anyhow::Result<()> {
    let phys_size = output
        .current_mode()
        .map(|m| smithay::utils::Size::<i32, smithay::utils::Physical>::from((m.size.w, m.size.h)))
        .unwrap_or_default();

    // Thumbnail update.
    if switcher_state == SwitcherState::Visible || switcher_state == SwitcherState::Galaxy {
        let planets: Vec<smithay::desktop::Window> = state
            .orbital
            .active_ws()
            .planets
            .iter()
            .map(|p| p.window.clone())
            .collect();
        for window in &planets {
            space_gl.thumbnails.update(renderer, window);
        }
        let refs: Vec<&smithay::desktop::Window> = planets.iter().collect();
        space_gl.thumbnails.retain(&refs);
    }

    // Collect render elements.
    let elements = state
        .space
        .render_elements_for_output::<GlowRenderer>(renderer, output, 1.0_f32)
        .unwrap_or_default();

    // Render via DrmCompositor.
    let _render_res = compositor
        .render_frame::<GlowRenderer, _>(
            renderer,
            &elements,
            [0.0_f32, 0.0, 0.0, 1.0],
            FrameFlags::DEFAULT,
        )
        .map_err(|e| anyhow::anyhow!("render_frame: {e:?}"))?;

    // Draw starfield + overlays on top.
    // DRM doesn't expose a GlowFrame after render_frame, so we fall back to
    // renderer.with_context() here.  This path runs in surfaceless mode which
    // is fine for KMS/GBM targets (no EGL window surface to worry about).
    renderer
        .with_context(|gl| unsafe {
            space_gl.draw_starfield_gl(&**gl, &state.renderer.starfield, cam.x, cam.y);
        })
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    match switcher_state {
        SwitcherState::Visible => {
            renderer
                .with_context(|gl| unsafe {
                    space_gl.draw_orbital_overlay_gl(&**gl, phys_size, &state.orbital);
                })
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        SwitcherState::Galaxy => {
            renderer
                .with_context(|gl| unsafe {
                    space_gl.draw_galaxy_view_gl(&**gl, phys_size, &state.orbital);
                })
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        SwitcherState::Hidden => {}
    }

    // Queue frame for page-flip.
    compositor
        .queue_frame(())
        .map_err(|e| anyhow::anyhow!("queue_frame: {e:?}"))?;

    // Frame callbacks.
    let now = Duration::from_millis(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    );
    const FRAME_DUR: Duration = Duration::from_millis(1000 / 60);
    for window in state.space.elements().cloned().collect::<Vec<_>>() {
        window.send_frame(output, now, Some(FRAME_DUR), |_, _| Some(output.clone()));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

fn handle_input_event(event: InputEvent<LibinputInputBackend>, state: &mut MilkyState) {
    match event {
        InputEvent::Keyboard { event } => {
            use smithay::backend::input::KeyState;
            use smithay::input::keyboard::{keysyms, FilterResult};

            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            let key_code = event.key_code();
            let key_state = event.state();
            let time = event.time_msec();

            if let Some(kb) = state.seat.get_keyboard() {
                kb.input::<(), _>(state, key_code, key_state, serial, time, |milky, _mods, handle| {
                    let pressed = key_state == KeyState::Pressed;
                    match handle.modified_sym().raw() {
                        keysyms::KEY_Super_L | keysyms::KEY_Super_R => {
                            if pressed {
                                if milky.orbital.state == SwitcherState::Hidden {
                                    milky.orbital.open();
                                }
                            } else if milky.orbital.state == SwitcherState::Visible {
                                milky.orbital.close();
                            }
                        }
                        keysyms::KEY_Tab if pressed => match milky.orbital.state {
                            SwitcherState::Visible => milky.orbital.highlight_next(),
                            SwitcherState::Galaxy => milky.orbital.highlight_next_ws(),
                            SwitcherState::Hidden => {}
                        },
                        keysyms::KEY_Return if pressed => match milky.orbital.state {
                            SwitcherState::Visible => milky.orbital.confirm_selection(),
                            SwitcherState::Galaxy => {
                                milky.orbital.confirm_ws_selection();
                                re_tile(milky);
                            }
                            SwitcherState::Hidden => {}
                        },
                        keysyms::KEY_g | keysyms::KEY_G if pressed => match milky.orbital.state {
                            SwitcherState::Galaxy => milky.orbital.exit_galaxy(),
                            _ => milky.orbital.enter_galaxy(),
                        },
                        keysyms::KEY_n | keysyms::KEY_N if pressed => {
                            milky.orbital.new_workspace();
                        }
                        keysyms::KEY_bracketright | keysyms::KEY_Right if pressed => {
                            milky.orbital.next_workspace();
                            re_tile(milky);
                        }
                        keysyms::KEY_bracketleft | keysyms::KEY_Left if pressed => {
                            milky.orbital.prev_workspace();
                            re_tile(milky);
                        }
                        keysyms::KEY_h | keysyms::KEY_H if pressed => {
                            milky.orbital.set_layout(crate::orbital::LayoutMode::HorizSplit);
                            re_tile(milky);
                        }
                        keysyms::KEY_v | keysyms::KEY_V if pressed => {
                            milky.orbital.set_layout(crate::orbital::LayoutMode::VertSplit);
                            re_tile(milky);
                        }
                        keysyms::KEY_m | keysyms::KEY_M if pressed => {
                            milky.orbital.set_layout(crate::orbital::LayoutMode::Monocle);
                            re_tile(milky);
                        }
                        keysyms::KEY_q | keysyms::KEY_Q if pressed => {
                            milky.loop_signal.stop();
                        }
                        _ => {}
                    }
                    FilterResult::Forward
                });
            }
        }

        InputEvent::PointerButton { event } => {
            if event.state() == ButtonState::Pressed {
                match state.orbital.state {
                    SwitcherState::Visible => {
                        if let Some(pos) = state.seat.get_pointer().map(|p| p.current_location()) {
                            state.orbital.pick(glam::Vec2::new(pos.x as f32, pos.y as f32));
                            state.orbital.confirm_selection();
                        }
                    }
                    SwitcherState::Galaxy => {
                        if let Some(pos) = state.seat.get_pointer().map(|p| p.current_location()) {
                            let sp = glam::Vec2::new(pos.x as f32, pos.y as f32);
                            let wp = state.orbital.camera.screen_to_world(sp);
                            let picked = state
                                .orbital
                                .workspaces
                                .iter()
                                .position(|ws| (wp - ws.world_pos).length() < 80.0);
                            if let Some(idx) = picked {
                                state.orbital.switch_workspace(idx);
                                re_tile(state);
                            }
                        }
                    }
                    SwitcherState::Hidden => {}
                }
            }
        }

        InputEvent::PointerMotionAbsolute { event } => {
            // Use raw x/y — libinput reports in device-space pixels on most hardware.
            let pos = smithay::utils::Point::<f64, smithay::utils::Logical>::from(
                (event.x(), event.y())
            );
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            if let Some(ptr) = state.seat.get_pointer() {
                ptr.motion(
                    state,
                    None,
                    &smithay::input::pointer::MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
            }
        }

        _ => {}
    }
}

fn re_tile(state: &mut MilkyState) {
    let screen = state.screen_rect();
    let ws = state.orbital.active_ws().clone();
    crate::compositor::apply_layout(&mut state.space, &ws, screen);
}
