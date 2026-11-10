//! The loader window in plain Win32 (gui.py's Window class): the controls, their texts and colors, the layout,
//! the worker thread of Load / Remove override / Verify, and the smoke mode.
//!
//! State lives in one `App` that the window procedure reaches through GWLP_USERDATA. Win32 calls re-enter the
//! window procedure (SendMessage to the list sends LVN_ITEMCHANGED back, MessageBox and the file dialog run their
//! own message loops), so `App` is used through `&self` only, with `Cell`/`RefCell` fields that are never borrowed
//! across a Win32 call that sends messages. Every handler runs inside `catch_unwind`: a panic never crosses the
//! window procedure; it is logged as "Unexpected error: ..." and marks the run as failed (exit code 1 in smoke mode).

use std::any::Any;
use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::c_void;
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use windows_sys::core::{w, BOOL, PCSTR, PCWSTR};
use windows_sys::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, DrawTextW, EndPaint, GetDC, GetMonitorInfoW, GetSysColor, GetTextExtentPoint32W,
    GetTextMetricsW, InvalidateRect, MonitorFromWindow, RedrawWindow, ReleaseDC, SelectObject, UpdateWindow,
    COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, DT_CALCRECT, DT_EDITCONTROL,
    DT_EXPANDTABS, DT_NOPREFIX, DT_WORDBREAK, HDC, HGDIOBJ, LOGFONTW, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, TEXTMETRICW,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, SetWindowTheme, CDDS_POSTPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NOTIFYPOSTPAINT,
    CLR_NONE, DRAWITEMSTRUCT, EM_REPLACESEL, EM_SETLIMITTEXT, EM_SETSEL, ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCF_FMT, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_PARAM,
    LVIF_STATE, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS, LVM_ENSUREVISIBLE, LVM_GETHEADER,
    LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETBKCOLOR, LVM_SETCOLUMNW, LVM_SETCOLUMNWIDTH,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR,
    LVNI_SELECTED, LVN_ITEMCHANGED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_NOSORTHEADER, LVS_REPORT,
    LVS_SHOWSELALWAYS, LVS_SINGLESEL, NMCUSTOMDRAW, NMHDR, NMLISTVIEW, NM_CUSTOMDRAW, ODT_BUTTON, WC_BUTTONW, WC_EDITW,
    WC_LISTVIEWW, WC_STATICW,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, SetProcessDpiAwarenessContext, SystemParametersInfoForDpi,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetActiveWindow, GetFocus, IsWindowEnabled, SetFocus, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallWindowProcW, CheckMenuRadioItem, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetClientRect, GetDlgCtrlID, GetMessageW, GetNextDlgTabItem, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsChild, IsDialogMessageW, IsWindow, IsWindowVisible,
    KillTimer, LoadCursorW, LoadIconW, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassExW, SendMessageW,
    SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TrackPopupMenuEx, TranslateMessage,
    BN_CLICKED, BS_OWNERDRAW, BS_PUSHBUTTON, BS_TYPEMASK, CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOHSCROLL, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_READONLY, GWLP_USERDATA, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE, HMENU, HWND_TOPMOST, IDC_ARROW,
    IDI_APPLICATION, IDOK, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MESSAGEBOX_STYLE,
    MF_BYCOMMAND, MF_STRING, MINMAXINFO, MSG, NONCLIENTMETRICSW, SB_BOTTOM, SPI_GETNONCLIENTMETRICS, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, SW_SHOWDEFAULT, TPMPARAMS,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_TOPALIGN, TPM_VERTICAL, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM,
    WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCCREATE, WM_NCDESTROY, WM_NEXTDLGCTL, WM_NOTIFY, WM_PAINT,
    WM_PRINTCLIENT, WM_QUERYENDSESSION, WM_SETFOCUS, WM_SETFONT, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSEXW, WNDPROC,
    WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use super::args::Options;
use super::layout::{self, Input, Layout, TextWidths, Wrapped};
use super::paint::{self, Owned};
use super::palette::{self, Colors, Rgb, SystemColors};
use super::smoke::{self, Step};
use super::text::{self, wide};
use super::{Stream, PROG};
use crate::controller::{language_for, resolve_theme, Action, Controller, Language, Theme};
use crate::platform::Platform;

/// Static control styles (WinUser.h); windows-sys has them only in Win32_System_SystemServices.
const SS_LEFT: u32 = 0x0000;
const SS_LEFTNOWORDWRAP: u32 = 0x000C;
const SS_NOPREFIX: u32 = 0x0080;
const SS_EDITCONTROL: u32 = 0x2000;
const SS_ENDELLIPSIS: u32 = 0x4000;

const CLASS_NAME: PCWSTR = w!("VbiosLoaderGuiWindow");

// Control ids. Not 1 or 2: IsDialogMessageW sends IDOK and IDCANCEL for Enter and Escape.
const ID_LANGUAGE: u16 = 100;
const ID_THEME: u16 = 101;
const ID_BANNER: u16 = 102;
const ID_LIST: u16 = 103;
const ID_REFRESH: u16 = 104;
const ID_FILE: u16 = 105;
const ID_PATH: u16 = 106;
const ID_VALIDATION: u16 = 107;
const ID_WORKING: u16 = 108;
const ID_LOAD: u16 = 109;
const ID_REMOVE: u16 = 110;
const ID_VERIFY: u16 = 111;
const ID_LOG: u16 = 112;

/// Popup menu item ids: base + index in Language::ALL or Theme::ALL.
const MENU_LANGUAGE: u32 = 1000;
const MENU_THEME: u32 = 1100;

/// Polls the worker's result in case the posted wake-up is lost (gui.py polled every 200 ms).
const TIMER_RESULT: usize = 1;
const RESULT_POLL_MS: u32 = 250;
/// Smoke steps: base + index in the schedule.
const TIMER_SMOKE: usize = 10;

/// The worker posts this when its result is in the channel.
const WM_APP_RESULT: u32 = WM_APP + 1;
/// The list's selection changed: handled after the list's own notification, as Tk queued <<TreeviewSelect>>.
const WM_APP_SELECTION: u32 = WM_APP + 2;

thread_local! {
    /// The header subclass's state: the App (as an address) and the header's own window procedure. There is one
    /// window per process, on this thread.
    static HEADER: Cell<(usize, isize)> = const { Cell::new((0, 0)) };
}

/// The list's column headers, as text keys.
const COLUMN_KEYS: [&str; 6] = [
    "col_index",
    "col_name",
    "col_pci",
    "col_location",
    "col_driver",
    "col_override",
];

/// Run the window until it closes; returns the exit code.
pub fn run(options: Options, platform: Arc<dyn Platform>) -> i32 {
    let smoke = smoke::enabled(std::env::var_os(super::SMOKE_ENV).as_deref());
    // SAFETY: process-wide setting; it fails on Windows before 10 1703 or when the manifest already set it.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let classes = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
    };
    // SAFETY: `classes` is a valid structure.
    unsafe { InitCommonControlsEx(&classes) };
    // SAFETY: no parameters.
    let language = options
        .lang
        .unwrap_or_else(|| language_for(unsafe { GetUserDefaultUILanguage() }));
    let app = Box::new(App::new(Controller::new(platform, language, options.theme), smoke));
    // A panic while opening (in the controller's first refresh, for example) must not drop the App under a live
    // window: report it, destroy the window, then drop.
    let opened = catch_unwind(AssertUnwindSafe(|| app.open()))
        .unwrap_or_else(|payload| Err(format!("unexpected error: {}", text::panic_message(&*payload))));
    let code = match opened {
        Ok(()) => {
            message_loop(app.hwnd.get());
            smoke::exit_code(smoke, app.failed.get())
        }
        Err(message) => {
            // Before destroying the window: its WM_QUIT would close the message box at once.
            super::emit(Stream::Err, &|_| format!("{PROG}: {message}\n"));
            let hwnd = app.hwnd.get();
            // SAFETY: the window, if any, was created by `open` on this thread.
            if !hwnd.is_null() && unsafe { IsWindow(hwnd) } != 0 {
                // SAFETY: as above.
                unsafe { DestroyWindow(hwnd) };
            }
            1
        }
    };
    // The window is destroyed: nothing refers to the App any more. Dropping it frees the fonts and brushes.
    drop(app);
    code
}

