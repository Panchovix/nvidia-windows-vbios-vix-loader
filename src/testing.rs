//! In-memory fakes of the device interface for tests (the FakeKey and FakePlatform of the Python tests):
//! no Windows, GPU or registry needed. Built for this crate's tests and with the `testing` feature.
//!
//! Also the helpers of tests/test_windows_loader.py: the synthetic ROM images (`image`, `chain`, `rom`), the
//! test adapter and the one-GPU platform with unrelated values in its key.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::devices::{
    decode, encode, Adapter, Decoded, REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_NONE, REG_QWORD, REG_SZ,
};
use crate::error::{Error, Result};
use crate::platform::{Platform, RegistryKey};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A test that panicked while holding the lock must not hide the state from the next assertion.
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How FakeKey keeps a value written with these bytes (FakeKey.set_raw): decoded when encoding it again gives the
/// same bytes, else the type and the bytes as they are (a REG_SZ with data after its first NUL, for example).
pub fn stored(kind: u32, raw: &[u8]) -> Decoded {
    let value = decode(kind, raw);
    if encode(&value).1 == raw {
        value
    } else {
        Decoded::Raw(kind, raw.to_vec())
    }
}

/// What a FakeKey write hook does with one `set_raw` call.
pub enum OnWrite {
    /// Write as FakeKey does (the default).
    Store,
    /// Write these bytes with this type instead.
    Replace(u32, Vec<u8>),
    /// Nothing happens and nothing is logged: the write is lost (a subclass that does not call super()).
    Skip,
    /// Fail with this error; nothing is logged.
    Fail(Error),
}

/// What a FakeKey delete hook does with one `delete` call.
pub enum OnDelete {
    /// Delete as FakeKey does (the default).
    Normal,
    /// Return Ok(false) without deleting or logging anything.
    Keep,
    /// Fail with this error; nothing is logged.
    Fail(Error),
}

type SetHook = Arc<dyn Fn(&str, &Decoded) -> Option<Error> + Send + Sync>;
type SetRawHook = Arc<dyn Fn(&str, u32, &[u8]) -> OnWrite + Send + Sync>;
type DeleteHook = Arc<dyn Fn(&str) -> OnDelete + Send + Sync>;
type GetHook = Arc<dyn Fn(&str) -> Option<Error> + Send + Sync>;
type FlushHook = Arc<dyn Fn() -> Option<Error> + Send + Sync>;

#[derive(Default)]
struct KeyState {
    /// Name -> value in insertion order, like the Python dict.
    values: Vec<(String, Decoded)>,
    calls: Vec<(String, String)>,
    flushes: usize,
    closed: usize,
    drop_writes: bool,
    on_set: Option<SetHook>,
    on_set_raw: Option<SetRawHook>,
    on_delete: Option<DeleteHook>,
    on_get: Option<GetHook>,
    on_flush: Option<FlushHook>,
}

impl KeyState {
    fn insert(&mut self, name: &str, value: Decoded) {
        match self.values.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = value,
            None => self.values.push((name.to_string(), value)),
        }
    }

    fn remove(&mut self, name: &str) -> Option<Decoded> {
        let position = self.values.iter().position(|(n, _)| n == name)?;
        Some(self.values.remove(position).1)
    }
}

/// A driver key in memory (FakeKey of the Python tests). Clones share the same key, so a test keeps one and
/// inspects it after the operations wrote through the platform. The values hold what `get` returns, or the type
/// and bytes for data that `get` cannot show exactly, as the registry stores it.
///
/// `set_raw` logs ("set", name) and, unless `drop_writes`, stores the value; `delete` logs ("delete", name) and
/// removes it (also with `drop_writes`, as in Python); `flush` counts. The keys that `FakePlatform::open_key`
/// hands out count their drops in `closed`. Hooks stand in for the Python subclasses of the tests (FailingKey,
/// KeepsFirmware, DropsBackup, KeepsRecords, AlteringKey).
#[derive(Clone, Default)]
pub struct FakeKey {
    state: Arc<Mutex<KeyState>>,
}

