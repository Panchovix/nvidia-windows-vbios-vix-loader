//! Texts printed before any window exists (`--help`, `--version`, argument errors, a failed start). The program is
//! built for the GUI subsystem, so it has no console of its own: output redirected to a file or a pipe goes
//! there; otherwise the text goes to the console of the program that started it (cmd, PowerShell); with no
//! console at all (started from Explorer or a shortcut), it is shown in a message box.

use std::io::Write;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileType, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, FILE_TYPE_PIPE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AttachConsole, FreeConsole, GetConsoleScreenBufferInfo, GetStdHandle, WriteConsoleW, ATTACH_PARENT_PROCESS,
    CONSOLE_SCREEN_BUFFER_INFO, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK};

use super::args::{columns_from_env, DEFAULT_COLUMNS};
use super::text::wide;
use super::{Stream, PROG};

/// Print `text(columns)` to the standard stream, the parent's console or a message box, in that order.
pub fn emit(stream: Stream, text: &dyn Fn(i64) -> String) {
    let env_columns = columns_from_env(std::env::var("COLUMNS").ok().as_deref());
    if write_redirected(stream, &|| text(env_columns.unwrap_or(DEFAULT_COLUMNS))) {
        return;
    }
    if let Some(console) = ParentConsole::attach() {
        let columns = env_columns.or_else(|| console.columns()).unwrap_or(DEFAULT_COLUMNS);
        if console.write(&text(columns)) {
            return;
        }
    }
    let body = wide(&text(env_columns.unwrap_or(DEFAULT_COLUMNS)));
    let title = wide(PROG);
    let icon = match stream {
        Stream::Out => MB_ICONINFORMATION,
        Stream::Err => MB_ICONERROR,
    };
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe { MessageBoxW(null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

/// When the standard stream is a file or a pipe (`> out.txt`, `| more`, CI redirection), write there with CRLF
/// line ends, as Python's text-mode stdout did on Windows. False when it is not redirected or the write failed.
fn write_redirected(stream: Stream, text: &dyn Fn() -> String) -> bool {
    let id = match stream {
        Stream::Out => STD_OUTPUT_HANDLE,
        Stream::Err => STD_ERROR_HANDLE,
    };
    // SAFETY: GetStdHandle only reads the process parameters.
    let handle = unsafe { GetStdHandle(id) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: GetFileType accepts any handle value and only queries it.
    let kind = unsafe { GetFileType(handle) };
    if kind != FILE_TYPE_DISK && kind != FILE_TYPE_PIPE {
        return false;
    }
    let bytes = text().replace('\n', "\r\n").into_bytes();
    let written = match stream {
        Stream::Out => {
            let mut out = std::io::stdout();
            out.write_all(&bytes).and_then(|()| out.flush())
        }
        Stream::Err => {
            let mut err = std::io::stderr();
            err.write_all(&bytes).and_then(|()| err.flush())
        }
    };
    written.is_ok()
}

/// The console of the parent process, attached for one write.
struct ParentConsole {
    output: HANDLE,
}

impl ParentConsole {
    fn attach() -> Option<ParentConsole> {
        // SAFETY: plain call; it fails when the parent has no console or this process already has one.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return None;
        }
        let name = wide("CONOUT$");
        // SAFETY: opens the attached console's screen buffer; the name is NUL-terminated.
        let output = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if output == INVALID_HANDLE_VALUE || output.is_null() {
            // SAFETY: detaches the console attached above.
            unsafe { FreeConsole() };
            return None;
        }
        Some(ParentConsole { output })
    }

    /// The console window's width, as Python's os.get_terminal_size reads it.
    fn columns(&self) -> Option<i64> {
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        // SAFETY: `output` is an open console handle with read access; `info` is a valid out-pointer.
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
            return None;
        }
        let columns = i64::from(info.srWindow.Right) - i64::from(info.srWindow.Left) + 1;
        (columns > 0).then_some(columns)
    }

    fn write(&self, text: &str) -> bool {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut done = 0;
        while done < units.len() {
            let chunk = &units[done..units.len().min(done + 16 * 1024)];
            let mut written = 0u32;
            // SAFETY: `chunk` is valid for its length; `written` is a valid out-pointer.
            let ok = unsafe { WriteConsoleW(self.output, chunk.as_ptr(), chunk.len() as u32, &mut written, null()) };
            if ok == 0 || written == 0 {
                return false;
            }
            done += written as usize;
        }
        true
    }
}

impl Drop for ParentConsole {
    fn drop(&mut self) {
        // SAFETY: closes the handle opened in `attach` and detaches from the console attached there.
        unsafe {
            CloseHandle(self.output);
            FreeConsole();
        }
    }
}
