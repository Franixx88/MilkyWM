//! Backend abstraction for MilkyWM.
//!
//! A `Backend` owns everything that is specific to one display/input source:
//!   - `Winit`   — runs the compositor inside an existing X11/Wayland window.
//!   - `Drm`     — takes over the GPU via DRM/KMS + libseat + libinput.
//!   - `Headless` — no display, used by integration tests.
//!
//! The enum variants *own* their state directly — no more `Rc<RefCell<_>>`
//! or `Arc<Mutex<_>>` captured in timer closures. Event-loop callbacks
//! reach the backend through `MilkyState::with_backend`, which temporarily
//! takes the backend out to give the closure mutable access to both the
//! backend and the rest of `MilkyState` simultaneously.
pub mod winit;
pub mod drm;
pub mod headless;

pub use self::winit::Winit;
pub use self::drm::Drm;
pub use self::headless::Headless;

use smithay::reexports::calloop::EventLoop;

use crate::state::MilkyState;

pub enum Backend {
    Winit(Winit),
    Drm(Drm),
    Headless(Headless),
}

impl Backend {
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Winit(_) => "winit",
            Self::Drm(_) => "drm",
            Self::Headless(_) => "headless",
        }
    }
}

/// Initialise the appropriate backend and attach it to the event loop.
/// The created backend is stored in `state.backend`.
/// Returns the name of the backend that was selected.
pub fn init(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &mut MilkyState,
) -> anyhow::Result<&'static str> {
    let choice = std::env::var("MILKYWM_BACKEND").ok();

    match choice.as_deref() {
        Some("headless") => {
            state.backend = Some(Backend::Headless(Headless::new()));
            Ok("headless")
        }
        Some("winit") => {
            winit::init_winit(event_loop, state)?;
            Ok("winit")
        }
        Some("drm") => {
            drm::init_drm(event_loop, state)?;
            Ok("drm")
        }
        None => {
            let nested = std::env::var("DISPLAY").is_ok()
                || std::env::var("WAYLAND_DISPLAY").is_ok();
            if nested {
                winit::init_winit(event_loop, state)?;
                Ok("winit")
            } else {
                drm::init_drm(event_loop, state)?;
                Ok("drm")
            }
        }
        Some(other) => Err(anyhow::anyhow!("unknown MILKYWM_BACKEND: {other}")),
    }
}