impl FakeKey {
    /// A key holding these values (later duplicates replace earlier ones, as in a dict).
    pub fn new<N: AsRef<str>>(values: impl IntoIterator<Item = (N, Decoded)>) -> FakeKey {
        let key = FakeKey::default();
        for (name, value) in values {
            key.insert(name.as_ref(), value);
        }
        key
    }

    /// `key.values[name] = value`.
    pub fn insert(&self, name: &str, value: Decoded) {
        lock(&self.state).insert(name, value);
    }

    /// `key.values = {...}`: these values replace all others (the log and counters stay).
    pub fn set_values<N: AsRef<str>>(&self, values: impl IntoIterator<Item = (N, Decoded)>) {
        let mut state = lock(&self.state);
        state.values.clear();
        for (name, value) in values {
            state.insert(name.as_ref(), value);
        }
    }

    /// `del key.values[name]`; the value, or None when there was none.
    pub fn remove(&self, name: &str) -> Option<Decoded> {
        lock(&self.state).remove(name)
    }

    /// All values in insertion order.
    pub fn values(&self) -> Vec<(String, Decoded)> {
        lock(&self.state).values.clone()
    }

    /// `key.values.get(name)`.
    pub fn value(&self, name: &str) -> Option<Decoded> {
        lock(&self.state)
            .values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }

    /// The exact (type, bytes) of a value, without the hooks.
    pub fn raw(&self, name: &str) -> Option<(u32, Vec<u8>)> {
        self.value(name).map(|value| encode(&value))
    }

    /// The writes in order: ("set" or "delete", name).
    pub fn calls(&self) -> Vec<(String, String)> {
        lock(&self.state).calls.clone()
    }

    pub fn flushes(&self) -> usize {
        lock(&self.state).flushes
    }

    /// How many keys opened through `FakePlatform::open_key` were dropped (closed).
    pub fn closed(&self) -> usize {
        lock(&self.state).closed
    }

    pub fn drop_writes(&self) -> bool {
        lock(&self.state).drop_writes
    }

    /// Writes are logged but not stored (deletes still happen, as in the Python fake).
    pub fn set_drop_writes(&self, drop: bool) {
        lock(&self.state).drop_writes = drop;
    }

    /// Called by `set` before it encodes the value: Some(error) fails the call (FailingKey of the tests).
    pub fn on_set(&self, hook: impl Fn(&str, &Decoded) -> Option<Error> + Send + Sync + 'static) {
        lock(&self.state).on_set = Some(Arc::new(hook));
    }

    /// Decides what each `set_raw` call does (DropsBackup, AlteringKey of the tests).
    pub fn on_set_raw(&self, hook: impl Fn(&str, u32, &[u8]) -> OnWrite + Send + Sync + 'static) {
        lock(&self.state).on_set_raw = Some(Arc::new(hook));
    }

    /// Decides what each `delete` call does (KeepsFirmware, KeepsRecords of the tests).
    pub fn on_delete(&self, hook: impl Fn(&str) -> OnDelete + Send + Sync + 'static) {
        lock(&self.state).on_delete = Some(Arc::new(hook));
    }

    /// Called by every read (`get_raw`, and so `get`): Some(error) fails it.
    pub fn on_get(&self, hook: impl Fn(&str) -> Option<Error> + Send + Sync + 'static) {
        lock(&self.state).on_get = Some(Arc::new(hook));
    }

    /// Called by `flush`: Some(error) fails it (the flush is not counted then).
    pub fn on_flush(&self, hook: impl Fn() -> Option<Error> + Send + Sync + 'static) {
        lock(&self.state).on_flush = Some(Arc::new(hook));
    }
}

impl RegistryKey for FakeKey {
    fn get_raw(&self, name: &str) -> Result<Option<(u32, Vec<u8>)>> {
        let hook = lock(&self.state).on_get.clone();
        if let Some(error) = hook.and_then(|hook| hook(name)) {
            return Err(error);
        }
        Ok(self.raw(name))
    }

