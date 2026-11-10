//! NvAPI (nvapi64.dll): read back the VBIOS image that the driver is running (nvapi.py).
//!
//! NvAPI_GPU_GetVbiosImage returns the image the driver is running: in the driver's default mode its own copy, in
//! GSP firmware mode the GSP firmware's working copy. Either is the override when the driver accepted it, otherwise
//! the card's own image. nvapi64.dll is loaded only when an image is read, from the System32 directory only. Every
//! function comes from nvapi_QueryInterface(id), which returns NULL for a function this driver does not have; the
//! tests pass their own query function.

use std::ffi::{c_char, c_void};
use std::ptr;

use crate::error::{Error, Result};

/// The only buffer size the driver accepts; an image this long may be cut short.
pub const IMAGE_SIZE: usize = 0xFA000;
/// NV_GPU_VBIOS_IMAGE version 2, 16 bytes.
pub const IMAGE_VERSION: u32 = 0x0002_0010;
pub const MAX_PHYSICAL_GPUS: usize = 64;
/// NVAPI_OK.
pub const OK: i32 = 0;
/// NVAPI_NO_IMPLEMENTATION: this driver does not implement the function.
pub const NO_IMPLEMENTATION: i32 = -3;
/// NvAPI_ShortString: the buffer that NvAPI_GetErrorMessage fills.
pub const SHORT_STRING: usize = 64;

/// The request of NvAPI_GPU_GetVbiosImage, laid out as in nvapi.h: 16 bytes on x64.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug)]
pub struct NV_GPU_VBIOS_IMAGE {
    /// IMAGE_VERSION.
    pub version: u32,
    /// In: the size of `buffer` (IMAGE_SIZE). Out: the size of the image written there.
    pub size: u32,
    pub buffer: *mut c_void,
}

const _: () = assert!(size_of::<NV_GPU_VBIOS_IMAGE>() == if cfg!(target_pointer_width = "64") { 16 } else { 12 });

/// An NvAPI function: its name after "NvAPI_", its nvapi_QueryInterface id and its C prototype. Each id has
/// exactly one prototype here, so an address from the query is always called through the right one.
trait Function {
    const NAME: &'static str;
    const ID: u32;
    /// The `extern "C"` function pointer type of this function (every NvAPI function returns an NvAPI_Status).
    type Fn: Copy;
}

macro_rules! functions {
    ($($kind:ident $name:literal $id:literal fn($($arg:ty),*);)*) => {
        $(
            #[doc = concat!("NvAPI_", $name, ".")]
            enum $kind {}

            impl Function for $kind {
                const NAME: &'static str = $name;
                const ID: u32 = $id;
                type Fn = unsafe extern "C" fn($($arg),*) -> i32;
            }
        )*

        /// (name, nvapi_QueryInterface id) of every NvAPI function the loader calls (nvapi.FUNCTIONS).
        pub const FUNCTIONS: &[(&str, u32)] = &[$(($name, $id)),*];
    };
}

functions! {
    Initialize "Initialize" 0x0150_E828 fn();
    Unload "Unload" 0xD22B_DD7E fn();
    GetErrorMessage "GetErrorMessage" 0x6C2D_048C fn(i32, *mut c_char);
    EnumPhysicalGpus "EnumPhysicalGPUs" 0xE5AC_921F fn(*mut *mut c_void, *mut u32);
    GetBusId "GPU_GetBusId" 0x1BE0_B8E5 fn(*mut c_void, *mut u32);
    GetBusSlotId "GPU_GetBusSlotId" 0x2A0A_350F fn(*mut c_void, *mut u32);
    GetVbiosImage "GPU_GetVbiosImage" 0xFC13_EE11 fn(*mut c_void, *mut NV_GPU_VBIOS_IMAGE);
}

/// nvapi_QueryInterface: the address of the NvAPI function with this id, or null when the driver does not have it.
pub struct Query<'a> {
    query: Box<dyn Fn(u32) -> *const c_void + 'a>,
}

