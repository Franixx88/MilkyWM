//! IPC server for MilkyWM.
//!
//! Listens on a Unix domain socket (`$XDG_RUNTIME_DIR/milkywm-<display>.sock`)
//! for newline-terminated JSON commands and returns JSON responses.
//!
//! Used by the `milkyctl` companion binary.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    path::PathBuf,
};

use calloop::{
    generic::Generic,
    Interest, Mode, PostAction,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{orbital::LayoutMode, state::MilkyState};
use smithay::reexports::calloop::EventLoop;

// ---------------------------------------------------------------------------
// Command / response types
// ---------------------------------------------------------------------------

/// Commands accepted over the IPC socket.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCommand {
    NextWorkspace,
    PrevWorkspace,
    NewWorkspace,
    SwitchWorkspace { index: usize },
    SetLayout { layout: LayoutArg },
    ToggleSwitcher,
    EnterGalaxy,
    ExitGalaxy,
    Exit,
    Status,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutArg {
    Horiz,
    Vert,
    Monocle,
}

impl From<LayoutArg> for LayoutMode {
    fn from(a: LayoutArg) -> Self {
        match a {
            LayoutArg::Horiz   => LayoutMode::HorizSplit,
            LayoutArg::Vert    => LayoutMode::VertSplit,
            LayoutArg::Monocle => LayoutMode::Monocle,
        }
    }
}

/// Response sent back over the socket.
#[derive(Debug, Serialize)]
pub struct IpcResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusData>,
}

#[derive(Debug, Serialize)]
pub struct StatusData {
    pub workspaces: usize,
    pub active: usize,
    pub layout: String,
}

impl IpcResponse {
    fn ok() -> Self {
        Self { ok: true, error: None, status: None }
    }
    fn err(msg: impl Into<String>) -> Self {
        Self { ok: false, error: Some(msg.into()), status: None }
    }
    fn with_status(data: StatusData) -> Self {
        Self { ok: true, error: None, status: Some(data) }
    }
}

// ---------------------------------------------------------------------------
// Socket path
// ---------------------------------------------------------------------------

pub fn sock_path(socket_name: &str) -> PathBuf {
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(runtime).join(format!("milkywm-{socket_name}.sock"))
}

// ---------------------------------------------------------------------------
// Initialise IPC source in calloop
// ---------------------------------------------------------------------------

pub fn init(
    event_loop: &mut EventLoop<'static, MilkyState>,
    state: &MilkyState,
) -> anyhow::Result<PathBuf> {
    let path = sock_path(&state.socket_name);

    // Remove stale socket from a previous run.
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)
        .map_err(|e| anyhow::anyhow!("IPC bind {:?}: {e}", path))?;
    listener.set_nonblocking(true)?;

    // Expose path for child processes launched by the compositor.
    std::env::set_var("MILKYWM_SOCK", &path);
    info!("IPC socket: {:?}", path);

    event_loop
        .handle()
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            |_readiness, listener, state| {
                loop {
                    match listener.accept() {
                        Ok((stream, _addr)) => handle_stream(stream, state),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => {
                            warn!("IPC accept: {e}");
                            break;
                        }
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("insert IPC source: {e}"))?;

    Ok(path)
}

// ---------------------------------------------------------------------------
// Per-connection handling (synchronous: read one command, write response)
// ---------------------------------------------------------------------------

fn handle_stream(stream: std::os::unix::net::UnixStream, state: &mut MilkyState) {
    stream.set_nonblocking(false).ok();

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }

    let response = match serde_json::from_str::<IpcCommand>(line.trim()) {
        Ok(cmd) => {
            debug!("IPC command: {cmd:?}");
            execute(cmd, state)
        }
        Err(e) => {
            warn!("IPC parse error: {e}");
            IpcResponse::err(format!("parse error: {e}"))
        }
    };

    let mut json = serde_json::to_string(&response).unwrap_or_else(|_| r#"{"ok":false}"#.into());
    json.push('\n');
    // Best-effort write; client may have disconnected.
    let mut writer = &stream;
    let _ = writer.write_all(json.as_bytes());
}

// ---------------------------------------------------------------------------
// Command execution
// ---------------------------------------------------------------------------

fn execute(cmd: IpcCommand, state: &mut MilkyState) -> IpcResponse {
    use crate::orbital::SwitcherState;

    match cmd {
        IpcCommand::NextWorkspace => {
            state.orbital.next_workspace();
            state.re_tile();
            IpcResponse::ok()
        }
        IpcCommand::PrevWorkspace => {
            state.orbital.prev_workspace();
            state.re_tile();
            IpcResponse::ok()
        }
        IpcCommand::NewWorkspace => {
            state.orbital.new_workspace();
            IpcResponse::ok()
        }
        IpcCommand::SwitchWorkspace { index } => {
            if index < state.orbital.workspaces.len() {
                state.orbital.switch_workspace(index);
                state.re_tile();
                IpcResponse::ok()
            } else {
                IpcResponse::err(format!(
                    "workspace {index} out of range ({})",
                    state.orbital.workspaces.len()
                ))
            }
        }
        IpcCommand::SetLayout { layout } => {
            state.orbital.set_layout(layout.into());
            state.re_tile();
            IpcResponse::ok()
        }
        IpcCommand::ToggleSwitcher => {
            match state.orbital.state {
                SwitcherState::Hidden  => state.orbital.open(),
                SwitcherState::Visible => state.orbital.close(),
                SwitcherState::Galaxy  => state.orbital.exit_galaxy(),
            }
            IpcResponse::ok()
        }
        IpcCommand::EnterGalaxy => {
            state.orbital.enter_galaxy();
            IpcResponse::ok()
        }
        IpcCommand::ExitGalaxy => {
            state.orbital.exit_galaxy();
            IpcResponse::ok()
        }
        IpcCommand::Exit => {
            info!("IPC exit command received");
            state.loop_signal.stop();
            IpcResponse::ok()
        }
        IpcCommand::Status => {
            let ws = &state.orbital.workspaces;
            let active = state.orbital.active;
            let layout = ws.get(active)
                .map(|w| format!("{:?}", w.layout).to_lowercase())
                .unwrap_or_else(|| "unknown".into());
            IpcResponse::with_status(StatusData {
                workspaces: ws.len(),
                active,
                layout,
            })
        }
    }
}