    fn set(&mut self, name: &str, value: &Decoded) -> Result<()> {
        let hook = lock(&self.state).on_set.clone();
        if let Some(error) = hook.and_then(|hook| hook(name, value)) {
            return Err(error);
        }
        let (kind, raw) = encode(value);
        self.set_raw(name, kind, &raw)
    }

    fn set_raw(&mut self, name: &str, kind: u32, raw: &[u8]) -> Result<()> {
        let hook = lock(&self.state).on_set_raw.clone();
        let (kind, raw) = match hook.map_or(OnWrite::Store, |hook| hook(name, kind, raw)) {
            OnWrite::Store => (kind, raw.to_vec()),
            OnWrite::Replace(kind, raw) => (kind, raw),
            OnWrite::Skip => return Ok(()),
            OnWrite::Fail(error) => return Err(error),
        };
        let mut state = lock(&self.state);
        state.calls.push(("set".to_string(), name.to_string()));
        if !state.drop_writes {
            state.insert(name, stored(kind, &raw));
        }
        Ok(())
    }

    fn delete(&mut self, name: &str) -> Result<bool> {
        let hook = lock(&self.state).on_delete.clone();
        match hook.map_or(OnDelete::Normal, |hook| hook(name)) {
            OnDelete::Normal => {}
            OnDelete::Keep => return Ok(false),
            OnDelete::Fail(error) => return Err(error),
        }
        let mut state = lock(&self.state);
        state.calls.push(("delete".to_string(), name.to_string()));
        Ok(state.remove(name).is_some())
    }

    fn flush(&mut self) -> Result<()> {
        let hook = lock(&self.state).on_flush.clone();
        if let Some(error) = hook.and_then(|hook| hook()) {
            return Err(error);
        }
        lock(&self.state).flushes += 1;
        Ok(())
    }
}

/// The key that `FakePlatform::open_key` hands out: the shared FakeKey, closed (counted) when dropped.
struct OpenKey {
    key: FakeKey,
}

impl RegistryKey for OpenKey {
    fn get_raw(&self, name: &str) -> Result<Option<(u32, Vec<u8>)>> {
        self.key.get_raw(name)
    }

    fn set(&mut self, name: &str, value: &Decoded) -> Result<()> {
        self.key.set(name, value)
    }

    fn set_raw(&mut self, name: &str, kind: u32, raw: &[u8]) -> Result<()> {
        self.key.set_raw(name, kind, raw)
    }

    fn delete(&mut self, name: &str) -> Result<bool> {
        self.key.delete(name)
    }

    fn flush(&mut self) -> Result<()> {
        self.key.flush()
    }
}

impl Drop for OpenKey {
    fn drop(&mut self) {
        lock(&self.key.state).closed += 1;
    }
}

/// Adapters, their keys by device instance id, administrator rights and the running VBIOS (FakePlatform of the
/// Python tests). `vbios_image` logs the instance id in `reads` and returns the next queued result, then `image`.
/// Every setting can change between operations, as the Python tests assign the attributes.
#[derive(Default)]
pub struct FakePlatform {
    adapters: Mutex<Vec<Adapter>>,
    adapters_error: Mutex<Option<Error>>,
    keys: Mutex<Vec<(String, FakeKey)>>,
    open_errors: Mutex<Vec<(String, Error)>>,
    open_panic: Mutex<Option<String>>,
    admin: Mutex<bool>,
    writable_opens: Mutex<usize>,
    image: Mutex<Vec<u8>>,
    queued_images: Mutex<VecDeque<Result<Vec<u8>>>>,
    reads: Mutex<Vec<String>>,
}