impl<'a> Query<'a> {
    /// A query function: the driver's nvapi_QueryInterface ([`Query::system32`]) or a fake in the tests.
    ///
    /// # Safety
    ///
    /// For each id in [`FUNCTIONS`], `query` must return null or the address of an `extern "C"` function with that
    /// function's nvapi.h prototype, and that function must stay callable while this `Query` is used.
    pub unsafe fn new(query: impl Fn(u32) -> *const c_void + 'a) -> Query<'a> {
        Query { query: Box::new(query) }
    }
}

#[cfg(windows)]
impl Query<'static> {
    /// nvapi_QueryInterface of the nvapi64.dll in the System32 directory (never the PATH or the program's folder).
    /// The DLL is loaded by the first successful call and stays loaded for the life of the process.
    pub fn system32() -> Result<Query<'static>> {
        let query_interface = system32::query_interface()?;
        let query = move |id: u32| {
            // SAFETY: nvapi_QueryInterface takes any id.
            unsafe { query_interface(id) }.cast_const()
        };
        // SAFETY: nvapi_QueryInterface returns null or the function with that id, which has the nvapi.h
        // prototype; nvapi64.dll is never unloaded, so the functions stay callable.
        Ok(unsafe { Query::new(query) })
    }
}

/// A physical GPU handle from NvAPI_EnumPhysicalGPUs.
#[derive(Clone, Copy, Debug)]
pub struct GpuHandle(*mut c_void);

/// NvAPI through a query function (nvapi.NvApi).
pub struct NvApi<'a> {
    query: Query<'a>,
}