fn message_loop(hwnd: HWND) {
    // SAFETY: all-zero is a valid MSG.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `msg` is a valid out-pointer.
        let result = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if result == 0 || result == -1 {
            break;
        }
        // A multi-line edit under IsDialogMessageW gets every key and, as in a dialog, posts WM_CLOSE to its parent
        // on Escape: Escape in the Messages area would close the window. It does nothing there, as in gui.py.
        if msg.message == WM_KEYDOWN && msg.wParam == usize::from(VK_ESCAPE) && is_log(hwnd, msg.hwnd) {
            continue;
        }
        // IsDialogMessageW moves the focus with Tab and Shift+Tab between the controls, as in a dialog.
        // SAFETY: `msg` was filled by GetMessageW.
        unsafe {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// True when `control` is the Messages area of the window.
fn is_log(window: HWND, control: HWND) -> bool {
    // SAFETY: plain queries; any window handle is accepted.
    unsafe { !control.is_null() && IsChild(window, control) != 0 && GetDlgCtrlID(control) == i32::from(ID_LOG) }
}

#[derive(Clone, Copy)]
struct Controls {
    language: HWND,
    theme: HWND,
    banner: HWND,
    list: HWND,
    header: HWND,
    refresh: HWND,
    file: HWND,
    path: HWND,
    validation: HWND,
    working: HWND,
    load: HWND,
    remove: HWND,
    verify: HWND,
    log: HWND,
}

impl Controls {
    /// The push buttons: native in the light theme, owner-drawn in the dark one.
    fn buttons(&self) -> [HWND; 7] {
        [
            self.language,
            self.theme,
            self.refresh,
            self.file,
            self.load,
            self.remove,
            self.verify,
        ]
    }

    /// Every control that gets the message font (the list passes it on to its header).
    fn all(&self) -> [HWND; 13] {
        [
            self.language,
            self.theme,
            self.banner,
            self.list,
            self.refresh,
            self.file,
            self.path,
            self.validation,
            self.working,
            self.load,
            self.remove,
            self.verify,
            self.log,
        ]
    }
}

/// The brushes returned for WM_CTLCOLOR*: they must outlive the message.
struct Brushes {
    bg: Owned,
    field: Owned,
    log: Owned,
    banner: Owned,
}

impl Brushes {
    fn new(colors: &Colors) -> Brushes {
        Brushes {
            bg: paint::brush(colors.bg),
            field: paint::brush(colors.field),
            log: paint::brush(colors.log_bg),
            banner: paint::brush(colors.banner_bg),
        }
    }

    fn none() -> Brushes {
        Brushes {
            bg: Owned::null(),
            field: Owned::null(),
            log: Owned::null(),
            banner: Owned::null(),
        }
    }
}

/// Fonts and colors; replaced on DPI and theme changes (the old objects are deleted after the controls let go).
struct Gdi {
    font: Owned,
    /// Line height of `font`.
    line: i32,
    colors: Colors,
    brushes: Brushes,
}

type JobResult = (bool, Vec<String>);

struct App {
    hwnd: Cell<HWND>,
    controls: OnceCell<Controls>,
    controller: RefCell<Controller>,
    smoke: bool,
    smoke_steps: Cell<Option<[(u32, Step); 3]>>,
    /// A handler panicked.
    failed: Cell<bool>,
    /// The action running in the worker (gui.py's self.action).
    running: Cell<Option<Action>>,
    /// The list is being filled: its selection events are not the user's.
    filling: Cell<bool>,
    /// A WM_APP_SELECTION is posted and not handled yet.
    selection_pending: Cell<bool>,
    /// Status index of each list row.
    rows: RefCell<Vec<usize>>,
    dpi: Cell<u32>,
    dark: Cell<bool>,
    gdi: RefCell<Gdi>,
    layout: Cell<Option<Layout>>,
    banner_shown: Cell<bool>,
    validation_ok: Cell<bool>,
    /// The control that had the focus when the window was deactivated.
    focus: Cell<HWND>,
    results: Receiver<JobResult>,
    sender: Sender<JobResult>,
}

/// Puts `filling` back to false when dropped, even after a panic.
struct Filling<'a>(&'a Cell<bool>);

impl<'a> Filling<'a> {
    fn start(flag: &'a Cell<bool>) -> Filling<'a> {
        flag.set(true);
        Filling(flag)
    }
}

impl Drop for Filling<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl App {
    fn new(controller: Controller, smoke: bool) -> App {
        let (sender, results) = channel();
        App {
            hwnd: Cell::new(null_mut()),
            controls: OnceCell::new(),
            controller: RefCell::new(controller),
            smoke,
            smoke_steps: Cell::new(None),
            failed: Cell::new(false),
            running: Cell::new(None),
            filling: Cell::new(false),
            selection_pending: Cell::new(false),
            rows: RefCell::new(Vec::new()),
            dpi: Cell::new(96),
            dark: Cell::new(false),
            gdi: RefCell::new(Gdi {
                font: Owned::null(),
                line: 16,
                colors: palette::colors(false, SystemColors::DEFAULT),
                brushes: Brushes::none(),
            }),
            layout: Cell::new(None),
            banner_shown: Cell::new(false),
            validation_ok: Cell::new(false),
            focus: Cell::new(null_mut()),
            results,
            sender,
        }
    }

    fn text(&self, key: &str) -> &'static str {
        self.controller.borrow().text(key)
    }

    fn busy(&self) -> bool {
        self.controller.borrow().busy()
    }

    // ---- creation --------------------------------------------------------------------------------------------

    /// Create the window and its controls, read the status and show it (gui.py's main up to mainloop).
    fn open(&self) -> Result<(), String> {
        // SAFETY: null asks for the module of the running executable.
        let instance: HINSTANCE = unsafe { GetModuleHandleW(null()) };
        // The program's icon (resource 1, when the build embeds one), else the default application icon.
        // SAFETY: an integer resource id (MAKEINTRESOURCE) or a stock icon id; failures return null.
        let icon = unsafe {
            let own = LoadIconW(instance, 1 as PCWSTR);
            if own.is_null() {
                LoadIconW(null_mut(), IDI_APPLICATION)
            } else {
                own
            }
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: icon,
            // SAFETY: a stock cursor.
            hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: CLASS_NAME,
            hIconSm: null_mut(),
        };
        // SAFETY: `class` is fully initialized; the class name is a static string.
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err(format!(
                "cannot open the window (RegisterClassExW error {})",
                last_error()
            ));
        }
        let title = wide(&text::window_title(self.text("title")));
        // SAFETY: the App outlives the window (`run` drops it after the message loop ends with WM_DESTROY); the
        // pointer reaches the window procedure in WM_NCCREATE.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                CLASS_NAME,
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                null_mut(),
                null_mut(),
                instance,
                self as *const App as *const c_void,
            )
        };
        if hwnd.is_null() {
            return Err(format!(
                "cannot open the window (CreateWindowExW error {})",
                last_error()
            ));
        }
        self.hwnd.set(hwnd);
        self.dpi.set(window_dpi(hwnd));
        self.create_controls(instance)?;
        self.make_font();
        self.apply_theme();
        self.retext();
        self.refresh();
        self.place();
        // SAFETY: `hwnd` is our window. SW_SHOWDEFAULT honors how the program was started (STARTUPINFO).
        unsafe {
            ShowWindow(hwnd, SW_SHOWDEFAULT);
            UpdateWindow(hwnd);
        }
        if let Some(c) = self.controls.get() {
            // SAFETY: the list is a child of our window.
            unsafe { SetFocus(c.list) };
        }
        if self.smoke {
            self.start_smoke();
        }
        Ok(())
    }

    fn create_controls(&self, instance: HINSTANCE) -> Result<(), String> {
        let parent = self.hwnd.get();
        let make = |class: PCWSTR, style: u32, ex_style: u32, id: u16| -> Result<HWND, String> {
            // SAFETY: a child of our window with a static class name; the id goes in the menu parameter.
            let hwnd = unsafe {
                CreateWindowExW(
                    ex_style,
                    class,
                    w!(""),
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    0,
                    0,
                    parent,
                    usize::from(id) as HMENU,
                    instance,
                    null(),
                )
            };
            if hwnd.is_null() {
                Err(format!(
                    "cannot create the window's controls (CreateWindowExW error {})",
                    last_error()
                ))
            } else {
                Ok(hwnd)
            }
        };
        let button = WS_TABSTOP | BS_PUSHBUTTON as u32;
        // Creation order is the Tab order.
        let language = make(WC_BUTTONW, button, 0, ID_LANGUAGE)?;
        let theme = make(WC_BUTTONW, button, 0, ID_THEME)?;
        let banner = make(WC_STATICW, SS_LEFT | SS_NOPREFIX | SS_EDITCONTROL, 0, ID_BANNER)?;
        let list_style = WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER;
        let list = make(WC_LISTVIEWW, list_style, WS_EX_CLIENTEDGE, ID_LIST)?;
        let refresh = make(WC_BUTTONW, button, 0, ID_REFRESH)?;
        let file = make(WC_BUTTONW, button, 0, ID_FILE)?;
        let path = make(
            WC_EDITW,
            WS_TABSTOP | (ES_READONLY | ES_AUTOHSCROLL) as u32,
            WS_EX_CLIENTEDGE,
            ID_PATH,
        )?;
        let validation = make(WC_STATICW, SS_LEFT | SS_NOPREFIX | SS_EDITCONTROL, 0, ID_VALIDATION)?;
        let working = make(
            WC_STATICW,
            SS_LEFTNOWORDWRAP | SS_NOPREFIX | SS_ENDELLIPSIS,
            0,
            ID_WORKING,
        )?;
        let load = make(WC_BUTTONW, button, 0, ID_LOAD)?;
        let remove = make(WC_BUTTONW, button, 0, ID_REMOVE)?;
        let verify = make(WC_BUTTONW, button, 0, ID_VERIFY)?;
        let log_style = WS_TABSTOP | WS_VSCROLL | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32;
        let log = make(WC_EDITW, log_style, WS_EX_CLIENTEDGE, ID_LOG)?;

        let extended = (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as usize;
        // SAFETY: plain control messages; the column structures and their texts live through each call.
        let header = unsafe {
            SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, extended, extended as LPARAM);
            for (index, width) in layout::COLUMNS.iter().enumerate() {
                let mut label = wide("");
                let mut column: LVCOLUMNW = std::mem::zeroed();
                column.mask = LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM;
                column.fmt = LVCFMT_LEFT;
                column.cx = layout::scale(*width, self.dpi.get());
                column.pszText = label.as_mut_ptr();
                column.iSubItem = index as i32;
                SendMessageW(list, LVM_INSERTCOLUMNW, index, &column as *const LVCOLUMNW as LPARAM);
            }
            // No limit on the Messages text (the default stops at 32K characters).
            SendMessageW(log, EM_SETLIMITTEXT, 0, 0);
            SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND
        };
        if !header.is_null() {
            // Classic subclassing (user32 only, unlike SetWindowSubclass, which needs comctl32 6 by name): the
            // header's messages go to header_proc first, which passes on what it does not handle.
            // SAFETY: header_proc has the WNDPROC signature; the App outlives the header (a grandchild of our
            // window), and header_proc puts the original procedure back in WM_NCDESTROY.
            let original = unsafe { SetWindowLongPtrW(header, GWLP_WNDPROC, header_proc as *const () as isize) };
            let _ = HEADER.try_with(|state| state.set((self as *const App as usize, original)));
        }
        let controls = Controls {
            language,
            theme,
            banner,
            list,
            header,
            refresh,
            file,
            path,
            validation,
            working,
            load,
            remove,
            verify,
            log,
        };
        // `open` runs once, so the cell is empty.
        let _ = self.controls.set(controls);
        Ok(())
    }

    /// Size the window for the list's 4 rows and 8 lines of messages (gui.py's natural size), never below the
    /// minimum or beyond the monitor's work area, and center it there.
    fn place(&self) {
        let hwnd = self.hwnd.get();
        let dpi = self.dpi.get();
        let (min_w, min_h) = layout::min_client(dpi);
        let width = layout::natural_width(dpi).max(min_w);
        let height = self
            .compute_layout(width, 0)
            .map_or(min_h, |l| l.natural_height)
            .max(min_h);
        let (mut window_w, mut window_h) = window_size(hwnd, width, height, dpi);
        let work = work_area(hwnd);
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        // SAFETY: `hwnd` is our window.
        unsafe {
            match work {
                Some(work) => {
                    window_w = window_w.min(work.right - work.left);
                    window_h = window_h.min(work.bottom - work.top);
                    let x = work.left + (work.right - work.left - window_w) / 2;
                    let y = work.top + (work.bottom - work.top - window_h) / 2;
                    SetWindowPos(hwnd, null_mut(), x, y, window_w, window_h, flags);
                }
                None => {
                    SetWindowPos(hwnd, null_mut(), 0, 0, window_w, window_h, flags | SWP_NOMOVE);
                }
            }
        }
    }

    fn start_smoke(&self) {
        let hwnd = self.hwnd.get();
        let (language, theme) = {
            let c = self.controller.borrow();
            (c.language(), c.theme())
        };
        let steps = smoke::schedule(language, resolve_theme(theme, windows_dark_mode));
        self.smoke_steps.set(Some(steps));
        // SAFETY: `hwnd` is our window.
        unsafe {
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            for (index, (delay, _)) in steps.iter().enumerate() {
                SetTimer(hwnd, TIMER_SMOKE + index, *delay, None);
            }
        }
    }

    // ---- fonts, colors and texts -------------------------------------------------------------------------

    /// The system message font at the window's DPI, set on every control; the previous font is deleted after.
    fn make_font(&self) {
        let hwnd = self.hwnd.get();
        let font = message_font(self.dpi.get());
        let line = line_height(hwnd, font.handle());
        let handle = font.handle();
        let previous = {
            let mut gdi = self.gdi.borrow_mut();
            gdi.line = line;
            std::mem::replace(&mut gdi.font, font)
        };
        if let Some(c) = self.controls.get() {
            for control in c.all() {
                // SAFETY: WM_SETFONT with a live font (or null for the system font); redraw now.
                unsafe { SendMessageW(control, WM_SETFONT, handle as WPARAM, 1) };
            }
        }
        drop(previous);
    }

    /// Resolve the theme and recolor everything, the title bar too (gui.py's apply_theme).
    fn apply_theme(&self) {
        let hwnd = self.hwnd.get();
        let theme = self.controller.borrow().theme();
        let dark = resolve_theme(theme, windows_dark_mode) == Theme::Dark;
        let colors = palette::colors(dark, system_colors());
        let previous = {
            let mut gdi = self.gdi.borrow_mut();
            gdi.colors = colors;
            std::mem::replace(&mut gdi.brushes, Brushes::new(&colors))
        };
        self.dark.set(dark);
        if let Some(c) = self.controls.get().copied() {
            let kind = if dark { BS_OWNERDRAW } else { BS_PUSHBUTTON } as isize;
            let (list_bg, list_fg) = if dark {
                (colors.list_bg, colors.list_fg)
            } else {
                let system = system_colors();
                (system.window, system.text)
            };
            let (list_theme, edit_theme) = if dark {
                (w!("DarkMode_Explorer"), w!("DarkMode_Explorer"))
            } else {
                (w!("Explorer"), null())
            };
            // SAFETY: plain control messages and style changes on our controls; SetWindowTheme takes static strings
            // or null.
            unsafe {
                // The button type is the low bits of the style (BS_TYPEMASK); changing it is what MFC's
                // ModifyStyle(BS_TYPEMASK, BS_OWNERDRAW) does.
                for button in c.buttons() {
                    let style = GetWindowLongPtrW(button, GWL_STYLE);
                    let changed = (style & !(BS_TYPEMASK as isize)) | kind;
                    if changed != style {
                        SetWindowLongPtrW(button, GWL_STYLE, changed);
                        InvalidateRect(button, null(), 1);
                    }
                }
                SendMessageW(c.list, LVM_SETBKCOLOR, 0, list_bg.colorref() as LPARAM);
                SendMessageW(c.list, LVM_SETTEXTBKCOLOR, 0, CLR_NONE as LPARAM);
                SendMessageW(c.list, LVM_SETTEXTCOLOR, 0, list_fg.colorref() as LPARAM);
                SetWindowTheme(c.list, list_theme, null());
                SetWindowTheme(c.path, edit_theme, null());
                SetWindowTheme(c.log, edit_theme, null());
                // The light theme keeps the native client edge; the dark one paints its own (painted_edge).
                for edit in [c.path, c.log] {
                    let style = GetWindowLongPtrW(edit, GWL_EXSTYLE);
                    let edge = WS_EX_CLIENTEDGE as isize;
                    let changed = if dark { style & !edge } else { style | edge };
                    if changed != style {
                        SetWindowLongPtrW(edit, GWL_EXSTYLE, changed);
                        let keep = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
                        SetWindowPos(edit, null_mut(), 0, 0, 0, 0, keep | SWP_FRAMECHANGED);
                    }
                }
            }
            // The edit controls move in or out of the painted edge.
            self.layout.set(None);
            self.layout();
        }
        dark_title_bar(hwnd, dark);
        dark_menus(dark);
        // SAFETY: `hwnd` is our window; repaint it and every control.
        unsafe {
            RedrawWindow(
                hwnd,
                null(),
                null_mut(),
                RDW_ERASE | RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME,
            )
        };
        drop(previous);
    }

    /// Write every text of the window in the controller's language.
    fn retext(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        set_text(self.hwnd.get(), &text::window_title(self.text("title")));
        for (control, key) in [
            (c.language, "language"),
            (c.theme, "theme"),
            (c.banner, "not_admin"),
            (c.refresh, "refresh"),
            (c.file, "file_button"),
            (c.load, "load"),
            (c.remove, "remove"),
            (c.verify, "verify"),
        ] {
            set_text(control, self.text(key));
        }
        for (index, key) in COLUMN_KEYS.iter().enumerate() {
            let mut label = wide(self.text(key));
            // SAFETY: all-zero is a valid LVCOLUMNW; the label lives through the call.
            unsafe {
                let mut column: LVCOLUMNW = std::mem::zeroed();
                column.mask = LVCF_TEXT;
                column.pszText = label.as_mut_ptr();
                SendMessageW(c.list, LVM_SETCOLUMNW, index, &column as *const LVCOLUMNW as LPARAM);
            }
        }
        // SAFETY: repaint the header and the section titles, which the window paints itself.
        unsafe {
            InvalidateRect(c.header, null(), 1);
            InvalidateRect(self.hwnd.get(), null(), 1);
        }
    }

    // ---- what gui.py's Window methods do ---------------------------------------------------------------------

    fn log(&self, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let Some(c) = self.controls.get().copied() else { return };
        let added = wide(&text::log_text(lines));
        // SAFETY: plain edit messages; `added` is NUL-terminated and lives through the call.
        unsafe {
            let end = GetWindowTextLengthW(c.log).max(0) as usize;
            SendMessageW(c.log, EM_SETSEL, end, end as LPARAM);
            SendMessageW(c.log, EM_REPLACESEL, 0, added.as_ptr() as LPARAM);
            SendMessageW(c.log, WM_VSCROLL, SB_BOTTOM as WPARAM, 0);
        }
    }

    fn refresh(&self) {
        if self.busy() {
            return;
        }
        let lines = self.controller.borrow_mut().refresh();
        self.log(&lines);
        self.fill();
    }

    /// Put the controller's rows in the list and select its GPU, without reporting that as the user's choice.
    fn fill(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        let (rows, selected) = {
            let controller = self.controller.borrow();
            (controller.rows(), controller.selected_index())
        };
        {
            let _filling = Filling::start(&self.filling);
            // SAFETY: plain list messages; every item structure and text lives through its call.
            unsafe {
                SendMessageW(c.list, LVM_DELETEALLITEMS, 0, 0);
                for (position, row) in rows.iter().enumerate() {
                    let mut first = wide(&row.index.to_string());
                    let mut item: LVITEMW = std::mem::zeroed();
                    item.mask = LVIF_TEXT | LVIF_PARAM;
                    item.iItem = position as i32;
                    item.pszText = first.as_mut_ptr();
                    item.lParam = row.index as LPARAM;
                    let at = SendMessageW(c.list, LVM_INSERTITEMW, 0, &item as *const LVITEMW as LPARAM);
                    if at < 0 {
                        continue;
                    }
                    let cells = [&row.name, &row.pci_id, &row.location, &row.driver, &row.override_text];
                    for (column, value) in cells.into_iter().enumerate() {
                        let mut value = wide(value);
                        let mut cell: LVITEMW = std::mem::zeroed();
                        cell.iSubItem = column as i32 + 1;
                        cell.pszText = value.as_mut_ptr();
                        SendMessageW(
                            c.list,
                            LVM_SETITEMTEXTW,
                            at as WPARAM,
                            &cell as *const LVITEMW as LPARAM,
                        );
                    }
                }
            }
            *self.rows.borrow_mut() = rows.iter().map(|row| row.index).collect();
            if let Some(position) = selected.and_then(|index| rows.iter().position(|row| row.index == index)) {
                select_row(c.list, position);
            }
        }
        self.fit_columns();
        self.update_controls();
    }

    /// gui.py's update_controls: banner, path, validation, button states and the "working" label.
    fn update_controls(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        let (admin, busy, path, (ok, validation), can_load, can_remove, can_verify) = {
            let controller = self.controller.borrow();
            (
                controller.admin(),
                controller.busy(),
                controller.path().unwrap_or_default().to_string(),
                controller.validation_text(),
                controller.can_load(),
                controller.can_remove(),
                controller.can_verify(),
            )
        };
        let working = match (busy, self.running.get()) {
            (false, _) => "",
            (true, Some(Action::Verify)) => self.text("verifying"),
            (true, _) => self.text("working"),
        };
        self.banner_shown.set(!admin);
        self.validation_ok.set(ok);
        set_text(c.path, &path);
        set_text(c.validation, &validation);
        set_text(c.working, working);
        // SAFETY: plain calls on our controls.
        unsafe {
            ShowWindow(c.banner, if admin { SW_HIDE } else { SW_SHOW });
            for (control, enabled) in [
                (c.refresh, !busy),
                (c.file, !busy),
                (c.load, can_load),
                (c.remove, can_remove),
                (c.verify, can_verify),
            ] {
                EnableWindow(control, BOOL::from(enabled));
            }
            // The validation color may have changed with the same text.
            InvalidateRect(c.validation, null(), 1);
        }
        self.keep_focus();
        self.layout();
    }

    /// A control disabled while it had the focus leaves the keyboard nowhere: move the focus to the list.
    fn keep_focus(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        let hwnd = self.hwnd.get();
        // SAFETY: plain focus queries and calls on our windows.
        unsafe {
            if GetActiveWindow() == hwnd {
                let focus = GetFocus();
                if focus.is_null() || (IsChild(hwnd, focus) != 0 && IsWindowEnabled(focus) == 0) {
                    SetFocus(c.list);
                }
            } else {
                let saved = self.focus.get();
                if !saved.is_null() && IsWindowEnabled(saved) == 0 {
                    self.focus.set(c.list);
                }
            }
        }
    }

    /// LVN_ITEMCHANGED: note the change and handle it once the list is done (WM_APP_SELECTION).
    fn list_changed(&self, change: &NMLISTVIEW) {
        if change.uChanged & LVIF_STATE == 0 || self.filling.get() || self.selection_pending.get() {
            return;
        }
        if (change.uNewState ^ change.uOldState) & LVIS_SELECTED != 0 {
            self.selection_pending.set(true);
            // SAFETY: posting to our own window.
            if unsafe { PostMessageW(self.hwnd.get(), WM_APP_SELECTION, 0, 0) } == 0 {
                self.selection_pending.set(false);
            }
        }
    }

    /// gui.py's on_select: the selected row becomes the controller's GPU (not while busy). A click in the empty
    /// part of the list clears the selection; gui.py's list always keeps one, so it goes back to the GPU.
    fn selection_changed(&self) {
        self.selection_pending.set(false);
        let Some(c) = self.controls.get().copied() else { return };
        // SAFETY: LVM_GETNEXTITEM from the start (-1) for the selected item.
        let position = unsafe { SendMessageW(c.list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as LPARAM) };
        if position >= 0 {
            let index = self.rows.borrow().get(position as usize).copied();
            if let Some(index) = index {
                if !self.busy() {
                    self.controller.borrow_mut().select(index);
                    self.update_controls();
                }
            }
            return;
        }
        let selected = self.controller.borrow().selected_index();
        let position = selected.and_then(|index| self.rows.borrow().iter().position(|row| *row == index));
        if let Some(position) = position {
            let _filling = Filling::start(&self.filling);
            select_row(c.list, position);
        }
    }

    fn choose_file(&self) {
        if self.busy() {
            return;
        }
        let hwnd = self.hwnd.get();
        let title = wide(self.text("file"));
        let filter = text::file_filter(self.text("file_types"), self.text("all_files"));
        // Room for long paths (\\?\ and paths beyond MAX_PATH).
        let mut buffer = vec![0u16; 32 * 1024];
        // SAFETY: all-zero is a valid OPENFILENAMEW; every pointer set below outlives the call.
        let chosen = unsafe {
            let mut dialog: OPENFILENAMEW = std::mem::zeroed();
            dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
            dialog.hwndOwner = hwnd;
            dialog.lpstrFilter = filter.as_ptr();
            dialog.nFilterIndex = 1;
            dialog.lpstrFile = buffer.as_mut_ptr();
            dialog.nMaxFile = buffer.len() as u32;
            dialog.lpstrTitle = title.as_ptr();
            dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
            GetOpenFileNameW(&mut dialog) != 0
        };
        if !chosen {
            return;
        }
        let path = text::from_wide(&buffer);
        if !path.is_empty() {
            self.controller.borrow_mut().choose_file(&path);
            self.update_controls();
        }
    }

    fn load(&self) {
        if self.busy() {
            return;
        }
        // Checks again now: the registry may have changed since the file was chosen.
        let question = self.controller.borrow_mut().confirm_load();
        self.update_controls();
        let Some(question) = question else { return };
        if self.ask(self.text("confirm_load_title"), &question) {
            self.start(Action::Load);
        }
    }

    fn remove(&self) {
        if !self.controller.borrow().can_remove() {
            return;
        }
        let answer = self.controller.borrow().confirm_remove();
        match answer {
            Err(line) => self.log(&[line]),
            Ok(question) => {
                if self.ask(self.text("confirm_remove_title"), &question) {
                    self.start(Action::Unload);
                }
            }
        }
    }

    fn verify(&self) {
        // Verify only reads: no confirmation.
        if self.controller.borrow().can_verify() {
            self.start(Action::Verify);
        }
    }

    /// Run the action in a worker thread; its result comes back through the channel (gui.py's queue).
    fn start(&self, action: Action) {
        let job = self.controller.borrow_mut().start(action);
        self.running.set(Some(action));
        self.update_controls();
        let hwnd = self.hwnd.get();
        let target = hwnd as usize;
        let unexpected = self.text("unexpected");
        let sender = self.sender.clone();
        let spawned = std::thread::Builder::new()
            .name("vbios-loader-job".to_string())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| job.run()))
                    .unwrap_or_else(|payload| (false, vec![text::unexpected_panic(unexpected, &*payload)]));
                if sender.send(result).is_ok() {
                    // SAFETY: PostMessageW may be called from any thread; if the window is gone it just fails.
                    unsafe { PostMessageW(target as HWND, WM_APP_RESULT, 0, 0) };
                }
            });
        match spawned {
            // SAFETY: a timer on our window, as a backup for the posted message.
            Ok(_) => unsafe {
                SetTimer(hwnd, TIMER_RESULT, RESULT_POLL_MS, None);
            },
            Err(error) => {
                self.controller.borrow_mut().finish();
                self.running.set(None);
                self.log(&[text::unexpected(unexpected, &error.to_string())]);
                self.update_controls();
            }
        }
    }

    /// The worker's result: not busy any more, log it, refresh and show it (gui.py's poll).
    fn take_result(&self) {
        let Ok((ok, lines)) = self.results.try_recv() else {
            return;
        };
        let hwnd = self.hwnd.get();
        // SAFETY: the timer was set on our window by `start`.
        unsafe { KillTimer(hwnd, TIMER_RESULT) };
        self.controller.borrow_mut().finish();
        self.running.set(None);
        self.log(&lines);
        self.refresh();
        let icon = if ok { MB_ICONINFORMATION } else { MB_ICONERROR };
        self.message_box(&text::result_text(&lines), self.text("title"), MB_OK | icon);
    }

    fn set_language(&self, language: Language) {
        // The running action finishes in the current language.
        if self.busy() {
            return;
        }
        self.controller.borrow_mut().set_language(language);
        self.retext();
        self.fill();
    }

    fn set_theme(&self, theme: Theme) {
        if self.busy() {
            return;
        }
        self.controller.borrow_mut().set_theme(theme);
        self.apply_theme();
        self.fill();
    }

    fn ask(&self, title: &str, question: &str) -> bool {
        self.message_box(question, title, MB_OKCANCEL | MB_ICONWARNING) == IDOK
    }

    fn message_box(&self, body: &str, title: &str, style: MESSAGEBOX_STYLE) -> i32 {
        let body = wide(body);
        let title = wide(title);
        // SAFETY: both strings are NUL-terminated and outlive the modal call; the owner is our window.
        unsafe { MessageBoxW(self.hwnd.get(), body.as_ptr(), title.as_ptr(), style) }
    }

    fn language_menu(&self) {
        let current = self.controller.borrow().language();
        let items: Vec<(u32, &str)> = Language::ALL
            .iter()
            .enumerate()
            .map(|(index, language)| (MENU_LANGUAGE + index as u32, language.native_name()))
            .collect();
        let checked = MENU_LANGUAGE + Language::ALL.iter().position(|l| *l == current).unwrap_or(0) as u32;
        let Some(c) = self.controls.get().copied() else { return };
        let chosen = self.popup(c.language, &items, checked);
        if let Some(language) = chosen
            .checked_sub(MENU_LANGUAGE)
            .and_then(|i| Language::ALL.get(i as usize))
        {
            self.set_language(*language);
        }
    }

    fn theme_menu(&self) {
        let current = self.controller.borrow().theme();
        let items: Vec<(u32, &str)> = Theme::ALL
            .iter()
            .enumerate()
            .map(|(index, theme)| (MENU_THEME + index as u32, self.text(theme.text_key())))
            .collect();
        let checked = MENU_THEME + Theme::ALL.iter().position(|t| *t == current).unwrap_or(0) as u32;
        let Some(c) = self.controls.get().copied() else { return };
        let chosen = self.popup(c.theme, &items, checked);
        if let Some(theme) = chosen.checked_sub(MENU_THEME).and_then(|i| Theme::ALL.get(i as usize)) {
            self.set_theme(*theme);
        }
    }

    /// A popup menu under `button` with radio-checked entries; returns the chosen id, 0 when dismissed.
    fn popup(&self, button: HWND, items: &[(u32, &str)], checked: u32) -> u32 {
        // SAFETY: the menu is created, shown and destroyed here; the item texts live through AppendMenuW, which
        // copies them.
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return 0;
            }
            for (id, label) in items {
                let label = wide(label);
                AppendMenuW(menu, MF_STRING, *id as usize, label.as_ptr());
            }
            if let (Some(first), Some(last)) = (items.first(), items.last()) {
                CheckMenuRadioItem(menu, first.0, last.0, checked, MF_BYCOMMAND);
            }
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            GetWindowRect(button, &mut rect);
            let params = TPMPARAMS {
                cbSize: size_of::<TPMPARAMS>() as u32,
                rcExclude: rect,
            };
            let flags = TPM_LEFTALIGN | TPM_TOPALIGN | TPM_VERTICAL | TPM_RETURNCMD | TPM_NONOTIFY;
            let chosen = TrackPopupMenuEx(menu, flags, rect.left, rect.bottom, self.hwnd.get(), &params);
            DestroyMenu(menu);
            chosen.max(0) as u32
        }
    }

    // ---- layout and painting ----------------------------------------------------------------------------------

    fn compute_layout(&self, width: i32, height: i32) -> Option<Layout> {
        let c = self.controls.get().copied()?;
        let hwnd = self.hwnd.get();
        let dpi = self.dpi.get();
        let (font, line) = {
            let gdi = self.gdi.borrow();
            (gdi.font.handle(), gdi.line)
        };
        let measure = Measure::new(hwnd, font)?;
        let texts = TextWidths {
            language: measure.width(&window_text(c.language)),
            theme: measure.width(&window_text(c.theme)),
            refresh: measure.width(&window_text(c.refresh)),
            file: measure.width(&window_text(c.file)),
            load: measure.width(&window_text(c.load)),
            remove: measure.width(&window_text(c.remove)),
            verify: measure.width(&window_text(c.verify)),
            gpus_title: measure.width(self.text("gpus")),
            file_title: measure.width(self.text("file")),
            messages_title: measure.width(self.text("messages")),
        };
        let banner = window_text(c.banner);
        let validation = window_text(c.validation);
        let wrap = |kind: Wrapped, width: i32| match kind {
            Wrapped::Banner => measure.wrapped_height(&banner, width),
            Wrapped::Validation => measure.wrapped_height(&validation, width),
        };
        Some(layout::compute(&Input {
            width,
            height,
            dpi,
            line,
            banner: self.banner_shown.get(),
            texts,
            wrap: &wrap,
        }))
    }

    /// Place every control for the current client size.
    fn layout(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        let hwnd = self.hwnd.get();
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: `client` is a valid out-pointer.
        unsafe { GetClientRect(hwnd, &mut client) };
        let (width, height) = (client.right - client.left, client.bottom - client.top);
        if width <= 0 || height <= 0 {
            return; // minimized
        }
        let Some(l) = self.compute_layout(width, height) else {
            return;
        };
        if self.layout.get() == Some(l) {
            // Nothing moves: only the columns may need to follow a new vertical scroll bar.
            self.fit_columns();
            return;
        }
        self.layout.set(Some(l));
        let edge = self.painted_edge().map_or(0, |width| 2 * width);
        let placed = [
            (c.language, l.language),
            (c.theme, l.theme),
            (c.banner, l.banner_text),
            (c.list, l.list),
            (c.refresh, l.refresh),
            (c.file, l.file),
            (c.path, l.path.inset(edge)),
            (c.validation, l.validation),
            (c.working, l.working),
            (c.load, l.load),
            (c.remove, l.remove),
            (c.verify, l.verify),
            (c.log, l.log.inset(edge)),
        ];
        // SAFETY: moving our own child windows; then repaint the frames, titles and banner of the window.
        // SWP_NOCOPYBITS: each control repaints at its new place. Without it Windows copies the control's pixels
        // from its old place, and as the controls move one after the other, one can land where the next still is,
        // whose copy then carries it along (a ghost "VBIOS file..." under the validation line after a resize).
        unsafe {
            for (control, r) in placed {
                SetWindowPos(
                    control,
                    null_mut(),
                    r.x,
                    r.y,
                    r.w.max(0),
                    r.h.max(0),
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
                );
            }
            InvalidateRect(hwnd, null(), 1);
        }
        self.fit_columns();
    }

    /// The width of each line of the edge the window paints around the path field and the Messages area, in the
    /// dark theme only. Their own client edge is themed light even with DarkMode_Explorer (which they keep for the
    /// dark scroll bar), so there they have none and sit inside this one.
    fn painted_edge(&self) -> Option<i32> {
        self.dark.get().then(|| layout::edge(self.dpi.get()))
    }

    /// Column widths for the list's current width: Name and Override take what is left (or give it back).
    fn fit_columns(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: plain list queries and messages.
        unsafe {
            GetClientRect(c.list, &mut client);
            let widths = layout::column_widths(client.right - client.left, self.dpi.get());
            for (index, width) in widths.iter().enumerate() {
                SendMessageW(c.list, LVM_SETCOLUMNWIDTH, index, *width as LPARAM);
            }
        }
    }

    fn erase(&self, hdc: HDC) {
        let hwnd = self.hwnd.get();
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let bg = self.gdi.borrow().colors.bg;
        // SAFETY: `client` is a valid out-pointer.
        unsafe { GetClientRect(hwnd, &mut client) };
        paint::fill(hdc, &client, bg);
    }

    /// The parts no control draws: the banner's colored area and the section frames with their titles.
    fn paint(&self) {
        let hwnd = self.hwnd.get();
        // Everything that could fail is read before BeginPaint, so EndPaint always follows it.
        let titles = [self.text("gpus"), self.text("file"), self.text("messages")];
        let (colors, font) = {
            let gdi = self.gdi.borrow();
            (gdi.colors, gdi.font.handle())
        };
        let (layout, banner_shown, dpi) = (self.layout.get(), self.banner_shown.get(), self.dpi.get());
        let edge = self.painted_edge();
        // SAFETY: all-zero is a valid PAINTSTRUCT for BeginPaint to fill.
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        // SAFETY: WM_PAINT of our window; EndPaint below.
        let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
        if let (false, Some(l)) = (hdc.is_null(), layout) {
            if let Some(banner) = l.banner.filter(|_| banner_shown) {
                paint::fill(hdc, &paint::win_rect(banner), colors.banner_bg);
            }
            for (section, title) in [l.gpus, l.files, l.messages].iter().zip(titles) {
                paint::section(hdc, section, title, &colors, font, dpi);
            }
            if let Some(width) = edge {
                paint::edge(hdc, l.path, colors.border, colors.field, width);
                paint::edge(hdc, l.log, colors.border, colors.log_bg, width);
            }
        }
        // SAFETY: pairs with BeginPaint.
        unsafe { EndPaint(hwnd, &ps) };
    }

    fn paint_header(&self, header: HWND, given: HDC) {
        let colors = self.gdi.borrow().colors;
        let dpi = self.dpi.get();
        let labels = COLUMN_KEYS.map(|key| self.text(key));
        if !given.is_null() {
            paint::header(header, given, &colors, dpi, &labels);
            return;
        }
        // SAFETY: all-zero is a valid PAINTSTRUCT for BeginPaint to fill.
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        // SAFETY: WM_PAINT of the header; EndPaint below.
        let hdc = unsafe { BeginPaint(header, &mut ps) };
        if !hdc.is_null() {
            paint::header(header, hdc, &colors, dpi, &labels);
        }
        // SAFETY: pairs with BeginPaint.
        unsafe { EndPaint(header, &ps) };
    }

    /// WM_CTLCOLOR*: the banner, the Messages area, the validation line (green or red), labels and, in the dark
    /// theme, the path field. None leaves the default (the light read-only path field stays native).
    fn control_color(&self, hdc: HDC, control: HWND) -> Option<LRESULT> {
        let c = self.controls.get().copied()?;
        let gdi = self.gdi.borrow();
        let colors = gdi.colors;
        let (fg, bg, brush) = if control == c.banner {
            (colors.banner_fg, colors.banner_bg, gdi.brushes.banner.handle())
        } else if control == c.log {
            (colors.log_fg, colors.log_bg, gdi.brushes.log.handle())
        } else if control == c.path {
            if !colors.dark {
                return None;
            }
            (colors.field_fg, colors.field, gdi.brushes.field.handle())
        } else if control == c.validation {
            let fg = if self.validation_ok.get() {
                colors.ok
            } else {
                colors.bad
            };
            (fg, colors.bg, gdi.brushes.bg.handle())
        } else {
            (colors.fg, colors.bg, gdi.brushes.bg.handle())
        };
        if brush.is_null() {
            return None;
        }
        // SAFETY: the DC the control is painting with.
        unsafe {
            windows_sys::Win32::Graphics::Gdi::SetTextColor(hdc, fg.colorref());
            windows_sys::Win32::Graphics::Gdi::SetBkColor(hdc, bg.colorref());
        }
        Some(brush as LRESULT)
    }

    fn draw_item(&self, item: *const DRAWITEMSTRUCT) -> Option<LRESULT> {
        let c = self.controls.get().copied()?;
        // SAFETY: WM_DRAWITEM's lParam points to a DRAWITEMSTRUCT valid during the message.
        let item = unsafe { item.as_ref()? };
        if item.CtlType != ODT_BUTTON {
            return None;
        }
        let menu = item.hwndItem == c.language || item.hwndItem == c.theme;
        let label = window_text(item.hwndItem);
        let (colors, font) = {
            let gdi = self.gdi.borrow();
            (gdi.colors, gdi.font.handle())
        };
        paint::button(item, &label, &colors, font, menu, self.dpi.get());
        Some(1)
    }

    /// The arrow of the native (light theme) menu buttons, drawn after the button paints itself.
    fn menu_button_draw(&self, draw: &NMCUSTOMDRAW) -> LRESULT {
        if self.dark.get() {
            return CDRF_DODEFAULT as LRESULT;
        }
        match draw.dwDrawStage {
            CDDS_PREPAINT => CDRF_NOTIFYPOSTPAINT as LRESULT,
            CDDS_POSTPAINT => {
                // SAFETY: plain queries.
                let enabled = unsafe { IsWindowEnabled(draw.hdr.hwndFrom) } != 0;
                let index = if enabled { COLOR_BTNTEXT } else { COLOR_GRAYTEXT };
                // SAFETY: plain query.
                let color = Rgb::from_colorref(unsafe { GetSysColor(index) });
                paint::arrow(
                    draw.hdc,
                    layout::menu_arrow(paint::layout_rect(&draw.rc), self.dpi.get()),
                    color,
                );
                CDRF_DODEFAULT as LRESULT
            }
            _ => CDRF_DODEFAULT as LRESULT,
        }
    }

    // ---- messages ---------------------------------------------------------------------------------------------

    fn handle(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_GETMINMAXINFO => {
                let dpi = window_dpi(hwnd);
                let (width, height) = layout::min_client(dpi);
                let (mut width, mut height) = window_size(hwnd, width, height, dpi);
                // The minimum is in DIPs (780 x 540 at 96 DPI); at a high scale on a small screen it would not fit,
                // and the bottom of the window would stay under the taskbar.
                if let Some(work) = work_area(hwnd) {
                    width = width.min(work.right - work.left);
                    height = height.min(work.bottom - work.top);
                }
                // SAFETY: WM_GETMINMAXINFO's lParam points to a MINMAXINFO to fill.
                if let Some(info) = unsafe { (lparam as *mut MINMAXINFO).as_mut() } {
                    info.ptMinTrackSize = POINT { x: width, y: height };
                }
                Some(0)
            }
            WM_SIZE => {
                self.layout();
                Some(0)
            }
            WM_ERASEBKGND => {
                self.erase(wparam as HDC);
                Some(1)
            }
            WM_PAINT => {
                self.paint();
                Some(0)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => self.control_color(wparam as HDC, lparam as HWND),
            WM_DRAWITEM => self.draw_item(lparam as *const DRAWITEMSTRUCT),
            WM_NOTIFY => self.notify(lparam as *const NMHDR),
            WM_COMMAND => {
                // Buttons notify BN_CLICKED; edits send other codes, which are ignored.
                let (id, code) = ((wparam & 0xFFFF) as u16, ((wparam >> 16) & 0xFFFF) as u32);
                if code == BN_CLICKED {
                    self.command(id);
                }
                Some(0)
            }
            WM_TIMER => {
                self.timer(wparam);
                Some(0)
            }
            WM_APP_RESULT => {
                self.take_result();
                Some(0)
            }
            WM_APP_SELECTION => {
                self.selection_changed();
                Some(0)
            }
            WM_DPICHANGED => {
                // SAFETY: WM_DPICHANGED's lParam points to the suggested window rectangle.
                let suggested = unsafe { (lparam as *const RECT).as_ref() }.copied();
                self.dpi_changed((wparam & 0xFFFF) as u32, suggested);
                Some(0)
            }
            WM_ACTIVATE => {
                if (wparam & 0xFFFF) as u32 == WA_INACTIVE {
                    // SAFETY: plain focus queries.
                    unsafe {
                        let focus = GetFocus();
                        if !focus.is_null() && IsChild(hwnd, focus) != 0 {
                            self.focus.set(focus);
                        }
                    }
                }
                None
            }
            WM_SETFOCUS => {
                self.restore_focus();
                Some(0)
            }
            WM_CLOSE => {
                // A load, unload or verify in progress finishes first.
                if !self.busy() {
                    // SAFETY: our window.
                    unsafe { DestroyWindow(hwnd) };
                }
                Some(0)
            }
            WM_QUERYENDSESSION => Some(LRESULT::from(!self.busy())),
            WM_NEXTDLGCTL => {
                // The Messages area (a multi-line edit) asks for the next or previous control on Tab and
                // Shift+Tab, or names one when the low word of lParam is set.
                // SAFETY: plain focus calls on our own window and its controls.
                unsafe {
                    let target = if lparam & 0xFFFF != 0 {
                        wparam as HWND
                    } else {
                        GetNextDlgTabItem(hwnd, GetFocus(), BOOL::from(wparam != 0))
                    };
                    if !target.is_null() {
                        SetFocus(target);
                    }
                }
                Some(0)
            }
            _ => None,
        }
    }

    fn notify(&self, header: *const NMHDR) -> Option<LRESULT> {
        let c = self.controls.get().copied()?;
        // SAFETY: WM_NOTIFY's lParam points to an NMHDR (the start of the notification's structure).
        let hdr = unsafe { header.as_ref()? };
        if hdr.hwndFrom == c.list && hdr.code == LVN_ITEMCHANGED {
            // SAFETY: LVN_ITEMCHANGED comes with an NMLISTVIEW.
            let change = unsafe { &*(header as *const NMLISTVIEW) };
            self.list_changed(change);
            return Some(0);
        }
        if hdr.code == NM_CUSTOMDRAW && (hdr.hwndFrom == c.language || hdr.hwndFrom == c.theme) {
            // SAFETY: a button's NM_CUSTOMDRAW comes with an NMCUSTOMDRAW.
            let draw = unsafe { &*(header as *const NMCUSTOMDRAW) };
            return Some(self.menu_button_draw(draw));
        }
        None
    }

    fn command(&self, id: u16) {
        // Enter (IDOK from IsDialogMessageW) presses the focused button.
        let id = if id == IDOK as u16 {
            self.focused_button().unwrap_or(0)
        } else {
            id
        };
        match id {
            ID_LANGUAGE => self.language_menu(),
            ID_THEME => self.theme_menu(),
            ID_REFRESH => self.refresh(),
            ID_FILE => self.choose_file(),
            ID_LOAD => self.load(),
            ID_REMOVE => self.remove(),
            ID_VERIFY => self.verify(),
            _ => {}
        }
    }

    fn focused_button(&self) -> Option<u16> {
        let c = self.controls.get().copied()?;
        // SAFETY: plain focus queries.
        unsafe {
            let focus = GetFocus();
            if !c.buttons().contains(&focus) || IsWindowEnabled(focus) == 0 {
                return None;
            }
            u16::try_from(GetDlgCtrlID(focus)).ok()
        }
    }

    fn timer(&self, id: usize) {
        if id == TIMER_RESULT {
            self.take_result();
            return;
        }
        let hwnd = self.hwnd.get();
        let Some(index) = id.checked_sub(TIMER_SMOKE) else {
            return;
        };
        let Some((_, step)) = self.smoke_steps.get().and_then(|steps| steps.get(index).copied()) else {
            return;
        };
        // SAFETY: the smoke timers are one-shot.
        unsafe { KillTimer(hwnd, id) };
        match step {
            Step::Language(language) => self.set_language(language),
            Step::Theme(theme) => self.set_theme(theme),
            // gui.py's root.destroy: closes even while busy.
            // SAFETY: our window.
            Step::Close => unsafe {
                DestroyWindow(hwnd);
            },
        }
    }

    fn dpi_changed(&self, dpi: u32, suggested: Option<RECT>) {
        let hwnd = self.hwnd.get();
        self.dpi.set(dpi.max(1));
        self.make_font();
        if let Some(r) = suggested {
            // SAFETY: our window, to the rectangle Windows suggests for the new DPI.
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
        self.layout();
        // SAFETY: our window.
        unsafe {
            RedrawWindow(
                hwnd,
                null(),
                null_mut(),
                RDW_ERASE | RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME,
            )
        };
    }

    /// Give the focus back to the control that had it, or to the list (the window is not a dialog).
    fn restore_focus(&self) {
        let Some(c) = self.controls.get().copied() else { return };
        let saved = self.focus.get();
        // SAFETY: plain window queries and SetFocus on our own controls.
        unsafe {
            let usable =
                !saved.is_null() && IsWindow(saved) != 0 && IsWindowEnabled(saved) != 0 && IsWindowVisible(saved) != 0;
            SetFocus(if usable { saved } else { c.list });
        }
    }

    /// A handler panicked: log it like gui.py's callback_error and remember the failure.
    fn caught(&self, payload: Box<dyn Any + Send>) {
        self.failed.set(true);
        let template = match self.controller.try_borrow() {
            Ok(controller) => controller.text("unexpected"),
            Err(_) => crate::controller::text(Language::En, "unexpected"),
        };
        self.log(&[text::unexpected_panic(template, &*payload)]);
    }
}

