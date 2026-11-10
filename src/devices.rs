//! NVIDIA display adapters and registry values (devices.py): types, value decoding and the PCI text helpers.
//! Nothing here needs Windows.

use std::fmt::Write as _;

use crate::pyfmt::{decimal_value, is_digit, repr_list, repr_str, strip};

pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_QWORD: u32 = 11;

/// A registry value as `RegistryKey::get` returns it (devices.decode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    /// A REG_DWORD of exactly 4 bytes.
    Dword(u32),
    /// A REG_SZ: its UTF-16LE text (invalid units replaced by U+FFFD) up to the first NUL.
    Sz(String),
    /// Any other type, or a REG_DWORD of another size: the type and the bytes as stored.
    Raw(u32, Vec<u8>),
}

impl Decoded {
    /// The registry type of this value.
    pub fn kind(&self) -> u32 {
        match self {
            Decoded::Dword(_) => REG_DWORD,
            Decoded::Sz(_) => REG_SZ,
            Decoded::Raw(kind, _) => *kind,
        }
    }
}

/// A present NVIDIA display adapter (devices.Adapter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// Position in `Platform::adapters()`, the index `status` shows and `--gpu` takes.
    pub index: usize,
    pub name: String,
    pub vendor: u16,
    pub device: u16,
    /// Subsystem device/vendor dword, as in SUBSYS_xxxxxxxx.
    pub subsystem: Option<u32>,
    /// PCI bus:device.function, for example "01:00.0".
    pub location: Option<String>,
    pub instance_id: String,
    /// Windows form, for example "32.0.16.1656".
    pub driver_version: Option<String>,
    /// "{class guid}\NNNN" under ...\Control\Class.
    pub driver_key: Option<String>,
}

/// "REG_DWORD", ..., or "type N" for a type without a name.
pub fn type_name(kind: u32) -> String {
    let name = match kind {
        REG_NONE => "REG_NONE",
        REG_SZ => "REG_SZ",
        REG_EXPAND_SZ => "REG_EXPAND_SZ",
        REG_BINARY => "REG_BINARY",
        REG_DWORD => "REG_DWORD",
        REG_MULTI_SZ => "REG_MULTI_SZ",
        REG_QWORD => "REG_QWORD",
        _ => return format!("type {kind}"),
    };
    name.to_string()
}

/// Python's `raw.decode('utf-16-le', 'replace')`: a lone surrogate unit becomes U+FFFD; a high surrogate at the
/// end of the data becomes one U+FFFD together with an odd byte after it; an odd trailing byte alone becomes one
/// U+FFFD. A byte order mark is kept as U+FEFF.
pub fn decode_utf16le(raw: &[u8]) -> String {
    let unit = |at: usize| u16::from_le_bytes([raw[at], raw[at + 1]]);
    let mut out = String::with_capacity(raw.len() / 2);
    let mut at = 0;
    while raw.len() - at >= 2 {
        let first = unit(at);
        at += 2;
        if !(0xD800..=0xDFFF).contains(&first) {
            out.push(char::from_u32(u32::from(first)).unwrap_or('\u{FFFD}'));
            continue;
        }
        if first >= 0xDC00 {
            // "illegal encoding": a low surrogate without a high one.
            out.push('\u{FFFD}');
            continue;
        }
        if raw.len() - at < 2 {
            // "unexpected end of data": the replacement covers everything to the end.
            out.push('\u{FFFD}');
            at = raw.len();
            break;
        }
        let second = unit(at);
        if (0xDC00..=0xDFFF).contains(&second) {
            at += 2;
            let cp = 0x10000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
        } else {
            // "illegal UTF-16 surrogate": only the high unit is replaced; the next unit is decoded again.
            out.push('\u{FFFD}');
        }
    }
    if at < raw.len() {
        // "truncated data": the odd trailing byte.
        out.push('\u{FFFD}');
    }
    out
}

/// (type, value) as `RegistryKey::get` returns it: a DWORD for a 4-byte REG_DWORD, the text up to the first NUL
/// for REG_SZ (decoded with Python's 'utf-16-le' + 'replace' rules, including an odd trailing byte), else the bytes.
pub fn decode(kind: u32, raw: &[u8]) -> Decoded {
    if kind == REG_DWORD && raw.len() == 4 {
        return Decoded::Dword(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]));
    }
    if kind == REG_SZ {
        let text = decode_utf16le(raw);
        let end = text.find('\0').unwrap_or(text.len());
        return Decoded::Sz(text[..end].to_string());
    }
    Decoded::Raw(kind, raw.to_vec())
}