impl<'a> NvApi<'a> {
    /// NvAPI through this query function; nothing is called yet.
    pub fn new(query: Query<'a>) -> NvApi<'a> {
        NvApi { query }
    }

    /// The function F, or the "this NVIDIA driver has no NvAPI_..." error when the query returns null.
    fn function<F: Function>(&self) -> Result<F::Fn> {
        let address = (self.query.query)(F::ID);
        if address.is_null() {
            return Err(missing(F::NAME));
        }
        const { assert!(size_of::<F::Fn>() == size_of::<*const c_void>()) };
        // SAFETY: F::Fn is a function pointer type of the same size as the address (checked above). By the
        // contract of Query::new, a non-null address for F::ID is a function with F's prototype, that is F::Fn.
        Ok(unsafe { std::mem::transmute_copy::<*const c_void, F::Fn>(&address) })
    }

    /// Call F (the closure makes the call) and turn its status into an error (nvapi.NvApi.call).
    fn call<F: Function>(&self, invoke: impl FnOnce(F::Fn) -> i32) -> Result<()> {
        let status = invoke(self.function::<F>()?);
        if status == NO_IMPLEMENTATION {
            return Err(missing(F::NAME));
        }
        if status != OK {
            return Err(Error::new(format!(
                "NvAPI_{} failed: {}",
                F::NAME,
                self.describe(status)
            )));
        }
        Ok(())
    }

    /// NvAPI_Initialize.
    pub fn initialize(&self) -> Result<()> {
        // SAFETY: NvAPI_Initialize takes no arguments.
        self.call::<Initialize>(|initialize| unsafe { initialize() })
    }

    /// NvAPI_Unload; its status is ignored, and so is a driver without it.
    pub fn unload(&self) {
        if let Ok(unload) = self.function::<Unload>() {
            // SAFETY: NvAPI_Unload takes no arguments.
            unsafe { unload() };
        }
    }

    /// "<NvAPI_GetErrorMessage text> (<status>)", or "status <status>" when the driver has no text for it.
    pub fn describe(&self, status: i32) -> String {
        let mut text: [c_char; SHORT_STRING] = [0; SHORT_STRING];
        if let Ok(error_message) = self.function::<GetErrorMessage>() {
            // SAFETY: text is the NvAPI_ShortString (SHORT_STRING bytes) that the function fills.
            if unsafe { error_message(status, text.as_mut_ptr()) } == OK {
                // Up to the first NUL (all 64 bytes without one), decoded as ASCII with 'replace'.
                let message: String = text
                    .iter()
                    .map(|&c| c as u8)
                    .take_while(|&b| b != 0)
                    .map(|b| {
                        if b.is_ascii() {
                            char::from(b)
                        } else {
                            char::REPLACEMENT_CHARACTER
                        }
                    })
                    .collect();
                if !message.is_empty() {
                    return format!("{message} ({status})");
                }
            }
        }
        format!("status {status}")
    }

    /// The physical GPU handle at PCI location "bb:dd.f": NvAPI gives the bus and the device (slot) number.
    pub fn find(&self, location: Option<&str>) -> Result<GpuHandle> {
        let Some(location) = location else {
            return Err(Error::new(
                "the PCI location of this GPU is unknown, so it cannot be found through NvAPI",
            ));
        };
        let (bus, device) = (int16(char_slice(location, 0, 2))?, int16(char_slice(location, 3, 5))?);
        let mut handles = [ptr::null_mut::<c_void>(); MAX_PHYSICAL_GPUS];
        let mut count = 0u32;
        // SAFETY: handles has room for NVAPI_MAX_PHYSICAL_GPUS handles and count is a u32, as the function expects.
        self.call::<EnumPhysicalGpus>(|enumerate| unsafe { enumerate(handles.as_mut_ptr(), &mut count) })?;
        let mut found = Vec::new();
        for &handle in &handles[..(count as usize).min(MAX_PHYSICAL_GPUS)] {
            let (mut bus_id, mut slot_id) = (0u32, 0u32);
            let get_bus = self.function::<GetBusId>()?;
            // SAFETY: a handle from NvAPI_EnumPhysicalGPUs and a u32 for the result.
            if unsafe { get_bus(handle, &mut bus_id) } != OK {
                continue;
            }
            let get_slot = self.function::<GetBusSlotId>()?;
            // SAFETY: as above.
            if unsafe { get_slot(handle, &mut slot_id) } == OK {
                found.push((handle, bus_id, slot_id));
            }
        }
        let matches: Vec<_> = found
            .iter()
            .filter(|&&(_, b, d)| (i128::from(b), i128::from(d)) == (bus, device))
            .collect();
        if let [&(handle, _, _)] = matches[..] {
            return Ok(GpuHandle(handle));
        }
        let listed: Vec<String> = found.iter().map(|(_, b, d)| format!("{b:02x}:{d:02x}")).collect();
        let listed = if listed.is_empty() {
            "none".to_string()
        } else {
            listed.join(", ")
        };
        Err(Error::new(format!(
            "NvAPI does not list exactly one GPU at PCI {} (it lists: {listed})",
            char_slice(location, 0, 5)
        )))
    }

    /// The VBIOS image the driver is running on this GPU, read into an IMAGE_SIZE buffer.
    pub fn image(&self, handle: GpuHandle) -> Result<Vec<u8>> {
        let mut buffer = vec![0u8; IMAGE_SIZE];
        let mut request = NV_GPU_VBIOS_IMAGE {
            version: IMAGE_VERSION,
            size: IMAGE_SIZE as u32,
            buffer: buffer.as_mut_ptr().cast(),
        };
        // SAFETY: a handle from NvAPI_EnumPhysicalGPUs and a version 2 request whose buffer has `size` writable
        // bytes; the request and the buffer outlive the call.
        self.call::<GetVbiosImage>(|get_image| unsafe { get_image(handle.0, &mut request) })?;
        let size = request.size as usize;
        if !(0 < size && size <= IMAGE_SIZE) {
            return Err(Error::new(format!(
                "NvAPI_GPU_GetVbiosImage returned an invalid size ({} bytes)",
                request.size
            )));
        }
        buffer.truncate(size);
        Ok(buffer)
    }
}

fn missing(name: &str) -> Error {
    Error::new(format!(
        "this NVIDIA driver has no NvAPI_{name}; reading the running VBIOS needs a driver that has it, for example \
         617.42"
    ))
}

/// text[start:end] in characters, as Python slices a str.
fn char_slice(text: &str, start: usize, end: usize) -> &str {
    let at = |n: usize| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
    &text[at(start)..at(end).max(at(start))]
}

/// Python int(text, 16) of a location slice, with Python's ValueError text when it is not a number.
fn int16(text: &str) -> Result<i128> {
    crate::pyfmt::parse_int(text, 16).ok_or_else(|| {
        Error::new(format!(
            "invalid literal for int() with base 16: {}",
            crate::pyfmt::repr_str(text)
        ))
    })
}

/// The VBIOS image the driver is running on the GPU at PCI location "bb:dd.f" (`Adapter::location`):
/// NvAPI_Initialize, the image, then NvAPI_Unload whatever happened (nothing to unload when Initialize fails).
pub fn read_vbios_image(location: Option<&str>, query: Query<'_>) -> Result<Vec<u8>> {
    let api = NvApi::new(query);
    api.initialize()?;
    let image = api.find(location).and_then(|handle| api.image(handle));
    api.unload();
    image
}

/// The full path of nvapi64.dll in this System32 directory (from GetSystemDirectoryW), joined as ntpath.join does.
pub fn dll_path(system_directory: &str) -> String {
    if system_directory.is_empty() || system_directory.ends_with(['\\', '/']) {
        format!("{system_directory}nvapi64.dll")
    } else {
        format!("{system_directory}\\nvapi64.dll")
    }
}

/// The error when nvapi64.dll or its nvapi_QueryInterface cannot be loaded; `detail` is what ctypes would say.
#[cfg_attr(not(windows), allow(dead_code))]
fn load_error(detail: &str) -> Error {
    Error::new(format!(
        "cannot load nvapi64.dll ({detail}); is the NVIDIA driver installed?"
    ))
}

/// Error texts of Windows calls as Python shows them, from the text FormatMessageW returned (None when it failed).
/// Pure, so they are tested on any system; `win32` uses them too.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod oserror {
    /// ctypes.FormatError(code).strip(); "<no description>" when FormatMessageW failed.
    pub(crate) fn format_error(message: Option<&str>) -> String {
        message.map_or_else(
            || "<no description>".to_string(),
            |text| crate::pyfmt::strip(text).to_string(),
        )
    }

