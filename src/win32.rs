//! Windows implementation of `platform::Platform` (win32.py): SetupAPI, the registry and NvAPI; also the two
//! console calls of the command line (its terminal width and Ctrl+C).
//!
//! Errors read as Python showed them: a registry call that fails gives "[Errno <status>] <what> failed: <text>"
//! (win32.RegistryKey._check), a SetupAPI call "[WinError <code>] <text>" (ctypes.WinError), the text being
//! FormatMessageW's in the user's language.

use std::ptr;

use windows_sys::core::{BOOL, GUID};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
    SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey, DICS_FLAG_GLOBAL, DIGCF_PRESENT, DIREG_DRV,
    GUID_DEVCLASS_DISPLAY, HDEVINFO, SPDRP_ADDRESS, SPDRP_BUSNUMBER, SPDRP_DEVICEDESC, SPDRP_DRIVER,
    SPDRP_LOCATION_INFORMATION, SP_DEVINFO_DATA,
};
use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{
    GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleCtrlHandler, CONSOLE_SCREEN_BUFFER_INFO, CTRL_BREAK_EVENT,
    CTRL_C_EVENT, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegFlushKey, RegQueryValueExW, RegSetValueExW, HKEY, KEY_READ, KEY_SET_VALUE,
};
use windows_sys::Win32::UI::Shell::IsUserAnAdmin;

use crate::devices::{parse_instance_id, parse_location, pci_location, Adapter, Decoded};
use crate::error::{Error, Result};
use crate::nvapi::{self, oserror};
use crate::platform::{Platform, RegistryKey};

const NVIDIA: u16 = 0x10DE;
/// MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT): the user's language, as ctypes and Python ask FormatMessageW.
const LANG_USER_DEFAULT: u32 = 0x0400;
const INVALID_HDEVINFO: HDEVINFO = -1;
/// Characters of the instance id buffer (win32.py).
const INSTANCE_ID_CHARS: usize = 512;
/// Characters of the buffer for a text property (win32.py).
const PROPERTY_CHARS: usize = 1024;

// SetupAPI checks cbSize: 32 bytes on 64-bit Windows, 28 (packed) on 32-bit.
const _: () = assert!(size_of::<SP_DEVINFO_DATA>() == if cfg!(target_pointer_width = "64") { 32 } else { 28 });

/// The text FormatMessageW has for a Windows error code (in the user's language, line end included), or None when
/// it has none.
pub(crate) fn system_message(code: u32) -> Option<String> {
    let mut buffer: *mut u16 = ptr::null_mut();
    let flags = FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS;
    // SAFETY: with FORMAT_MESSAGE_ALLOCATE_BUFFER, lpBuffer is the address of a PWSTR that receives a LocalAlloc
    // buffer; no source, and no arguments are read (FORMAT_MESSAGE_IGNORE_INSERTS).
    let length = unsafe {
        FormatMessageW(
            flags,
            ptr::null(),
            code,
            LANG_USER_DEFAULT,
            (&raw mut buffer).cast(),
            0,
            ptr::null(),
        )
    };
    if buffer.is_null() {
        return None;
    }
    let text = (length > 0).then(|| {
        // SAFETY: FormatMessageW wrote `length` characters at `buffer`.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(buffer, length as usize) })
    });
    // SAFETY: FormatMessageW allocated the buffer with LocalAlloc; it is not used after this.
    unsafe { LocalFree(buffer.cast()) };
    text
}

/// GetLastError() of the Windows call that just failed.
fn last_error() -> u32 {
    // SAFETY: no arguments; called right after the call that failed, before any other Windows call.
    unsafe { GetLastError() }
}

/// ctypes.WinError(code).
fn win_error(code: u32) -> Error {
    Error::new(oserror::win_error(code, system_message(code).as_deref()))
}

