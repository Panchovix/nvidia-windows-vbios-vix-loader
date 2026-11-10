//! GDI drawing of the parts the window paints itself: section frames and titles, the dark owner-drawn buttons,
//! the menu buttons' arrow and the dark list header. Every brush and pen is created for one call and deleted.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, FrameRect, Polygon, SelectObject, SetBkMode,
    SetTextColor, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOCLIP, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, HDC, HFONT, HGDIOBJ, PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, HDM_GETITEMCOUNT, HDM_GETITEMRECT, ODS_DISABLED, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, SendMessageW, WM_GETFONT};

use super::layout::{self, Rect, Section};
use super::palette::{Colors, Rgb};
use super::text::wide;

/// A layout rectangle as a Win32 RECT.
pub fn win_rect(r: Rect) -> RECT {
    RECT {
        left: r.x,
        top: r.y,
        right: r.right(),
        bottom: r.bottom(),
    }
}

/// A Win32 RECT as a layout rectangle.
pub fn layout_rect(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// A GDI object deleted when dropped (fonts, brushes, pens). A null handle is allowed and ignored.
pub struct Owned(pub HGDIOBJ);

impl Owned {
    pub fn null() -> Owned {
        Owned(null_mut())
    }

    pub fn handle(&self) -> HGDIOBJ {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the object was created by this program and is not selected into any DC any more.
            unsafe { DeleteObject(self.0) };
        }
    }
}

/// A solid brush of `color`.
pub fn brush(color: Rgb) -> Owned {
    // SAFETY: plain GDI object creation; a failure gives a null handle, which Owned ignores.
    Owned(unsafe { CreateSolidBrush(color.colorref()) })
}

/// An object selected into a DC until dropped, when the previous one is put back.
struct Selected {
    hdc: HDC,
    previous: HGDIOBJ,
}

impl Selected {
    fn new(hdc: HDC, object: HGDIOBJ) -> Selected {
        let previous = if object.is_null() {
            null_mut()
        } else {
            // SAFETY: `hdc` is a valid DC for the duration of the paint and `object` a live GDI object.
            unsafe { SelectObject(hdc, object) }
        };
        Selected { hdc, previous }
    }
}

impl Drop for Selected {
    fn drop(&mut self) {
        if !self.previous.is_null() {
            // SAFETY: puts back the object that was selected before `new`.
            unsafe { SelectObject(self.hdc, self.previous) };
        }
    }
}

pub fn fill(hdc: HDC, rect: &RECT, color: Rgb) {
    let brush = brush(color);
    if !brush.handle().is_null() {
        // SAFETY: valid DC, rectangle and brush.
        unsafe { FillRect(hdc, rect, brush.handle()) };
    }
}

/// A one-pixel border inside `rect`.
pub fn frame(hdc: HDC, rect: &RECT, color: Rgb) {
    let brush = brush(color);
    if !brush.handle().is_null() {
        // SAFETY: valid DC, rectangle and brush.
        unsafe { FrameRect(hdc, rect, brush.handle()) };
    }
}

/// The client edge of a control at `rect`, painted by the window (dark theme): `width` pixels of `line`, then
/// `width` pixels of `inner`, the control's background. The control itself sits `2 * width` pixels inside.
pub fn edge(hdc: HDC, rect: Rect, line: Rgb, inner: Rgb, width: i32) {
    for step in 0..2 * width {
        let ring = rect.inset(step);
        if ring.w <= 0 || ring.h <= 0 {
            break;
        }
        frame(hdc, &win_rect(ring), if step < width { line } else { inner });
    }
}

/// Text in `rect` with these DrawText flags, in `color` with `font`, on a transparent background.
pub fn text(hdc: HDC, text: &str, rect: &RECT, color: Rgb, font: HFONT, flags: DRAW_TEXT_FORMAT) {
    let _font = Selected::new(hdc, font);
    let units = wide(text);
    let mut rect = *rect;
    // SAFETY: valid DC; `units` is NUL-terminated (-1 length) and `rect` a valid in/out rectangle.
    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, color.colorref());
        DrawTextW(hdc, units.as_ptr(), -1, &mut rect, flags | DT_NOPREFIX);
    }
}