    /// str(ctypes.WinError(code)): "[WinError <code>] <text>", the code as the C int that ctypes keeps.
    pub(crate) fn win_error(code: u32, message: Option<&str>) -> String {
        format!("[WinError {}] {}", code as i32, format_error(message))
    }

    /// str(OSError(status, f'{what} failed: {ctypes.FormatError(status).strip()}')) (win32.RegistryKey._check).
    pub(crate) fn errno_error(status: u32, what: &str, message: Option<&str>) -> String {
        format!("[Errno {}] {what} failed: {}", status as i32, format_error(message))
    }

    /// str() of the OSError of PyErr_SetFromWindowsErr(code): trailing spaces, line ends and dots removed.
    pub(crate) fn windows_err(code: u32, message: Option<&str>) -> String {
        match message {
            Some(text) => format!(
                "[WinError {}] {}",
                code as i32,
                text.trim_end_matches(|c| c <= ' ' || c == '.')
            ),
            None => format!("Windows Error {code:#x}"),
        }
    }

    /// What ctypes says when LoadLibraryExW fails with ERROR_MOD_NOT_FOUND.
    pub(crate) fn module_not_found(path: &str) -> String {
        let path: String = path.chars().take(500).collect();
        format!(
            "Could not find module '{path}' (or one of its dependencies). Try using the full path with constructor \
             syntax."
        )
    }
}

/// Loading nvapi64.dll from System32 (Windows only).
#[cfg(windows)]
mod system32 {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{GetLastError, ERROR_MOD_NOT_FOUND};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    use super::oserror::{module_not_found, windows_err};
    use super::{dll_path, load_error};
    use crate::error::Result;
    use crate::win32::system_message;

    /// `void *nvapi_QueryInterface(unsigned int id)` (cdecl, the only calling convention on x64).
    pub(super) type QueryInterface = unsafe extern "C" fn(u32) -> *mut c_void;

    static QUERY: OnceLock<QueryInterface> = OnceLock::new();

    /// nvapi_QueryInterface, loading nvapi64.dll on the first successful call (a failure is tried again next time,
    /// as Python does).
    pub(super) fn query_interface() -> Result<QueryInterface> {
        if let Some(&query) = QUERY.get() {
            return Ok(query);
        }
        let query = load().map_err(|detail| load_error(&detail))?;
        Ok(*QUERY.get_or_init(|| query))
    }

    fn last_error() -> u32 {
        // SAFETY: no arguments; called right after the call that failed.
        unsafe { GetLastError() }
    }

    fn system_directory() -> Result<String, String> {
        let mut buffer = vec![0u16; 260];
        loop {
            // SAFETY: the buffer has buffer.len() characters, the size passed.
            let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
            if length == 0 {
                let code = last_error();
                return Err(windows_err(code, system_message(code).as_deref()));
            }
            if length < buffer.len() {
                return Ok(String::from_utf16_lossy(&buffer[..length]));
            }
            buffer.resize(length, 0); // too small: length is the size needed, NUL included
        }
    }

    /// nvapi_QueryInterface, or the text ctypes would put in the error (FileNotFoundError, WinError or
    /// AttributeError).
    fn load() -> Result<QueryInterface, String> {
        let path = dll_path(&system_directory()?);
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated full path and a null file handle. LOAD_LIBRARY_SEARCH_SYSTEM32: the DLL's own
        // dependencies come from System32 only.
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            let code = last_error();
            return Err(if code == ERROR_MOD_NOT_FOUND {
                module_not_found(&path)
            } else {
                windows_err(code, system_message(code).as_deref())
            });
        }
        // The module is never freed: the functions that the query returns must stay callable.
        // SAFETY: a loaded module and a NUL-terminated name.
        let Some(address) = (unsafe { GetProcAddress(module, c"nvapi_QueryInterface".as_ptr().cast()) }) else {
            return Err("function 'nvapi_QueryInterface' not found".to_string());
        };
        // SAFETY: the export nvapi_QueryInterface has the QueryInterface prototype; GetProcAddress returns every
        // export as the same generic function pointer type, so this only gives the pointer its real type.
        Ok(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, QueryInterface>(address) })
    }
}

