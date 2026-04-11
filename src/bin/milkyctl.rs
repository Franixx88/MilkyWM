//! milkyctl — command-line IPC client for MilkyWM.
//!
//! Usage:
//!   milkyctl status
//!   milkyctl next-workspace
//!   milkyctl prev-workspace
//!   milkyctl new-workspace
//!   milkyctl switch <N>
//!   milkyctl layout horiz|vert|monocle
//!   milkyctl toggle-switcher
//!   milkyctl enter-galaxy
//!   milkyctl exit-galaxy
//!   milkyctl exit
//!
//! The socket path is read from `$MILKYWM_SOCK`, falling back to
//! `$XDG_RUNTIME_DIR/milkywm-<$WAYLAND_DISPLAY>.sock`.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_help();
        return;
    }

    let json = match build_command(&args) {
        Some(j) => j,
        None => {
            eprintln!("milkyctl: unknown command '{}'", args[0]);
            eprintln!("Run 'milkyctl --help' for usage.");
            process::exit(1);
        }
    };

    let path = socket_path();
    let mut stream = UnixStream::connect(&path).unwrap_or_else(|e| {
        eprintln!("milkyctl: cannot connect to {:?}: {e}", path);
        eprintln!("Is MilkyWM running?");
        process::exit(1);
    });

    // Send command (newline-terminated JSON).
    writeln!(stream, "{json}").unwrap_or_else(|e| {
        eprintln!("milkyctl: write error: {e}");
        process::exit(1);
    });

    // Read response.
    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response).unwrap_or_else(|e| {
        eprintln!("milkyctl: read error: {e}");
        process::exit(1);
    });

    // Pretty-print the response.
    let trimmed = response.trim();
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if val.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let err = val.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            eprintln!("Error: {err}");
            process::exit(1);
        }
        if let Some(status) = val.get("status") {
            // Pretty status output.
            println!(
                "workspaces: {}  active: {}  layout: {}",
                status.get("workspaces").and_then(|v| v.as_u64()).unwrap_or(0),
                status.get("active").and_then(|v| v.as_u64()).unwrap_or(0),
                status.get("layout").and_then(|v| v.as_str()).unwrap_or("?"),
            );
        } else {
            println!("ok");
        }
    } else {
        // Fallback: print raw response.
        print!("{trimmed}");
    }
}

// ---------------------------------------------------------------------------
// Command builder
// ---------------------------------------------------------------------------

fn build_command(args: &[String]) -> Option<String> {
    let cmd = match args[0].as_str() {
        "status"           => r#"{"cmd":"status"}"#.into(),
        "next-workspace"   => r#"{"cmd":"next_workspace"}"#.into(),
        "prev-workspace"   => r#"{"cmd":"prev_workspace"}"#.into(),
        "new-workspace"    => r#"{"cmd":"new_workspace"}"#.into(),
        "toggle-switcher"  => r#"{"cmd":"toggle_switcher"}"#.into(),
        "enter-galaxy"     => r#"{"cmd":"enter_galaxy"}"#.into(),
        "exit-galaxy"      => r#"{"cmd":"exit_galaxy"}"#.into(),
        "exit"             => r#"{"cmd":"exit"}"#.into(),

        "switch" => {
            let idx: usize = args.get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    eprintln!("Usage: milkyctl switch <N>");
                    process::exit(1);
                });
            format!(r#"{{"cmd":"switch_workspace","index":{idx}}}"#)
        }

        "layout" => {
            let layout = args.get(1).map(|s| s.as_str()).unwrap_or_else(|| {
                eprintln!("Usage: milkyctl layout horiz|vert|monocle");
                process::exit(1);
            });
            match layout {
                "horiz" | "h"   => r#"{"cmd":"set_layout","layout":"horiz"}"#.into(),
                "vert"  | "v"   => r#"{"cmd":"set_layout","layout":"vert"}"#.into(),
                "monocle" | "m" => r#"{"cmd":"set_layout","layout":"monocle"}"#.into(),
                other => {
                    eprintln!("milkyctl: unknown layout '{other}' (horiz|vert|monocle)");
                    process::exit(1);
                }
            }
        }

        _ => return None,
    };
    Some(cmd)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("MILKYWM_SOCK") {
        return PathBuf::from(p);
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".into());
    let display = std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_else(|_| "wayland-0".into());
    PathBuf::from(runtime).join(format!("milkywm-{display}.sock"))
}

fn print_help() {
    println!(
        "milkyctl — MilkyWM IPC client

USAGE:
  milkyctl <command> [args]

COMMANDS:
  status                    Show compositor status
  next-workspace            Switch to next workspace
  prev-workspace            Switch to previous workspace
  new-workspace             Create a new workspace
  switch <N>                Switch to workspace N (0-based)
  layout horiz|vert|monocle Change tiling layout
  toggle-switcher           Open/close the orbital switcher
  enter-galaxy              Enter galaxy view
  exit-galaxy               Exit galaxy view
  exit                      Quit MilkyWM

ENVIRONMENT:
  MILKYWM_SOCK              Override socket path
  WAYLAND_DISPLAY           Used to locate socket if MILKYWM_SOCK is unset
"
    );
}
