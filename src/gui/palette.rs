//! The window's colors: gui.py's two palettes, and the colors each part is painted with.
//!
//! Light keeps the native look (system colors for the window, native buttons and lists) and uses the palette
//! for the banner, the validation line and the Messages area, as gui.py did. Dark paints everything from the
//! dark palette.

/// An sRGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// From 0xRRGGBB.
    pub const fn hex(value: u32) -> Rgb {
        Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    /// The Win32 COLORREF, 0x00BBGGRR.
    pub const fn colorref(self) -> u32 {
        self.0 as u32 | (self.1 as u32) << 8 | (self.2 as u32) << 16
    }

    /// From a Win32 COLORREF (GetSysColor).
    pub const fn from_colorref(value: u32) -> Rgb {
        Rgb(value as u8, (value >> 8) as u8, (value >> 16) as u8)
    }

    /// `self` moved `amount`/255 of the way towards `other`.
    pub fn mix(self, other: Rgb, amount: u8) -> Rgb {
        let channel = |a: u8, b: u8| {
            let (a, b, t) = (u32::from(a), u32::from(b), u32::from(amount));
            ((a * (255 - t) + b * t + 127) / 255) as u8
        };
        Rgb(
            channel(self.0, other.0),
            channel(self.1, other.1),
            channel(self.2, other.2),
        )
    }
}

/// gui.py's PALETTES entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub ok: Rgb,
    pub bad: Rgb,
    pub banner_bg: Rgb,
    pub banner_fg: Rgb,
    pub text_bg: Rgb,
    pub text_fg: Rgb,
    pub select: Rgb,
    pub select_fg: Rgb,
    /// Only the dark palette paints the rest of the window.
    pub dark: Option<DarkPalette>,
}

/// The colors gui.py gave ttk's clam theme in the dark palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DarkPalette {
    pub bg: Rgb,
    pub field: Rgb,
    pub fg: Rgb,
    pub border: Rgb,
    pub button: Rgb,
    pub active: Rgb,
    pub pressed: Rgb,
    pub disabled: Rgb,
}

pub const LIGHT: Palette = Palette {
    ok: Rgb::hex(0x176317),
    bad: Rgb::hex(0x9b1c1c),
    banner_bg: Rgb::hex(0xfde2e1),
    banner_fg: Rgb::hex(0x7a1010),
    text_bg: Rgb::hex(0xffffff),
    text_fg: Rgb::hex(0x000000),
    select: Rgb::hex(0x0078d7),
    select_fg: Rgb::hex(0xffffff),
    dark: None,
};

pub const DARK: Palette = Palette {
    ok: Rgb::hex(0x7fd88f),
    bad: Rgb::hex(0xff8a80),
    banner_bg: Rgb::hex(0x4a1f1f),
    banner_fg: Rgb::hex(0xffd8d4),
    text_bg: Rgb::hex(0x1b1b1b),
    text_fg: Rgb::hex(0xe8e8e8),
    select: Rgb::hex(0x1f5fa8),
    select_fg: Rgb::hex(0xffffff),
    dark: Some(DarkPalette {
        bg: Rgb::hex(0x202020),
        field: Rgb::hex(0x2b2b2b),
        fg: Rgb::hex(0xe8e8e8),
        border: Rgb::hex(0x454545),
        button: Rgb::hex(0x333333),
        active: Rgb::hex(0x404040),
        pressed: Rgb::hex(0x4a4a4a),
        disabled: Rgb::hex(0x7a7a7a),
    }),
};

/// The Windows colors the light theme follows (GetSysColor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemColors {
    /// COLOR_BTNFACE: the window background.
    pub face: Rgb,
    /// COLOR_WINDOWTEXT: labels and section titles.
    pub text: Rgb,
    /// COLOR_WINDOW: list and field backgrounds.
    pub window: Rgb,
    /// COLOR_GRAYTEXT: disabled text.
    pub gray: Rgb,
}

impl SystemColors {
    /// The usual Windows 10/11 values, for tests.
    pub const DEFAULT: SystemColors = SystemColors {
        face: Rgb::hex(0xf0f0f0),
        text: Rgb::hex(0x000000),
        window: Rgb::hex(0xffffff),
        gray: Rgb::hex(0x6d6d6d),
    };
}

/// What each part of the window is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub dark: bool,
    /// Window background, behind labels and section frames.
    pub bg: Rgb,
    /// Labels and section titles.
    pub fg: Rgb,
    /// Section frames.
    pub frame: Rgb,
    /// The read-only path field.
    pub field: Rgb,
    pub field_fg: Rgb,
    /// The Messages area.
    pub log_bg: Rgb,
    pub log_fg: Rgb,
    pub banner_bg: Rgb,
    pub banner_fg: Rgb,
    /// Validation line: accepted or refused.
    pub ok: Rgb,
    pub bad: Rgb,
    /// Owner-drawn buttons (dark): face, pressed face, border, disabled text, keyboard focus.
    pub button: Rgb,
    pub pressed: Rgb,
    pub border: Rgb,
    pub disabled_fg: Rgb,
    pub focus: Rgb,
    /// The GPU list and its header.
    pub list_bg: Rgb,
    pub list_fg: Rgb,
    pub header_bg: Rgb,
    pub header_fg: Rgb,
}