/// The window procedure: finds the App, runs the handler inside catch_unwind.
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: WM_NCCREATE's lParam is the CREATESTRUCTW; lpCreateParams is the App passed to CreateWindowExW.
        unsafe {
            if let Some(create) = (lparam as *const CREATESTRUCTW).as_ref() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                if let Some(app) = (create.lpCreateParams as *const App).as_ref() {
                    app.hwnd.set(hwnd);
                }
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    // SAFETY: reads the pointer stored at WM_NCCREATE (null before it and after WM_NCDESTROY).
    let app = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const App;
    // SAFETY: the App outlives the window: `run` drops it only after the window is destroyed.
    let Some(app) = (unsafe { app.as_ref() }) else {
        // SAFETY: default processing.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        WM_DESTROY => {
            // Outside catch_unwind on purpose: the message loop must end whatever happens.
            // SAFETY: plain call.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        WM_NCDESTROY => {
            // SAFETY: the window is going away: no message may reach the App through it any more.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
        }
        _ => {}
    }
    match catch_unwind(AssertUnwindSafe(|| app.handle(hwnd, msg, wparam, lparam))) {
        Ok(Some(result)) => result,
        // SAFETY: default processing.
        Ok(None) => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        Err(payload) => {
            let _ = catch_unwind(AssertUnwindSafe(|| app.caught(payload)));
            if msg == WM_CLOSE {
                0
            } else {
                // SAFETY: default processing.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
    }
}

/// The list header's subclass: in the dark theme it is painted by `paint::header`; everything else goes to the
/// header's own window procedure.
unsafe extern "system" fn header_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let (app, original) = HEADER.try_with(Cell::get).unwrap_or((0, 0));
    // SAFETY: `original` is the value GWLP_WNDPROC held before header_proc replaced it: a window procedure.
    let original: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(original) };
    let pass_on = || {
        // SAFETY: the header's own procedure with the message as received (DefWindowProcW if it is unknown).
        unsafe {
            match original {
                Some(_) => CallWindowProcW(original, hwnd, msg, wparam, lparam),
                None => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    };
    if msg == WM_NCDESTROY {
        if let Some(procedure) = original {
            // SAFETY: the header is going away: give it back its own procedure first.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, procedure as *const () as isize) };
        }
        let _ = HEADER.try_with(|state| state.set((0, 0)));
        return pass_on();
    }
    // SAFETY: the App outlives the header (cleared above when the header is destroyed).
    let Some(app) = (unsafe { (app as *const App).as_ref() }) else {
        return pass_on();
    };
    let handled = catch_unwind(AssertUnwindSafe(|| {
        if !app.dark.get() {
            return None;
        }
        match msg {
            WM_ERASEBKGND => Some(1),
            WM_PAINT | WM_PRINTCLIENT => {
                app.paint_header(hwnd, wparam as HDC);
                Some(0)
            }
            _ => None,
        }
    }));
    match handled {
        Ok(Some(result)) => result,
        Ok(None) => pass_on(),
        Err(payload) => {
            let _ = catch_unwind(AssertUnwindSafe(|| app.caught(payload)));
            pass_on()
        }
    }
}

// ---- small Win32 helpers -------------------------------------------------------------------------------------

fn last_error() -> u32 {
    // SAFETY: plain call.
    unsafe { GetLastError() }
}

fn window_dpi(hwnd: HWND) -> u32 {
    // SAFETY: plain query; 0 for an invalid window.
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi,
    }
}