/// A NUL-terminated UTF-16 copy of a str, for the W functions.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The text in a UTF-16 buffer up to its first NUL (all of it without one), as ctypes' `.value` reads it.
fn wide_text(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// An open registry key, such as an adapter's driver key (win32.RegistryKey); closed on drop.
pub struct Win32Key {
    handle: HKEY,
}

impl Win32Key {
    /// Nothing when status is ERROR_SUCCESS, else "[Errno <status>] <what> failed: <text>".
    fn check(status: u32, what: &str) -> Result<()> {
        if status == ERROR_SUCCESS {
            return Ok(());
        }
        Err(Error::new(oserror::errno_error(
            status,
            what,
            system_message(status).as_deref(),
        )))
    }
}

impl RegistryKey for Win32Key {
    fn get_raw(&self, name: &str) -> Result<Option<(u32, Vec<u8>)>> {
        let name_w = wide(name);
        let (mut kind, mut size) = (0u32, 0u32);
        loop {
            let mut buffer = vec![0u8; size.max(1) as usize];
            size = buffer.len() as u32;
            // SAFETY: an open key, a NUL-terminated name, a buffer of `size` bytes and valid out pointers.
            let status = unsafe {
                RegQueryValueExW(
                    self.handle,
                    name_w.as_ptr(),
                    ptr::null(),
                    &mut kind,
                    buffer.as_mut_ptr(),
                    &mut size,
                )
            };
            match status {
                ERROR_FILE_NOT_FOUND => return Ok(None),
                // The buffer is too small (or the value grew since the last call): size is the size needed now.
                // It always grows, so this cannot loop forever.
                ERROR_MORE_DATA => size = size.max((buffer.len() as u32).saturating_mul(2)),
                _ => {
                    Self::check(status, &format!("reading {name}"))?;
                    buffer.truncate(size as usize);
                    return Ok(Some((kind, buffer)));
                }
            }
        }
    }

    fn set_raw(&mut self, name: &str, kind: u32, raw: &[u8]) -> Result<()> {
        let what = format!("writing {name}");
        let Ok(size) = u32::try_from(raw.len()) else {
            return Err(Error::new(format!("{what} failed: the value is too large")));
        };
        // For REG_SZ, REG_EXPAND_SZ and REG_MULTI_SZ data whose last character is not NUL, RegSetValueExW stores
        // one more character when the two bytes after the counted data are zero. Two non-zero bytes there keep
        // the stored data exactly raw.len() bytes. The NUL after them is the one a Python bytes object has.
        let mut data = Vec::with_capacity(raw.len() + 3);
        data.extend_from_slice(raw);
        data.extend_from_slice(&[0xFF, 0xFF, 0]);
        let name_w = wide(name);
        // SAFETY: an open key, a NUL-terminated name, and `size` bytes of data followed by 3 more readable bytes.
        let status = unsafe { RegSetValueExW(self.handle, name_w.as_ptr(), 0, kind, data.as_ptr(), size) };
        Self::check(status, &what)
    }

    fn delete(&mut self, name: &str) -> Result<bool> {
        let name_w = wide(name);
        // SAFETY: an open key and a NUL-terminated name.
        let status = unsafe { RegDeleteValueW(self.handle, name_w.as_ptr()) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        Self::check(status, &format!("deleting {name}"))?;
        Ok(true)
    }

    fn flush(&mut self) -> Result<()> {
        // SAFETY: an open key.
        Self::check(unsafe { RegFlushKey(self.handle) }, "flushing the driver key")
    }
}

impl Drop for Win32Key {
    fn drop(&mut self) {
        // SAFETY: the key is open and is closed only here.
        unsafe { RegCloseKey(self.handle) };
    }
}

/// A present NVIDIA display device of a device information set.
struct Device {
    data: SP_DEVINFO_DATA,
    instance_id: String,
    /// (vendor, device, subsystem) of the instance id.
    ids: (u16, u16, Option<u32>),
}

/// The present display devices (a SetupAPI device information set); destroyed on drop.
struct DeviceInfoList(HDEVINFO);

impl DeviceInfoList {
    fn present_displays() -> Result<DeviceInfoList> {
        // SAFETY: a class GUID, no enumerator and no window.
        let info = unsafe { SetupDiGetClassDevsW(&GUID_DEVCLASS_DISPLAY, ptr::null(), ptr::null_mut(), DIGCF_PRESENT) };
        if info == INVALID_HDEVINFO || info == 0 {
            return Err(win_error(last_error()));
        }
        Ok(DeviceInfoList(info))
    }

    /// The NVIDIA devices of the set, enumerated lazily as win32.py's generator does (an enumeration error ends
    /// it); devices whose instance id cannot be read are skipped.
    fn nvidia(&self) -> impl Iterator<Item = Result<Device>> + '_ {
        let mut index = 0u32;
        let mut done = false;
        std::iter::from_fn(move || {
            while !done {
                let mut data = SP_DEVINFO_DATA {
                    cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                    ClassGuid: GUID::from_u128(0),
                    DevInst: 0,
                    Reserved: 0,
                };
                // SAFETY: an open set and an SP_DEVINFO_DATA with its cbSize set, as SetupAPI requires.
                if unsafe { SetupDiEnumDeviceInfo(self.0, index, &mut data) } == 0 {
                    done = true;
                    let code = last_error();
                    return (code != ERROR_NO_MORE_ITEMS).then(|| Err(win_error(code)));
                }
                index += 1;
                let mut buffer = [0u16; INSTANCE_ID_CHARS];
                // SAFETY: a device of this set and a buffer of INSTANCE_ID_CHARS characters, the size passed (in
                // characters); the required size is not wanted (null).
                let ok = unsafe {
                    SetupDiGetDeviceInstanceIdW(
                        self.0,
                        &data,
                        buffer.as_mut_ptr(),
                        INSTANCE_ID_CHARS as u32,
                        ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    continue;
                }
                let instance_id = wide_text(&buffer);
                if let Some(ids @ (NVIDIA, _, _)) = parse_instance_id(&instance_id) {
                    return Some(Ok(Device { data, instance_id, ids }));
                }
            }
            None
        })
    }

    /// A text property (SPDRP_*) up to its first NUL; None when it cannot be read or is empty.
    fn text_property(&self, device: &Device, property: u32) -> Option<String> {
        let mut buffer = [0u16; PROPERTY_CHARS];
        // SAFETY: a device of this set and a buffer of size_of_val(&buffer) bytes, the size passed (in bytes); the
        // type and the required size are not wanted (null).
        let ok = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                self.0,
                &device.data,
                property,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return None;
        }
        Some(wide_text(&buffer)).filter(|text| !text.is_empty())
    }

    /// A DWORD property (SPDRP_*); None when it cannot be read (or is longer than 4 bytes).
    fn dword_property(&self, device: &Device, property: u32) -> Option<u32> {
        let mut value = 0u32;
        // SAFETY: a device of this set and a 4-byte buffer, the size passed; type and required size are null.
        let ok = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                self.0,
                &device.data,
                property,
                ptr::null_mut(),
                (&raw mut value).cast(),
                size_of::<u32>() as u32,
                ptr::null_mut(),
            )
        };
        (ok != 0).then_some(value)
    }

    /// PCI bus:device.function from the bus number and address (which do not depend on the Windows display
    /// language), else from the English location text.
    fn location(&self, device: &Device) -> Option<String> {
        pci_location(
            self.dword_property(device, SPDRP_BUSNUMBER),
            self.dword_property(device, SPDRP_ADDRESS),
        )
        .or_else(|| parse_location(self.text_property(device, SPDRP_LOCATION_INFORMATION).as_deref()))
    }

    /// The device's driver key, for reading, or for reading and writing.
    fn open(&self, device: &Device, write: bool) -> Result<Win32Key> {
        let access = KEY_READ | if write { KEY_SET_VALUE } else { 0 };
        // SAFETY: a device of this set; the key returned is ours to close (Win32Key).
        let handle = unsafe { SetupDiOpenDevRegKey(self.0, &device.data, DICS_FLAG_GLOBAL, 0, DIREG_DRV, access) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(win_error(last_error()));
        }
        Ok(Win32Key { handle })
    }
}

