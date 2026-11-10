//! vbios-loader: load a modified VBIOS on Windows through the NVIDIA driver's registry override (cli.py).
//!
//! Nothing is flashed: the image is stored in the GPU's driver registry key, and the driver uses it after the next
//! Windows restart. unload removes it; verify checks that the driver is running it. The arguments, texts, `--json`
//! output and exit codes are those of the Python loader (argparse included): 0 done, 1 refused or failed,
//! 2 invalid arguments, 4 verify found the override not running (or unstable), 70 unexpected internal error,
//! 130 interrupted.

use std::io::{self, Write};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::json::{Map, Value};
use crate::nvapi::IMAGE_SIZE;
use crate::operations::{load, status, unload, verify, Selection, MIN_DRIVER, VERIFY_OK};
use crate::platform::Platform;
use crate::pyfmt::parse_int;

mod argparse;

use argparse::{Action, Io, Kind, Namespace, Parser, Val};

pub const PROG: &str = "vbios-loader";

/// The first line of cli.py's docstring: the description in --help.
const DESCRIPTION: &str =
    "vbios-loader: load a modified VBIOS on Windows through the NVIDIA driver's registry override.";

/// The platform of this system: `win32::Win32Platform` on Windows, else the error "this tool runs on Windows only".
pub fn windows_platform() -> Result<Arc<dyn Platform>> {
    #[cfg(windows)]
    {
        Ok(Arc::new(crate::win32::Win32Platform::new()))
    }
    #[cfg(not(windows))]
    {
        Err(Error::new("this tool runs on Windows only"))
    }
}

/// cli.parser(): the argparse parser of the command line.
fn parser() -> Parser {
    let mut root = Parser::new(PROG.to_string(), Some(DESCRIPTION));
    root.add_version(format!("{PROG} {}", crate::VERSION));
    let mut status = Parser::new(format!("{PROG} status"), None);
    status.add(Action::option(
        &["--json"],
        "json",
        Kind::StoreTrue,
        "machine-readable output",
    ));
    let mut commands = vec![("status", "list NVIDIA GPUs and their override state", status)];
    let changes = [
        (
            "load",
            "store a VBIOS override for one GPU; it applies after restarting Windows",
        ),
        (
            "unload",
            "remove the override and restore the previous firmware setting; it applies after restarting Windows",
        ),
        (
            "verify",
            "compare the VBIOS the driver is running with the stored override",
        ),
    ];
    for (name, help) in changes {
        let mut p = Parser::new(format!("{PROG} {name}"), None);
        if name == "load" {
            p.add(Action::positional(
                "payload",
                Kind::Path,
                Some("VBIOS file (see the README for which file to use)"),
            ));
        }
        p.add(Action::option(&["--gpu"], "gpu", Kind::Int, "GPU index as shown by status").with_metavar("INDEX"));
        p.add(
            Action::option(
                &["--pci"],
                "pci",
                Kind::Text,
                "PCI location as shown by status, for example 01:00.0",
            )
            .with_metavar("BUS:DEV.FN"),
        );
        if name != "verify" {
            p.add(Action::option(
                &["--dry-run"],
                "dry_run",
                Kind::StoreTrue,
                "show the registry changes without making them",
            ));
            p.add(Action::option(
                &["--no-restart"],
                "no_restart",
                Kind::StoreTrue,
                "no effect, kept for compatibility: the loader no longer restarts the GPU",
            ));
        }
        p.add(Action::option(
            &["--json"],
            "json",
            Kind::StoreTrue,
            "machine-readable output",
        ));
        commands.push((name, help, p));
    }
    root.add_commands("command", commands);
    root
}

/// The parsed command line.
struct Args {
    command: String,
    json: bool,
    gpu: Option<i128>,
    pci: Option<String>,
    dry_run: bool,
    /// str(Path(payload)).
    payload: String,
}