impl FakePlatform {
    /// `FakePlatform(adapters, keys, admin, image)`.
    pub fn new<N: AsRef<str>>(
        adapters: Vec<Adapter>,
        keys: impl IntoIterator<Item = (N, FakeKey)>,
        admin: bool,
        image: Vec<u8>,
    ) -> FakePlatform {
        let platform = FakePlatform {
            adapters: Mutex::new(adapters),
            ..FakePlatform::default()
        };
        for (instance_id, key) in keys {
            platform.set_key(instance_id.as_ref(), key);
        }
        platform.set_admin(admin);
        platform.set_image(image);
        platform
    }

    /// The key of this device instance id (shared, not a copy). Panics when there is none: a test error.
    pub fn key(&self, instance_id: &str) -> FakeKey {
        lock(&self.keys)
            .iter()
            .find(|(id, _)| id == instance_id)
            .map(|(_, key)| key.clone())
            .unwrap_or_else(|| panic!("FakePlatform has no key for {instance_id}"))
    }

    /// `fake.keys[instance_id] = key`.
    pub fn set_key(&self, instance_id: &str, key: FakeKey) {
        let mut keys = lock(&self.keys);
        match keys.iter_mut().find(|(id, _)| id == instance_id) {
            Some(entry) => entry.1 = key,
            None => keys.push((instance_id.to_string(), key)),
        }
    }

    pub fn set_adapters(&self, adapters: Vec<Adapter>) {
        *lock(&self.adapters) = adapters;
    }

    /// `adapters()` fails with this error (None: it works again).
    pub fn fail_adapters(&self, error: Option<Error>) {
        *lock(&self.adapters_error) = error;
    }

    /// `open_key` of this device instance id fails with this error (after counting a writable open).
    pub fn fail_open(&self, instance_id: &str, error: Error) {
        lock(&self.open_errors).push((instance_id.to_string(), error));
    }

    /// `open_key` panics with this message (None: it works again): an unexpected internal error, as the Python
    /// tests raise RuntimeError from a replaced open_key.
    pub fn panic_on_open(&self, message: Option<&str>) {
        *lock(&self.open_panic) = message.map(str::to_string);
    }

    pub fn admin(&self) -> bool {
        *lock(&self.admin)
    }

    pub fn set_admin(&self, admin: bool) {
        *lock(&self.admin) = admin;
    }

    /// How many times `open_key` was asked for a writable key.
    pub fn writable_opens(&self) -> usize {
        *lock(&self.writable_opens)
    }

    /// What `vbios_image` returns once the queue is empty.
    pub fn set_image(&self, image: Vec<u8>) {
        *lock(&self.image) = image;
    }

    /// Queue one result for the next `vbios_image` call (two different reads, a failing read).
    pub fn queue_image(&self, result: Result<Vec<u8>>) {
        lock(&self.queued_images).push_back(result);
    }

    /// The device instance ids `vbios_image` was called for, in order.
    pub fn reads(&self) -> Vec<String> {
        lock(&self.reads).clone()
    }
}

impl Platform for FakePlatform {
    fn adapters(&self) -> Result<Vec<Adapter>> {
        if let Some(error) = lock(&self.adapters_error).clone() {
            return Err(error);
        }
        Ok(lock(&self.adapters).clone())
    }

    fn open_key<'a>(&'a self, adapter: &Adapter, write: bool) -> Result<Box<dyn RegistryKey + 'a>> {
        if write {
            *lock(&self.writable_opens) += 1;
        }
        let panic_message = lock(&self.open_panic).clone();
        if let Some(message) = panic_message {
            panic!("{message}");
        }
        let failure = lock(&self.open_errors)
            .iter()
            .find(|(id, _)| *id == adapter.instance_id)
            .map(|(_, e)| e.clone());
        if let Some(error) = failure {
            return Err(error);
        }
        let key = lock(&self.keys)
            .iter()
            .find(|(id, _)| *id == adapter.instance_id)
            .map(|(_, key)| key.clone());
        match key {
            Some(key) => Ok(Box::new(OpenKey { key })),
            None => Err(Error::new(format!(
                "FakePlatform has no key for {}",
                adapter.instance_id
            ))),
        }
    }

    fn is_admin(&self) -> bool {
        self.admin()
    }

    fn vbios_image(&self, adapter: &Adapter) -> Result<Vec<u8>> {
        lock(&self.reads).push(adapter.instance_id.clone());
        match lock(&self.queued_images).pop_front() {
            Some(result) => result,
            None => Ok(lock(&self.image).clone()),
        }
    }
}

