"""status, load, unload and verify on a device platform (devices.py). Pure logic: no Windows calls of its own.

load stores the chain in the adapter's driver key, the same values the NVIDIA driver reads for
a VBIOS override: 'vbios' (REG_BINARY), 'EnableGpuFirmware' = 0x11, and no 'romOverrideXX'
records. The previous EnableGpuFirmware value (or its absence) is saved once in BACKUP so that
unload can restore it: 'absent', '0x%08x' for a REG_DWORD, or for any other type (or a DWORD of
another size) its exact type and bytes in .reg file notation, 'hex(<type in hex>):<bytes>', for
example 'hex(1):31,00,00,00'. unload also deletes 'romOverrideXX' records. Nothing else in the key is touched.
verify reads the image the driver is running (twice, through NvAPI) and compares it with 'vbios'.
"""
from __future__ import annotations
import hashlib
import re
from ..families import architecture
from ..payload import PayloadError
from .devices import (REG_BINARY, REG_DWORD, REG_SZ, decode, normalize_location, nvidia_version, type_name,
                      value_text)
from .image import extract_chain
from .nvapi import IMAGE_SIZE

VBIOS = 'vbios'
FIRMWARE = 'EnableGpuFirmware'
FIRMWARE_OVERRIDE = 0x11      # GSP firmware enabled (1) | fallback allowed (0x10); the driver default is 0x12
BACKUP = 'VixLoaderPreviousEnableGpuFirmware'
ABSENT = 'absent'
MAX_SAVED = 4096              # bytes of a non-DWORD EnableGpuFirmware that load saves exactly; larger is refused
RAW_BACKUP = re.compile(r'hex\((0|[1-9a-f][0-9a-f]{0,7})\):((?:[0-9a-f]{2}(?:,[0-9a-f]{2})*)?)')
PATCH_RECORDS = tuple(f'romOverride{i:02X}' for i in range(256))
MIN_DRIVER = (616, 56)
RESTART_HINT = 'Restart Windows to apply the change.'
VERIFY_OK = ('override-active', 'no-override')    # the driver runs what the registry says


class LoaderError(Exception):
    pass


def _dword(value):
    """The integer of a well-formed REG_DWORD value, else None."""
    return value[1] if value and value[0] == REG_DWORD and isinstance(value[1], int) else None


def _raw_backup(kind, raw):
    """The BACKUP text for a value that is not a well-formed REG_DWORD, as a .reg file writes it."""
    return f'hex({kind:x}):' + ','.join(f'{byte:02x}' for byte in raw)


def _saved(backup):
    """ABSENT, the DWORD saved in BACKUP, or the (type, bytes) of a saved non-DWORD value; None when malformed."""
    text = backup[1] if backup[0] == REG_SZ else None
    if text is None:
        return None
    if text == ABSENT:
        return ABSENT
    match = RAW_BACKUP.fullmatch(text)
    if match:
        raw = bytes.fromhex(match.group(2).replace(',', ''))
        return (int(match.group(1), 16), raw) if len(raw) <= MAX_SAVED else None
    try:
        value = int(text, 16)
    except ValueError:
        return None
    return value if 0 <= value <= 0xFFFFFFFF else None


def _backup_text(backup):
    """BACKUP for status: as stored for absent or a DWORD, readable for an exact copy."""
    saved = _saved(backup)
    if isinstance(saved, tuple):
        return f'{type_name(saved[0])} {value_text(*saved)}'
    return backup[1] if backup[0] == REG_SZ else 'unexpected ' + type_name(backup[0])


def select(adapters, gpu=None, pci=None, instance_id=None):
    """One adapter by status index, PCI location or device instance id; without any, the only NVIDIA adapter."""
    if sum(x is not None for x in (gpu, pci, instance_id)) > 1:
        raise LoaderError('use either --gpu or --pci, not both')
    if instance_id is not None:
        matches = [a for a in adapters if a.instance_id == instance_id]
    elif pci is not None:
        location = normalize_location(pci)
        if location is None:
            raise LoaderError(f'{pci!r} is not a PCI location such as 01:00.0')
        matches = [a for a in adapters if a.location == location]
    elif gpu is not None:
        matches = [a for a in adapters if a.index == gpu]
    else:
        if len(adapters) > 1:
            raise LoaderError(f'{len(adapters)} NVIDIA GPUs found; choose one with --gpu INDEX or --pci BUS:DEV.FN')
        matches = adapters
    if len(matches) != 1:
        raise LoaderError('no NVIDIA GPU matches that selection; run status to list them' if not matches else
                          'more than one GPU has that PCI location; use --gpu INDEX')
    return matches[0]


