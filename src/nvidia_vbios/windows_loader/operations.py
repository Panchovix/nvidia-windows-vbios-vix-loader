"""status, load, unload and verify on a device platform (devices.py). Pure logic: no Windows calls of its own.

load stores the chain in the adapter's driver key, the same values the NVIDIA driver reads for
a VBIOS override: 'vbios' (REG_BINARY), 'EnableGpuFirmware' = 0x11, and no 'romOverrideXX'
records. The previous EnableGpuFirmware value (or its absence) is saved once in BACKUP so that
unload can restore it. unload also deletes 'romOverrideXX' records. Nothing else in the key is touched.
verify reads the image the driver is running (twice, through NvAPI) and compares it with 'vbios'.
"""
from __future__ import annotations
import hashlib
from ..families import architecture
from ..payload import PayloadError
from .devices import REG_BINARY, REG_DWORD, REG_SZ, TYPE_NAMES, normalize_location, nvidia_version
from .image import extract_chain
from .nvapi import IMAGE_SIZE

VBIOS = 'vbios'
FIRMWARE = 'EnableGpuFirmware'
FIRMWARE_OVERRIDE = 0x11      # GSP firmware enabled (1) | fallback allowed (0x10); the driver default is 0x12
BACKUP = 'VixLoaderPreviousEnableGpuFirmware'
ABSENT = 'absent'
PATCH_RECORDS = tuple(f'romOverride{i:02X}' for i in range(256))
MIN_DRIVER = (616, 56)
RESTART_HINT = 'Restart Windows to apply the change.'
VERIFY_OK = ('override-active', 'no-override')    # the driver runs what the registry says


class LoaderError(Exception):
    pass


def _dword(value):
    """The integer of a well-formed REG_DWORD value, else None."""
    return value[1] if value and value[0] == REG_DWORD and isinstance(value[1], int) else None


def _saved(backup):
    """ABSENT or the integer saved in BACKUP; None when the value is malformed."""
    text = backup[1] if backup[0] == REG_SZ else None
    if text == ABSENT:
        return ABSENT
    try:
        return int(text, 16)
    except (TypeError, ValueError):
        return None


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
    vbios, firmware, backup = key.get(VBIOS), key.get(FIRMWARE), key.get(BACKUP)
    stored = vbios[1] if vbios and vbios[0] == REG_BINARY else None
    return {
        'vbios': None if vbios is None else {
            'type': TYPE_NAMES.get(vbios[0], str(vbios[0])),
            'size': len(stored) if stored is not None else None,
            'sha256': hashlib.sha256(stored).hexdigest() if stored is not None else None},
        'enable_gpu_firmware': None if firmware is None else
            (f'{_dword(firmware):#04x}' if _dword(firmware) is not None else
             'unexpected ' + TYPE_NAMES.get(firmware[0], str(firmware[0]))),
        'saved_previous_enable_gpu_firmware': None if backup is None else
            (backup[1] if backup[0] == REG_SZ else 'unexpected ' + TYPE_NAMES.get(backup[0], str(backup[0]))),
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
    entry = {'op': 'set', 'name': name, 'type': TYPE_NAMES[kind]}
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
        firmware, backup = key.get(FIRMWARE), key.get(BACKUP)
        steps, message = [], None
        if backup is not None:
            if _saved(backup) is None:
                raise LoaderError(f'{BACKUP} holds an unexpected value; run unload first')
        else:
            if firmware is not None and _dword(firmware) is None:
                raise LoaderError(f'{FIRMWARE} holds an unexpected value; not changing it')
            steps.append(('set', BACKUP, REG_SZ, ABSENT if firmware is None else f'{firmware[1]:#010x}'))
            if before['vbios'] is not None or before['rom_override_records']:
                message = (f'An override written by another tool (for example mVolt+) is replaced and will not be '
                           f'restored; unload restores {FIRMWARE} to its current value ({before["enable_gpu_firmware"] or "not set"}).')
        steps += [('delete', name) for name in before['rom_override_records']]
        steps += [('set', VBIOS, REG_BINARY, chain.data), ('set', FIRMWARE, REG_DWORD, FIRMWARE_OVERRIDE)]
        report = {'gpu': _adapter_info(adapter), 'dry_run': dry_run, 'before': before,
                  'payload': {'family': chain.family, 'size': len(chain.data),
                              'sha256': hashlib.sha256(chain.data).hexdigest(), 'file_size': len(data),
                              'dropped_prefix': chain.start, 'dropped_trailing': len(data) - chain.end},
                  'steps': [_step_json(step) for step in steps]}
        if message:
            report['message'] = message
        if dry_run:
            return report
        _apply(key, steps)
        after = key.get(VBIOS), key.get(FIRMWARE), key.get(BACKUP)
        if after[0] != (REG_BINARY, chain.data) or after[1] != (REG_DWORD, FIRMWARE_OVERRIDE) or after[2] is None \
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
        expected_firmware = key.get(FIRMWARE)
        saved = None if backup is None else _saved(backup)
        if saved == ABSENT:
            steps.append(('delete', FIRMWARE))
            expected_firmware = None
        elif saved is not None:
            steps.append(('set', FIRMWARE, REG_DWORD, saved))
            expected_firmware = (REG_DWORD, saved)
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
        _apply(key, steps)
        if key.get(VBIOS) is not None or key.get(BACKUP) is not None or key.get(FIRMWARE) != expected_firmware \
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