/// The test GPUs of tests/test_windows_loader.py.
pub const RTX_5070: u16 = 0x2C09;
pub const RTX_5070_TI: u16 = 0x2C05;
pub const RTX_4090: u16 = 0x2684;
pub const RTX_A2000: u16 = 0x2531;
pub const A100: u16 = 0x20B5;
pub const RTX_2060_SUPER: u16 = 0x1F06;
/// The flash prefix and trailing bytes `rom` puts around the chain.
pub const PREFIX: usize = 0x400;
pub const TRAILING: usize = 0x300;

/// The options of the Python test helper image().
#[derive(Clone, Copy, Debug)]
pub struct ImageOptions {
    /// 0 legacy x86 (55AA + PCIR, with a zero checksum), 3 EFI, 0xE0 NVIDIA data image ("VN" + NPDS).
    pub code_type: u8,
    /// The PCIR/NPDS length in 512-byte blocks.
    pub blocks: u16,
    /// The last-image flag (in the NPDE, or in the PCIR without one).
    pub last: bool,
    /// The NPDE length in blocks (None: `blocks`); the image is this long.
    pub npde_blocks: Option<u16>,
    /// Whether the image has an NPDE header.
    pub npde: bool,
    /// The PCIR last flag (None: `last` when there is no NPDE).
    pub pcir_last: Option<bool>,
}

impl Default for ImageOptions {
    fn default() -> Self {
        ImageOptions {
            code_type: 0,
            blocks: 2,
            last: false,
            npde_blocks: None,
            npde: true,
            pcir_last: None,
        }
    }
}

/// One ROM image: legacy/EFI (55AA + PCIR) or NVIDIA data (VN + NPDS), optionally with an NPDE header.
pub fn image(device: u16, options: ImageOptions) -> Vec<u8> {
    let ImageOptions {
        code_type,
        blocks,
        last,
        npde_blocks,
        npde,
        pcir_last,
    } = options;
    let npde_blocks = npde_blocks.filter(|&n| n != 0).unwrap_or(blocks);
    let mut data = vec![0xFFu8; usize::from(npde_blocks) * 512];
    data[0..2].copy_from_slice(if code_type != 0xE0 { b"\x55\xaa" } else { b"VN" });
    data[0x18..0x1A].copy_from_slice(&0x40u16.to_le_bytes());
    data[0x40..0x58].fill(0);
    data[0x40..0x44].copy_from_slice(if code_type != 0xE0 { b"PCIR" } else { b"NPDS" });
    data[0x44..0x48].copy_from_slice(&(0x10DE | u32::from(device) << 16).to_le_bytes());
    data[0x4A..0x4C].copy_from_slice(&0x18u16.to_le_bytes());
    data[0x50..0x52].copy_from_slice(&blocks.to_le_bytes());
    data[0x54] = code_type;
    data[0x55] = if pcir_last.unwrap_or(last && !npde) { 0x80 } else { 0 };
    if npde {
        data[0x60..0x64].copy_from_slice(b"NPDE");
        data[0x64..0x66].copy_from_slice(&0x101u16.to_le_bytes());
        data[0x66..0x68].copy_from_slice(&0x0Cu16.to_le_bytes());
        data[0x68..0x6A].copy_from_slice(&npde_blocks.to_le_bytes());
        data[0x6A] = if last { 0x80 } else { 0 };
        data[0x6B] = 0;
    }
    if code_type == 0 {
        let end = usize::from(blocks) * 512;
        data[end - 1] = 0;
        let sum = data[..end].iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        data[end - 1] = sum.wrapping_neg();
    }
    data
}