impl Args {
    fn from(namespace: &Namespace) -> Args {
        let text = |dest: &str| match namespace.get(dest) {
            Some(Val::Text(text)) => Some(text.clone()),
            _ => None,
        };
        let flag = |dest: &str| namespace.get(dest) == Some(&Val::Bool(true));
        Args {
            command: text("command").unwrap_or_default(),
            json: flag("json"),
            gpu: match namespace.get("gpu") {
                Some(Val::Int(value)) => Some(*value),
                _ => None,
            },
            pci: text("pci"),
            dry_run: flag("dry_run"),
            payload: text("payload").unwrap_or_default(),
        }
    }
}

/// str(pathlib.Path(text)) as Python 3.12 prints it, for Windows (`windows`) or POSIX paths: separators collapsed
/// ('/' becomes '\' on Windows), '.' components and a trailing separator dropped, '.' for an empty path. The CLI
/// reads the payload from this text and shows it in "cannot read ...", as cli.py did with `type=Path`.
pub fn path_text(text: &str, windows: bool) -> String {
    if text.is_empty() {
        return ".".to_string();
    }
    let sep = if windows { '\\' } else { '/' };
    let path: String = if windows {
        text.replace('/', "\\")
    } else {
        text.to_string()
    };
    let (drive, mut root, rel) = if windows {
        nt_splitroot(&path)
    } else {
        posix_splitroot(&path)
    };
    if windows && root.is_empty() && drive.starts_with(sep) && !drive.ends_with(sep) {
        // \\server\share (or \\?\UNC\server\share) without a separator after it still has a root.
        let parts: Vec<&str> = drive.split(sep).collect();
        if (parts.len() == 4 && !"?.".contains(parts[2])) || parts.len() == 6 {
            root = "\\".to_string();
        }
    }
    let tail: Vec<&str> = rel.split(sep).filter(|part| !part.is_empty() && *part != ".").collect();
    let joined = tail.join(&sep.to_string());
    let shown = if !drive.is_empty() || !root.is_empty() {
        format!("{drive}{root}{joined}")
    } else if windows && tail.first().is_some_and(|first| !nt_splitroot(first).0.is_empty()) {
        // A first part such as "c:x" would read as a drive.
        format!(".\\{joined}")
    } else {
        joined
    };
    if shown.is_empty() {
        ".".to_string()
    } else {
        shown
    }
}

/// posixpath.splitroot: ("", root, rest) with a root of "", "/" or exactly "//".
fn posix_splitroot(path: &str) -> (String, String, String) {
    if !path.starts_with('/') {
        (String::new(), String::new(), path.to_string())
    } else if !path[1..].starts_with('/') || path[2..].starts_with('/') {
        (String::new(), "/".to_string(), path[1..].to_string())
    } else {
        (String::new(), "//".to_string(), path[2..].to_string())
    }
}

/// ntpath.splitroot of a path whose separators are already '\': (drive, root, rest).
fn nt_splitroot(path: &str) -> (String, String, String) {
    let chars: Vec<char> = path.chars().collect();
    let text = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
    let n = chars.len();
    if chars.first() == Some(&'\\') {
        if chars.get(1) == Some(&'\\') {
            // UNC drives (\\server\share, \\?\UNC\server\share) and device drives (\\.\device, \\?\device).
            let head: String = chars.iter().take(8).flat_map(|c| c.to_uppercase()).collect();
            let start = if head == "\\\\?\\UNC\\" { 8 } else { 2 };
            let find = |from: usize| (from..n).find(|&i| chars[i] == '\\');
            let Some(index) = find(start) else {
                return (path.to_string(), String::new(), String::new());
            };
            let Some(index2) = find(index + 1) else {
                return (path.to_string(), String::new(), String::new());
            };
            return (text(0..index2), text(index2..index2 + 1), text(index2 + 1..n));
        }
        return (String::new(), text(0..1), text(1..n));
    }
    if chars.get(1) == Some(&':') {
        if chars.get(2) == Some(&'\\') {
            return (text(0..2), text(2..3), text(3..n));
        }
        return (text(0..2), String::new(), text(2..n));
    }
    (String::new(), String::new(), path.to_string())
}