impl Drop for DeviceInfoList {
    fn drop(&mut self) {
        // SAFETY: the set is open and is destroyed only here; keys opened from it stay valid.
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// The present NVIDIA display adapters of this PC and their driver keys.
pub struct Win32Platform {
    _private: (),
}

impl Win32Platform {
    pub fn new() -> Self {
        Win32Platform { _private: () }
    }
}

impl Default for Win32Platform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for Win32Platform {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        let list = DeviceInfoList::present_displays()?;
        let mut found = Vec::new();
        for device in list.nvidia() {
            let device = device?;
            let (vendor, device_id, subsystem) = device.ids;
            let (mut name, mut version) = (list.text_property(&device, SPDRP_DEVICEDESC), None);
            // A key that cannot be opened leaves the SetupAPI name and no version; one that cannot be read fails.
            if let Ok(key) = list.open(&device, false) {
                if let Some(Decoded::Sz(text)) = key.get("DriverDesc")? {
                    name = Some(text);
                }
                if let Some(Decoded::Sz(text)) = key.get("DriverVersion")? {
                    version = Some(text);
                }
            }
            found.push(Adapter {
                index: 0,
                name: name
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "NVIDIA display adapter".to_string()),
                vendor,
                device: device_id,
                subsystem,
                location: list.location(&device),
                driver_key: list.text_property(&device, SPDRP_DRIVER),
                instance_id: device.instance_id,
                driver_version: version,
            });
        }
        // Location missing last, then the location text, then the instance id.
        found.sort_by(|a, b| {
            (a.location.is_none(), &a.location, &a.instance_id).cmp(&(
                b.location.is_none(),
                &b.location,
                &b.instance_id,
            ))
        });
        for (index, adapter) in found.iter_mut().enumerate() {
            adapter.index = index;
        }
        Ok(found)
    }

