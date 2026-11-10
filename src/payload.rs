//! Override payload checks that mirror the patched kernel loader (payload.py): exactly one NVIDIA legacy PCI image,
//! for this GPU's PCI ID, with a zero checksum, plus the audited GSP receiver capacities.

use std::fmt;

use crate::families;

pub const MAX_SIZE: usize = 4 * 1024 * 1024;
pub const GA10X_MAX_SIZE: usize = 512 * 1024;
pub const SCOPE: [&str; 4] = ["turing", "ampere", "ada", "blackwell-gb20x"];

/// Why a payload is refused (payload.PayloadError); the text is shown to the user as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadError(pub String);

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PayloadError {}

/// Little-endian u16 at `at`; callers have checked the bounds (Python's struct.unpack_from('<H')).
fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// Offset of the single legacy image the kernel would select.
pub fn find_legacy(data: &[u8], vendor: u16, device: u16) -> Result<usize, PayloadError> {
    let size = data.len();
    if !(0x200..=MAX_SIZE).contains(&size) {
        return Err(PayloadError(format!(
            "payload is {size} bytes; the loader accepts 512 B to 4 MiB"
        )));
    }
    let pci_id = u32::from(vendor) | u32::from(device) << 16;
    let mut found = Vec::new();
    // Every offset leaves at least 0x200 bytes, so the header reads below stay inside the data.
    for offset in (0..=size - 0x200).step_by(0x200) {
        if data[offset..offset + 2] != [0x55, 0xAA] {
            continue;
        }
        let relative = usize::from(u16_at(data, offset + 0x18));
        if relative > size - offset - 0x18 {
            continue;
        }
        let pcir = offset + relative;
        if &data[pcir..pcir + 4] != b"PCIR" || data[pcir + 0x14] != 0 {
            continue;
        }
        let image_id = u32_at(data, pcir + 4);
        let length = usize::from(u16_at(data, pcir + 0x10)) * 512;
        if image_id != pci_id {
            return Err(PayloadError(format!(
                "legacy image at {offset:#x} is for PCI ID {:04x}:{:04x}, not {vendor:04x}:{device:04x}",
                image_id & 0xffff,
                image_id >> 16
            )));
        }
        if length == 0 || length > size - offset || relative > length || length - relative < 0x18 {
            return Err(PayloadError(format!(
                "legacy image at {offset:#x} has an invalid length"
            )));
        }
        if data[offset..offset + length]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
            != 0
        {
            return Err(PayloadError(format!(
                "legacy image at {offset:#x} has a nonzero checksum"
            )));
        }
        found.push(offset);
    }
    match found.as_slice() {
        [offset] => Ok(*offset),
        _ => {
            let hint = if found.len() > 1 {
                " (a full ROM with two banks?); load a single-chain payload instead: Vix Editor saves one as \
                 <name>-loader.bin next to an edited Turing, Ampere or Ada copy, and its `normalize` command \
                 exports one from any ROM"
            } else {
                ""
            };
            Err(PayloadError(format!(
                "payload has {} legacy images; the driver needs exactly one{hint}",
                found.len()
            )))
        }
    }
}

/// Bytes the driver hands to the GSP: from the selected legacy image to the end of the data.
pub fn gsp_bytes(data: &[u8], vendor: u16, device: u16) -> Result<usize, PayloadError> {
    Ok(data.len() - find_legacy(data, vendor, device)?)
}

/// Validate a payload for this GPU and return its architecture family.
pub fn check_payload(data: &[u8], vendor: u16, device: u16) -> Result<&'static str, PayloadError> {
    if vendor != 0x10de {
        return Err(PayloadError("only NVIDIA GPUs are supported".to_string()));
    }
    let family = families::family(device);
    if !SCOPE.contains(&family) {
        return Err(PayloadError(format!(
            "PCI device {device:04x} is outside the supported Turing/Ampere/Ada/GB20x scope"
        )));
    }
    find_legacy(data, vendor, device)?;
    Ok(family)
}

/// check_payload plus the audited 512 KiB TU10x/TU11x, GA10x and AD10x GSP receiver capacity.
///
/// The driver sends everything from the selected legacy image to the end of the payload. GA100 uses different,
/// unaudited firmware; no 512 KiB capacity is inferred for it or for Blackwell. The Turing value comes from static
/// emulation of gsp_tu10x.bin 615.78.08, not from hardware.
pub fn check_loadable_payload(data: &[u8], vendor: u16, device: u16) -> Result<&'static str, PayloadError> {
    let family = check_payload(data, vendor, device)?;
    if family == "ada" || family == "turing" || (family == "ampere" && !families::is_ga100(device)) {
        let size = gsp_bytes(data, vendor, device)?;
        if size > GA10X_MAX_SIZE {
            let chip = match family {
                "ampere" => "Ampere GA10x",
                "ada" => "Ada AD10x",
                _ => "Turing TU10x/TU11x",
            };
            let hint = if family == "turing" {
                // One legacy image (two are refused above), so the extra bytes follow the image chain: plain
                // normalize keeps them, the e0-end export ends at the last data image.
                "The file keeps flash bytes after its image chain (a full-size dump whose second bank is erased?). \
                 Load the chain alone instead: Vix Editor saves it as <name>-loader-e0end.bin next to an edited \
                 copy, or export it with `nvbios normalize edited.rom payload.bin --representation \
                 turing-e0-end-v1`."
                    .to_string()
            } else {
                let representation = if family == "ampere" {
                    "ampere-relocated-v1"
                } else {
                    "ada-relocated-v2"
                };
                format!(
                    "Load an experimental relocated payload instead: Vix Editor saves one as <name>-loader.bin next \
                     to an edited copy, or export it with `nvbios normalize edited.rom payload.bin --representation \
                     {representation}`."
                )
            };
            return Err(PayloadError(format!(
                "{chip} GSP accepts at most 512 KiB (524288 bytes) from the legacy image; this payload sends {size} \
                 bytes. {hint}"
            )));
        }
    }
    Ok(family)
}
