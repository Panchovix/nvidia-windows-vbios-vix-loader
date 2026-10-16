"""The bytes stored in the 'vbios' registry value: the ROM image chain of one GPU.

The file must pass the same checks as the Linux loader (payload.py): exactly one NVIDIA
legacy image, for this GPU's PCI ID, with a zero checksum. The stored value starts at
that legacy image and ends at the end of the last image of its chain, following the
NPDE extended lengths and last-image flags. Bytes before the legacy image (a flash
prefix) and after the chain are not stored.
"""
from __future__ import annotations
from dataclasses import dataclass
import struct
from ..families import GA100_DEVICE_IDS
from ..payload import PayloadError, check_loadable_payload, check_payload, find_legacy


@dataclass(frozen=True)
class Chain:
    data: bytes      # what goes into the registry
    family: str
    start: int       # offset of the legacy image in the file
    end: int         # offset just past the last image in the file


def chain_end(data: bytes, start: int) -> int:
    """End offset of the image chain that starts at `start` (PCIR/NPDS images, NPDE lengths and last flags)."""
    at = start
    while True:
        if at + 0x1A > len(data):
            raise PayloadError(f'the image chain is truncated at {at:#x} (no image marked last)')
        signature = struct.unpack_from('<H', data, at)[0]
        if signature not in (0xAA55, 0x4E56):
            raise PayloadError(f'unexpected ROM signature {signature:#06x} at {at:#x}')
        header = at + struct.unpack_from('<H', data, at + 0x18)[0]
        if data[header:header + 4] != (b'PCIR' if signature == 0xAA55 else b'NPDS') or header + 0x18 > len(data):
            raise PayloadError(f'missing PCI data structure for the image at {at:#x}')
        blocks = struct.unpack_from('<H', data, header + 0x10)[0]
        last = bool(data[header + 0x15] & 0x80)
        npde = (header + struct.unpack_from('<H', data, header + 0x0A)[0] + 0xF) & ~0xF
        if data[npde:npde + 4] == b'NPDE' and npde + 0x0B <= len(data) and \
                struct.unpack_from('<H', data, npde + 4)[0] in (0x100, 0x101):
            blocks = struct.unpack_from('<H', data, npde + 8)[0]
            if struct.unpack_from('<H', data, npde + 6)[0] >= 0x0B:
                last = bool(data[npde + 0x0A] & 0x80)
        if not blocks or at + blocks * 512 > len(data):
            raise PayloadError(f'the image at {at:#x} has an invalid length')
        at += blocks * 512
        if last:
            return at


def extract_chain(data: bytes, vendor: int, device: int) -> Chain:
    """Validate a payload file for this GPU and return the chain to store, or raise PayloadError."""
    check_payload(data, vendor, device)
    if device in GA100_DEVICE_IDS:
        raise PayloadError('GA100 (A100/A30/A800) uses different GSP firmware and is not supported')
    start = find_legacy(data, vendor, device)
    end = chain_end(data, start)
    chain = data[start:end]
    # Same checks on the stored bytes: one legacy image, and the 512 KiB GA10x/AD10x capacity.
    family = check_loadable_payload(chain, vendor, device)
    return Chain(chain, family, start, end)
