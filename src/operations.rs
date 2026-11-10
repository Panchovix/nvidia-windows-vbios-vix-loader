//! status, load, unload and verify on a `Platform` (operations.py). Pure logic: no Windows calls of its own.
//!
//! load stores the chain in the adapter's driver key, where the NVIDIA driver reads a VBIOS override: 'vbios'
//! (REG_BINARY), and no 'romOverrideXX' records. 'EnableGpuFirmware' is left as it is; its value before the first
//! load (or its absence) is saved once in BACKUP so that unload can restore it ('absent', '0x%08x' for a REG_DWORD,
//! or 'hex(<type>):<bytes>' in .reg notation for anything else). Loader 0.3.x also set EnableGpuFirmware = 0x11;
//! load puts the saved value back in that case. unload also deletes 'romOverrideXX' records. Nothing else in the
//! key is touched. The driver keeps the image it loaded until Windows restarts. verify reads the image the driver
//! is running (twice, through NvAPI) and compares it with 'vbios'.
//!
//! The reports are `json::Map`s with the keys, order and values of the Python dicts, so `--json` prints the same
//! text and the window reads the same fields.

use crate::devices::{
    decode, normalize_location, nvidia_version, type_name, value_text, Adapter, Decoded, REG_BINARY, REG_DWORD,
};
use crate::error::{Error, Result};
use crate::families;
use crate::image::{extract_chain, Chain};
use crate::json::{Map, Value};
use crate::nvapi::IMAGE_SIZE;
use crate::platform::{Platform, RegistryKey};
use crate::pyfmt::{parse_int, repr_str};
use crate::sha256;

pub const VBIOS: &str = "vbios";
pub const FIRMWARE: &str = "EnableGpuFirmware";
/// The value loader 0.3.x wrote: GSP firmware enabled (1) | fallback allowed (0x10). The driver default is 0x12.
pub const FIRMWARE_OVERRIDE: u32 = 0x11;
pub const BACKUP: &str = "VixLoaderPreviousEnableGpuFirmware";
pub const ABSENT: &str = "absent";
/// Bytes of a non-DWORD EnableGpuFirmware that load saves exactly; larger is refused.
pub const MAX_SAVED: usize = 4096;
pub const MIN_DRIVER: (u32, u32) = (616, 56);
pub const RESTART_HINT: &str = "Restart Windows to apply the change. Shutting down works too when Windows Fast \
Startup is off. With it on, the change waits for the next restart, and nothing has to be repeated.";
/// verify states in which the driver runs what the registry says.
pub const VERIFY_OK: [&str; 2] = ["override-active", "no-override"];

/// The 256 value names 'romOverride00' .. 'romOverrideFF' that unload deletes.
pub fn patch_records() -> Vec<String> {
    (0..256).map(|i| format!("romOverride{i:02X}")).collect()
}

/// Which GPU a command works on: `--gpu INDEX`, `--pci BUS:DEV.FN` or a device instance id (the window).
/// All None: the only NVIDIA adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub gpu: Option<i128>,
    pub pci: Option<String>,
    pub instance_id: Option<String>,
}

/// A value's exact (type, bytes) as `RegistryKey::get_raw` returns it; None when it does not exist.
type RawValue = Option<(u32, Vec<u8>)>;

/// EnableGpuFirmware as loader 0.3.x left it (0x11), as `get_raw` returns it.
fn old_firmware() -> RawValue {
    Some((REG_DWORD, FIRMWARE_OVERRIDE.to_le_bytes().to_vec()))
}

/// One registry change: what `_apply` does and the report's 'steps' show.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// `RegistryKey::set`: a REG_SZ text, a REG_DWORD number or REG_BINARY bytes.
    Set(String, Decoded),
    /// `RegistryKey::set_raw`: the exact type and bytes of a saved non-DWORD value.
    Restore(String, u32, Vec<u8>),
    Delete(String),
}

/// What BACKUP holds (operations._saved): 'absent', a DWORD, or the exact (type, bytes) of a non-DWORD value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Saved {
    Absent,
    Dword(u32),
    Raw(u32, Vec<u8>),
}

/// The integer of a well-formed REG_DWORD value, else None.
fn dword_of(value: &Decoded) -> Option<u32> {
    match value {
        Decoded::Dword(v) => Some(*v),
        _ => None,
    }
}