/// exc.strerror of an OSError for this error, as the Python loader showed it in "cannot read ...": the C library
/// text of the error number (on Windows, the C runtime's text for the errno it maps the Windows error to), without
/// Rust's " (os error N)".
pub fn os_error_text(error: &io::Error) -> String {
    match error.raw_os_error() {
        #[cfg(windows)]
        Some(code) => crt_strerror(crt_errno(code)).to_string(),
        #[cfg(not(windows))]
        Some(code) => {
            let text = error.to_string();
            text.strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text)
                .to_string()
        }
        None => error.to_string(),
    }
}

/// The errno that the Microsoft C runtime gives a Windows error (_dosmaperr), which Python's open() reports.
#[cfg(any(windows, test))]
fn crt_errno(code: i32) -> i32 {
    const ENOEXEC: i32 = 8;
    const EACCES: i32 = 13;
    match code {
        2 | 3 | 15 | 18 | 53 | 67 | 161 | 206 => 2, // ENOENT
        4 => 24,                                    // EMFILE
        5 | 16 | 33 | 65 | 82 | 83 | 108 | 132 | 158 | 167 => EACCES,
        6 | 114 | 130 => 9,   // EBADF
        7..=9 | 1816 => 12,   // ENOMEM
        10 => 7,              // E2BIG
        11 => ENOEXEC,        // ENOEXEC
        17 => 18,             // EXDEV
        80 | 183 => 17,       // EEXIST
        89 | 164 | 215 => 11, // EAGAIN
        109 => 32,            // EPIPE
        112 => 28,            // ENOSPC
        128 | 129 => 10,      // ECHILD
        145 => 41,            // ENOTEMPTY
        1113 => 42,           // EILSEQ
        19..=36 => EACCES,    // ERROR_WRITE_PROTECT .. ERROR_SHARING_BUFFER_EXCEEDED
        188..=202 => ENOEXEC, // the executable format errors
        _ => 22,              // EINVAL, also for 1, 12, 13, 87, 131
    }
}

/// The Microsoft C runtime's strerror text of the errno values that `crt_errno` gives.
#[cfg(any(windows, test))]
fn crt_strerror(errno: i32) -> &'static str {
    match errno {
        2 => "No such file or directory",
        7 => "Arg list too long",
        8 => "Exec format error",
        9 => "Bad file descriptor",
        10 => "No child processes",
        11 => "Resource temporarily unavailable",
        12 => "Not enough space",
        13 => "Permission denied",
        17 => "File exists",
        18 => "Improper link",
        24 => "Too many open files",
        28 => "No space left on device",
        32 => "Broken pipe",
        41 => "Directory not empty",
        42 => "Illegal byte sequence",
        _ => "Invalid argument",
    }
}

/// shutil.get_terminal_size().columns, the width argparse formats for: the COLUMNS variable when Python's int()
/// reads it as a positive number, else the width of the console (None: not a console), else 80.
pub fn terminal_columns(columns_variable: Option<&str>, console: impl FnOnce() -> Option<i128>) -> i128 {
    match columns_variable.and_then(|text| parse_int(text, 10)) {
        Some(columns) if columns > 0 => columns,
        _ => match console() {
            Some(columns) if columns != 0 => columns,
            _ => 80,
        },
    }
}

// The width of the console window that stdout writes to (os.get_terminal_size), when stdout is a console.
#[cfg(windows)]
use crate::win32::console_columns;

/// Python asks the terminal for its size; without a C library binding here, other systems use the fallback.
#[cfg(not(windows))]
fn console_columns() -> Option<i128> {
    None
}

fn system_columns() -> i128 {
    let variable = std::env::var("COLUMNS").ok();
    terminal_columns(variable.as_deref(), console_columns)
}