def describe(key):
    """The override-related values of one driver key."""
    vbios, firmware, backup = key.get(VBIOS), key.get_raw(FIRMWARE), key.get(BACKUP)
    stored = vbios[1] if vbios and vbios[0] == REG_BINARY else None
    dword = None if firmware is None else _dword(decode(*firmware))
    return {
        'vbios': None if vbios is None else {
            'type': type_name(vbios[0]),
            'size': len(stored) if stored is not None else None,
            'sha256': hashlib.sha256(stored).hexdigest() if stored is not None else None},
        'enable_gpu_firmware': None if firmware is None else
            (f'{dword:#04x}' if dword is not None else f'unexpected {type_name(firmware[0])} {value_text(*firmware)}'),
        'saved_previous_enable_gpu_firmware': None if backup is None else _backup_text(backup),
        'rom_override_records': [name for name in PATCH_RECORDS if key.get(name) is not None],
    }


def _adapter_info(adapter):
    version = nvidia_version(adapter.driver_version)
    family = architecture(adapter.device)['family'] if adapter.vendor == 0x10DE else 'unknown'
    return {'index': adapter.index, 'name': adapter.name, 'pci_id': f'{adapter.vendor:04x}:{adapter.device:04x}',
            'subsystem': None if adapter.subsystem is None else f'{adapter.subsystem:08x}',
            'location': adapter.location, 'instance_id': adapter.instance_id, 'family': family,
            'driver_version': adapter.driver_version,
            'nvidia_driver_version': None if version is None else f'{version[0]}.{version[1]:02d}',
            'driver_supported': version is not None and version >= MIN_DRIVER,
            'driver_key': adapter.driver_key}


def status(platform):
    report = []
    for adapter in platform.adapters():
        entry = _adapter_info(adapter)
        try:
            key = platform.open_key(adapter)
            try:
                entry['override'] = describe(key)
            finally:
                key.close()
        except OSError as exc:
            entry['override'], entry['error'] = None, f'cannot read the driver key: {exc}'
        report.append(entry)
    return report


def _step_json(step):
    op, name, *rest = step
    if op == 'delete':
        return {'op': 'delete', 'name': name}
    kind, value = rest
    if op == 'restore':        # exact type and bytes of a saved non-DWORD value
        return {'op': 'set', 'name': name, 'type': type_name(kind), 'value': value_text(kind, value),
                'data': value.hex()}
    entry = {'op': 'set', 'name': name, 'type': type_name(kind)}
    if kind == REG_BINARY:
        entry.update(size=len(value), sha256=hashlib.sha256(value).hexdigest())
    else:
        entry['value'] = f'{value:#04x}' if kind == REG_DWORD else value
    return entry


def _apply(key, steps):
    try:
        for op, name, *rest in steps:
            if op == 'set':
                key.set(name, *rest)
            elif op == 'restore':
                key.set_raw(name, *rest)
            else:
                key.delete(name)
        key.flush()
    except OSError as exc:
        raise LoaderError(f'{exc}; the registry may be partly changed: run status, then unload, '
                          'before restarting Windows') from None


def _restart(platform, adapter, restart):
    if not restart:
        return {'requested': False, 'ok': None, 'message': 'Device not restarted (--no-restart). ' + RESTART_HINT}
    code, output = platform.restart_device(adapter.instance_id)
    if code == 0:
        return {'requested': True, 'ok': True, 'message': 'Device restarted.'}
    detail = 'pnputil did not finish' if code is None else f'pnputil exited with code {code}'
    return {'requested': True, 'ok': False, 'exit_code': code, 'output': output,
            'message': f'The values are written, but the device restart failed ({detail}). {RESTART_HINT}'}


def _prepare(platform, gpu, pci, instance_id, dry_run):
    adapter = select(platform.adapters(), gpu, pci, instance_id)
    if not dry_run and not platform.is_admin():
        raise LoaderError('administrator rights are required; run this from an elevated PowerShell')
    return adapter