/// The BACKUP text for a value that is not a well-formed REG_DWORD, as a .reg file writes it.
fn raw_backup(kind: u32, raw: &[u8]) -> String {
    let bytes: Vec<String> = raw.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("hex({kind:x}):{}", bytes.join(","))
}

/// RAW_BACKUP.fullmatch: `hex\((0|[1-9a-f][0-9a-f]{0,7})\):((?:[0-9a-f]{2}(?:,[0-9a-f]{2})*)?)` (lower case only).
fn parse_raw_backup(text: &str) -> Option<(u32, Vec<u8>)> {
    let lower_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    // The type holds no ')', so the first "):" ends it.
    let (kind, data) = text.strip_prefix("hex(")?.split_once("):")?;
    let kind_ok = kind == "0" || (matches!(kind.len(), 1..=8) && kind.bytes().all(lower_hex) && !kind.starts_with('0'));
    if !kind_ok {
        return None;
    }
    let raw = if data.is_empty() {
        Vec::new()
    } else {
        data.split(',')
            .map(|pair| match pair.as_bytes() {
                [high, low] if lower_hex(*high) && lower_hex(*low) => u8::from_str_radix(pair, 16).ok(),
                _ => None,
            })
            .collect::<Option<Vec<u8>>>()?
    };
    Some((u32::from_str_radix(kind, 16).ok()?, raw))
}

/// ABSENT, the DWORD saved in BACKUP, or the (type, bytes) of a saved non-DWORD value; None when malformed.
fn saved(backup: &Decoded) -> Option<Saved> {
    let Decoded::Sz(text) = backup else {
        return None;
    };
    if text == ABSENT {
        return Some(Saved::Absent);
    }
    if let Some((kind, raw)) = parse_raw_backup(text) {
        return (raw.len() <= MAX_SAVED).then_some(Saved::Raw(kind, raw));
    }
    let value = parse_int(text, 16)?;
    u32::try_from(value).ok().map(Saved::Dword)
}

/// BACKUP for status: as stored for absent or a DWORD, readable for an exact copy.
fn backup_text(backup: &Decoded) -> String {
    if let Some(Saved::Raw(kind, raw)) = saved(backup) {
        return format!("{} {}", type_name(kind), value_text(kind, &raw));
    }
    match backup {
        Decoded::Sz(text) => text.clone(),
        other => format!("unexpected {}", type_name(other.kind())),
    }
}

/// The step that writes a saved EnableGpuFirmware back (deletes it when it was absent), and the (type, bytes)
/// that EnableGpuFirmware reads back as afterwards (None: deleted).
fn put_back(saved: &Saved) -> (Step, RawValue) {
    match saved {
        Saved::Absent => (Step::Delete(FIRMWARE.to_string()), None),
        Saved::Dword(value) => (
            Step::Set(FIRMWARE.to_string(), Decoded::Dword(*value)),
            Some((REG_DWORD, value.to_le_bytes().to_vec())),
        ),
        Saved::Raw(kind, raw) => (
            Step::Restore(FIRMWARE.to_string(), *kind, raw.clone()),
            Some((*kind, raw.clone())),
        ),
    }
}

/// put_back of the saved value when EnableGpuFirmware is still the 0x11 that loader 0.3.x wrote over another
/// value; None otherwise (no or a malformed BACKUP, 0x11 saved, or EnableGpuFirmware changed since).
fn set_by_old_loader(firmware: &RawValue, backup: Option<&Decoded>) -> Option<(Step, RawValue)> {
    let saved = saved(backup?)?;
    if *firmware != old_firmware() {
        return None;
    }
    let (step, restored) = put_back(&saved);
    (restored != old_firmware()).then_some((step, restored))
}