/// The work area (the screen without the taskbar) of the monitor the window is on.
fn work_area(hwnd: HWND) -> Option<RECT> {
    // SAFETY: `hwnd` is our window; `info` is a valid MONITORINFO with cbSize set.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcWork)
    }
}

/// The window size for a client area of `width` x `height` at `dpi`.
fn window_size(hwnd: HWND, width: i32, height: i32, dpi: u32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: plain queries; `rect` is a valid in/out pointer.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        AdjustWindowRectExForDpi(&mut rect, style, 0, ex_style, dpi);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: plain queries; the buffer has room for the text and its NUL.
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buffer = vec![0u16; len + 1];
        let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied.min(len)])
    }
}

/// SetWindowTextW only when the text changes (no flicker, and the path field keeps its scroll position).
fn set_text(hwnd: HWND, text: &str) {
    if window_text(hwnd) != text {
        let units = wide(text);
        // SAFETY: `units` is NUL-terminated and outlives the call.
        unsafe { SetWindowTextW(hwnd, units.as_ptr()) };
    }
}

fn select_row(list: HWND, position: usize) {
    // SAFETY: all-zero is a valid LVITEMW; plain list messages.
    unsafe {
        let mut state: LVITEMW = std::mem::zeroed();
        state.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        state.state = LVIS_SELECTED | LVIS_FOCUSED;
        SendMessageW(list, LVM_SETITEMSTATE, position, &state as *const LVITEMW as LPARAM);
        SendMessageW(list, LVM_ENSUREVISIBLE, position, 0);
    }
}

