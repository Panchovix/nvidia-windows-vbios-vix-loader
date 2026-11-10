//! VBIOS_LOADER_GUI_SMOKE=1, used by CI for a screenshot: the window stays on top, switches to the other language
//! after 3 s and to the contrasting theme after 5 s (so the screenshot shows both), and closes after
//! SMOKE_SECONDS. The exit code is 1 when a window handler failed, else 0.

use std::ffi::OsStr;

use crate::controller::{Language, Theme};

/// What the window does at each moment of the smoke run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Language(Language),
    Theme(Theme),
    Close,
}

/// True when the variable is exactly "1".
pub fn enabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// The first language that is not `current` (gui.py: next code in LANGUAGES that differs).
pub fn other_language(current: Language) -> Language {
    Language::ALL
        .into_iter()
        .find(|language| *language != current)
        .unwrap_or(current)
}

/// Light when the resolved theme is dark, else dark.
pub fn contrast(resolved: Theme) -> Theme {
    if resolved == Theme::Dark {
        Theme::Light
    } else {
        Theme::Dark
    }
}

/// (milliseconds after the window opened, step), computed when the window opens like gui.py did.
pub fn schedule(language: Language, resolved: Theme) -> [(u32, Step); 3] {
    [
        (3000, Step::Language(other_language(language))),
        (5000, Step::Theme(contrast(resolved))),
        (super::SMOKE_SECONDS * 1000, Step::Close),
    ]
}

/// The process exit code: 1 only in smoke mode after a handler failed.
pub fn exit_code(smoke: bool, failed: bool) -> i32 {
    i32::from(smoke && failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_switches_language_then_theme_then_closes() {
        assert_eq!(
            schedule(Language::En, Theme::Light),
            [
                (3000, Step::Language(Language::Es)),
                (5000, Step::Theme(Theme::Dark)),
                (20000, Step::Close)
            ]
        );
        assert_eq!(
            schedule(Language::Es, Theme::Dark),
            [
                (3000, Step::Language(Language::En)),
                (5000, Step::Theme(Theme::Light)),
                (20000, Step::Close)
            ]
        );
        assert_eq!(contrast(Theme::Light), Theme::Dark);
        assert_eq!(contrast(Theme::Dark), Theme::Light);
    }

    #[test]
    fn smoke_mode_and_exit_code() {
        assert!(enabled(Some(OsStr::new("1"))));
        assert!(!enabled(Some(OsStr::new("true"))));
        assert!(!enabled(Some(OsStr::new(" 1"))));
        assert!(!enabled(None));
        assert_eq!(
            (exit_code(true, true), exit_code(true, false), exit_code(false, true)),
            (1, 0, 0)
        );
    }
}
