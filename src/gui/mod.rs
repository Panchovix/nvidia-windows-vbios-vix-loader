//! vbios-loader-gui: a window to pick a GPU and a VBIOS file, then load, remove or verify the override (gui.py).
//!
//! Widgets only; the state, checks and texts are in `controller`, the registry work in `operations`.
//! VBIOS_LOADER_GUI_SMOKE=1 opens the window, reads the status, switches to the other language and theme, and
//! closes it after a few seconds.
//!
//! The window is plain Win32 (no GPU-rendered toolkit), so it also works in Safe Mode, where the README sends
//! users to remove an override. Everything that needs no Windows lives in submodules tested on any system: the
//! command line (`args`), the geometry (`layout`), the colors (`palette`), the smoke schedule (`smoke`) and small
//! text helpers (`text`). `window`, `paint` and `console` are the Win32 parts.

use std::sync::Arc;

use crate::platform::Platform;

mod args;
#[cfg(windows)]
mod console;
#[cfg_attr(not(windows), allow(dead_code))]
mod layout;
#[cfg(windows)]
mod paint;
#[cfg_attr(not(windows), allow(dead_code))]
mod palette;
#[cfg_attr(not(windows), allow(dead_code))]
mod smoke;
#[cfg_attr(not(windows), allow(dead_code))]
mod text;
#[cfg(windows)]
mod window;

pub const PROG: &str = "vbios-loader-gui";
pub const SMOKE_ENV: &str = "VBIOS_LOADER_GUI_SMOKE";
pub const SMOKE_SECONDS: u32 = 20;

/// Where a text printed before the window opens goes: argparse prints help and the version to stdout, errors to
/// stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stream {
    Out,
    Err,
}

/// The process entry of vbios-loader-gui.exe; returns the exit code. An unexpected panic is reported (in a message
/// box when there is no console) and gives exit code 1, as an uncaught exception did in gui.py.
pub fn main() -> i32 {
    match std::panic::catch_unwind(start) {
        Ok(code) => code,
        Err(payload) => {
            let message = text::panic_message(&*payload);
            emit(Stream::Err, &|_| format!("{PROG}: unexpected error: {message}\n"));
            1
        }
    }
}

fn start() -> i32 {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let options = match args::parse(&argv) {
        args::Parsed::Run(options) => options,
        args::Parsed::Help => {
            emit(Stream::Out, &args::help_text);
            return 0;
        }
        args::Parsed::Version => {
            emit(Stream::Out, &args::version_text);
            return 0;
        }
        args::Parsed::Error(error) => {
            let message = error.message(&crate::pyfmt::repr_str);
            emit(Stream::Err, &|columns| args::error_text(columns, &message));
            return 2;
        }
    };
    let platform = match crate::cli::windows_platform() {
        Ok(platform) => platform,
        Err(error) => {
            emit(Stream::Err, &|_| format!("{PROG}: {error}\n"));
            return 1;
        }
    };
    run(options, platform)
}

#[cfg(windows)]
fn run(options: args::Options, platform: Arc<dyn Platform>) -> i32 {
    window::run(options, platform)
}

/// Not reached in practice: `windows_platform` refuses to run on other systems.
#[cfg(not(windows))]
fn run(options: args::Options, platform: Arc<dyn Platform>) -> i32 {
    let _ = (options, platform);
    emit(Stream::Err, &|_| format!("{PROG}: this tool runs on Windows only\n"));
    1
}

/// Print a text made for a terminal `columns` wide (argparse wraps its help to the terminal).
#[cfg(windows)]
fn emit(stream: Stream, text: &dyn Fn(i64) -> String) {
    console::emit(stream, text);
}

#[cfg(not(windows))]
fn emit(stream: Stream, text: &dyn Fn(i64) -> String) {
    use std::io::Write;
    let columns = std::env::var("COLUMNS").ok();
    let text = text(args::columns_from_env(columns.as_deref()).unwrap_or(args::DEFAULT_COLUMNS));
    let _ = match stream {
        Stream::Out => std::io::stdout().write_all(text.as_bytes()),
        Stream::Err => std::io::stderr().write_all(text.as_bytes()),
    };
}