fn system_colors() -> SystemColors {
    // SAFETY: plain queries.
    let color = |index| Rgb::from_colorref(unsafe { GetSysColor(index) });
    SystemColors {
        face: color(COLOR_BTNFACE),
        text: color(COLOR_WINDOWTEXT),
        window: color(COLOR_WINDOW),
        gray: color(COLOR_GRAYTEXT),
    }
}

/// True when Windows asks apps for the dark mode: HKCU\...\Themes\Personalize AppsUseLightTheme is a REG_DWORD 0.
/// Any error means light.
fn windows_dark_mode() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the value is read into a u32 of the size passed; the strings are static.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut value as *mut u32 as *mut c_void,
            &mut size,
        )
    };
    status == 0 && size == 4 && value == 0
}

/// Ask Windows to draw the title bar dark or light, like the rest of the window (gui.py's dark_title_bar).
fn dark_title_bar(hwnd: HWND, dark: bool) {
    let value = BOOL::from(dark);
    // Windows 10 20H1 and later use 20; 19 before.
    for attribute in [DWMWA_USE_IMMERSIVE_DARK_MODE as u32, 19] {
        // SAFETY: the attribute value is a BOOL of the size passed.
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute,
                &value as *const BOOL as *const c_void,
                size_of::<BOOL>() as u32,
            )
        };
        if result == 0 {
            break;
        }
    }
    // Redraw the frame now.
    // SAFETY: our window.
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    };
}