/// The colors of the light (`dark` false) or dark theme.
pub fn colors(dark: bool, system: SystemColors) -> Colors {
    match (dark, DARK.dark) {
        (true, Some(d)) => Colors {
            dark: true,
            bg: d.bg,
            fg: d.fg,
            frame: d.border,
            field: d.field,
            field_fg: d.fg,
            log_bg: DARK.text_bg,
            log_fg: DARK.text_fg,
            banner_bg: DARK.banner_bg,
            banner_fg: DARK.banner_fg,
            ok: DARK.ok,
            bad: DARK.bad,
            button: d.button,
            pressed: d.pressed,
            border: d.border,
            disabled_fg: d.disabled,
            focus: DARK.select,
            list_bg: d.field,
            list_fg: d.fg,
            header_bg: d.button,
            header_fg: d.fg,
        },
        _ => Colors {
            dark: false,
            bg: system.face,
            fg: system.text,
            // A light line on the window color (#c8c8c8 on the usual #f0f0f0).
            frame: system.face.mix(system.text, 43),
            field: system.face,
            field_fg: system.text,
            log_bg: LIGHT.text_bg,
            log_fg: LIGHT.text_fg,
            banner_bg: LIGHT.banner_bg,
            banner_fg: LIGHT.banner_fg,
            ok: LIGHT.ok,
            bad: LIGHT.bad,
            button: system.face,
            pressed: system.face.mix(system.text, 25),
            border: system.face.mix(system.text, 110),
            disabled_fg: system.gray,
            focus: LIGHT.select,
            list_bg: system.window,
            list_fg: system.text,
            header_bg: system.window,
            header_fg: system.text,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Rgb {
        let value = u32::from_str_radix(text.trim_start_matches('#'), 16).unwrap_or(u32::MAX);
        Rgb::hex(value)
    }

    #[test]
    fn palettes_are_the_ones_of_gui_py() {
        // gui.py's PALETTES, copied as text.
        let light = [
            ("#176317", LIGHT.ok),
            ("#9b1c1c", LIGHT.bad),
            ("#fde2e1", LIGHT.banner_bg),
            ("#7a1010", LIGHT.banner_fg),
            ("#ffffff", LIGHT.text_bg),
            ("#000000", LIGHT.text_fg),
            ("#0078d7", LIGHT.select),
            ("#ffffff", LIGHT.select_fg),
        ];
        for (text, color) in light {
            assert_eq!(parse(text), color, "{text}");
        }
        assert!(LIGHT.dark.is_none());
        let dark = DARK.dark.unwrap_or(DarkPalette {
            bg: Rgb(0, 0, 0),
            field: Rgb(0, 0, 0),
            fg: Rgb(0, 0, 0),
            border: Rgb(0, 0, 0),
            button: Rgb(0, 0, 0),
            active: Rgb(0, 0, 0),
            pressed: Rgb(0, 0, 0),
            disabled: Rgb(0, 0, 0),
        });
        let dark_colors = [
            ("#7fd88f", DARK.ok),
            ("#ff8a80", DARK.bad),
            ("#4a1f1f", DARK.banner_bg),
            ("#ffd8d4", DARK.banner_fg),
            ("#1b1b1b", DARK.text_bg),
            ("#e8e8e8", DARK.text_fg),
            ("#1f5fa8", DARK.select),
            ("#ffffff", DARK.select_fg),
            ("#202020", dark.bg),
            ("#2b2b2b", dark.field),
            ("#e8e8e8", dark.fg),
            ("#454545", dark.border),
            ("#333333", dark.button),
            ("#404040", dark.active),
            ("#4a4a4a", dark.pressed),
            ("#7a7a7a", dark.disabled),
        ];
        for (text, color) in dark_colors {
            assert_eq!(parse(text), color, "{text}");
        }
    }

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(Rgb::hex(0x123456).colorref(), 0x0056_3412);
        assert_eq!(Rgb::from_colorref(0x0056_3412), Rgb::hex(0x123456));
        assert_eq!(Rgb(0, 0, 0).mix(Rgb(255, 255, 255), 255), Rgb(255, 255, 255));
        assert_eq!(Rgb(10, 20, 30).mix(Rgb(250, 0, 30), 0), Rgb(10, 20, 30));
    }

    #[test]
    fn each_theme_paints_with_its_palette() {
        let dark = colors(true, SystemColors::DEFAULT);
        assert!(dark.dark);
        assert_eq!(
            (dark.bg, dark.fg, dark.log_bg, dark.banner_bg),
            (
                Rgb::hex(0x202020),
                Rgb::hex(0xe8e8e8),
                Rgb::hex(0x1b1b1b),
                Rgb::hex(0x4a1f1f)
            )
        );
        assert_eq!(
            (dark.ok, dark.bad, dark.button, dark.pressed),
            (
                Rgb::hex(0x7fd88f),
                Rgb::hex(0xff8a80),
                Rgb::hex(0x333333),
                Rgb::hex(0x4a4a4a)
            )
        );
        assert_eq!(
            (dark.list_bg, dark.header_bg, dark.focus),
            (Rgb::hex(0x2b2b2b), Rgb::hex(0x333333), Rgb::hex(0x1f5fa8))
        );
        let light = colors(false, SystemColors::DEFAULT);
        assert!(!light.dark);
        assert_eq!(
            (light.bg, light.fg, light.log_bg, light.log_fg),
            (Rgb::hex(0xf0f0f0), Rgb::hex(0), Rgb::hex(0xffffff), Rgb::hex(0))
        );
        assert_eq!(
            (light.ok, light.bad, light.banner_bg, light.banner_fg),
            (LIGHT.ok, LIGHT.bad, LIGHT.banner_bg, LIGHT.banner_fg)
        );
        assert_eq!(light.frame, Rgb::hex(0xc8c8c8));
    }
}