    fn open_key<'a>(&'a self, adapter: &Adapter, write: bool) -> Result<Box<dyn RegistryKey + 'a>> {
        let list = DeviceInfoList::present_displays()?;
        for device in list.nvidia() {
            let device = device?;
            if device.instance_id == adapter.instance_id {
                return Ok(Box::new(list.open(&device, write)?));
            }
        }
        Err(Error::new(format!(
            "device {} is no longer present",
            adapter.instance_id
        )))
    }

    fn is_admin(&self) -> bool {
        // SAFETY: no arguments.
        unsafe { IsUserAnAdmin() != 0 }
    }

    fn vbios_image(&self, adapter: &Adapter) -> Result<Vec<u8>> {
        nvapi::read_vbios_image(adapter.location.as_deref(), nvapi::Query::system32()?)
    }
}

/// The width of the console window that stdout writes to (os.get_terminal_size), or None when stdout is not a
/// console. The command line wraps its help to it, as argparse does.
pub(crate) fn console_columns() -> Option<i128> {
    // SAFETY: GetStdHandle has no preconditions. GetConsoleScreenBufferInfo only writes the local structure (plain
    // integers, valid when zeroed) and fails without side effects when the handle is not a console.
    let info = unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(handle, &mut info) == 0 {
            return None;
        }
        info
    };
    Some(i128::from(info.srWindow.Right) - i128::from(info.srWindow.Left) + 1)
}

/// Ctrl+C or Ctrl+Break in the console: the command line exits with 130 at once.
pub(crate) fn exit_on_interrupt() {
    unsafe extern "system" fn handler(kind: u32) -> BOOL {
        if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
            // process::exit does not unwind, so no panic crosses this extern boundary.
            std::process::exit(130);
        }
        0
    }
    // SAFETY: the handler is a plain function that lives as long as the process and only exits it.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(test)]