/// Dark or light popup menus through uxtheme's SetPreferredAppMode (ordinal 135) and FlushMenuThemes (136), which
/// exist from Windows 10 1903 (build 18362). Undocumented, so only on those builds, and any failure is ignored:
/// the menus then stay light.
fn dark_menus(dark: bool) {
    if windows_build() < 18362 {
        return;
    }
    // SAFETY: uxtheme.dll is loaded (SetWindowTheme is imported); ordinals are passed as MAKEINTRESOURCEA. On
    // these builds 135 is `int SetPreferredAppMode(int)` (2 force dark, 3 force light) and 136
    // `void FlushMenuThemes(void)`.
    unsafe {
        let module = GetModuleHandleW(w!("uxtheme.dll"));
        if module.is_null() {
            return;
        }
        let set_mode = GetProcAddress(module, 135usize as PCSTR);
        let flush = GetProcAddress(module, 136usize as PCSTR);
        if let (Some(set_mode), Some(flush)) = (set_mode, flush) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            set_mode(if dark { 2 } else { 3 });
            flush();
        }
    }
}

/// The Windows build number from ntdll's RtlGetVersion (GetVersionEx depends on the manifest); 0 if unknown.
fn windows_build() -> u32 {
    // SAFETY: ntdll.dll is always loaded; RtlGetVersion(OSVERSIONINFOW*) -> NTSTATUS fills the structure whose
    // size field is set.
    unsafe {
        let module = GetModuleHandleW(w!("ntdll.dll"));
        if module.is_null() {
            return 0;
        }
        let Some(get_version) = GetProcAddress(module, c"RtlGetVersion".as_ptr() as PCSTR) else {
            return 0;
        };
        let get_version: unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32 = std::mem::transmute(get_version);
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        if get_version(&mut info) == 0 {
            info.dwBuildNumber
        } else {
            0
        }
    }
}