/// The type and bytes `RegistryKey::set` writes: 4 bytes for a DWORD, NUL-terminated UTF-16LE for a REG_SZ text,
/// else the bytes as they are.
pub fn encode(value: &Decoded) -> (u32, Vec<u8>) {
    match value {
        Decoded::Dword(v) => (REG_DWORD, v.to_le_bytes().to_vec()),
        Decoded::Sz(text) => (
            REG_SZ,
            text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect(),
        ),
        Decoded::Raw(kind, raw) => (*kind, raw.clone()),
    }
}

/// Python's `data.hex(' ')`: lowercase byte pairs separated by spaces.
pub fn hex_spaced(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 3);
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A short readable form of a value's exact data, for status and the planned steps (devices.value_text): the
/// number for a DWORD/QWORD of the right size, Python repr() of the text (or of the list for REG_MULTI_SZ) cut to
/// 60 characters plus the size for text types of even length, else the size and the first 16 bytes in hex.
pub fn value_text(kind: u32, raw: &[u8]) -> String {
    let size = format!("{} byte{}", raw.len(), if raw.len() == 1 { "" } else { "s" });
    if (kind == REG_DWORD && raw.len() == 4) || (kind == REG_QWORD && raw.len() == 8) {
        let value = raw.iter().rev().fold(0u64, |acc, byte| acc << 8 | u64::from(*byte));
        return format!("{value:#04x}");
    }
    if matches!(kind, REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ) && raw.len() % 2 == 0 {
        let text = decode_utf16le(raw);
        let shown = if kind == REG_MULTI_SZ {
            let text = text
                .strip_suffix("\0\0")
                .or_else(|| text.strip_suffix('\0'))
                .unwrap_or(&text);
            let items: Vec<String> = if text.is_empty() {
                Vec::new()
            } else {
                text.split('\0').map(String::from).collect()
            };
            repr_list(&items)
        } else {
            repr_str(text.strip_suffix('\0').unwrap_or(&text))
        };
        // Lengths and the cut are in characters, as Python counts them.
        let shown = if shown.chars().count() <= 60 {
            shown
        } else {
            let mut cut: String = shown.chars().take(57).collect();
            cut.push_str("...");
            cut
        };
        return format!("{shown} ({size})");
    }
    if raw.is_empty() {
        return size;
    }
    let more = if raw.len() > 16 { " ..." } else { "" };
    format!("{size}: {}{more}", hex_spaced(&raw[..raw.len().min(16)]))
}

/// Matches `pattern` (ASCII) at `chars[*pos..]` the way a Python str regex with re.I does: ASCII letters in either
/// case, plus U+0130 and U+0131 for 'i' and U+017F for 's' (Unicode case folding); advances `pos` on success.
fn literal_ignore_case(chars: &[char], pos: &mut usize, pattern: &str) -> Option<()> {
    for expected in pattern.chars() {
        let found = *chars.get(*pos)?;
        let same = found.eq_ignore_ascii_case(&expected)
            || (expected.eq_ignore_ascii_case(&'i') && matches!(found, '\u{130}' | '\u{131}'))
            || (expected.eq_ignore_ascii_case(&'s') && found == '\u{17f}');
        if !same {
            return None;
        }
        *pos += 1;
    }
    Some(())
}

/// Exactly `count` ASCII hex digits at `chars[*pos..]` ([0-9A-F] with re.I matches no other character).
fn hex_field(chars: &[char], pos: &mut usize, count: usize) -> Option<u32> {
    let field = chars.get(*pos..*pos + count)?;
    let mut value = 0u32;
    for c in field {
        value = value << 4 | c.to_digit(16).filter(|_| c.is_ascii())?;
    }
    *pos += count;
    Some(value)
}

/// (vendor, device, subsystem) from "PCI\VEN_10DE&DEV_2C09&SUBSYS_205E1043&REV_A1\...", or None.
pub fn parse_instance_id(instance_id: &str) -> Option<(u16, u16, Option<u32>)> {
    // re.match(r'PCI\\VEN_([0-9A-F]{4})&DEV_([0-9A-F]{4})(?:&SUBSYS_([0-9A-F]{8}))?', instance_id, re.I)
    let chars: Vec<char> = instance_id.chars().collect();
    let mut pos = 0;
    literal_ignore_case(&chars, &mut pos, "PCI\\VEN_")?;
    let vendor = hex_field(&chars, &mut pos, 4)?;
    literal_ignore_case(&chars, &mut pos, "&DEV_")?;
    let device = hex_field(&chars, &mut pos, 4)?;
    let subsystem = literal_ignore_case(&chars, &mut pos, "&SUBSYS_").and_then(|()| hex_field(&chars, &mut pos, 8));
    Some((vendor as u16, device as u16, subsystem))
}