/// One adapter by status index, PCI location or device instance id; without any, the only NVIDIA adapter.
pub fn select<'a>(adapters: &'a [Adapter], selection: &Selection) -> Result<&'a Adapter> {
    let given = [
        selection.gpu.is_some(),
        selection.pci.is_some(),
        selection.instance_id.is_some(),
    ];
    if given.iter().filter(|given| **given).count() > 1 {
        return Err(Error::new("use either --gpu or --pci, not both"));
    }
    let matches: Vec<&Adapter> = if let Some(instance_id) = &selection.instance_id {
        adapters.iter().filter(|a| a.instance_id == *instance_id).collect()
    } else if let Some(pci) = &selection.pci {
        let Some(location) = normalize_location(pci) else {
            return Err(Error::new(format!(
                "{} is not a PCI location such as 01:00.0",
                repr_str(pci)
            )));
        };
        adapters
            .iter()
            .filter(|a| a.location.as_deref() == Some(location.as_str()))
            .collect()
    } else if let Some(gpu) = selection.gpu {
        adapters
            .iter()
            .filter(|a| i128::try_from(a.index).is_ok_and(|index| index == gpu))
            .collect()
    } else {
        if adapters.len() > 1 {
            return Err(Error::new(format!(
                "{} NVIDIA GPUs found; choose one with --gpu INDEX or --pci BUS:DEV.FN",
                adapters.len()
            )));
        }
        adapters.iter().collect()
    };
    match matches.as_slice() {
        [adapter] => Ok(adapter),
        [] => Err(Error::new(
            "no NVIDIA GPU matches that selection; run status to list them",
        )),
        _ => Err(Error::new("more than one GPU has that PCI location; use --gpu INDEX")),
    }
}

/// The override-related values of one driver key, before they become the report map.
struct Description {
    /// (type name, the bytes when it is a REG_BINARY) of 'vbios'.
    vbios: Option<(String, Option<Vec<u8>>)>,
    enable_gpu_firmware: Option<String>,
    saved_previous: Option<String>,
    gsp_forced_by_old_loader: bool,
    rom_override_records: Vec<String>,
}

impl Description {
    fn read(key: &dyn RegistryKey) -> Result<Description> {
        let vbios = key.get(VBIOS)?;
        let firmware = key.get_raw(FIRMWARE)?;
        let backup = key.get(BACKUP)?;
        let mut records = Vec::new();
        for name in patch_records() {
            if key.get(&name)?.is_some() {
                records.push(name);
            }
        }
        let enable_gpu_firmware = firmware
            .as_ref()
            .map(|(kind, raw)| match dword_of(&decode(*kind, raw)) {
                Some(value) => format!("{value:#04x}"),
                None => format!("unexpected {} {}", type_name(*kind), value_text(*kind, raw)),
            });
        Ok(Description {
            vbios: vbios.map(|value| {
                let stored = match &value {
                    Decoded::Raw(REG_BINARY, data) => Some(data.clone()),
                    _ => None,
                };
                (type_name(value.kind()), stored)
            }),
            enable_gpu_firmware,
            saved_previous: backup.as_ref().map(backup_text),
            gsp_forced_by_old_loader: set_by_old_loader(&firmware, backup.as_ref()).is_some(),
            rom_override_records: records,
        })
    }

    fn to_map(&self) -> Map {
        let mut map = Map::new();
        let vbios = self.vbios.as_ref().map(|(kind, stored)| {
            let mut vbios = Map::new();
            vbios.insert("type", kind.as_str());
            vbios.insert("size", stored.as_ref().map(Vec::len));
            vbios.insert("sha256", stored.as_deref().map(sha256::hex));
            vbios
        });
        map.insert("vbios", vbios);
        map.insert("enable_gpu_firmware", self.enable_gpu_firmware.clone());
        map.insert("saved_previous_enable_gpu_firmware", self.saved_previous.clone());
        map.insert("gsp_forced_by_old_loader", self.gsp_forced_by_old_loader);
        map.insert("rom_override_records", string_list(&self.rom_override_records));
        map
    }
}

fn string_list(items: &[String]) -> Value {
    Value::Array(items.iter().map(Value::from).collect())
}

/// The override-related values of one driver key (operations.describe).
pub fn describe(key: &dyn RegistryKey) -> Result<Map> {
    Ok(Description::read(key)?.to_map())
}