/// A section: the frame, then the title on its top edge with the line cleared around it.
pub fn section(hdc: HDC, section: &Section, title: &str, colors: &Colors, font: HFONT, dpi: u32) {
    frame(hdc, &win_rect(section.frame), colors.frame);
    let pad = layout::scale(layout::TITLE_PAD, dpi);
    let t = section.title;
    fill(
        hdc,
        &RECT {
            left: t.x - pad,
            top: t.y,
            right: t.right() + pad,
            bottom: t.bottom(),
        },
        colors.bg,
    );
    text(
        hdc,
        title,
        &win_rect(t),
        colors.fg,
        font,
        DT_LEFT | DT_SINGLELINE | DT_NOCLIP,
    );
}

/// The down arrow of a menu button, a filled triangle.
pub fn arrow(hdc: HDC, points: [(i32, i32); 3], color: Rgb) {
    let brush = brush(color);
    // SAFETY: plain GDI object creation; a failure gives a null handle, which Owned ignores.
    let pen = Owned(unsafe { CreatePen(PS_SOLID, 1, color.colorref()) });
    let _brush = Selected::new(hdc, brush.handle());
    let _pen = Selected::new(hdc, pen.handle());
    let points = points.map(|(x, y)| POINT { x, y });
    // SAFETY: valid DC and three points.
    unsafe { Polygon(hdc, points.as_ptr(), 3) };
}

/// A dark owner-drawn push button: face, border (the accent color while it has the keyboard focus), centered
/// text, and the arrow of a menu button. Disabled and pressed states as gui.py's dark ttk style.
pub fn button(item: &DRAWITEMSTRUCT, label: &str, colors: &Colors, font: HFONT, menu: bool, dpi: u32) {
    let hdc = item.hDC;
    let rect = item.rcItem;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let focused = item.itemState & ODS_FOCUS != 0 && item.itemState & ODS_NOFOCUSRECT == 0;
    let face = if disabled {
        colors.bg
    } else if pressed {
        colors.pressed
    } else {
        colors.button
    };
    fill(hdc, &rect, face);
    if focused {
        frame(hdc, &rect, colors.focus);
        let inner = RECT {
            left: rect.left + 1,
            top: rect.top + 1,
            right: rect.right - 1,
            bottom: rect.bottom - 1,
        };
        frame(hdc, &inner, colors.focus);
    } else {
        frame(hdc, &rect, colors.border);
    }
    let fg = if disabled { colors.disabled_fg } else { colors.fg };
    let mut label_rect = rect;
    if pressed {
        label_rect.left += 1;
        label_rect.top += 1;
        label_rect.right += 1;
        label_rect.bottom += 1;
    }
    text(
        hdc,
        label,
        &label_rect,
        fg,
        font,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    if menu {
        arrow(hdc, layout::menu_arrow(layout_rect(&label_rect), dpi), fg);
    }
}

/// The list header in the dark theme: dark items with light text, a divider after each column and a line under
/// them. The header's own painting would be light. `labels` are the column titles, by column index.
pub fn header(header: HWND, hdc: HDC, colors: &Colors, dpi: u32, labels: &[&str]) {
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `header` is a live window; `client` a valid out-pointer.
    unsafe { GetClientRect(header, &mut client) };
    fill(hdc, &client, colors.header_bg);
    // SAFETY: WM_GETFONT has no parameters and returns the header's font (or null for the system font).
    let font = unsafe { SendMessageW(header, WM_GETFONT, 0, 0) } as HFONT;
    // SAFETY: HDM_GETITEMCOUNT has no parameters.
    let count = unsafe { SendMessageW(header, HDM_GETITEMCOUNT, 0, 0) };
    let inset = layout::scale(6, dpi);
    for index in 0..count.max(0) {
        let mut item = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: HDM_GETITEMRECT fills the RECT passed by pointer.
        let found = unsafe {
            SendMessageW(
                header,
                HDM_GETITEMRECT,
                index as WPARAM,
                &mut item as *mut RECT as LPARAM,
            )
        };
        if found == 0 {
            continue;
        }
        let divider = RECT {
            left: item.right - 1,
            top: item.top + inset / 2,
            right: item.right,
            bottom: item.bottom - inset / 2,
        };
        fill(hdc, &divider, colors.border);
        let label = labels.get(index as usize).copied().unwrap_or("");
        let label_rect = RECT {
            left: item.left + inset,
            top: item.top,
            right: item.right - inset,
            bottom: item.bottom,
        };
        text(
            hdc,
            label,
            &label_rect,
            colors.header_fg,
            font,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    let line = RECT {
        left: client.left,
        top: client.bottom - 1,
        right: client.right,
        bottom: client.bottom,
    };
    fill(hdc, &line, colors.border);
}
