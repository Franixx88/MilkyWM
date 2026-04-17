//! Shared input handling used by both the winit and drm backends.
//!
//! The backends only differ in how they produce raw events — the semantic
//! logic (keyboard shortcuts, switcher picks, pointer forwarding) is the same.
use smithay::{
    backend::input::ButtonState,
    input::{
        keyboard::{keysyms, FilterResult, ModifiersState},
        pointer::{ButtonEvent, MotionEvent},
    },
    utils::{Logical, Point, Serial, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};

use crate::{
    orbital::{LayoutMode, SwitcherState},
    state::MilkyState,
};

/// Handle a keyboard shortcut. Returns `FilterResult::Intercept` if the key
/// should not be forwarded to the focused client.
pub fn handle_shortcut(
    milky: &mut MilkyState,
    keysym: u32,
    pressed: bool,
    mods: &ModifiersState,
) -> FilterResult<()> {
    // Under winit (nested in Hyprland etc.) the host compositor intercepts
    // Super globally — we never see it. Fall back to Alt as the modkey.
    let nested = milky.is_nested();
    let mod_held = if nested { mods.alt } else { mods.logo };
    let is_toggle_key = if nested {
        matches!(keysym, keysyms::KEY_Alt_L | keysyms::KEY_Alt_R)
    } else {
        matches!(keysym, keysyms::KEY_Super_L | keysyms::KEY_Super_R)
    };

    match keysym {
        // Mod+T — launch terminal (do not forward).
        keysyms::KEY_t | keysyms::KEY_T if pressed && mod_held => {
            if milky.orbital.state == SwitcherState::Visible {
                milky.orbital.close();
            }
            launch_terminal(&milky.socket_name);
            return FilterResult::Intercept(());
        }

        // Mod+Q — quit compositor (do not forward).
        keysyms::KEY_q | keysyms::KEY_Q if pressed && mod_held => {
            milky.loop_signal.stop();
            return FilterResult::Intercept(());
        }

        // Mod key alone — toggle orbital switcher (System view).
        // `is_toggle_key` resolves to Super on TTY, Alt on nested winit.
        sym if is_toggle_key => {
            let _ = sym;
            if pressed {
                if milky.orbital.state == SwitcherState::Hidden {
                    milky.orbital.open();
                }
            } else if milky.orbital.state == SwitcherState::Visible {
                milky.orbital.close();
            }
        }

        // Tab — navigate planets / workspaces while the switcher is open.
        // Held Super + Tab repeats via xkb key-repeat, which is fine (idempotent).
        keysyms::KEY_Tab if pressed && milky.orbital.state != SwitcherState::Hidden => {
            match milky.orbital.state {
                SwitcherState::Visible => milky.orbital.highlight_next(),
                SwitcherState::Galaxy => milky.orbital.highlight_next_ws(),
                SwitcherState::Hidden => {}
            }
            return FilterResult::Intercept(());
        }

        // Return — confirm selection (retiles on workspace switch and focuses new sun).
        keysyms::KEY_Return if pressed && milky.orbital.state != SwitcherState::Hidden => {
            match milky.orbital.state {
                SwitcherState::Visible => {
                    milky.orbital.confirm_selection();
                    milky.re_tile();
                    focus_active_sun(milky, SERIAL_COUNTER.next_serial());
                }
                SwitcherState::Galaxy => {
                    milky.orbital.confirm_ws_selection();
                    milky.re_tile();
                    focus_active_sun(milky, SERIAL_COUNTER.next_serial());
                }
                SwitcherState::Hidden => {}
            }
            return FilterResult::Intercept(());
        }

        // Super+G — toggle Galaxy view.
        keysyms::KEY_g | keysyms::KEY_G if pressed && mod_held => {
            match milky.orbital.state {
                SwitcherState::Galaxy => milky.orbital.exit_galaxy(),
                _ => milky.orbital.enter_galaxy(),
            }
            return FilterResult::Intercept(());
        }

        // Super+N — new workspace.
        keysyms::KEY_n | keysyms::KEY_N if pressed && mod_held => {
            milky.orbital.new_workspace();
            return FilterResult::Intercept(());
        }

        // Super+Right / Super+] — next workspace.
        keysyms::KEY_bracketright | keysyms::KEY_Right if pressed && mod_held => {
            milky.orbital.next_workspace();
            milky.re_tile();
            return FilterResult::Intercept(());
        }

        // Super+Left / Super+[ — previous workspace.
        keysyms::KEY_bracketleft | keysyms::KEY_Left if pressed && mod_held => {
            milky.orbital.prev_workspace();
            milky.re_tile();
            return FilterResult::Intercept(());
        }

        // Super+H / Super+V / Super+M — layout modes.
        keysyms::KEY_h | keysyms::KEY_H if pressed && mod_held => {
            milky.orbital.set_layout(LayoutMode::HorizSplit);
            milky.re_tile();
            return FilterResult::Intercept(());
        }
        keysyms::KEY_v | keysyms::KEY_V if pressed && mod_held => {
            milky.orbital.set_layout(LayoutMode::VertSplit);
            milky.re_tile();
            return FilterResult::Intercept(());
        }
        keysyms::KEY_m | keysyms::KEY_M if pressed && mod_held => {
            milky.orbital.set_layout(LayoutMode::Monocle);
            milky.re_tile();
            return FilterResult::Intercept(());
        }

        _ => {}
    }
    FilterResult::Forward
}

fn launch_terminal(socket_name: &str) {
    std::process::Command::new("foot")
        .env("WAYLAND_DISPLAY", socket_name)
        .spawn()
        .or_else(|_| {
            std::process::Command::new("alacritty")
                .env("WAYLAND_DISPLAY", socket_name)
                .spawn()
        })
        .or_else(|_| {
            std::process::Command::new("kitty")
                .env("WAYLAND_DISPLAY", socket_name)
                .spawn()
        })
        .or_else(|_| {
            std::process::Command::new("xterm")
                .env("DISPLAY", std::env::var("DISPLAY").unwrap_or_default())
                .spawn()
        })
        .ok();
}

/// Give keyboard focus to the active workspace's sun, if any.
pub fn focus_active_sun(state: &mut MilkyState, serial: Serial) {
    let Some(sun) = state.orbital.sun().cloned() else { return };
    let Some(surf) = sun.wl_surface() else { return };
    let Some(kb) = state.seat.get_keyboard() else { return };
    kb.set_focus(state, Some(surf.into_owned()), serial);
}

/// Handle a pointer button event. Forwards the event to the seat (so clients
/// still receive it) and then interprets it against the orbital switcher
/// state for planet / workspace picking and window focus.
pub fn dispatch_pointer_button(
    state: &mut MilkyState,
    button: u32,
    button_state: ButtonState,
    time: u32,
) {
    let serial = SERIAL_COUNTER.next_serial();
    if let Some(ptr) = state.seat.get_pointer() {
        ptr.button(
            state,
            &ButtonEvent {
                serial,
                time,
                button,
                state: button_state,
            },
        );
    }

    if button_state != ButtonState::Pressed {
        return;
    }

    let sp = state.cursor_pos;
    let screen = glam::Vec2::new(sp.x as f32, sp.y as f32);

    match state.orbital.state {
        SwitcherState::Visible => {
            if state.orbital.pick(screen) {
                state.orbital.confirm_selection();
                state.re_tile();
                focus_active_sun(state, serial);
            }
        }
        SwitcherState::Galaxy => {
            if let Some(idx) = state.orbital.pick_ws_screen_pub(screen) {
                state.orbital.switch_workspace(idx);
                state.re_tile();
                focus_active_sun(state, serial);
            } else {
                state.orbital.exit_galaxy();
            }
        }
        SwitcherState::Hidden => {
            if let Some((window, _)) = state
                .space
                .element_under(sp)
                .map(|(w, l)| (w.clone(), l))
            {
                if let Some(surf) = window.wl_surface() {
                    if let Some(kb) = state.seat.get_keyboard() {
                        kb.set_focus(state, Some(surf.into_owned()), serial);
                    }
                }
                state.orbital.set_sun(window);
                state.re_tile();
            }
        }
    }
}

/// Dispatch a pointer motion to the given logical screen position.
///
/// Updates the compositor's cursor position, the orbital hover highlight,
/// and forwards a `MotionEvent` to the seat. While the orbital switcher is
/// open, pointer focus is suppressed so clients don't see hover events.
pub fn dispatch_cursor_motion(state: &mut MilkyState, pos: Point<f64, Logical>, time: u32) {
    state.cursor_pos = pos;

    let screen = glam::Vec2::new(pos.x as f32, pos.y as f32);
    state.orbital.hover_at(screen);

    let serial = SERIAL_COUNTER.next_serial();
    if let Some(ptr) = state.seat.get_pointer() {
        let focus = if state.orbital.state == SwitcherState::Hidden {
            state
                .space
                .element_under(pos)
                .and_then(|(window, window_loc)| {
                    let local = Point::<f64, Logical>::from((
                        pos.x - window_loc.x as f64,
                        pos.y - window_loc.y as f64,
                    ));
                    window.wl_surface().map(|s| (s.into_owned(), local))
                })
        } else {
            None
        };
        ptr.motion(
            state,
            focus,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
    }
}
