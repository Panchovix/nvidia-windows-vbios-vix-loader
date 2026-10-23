"""vbios-loader: load a modified VBIOS on Windows through the NVIDIA driver's registry override.

Nothing is flashed: the image is stored in the GPU's driver registry key and used by the
driver when the device starts. unload removes it; verify checks that the driver is running it.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import sys
from . import __version__
from .nvapi import IMAGE_SIZE
from .operations import MIN_DRIVER, VERIFY_OK, LoaderError, load, status, unload, verify

PROG = 'vbios-loader'


def windows_platform():
    if os.name != 'nt':
        raise LoaderError('this tool runs on Windows only')
    from .win32 import Win32Platform
    return Win32Platform()


def parser():
    root = argparse.ArgumentParser(prog=PROG, description=__doc__.split('\n')[0])
    root.add_argument('--version', action='version', version=f'{PROG} {__version__}')
    commands = root.add_subparsers(dest='command', required=True)
    p = commands.add_parser('status', help='list NVIDIA GPUs and their override state')
    p.add_argument('--json', action='store_true', help='machine-readable output')
    for name, text in (('load', 'store a VBIOS override for one GPU and restart it'),
                       ('unload', 'remove the override and restore the previous firmware setting'),
                       ('verify', 'compare the VBIOS the driver is running with the stored override')):
        p = commands.add_parser(name, help=text)
        if name == 'load':
            p.add_argument('payload', type=Path, help='VBIOS file (see the README for which file to use)')
        p.add_argument('--gpu', type=int, metavar='INDEX', help='GPU index as shown by status')
        p.add_argument('--pci', metavar='BUS:DEV.FN', help='PCI location as shown by status, for example 01:00.0')
        if name != 'verify':
            p.add_argument('--dry-run', action='store_true', help='show the registry changes without making them')
            p.add_argument('--no-restart', action='store_true', help='do not restart the device afterwards')
        p.add_argument('--json', action='store_true', help='machine-readable output')
    return root


def _override_lines(override):
    vbios = override['vbios']
    lines = ['  override: ' + ('none' if vbios is None else
                               f"vbios {vbios['size']} bytes, sha256 {vbios['sha256']}" if vbios['size'] is not None
                               else f"vbios has an unexpected type ({vbios['type']})")]
    lines.append(f"  EnableGpuFirmware: {override['enable_gpu_firmware'] or 'not set'}"
                 + (f" (saved previous value: {override['saved_previous_enable_gpu_firmware']})"
                    if override['saved_previous_enable_gpu_firmware'] else ''))
    if override['rom_override_records']:
        lines.append(f"  romOverride records: {len(override['rom_override_records'])}")
    return lines


def _gpu_lines(gpu):
    subsystem = f" subsystem {gpu['subsystem']}" if gpu['subsystem'] else ''
    driver = gpu['nvidia_driver_version'] or 'unknown'
    gate = '' if gpu['driver_supported'] else f' (too old: {MIN_DRIVER[0]}.{MIN_DRIVER[1]} or newer is required)'
    return [f"GPU {gpu['index']}: {gpu['name']}",
            f"  PCI {gpu['location'] or 'unknown location'}, ID {gpu['pci_id']}{subsystem}, {gpu['family']}",
            f"  driver {driver} ({gpu['driver_version'] or 'unknown'}){gate}"] + \
        ([f"  driver key: {gpu['driver_key']}"] if gpu['driver_key'] else [])


def _human_status(report):
    if not report:
        return 'No NVIDIA GPU found.'
    lines = []
    for gpu in report:
        lines += _gpu_lines(gpu)
        lines += [f"  {gpu['error']}"] if gpu.get('error') else _override_lines(gpu['override'])
    return '\n'.join(lines)


def step_text(step):
    """One registry change of a report, for example 'set EnableGpuFirmware = REG_DWORD 0x11'."""
    detail = (f"{step['size']} bytes, sha256 {step['sha256']}" if 'size' in step else step.get('value', ''))
    if len(detail) > 100:      # a long saved value; --json keeps all of it
        detail = f'{detail[:80]}... ({len(detail)} characters)'
    return f"{step['op']} {step['name']}" + (f" = {step['type']} {detail}" if step['op'] == 'set' else '')


def _human_change(report):
    lines = _gpu_lines(report['gpu'])
    payload = report.get('payload')
    if payload:
        lines.append(f"  payload: {payload['family']}, {payload['size']} bytes stored "
                     f"(file {payload['file_size']} bytes; {payload['dropped_prefix']} leading and "
                     f"{payload['dropped_trailing']} trailing bytes not stored)")
    lines.append('Planned registry changes:' if report['dry_run'] else 'Registry changes:')
    lines += ['  ' + step_text(step) for step in report['steps']]
    if report.get('message'):
        lines.append(report['message'])
    if report['dry_run']:
        lines.append('Dry run: nothing was written.')
    elif report.get('restart'):
        lines.append(report['restart']['message'])
    return '\n'.join(lines)


def _image_text(image):
    return f"{image['size']} bytes, sha256 {image['sha256']}"


def _human_verify(report):
    readback, configured = report['readback'], report['configured']
    lines = _gpu_lines(report['gpu'])
    lines.append('  running VBIOS (NvAPI): ' + _image_text(readback)
                 + (f' (NvAPI returns at most {IMAGE_SIZE} bytes)' if readback['truncated'] else ''))
    if report.get('second_readback'):
        lines.append('  second read: ' + _image_text(report['second_readback']))
    lines.append('  stored override: ' + (_image_text(configured) if configured else 'none'))
    lines.append(report['message'])
    return '\n'.join(lines)


def main(argv=None, platform=None):
    args = parser().parse_args(argv)
    try:
        platform = platform or windows_platform()
        if args.command == 'status':
            report = status(platform)
            print(json.dumps(report, indent=2) if args.json else _human_status(report))
            return 0
        if args.command == 'verify':
            report = verify(platform, gpu=args.gpu, pci=args.pci)
            ok = report['state'] in VERIFY_OK
            print(json.dumps(dict(report, ok=ok), indent=2) if args.json else _human_verify(report))
            return 0 if ok else 4
        common = dict(gpu=args.gpu, pci=args.pci, dry_run=args.dry_run, restart=not args.no_restart)
        if args.command == 'load':
            try:
                data = args.payload.read_bytes()
            except OSError as exc:
                raise LoaderError(f'cannot read {args.payload}: {exc.strerror or exc}') from None
            report = load(platform, data, **common)
        else:
            report = unload(platform, **common)
    except (LoaderError, OSError) as exc:
        if args.json:
            print(json.dumps({'ok': False, 'error': str(exc)}, indent=2))
        print(f'{PROG}: {exc}', file=sys.stderr)
        return 1
    restart_failed = (report.get('restart') or {}).get('ok') is False
    if args.json:
        print(json.dumps(dict(report, ok=not restart_failed), indent=2))
    else:
        print(_human_change(report))
    return 3 if restart_failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