def load(platform, data, *, gpu=None, pci=None, instance_id=None, dry_run=False, restart=True):
    adapter = _prepare(platform, gpu, pci, instance_id, dry_run)
    version = nvidia_version(adapter.driver_version)
    if version is None:
        raise LoaderError(f'cannot read the NVIDIA driver version of GPU {adapter.index} ({adapter.driver_version!r})')
    if version < MIN_DRIVER:
        raise LoaderError(f'NVIDIA driver {version[0]}.{version[1]:02d} is too old; '
                          f'{MIN_DRIVER[0]}.{MIN_DRIVER[1]} or newer is required')
    try:
        chain = extract_chain(data, adapter.vendor, adapter.device)
    except PayloadError as exc:
        raise LoaderError(f'payload refused for GPU {adapter.index} ({adapter.vendor:04x}:{adapter.device:04x}): '
                          f'{exc}') from None
    key = platform.open_key(adapter, write=not dry_run)
    try:
        before = describe(key)
        firmware, backup = key.get_raw(FIRMWARE), key.get(BACKUP)
        steps, notes, extra = [], [], {}
        if backup is not None:
            if _saved(backup) is None:
                raise LoaderError(f'{BACKUP} holds an unexpected value; run unload first')
        else:
            dword = None if firmware is None else _dword(decode(*firmware))
            if firmware is None:
                backup = (REG_SZ, ABSENT)
            elif dword is not None:
                backup = (REG_SZ, f'{dword:#010x}')
            else:
                # Set by another tool or by hand: saved byte for byte, and unload writes it back the same way.
                kind, raw = firmware
                if len(raw) > MAX_SAVED:
                    raise LoaderError(f'{FIRMWARE} holds a {type_name(kind)} value of {len(raw)} bytes, too large to '
                                      f'save (at most {MAX_SAVED}); not changing it')
                backup = (REG_SZ, _raw_backup(kind, raw))
                extra['raw_backup'] = {'type': type_name(kind), 'size': len(raw), 'value': value_text(kind, raw),
                                       'backup': backup[1]}
                shown = f'{len(raw)}-byte REG_DWORD' if kind == REG_DWORD else type_name(kind)
                held, saved = (('holds', 'Load saves it exactly as it is and replaces it with') if dry_run else
                               ('held', 'It was saved exactly as it was and replaced by'))
                notes.append(f'{FIRMWARE} {held} a {shown} value (set by another tool or by hand). {saved} REG_DWORD '
                             f'{FIRMWARE_OVERRIDE:#04x}, the value the override needs; unload puts it back.')
            steps.append(('set', BACKUP, *backup))
            if before['vbios'] is not None or before['rom_override_records']:
                extra['replaces_other_override'] = True
                current = (before['enable_gpu_firmware'] or 'not set').removeprefix('unexpected ')
                notes.insert(0, 'An override written by another tool (for example mVolt+) is replaced and will not be '
                                f'restored; unload restores {FIRMWARE} to its current value ({current}).')
        steps += [('delete', name) for name in before['rom_override_records']]
        steps += [('set', VBIOS, REG_BINARY, chain.data), ('set', FIRMWARE, REG_DWORD, FIRMWARE_OVERRIDE)]
        report = {'gpu': _adapter_info(adapter), 'dry_run': dry_run, 'before': before,
                  'payload': {'family': chain.family, 'size': len(chain.data),
                              'sha256': hashlib.sha256(chain.data).hexdigest(), 'file_size': len(data),
                              'dropped_prefix': chain.start, 'dropped_trailing': len(data) - chain.end},
                  'steps': [_step_json(step) for step in steps], **extra}
        if notes:
            report['message'] = ' '.join(notes)
        if dry_run:
            return report
        _apply(key, steps)
        after = key.get(VBIOS), key.get(FIRMWARE), key.get(BACKUP)
        if after[0] != (REG_BINARY, chain.data) or after[1] != (REG_DWORD, FIRMWARE_OVERRIDE) or after[2] != backup \
                or any(key.get(name) is not None for name in PATCH_RECORDS):
            raise LoaderError('the registry values read back differ from what was written; run unload')
        report['after'] = describe(key)
    finally:
        key.close()
    report['restart'] = _restart(platform, adapter, restart)
    return report


