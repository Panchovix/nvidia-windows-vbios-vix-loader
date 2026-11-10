//! Where every part of the window goes, in pixels at the window's DPI: gui.py's grid, top to bottom, with 8 px
//! margins at 96 DPI. The menu bar, the not-admin banner (only without administrator rights), the "NVIDIA GPUs"
//! section (list, Refresh right-aligned under it), the "VBIOS file" section (button, path, validation line), the
//! actions row ("working" label, Load, Remove override, Verify) and the "Messages" section. The list and the
//! Messages area take their natural height (4 rows, 8 lines) plus the remaining height, shared 1:2 like the
//! grid weights in gui.py.

/// A rectangle in client pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// The rectangle `by` pixels smaller on every side.
    pub const fn inset(&self, by: i32) -> Rect {
        Rect::new(self.x + by, self.y + by, self.w - 2 * by, self.h - 2 * by)
    }
}

/// A frame drawn around a group of controls, with its title on the top edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// The frame; its top edge runs through the middle of the title.
    pub frame: Rect,
    /// Where the title text goes; the frame line is cleared `TITLE_PAD` pixels around it.
    pub title: Rect,
}

/// Lengths at 96 DPI.
const MARGIN: i32 = 8;
const BAR_TOP: i32 = 6;
const GAP: i32 = 6;
const PAD: i32 = 8;
const TITLE_GAP: i32 = 4;
const TITLE_INSET: i32 = 8;
/// Space cleared on the frame line on each side of a section title.
pub const TITLE_PAD: i32 = 3;
const BANNER_PAD_X: i32 = 10;
const BANNER_PAD_Y: i32 = 6;
const BUTTON_MIN_W: i32 = 75;
const BUTTON_MIN_H: i32 = 23;
const BUTTON_PAD: i32 = 12;
const MENU_PAD: i32 = 22;
/// The 3D border of a list or edit control (WS_EX_CLIENTEDGE), per side.
const EDGE: i32 = 2;
/// A vertical scroll bar, for the natural width only.
const SCROLL: i32 = 17;
const LIST_ROWS: i32 = 4;
const LOG_LINES: i32 = 8;
/// GPU list columns (#, Name, PCI ID, Location, Driver, Override), as in gui.py; Name and Override stretch.
pub const COLUMNS: [i32; 6] = [40, 230, 90, 80, 150, 300];
const COLUMN_MIN: i32 = 30;
const STRETCH: [usize; 2] = [1, 5];
/// The smallest client area, as gui.py's root.minsize(780, 540).
pub const MIN_CLIENT: (i32, i32) = (780, 540);

/// `px` at 96 DPI scaled to `dpi`, rounded.
pub fn scale(px: i32, dpi: u32) -> i32 {
    let dpi = i64::from(dpi.max(1));
    ((i64::from(px) * dpi + 48).div_euclid(96)) as i32
}

/// Width of each of the two lines of a client edge (SM_CXEDGE is 2 of them at 96 DPI): the dark theme paints the
/// edge of the edit controls itself, a border line and then a line of the control's background.
pub fn edge(dpi: u32) -> i32 {
    scale(1, dpi).max(1)
}

/// The texts that wrap to the available width; the caller measures them (DrawText with DT_WORDBREAK).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrapped {
    Banner,
    Validation,
}

/// Widths in pixels of the texts that size controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextWidths {
    pub language: i32,
    pub theme: i32,
    pub refresh: i32,
    pub file: i32,
    pub load: i32,
    pub remove: i32,
    pub verify: i32,
    pub gpus_title: i32,
    pub file_title: i32,
    pub messages_title: i32,
}

/// What the layout depends on.
pub struct Input<'a> {
    /// Client area size.
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
    /// Height of a line of the message font.
    pub line: i32,
    /// The not-admin banner is shown.
    pub banner: bool,
    pub texts: TextWidths,
    /// Height of a wrapped text at a width.
    pub wrap: &'a dyn Fn(Wrapped, i32) -> i32,
}

/// The place of every control and painted part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub language: Rect,
    pub theme: Rect,
    /// The banner's colored area (None when hidden) and its text inside the padding.
    pub banner: Option<Rect>,
    pub banner_text: Rect,
    pub gpus: Section,
    pub list: Rect,
    pub refresh: Rect,
    pub files: Section,
    pub file: Rect,
    pub path: Rect,
    pub validation: Rect,
    pub working: Rect,
    pub load: Rect,
    pub remove: Rect,
    pub verify: Rect,
    pub messages: Section,
    pub log: Rect,
    /// The client height at which the list shows 4 rows and the Messages area 8 lines.
    pub natural_height: i32,
}