/// The 'gpu' entry of the reports: the adapter's fields, its family and its NVIDIA driver version
/// (operations._adapter_info).
pub fn adapter_info(adapter: &Adapter) -> Map {
    let version = nvidia_version(adapter.driver_version.as_deref());
    let family = if adapter.vendor == 0x10DE {
        families::family(adapter.device)
    } else {
        "unknown"
    };
    let mut info = Map::new();
    info.insert("index", adapter.index);
    info.insert("name", adapter.name.as_str());
    info.insert("pci_id", format!("{:04x}:{:04x}", adapter.vendor, adapter.device));
    info.insert(
        "subsystem",
        adapter.subsystem.map(|subsystem| format!("{subsystem:08x}")),
    );
    info.insert("location", adapter.location.clone());
    info.insert("instance_id", adapter.instance_id.as_str());
    info.insert("family", family);
    info.insert("driver_version", adapter.driver_version.clone());
    info.insert(
        "nvidia_driver_version",
        version.map(|(major, minor)| format!("{major}.{minor:02}")),
    );
    info.insert("driver_supported", version.is_some_and(|version| version >= MIN_DRIVER));
    info.insert("driver_key", adapter.driver_key.clone());
    info
}

/// One entry per adapter (operations.status): the adapter fields and 'override', or 'override': null and 'error'.
pub fn status(platform: &dyn Platform) -> Result<Vec<Value>> {
    let mut report = Vec::new();
    for adapter in platform.adapters()? {
        let mut entry = adapter_info(&adapter);
        // The key is closed (dropped) as soon as it is described.
        match platform.open_key(&adapter, false).and_then(|key| describe(&*key)) {
            Ok(described) => entry.insert("override", described),
            Err(error) => {
                entry.insert("override", Value::Null);
                entry.insert("error", format!("cannot read the driver key: {error}"));
            }
        }
        report.push(Value::Object(entry));
    }
    Ok(report)
}