/// A legacy image (with these options), an EFI image and a last NVIDIA data image of `data_blocks` blocks.
pub fn chain(device: u16, data_blocks: u16, options: ImageOptions) -> Vec<u8> {
    let mut data = image(device, options);
    data.extend(image(
        device,
        ImageOptions {
            code_type: 3,
            blocks: 1,
            ..ImageOptions::default()
        },
    ));
    data.extend(image(
        device,
        ImageOptions {
            code_type: 0xE0,
            blocks: data_blocks,
            last: true,
            ..ImageOptions::default()
        },
    ));
    data
}

/// A flash-like file: prefix, the chain, then trailing bytes.
pub fn rom(device: u16, data_blocks: u16) -> Vec<u8> {
    let mut data = vec![0xFF; PREFIX];
    data.extend(chain(device, data_blocks, ImageOptions::default()));
    data.extend(vec![0xFF; TRAILING]);
    data
}

/// The test adapter: "GPU {index}", 10de:{device}, subsystem 205e1043.
pub fn adapter(index: usize, device: u16, location: Option<&str>, version: Option<&str>) -> Adapter {
    Adapter {
        index,
        name: format!("GPU {index}"),
        vendor: 0x10DE,
        device,
        subsystem: Some(0x205E1043),
        location: location.map(str::to_string),
        instance_id: format!("PCI\\VEN_10DE&DEV_{device:04X}&SUBSYS_205E1043&REV_A1\\4&{index}"),
        driver_version: version.map(str::to_string),
        driver_key: Some(format!("{{4d36e968-e325-11ce-bfc1-08002be10318}}\\000{index}")),
    }
}

/// The values every test key holds besides the override ones.
pub fn unrelated() -> Vec<(String, Decoded)> {
    vec![
        (
            "DriverDesc".to_string(),
            Decoded::Sz("NVIDIA GeForce RTX 5070".to_string()),
        ),
        ("UnrelatedDword".to_string(), Decoded::Dword(7)),
        ("UnrelatedBinary".to_string(), Decoded::Raw(REG_BINARY, vec![1, 2])),
    ]
}

/// One GPU at 01:00.0 with driver 32.0.16.1656, its key with `unrelated()` and `values`, admin rights and an empty
/// running image (the Python test helper platform()).
pub fn platform(device: u16, values: Vec<(&str, Decoded)>) -> (FakePlatform, FakeKey) {
    platform_with_version(device, values, Some("32.0.16.1656"))
}

/// platform() with another driver version.
pub fn platform_with_version(
    device: u16,
    values: Vec<(&str, Decoded)>,
    version: Option<&str>,
) -> (FakePlatform, FakeKey) {
    let gpu = adapter(0, device, Some("01:00.0"), version);
    let key = FakeKey::new(unrelated());
    for (name, value) in values {
        key.insert(name, value);
    }
    let fake = FakePlatform::new(
        vec![gpu.clone()],
        [(gpu.instance_id.as_str(), key.clone())],
        true,
        Vec::new(),
    );
    (fake, key)
}

/// A REG_SZ text value, as `(REG_SZ, 'text')` in the Python tests.
pub fn sz(text: &str) -> Decoded {
    Decoded::Sz(text.to_string())
}

/// A REG_DWORD value, as `(REG_DWORD, 0x12)` in the Python tests.
pub fn dword(value: u32) -> Decoded {
    Decoded::Dword(value)
}

/// A REG_BINARY value, as `(REG_BINARY, b'...')` in the Python tests.
pub fn binary(data: &[u8]) -> Decoded {
    Decoded::Raw(REG_BINARY, data.to_vec())
}

/// Exact bytes of any type, as `(kind, b'...')` in the Python tests (also a REG_DWORD or REG_SZ that `get` cannot
/// show exactly).
pub fn raw(kind: u32, data: &[u8]) -> Decoded {
    Decoded::Raw(kind, data.to_vec())
}