/// SPDRP_BUSNUMBER and SPDRP_ADDRESS (device << 16 | function) -> "01:00.0"; None when missing or invalid.
pub fn pci_location(bus: Option<u32>, address: Option<u32>) -> Option<String> {
    // Unlike the location text, these numbers do not depend on the Windows display language.
    let (bus, address) = (bus?, address?);
    let (device, function) = (address >> 16, address & 0xFFFF);
    if !(bus <= 0xFF && device < 32 && function < 8) {
        return None;
    }
    Some(format!("{bus:02x}:{device:02x}.{function:x}"))
}

/// The run of Unicode decimal digits (`\d+`) at `chars[*pos..]`, as digit values; None when there is none.
fn decimal_run(chars: &[char], pos: &mut usize) -> Option<Vec<u8>> {
    let mut digits = Vec::new();
    while let Some(value) = chars.get(*pos).and_then(|c| decimal_value(*c)) {
        digits.push(value as u8);
        *pos += 1;
    }
    (!digits.is_empty()).then_some(digits)
}

/// `f'{int(digits):0{width}x}'` for decimal digits of any length; None beyond Python's 4300-digit int() limit
/// (where Python raises ValueError).
fn decimal_to_hex(digits: &[u8], width: usize) -> Option<String> {
    if digits.len() > 4300 {
        return None;
    }
    // Little-endian 32-bit limbs: value = value * 10 + digit.
    let mut limbs: Vec<u32> = vec![0];
    for &digit in digits {
        let mut carry = u64::from(digit);
        for limb in limbs.iter_mut() {
            let product = u64::from(*limb) * 10 + carry;
            *limb = product as u32;
            carry = product >> 32;
        }
        if carry > 0 {
            limbs.push(carry as u32);
        }
    }
    while limbs.len() > 1 && limbs.last() == Some(&0) {
        limbs.pop();
    }
    let mut text = String::new();
    for (i, limb) in limbs.iter().rev().enumerate() {
        if i == 0 {
            let _ = write!(text, "{limb:x}");
        } else {
            let _ = write!(text, "{limb:08x}");
        }
    }
    Some(format!("{text:0>width$}"))
}

/// "PCI bus 1, device 0, function 0" -> "01:00.0"; None when the text is not an English PCI location.
pub fn parse_location(text: Option<&str>) -> Option<String> {
    // re.search(r'PCI bus (\d+), device (\d+), function (\d+)', text or '', re.I): the first position where the
    // whole pattern matches. \d+ is greedy and only a comma or the end follows it, so no backtracking is needed.
    let chars: Vec<char> = text.unwrap_or("").chars().collect();
    let at = |start: usize| -> Option<[Vec<u8>; 3]> {
        let mut pos = start;
        literal_ignore_case(&chars, &mut pos, "PCI bus ")?;
        let bus = decimal_run(&chars, &mut pos)?;
        literal_ignore_case(&chars, &mut pos, ", device ")?;
        let device = decimal_run(&chars, &mut pos)?;
        literal_ignore_case(&chars, &mut pos, ", function ")?;
        let function = decimal_run(&chars, &mut pos)?;
        Some([bus, device, function])
    };
    let [bus, device, function] = (0..chars.len()).find_map(at)?;
    Some(format!(
        "{}:{}.{}",
        decimal_to_hex(&bus, 2)?,
        decimal_to_hex(&device, 2)?,
        decimal_to_hex(&function, 1)?
    ))
}

/// 1 to `max` ASCII hex digits, the whole of `part` ([0-9a-f]{1,max} with re.I).
fn hex_part(part: &str, max: usize) -> Option<u32> {
    if part.is_empty() || part.len() > max || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(part, 16).ok()
}