static NULL: Value = Value::Null;

/// `mapping[key]` of a report (null for a key that is missing, which the reports never have).
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&NULL)
}

/// f"{value}" for the values the reports hold.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Int(number) => number.to_string(),
        Value::Str(text) => text.clone(),
        other => other.to_json_pretty(),
    }
}

/// `value or default`.
fn or(value: &Value, default: &str) -> String {
    if value.truthy() {
        py_str(value)
    } else {
        default.to_string()
    }
}

fn override_lines(override_: &Value) -> Vec<String> {
    let vbios = field(override_, "vbios");
    let shown = if vbios.is_null() {
        "none".to_string()
    } else if !field(vbios, "size").is_null() {
        format!(
            "vbios {} bytes, sha256 {}",
            py_str(field(vbios, "size")),
            py_str(field(vbios, "sha256"))
        )
    } else {
        format!("vbios has an unexpected type ({})", py_str(field(vbios, "type")))
    };
    let saved = field(override_, "saved_previous_enable_gpu_firmware");
    let saved = if saved.truthy() {
        format!(" (saved previous value: {})", py_str(saved))
    } else {
        String::new()
    };
    let mut lines = vec![
        format!("  override: {shown}"),
        format!(
            "  EnableGpuFirmware: {}{saved}",
            or(field(override_, "enable_gpu_firmware"), "not set")
        ),
    ];
    if field(override_, "gsp_forced_by_old_loader").truthy() {
        lines.push(
            "  GSP firmware mode: on, set by loader 0.3.2 or older; load the file again with this version (or \
             unload it) and restart Windows."
                .to_string(),
        );
    }
    let records = field(override_, "rom_override_records");
    if records.truthy() {
        let count = records.as_array().map_or(0, Vec::len);
        lines.push(format!("  romOverride records: {count}"));
    }
    lines
}

fn gpu_lines(gpu: &Value) -> Vec<String> {
    let subsystem = field(gpu, "subsystem");
    let subsystem = if subsystem.truthy() {
        format!(" subsystem {}", py_str(subsystem))
    } else {
        String::new()
    };
    let gate = if field(gpu, "driver_supported").truthy() {
        String::new()
    } else {
        format!(" (too old: {}.{} or newer is required)", MIN_DRIVER.0, MIN_DRIVER.1)
    };
    let mut lines = vec![
        format!("GPU {}: {}", py_str(field(gpu, "index")), py_str(field(gpu, "name"))),
        format!(
            "  PCI {}, ID {}{subsystem}, {}",
            or(field(gpu, "location"), "unknown location"),
            py_str(field(gpu, "pci_id")),
            py_str(field(gpu, "family"))
        ),
        format!(
            "  driver {} ({}){gate}",
            or(field(gpu, "nvidia_driver_version"), "unknown"),
            or(field(gpu, "driver_version"), "unknown")
        ),
    ];
    let driver_key = field(gpu, "driver_key");
    if driver_key.truthy() {
        lines.push(format!("  driver key: {}", py_str(driver_key)));
    }
    lines
}

fn human_status(report: &[Value]) -> String {
    if report.is_empty() {
        return "No NVIDIA GPU found.".to_string();
    }
    let mut lines = Vec::new();
    for gpu in report {
        lines.extend(gpu_lines(gpu));
        let error = field(gpu, "error");
        if error.truthy() {
            lines.push(format!("  {}", py_str(error)));
        } else {
            lines.extend(override_lines(field(gpu, "override")));
        }
    }
    lines.join("\n")
}