/// Height of push buttons, the menu buttons and the path field.
pub fn button_height(line: i32, dpi: u32) -> i32 {
    scale(BUTTON_MIN_H, dpi).max(line + scale(8, dpi))
}

fn button_width(text: i32, dpi: u32) -> i32 {
    scale(BUTTON_MIN_W, dpi).max(text + 2 * scale(BUTTON_PAD, dpi))
}

/// Menu buttons are as wide as their text plus room on both sides, the right one holding the arrow.
fn menu_width(text: i32, dpi: u32) -> i32 {
    text + 2 * scale(MENU_PAD, dpi)
}

/// The down arrow of a menu button: a small triangle centered in the button's right padding.
pub fn menu_arrow(button: Rect, dpi: u32) -> [(i32, i32); 3] {
    let half = scale(4, dpi);
    let height = scale(4, dpi);
    let cx = button.right() - scale(12, dpi);
    let top = button.y + (button.h - height) / 2;
    [(cx - half, top), (cx + half, top), (cx, top + height)]
}

fn natural_list(line: i32, dpi: u32) -> i32 {
    let header = line + scale(9, dpi);
    let row = line + scale(4, dpi);
    header + LIST_ROWS * row + 2 * scale(EDGE, dpi)
}

fn minimum_list(line: i32, dpi: u32) -> i32 {
    line + scale(9, dpi) + line + scale(4, dpi) + 2 * scale(EDGE, dpi)
}

fn natural_log(line: i32, dpi: u32) -> i32 {
    LOG_LINES * line + 2 * scale(EDGE, dpi) + scale(4, dpi)
}

fn minimum_log(line: i32, dpi: u32) -> i32 {
    2 * line + 2 * scale(EDGE, dpi) + scale(4, dpi)
}

fn section(x: i32, top: i32, width: i32, height: i32, title_width: i32, line: i32, dpi: u32) -> Section {
    let middle = line / 2;
    Section {
        frame: Rect::new(x, top + middle, width, (height - middle).max(0)),
        title: Rect::new(x + scale(TITLE_INSET, dpi), top, title_width.max(0), line),
    }
}

/// Place everything in a client area of `input.width` x `input.height`.
pub fn compute(input: &Input) -> Layout {
    let dpi = input.dpi;
    let s = |px: i32| scale(px, dpi);
    let line = input.line.max(1);
    let (m, gap, pad) = (s(MARGIN), s(GAP), s(PAD));
    let button_h = button_height(line, dpi);
    let content_w = (input.width - 2 * m).max(0);
    let inner_w = (content_w - 2 * pad).max(0);
    let mut layout = Layout::default();

    let mut y = s(BAR_TOP);
    layout.language = Rect::new(m, y, menu_width(input.texts.language, dpi), button_h);
    layout.theme = Rect::new(
        layout.language.right() + gap,
        y,
        menu_width(input.texts.theme, dpi),
        button_h,
    );
    y += button_h;

    if input.banner {
        y += m;
        let text_w = (content_w - 2 * s(BANNER_PAD_X)).max(0);
        let text_h = (input.wrap)(Wrapped::Banner, text_w).max(line);
        let banner = Rect::new(m, y, content_w, text_h + 2 * s(BANNER_PAD_Y));
        layout.banner_text = Rect::new(m + s(BANNER_PAD_X), y + s(BANNER_PAD_Y), text_w, text_h);
        layout.banner = Some(banner);
        y += banner.h;
    }

    // Heights of everything but the list and the Messages area, which share the rest 1:2.
    let validation_h = (input.wrap)(Wrapped::Validation, inner_w).max(line);
    let head = line + s(TITLE_GAP);
    let gpus_fixed = head + gap + button_h + pad;
    let files_h = head + button_h + gap + validation_h + pad;
    let messages_fixed = head + pad;
    let fixed = y + m + gpus_fixed + m + files_h + m + button_h + m + messages_fixed + m;
    let (natural_list_h, natural_log_h) = (natural_list(line, dpi), natural_log(line, dpi));
    layout.natural_height = fixed + natural_list_h + natural_log_h;
    let extra = input.height - layout.natural_height;
    let list_extra = extra.div_euclid(3);
    let list_h = (natural_list_h + list_extra).max(minimum_list(line, dpi));
    let log_h = (natural_log_h + extra - list_extra).max(minimum_log(line, dpi));

    y += m;
    let gpus_h = gpus_fixed + list_h;
    layout.gpus = section(m, y, content_w, gpus_h, input.texts.gpus_title, line, dpi);
    layout.list = Rect::new(m + pad, y + head, inner_w, list_h);
    let refresh_w = button_width(input.texts.refresh, dpi);
    layout.refresh = Rect::new(
        m + content_w - pad - refresh_w,
        layout.list.bottom() + gap,
        refresh_w,
        button_h,
    );
    y += gpus_h + m;

    layout.files = section(m, y, content_w, files_h, input.texts.file_title, line, dpi);
    layout.file = Rect::new(m + pad, y + head, button_width(input.texts.file, dpi), button_h);
    let path_x = layout.file.right() + gap;
    layout.path = Rect::new(path_x, y + head, (m + content_w - pad - path_x).max(0), button_h);
    layout.validation = Rect::new(m + pad, layout.file.bottom() + gap, inner_w, validation_h);
    y += files_h + m;

    let verify_w = button_width(input.texts.verify, dpi);
    let remove_w = button_width(input.texts.remove, dpi);
    let load_w = button_width(input.texts.load, dpi);
    layout.verify = Rect::new(m + content_w - verify_w, y, verify_w, button_h);
    layout.remove = Rect::new(layout.verify.x - gap - remove_w, y, remove_w, button_h);
    layout.load = Rect::new(layout.remove.x - gap - load_w, y, load_w, button_h);
    layout.working = Rect::new(m, y + (button_h - line) / 2, (layout.load.x - gap - m).max(0), line);
    y += button_h + m;

    layout.messages = section(
        m,
        y,
        content_w,
        messages_fixed + log_h,
        input.texts.messages_title,
        line,
        dpi,
    );
    layout.log = Rect::new(m + pad, y + head, inner_w, log_h);
    layout
}