/// The system message font (NONCLIENTMETRICS lfMessageFont) at `dpi`, else Segoe UI 9 pt.
fn message_font(dpi: u32) -> Owned {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is a NONCLIENTMETRICSW with cbSize set, as SPI_GETNONCLIENTMETRICS expects.
    let found = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut NONCLIENTMETRICSW as *mut c_void,
            0,
            dpi,
        )
    } != 0;
    let mut font = metrics.lfMessageFont;
    if !found {
        // Segoe UI 9 pt, the usual message font.
        font = LOGFONTW {
            lfHeight: -layout::scale(12, dpi),
            lfWeight: 400,
            lfCharSet: 1,
            ..Default::default()
        };
        for (slot, unit) in font.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
            *slot = unit;
        }
    }
    // SAFETY: a valid LOGFONTW; a failure gives null, the system font.
    Owned(unsafe { CreateFontIndirectW(&font) } as HGDIOBJ)
}

fn line_height(hwnd: HWND, font: HGDIOBJ) -> i32 {
    Measure::new(hwnd, font).map_or(16, |measure| measure.line())
}

/// A DC of the window with the message font selected, for measuring texts.
struct Measure {
    hwnd: HWND,
    hdc: HDC,
    previous: HGDIOBJ,
}

impl Measure {
    fn new(hwnd: HWND, font: HGDIOBJ) -> Option<Measure> {
        // SAFETY: a DC of our window, released in Drop.
        let hdc = unsafe { GetDC(hwnd) };
        if hdc.is_null() {
            return None;
        }
        let previous = if font.is_null() {
            null_mut()
        } else {
            // SAFETY: a live font into a valid DC; put back in Drop.
            unsafe { SelectObject(hdc, font) }
        };
        Some(Measure { hwnd, hdc, previous })
    }