/// One registry change of a report, for example "set EnableGpuFirmware = REG_DWORD 0x12" (cli.step_text).
pub fn step_text(step: &Value) -> String {
    let mut detail = if step.get("size").is_some() {
        format!(
            "{} bytes, sha256 {}",
            py_str(field(step, "size")),
            py_str(field(step, "sha256"))
        )
    } else {
        step.get("value").map_or_else(String::new, py_str)
    };
    let length = detail.chars().count();
    if length > 100 {
        // A long saved value; --json keeps all of it.
        let start: String = detail.chars().take(80).collect();
        detail = format!("{start}... ({length} characters)");
    }
    let op = field(step, "op");
    let head = format!("{} {}", py_str(op), py_str(field(step, "name")));
    if op.as_str() == Some("set") {
        format!("{head} = {} {detail}", py_str(field(step, "type")))
    } else {
        head
    }
}

fn human_change(report: &Value) -> String {
    let mut lines = gpu_lines(field(report, "gpu"));
    let payload = field(report, "payload");
    if payload.truthy() {
        lines.push(format!(
            "  payload: {}, {} bytes stored (file {} bytes; {} leading and {} trailing bytes not stored)",
            py_str(field(payload, "family")),
            py_str(field(payload, "size")),
            py_str(field(payload, "file_size")),
            py_str(field(payload, "dropped_prefix")),
            py_str(field(payload, "dropped_trailing"))
        ));
    }
    let dry_run = field(report, "dry_run").truthy();
    let heading = if dry_run {
        "Planned registry changes:"
    } else {
        "Registry changes:"
    };
    lines.push(heading.to_string());
    if let Some(steps) = field(report, "steps").as_array() {
        lines.extend(steps.iter().map(|step| format!("  {}", step_text(step))));
    }
    let message = field(report, "message");
    if message.truthy() {
        lines.push(py_str(message));
    }
    let next_step = field(report, "next_step");
    if dry_run {
        lines.push("Dry run: nothing was written.".to_string());
    } else if next_step.truthy() {
        lines.push(py_str(next_step));
    }
    lines.join("\n")
}

fn image_text(image: &Value) -> String {
    format!(
        "{} bytes, sha256 {}",
        py_str(field(image, "size")),
        py_str(field(image, "sha256"))
    )
}

fn human_verify(report: &Value) -> String {
    let readback = field(report, "readback");
    let configured = field(report, "configured");
    let mut lines = gpu_lines(field(report, "gpu"));
    let truncated = if field(readback, "truncated").truthy() {
        format!(" (NvAPI returns at most {IMAGE_SIZE} bytes)")
    } else {
        String::new()
    };
    lines.push(format!("  running VBIOS (NvAPI): {}{truncated}", image_text(readback)));
    let second = field(report, "second_readback");
    if second.truthy() {
        lines.push(format!("  second read: {}", image_text(second)));
    }
    let stored = if configured.truthy() {
        image_text(configured)
    } else {
        "none".to_string()
    };
    lines.push(format!("  stored override: {stored}"));
    lines.push(py_str(field(report, "message")));
    lines.join("\n")
}

/// print(text): the text and a newline. Write errors are ignored (argparse ignores them too).
fn print(out: &mut dyn Write, text: &str) {
    let _ = writeln!(out, "{text}");
}

/// `dict(report, ok=ok)` printed with json.dumps(indent=2).
fn with_ok(mut report: Map, ok: bool) -> String {
    report.insert("ok", ok);
    Value::Object(report).to_json_pretty()
}

/// The command after parsing (the try block of cli.main): the exit code, or the error to report.
fn execute(args: &Args, platform: Option<Arc<dyn Platform>>, out: &mut dyn Write) -> Result<i32> {
    let platform = match platform {
        Some(platform) => platform,
        None => windows_platform()?,
    };
    let selection = Selection {
        gpu: args.gpu,
        pci: args.pci.clone(),
        instance_id: None,
    };
    match args.command.as_str() {
        "status" => {
            let report = status(&*platform)?;
            let text = if args.json {
                Value::Array(report).to_json_pretty()
            } else {
                human_status(&report)
            };
            print(out, &text);
            Ok(0)
        }
        "verify" => {
            let report = verify(&*platform, &selection)?;
            let state = report.get("state").and_then(Value::as_str).unwrap_or("");
            let ok = VERIFY_OK.contains(&state);
            let text = if args.json {
                with_ok(report, ok)
            } else {
                human_verify(&Value::Object(report))
            };
            print(out, &text);
            Ok(if ok { 0 } else { 4 })
        }
        command => {
            // --no-restart has no effect.
            let report = if command == "load" {
                let data = std::fs::read(&args.payload)
                    .map_err(|error| Error::new(format!("cannot read {}: {}", args.payload, os_error_text(&error))))?;
                load(&*platform, &data, &selection, args.dry_run)?
            } else {
                unload(&*platform, &selection, args.dry_run)?
            };
            let text = if args.json {
                with_ok(report, true)
            } else {
                human_change(&Value::Object(report))
            };
            print(out, &text);
            Ok(0)
        }
    }
}