#[cfg(test)]
mod tests {
    //! NvApiTests of tests/test_windows_loader_verify.py: fake NvAPI functions (real `extern "C"` functions, called
    //! through pointers from the query as on Windows) behind a fake query. Each test thread has its own fake state.

    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    type Proto<F> = <F as Function>::Fn;
    type Gpu = (u32, u32, Vec<u8>);

    const MESSAGES: &[(i32, &[u8])] = &[
        (-174, b"NVAPI_INSUFFICIENT_BUFFER"),
        (-101, b"NVAPI_EXPECTED_PHYSICAL_GPU_HANDLE"),
        (-6, b"NVAPI_NVIDIA_DEVICE_NOT_FOUND"),
        (-200, b"caf\xc3\xa9"),
        (-201, &[b'A'; SHORT_STRING]),
    ];

    /// GPUs as (bus, slot, image); `missing` functions are absent, `status` makes a function fail, and
    /// `handle_status` makes GPU_GetBusId or GPU_GetBusSlotId fail for one handle.
    #[derive(Default)]
    struct Fake {
        gpus: Vec<Gpu>,
        missing: Vec<&'static str>,
        status: HashMap<&'static str, i32>,
        handle_status: HashMap<(&'static str, usize), i32>,
        calls: Vec<&'static str>,
    }

    thread_local! {
        static FAKE: RefCell<Fake> = RefCell::new(Fake::default());
    }

    fn with<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
        FAKE.with(|fake| f(&mut fake.borrow_mut()))
    }

    fn install(gpus: &[(u32, u32, &[u8])]) {
        let gpus = gpus
            .iter()
            .map(|&(bus, slot, image)| (bus, slot, image.to_vec()))
            .collect();
        with(|fake| {
            *fake = Fake {
                gpus,
                ..Fake::default()
            }
        });
    }