/// A small deterministic pseudo-random generator (xorshift64*) for the fuzz tests: no crate, same cases each run.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `bound` (0 when `bound` is 0).
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }

    pub fn byte(&mut self) -> u8 {
        self.next_u64() as u8
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }

    /// One of `items`; panics on an empty slice (a test error).
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// A character from a mix of ASCII, controls, whitespace, Unicode digits, case-folding letters and any other
    /// code point.
    pub fn char(&mut self) -> char {
        const SPECIAL: &str =
            "\0\t\n\x0b\x0c\r\x1c\x1f\x7f\u{85}\u{a0}\u{ad}\u{130}\u{131}\u{17f}\u{212a}\u{660}\u{669}\
                               \u{b2}\u{2460}\u{2028}\u{3000}\u{feff}\u{fffd}\u{ff10}\u{1d7ce}\u{1f600}\u{e0001}\
                               \u{10ffff}'\"\\_.:,&+-xX";
        match self.below(4) {
            0 => char::from(b' ' + self.below(95) as u8),
            1 => SPECIAL.chars().nth(self.below(SPECIAL.chars().count())).unwrap_or('x'),
            2 => char::from_u32(0x80 + self.below(0x780) as u32).unwrap_or('?'),
            _ => char::from_u32(self.below(0x110000) as u32).unwrap_or('\u{fffd}'),
        }
    }

    pub fn text(&mut self, len: usize) -> String {
        (0..len).map(|_| self.char()).collect()
    }
}

