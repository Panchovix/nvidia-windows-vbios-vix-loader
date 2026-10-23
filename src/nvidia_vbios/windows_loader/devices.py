"""NVIDIA display adapters and their driver registry key, behind a small interface.

A platform object provides:

- adapters() -> list[Adapter]: present NVIDIA display adapters, in a stable order;
- open_key(adapter, write=False) -> key: the adapter's driver key
  (HKLM\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-...}\\NNNN), with
  get(name) -> (type, value) | None, set(name, type, value), delete(name) -> bool,
  get_raw(name) -> (type, bytes) | None and set_raw(name, type, bytes) for a value's exact data,
  flush() and close(); get and set convert with decode and encode below;
- is_admin() -> bool;
- restart_device(instance_id) -> (exit code or None, output text);
- vbios_image(adapter) -> bytes: the VBIOS image the driver is running (nvapi.py).

win32.py implements it with SetupAPI and the registry functions; tests use a fake.
Nothing in this module needs Windows.
"""
from __future__ import annotations
from dataclasses import dataclass
import re

REG_NONE, REG_SZ, REG_EXPAND_SZ, REG_BINARY, REG_DWORD, REG_MULTI_SZ, REG_QWORD = 0, 1, 2, 3, 4, 7, 11
TYPE_NAMES = {REG_NONE: 'REG_NONE', REG_SZ: 'REG_SZ', REG_EXPAND_SZ: 'REG_EXPAND_SZ', REG_BINARY: 'REG_BINARY',
              REG_DWORD: 'REG_DWORD', REG_MULTI_SZ: 'REG_MULTI_SZ', REG_QWORD: 'REG_QWORD'}


def type_name(kind):
    return TYPE_NAMES.get(kind, f'type {kind}')


def decode(kind, raw):
    """(type, value) as get() returns it: an int for a 4-byte REG_DWORD, the text up to the first NUL for
    REG_SZ, else the bytes as stored."""
    if kind == REG_DWORD and len(raw) == 4:
        return REG_DWORD, int.from_bytes(raw, 'little')
    if kind == REG_SZ:
        return REG_SZ, raw.decode('utf-16-le', 'replace').split('\0', 1)[0]
    return kind, bytes(raw)


def encode(kind, value):
    """The bytes set() writes: 4 bytes for an int REG_DWORD, NUL-terminated UTF-16 for a REG_SZ text, else the bytes."""
    if kind == REG_DWORD and isinstance(value, int):
        return value.to_bytes(4, 'little')
    if kind == REG_SZ and isinstance(value, str):
        return (value + '\0').encode('utf-16-le')
    return bytes(value)


def value_text(kind, raw):
    """A short readable form of a value's exact data, for status and the planned steps."""
    size = f'{len(raw)} byte' + ('' if len(raw) == 1 else 's')
    if kind in (REG_DWORD, REG_QWORD) and len(raw) == (4 if kind == REG_DWORD else 8):
        return f"{int.from_bytes(raw, 'little'):#04x}"
    if kind in (REG_SZ, REG_EXPAND_SZ, REG_MULTI_SZ) and len(raw) % 2 == 0:
        text = raw.decode('utf-16-le', 'replace')
        if kind == REG_MULTI_SZ:
            text = text[:-2] if text.endswith('\0\0') else text[:-1] if text.endswith('\0') else text
            shown = repr(text.split('\0') if text else [])
        else:
            shown = repr(text[:-1] if text.endswith('\0') else text)
        return (shown if len(shown) <= 60 else shown[:57] + '...') + f' ({size})'
    return size + (': ' + raw[:16].hex(' ') + (' ...' if len(raw) > 16 else '') if raw else '')


@dataclass(frozen=True)
class Adapter:
    index: int
    name: str
    vendor: int
    device: int
    subsystem: int | None     # subsystem device/vendor dword, as in SUBSYS_xxxxxxxx
    location: str | None      # PCI bus:device.function, for example 01:00.0
    instance_id: str
    driver_version: str | None    # Windows form, for example 32.0.16.1656
    driver_key: str | None = None  # {class guid}\NNNN under ...\Control\Class


def parse_instance_id(instance_id: str):
    """(vendor, device, subsystem) from PCI\\VEN_10DE&DEV_2C09&SUBSYS_205E1043&REV_A1\\..., or None."""
    match = re.match(r'PCI\\VEN_([0-9A-F]{4})&DEV_([0-9A-F]{4})(?:&SUBSYS_([0-9A-F]{8}))?', instance_id, re.I)
    if not match:
        return None
    vendor, device, subsystem = match.groups()
    return int(vendor, 16), int(device, 16), int(subsystem, 16) if subsystem else None


def pci_location(bus: int | None, address: int | None):
    """SPDRP_BUSNUMBER and SPDRP_ADDRESS (device << 16 | function) -> '01:00.0'; None when missing or invalid.

    Unlike the location text, these numbers do not depend on the Windows display language.
    """
    if bus is None or address is None:
        return None
    device, function = address >> 16, address & 0xFFFF
    if not (0 <= bus <= 0xFF and device < 32 and function < 8):
        return None
    return f'{bus:02x}:{device:02x}.{function:x}'


def parse_location(text: str | None):
    """'PCI bus 1, device 0, function 0' -> '01:00.0'; None when the text is not an English PCI location."""
    match = re.search(r'PCI bus (\d+), device (\d+), function (\d+)', text or '', re.I)
    if not match:
        return None
    bus, device, function = map(int, match.groups())
    return f'{bus:02x}:{device:02x}.{function:x}'


def normalize_location(text: str):
    """User input 'BUS:DEV.FN' (hex, optional 0000: domain) -> '01:00.0', or None when malformed."""
    match = re.fullmatch(r'(?:([0-9a-f]{1,4}):)?([0-9a-f]{1,2}):([0-9a-f]{1,2})\.([0-7])', text.strip(), re.I)
    if not match or int(match.group(1) or '0', 16):
        return None
    bus, device, function = (int(part, 16) for part in match.groups()[1:])
    return f'{bus:02x}:{device:02x}.{function:x}' if device < 32 else None


def nvidia_version(windows_version: str | None):
    """Windows driver version 32.0.16.1656 -> (616, 56): the last five digits of the last two groups."""
    parts = (windows_version or '').strip().split('.')
    if len(parts) != 4 or not all(part.isdigit() for part in parts):
        return None
    digits = (parts[2] + parts[3].zfill(4))[-5:]
    return int(digits[:3]), int(digits[3:])
