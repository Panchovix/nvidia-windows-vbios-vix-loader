"""NVIDIA display adapters and their driver registry key, behind a small interface.

A platform object provides:

- adapters() -> list[Adapter]: present NVIDIA display adapters, in a stable order;
- open_key(adapter, write=False) -> key: the adapter's driver key
  (HKLM\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-...}\\NNNN), with
  get(name) -> (type, value) | None, set(name, type, value), delete(name) -> bool,
  flush() and close();
- is_admin() -> bool;
- restart_device(instance_id) -> (exit code or None, output text).

win32.py implements it with SetupAPI and the registry functions; tests use a fake.
Nothing in this module needs Windows.
"""
from __future__ import annotations
from dataclasses import dataclass
import re

REG_SZ, REG_BINARY, REG_DWORD = 1, 3, 4
TYPE_NAMES = {REG_SZ: 'REG_SZ', REG_BINARY: 'REG_BINARY', REG_DWORD: 'REG_DWORD'}


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


def parse_location(text: str | None):
    """'PCI bus 1, device 0, function 0' -> '01:00.0'; None when the text is not a PCI location."""
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