fn step_json(step: &Step) -> Value {
    let mut entry = Map::new();
    match step {
        Step::Delete(name) => {
            entry.insert("op", "delete");
            entry.insert("name", name.as_str());
        }
        Step::Restore(name, kind, raw) => {
            // The exact type and bytes of a saved non-DWORD value.
            entry.insert("op", "set");
            entry.insert("name", name.as_str());
            entry.insert("type", type_name(*kind));
            entry.insert("value", value_text(*kind, raw));
            entry.insert("data", raw.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
        }
        Step::Set(name, value) => {
            entry.insert("op", "set");
            entry.insert("name", name.as_str());
            entry.insert("type", type_name(value.kind()));
            match value {
                Decoded::Raw(REG_BINARY, data) => {
                    entry.insert("size", data.len());
                    entry.insert("sha256", sha256::hex(data));
                }
                Decoded::Dword(number) => entry.insert("value", format!("{number:#04x}")),
                Decoded::Sz(text) => entry.insert("value", text.as_str()),
                Decoded::Raw(kind, raw) => entry.insert("value", value_text(*kind, raw)),
            }
        }
    }
    Value::Object(entry)
}

fn apply_steps(key: &mut dyn RegistryKey, steps: &[Step]) -> Result<()> {
    for step in steps {
        match step {
            Step::Set(name, value) => key.set(name, value)?,
            Step::Restore(name, kind, raw) => key.set_raw(name, *kind, raw)?,
            Step::Delete(name) => {
                key.delete(name)?;
            }
        }
    }
    key.flush()
}

/// Write the steps and flush; a failure says that the key may be half written.
fn apply(key: &mut dyn RegistryKey, steps: &[Step]) -> Result<()> {
    apply_steps(key, steps).map_err(|error| {
        Error::new(format!(
            "{error}; the registry may be partly changed: run status, then unload, before restarting Windows"
        ))
    })
}

/// True when any 'romOverrideXX' record exists (stops at the first one).
fn any_record(key: &dyn RegistryKey) -> Result<bool> {
    for name in patch_records() {
        if key.get(&name)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn prepare(platform: &dyn Platform, selection: &Selection, dry_run: bool) -> Result<Adapter> {
    let adapters = platform.adapters()?;
    let adapter = select(&adapters, selection)?.clone();
    if !dry_run && !platform.is_admin() {
        return Err(Error::new(
            "administrator rights are required; run this from an elevated PowerShell",
        ));
    }
    Ok(adapter)
}

/// operations.load: validate the payload for the selected GPU, plan and (unless dry_run) apply the registry steps.
pub fn load(platform: &dyn Platform, data: &[u8], selection: &Selection, dry_run: bool) -> Result<Map> {
    let adapter = prepare(platform, selection, dry_run)?;
    let Some(version) = nvidia_version(adapter.driver_version.as_deref()) else {
        let shown = adapter
            .driver_version
            .as_deref()
            .map_or_else(|| "None".to_string(), repr_str);
        return Err(Error::new(format!(
            "cannot read the NVIDIA driver version of GPU {} ({shown})",
            adapter.index
        )));
    };
    if version < MIN_DRIVER {
        return Err(Error::new(format!(
            "NVIDIA driver {}.{:02} is too old; {}.{} or newer is required",
            version.0, version.1, MIN_DRIVER.0, MIN_DRIVER.1
        )));
    }
    let chain = extract_chain(data, adapter.vendor, adapter.device).map_err(|error| {
        Error::new(format!(
            "payload refused for GPU {} ({:04x}:{:04x}): {error}",
            adapter.index, adapter.vendor, adapter.device
        ))
    })?;
    let mut key = platform.open_key(&adapter, !dry_run)?;
    let report = load_into(&mut *key, &adapter, data, &chain, dry_run);
    drop(key);
    let mut report = report?;
    if !dry_run {
        // The driver reads the new image only after a Windows restart.
        report.insert("next_step", RESTART_HINT);
    }
    Ok(report)
}

/// The part of load that works on the open key (the try block of operations.load).
fn load_into(key: &mut dyn RegistryKey, adapter: &Adapter, data: &[u8], chain: &Chain, dry_run: bool) -> Result<Map> {
    let before = Description::read(&*key)?;
    let firmware = key.get_raw(FIRMWARE)?;
    let mut backup = key.get(BACKUP)?;
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    let mut extra = Map::new();
    // EnableGpuFirmware reads back unchanged unless loader 0.3.x set it.
    let mut expected = firmware.clone();
    let mut old = None;
    if let Some(existing) = &backup {
        if saved(existing).is_none() {
            return Err(Error::new(format!(
                "{BACKUP} holds an unexpected value; run unload first"
            )));
        }
        old = set_by_old_loader(&firmware, Some(existing));
    } else {
        let text = match &firmware {
            None => ABSENT.to_string(),
            Some((kind, raw)) => match dword_of(&decode(*kind, raw)) {
                Some(value) => format!("{value:#010x}"),
                None => {
                    // Set by another tool or by hand: saved byte for byte, and unload writes it back the same way.
                    if raw.len() > MAX_SAVED {
                        return Err(Error::new(format!(
                            "{FIRMWARE} holds a {} value of {} bytes, too large to save (at most {MAX_SAVED}); not \
                             changing it",
                            type_name(*kind),
                            raw.len()
                        )));
                    }
                    let text = raw_backup(*kind, raw);
                    let mut info = Map::new();
                    info.insert("type", type_name(*kind));
                    info.insert("size", raw.len());
                    info.insert("value", value_text(*kind, raw));
                    info.insert("backup", text.as_str());
                    extra.insert("raw_backup", info);
                    text
                }
            },
        };
        steps.push(Step::Set(BACKUP.to_string(), Decoded::Sz(text.clone())));
        backup = Some(Decoded::Sz(text));
        if before.vbios.is_some() || !before.rom_override_records.is_empty() {
            extra.insert("replaces_other_override", true);
            let current = before.enable_gpu_firmware.as_deref().unwrap_or("not set");
            let current = current.strip_prefix("unexpected ").unwrap_or(current);
            notes.push(format!(
                "An override written by another tool (for example mVolt+) is replaced and will not be restored; \
                 unload restores {FIRMWARE} to its current value ({current})."
            ));
        }
    }
    steps.extend(
        before
            .rom_override_records
            .iter()
            .map(|name| Step::Delete(name.clone())),
    );
    steps.push(Step::Set(
        VBIOS.to_string(),
        Decoded::Raw(REG_BINARY, chain.data.clone()),
    ));
    if let Some((step, restored)) = old {
        // Loader 0.3.x replaced the saved value with 0x11: put it back with the step unload uses.
        steps.push(step);
        expected = restored;
        let shown = before.saved_previous.clone().unwrap_or_else(|| "None".to_string());
        extra.insert("restores_firmware", before.saved_previous.clone());
        notes.push(format!(
            "{FIRMWARE} goes back to its value before the first load: {shown}. Loader 0.3.2 or older set it to \
             {FIRMWARE_OVERRIDE:#04x}, which turns on the driver's GSP firmware mode; this version no longer turns \
             that mode on."
        ));
    } else if expected == old_firmware() {
        // unload writes the saved 0x11 back, so the note says to delete it while no override is loaded.
        extra.insert("keeps_gsp_firmware_mode", true);
        notes.push(format!(
            "GSP firmware mode stays on because {FIRMWARE} was already {FIRMWARE_OVERRIDE:#04x} before the first \
             load (for example set by mVolt+). For the driver's default mode, run unload (it puts \
             {FIRMWARE_OVERRIDE:#04x} back), delete {FIRMWARE}, load again and restart Windows."
        ));
    }
    let mut payload = Map::new();
    payload.insert("family", chain.family);
    payload.insert("size", chain.data.len());
    payload.insert("sha256", sha256::hex(&chain.data));
    payload.insert("file_size", data.len());
    payload.insert("dropped_prefix", chain.start);
    payload.insert("dropped_trailing", data.len() - chain.end);
    let mut report = Map::new();
    report.insert("gpu", adapter_info(adapter));
    report.insert("dry_run", dry_run);
    report.insert("before", before.to_map());
    report.insert("payload", payload);
    report.insert("steps", Value::Array(steps.iter().map(step_json).collect()));
    for (name, value) in extra.iter() {
        report.insert(name, value.clone());
    }
    if !notes.is_empty() {
        report.insert("message", notes.join(" "));
    }
    if dry_run {
        return Ok(report);
    }
    apply(key, &steps)?;
    let after_vbios = key.get(VBIOS)?;
    let after_firmware = key.get_raw(FIRMWARE)?;
    let after_backup = key.get(BACKUP)?;
    if after_vbios != Some(Decoded::Raw(REG_BINARY, chain.data.clone()))
        || after_firmware != expected
        || after_backup != backup
        || any_record(&*key)?
    {
        return Err(Error::new(
            "the registry values read back differ from what was written; run unload",
        ));
    }
    report.insert("after", describe(&*key)?);
    Ok(report)
}

/// operations.unload.
pub fn unload(platform: &dyn Platform, selection: &Selection, dry_run: bool) -> Result<Map> {
    let adapter = prepare(platform, selection, dry_run)?;
    let mut key = platform.open_key(&adapter, !dry_run)?;
    let result = unload_from(&mut *key, &adapter, dry_run);
    drop(key);
    let (mut report, applied) = result?;
    if applied {
        // The driver keeps the removed image cached, and running, until Windows restarts.
        report.insert("next_step", RESTART_HINT);
    }
    Ok(report)
}

/// The part of unload that works on the open key; true when the steps were applied.
fn unload_from(key: &mut dyn RegistryKey, adapter: &Adapter, dry_run: bool) -> Result<(Map, bool)> {
    let before = Description::read(&*key)?;
    let backup = key.get(BACKUP)?;
    let mut steps = Vec::new();
    let mut message = None;
    let mut expected_firmware = key.get_raw(FIRMWARE)?;
    match backup.as_ref().and_then(saved) {
        Some(saved) => {
            let (step, restored) = put_back(&saved);
            expected_firmware = restored;
            steps.push(step);
        }
        None if backup.is_some() => {
            message = Some(format!(
                "{BACKUP} holds an unexpected value; {FIRMWARE} is left as it is."
            ));
        }
        None => message = Some(format!("{FIRMWARE} was not saved by this tool and is left as it is.")),
    }
    steps.extend(
        before
            .rom_override_records
            .iter()
            .map(|name| Step::Delete(name.clone())),
    );
    if before.vbios.is_some() {
        steps.push(Step::Delete(VBIOS.to_string()));
    }
    if backup.is_some() {
        steps.push(Step::Delete(BACKUP.to_string()));
    }
    let mut report = Map::new();
    report.insert("gpu", adapter_info(adapter));
    report.insert("dry_run", dry_run);
    report.insert("before", before.to_map());
    report.insert("steps", Value::Array(steps.iter().map(step_json).collect()));
    if steps.is_empty() {
        report.insert("message", "No override is stored for this GPU.");
        return Ok((report, false));
    }
    if let Some(message) = message {
        report.insert("message", message);
    }
    if dry_run {
        return Ok((report, false));
    }
    // The saved value is deleted only after EnableGpuFirmware reads back exactly as expected.
    let delete_backup = Step::Delete(BACKUP.to_string());
    let first: Vec<Step> = steps.iter().filter(|step| **step != delete_backup).cloned().collect();
    apply(key, &first)?;
    if key.get_raw(FIRMWARE)? != expected_firmware {
        let kept = if backup.is_some() {
            format!("; {BACKUP} is kept, run unload again")
        } else {
            String::new()
        };
        return Err(Error::new(format!(
            "{FIRMWARE} read back differs from what was expected{kept}"
        )));
    }
    if backup.is_some() {
        apply(key, &[delete_backup])?;
    }
    if key.get(VBIOS)?.is_some() || key.get(BACKUP)?.is_some() || any_record(&*key)? {
        return Err(Error::new(
            "the registry values read back differ from what was expected",
        ));
    }
    report.insert("after", describe(&*key)?);
    Ok((report, true))
}

fn image_map(data: &[u8]) -> Map {
    let mut image = Map::new();
    image.insert("size", data.len());
    image.insert("sha256", sha256::hex(data));
    image
}

/// operations.verify: read the running image twice and compare it with 'vbios'. Read-only.
pub fn verify(platform: &dyn Platform, selection: &Selection) -> Result<Map> {
    let adapters = platform.adapters()?;
    let adapter = select(&adapters, selection)?;
    let (stored, records) = {
        let key = platform.open_key(adapter, false)?;
        let stored = key.get(VBIOS)?;
        (stored, Description::read(&*key)?.rom_override_records)
    };
    let configured = match stored {
        Some(Decoded::Raw(REG_BINARY, data)) => Some(data),
        _ => None,
    };
    let running = platform.vbios_image(adapter)?;
    let again = platform.vbios_image(adapter)?;
    let mut readback = image_map(&running);
    readback.insert("truncated", running.len() == IMAGE_SIZE);
    let mut report = Map::new();
    report.insert("gpu", adapter_info(adapter));
    report.insert("readback", readback);
    report.insert("configured", configured.as_deref().map(image_map));
    report.insert("match", Value::Null);
    report.insert("compared", Value::Null);
    report.insert("first_difference", Value::Null);
    report.insert("rom_override_records", string_list(&records));
    let (state, message) = if running != again {
        report.insert("second_readback", image_map(&again));
        (
            "unstable",
            "Two reads of the running VBIOS differ; try again.".to_string(),
        )
    } else if let Some(configured) = &configured {
        // NvAPI cuts the image at IMAGE_SIZE bytes: a longer override is compared up to there.
        let prefix = running.len() == IMAGE_SIZE && IMAGE_SIZE < configured.len();
        let expected = if prefix {
            &configured[..IMAGE_SIZE]
        } else {
            &configured[..]
        };
        let matched = running == expected;
        report.insert("match", matched);
        report.insert("compared", if prefix { "prefix" } else { "full" });
        if matched && prefix {
            let message = format!(
                "The first {IMAGE_SIZE} bytes the driver is running match the stored override; NvAPI returns no \
                 more, so the last {} bytes cannot be read back.",
                configured.len() - IMAGE_SIZE
            );
            ("override-active", message)
        } else if matched {
            (
                "override-active",
                "The driver is using the stored override (verified byte by byte).".to_string(),
            )
        } else {
            let offset = running
                .iter()
                .zip(expected)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| running.len().min(expected.len()));
            report.insert("first_difference", offset);
            let message = format!(
                "The driver is NOT using the stored override (first difference at offset {offset:#x}). If it was \
                 just loaded, restart Windows; otherwise the driver may have rejected it and is running the card's \
                 own VBIOS."
            );
            ("override-not-active", message)
        }
    } else {
        // The driver keeps a removed override in its image cache, running, until Windows restarts, and the
        // readback cannot tell it from the card's own VBIOS.
        let mut message = "No override is stored, so the driver should be running the card's own VBIOS. If one \
                           was just removed, restart Windows first."
            .to_string();
        if !records.is_empty() {
            message.push_str(" romOverride records are stored too; the driver may apply them to the card's VBIOS.");
        }
        ("no-override", message)
    };
    report.insert("state", state);
    report.insert("message", message);
    Ok(report)
}

#[cfg(test)]
mod tests;