/// The client width at which the GPU columns fit at their own widths with a vertical scroll bar.
pub fn natural_width(dpi: u32) -> i32 {
    let columns: i32 = COLUMNS.iter().map(|w| scale(*w, dpi)).sum();
    columns + scale(SCROLL, dpi) + 2 * scale(EDGE, dpi) + 2 * scale(PAD, dpi) + 2 * scale(MARGIN, dpi)
}

/// The smallest client area at `dpi`.
pub fn min_client(dpi: u32) -> (i32, i32) {
    (scale(MIN_CLIENT.0, dpi), scale(MIN_CLIENT.1, dpi))
}

/// Column widths for a list whose client area is `total` pixels wide: the widths of gui.py at `dpi`, with the
/// difference given to (or taken from) Name and Override, half each, never below the minimum width.
pub fn column_widths(total: i32, dpi: u32) -> [i32; 6] {
    let mut widths = COLUMNS.map(|w| scale(w, dpi));
    let minimum = scale(COLUMN_MIN, dpi);
    let [name, over] = STRETCH;
    let extra = total - widths.iter().sum::<i32>();
    if extra >= 0 {
        widths[name] += extra / 2;
        widths[over] += extra - extra / 2;
    } else {
        let deficit = -extra;
        let (name_room, over_room) = ((widths[name] - minimum).max(0), (widths[over] - minimum).max(0));
        let first = (deficit - deficit / 2).min(over_room);
        let take_name = (deficit - first).min(name_room);
        let take_over = (deficit - take_name).min(over_room);
        widths[name] -= take_name;
        widths[over] -= take_over;
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXTS: TextWidths = TextWidths {
        language: 55,
        theme: 40,
        refresh: 45,
        file: 70,
        load: 28,
        remove: 100,
        verify: 35,
        gpus_title: 70,
        file_title: 55,
        messages_title: 58,
    };

    /// About 6 px per character of the English texts at 96 DPI, scaled; 15 px lines.
    fn wrap(kind: Wrapped, width: i32) -> i32 {
        let chars = match kind {
            Wrapped::Banner => 160,
            Wrapped::Validation => 60,
        };
        let lines = (chars * 6 + width.max(1) - 1) / width.max(1);
        lines.max(1) * 15
    }

    fn layout(width: i32, height: i32, banner: bool) -> Layout {
        compute(&Input {
            width,
            height,
            dpi: 96,
            line: 15,
            banner,
            texts: TEXTS,
            wrap: &wrap,
        })
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.x >= outer.x && inner.y >= outer.y && inner.right() <= outer.right() && inner.bottom() <= outer.bottom()
    }

    #[test]
    fn rows_stack_with_margins_and_nothing_overlaps() {
        for banner in [false, true] {
            let l = layout(940, 640, banner);
            assert_eq!((l.language.x, l.language.y), (8, 6));
            assert_eq!(l.theme.x, l.language.right() + 6);
            let top = match l.banner {
                Some(b) => {
                    assert_eq!((b.x, b.y, b.w), (8, l.language.bottom() + 8, 924));
                    assert!(inside(l.banner_text, b));
                    b.bottom()
                }
                None => l.language.bottom(),
            };
            assert_eq!(l.gpus.title.y, top + 8);
            assert!(inside(l.list, l.gpus.frame) && inside(l.refresh, l.gpus.frame));
            assert_eq!(l.refresh.right(), l.list.right());
            assert_eq!(l.refresh.y, l.list.bottom() + 6);
            assert_eq!(l.files.title.y, l.gpus.frame.bottom() + 8);
            for r in [l.file, l.path, l.validation] {
                assert!(inside(r, l.files.frame), "{r:?}");
            }
            assert_eq!(l.path.x, l.file.right() + 6);
            assert_eq!(l.path.right(), l.list.right());
            assert_eq!(l.load.y, l.files.frame.bottom() + 8);
            assert_eq!(
                (l.verify.right(), l.remove.right() + 6, l.load.right() + 6),
                (932, l.verify.x, l.remove.x)
            );
            assert!(l.working.right() <= l.load.x && l.working.x == 8);
            assert_eq!(l.messages.title.y, l.load.bottom() + 8);
            assert!(inside(l.log, l.messages.frame));
            assert_eq!(l.messages.frame.bottom() + 8, 640);
        }
    }

    #[test]
    fn extra_height_goes_to_the_list_and_the_messages_one_to_two() {
        let natural = layout(940, 0, true).natural_height;
        let base = layout(940, natural, true);
        let taller = layout(940, natural + 300, true);
        assert_eq!(taller.list.h - base.list.h, 100);
        assert_eq!(taller.log.h - base.log.h, 200);
        assert_eq!(base.log.h, 8 * 15 + 4 + 4);
        assert_eq!(base.list.h, 24 + 4 * 19 + 4);
        // At the minimum size every part still fits in the window.
        let (w, h) = min_client(96);
        let small = layout(w, h, true);
        assert!(small.messages.frame.bottom() <= h, "{small:?}");
        assert!(small.list.h >= 15 + 9 + 15 + 4 + 4);
    }

    #[test]
    fn hidden_banner_moves_everything_up() {
        let with = layout(940, 640, true);
        let without = layout(940, 640, false);
        assert!(without.banner.is_none());
        assert!(without.gpus.frame.y < with.gpus.frame.y);
        assert!(without.list.h > with.list.h);
    }

    #[test]
    fn buttons_have_a_minimum_width_and_fit_their_text() {
        let l = layout(940, 640, false);
        assert_eq!(l.load.w, 75);
        assert_eq!(l.remove.w, 100 + 24);
        assert_eq!(l.language.w, 55 + 44);
        assert_eq!(l.load.h, 23);
        let arrow = menu_arrow(l.language, 96);
        assert!(arrow
            .iter()
            .all(|(x, y)| *x > l.language.x + 55 && *x < l.language.right() && *y > l.language.y));
    }

    #[test]
    fn everything_scales_with_dpi() {
        assert_eq!(
            (scale(8, 96), scale(8, 144), scale(8, 120), scale(75, 192)),
            (8, 12, 10, 150)
        );
        assert_eq!(min_client(144), (1170, 810));
        let l = compute(&Input {
            width: 1410,
            height: 960,
            dpi: 144,
            line: 22,
            banner: false,
            texts: TEXTS,
            wrap: &wrap,
        });
        assert_eq!((l.language.x, l.language.y), (12, 9));
        assert_eq!(l.verify.right(), 1410 - 12);
        assert_eq!(natural_width(96), 943);
    }

    #[test]
    fn columns_stretch_name_and_override() {
        assert_eq!(column_widths(890, 96), COLUMNS);
        assert_eq!(column_widths(1000, 96), [40, 285, 90, 80, 150, 355]);
        assert_eq!(column_widths(731, 96), [40, 151, 90, 80, 150, 220]);
        // Name stops at the minimum width; Override gives the rest, then the list scrolls sideways.
        assert_eq!(column_widths(500, 96), [40, 35, 90, 80, 150, 105]);
        assert_eq!(column_widths(450, 96), [40, 30, 90, 80, 150, 60]);
        assert_eq!(column_widths(100, 96), [40, 30, 90, 80, 150, 30]);
        assert_eq!(column_widths(1335, 144), [60, 345, 135, 120, 225, 450]);
        for total in [600, 731, 890, 1200] {
            assert_eq!(column_widths(total, 96).iter().sum::<i32>(), total);
        }
    }
}