/// UTF-16LE bytes of a text, as the registry stores REG_SZ data.
pub fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// (type, data, saved BACKUP text or None) of the Python test EVERY_TYPE: the shapes of EnableGpuFirmware that
/// load saves exactly and unload writes back with set_raw (odd sizes, texts without their NUL, DWORDs of another
/// size, unknown types).
pub fn every_type() -> Vec<(u32, Vec<u8>, Option<&'static str>)> {
    let mut big = Vec::new();
    for _ in 0..16 {
        big.extend(0..=255u8);
    }
    vec![
        (REG_SZ, utf16le("17\u{0}"), Some("hex(1):31,00,37,00,00,00")),
        (REG_SZ, utf16le("1\u{0}7\u{0}"), Some("hex(1):31,00,00,00,37,00,00,00")), // data after a NUL
        (REG_SZ, b"".to_vec(), Some("hex(1):")),                                   // empty, no terminator
        (REG_SZ, b"\0\0".to_vec(), Some("hex(1):00,00")),
        (REG_SZ, b"1".to_vec(), Some("hex(1):31")),          // odd size
        (REG_SZ, utf16le("17"), Some("hex(1):31,00,37,00")), // no terminator
        (REG_SZ, b"1\x007\x00\x00".to_vec(), Some("hex(1):31,00,37,00,00")), // odd size, no terminator
        (REG_EXPAND_SZ, utf16le("%a%"), Some("hex(2):25,00,61,00,25,00")),
        (REG_EXPAND_SZ, utf16le("%SystemRoot%\u{0}"), None),
        (
            REG_MULTI_SZ,
            utf16le("a\u{0}bc\u{0}\u{0}"),
            Some("hex(7):61,00,00,00,62,00,63,00,00,00,00,00"),
        ),
        (REG_MULTI_SZ, utf16le("a"), Some("hex(7):61,00")),
        (
            REG_QWORD,
            0x11u64.to_le_bytes().to_vec(),
            Some("hex(b):11,00,00,00,00,00,00,00"),
        ),
        (REG_BINARY, b"\x11".to_vec(), Some("hex(3):11")),
        (REG_NONE, b"".to_vec(), Some("hex(0):")),
        (REG_NONE, b"\x11\0\0\0".to_vec(), Some("hex(0):11,00,00,00")),
        (REG_DWORD, b"\x11\0".to_vec(), Some("hex(4):11,00")),
        (
            REG_DWORD,
            0x12u64.to_le_bytes().to_vec(),
            Some("hex(4):12,00,00,00,00,00,00,00"),
        ),
        (0x12345, b"\xff".to_vec(), Some("hex(12345):ff")),
        (REG_BINARY, big, None), // the largest saved
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{REG_DWORD, REG_SZ};

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn fakes_are_send_and_sync() {
        assert_send_sync::<FakeKey>();
        assert_send_sync::<FakePlatform>();
    }

    #[test]
    fn fake_key_keeps_values_like_the_python_fake() {
        let key = FakeKey::new([("a", dword(1))]);
        let mut writer = key.clone();
        // A REG_SZ written with its NUL is kept as text; without it, as bytes; DWORDs of 4 bytes as numbers.
        writer.set_raw("text", REG_SZ, &[0x31, 0, 0, 0]).unwrap();
        writer.set_raw("odd", REG_SZ, &[0x31, 0]).unwrap();
        writer.set_raw("a", REG_DWORD, &[2, 0, 0, 0]).unwrap();
        assert!(writer.delete("odd").unwrap());
        assert!(!writer.delete("missing").unwrap());
        writer.flush().unwrap();
        assert_eq!(
            key.values(),
            vec![("a".to_string(), dword(2)), ("text".to_string(), sz("1"))]
        );
        let calls: Vec<(String, String)> = [
            ("set", "text"),
            ("set", "odd"),
            ("set", "a"),
            ("delete", "odd"),
            ("delete", "missing"),
        ]
        .iter()
        .map(|(op, name)| (op.to_string(), name.to_string()))
        .collect();
        assert_eq!(key.calls(), calls);
        assert_eq!((key.flushes(), key.closed()), (1, 0));
        // drop_writes loses writes but not deletes, as in Python.
        key.set_drop_writes(true);
        writer.set_raw("b", REG_DWORD, &[1, 0, 0, 0]).unwrap();
        assert!(writer.delete("a").unwrap());
        assert_eq!(key.values(), vec![("text".to_string(), sz("1"))]);
        key.set_values([("z", raw(REG_SZ, b"1\0"))]);
        assert_eq!(
            RegistryKey::get_raw(&key, "z").unwrap(),
            Some((REG_SZ, b"1\0".to_vec()))
        );
        assert_eq!(RegistryKey::get(&key, "z").unwrap(), Some(sz("1")));
    }

    #[test]
    fn fake_platform_counts_opens_closes_and_reads() {
        let (fake, key) = platform(RTX_5070, vec![]);
        let gpu = fake.adapters().unwrap()[0].clone();
        drop(fake.open_key(&gpu, true).unwrap());
        drop(fake.open_key(&gpu, false).unwrap());
        assert_eq!((fake.writable_opens(), key.closed()), (1, 2));
        fake.set_image(b"card".to_vec());
        fake.queue_image(Ok(b"first".to_vec()));
        fake.queue_image(Err(Error::new("failed")));
        assert_eq!(fake.vbios_image(&gpu), Ok(b"first".to_vec()));
        assert_eq!(fake.vbios_image(&gpu), Err(Error::new("failed")));
        assert_eq!(fake.vbios_image(&gpu), Ok(b"card".to_vec()));
        assert_eq!(fake.reads().len(), 3);
        fake.fail_open(&gpu.instance_id, Error::new("gone"));
        assert_eq!(fake.open_key(&gpu, false).err(), Some(Error::new("gone")));
        fake.panic_on_open(Some("boom"));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fake.open_key(&gpu, false).is_ok()));
        assert!(panicked.is_err());
    }

    #[test]
    fn synthetic_rom_layout() {
        let data = rom(RTX_5070, 4);
        assert_eq!(data.len(), PREFIX + 1024 + 512 + 4 * 512 + TRAILING);
        assert_eq!(&data[PREFIX..PREFIX + 2], b"\x55\xaa");
        assert_eq!(
            data[PREFIX..PREFIX + 1024]
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            0
        );
        assert_eq!(&data[PREFIX + 1536..PREFIX + 1538], b"VN");
        let mut rng = Rng::new(1);
        let first = rng.next_u64();
        assert_eq!(Rng::new(1).next_u64(), first);
    }
}