mod tests {
    //! Windows only (CI): the registry key on a volatile scratch key under HKEY_CURRENT_USER\Software, deleted
    //! afterwards, and SetupAPI on whatever display adapters the machine has. No GPU or driver key is touched.

    use super::*;
    use crate::devices::{decode, REG_BINARY, REG_DWORD, REG_SZ};
    use crate::testing::every_type;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::System::Registry::{
        RegCreateKeyExW, RegDeleteKeyW, RegOpenKeyExW, HKEY_CURRENT_USER, REG_OPTION_VOLATILE,
    };

    /// A volatile key HKCU\Software\VixLoaderTest-<test>-<pid>-<time>, open for reading and writing; deleted on
    /// drop.
    struct Scratch {
        subkey: Vec<u16>,
        key: Win32Key,
    }

    impl Scratch {
        fn new(test: &str) -> Scratch {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos());
            let subkey = wide(&format!(
                "Software\\VixLoaderTest-{test}-{}-{nanos}",
                std::process::id()
            ));
            let mut handle: HKEY = ptr::null_mut();
            // SAFETY: a NUL-terminated subkey of HKCU, no class or security attributes, and a valid out pointer.
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    0,
                    ptr::null(),
                    REG_OPTION_VOLATILE,
                    KEY_READ | KEY_SET_VALUE,
                    ptr::null(),
                    &mut handle,
                    ptr::null_mut(),
                )
            };
            assert_eq!(status, ERROR_SUCCESS, "creating the scratch key");
            Scratch {
                subkey,
                key: Win32Key { handle },
            }
        }

        /// The same key, opened again for reading only.
        fn read_only(&self) -> Win32Key {
            let mut handle: HKEY = ptr::null_mut();
            // SAFETY: a NUL-terminated subkey of HKCU and a valid out pointer.
            let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, self.subkey.as_ptr(), 0, KEY_READ, &mut handle) };
            assert_eq!(status, ERROR_SUCCESS, "opening the scratch key");
            Win32Key { handle }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // SAFETY: a NUL-terminated subkey of HKCU that this test created (it has no subkeys). The open handle
            // in self.key, closed right after this, does not prevent the deletion.
            unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, self.subkey.as_ptr()) };
        }
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn a_reg_sz_without_its_final_nul_reads_back_exactly() {
        let mut scratch = Scratch::new("sz");
        let raw = utf16("abc");
        scratch.key.set_raw("text", REG_SZ, &raw).unwrap();
        assert_eq!(scratch.key.get_raw("text").unwrap(), Some((REG_SZ, raw)));
        let terminated = utf16("abc\0");
        scratch.key.set_raw("text", REG_SZ, &terminated).unwrap();
        assert_eq!(scratch.key.get_raw("text").unwrap(), Some((REG_SZ, terminated)));
    }

    #[test]
    fn a_reg_binary_round_trip_and_a_large_value() {
        let mut scratch = Scratch::new("binary");
        scratch.key.set_raw("small", REG_BINARY, b"\x00\x01\xff").unwrap();
        assert_eq!(
            scratch.key.get_raw("small").unwrap(),
            Some((REG_BINARY, vec![0, 1, 0xFF]))
        );
        let large: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect(); // over 64 KiB, like a VBIOS
        scratch.key.set_raw("large", REG_BINARY, &large).unwrap();
        assert_eq!(scratch.key.get_raw("large").unwrap(), Some((REG_BINARY, large)));
        scratch.key.set_raw("empty", REG_BINARY, b"").unwrap();
        assert_eq!(scratch.key.get_raw("empty").unwrap(), Some((REG_BINARY, Vec::new())));
        scratch.key.flush().unwrap();
    }

    #[test]
    fn every_value_shape_is_written_and_read_back_exactly() {
        // The shapes load saves from EnableGpuFirmware and unload writes back with set_raw: texts without their
        // NUL or of odd size, DWORDs of another size, unknown types. The trick in set_raw keeps them exact.
        let mut scratch = Scratch::new("shapes");
        for (index, (kind, data, _)) in every_type().into_iter().enumerate() {
            let name = format!("value{index}");
            scratch.key.set_raw(&name, kind, &data).unwrap();
            assert_eq!(
                scratch.key.get_raw(&name).unwrap(),
                Some((kind, data.clone())),
                "type {kind:#x}"
            );
            assert_eq!(
                scratch.key.get(&name).unwrap(),
                Some(decode(kind, &data)),
                "type {kind:#x}"
            );
        }
        // set() writes a DWORD as 4 bytes and a text with its NUL (win32.RegistryKey.set).
        scratch.key.set("dword", &Decoded::Dword(0x11)).unwrap();
        assert_eq!(
            scratch.key.get_raw("dword").unwrap(),
            Some((REG_DWORD, vec![0x11, 0, 0, 0]))
        );
        scratch
            .key
            .set("text", &Decoded::Sz("hex(1):31,00".to_string()))
            .unwrap();
        assert_eq!(
            scratch.key.get_raw("text").unwrap(),
            Some((REG_SZ, utf16("hex(1):31,00\0")))
        );
    }

    #[test]
    fn missing_values_are_none_and_their_delete_is_false() {
        let mut scratch = Scratch::new("missing");
        assert_eq!(scratch.key.get_raw("absent").unwrap(), None);
        assert!(!scratch.key.delete("absent").unwrap());
        scratch.key.set_raw("present", REG_BINARY, b"x").unwrap();
        assert!(scratch.key.delete("present").unwrap());
        assert_eq!(scratch.key.get_raw("present").unwrap(), None);
    }

    #[test]
    fn registry_errors_read_as_python_shows_them() {
        let scratch = Scratch::new("errors");
        let mut key = scratch.read_only();
        let error = key.set_raw("value", REG_BINARY, b"x").unwrap_err().to_string();
        // ERROR_ACCESS_DENIED, in the language of the machine.
        assert!(error.starts_with("[Errno 5] writing value failed: "), "{error}");
        assert!(!error.ends_with(char::is_whitespace) && error.len() > "[Errno 5] writing value failed: ".len());
        let message = system_message(5).unwrap_or_default();
        assert!(!message.trim().is_empty());
        assert_eq!(win_error(5).to_string(), format!("[WinError 5] {}", message.trim()));
    }

    #[test]
    fn adapters_are_listed_without_nvidia_gpus_too() {
        let platform = Win32Platform::new();
        let adapters = platform.adapters().unwrap();
        for (index, adapter) in adapters.iter().enumerate() {
            assert_eq!((adapter.index, adapter.vendor), (index, NVIDIA));
        }
        let gone = Adapter {
            index: 0,
            name: "gone".to_string(),
            vendor: NVIDIA,
            device: 0,
            subsystem: None,
            location: None,
            instance_id: "PCI\\VEN_10DE&DEV_0000\\VIXLOADERTEST".to_string(),
            driver_version: None,
            driver_key: None,
        };
        match platform.open_key(&gone, false) {
            Ok(_) => panic!("opened the key of a device that does not exist"),
            Err(error) => assert_eq!(
                error.message(),
                "device PCI\\VEN_10DE&DEV_0000\\VIXLOADERTEST is no longer present"
            ),
        }
        let _ = platform.is_admin();
    }

    #[test]
    fn texts_of_utf16_buffers() {
        assert_eq!(wide("ab"), [0x61, 0x62, 0]);
        assert_eq!(wide_text(&[0x61, 0x62, 0, 0x63]), "ab");
        assert_eq!(wide_text(&[0x61, 0x62]), "ab");
        assert_eq!(wide_text(&[0xD800, 0x61, 0]), "\u{FFFD}a");
    }
}