    fn line(&self) -> i32 {
        // SAFETY: all-zero is a valid TEXTMETRICW for GetTextMetricsW to fill.
        let mut metrics: TEXTMETRICW = unsafe { std::mem::zeroed() };
        // SAFETY: valid DC and out-pointer.
        if unsafe { GetTextMetricsW(self.hdc, &mut metrics) } == 0 {
            return 16;
        }
        metrics.tmHeight.max(1)
    }

    fn width(&self, text: &str) -> i32 {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut size = windows_sys::Win32::Foundation::SIZE { cx: 0, cy: 0 };
        // SAFETY: valid DC; `units` is valid for its length.
        unsafe { GetTextExtentPoint32W(self.hdc, units.as_ptr(), units.len() as i32, &mut size) };
        size.cx
    }

    /// Height of `text` wrapped to `width` the way the static controls wrap it (SS_LEFT | SS_EDITCONTROL).
    fn wrapped_height(&self, text: &str, width: i32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let units = wide(text);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width.max(1),
            bottom: 0,
        };
        let flags = DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_EXPANDTABS | DT_NOPREFIX;
        // SAFETY: valid DC; `units` is NUL-terminated; `rect` is a valid in/out pointer.
        unsafe { DrawTextW(self.hdc, units.as_ptr(), -1, &mut rect, flags) };
        rect.bottom - rect.top
    }
}

impl Drop for Measure {
    fn drop(&mut self) {
        // SAFETY: puts back the DC's font and releases the DC obtained in `new`.
        unsafe {
            if !self.previous.is_null() {
                SelectObject(self.hdc, self.previous);
            }
            ReleaseDC(self.hwnd, self.hdc);
        }
    }
}