/// Run the command line with these arguments (without the program name), writing to `out` and `err`; returns the
/// exit code. `platform` None means `windows_platform()`. Testable on any system.
pub fn run(args: &[String], platform: Option<Arc<dyn Platform>>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_with_columns(args, platform, out, err, &system_columns)
}

/// `run` with the terminal width that help, usage and error texts are formatted for (see `terminal_columns`)
/// taken from `columns` instead of the COLUMNS variable and the console: for tests and the comparison tool.
pub fn run_with_columns(
    args: &[String],
    platform: Option<Arc<dyn Platform>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    columns: &dyn Fn() -> i128,
) -> i32 {
    let parsed = {
        let mut io = Io {
            out: &mut *out,
            err: &mut *err,
            columns,
        };
        parser().parse_args(args, &mut io)
    };
    let args = match parsed {
        Ok(namespace) => Args::from(&namespace),
        Err(code) => return code,
    };
    match execute(&args, platform, out) {
        Ok(code) => code,
        Err(error) => {
            if args.json {
                let mut report = Map::new();
                report.insert("ok", false);
                report.insert("error", error.message());
                print(out, &Value::Object(report).to_json_pretty());
            }
            let _ = writeln!(err, "{PROG}: {error}");
            1
        }
    }
}

/// The line end of Python's text streams: "\r\n" on Windows.
const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// sys.stdout and sys.stderr of Python: each "\n" is written as `newline` ("\r\n" on Windows).
struct Newlines<W: Write> {
    inner: W,
    newline: &'static str,
}

impl<W: Write> Write for Newlines<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.newline == "\n" {
            return self.inner.write(buf);
        }
        let mut start = 0;
        for (index, byte) in buf.iter().enumerate() {
            if *byte == b'\n' {
                self.inner.write_all(&buf[start..index])?;
                self.inner.write_all(self.newline.as_bytes())?;
                start = index + 1;
            }
        }
        self.inner.write_all(&buf[start..])?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The message of a panic payload (`&str` or `String`).
pub(crate) fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "Box<dyn Any>".to_string()
    }
}

/// The exit code of `work`, or 70 when it panics (the panic hook has printed the message).
fn guarded(work: impl FnOnce() -> i32) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or(70)
}

/// The process entry of vbios-loader.exe: OS arguments, stdout and stderr, the Windows platform. A panic prints
/// "vbios-loader: unexpected error: ..." and returns 70; Ctrl+C exits with 130.
pub fn main() -> i32 {
    #[cfg(windows)]
    crate::win32::exit_on_interrupt();
    std::panic::set_hook(Box::new(|info| {
        let _ = write!(
            io::stderr(),
            "{PROG}: unexpected error: {}{NEWLINE}",
            panic_text(info.payload())
        );
    }));
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let mut out = Newlines {
        inner: io::stdout(),
        newline: NEWLINE,
    };
    let mut err = Newlines {
        inner: io::stderr(),
        newline: NEWLINE,
    };
    let code = guarded(|| run(&args, None, &mut out, &mut err));
    let _ = out.flush();
    let _ = err.flush();
    code
}

#[cfg(test)]
mod tests;