/// User input "BUS:DEV.FN" (hex, optional 0000: domain) -> "01:00.0", or None when malformed.
pub fn normalize_location(text: &str) -> Option<String> {
    // re.fullmatch(r'(?:([0-9a-f]{1,4}):)?([0-9a-f]{1,2}):([0-9a-f]{1,2})\.([0-7])', text.strip(), re.I):
    // the classes hold no ':' or '.', so the number of colons decides whether the domain is there.
    let parts: Vec<&str> = strip(text).split(':').collect();
    let (domain, bus, rest) = match parts.as_slice() {
        [bus, rest] => ("0", *bus, *rest),
        [domain, bus, rest] => (*domain, *bus, *rest),
        _ => return None,
    };
    let (device, function) = rest.split_once('.')?;
    let function = match function.as_bytes() {
        [digit @ b'0'..=b'7'] => u32::from(digit - b'0'),
        _ => return None,
    };
    if hex_part(domain, 4)? != 0 {
        return None;
    }
    let (bus, device) = (hex_part(bus, 2)?, hex_part(device, 2)?);
    (device < 32).then(|| format!("{bus:02x}:{device:02x}.{function:x}"))
}

/// Windows driver version "32.0.16.1656" -> (616, 56): the last five digits of the last two groups.
///
/// Like Python, every group must be str.isdigit(); one difference: where the five digits hold a character that
/// str.isdigit() accepts but int() does not (a superscript, for example), Python raised ValueError and this
/// returns None (the version cannot be read).
pub fn nvidia_version(windows_version: Option<&str>) -> Option<(u32, u32)> {
    let parts: Vec<&str> = strip(windows_version.unwrap_or("")).split('.').collect();
    if parts.len() != 4 || !parts.iter().all(|part| !part.is_empty() && part.chars().all(is_digit)) {
        return None;
    }
    // (parts[2] + parts[3].zfill(4))[-5:]
    let last: Vec<char> = parts[3].chars().collect();
    let mut digits: Vec<char> = parts[2].chars().collect();
    digits.extend(std::iter::repeat_n('0', 4usize.saturating_sub(last.len())));
    digits.extend(last);
    let digits = &digits[digits.len() - 5..];
    let number = |chars: &[char]| {
        chars
            .iter()
            .try_fold(0u32, |acc, c| Some(acc * 10 + decimal_value(*c)?))
    };
    Some((number(&digits[..3])?, number(&digits[3..])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn windows_strings() {
        // SelectionTests.test_windows_strings
        assert_eq!(nvidia_version(Some("32.0.16.1656")), Some((616, 56)));
        assert_eq!(nvidia_version(Some("31.0.15.5222")), Some((552, 22)));
        assert_eq!(nvidia_version(Some("32.0.16.612")), Some((606, 12)));
        assert_eq!(nvidia_version(Some("616.56")), None);
        assert_eq!(
            parse_instance_id(r"PCI\VEN_10DE&DEV_2C09&SUBSYS_205E1043&REV_A1\4&1A2B3C4D&0&0008"),
            Some((0x10DE, 0x2C09, Some(0x205E1043)))
        );
        assert_eq!(parse_instance_id(r"ROOT\BasicDisplay\0000"), None);
        assert_eq!(
            parse_location(Some("PCI bus 10, device 0, function 1")).as_deref(),
            Some("0a:00.1")
        );
        assert_eq!(parse_location(None), None);
        assert_eq!(parse_location(Some("Bus PCI 12, dispositivo 0, función 0")), None); // translated text
        assert_eq!(pci_location(Some(12), Some(0)).as_deref(), Some("0c:00.0")); // SPDRP_BUSNUMBER/ADDRESS
        assert_eq!(pci_location(Some(0x41), Some(0x1F0007)).as_deref(), Some("41:1f.7"));
        for (bus, address) in [
            (None, Some(0)),
            (Some(1), None),
            (Some(256), Some(0)),
            (Some(1), Some(0x200000)),
            (Some(1), Some(8)),
        ] {
            assert_eq!(pci_location(bus, address), None);
        }
        assert_eq!(normalize_location("0A:1f.7").as_deref(), Some("0a:1f.7"));
    }

    #[test]
    fn python_edge_cases() {
        // Expected values from the Python functions (CPython 3.12).
        assert_eq!(nvidia_version(Some(" 32.0.16.1656 ")), Some((616, 56)));
        assert_eq!(nvidia_version(Some("32.0.16.56")), Some((600, 56)));
        assert_eq!(nvidia_version(Some("32.0.16.")), None);
        assert_eq!(nvidia_version(Some("32.0.١٦.١٦٥٦")), Some((616, 56)));
        assert_eq!(nvidia_version(Some("32.0.²16.1656")), Some((616, 56))); // outside the five digits
        assert_eq!(nvidia_version(Some("32.0.16.16²6")), None); // Python: ValueError
        assert_eq!(nvidia_version(None), None);
        assert_eq!(
            parse_location(Some("PCI bus 99999999999999999999999, device 1, function 2")).as_deref(),
            Some("152d02c7e14af67fffff:01.2")
        );
        assert_eq!(
            parse_location(Some("xx pcı BUſ ١, devİce 0, function 0 yy")).as_deref(),
            Some("01:00.0")
        );
        assert_eq!(
            parse_location(Some("PCI bus PCI bus 3, device 4, function 5")).as_deref(),
            Some("03:04.5")
        );
        assert_eq!(
            parse_location(Some(&format!("PCI bus {}, device 0, function 0", "9".repeat(4301)))),
            None
        );
        assert_eq!(normalize_location(" 0A:1f.7\u{3000}").as_deref(), Some("0a:1f.7"));
        assert_eq!(normalize_location("0000:0a:1f.7").as_deref(), Some("0a:1f.7"));
        assert_eq!(normalize_location("0:0a:1f.7").as_deref(), Some("0a:1f.7"));
        assert_eq!(normalize_location("\x1c0a:1f.7").as_deref(), Some("0a:1f.7"));
        for text in [
            "a:20.0",
            "01:00",
            "0001:01:00.0",
            "1",
            "01:00.8",
            "01:000.0",
            "00000:01:00.0",
            "01:00.0.0",
            "",
        ] {
            assert_eq!(normalize_location(text), None, "{text:?}");
        }
        assert_eq!(
            parse_instance_id("pcı\\ven_10de&dev_2c09&ſubſyſ_205e1043"),
            Some((0x10DE, 0x2C09, Some(0x205E1043)))
        );
        assert_eq!(
            parse_instance_id("PCI\\VEN_10DE&DEV_2C09&SUBSYS_205E104"),
            Some((0x10DE, 0x2C09, None))
        );
        assert_eq!(parse_instance_id("PCI\\VEN_10DE&DEV_2C0"), None);
        assert_eq!(parse_instance_id(" PCI\\VEN_10DE&DEV_2C09"), None);
    }

    #[test]
    fn utf16_replace_like_python() {
        let cases: &[(&[u8], &str)] = &[
            (b"\x00\xd8", "\u{fffd}"),
            (b"\x00\xd8\x41", "\u{fffd}"),
            (b"\x00\xd8\x41\x00", "\u{fffd}A"),
            (b"\x00\xdc\x41\x00", "\u{fffd}A"),
            (b"\x00\xd8\x00\xd8\x00\xdc", "\u{fffd}\u{10000}"),
            (b"\x41", "\u{fffd}"),
            (b"\x41\x00\x42", "A\u{fffd}"),
            (b"\x00\xd8\x00", "\u{fffd}"),
            (b"\x3d\xd8\x00\xde", "😀"),
            (b"\xff\xfe\x41\x00", "\u{feff}A"),
            (b"\x00\xdc\x00\xd8", "\u{fffd}\u{fffd}"),
            (b"\x00\xd8\x00\xdc\x00", "\u{10000}\u{fffd}"),
            (b"", ""),
        ];
        for (raw, text) in cases {
            assert_eq!(decode_utf16le(raw), *text, "{raw:02x?}");
        }
    }

    #[test]
    fn decode_and_encode() {
        assert_eq!(decode(REG_DWORD, &[0x12, 0, 0, 0]), Decoded::Dword(0x12));
        assert_eq!(decode(REG_DWORD, &[0x11, 0]), Decoded::Raw(REG_DWORD, vec![0x11, 0]));
        assert_eq!(decode(REG_SZ, &utf16("1\u{0}7\u{0}")), Decoded::Sz("1".into()));
        assert_eq!(decode(REG_SZ, b"\x41"), Decoded::Sz("\u{fffd}".into()));
        assert_eq!(decode(REG_EXPAND_SZ, b"x"), Decoded::Raw(REG_EXPAND_SZ, b"x".to_vec()));
        assert_eq!(encode(&Decoded::Dword(0x11)), (REG_DWORD, vec![0x11, 0, 0, 0]));
        assert_eq!(
            encode(&Decoded::Sz("hex(1):31,00".into())),
            (REG_SZ, utf16("hex(1):31,00\u{0}"))
        );
        assert_eq!(encode(&Decoded::Raw(0x12345, vec![0xff])), (0x12345, vec![0xff]));
        assert_eq!(type_name(REG_QWORD), "REG_QWORD");
        assert_eq!(type_name(0x12345), "type 74565");
    }

    #[test]
    fn never_panics_on_random_text_and_bytes() {
        use crate::testing::Rng;
        let mut rng = Rng::new(0xDEC0DE);
        for _ in 0..20000 {
            let size = rng.below(70);
            let text = rng.text(size);
            let _ = parse_instance_id(&text);
            let _ = parse_location(Some(&text));
            let _ = normalize_location(&text);
            let _ = nvidia_version(Some(&text));
            let digits: String = (0..rng.below(12))
                .map(|_| if rng.below(5) == 0 { rng.char() } else { '7' })
                .collect();
            let _ = nvidia_version(Some(&format!("32.0.{digits}.{digits}")));
            let _ = parse_location(Some(&format!("PCI bus {digits}, device {digits}, function {digits}")));
            let size = rng.below(40);
            let raw = rng.bytes(size);
            for kind in [
                REG_NONE,
                REG_SZ,
                REG_EXPAND_SZ,
                REG_BINARY,
                REG_DWORD,
                REG_MULTI_SZ,
                REG_QWORD,
                0x12345,
            ] {
                let value = decode(kind, &raw);
                let _ = encode(&value);
                let _ = value_text(kind, &raw);
            }
            let _ = pci_location(Some(rng.next_u64() as u32), Some(rng.next_u64() as u32));
        }
    }

    #[test]
    fn value_texts() {
        // The status texts of RawFirmwareTests and more, as devices.value_text gives them.
        let cases: Vec<(u32, Vec<u8>, &str)> = vec![
            (REG_SZ, utf16("17\u{0}"), "'17' (6 bytes)"),
            (REG_SZ, utf16("1\u{0}7\u{0}"), "'1\\x007' (8 bytes)"),
            (REG_MULTI_SZ, utf16("a\u{0}b\u{0}\u{0}"), "['a', 'b'] (10 bytes)"),
            (REG_MULTI_SZ, vec![], "[] (0 bytes)"),
            (REG_MULTI_SZ, vec![0, 0], "[] (2 bytes)"),
            (REG_MULTI_SZ, vec![0, 0, 0, 0], "[] (4 bytes)"),
            (REG_MULTI_SZ, vec![0; 6], "['', ''] (6 bytes)"),
            (REG_MULTI_SZ, utf16("a\u{0}\u{0}\u{0}"), "['a', ''] (8 bytes)"),
            (REG_EXPAND_SZ, vec![], "'' (0 bytes)"),
            (REG_NONE, vec![], "0 bytes"),
            (REG_BINARY, vec![0x11], "1 byte: 11"),
            (REG_DWORD, vec![0; 4], "0x00"),
            (REG_QWORD, vec![0x11, 0, 0, 0, 0, 0, 0, 0], "0x11"),
            (REG_QWORD, vec![0xff; 8], "0xffffffffffffffff"),
            (REG_DWORD, vec![1, 2, 3], "3 bytes: 01 02 03"),
            (REG_DWORD, vec![0x11, 0], "2 bytes: 11 00"),
            (REG_SZ, vec![0x41], "1 byte: 41"),
            (0x12345, vec![], "0 bytes"),
            (REG_SZ, utf16("\u{11}\u{0}"), "'\\x11' (4 bytes)"),
            (
                REG_SZ,
                utf16(&"x".repeat(70)),
                "'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx... (140 bytes)",
            ),
            (
                REG_SZ,
                utf16(&"'".repeat(70)),
                "\"''''''''''''''''''''''''''''''''''''''''''''''''''''''''... (140 bytes)",
            ),
            (
                REG_MULTI_SZ,
                utf16(&"ab\u{0}".repeat(30)),
                "['ab', 'ab', 'ab', 'ab', 'ab', 'ab', 'ab', 'ab', 'ab', 'a... (180 bytes)",
            ),
        ];
        for (kind, raw, text) in cases {
            assert_eq!(value_text(kind, &raw), text, "{kind} {raw:02x?}");
        }
        assert_eq!(
            value_text(REG_BINARY, &[0; 20]),
            format!("20 bytes: {}...", "00 ".repeat(16))
        );
    }
}