def unload(platform, *, gpu=None, pci=None, instance_id=None, dry_run=False, restart=True):
    adapter = _prepare(platform, gpu, pci, instance_id, dry_run)
    key = platform.open_key(adapter, write=not dry_run)
    try:
        before = describe(key)
        backup = key.get(BACKUP)
        steps, message = [], None
        expected_firmware = key.get_raw(FIRMWARE)
        saved = None if backup is None else _saved(backup)
        if saved == ABSENT:
            steps.append(('delete', FIRMWARE))
            expected_firmware = None
        elif isinstance(saved, int):
            steps.append(('set', FIRMWARE, REG_DWORD, saved))
            expected_firmware = (REG_DWORD, saved.to_bytes(4, 'little'))
        elif saved is not None:
            steps.append(('restore', FIRMWARE, *saved))
            expected_firmware = saved
        elif backup is not None:
            message = f'{BACKUP} holds an unexpected value; {FIRMWARE} is left as it is.'
        else:
            message = f'{FIRMWARE} was not saved by this tool and is left as it is.'
        steps += [('delete', name) for name in before['rom_override_records']]
        if before['vbios'] is not None:
            steps.append(('delete', VBIOS))
        if backup is not None:
            steps.append(('delete', BACKUP))
        report = {'gpu': _adapter_info(adapter), 'dry_run': dry_run, 'before': before,
                  'steps': [_step_json(step) for step in steps]}
        if not steps:
            report['message'] = 'No override is stored for this GPU.'
            return report
        if message:
            report['message'] = message
        if dry_run:
            return report
        # The saved value is deleted only after EnableGpuFirmware reads back exactly as expected.
        _apply(key, [step for step in steps if step != ('delete', BACKUP)])
        if key.get_raw(FIRMWARE) != expected_firmware:
            raise LoaderError(f'{FIRMWARE} read back differs from what was expected'
                              + (f'; {BACKUP} is kept, run unload again' if backup is not None else ''))
        if backup is not None:
            _apply(key, [('delete', BACKUP)])
        if key.get(VBIOS) is not None or key.get(BACKUP) is not None \
                or any(key.get(name) is not None for name in PATCH_RECORDS):
            raise LoaderError('the registry values read back differ from what was expected')
        report['after'] = describe(key)
    finally:
        key.close()
    report['restart'] = _restart(platform, adapter, restart)
    return report


def _image(data):
    return {'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()}


def verify(platform, *, gpu=None, pci=None, instance_id=None):
    """Compare the VBIOS image the driver is running (read twice) with the override in the driver key. Read-only."""
    adapter = select(platform.adapters(), gpu, pci, instance_id)
    key = platform.open_key(adapter)
    try:
        stored, records = key.get(VBIOS), describe(key)['rom_override_records']
    finally:
        key.close()
    configured = stored[1] if stored and stored[0] == REG_BINARY else None
    running, again = platform.vbios_image(adapter), platform.vbios_image(adapter)
    report = {'gpu': _adapter_info(adapter), 'readback': dict(_image(running), truncated=len(running) == IMAGE_SIZE),
              'configured': None if configured is None else _image(configured), 'match': None, 'compared': None,
              'first_difference': None, 'rom_override_records': records}
    if running != again:
        state, message = 'unstable', 'Two reads of the running VBIOS differ; try again.'
        report['second_readback'] = _image(again)
    elif configured is None:
        # A removed override keeps running until the GPU restarts, and the readback cannot tell the two apart.
        state, message = 'no-override', ("No override is stored, so the driver should be running the card's own "
                                         'VBIOS. If one was just removed, restart the GPU or Windows first.')
        if records:
            message += " romOverride records are stored too; the driver may apply them to the card's VBIOS."
    else:
        # NvAPI cuts the image at IMAGE_SIZE bytes: a longer override is compared up to there.
        prefix = len(running) == IMAGE_SIZE < len(configured)
        expected = configured[:IMAGE_SIZE] if prefix else configured
        report.update(match=running == expected, compared='prefix' if prefix else 'full')
        if report['match'] and prefix:
            state = 'override-active'
            message = (f'The first {IMAGE_SIZE} bytes the driver is running match the stored override; NvAPI returns '
                       f'no more, so the last {len(configured) - IMAGE_SIZE} bytes cannot be read back.')
        elif report['match']:
            state, message = 'override-active', 'The driver is using the stored override (verified byte by byte).'
        else:
            offset = next((i for i, (a, b) in enumerate(zip(running, expected)) if a != b),
                          min(len(running), len(expected)))
            report['first_difference'] = offset
            state = 'override-not-active'
            message = (f'The driver is NOT using the stored override (first difference at offset {offset:#x}). If it '
                       'was just loaded, restart the GPU or Windows; otherwise the driver may have rejected it and is '
                       "running the card's own VBIOS.")
    report.update(state=state, message=message)
    return report
