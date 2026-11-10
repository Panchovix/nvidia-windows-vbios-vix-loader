//! Small text helpers of the window that need no Windows: UTF-16 strings, the Messages text, the file dialog
//! filter and the "Unexpected error" line.

use std::any::Any;

/// A NUL-terminated UTF-16 string for Win32. A NUL inside the text would end it early, so it becomes U+FFFD.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16()
        .map(|unit| if unit == 0 { 0xFFFD } else { unit })
        .chain(Some(0))
        .collect()
}

/// UTF-16 to a String (lossy), up to the first NUL.
pub fn from_wide(units: &[u16]) -> String {
    let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// What one `log` call appends to the Messages area: gui.py's `'\n'.join(lines) + '\n'`, with the CRLF line
/// ends a Windows edit control needs (a lone CR or LF inside a line also becomes CRLF).
pub fn log_text(lines: &[String]) -> String {
    let joined = lines.join("\n") + "\n";
    let normalized = joined.replace("\r\n", "\n").replace('\r', "\n");
    normalized.replace('\n', "\r\n")
}

/// The text of the result message box: the lines separated by a blank line.
pub fn result_text(lines: &[String]) -> String {
    lines.join("\n\n")
}

/// The window title: "<title> <version>".
pub fn window_title(title: &str) -> String {
    format!("{title} {}", crate::VERSION)
}

/// `text('unexpected').format(message=message)`.
pub fn unexpected(template: &str, message: &str) -> String {
    template.replace("{message}", message)
}

/// The text of a caught panic (the command line's `panic_text`).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    crate::cli::panic_text(payload)
}

/// The "Unexpected error" line of a caught panic: "panic: <message>" in place of Python's "<Type>: <message>",
/// as `controller::Job::run` writes it.
pub fn unexpected_panic(template: &str, payload: &(dyn Any + Send)) -> String {
    unexpected(template, &format!("panic: {}", panic_message(payload)))
}

/// The GetOpenFileNameW filter: "<file_types> (*.rom;*.bin)" and "<all_files> (*.*)", each a NUL-terminated
/// (name, pattern) pair, with a final NUL.
pub fn file_filter(file_types: &str, all_files: &str) -> Vec<u16> {
    let mut filter = Vec::new();
    for part in [
        format!("{file_types} (*.rom;*.bin)"),
        "*.rom;*.bin".to_string(),
        format!("{all_files} (*.*)"),
        "*.*".to_string(),
    ] {
        filter.extend(part.encode_utf16().filter(|unit| *unit != 0));
        filter.push(0);
    }
    filter.push(0);
    filter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_end_with_one_nul() {
        assert_eq!(wide("Ñu"), [0xD1, 0x75, 0]);
        assert_eq!(wide("a\0b"), [0x61, 0xFFFD, 0x62, 0]);
        assert_eq!(from_wide(&[0x61, 0x62, 0, 0x63]), "ab");
        assert_eq!(from_wide(&[0x61]), "a");
    }

    #[test]
    fn messages_get_crlf_line_ends() {
        let lines = vec!["GPU 0: VBIOS loaded.".to_string(), "Restart\nWindows.".to_string()];
        assert_eq!(log_text(&lines), "GPU 0: VBIOS loaded.\r\nRestart\r\nWindows.\r\n");
        assert_eq!(log_text(&["a\r\nb\rc".to_string()]), "a\r\nb\r\nc\r\n");
        assert_eq!(result_text(&lines), "GPU 0: VBIOS loaded.\n\nRestart\nWindows.");
    }

    #[test]
    fn titles_and_unexpected_errors() {
        assert_eq!(
            window_title("Windows VBIOS Vix Loader"),
            format!("Windows VBIOS Vix Loader {}", crate::VERSION)
        );
        assert_eq!(
            unexpected("Unexpected error: {message}", "boom"),
            "Unexpected error: boom"
        );
        // The payloads panic!() makes: &str for a literal, String for a formatted message.
        let literal: Box<dyn Any + Send> = Box::new("boom");
        let formatted: Box<dyn Any + Send> = Box::new(format!("index {} out of range", 3));
        let other: Box<dyn Any + Send> = Box::new(7u8);
        assert_eq!(panic_message(&*literal), "boom");
        assert_eq!(panic_message(&*formatted), "index 3 out of range");
        assert_eq!(panic_message(&*other), "Box<dyn Any>");
        assert_eq!(
            unexpected_panic("Unexpected error: {message}", &*literal),
            "Unexpected error: panic: boom"
        );
    }

    #[test]
    fn file_filter_is_a_double_nul_list() {
        let filter = file_filter("VBIOS files", "All files");
        let text = String::from_utf16_lossy(&filter);
        assert_eq!(text, "VBIOS files (*.rom;*.bin)\0*.rom;*.bin\0All files (*.*)\0*.*\0\0");
    }
}