    fn calls() -> Vec<&'static str> {
        with(|fake| fake.calls.clone())
    }

    /// Run a fake's body: a panic (a bug in a test) must not unwind out of an `extern "C"` function.
    fn guard(body: impl FnOnce() -> i32) -> i32 {
        catch_unwind(AssertUnwindSafe(body)).unwrap_or(-999)
    }

    fn enter(name: &'static str) -> i32 {
        with(|fake| {
            fake.calls.push(name);
            fake.status.get(name).copied().unwrap_or(0)
        })
    }

    fn handle_of(index: usize) -> *mut c_void {
        ptr::without_provenance_mut(0x1000 + index)
    }

    fn gpu(fake: &Fake, handle: *mut c_void) -> Option<&Gpu> {
        handle.addr().checked_sub(0x1000).and_then(|index| fake.gpus.get(index))
    }

    extern "C" fn initialize() -> i32 {
        guard(|| enter("Initialize"))
    }

    extern "C" fn unload() -> i32 {
        guard(|| enter("Unload"))
    }

    extern "C" fn error_message(status: i32, text: *mut c_char) -> i32 {
        guard(|| {
            let Some(&(_, message)) = MESSAGES.iter().find(|&&(s, _)| s == status) else {
                return -5;
            };
            let mut bytes = message.to_vec();
            bytes.push(0);
            bytes.truncate(SHORT_STRING);
            // SAFETY: text is the caller's NvAPI_ShortString of SHORT_STRING bytes.
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), text, bytes.len()) };
            0
        })
    }

    extern "C" fn enum_gpus(handles: *mut *mut c_void, count: *mut u32) -> i32 {
        guard(|| {
            let gpus = with(|fake| fake.gpus.len());
            for index in 0..gpus {
                // SAFETY: the caller's array has MAX_PHYSICAL_GPUS entries; the tests have fewer GPUs.
                unsafe { handles.add(index).write(handle_of(index)) };
            }
            // SAFETY: count is the caller's u32.
            unsafe { count.write(gpus as u32) };
            enter("EnumPhysicalGPUs")
        })
    }

    fn ids(handle: *mut c_void, value: *mut u32, name: &'static str, pick: fn(&Gpu) -> u32) -> i32 {
        guard(|| {
            with(|fake| {
                let Some(gpu) = gpu(fake, handle) else {
                    return -101;
                };
                // SAFETY: value is the caller's u32.
                unsafe { value.write(pick(gpu)) };
                fake.handle_status.get(&(name, handle.addr())).copied().unwrap_or(0)
            })
        })
    }

    extern "C" fn bus_id(handle: *mut c_void, value: *mut u32) -> i32 {
        ids(handle, value, "GPU_GetBusId", |gpu| gpu.0)
    }

    extern "C" fn slot_id(handle: *mut c_void, value: *mut u32) -> i32 {
        ids(handle, value, "GPU_GetBusSlotId", |gpu| gpu.1)
    }

    extern "C" fn vbios_image(handle: *mut c_void, request: *mut NV_GPU_VBIOS_IMAGE) -> i32 {
        guard(|| {
            let status = enter("GPU_GetVbiosImage");
            if status != 0 {
                return status;
            }
            with(|fake| {
                // SAFETY: request is the caller's NV_GPU_VBIOS_IMAGE.
                let request = unsafe { &mut *request };
                let Some(gpu) = gpu(fake, handle) else {
                    return -101;
                };
                if request.version != 0x0002_0010 {
                    return -9;
                }
                if request.buffer.is_null() || request.size != 0xFA000 {
                    request.size = 0xFA000;
                    return -174;
                }
                let data = &gpu.2[..gpu.2.len().min(0xFA000)];
                // SAFETY: the caller's buffer has request.size (0xFA000) bytes; data is at most that long.
                unsafe { ptr::copy_nonoverlapping(data.as_ptr(), request.buffer.cast::<u8>(), data.len()) };
                request.size = data.len() as u32;
                0
            })
        })
    }

    /// The fake query; the casts through Proto<F> check each fake's signature against F's prototype.
    fn fake_query() -> Query<'static> {
        let table: [(u32, *const c_void); 7] = [
            (Initialize::ID, initialize as Proto<Initialize> as *const c_void),
            (Unload::ID, unload as Proto<Unload> as *const c_void),
            (
                GetErrorMessage::ID,
                error_message as Proto<GetErrorMessage> as *const c_void,
            ),
            (
                EnumPhysicalGpus::ID,
                enum_gpus as Proto<EnumPhysicalGpus> as *const c_void,
            ),
            (GetBusId::ID, bus_id as Proto<GetBusId> as *const c_void),
            (GetBusSlotId::ID, slot_id as Proto<GetBusSlotId> as *const c_void),
            (GetVbiosImage::ID, vbios_image as Proto<GetVbiosImage> as *const c_void),
        ];
        let query = move |id: u32| {
            let name = FUNCTIONS.iter().find(|&&(_, i)| i == id).map(|&(name, _)| name);
            match (name, table.iter().find(|&&(i, _)| i == id)) {
                (Some(name), Some(&(_, address))) if !with(|fake| fake.missing.contains(&name)) => address,
                _ => ptr::null(),
            }
        };
        // SAFETY: each id maps to a fake with that function's prototype (checked by the casts above), and the
        // fakes are plain functions, always callable.
        unsafe { Query::new(query) }
    }

    fn read(location: Option<&str>) -> Result<Vec<u8>> {
        read_vbios_image(location, fake_query())
    }

    fn error(location: Option<&str>) -> String {
        match read(location) {
            Ok(image) => panic!("read {} bytes instead of failing", image.len()),
            Err(error) => error.message().to_string(),
        }
    }

    #[test]
    fn structure_and_function_ids() {
        assert_eq!(
            size_of::<NV_GPU_VBIOS_IMAGE>(),
            if size_of::<*const c_void>() == 8 { 16 } else { 12 }
        );
        assert_eq!(
            (
                std::mem::offset_of!(NV_GPU_VBIOS_IMAGE, size),
                std::mem::offset_of!(NV_GPU_VBIOS_IMAGE, buffer)
            ),
            (4, 8)
        );
        assert_eq!(
            (IMAGE_SIZE, IMAGE_VERSION, MAX_PHYSICAL_GPUS),
            (1_024_000, 16 | 2 << 16, 64)
        );
        assert_eq!(
            FUNCTIONS,
            [
                ("Initialize", 0x0150E828),
                ("Unload", 0xD22BDD7E),
                ("GetErrorMessage", 0x6C2D048C),
                ("EnumPhysicalGPUs", 0xE5AC921F),
                ("GPU_GetBusId", 0x1BE0B8E5),
                ("GPU_GetBusSlotId", 0x2A0A350F),
                ("GPU_GetVbiosImage", 0xFC13EE11),
            ]
        );
    }

    #[test]
    fn the_gpu_is_found_by_bus_and_slot() {
        install(&[(1, 0, b"first"), (2, 0, b"second"), (0x41, 3, b"third")]);
        assert_eq!(read(Some("02:00.0")).unwrap(), b"second");
        assert_eq!(read(Some("41:03.0")).unwrap(), b"third");
        let calls = calls();
        assert_eq!((calls[0], calls[calls.len() - 1]), ("Initialize", "Unload"));
        with(|fake| fake.handle_status.insert(("GPU_GetBusId", 0x1000), -101)); // a GPU that cannot answer is skipped
        assert_eq!(read(Some("02:00.0")).unwrap(), b"second");
        install(&[(1, 0, b"slot 0"), (1, 1, b"slot 1")]); // same bus: the slot decides
        assert_eq!(read(Some("01:01.0")).unwrap(), b"slot 1");
        assert_eq!(read(Some("01:00.0")).unwrap(), b"slot 0");
        with(|fake| fake.handle_status.insert(("GPU_GetBusSlotId", 0x1000), -101));
        assert_eq!(
            error(Some("01:00.0")),
            "NvAPI does not list exactly one GPU at PCI 01:00 (it lists: 01:01)"
        );
    }

    #[test]
    fn a_long_image_is_cut_at_the_buffer_size() {
        let long: Vec<u8> = (0..5000).flat_map(|_| 0..=255u8).collect();
        install(&[(1, 0, &long)]);
        let data = read(Some("01:00.0")).unwrap();
        assert_eq!(data.len(), IMAGE_SIZE);
        assert_eq!(data, long[..IMAGE_SIZE]);
    }

    #[test]
    fn errors_name_the_cause_and_unload() {
        let gpus: &[(u32, u32, &[u8])] = &[(1, 0, b"first"), (2, 0, b"second")];
        let no_image = "this NVIDIA driver has no NvAPI_GPU_GetVbiosImage; reading the running VBIOS needs a driver \
                        that has it, for example 617.42";
        let pci = Some("01:00.0");
        type Case<'a> = (&'a [&'static str], &'a [(&'static str, i32)], Option<&'a str>, &'a str);
        let cases: &[Case] = &[
            (&["GPU_GetVbiosImage"], &[], pci, no_image),
            (&[], &[("GPU_GetVbiosImage", -3)], pci, no_image),
            (
                &[],
                &[("GPU_GetVbiosImage", -174)],
                pci,
                "NvAPI_GPU_GetVbiosImage failed: NVAPI_INSUFFICIENT_BUFFER (-174)",
            ),
            (
                &[],
                &[("GPU_GetVbiosImage", -104)],
                pci,
                "NvAPI_GPU_GetVbiosImage failed: status -104",
            ),
            (
                &["GetErrorMessage"],
                &[("GPU_GetVbiosImage", -174)],
                pci,
                "NvAPI_GPU_GetVbiosImage failed: status -174",
            ),
            (
                &[],
                &[("EnumPhysicalGPUs", -6)],
                pci,
                "NvAPI_EnumPhysicalGPUs failed: NVAPI_NVIDIA_DEVICE_NOT_FOUND (-6)",
            ),
            (
                &[],
                &[],
                Some("03:00.0"),
                "NvAPI does not list exactly one GPU at PCI 03:00 (it lists: 01:00, 02:00)",
            ),
            (
                &[],
                &[],
                None,
                "the PCI location of this GPU is unknown, so it cannot be found through NvAPI",
            ),
        ];
        for &(missing, status, location, message) in cases {
            install(gpus);
            with(|fake| {
                fake.missing = missing.to_vec();
                fake.status = status.iter().copied().collect();
            });
            assert_eq!(error(location), message);
            assert_eq!(calls().last(), Some(&"Unload"), "{message}");
        }
        install(gpus);
        with(|fake| fake.status.insert("Initialize", -1));
        assert_eq!(error(pci), "NvAPI_Initialize failed: status -1");
        assert_eq!(calls(), ["Initialize"]); // nothing to unload
    }

    #[test]
    fn error_texts_are_ascii_with_replacement_and_at_most_64_bytes() {
        let cases = [
            (-200, "caf\u{FFFD}\u{FFFD} (-200)".to_string()),
            (-201, format!("{} (-201)", "A".repeat(64))),
        ];
        for (status, text) in cases {
            install(&[(1, 0, b"first")]);
            with(|fake| fake.status.insert("GPU_GetVbiosImage", status));
            assert_eq!(
                error(Some("01:00.0")),
                format!("NvAPI_GPU_GetVbiosImage failed: {text}")
            );
        }
    }

    #[test]
    fn an_empty_image_is_an_invalid_size_and_an_empty_list_lists_none() {
        install(&[(1, 0, b"")]);
        assert_eq!(
            error(Some("01:00.0")),
            "NvAPI_GPU_GetVbiosImage returned an invalid size (0 bytes)"
        );
        assert_eq!(calls().last(), Some(&"Unload"));
        install(&[]);
        with(|fake| fake.missing = vec!["GPU_GetBusId"]); // never asked for: there is no GPU
        assert_eq!(
            error(Some("01:00.0")),
            "NvAPI does not list exactly one GPU at PCI 01:00 (it lists: none)"
        );
        install(&[(1, 0, b"first")]);
        with(|fake| fake.missing = vec!["GPU_GetBusSlotId"]);
        assert!(error(Some("01:00.0")).starts_with("this NVIDIA driver has no NvAPI_GPU_GetBusSlotId; "));
        assert_eq!(calls().last(), Some(&"Unload"));
    }

    #[test]
    fn a_driver_without_unload_still_reads() {
        install(&[(1, 0, b"first")]);
        with(|fake| fake.missing = vec!["Unload"]);
        assert_eq!(read(Some("01:00.0")).unwrap(), b"first");
        assert_eq!(calls(), ["Initialize", "EnumPhysicalGPUs", "GPU_GetVbiosImage"]);
    }

    #[test]
    fn locations_are_sliced_and_parsed_as_python_does() {
        let location = "01:00.0";
        assert_eq!(
            (
                char_slice(location, 0, 2),
                char_slice(location, 3, 5),
                char_slice(location, 0, 5)
            ),
            ("01", "00", "01:00")
        );
        assert_eq!(
            (char_slice("é1:x", 0, 2), char_slice("1", 3, 5), char_slice("", 0, 2)),
            ("é1", "", "")
        );
        let cases = [
            ("01", Some(1)),
            ("ff", Some(255)),
            (" 1", Some(1)),
            ("1 ", Some(1)),
            ("\u{85}1", Some(1)),
            ("+a", Some(10)),
            ("-1", Some(-1)),
            ("0x", None),
            ("1_", None),
            ("_1", None),
            ("0x_1", Some(1)),
            ("0x__1", None),
            ("1_0", Some(16)),
            ("1__0", None),
            (":0", None),
            ("", None),
            ("  ", None),
            ("0b1", Some(177)),
            ("\x1c1", None),
            ("- 1", None),
            ("+-1", None),
            ("\u{661}\u{660}", Some(16)),
            ("\u{ff10}f", Some(15)),
        ];
        for (text, value) in cases {
            assert_eq!(int16(text).ok(), value, "{text:?}");
        }
        assert_eq!(
            int16("\x1c1").unwrap_err().message(),
            "invalid literal for int() with base 16: '\\x1c1'"
        );
        install(&[(1, 0, b"first")]);
        assert_eq!(error(Some(":0:00.0")), "invalid literal for int() with base 16: ':0'");
        assert_eq!(calls(), ["Initialize", "Unload"]);
    }

    #[test]
    fn nvapi64_dll_is_loaded_from_system32_only() {
        assert_eq!(dll_path("D:\\Win\\System32"), "D:\\Win\\System32\\nvapi64.dll");
        assert_eq!(dll_path("C:\\WINDOWS\\system32"), "C:\\WINDOWS\\system32\\nvapi64.dll");
        assert_eq!(dll_path("C:\\"), "C:\\nvapi64.dll");
        assert_eq!(
            load_error("function 'nvapi_QueryInterface' not found").message(),
            "cannot load nvapi64.dll (function 'nvapi_QueryInterface' not found); is the NVIDIA driver installed?"
        );
    }

    #[test]
    fn windows_errors_read_as_python_shows_them() {
        use oserror::*;
        let denied = Some("Access is denied.\r\n");
        assert_eq!(format_error(denied), "Access is denied.");
        assert_eq!(format_error(None), "<no description>");
        assert_eq!(win_error(5, denied), "[WinError 5] Access is denied.");
        assert_eq!(win_error(0xE000_020B, None), "[WinError -536870389] <no description>");
        assert_eq!(
            errno_error(
                2,
                "reading vbios",
                Some("The system cannot find the file specified.\r\n")
            ),
            "[Errno 2] reading vbios failed: The system cannot find the file specified."
        );
        assert_eq!(
            windows_err(193, Some("%1 is not a valid Win32 application.\r\n")),
            "[WinError 193] %1 is not a valid Win32 application"
        );
        assert_eq!(windows_err(0xE000_0100, None), "Windows Error 0xe0000100");
        assert_eq!(
            module_not_found("C:\\WINDOWS\\system32\\nvapi64.dll"),
            "Could not find module 'C:\\WINDOWS\\system32\\nvapi64.dll' (or one of its dependencies). Try using the \
             full path with constructor syntax."
        );
    }
}
