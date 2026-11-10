//! The device interface that `operations` works on (devices.py's "platform object").
//!
//! `win32::Win32Platform` implements it with SetupAPI, the registry and NvAPI; the tests use fakes. Nothing in
//! this module needs Windows.

use crate::devices::{decode, encode, Adapter, Decoded};
use crate::error::Result;

/// One adapter's driver key (HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-...}\NNNN), open for reading
/// or for reading and writing. Dropping it closes the key.
pub trait RegistryKey {
    /// (type, exact bytes) of a value, or None when it does not exist.
    fn get_raw(&self, name: &str) -> Result<Option<(u32, Vec<u8>)>>;

    /// Write exactly these bytes with this type; nothing is added or converted (no terminating NUL is added to
    /// a REG_SZ whose data does not end with one).
    fn set_raw(&mut self, name: &str, kind: u32, raw: &[u8]) -> Result<()>;

    /// Delete a value; Ok(false) when it did not exist.
    fn delete(&mut self, name: &str) -> Result<bool>;

    /// Flush the key to disk (RegFlushKey).
    fn flush(&mut self) -> Result<()>;

    /// The value as `devices::decode` shows it, or None when it does not exist.
    fn get(&self, name: &str) -> Result<Option<Decoded>> {
        Ok(self.get_raw(name)?.map(|(kind, raw)| decode(kind, &raw)))
    }

    /// Write a decoded value with `devices::encode` (a DWORD as 4 bytes, a REG_SZ text NUL-terminated).
    fn set(&mut self, name: &str, value: &Decoded) -> Result<()> {
        let (kind, raw) = encode(value);
        self.set_raw(name, kind, &raw)
    }
}

/// Present NVIDIA display adapters, their driver keys, administrator rights and the running VBIOS.
///
/// `Send + Sync` because the window runs Load, Remove override and Verify in a worker thread.
pub trait Platform: Send + Sync {
    /// Present NVIDIA display adapters in a stable order (location, then instance id), indexed from 0.
    fn adapters(&self) -> Result<Vec<Adapter>>;

    /// The adapter's driver key; `write` asks for KEY_SET_VALUE too. Fails when the device is gone.
    fn open_key<'a>(&'a self, adapter: &Adapter, write: bool) -> Result<Box<dyn RegistryKey + 'a>>;

    /// True when the process runs with administrator rights (elevated).
    fn is_admin(&self) -> bool;

    /// The VBIOS image the driver is running on this adapter (NvAPI_GPU_GetVbiosImage, see `nvapi`).
    fn vbios_image(&self, adapter: &Adapter) -> Result<Vec<u8>>;
}
