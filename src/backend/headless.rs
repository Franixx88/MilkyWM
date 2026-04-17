//! Headless backend — no display, no input, no rendering.
//!
//! A placeholder for integration tests. Lets `Backend::Headless` variant
//! exist so that test harnesses can drive `MilkyState` through protocol
//! handlers without needing a GPU or a windowing system.
//!
//! Currently carries no state. When a test-oriented render path is added,
//! it will gain a software framebuffer and a virtual Output here.

pub struct Headless;

impl Headless {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}
