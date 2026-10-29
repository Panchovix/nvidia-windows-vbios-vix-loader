"""Windows registry loader: chain extraction and registry operations on a fake device platform.

No Windows, GPU, registry or ROM corpus is needed.
"""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from nvidia_vbios.payload import GA10X_MAX_SIZE, PayloadError
from nvidia_vbios.windows_loader import cli, operations as ops
from nvidia_vbios.windows_loader.devices import (REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_NONE,
                                                 REG_QWORD, REG_SZ, Adapter, decode, encode, normalize_location,
                                                 nvidia_version, parse_instance_id, parse_location, pci_location,
                                                 type_name, value_text)
from nvidia_vbios.windows_loader.image import extract_chain

RTX_5070, RTX_5070_TI, RTX_4090, RTX_A2000, A100 = 0x2C09, 0x2C05, 0x2684, 0x2531, 0x20B5
RTX_2060_SUPER = 0x1F06
PREFIX, TRAILING = 0x400, 0x300


def image(device, *, code_type=0, blocks=2, last=False, npde_blocks=None, npde=True, pcir_last=None):
    """One ROM image: legacy/EFI (55AA + PCIR) or NVIDIA data (VN + NPDS), optionally with an NPDE header."""
    data = bytearray(b'\xff' * ((npde_blocks or blocks) * 512))
    data[0:2] = b'\x55\xaa' if code_type != 0xE0 else b'VN'
    data[0x18:0x1A] = (0x40).to_bytes(2, 'little')
    data[0x40:0x58] = bytes(0x18)
    data[0x40:0x44] = b'PCIR' if code_type != 0xE0 else b'NPDS'
    data[0x44:0x48] = (0x10DE | device << 16).to_bytes(4, 'little')
    data[0x4A:0x4C] = (0x18).to_bytes(2, 'little')
    data[0x50:0x52] = blocks.to_bytes(2, 'little')
    data[0x54] = code_type
    data[0x55] = 0x80 if (last and not npde if pcir_last is None else pcir_last) else 0
    if npde:
        data[0x60:0x6C] = b'NPDE' + (0x101).to_bytes(2, 'little') + (0x0C).to_bytes(2, 'little') + \
            (npde_blocks or blocks).to_bytes(2, 'little') + bytes([0x80 if last else 0, 0])
    if code_type == 0:
        data[blocks * 512 - 1] = 0
        data[blocks * 512 - 1] = -sum(data[:blocks * 512]) & 0xFF
    return bytes(data)


def chain(device, data_blocks=4, **options):
    return image(device, **options) + image(device, code_type=3, blocks=1) + \
        image(device, code_type=0xE0, blocks=data_blocks, last=True)


def rom(device, data_blocks=4):
    """A flash-like file: prefix, the chain, then trailing bytes."""
    return b'\xff' * PREFIX + chain(device, data_blocks) + b'\xff' * TRAILING


class FakeKey:
    """A driver key in memory. values holds (type, value) as get() returns it, or (type, bytes) for data that
    get() cannot show exactly (a REG_SZ with data after its first NUL, for example), as the registry stores it."""
    def __init__(self, values=None, drop_writes=False):
        self.values = dict(values or {})
        self.calls = []
        self.flushes = 0
        self.closed = 0
        self.drop_writes = drop_writes

    def get_raw(self, name):
        value = self.values.get(name)
        return None if value is None else (value[0], encode(*value))

    def get(self, name):
        value = self.get_raw(name)
        return None if value is None else decode(*value)

    def set(self, name, kind, value):
        self.set_raw(name, kind, encode(kind, value))

    def set_raw(self, name, kind, raw):
        self.calls.append(('set', name))
        if not self.drop_writes:
            value = decode(kind, raw)
            self.values[name] = value if encode(*value) == raw else (kind, bytes(raw))

    def delete(self, name):
        self.calls.append(('delete', name))
        return self.values.pop(name, None) is not None

    def flush(self):
        self.flushes += 1

    def close(self):
        self.closed += 1


class FakePlatform:
    def __init__(self, adapters=(), keys=None, admin=True, image=b''):
        self._adapters, self.keys, self.admin = list(adapters), keys or {}, admin
        self.writable_opens = 0
        self.image, self.reads = image, []      # what vbios_image returns, and the devices it was read from

    def adapters(self):
        return self._adapters

    def open_key(self, adapter, write=False):
        self.writable_opens += write
        return self.keys[adapter.instance_id]

    def is_admin(self):
        return self.admin

    def vbios_image(self, adapter):
        self.reads.append(adapter.instance_id)
        return self.image


def adapter(index=0, device=RTX_5070, location='01:00.0', version='32.0.16.1656'):
    return Adapter(index=index, name=f'GPU {index}', vendor=0x10DE, device=device, subsystem=0x205E1043,
                   location=location, instance_id=f'PCI\\VEN_10DE&DEV_{device:04X}&SUBSYS_205E1043&REV_A1\\4&{index}',
                   driver_version=version, driver_key=f'{{4d36e968-e325-11ce-bfc1-08002be10318}}\\000{index}')


UNRELATED = {'DriverDesc': (REG_SZ, 'NVIDIA GeForce RTX 5070'), 'UnrelatedDword': (REG_DWORD, 7),
             'UnrelatedBinary': (REG_BINARY, b'\x01\x02')}


def platform(device=RTX_5070, values=None, **options):
    gpu = adapter(device=device, version=options.pop('version', '32.0.16.1656'))
    key = FakeKey({**UNRELATED, **(values or {})})
    return FakePlatform([gpu], {gpu.instance_id: key}, **options), key


class ChainTests(unittest.TestCase):
    def test_prefix_and_trailing_bytes_are_dropped(self):
        data = rom(RTX_5070)
        result = extract_chain(data, 0x10DE, RTX_5070)
        self.assertEqual(result.data, chain(RTX_5070))
        self.assertEqual((result.start, result.end, result.family), (PREFIX, len(data) - TRAILING, 'blackwell-gb20x'))

    def test_npde_length_and_last_flag_win_over_pcir(self):
        # The legacy image's PCIR says 2 blocks; its NPDE extends it to 3. The PCIR last flag alone does not end the chain.
        data = image(RTX_5070, npde_blocks=3, pcir_last=True) + image(RTX_5070, code_type=0xE0, blocks=2, last=True) + b'\xff' * 512
        self.assertEqual(len(extract_chain(data, 0x10DE, RTX_5070).data), 5 * 512)
        # Without NPDE headers the PCIR lengths and last flag are used.
        data = image(RTX_5070, npde=False) + image(RTX_5070, code_type=0xE0, blocks=2, last=True, npde=False)
        self.assertEqual(extract_chain(data + bytes(512), 0x10DE, RTX_5070).data, data)

    def test_chain_without_a_last_image_or_with_two_banks_is_refused(self):
        unterminated = image(RTX_5070) + image(RTX_5070, code_type=0xE0, blocks=2)
        with self.assertRaisesRegex(PayloadError, 'truncated'):
            extract_chain(unterminated, 0x10DE, RTX_5070)
        with self.assertRaisesRegex(PayloadError, 'exactly one'):
            extract_chain(chain(RTX_5070) * 2, 0x10DE, RTX_5070)

    def test_wrong_device_ga100_and_non_nvidia_are_refused(self):
        with self.assertRaisesRegex(PayloadError, 'not 10de:2c05'):
            extract_chain(rom(RTX_5070), 0x10DE, RTX_5070_TI)
        with self.assertRaisesRegex(PayloadError, 'GA100'):
            extract_chain(rom(A100), 0x10DE, A100)
        with self.assertRaisesRegex(PayloadError, 'only NVIDIA'):
            extract_chain(rom(RTX_5070), 0x1002, RTX_5070)

    def test_ada_and_ampere_chains_are_limited_to_512_kib_blackwell_is_not(self):
        fixed = 2 * 512 + 512                       # legacy + EFI in front of the data image
        fits = (GA10X_MAX_SIZE - fixed) // 512
        for device, family in ((RTX_4090, 'ada'), (RTX_A2000, 'ampere')):
            with self.subTest(family=family):
                self.assertEqual(len(extract_chain(rom(device, fits), 0x10DE, device).data), GA10X_MAX_SIZE)
                with self.assertRaisesRegex(PayloadError, '512 KiB'):
                    extract_chain(rom(device, fits + 1), 0x10DE, device)
        self.assertEqual(len(extract_chain(rom(RTX_5070, 1800), 0x10DE, RTX_5070).data), fixed + 1800 * 512)


    def test_turing_chains_end_at_the_type_0x70_image_or_at_the_last_data_image(self):
        # Experimental: Turing ends its chain with a small type-0x70 image (NPDS flagged last, no NPDE).
        tail = bytearray(image(0, code_type=0xE0, blocks=1, last=True, npde=False))
        tail[0x54] = 0x70
        head = image(RTX_2060_SUPER) + image(RTX_2060_SUPER, code_type=3, blocks=1) + \
            image(RTX_2060_SUPER, code_type=0xE0, blocks=1)
        default = head + image(RTX_2060_SUPER, code_type=0xE0, blocks=4) + bytes(tail)
        result = extract_chain(b'\xff' * PREFIX + default + b'\xff' * TRAILING, 0x10DE, RTX_2060_SUPER)
        self.assertEqual((result.data, result.family), (default, 'turing'))
        # turing-e0-end-v1: the last data image carries both last-image flags and the tail is gone.
        e0_end = head + image(RTX_2060_SUPER, code_type=0xE0, blocks=4, last=True, pcir_last=True)
        self.assertEqual(extract_chain(e0_end + b'\xff' * TRAILING, 0x10DE, RTX_2060_SUPER).data, e0_end)
        fits = (GA10X_MAX_SIZE - 2 * 512 - 512) // 512
        with self.assertRaisesRegex(PayloadError, 'Turing TU10x/TU11x GSP accepts at most 512 KiB'):
            extract_chain(rom(RTX_2060_SUPER, fits + 1), 0x10DE, RTX_2060_SUPER)


class LoadUnloadTests(unittest.TestCase):
    def test_load_writes_the_override_and_removes_patch_records_only(self):
        records = {'romOverride00': (REG_BINARY, b'x'), 'romOverride7F': (REG_BINARY, b'y')}
        fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x12), **records})
        report = ops.load(fake, rom(RTX_5070))
        self.assertEqual(key.values[ops.VBIOS], (REG_BINARY, chain(RTX_5070)))
        self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x12))       # the driver's default stays
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, '0x00000012'))
        self.assertFalse(any(name.startswith('romOverride') for name in key.values))
        for name, value in UNRELATED.items():
            self.assertEqual(key.values[name], value)
        self.assertEqual({name for _, name in key.calls}, {ops.BACKUP, ops.VBIOS, 'romOverride00', 'romOverride7F'})
        self.assertEqual(key.flushes, 1)
        self.assertEqual(report['next_step'], ops.RESTART_HINT)
        self.assertEqual(report['payload']['dropped_prefix'], PREFIX)
        self.assertEqual(report['payload']['dropped_trailing'], TRAILING)

    def test_unload_restores_a_previous_value(self):
        fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x12)})
        ops.load(fake, rom(RTX_5070))
        self.assertEqual(ops.unload(fake)['next_step'], ops.RESTART_HINT)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x12)})

    def test_unload_deletes_a_value_that_was_absent(self):
        fake, key = platform()
        ops.load(fake, rom(RTX_5070))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, ops.ABSENT))
        ops.unload(fake)
        self.assertEqual(key.values, UNRELATED)

    def test_second_load_keeps_the_original_backup(self):
        fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x12)})
        ops.load(fake, rom(RTX_5070))
        ops.load(fake, rom(RTX_5070, data_blocks=6))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, '0x00000012'))
        self.assertEqual(key.values[ops.VBIOS], (REG_BINARY, chain(RTX_5070, 6)))
        ops.unload(fake)
        self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x12))

    def test_unload_without_our_backup_leaves_enable_gpu_firmware(self):
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'old'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        report = ops.unload(fake)
        self.assertNotIn(ops.VBIOS, key.values)
        self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x11))
        self.assertIn('left as it is', report['message'])
        fake, key = platform()
        report = ops.unload(fake)
        self.assertIn('No override', report['message'])
        self.assertNotIn('next_step', report)                  # nothing changed, nothing to apply
        self.assertEqual(key.calls, [])

    def test_dry_run_writes_nothing_and_needs_no_admin(self):
        for operation in (lambda p: ops.load(p, rom(RTX_5070), dry_run=True), lambda p: ops.unload(p, dry_run=True)):
            fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x12), ops.VBIOS: (REG_BINARY, b'old'),
                                         'romOverride01': (REG_BINARY, b'x')}, admin=False)
            report = operation(fake)
            self.assertTrue(report['dry_run'] and report['steps'])
            self.assertNotIn('next_step', report)
            self.assertEqual((key.calls, key.flushes, fake.writable_opens), ([], 0, 0))
        self.assertIn({'op': 'delete', 'name': 'romOverride01'},
                      ops.load(platform(admin=False, values={'romOverride01': (REG_BINARY, b'x')})[0],
                               rom(RTX_5070), dry_run=True)['steps'])

    def test_refusals_write_nothing(self):
        cases = [(dict(admin=False), {}, 'administrator'),
                 (dict(version='32.0.15.6094'), {}, '560.94 is too old'),
                 (dict(version='unknown'), {}, 'cannot read the NVIDIA driver version'),
                 (dict(device=RTX_5070_TI), {}, 'payload refused.*not 10de:2c05'),
                 (dict(device=RTX_4090), {}, 'payload refused'),
                 (dict(values={ops.FIRMWARE: (REG_BINARY, bytes(ops.MAX_SAVED + 1))}), {},
                  'EnableGpuFirmware holds a REG_BINARY value of 4097 bytes, too large to save \\(at most 4096\\); '
                  'not changing it'),
                 (dict(values={ops.BACKUP: (REG_DWORD, 0x12)}), {}, 'run unload first'),
                 (dict(values={ops.BACKUP: (REG_SZ, 'hex(1):3')}), {}, 'run unload first')]
        for options, _, message in cases:
            with self.subTest(message=message):
                fake, key = platform(**options)
                with self.assertRaisesRegex(ops.LoaderError, message):
                    ops.load(fake, rom(RTX_5070, 1100))
                self.assertEqual(key.calls, [])
        fake, key = platform(admin=False, values={ops.VBIOS: (REG_BINARY, b'old')})
        with self.assertRaisesRegex(ops.LoaderError, 'administrator'):
            ops.unload(fake)
        self.assertEqual(key.calls, [])

    def test_partial_write_failure_says_so_and_unload_recovers(self):
        class FailingKey(FakeKey):
            def set(self, name, kind, value):
                if name == ops.VBIOS:
                    raise OSError(5, 'writing vbios failed: Access is denied')
                super().set(name, kind, value)
        fake, key = platform()
        gpu = fake.adapters()[0]
        fake.keys[gpu.instance_id] = key = FailingKey(key.values)
        with self.assertRaisesRegex(ops.LoaderError, 'Access is denied; the registry may be partly changed.*unload'):
            ops.load(fake, rom(RTX_5070))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, ops.ABSENT))     # written before the failure
        ops.unload(fake)
        self.assertEqual(key.values, UNRELATED)

    def test_other_tools_override_is_replaced_and_its_records_removed_by_unload(self):
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        self.assertIn('another tool', ops.load(fake, rom(RTX_5070))['message'])
        self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x11))
        ops.unload(fake)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})
        fake, key = platform(values={'romOverride00': (REG_BINARY, b'r'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        report = ops.unload(fake)
        self.assertIn('left as it is', report['message'])
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})

    def test_malformed_values_are_reported_and_unload_still_removes_the_image(self):
        fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, b'\x11\x00')})
        self.assertEqual(ops.status(fake)[0]['override']['enable_gpu_firmware'], 'unexpected REG_DWORD 2 bytes: 11 00')
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'x'), ops.BACKUP: (REG_SZ, 'junk'),
                                     ops.FIRMWARE: (REG_DWORD, 0x11)})
        self.assertIn('unexpected value', ops.unload(fake)['message'])
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})

    def test_read_back_mismatch_is_an_error(self):
        fake, key = platform()
        key.drop_writes = True
        with self.assertRaisesRegex(ops.LoaderError, 'read back'):
            ops.load(fake, rom(RTX_5070))

        class KeepsFirmware(FakeKey):          # EnableGpuFirmware is not deleted when 0.3.x's 0x11 is undone
            def delete(self, name):
                return False if name == ops.FIRMWARE else super().delete(name)
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'old'), ops.FIRMWARE: (REG_DWORD, 0x11),
                                     ops.BACKUP: (REG_SZ, ops.ABSENT)})
        fake.keys[fake.adapters()[0].instance_id] = key = KeepsFirmware(key.values)
        with self.assertRaisesRegex(ops.LoaderError, 'read back differ from what was written; run unload'):
            ops.load(fake, rom(RTX_5070))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, ops.ABSENT))

        class DropsBackup(FakeKey):            # the saved value is not written
            def set_raw(self, name, kind, raw):
                if name != ops.BACKUP:
                    super().set_raw(name, kind, raw)

        class KeepsRecords(FakeKey):           # romOverride records are not deleted
            def delete(self, name):
                return False if name.startswith('romOverride') else super().delete(name)
        for key_class, values in ((DropsBackup, {}), (KeepsRecords, {'romOverride00': (REG_BINARY, b'r')})):
            with self.subTest(key=key_class.__name__):
                fake, key = platform(values=values)
                fake.keys[fake.adapters()[0].instance_id] = key_class(key.values)
                with self.assertRaisesRegex(ops.LoaderError, 'read back differ from what was written; run unload'):
                    ops.load(fake, rom(RTX_5070))

    def test_load_and_unload_ask_for_a_windows_restart_and_restart_nothing(self):
        import nvidia_vbios.windows_loader.win32 as win32  # importable without its DLLs
        self.assertFalse(hasattr(win32.Win32Platform, 'restart_device'))      # no pnputil
        self.assertEqual(ops.RESTART_HINT, 'Restart Windows to apply the change. Shutting down works too when Windows '
                                           'Fast Startup is off. With it on, the change waits for the next restart, '
                                           'and nothing has to be repeated.')
        fake, _ = platform()
        self.assertNotIn('next_step', ops.load(fake, rom(RTX_5070), dry_run=True))
        for report in (ops.load(fake, rom(RTX_5070)), ops.unload(fake, dry_run=True), ops.unload(fake)):
            self.assertNotIn('restart', report)
            self.assertEqual(report.get('next_step'), None if report['dry_run'] else ops.RESTART_HINT)


class DefaultModeTests(unittest.TestCase):
    """load leaves EnableGpuFirmware to the driver's default mode; it only undoes the 0x11 that loader 0.3.x wrote."""

    def test_a_first_load_leaves_enable_gpu_firmware_as_it_is(self):
        values = ((None, ops.ABSENT), ((REG_DWORD, 0x12), '0x00000012'),
                  ((REG_SZ, '1\0'.encode('utf-16-le')), 'hex(1):31,00,00,00'))
        for device in (RTX_4090, RTX_5070, RTX_A2000, RTX_2060_SUPER):
            for firmware, saved in values:
                with self.subTest(device=f'{device:04x}', saved=saved):
                    fake, key = platform(device, values=firmware and {ops.FIRMWARE: firmware})
                    before = key.get_raw(ops.FIRMWARE)
                    report = ops.load(fake, rom(device))
                    self.assertEqual(key.get_raw(ops.FIRMWARE), before)
                    self.assertNotIn(ops.FIRMWARE, {name for _, name in key.calls})
                    self.assertEqual(key.values[ops.BACKUP], (REG_SZ, saved))
                    self.assertEqual(key.values[ops.VBIOS], (REG_BINARY, chain(device)))
                    self.assertEqual([step['name'] for step in report['steps']], [ops.BACKUP, ops.VBIOS])
                    self.assertFalse({'message', 'restores_firmware', 'keeps_gsp_firmware_mode'} & set(report))
                    self.assertFalse(report['after']['gsp_forced_by_old_loader'])
                    self.assertEqual(report['next_step'], ops.RESTART_HINT)
                    ops.unload(fake)
                    self.assertEqual(key.get_raw(ops.FIRMWARE), before)
                    self.assertEqual(set(key.values), set(UNRELATED) | ({ops.FIRMWARE} if firmware else set()))

    def test_keys_left_by_0_3_2_get_the_saved_value_back(self):
        # vbios, EnableGpuFirmware = 0x11 and the value saved before it, as loader 0.3.2 left them.
        cases = ((ops.ABSENT, {'op': 'delete', 'name': ops.FIRMWARE}, None),
                 ('0x00000012', {'op': 'set', 'name': ops.FIRMWARE, 'type': 'REG_DWORD', 'value': '0x12'},
                  (REG_DWORD, b'\x12\0\0\0')),
                 ('hex(1):31,00,00,00', {'op': 'set', 'name': ops.FIRMWARE, 'type': 'REG_SZ',
                                         'value': "'1' (4 bytes)", 'data': '31000000'}, (REG_SZ, b'1\0\0\0')))
        for text, step, restored in cases:
            with self.subTest(saved=text):
                fake, key = platform(values={ops.VBIOS: (REG_BINARY, chain(RTX_5070)), ops.FIRMWARE: (REG_DWORD, 0x11),
                                             ops.BACKUP: (REG_SZ, text)}, admin=False)
                override = ops.status(fake)[0]['override']
                self.assertTrue(override['gsp_forced_by_old_loader'])
                shown = override['saved_previous_enable_gpu_firmware']
                report = ops.load(fake, rom(RTX_5070, data_blocks=6), dry_run=True)
                self.assertEqual(report['steps'], [{'op': 'set', 'name': ops.VBIOS, 'type': 'REG_BINARY',
                                                    'size': len(chain(RTX_5070, 6)),
                                                    'sha256': report['payload']['sha256']}, step])
                self.assertEqual(report['restores_firmware'], shown)
                self.assertEqual(report['message'], 'EnableGpuFirmware goes back to its value before the first load: '
                                                    f"{shown}. Loader 0.3.2 or older set it to 0x11, which turns on "
                                                    "the driver's GSP firmware mode; this version no longer turns "
                                                    'that mode on.')
                self.assertEqual(key.calls, [])
                fake.admin = True
                report = ops.load(fake, rom(RTX_5070, data_blocks=6))
                self.assertEqual(key.get_raw(ops.FIRMWARE), restored)
                self.assertEqual(key.values[ops.BACKUP], (REG_SZ, text))      # kept: unload restores it again
                self.assertEqual(key.values[ops.VBIOS], (REG_BINARY, chain(RTX_5070, 6)))
                self.assertFalse(report['after']['gsp_forced_by_old_loader'])
                self.assertEqual(report['next_step'], ops.RESTART_HINT)
                report = ops.load(fake, rom(RTX_5070))                         # nothing left to undo
                self.assertEqual([step['name'] for step in report['steps']], [ops.VBIOS])
                self.assertNotIn('message', report)
                ops.unload(fake)
                self.assertEqual(key.get_raw(ops.FIRMWARE), restored)
                self.assertEqual(set(key.values), set(UNRELATED) | ({ops.FIRMWARE} if restored else set()))

    def test_a_0x11_set_before_the_first_load_stays_with_a_note(self):
        note = ('GSP firmware mode stays on because EnableGpuFirmware was already 0x11 before the first load (for '
                "example set by mVolt+). For the driver's default mode, run unload (it puts 0x11 back), delete "
                'EnableGpuFirmware, load again and restart Windows.')
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        report = ops.load(fake, rom(RTX_5070))
        self.assertEqual((report['replaces_other_override'], report['keeps_gsp_firmware_mode']), (True, True))
        self.assertTrue(report['message'].startswith('An override written by another tool'))
        self.assertTrue(report['message'].endswith('(0x11). ' + note))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, '0x00000011'))
        self.assertEqual(key.get_raw(ops.FIRMWARE), (REG_DWORD, b'\x11\0\0\0'))
        self.assertFalse(report['after']['gsp_forced_by_old_loader'])
        for backup in ('0x00000011', 'hex(4):11,00,00,00'):        # also an exact copy of the same DWORD
            with self.subTest(backup=backup):
                key.values[ops.BACKUP] = (REG_SZ, backup)
                report = ops.load(fake, rom(RTX_5070, data_blocks=6))
                self.assertEqual((report['message'], report['keeps_gsp_firmware_mode']), (note, True))
                self.assertNotIn('restores_firmware', report)
                self.assertEqual([step['name'] for step in report['steps']], [ops.VBIOS])
                self.assertEqual(key.get_raw(ops.FIRMWARE), (REG_DWORD, b'\x11\0\0\0'))
        ops.unload(fake)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})
        # As the note says: deleted while no override is loaded, it is saved as absent and unload leaves it deleted.
        del key.values[ops.FIRMWARE]
        self.assertNotIn('message', ops.load(fake, rom(RTX_5070)))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, ops.ABSENT))
        ops.unload(fake)
        self.assertEqual(key.values, UNRELATED)


# EnableGpuFirmware values that are not a 4-byte REG_DWORD (set by another tool or by hand): load saves the
# exact type and bytes in .reg notation and leaves the value, and unload writes them back. (type, data, backup text
# or None)
EVERY_TYPE = [
    (REG_SZ, '17\0'.encode('utf-16-le'), 'hex(1):31,00,37,00,00,00'),
    (REG_SZ, '1\x007\0'.encode('utf-16-le'), 'hex(1):31,00,00,00,37,00,00,00'),     # data after a NUL
    (REG_SZ, b'', 'hex(1):'),                                                         # empty, no terminator
    (REG_SZ, b'\0\0', 'hex(1):00,00'),
    (REG_SZ, b'1', 'hex(1):31'),                                                      # odd size
    (REG_SZ, '17'.encode('utf-16-le'), 'hex(1):31,00,37,00'),                         # no terminator
    (REG_SZ, b'1\x007\x00\x00', 'hex(1):31,00,37,00,00'),                             # odd size, no terminator
    (REG_EXPAND_SZ, '%a%'.encode('utf-16-le'), 'hex(2):25,00,61,00,25,00'),
    (REG_EXPAND_SZ, '%SystemRoot%\0'.encode('utf-16-le'), None),
    (REG_MULTI_SZ, 'a\0bc\0\0'.encode('utf-16-le'), 'hex(7):61,00,00,00,62,00,63,00,00,00,00,00'),
    (REG_MULTI_SZ, 'a'.encode('utf-16-le'), 'hex(7):61,00'),
    (REG_QWORD, (0x11).to_bytes(8, 'little'), 'hex(b):11,00,00,00,00,00,00,00'),
    (REG_BINARY, b'\x11', 'hex(3):11'),
    (REG_NONE, b'', 'hex(0):'),
    (REG_NONE, b'\x11\0\0\0', 'hex(0):11,00,00,00'),
    (REG_DWORD, b'\x11\0', 'hex(4):11,00'),
    (REG_DWORD, (0x12).to_bytes(8, 'little'), 'hex(4):12,00,00,00,00,00,00,00'),
    (0x12345, b'\xff', 'hex(12345):ff'),
    (REG_BINARY, bytes(range(256)) * 16, None),                                       # the largest saved
]
# BACKUP texts load refuses ("run unload first") and unload does not restore from.
MALFORMED_BACKUPS = ['junk', '', ' absent', 'hex(1):3', 'hex(1):31,0', 'hex(1)31,00', 'hex(01):31', 'hex(1):31,,00',
                     'hex(1):31,00,', 'HEX(1):31', 'hex(1):3g', 'hex(1):3G', 'hex(100000000):', 'hex():31',
                     'hex(1): 31', 'hex(1):3100', 'hex(-1):31', '-1', '0x100000000',
                     'hex(3):' + ','.join(['00'] * (ops.MAX_SAVED + 1))]


class RawFirmwareTests(unittest.TestCase):
    def test_every_type_is_saved_exactly_and_unload_puts_it_back(self):
        for kind, raw, text in EVERY_TYPE:
            with self.subTest(kind=kind, raw=raw[:16]):
                fake, key = platform(values={ops.FIRMWARE: (kind, raw)})
                self.assertEqual(key.get_raw(ops.FIRMWARE), (kind, raw))
                report = ops.load(fake, rom(RTX_5070))
                backup = 'hex(%x):' % kind + ','.join(f'{byte:02x}' for byte in raw)
                self.assertEqual(backup, text or backup)
                self.assertEqual(key.values[ops.BACKUP], (REG_SZ, backup))
                self.assertEqual(key.get_raw(ops.FIRMWARE), (kind, raw))         # left as it is
                self.assertEqual(report['raw_backup'], {'type': type_name(kind), 'size': len(raw),
                                                        'value': value_text(kind, raw), 'backup': backup})
                self.assertNotIn('message', report)
                self.assertNotIn('replaces_other_override', report)
                self.assertEqual(report['after']['saved_previous_enable_gpu_firmware'],
                                 f'{type_name(kind)} {value_text(kind, raw)}')
                ops.load(fake, rom(RTX_5070, data_blocks=6))           # a second load keeps the saved value
                self.assertEqual(key.values[ops.BACKUP], (REG_SZ, backup))
                report = ops.unload(fake)
                self.assertEqual(key.get_raw(ops.FIRMWARE), (kind, raw))
                self.assertEqual(set(key.values), {*UNRELATED, ops.FIRMWARE})
                self.assertEqual({name: key.values[name] for name in UNRELATED}, UNRELATED)
                self.assertNotIn('message', report)

    def test_status_shows_the_type_and_the_saved_value(self):
        qword = (0x11).to_bytes(8, 'little')
        fake, key = platform(values={ops.FIRMWARE: (REG_QWORD, qword)})
        self.assertEqual(ops.status(fake)[0]['override']['enable_gpu_firmware'], 'unexpected REG_QWORD 0x11')
        ops.load(fake, rom(RTX_5070))
        override = ops.status(fake)[0]['override']
        self.assertEqual((override['enable_gpu_firmware'], override['saved_previous_enable_gpu_firmware']),
                         ('unexpected REG_QWORD 0x11', 'REG_QWORD 0x11'))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            cli.main(['status'], platform=fake)
        self.assertIn('EnableGpuFirmware: unexpected REG_QWORD 0x11 (saved previous value: REG_QWORD 0x11)',
                      out.getvalue())
        for kind, raw, shown in ((REG_SZ, '17\0'.encode('utf-16-le'), "unexpected REG_SZ '17' (6 bytes)"),
                                 (REG_SZ, '1\x007\0'.encode('utf-16-le'), "unexpected REG_SZ '1\\x007' (8 bytes)"),
                                 (REG_MULTI_SZ, 'a\0b\0\0'.encode('utf-16-le'),
                                  "unexpected REG_MULTI_SZ ['a', 'b'] (10 bytes)"),
                                 (REG_EXPAND_SZ, b'', "unexpected REG_EXPAND_SZ '' (0 bytes)"),
                                 (REG_NONE, b'', 'unexpected REG_NONE 0 bytes'),
                                 (REG_BINARY, b'\x11', 'unexpected REG_BINARY 1 byte: 11'),
                                 (REG_BINARY, bytes(20), 'unexpected REG_BINARY 20 bytes: ' + '00 ' * 16 + '...'),
                                 (0x12345, b'', 'unexpected type 74565 0 bytes')):
            fake, _ = platform(values={ops.FIRMWARE: (kind, raw)})
            self.assertEqual(ops.status(fake)[0]['override']['enable_gpu_firmware'], shown)

    def test_backups_written_by_0_3_1_still_unload(self):
        for text, expected in (('0x00000012', (REG_DWORD, b'\x12\0\0\0')), ('0x00000000', (REG_DWORD, bytes(4))),
                               ('0xffffffff', (REG_DWORD, b'\xff' * 4)), (ops.ABSENT, None)):
            with self.subTest(text=text):
                fake, key = platform(values={ops.VBIOS: (REG_BINARY, chain(RTX_5070)), ops.BACKUP: (REG_SZ, text),
                                             ops.FIRMWARE: (REG_DWORD, 0x11)})
                ops.load(fake, rom(RTX_5070, data_blocks=6))           # accepted, and kept as it is
                self.assertEqual(key.values[ops.BACKUP], (REG_SZ, text))
                self.assertEqual(key.get_raw(ops.FIRMWARE), expected)  # 0.3.x's 0x11 is undone already
                step = {'op': 'delete', 'name': ops.FIRMWARE} if expected is None else \
                    {'op': 'set', 'name': ops.FIRMWARE, 'type': 'REG_DWORD', 'value': f'{int(text, 16):#04x}'}
                self.assertEqual(ops.unload(fake, dry_run=True)['steps'][0], step)
                ops.unload(fake)
                self.assertEqual(key.get_raw(ops.FIRMWARE), expected)
                self.assertEqual(set(key.values), set(UNRELATED) | ({ops.FIRMWARE} if expected else set()))

    def test_malformed_backups_are_refused_and_not_restored(self):
        for text in MALFORMED_BACKUPS:
            with self.subTest(text=text[:20]):
                fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'x'), ops.BACKUP: (REG_SZ, text),
                                             ops.FIRMWARE: (REG_DWORD, 0x11)})
                with self.assertRaisesRegex(ops.LoaderError, 'holds an unexpected value; run unload first'):
                    ops.load(fake, rom(RTX_5070))
                self.assertEqual(key.calls, [])
                self.assertIn('unexpected value; EnableGpuFirmware is left as it is', ops.unload(fake)['message'])
                self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})

    def test_dry_runs_show_the_steps(self):
        qword = (0x11).to_bytes(8, 'little')
        fake, key = platform(values={ops.FIRMWARE: (REG_QWORD, qword)}, admin=False)
        report = ops.load(fake, rom(RTX_5070), dry_run=True)
        self.assertEqual(report['steps'][0], {'op': 'set', 'name': ops.BACKUP, 'type': 'REG_SZ',
                                              'value': 'hex(b):11,00,00,00,00,00,00,00'})
        self.assertEqual([step['name'] for step in report['steps']], [ops.BACKUP, ops.VBIOS])
        self.assertNotIn('message', report)
        self.assertEqual(key.calls, [])
        out = io.StringIO()
        with contextlib.redirect_stdout(out), tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'edited.rom'
            path.write_bytes(rom(RTX_5070))
            self.assertEqual(cli.main(['load', str(path), '--dry-run'], platform=fake), 0)
        self.assertIn('  set VixLoaderPreviousEnableGpuFirmware = REG_SZ hex(b):11,00,00,00,00,00,00,00\n'
                      '  set vbios = REG_BINARY ', out.getvalue())
        self.assertNotIn('set EnableGpuFirmware', out.getvalue())
        self.assertTrue(out.getvalue().endswith('\nDry run: nothing was written.\n'))

        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'x'), ops.FIRMWARE: (REG_DWORD, 0x11),
                                     ops.BACKUP: (REG_SZ, 'hex(1):31,00,00,00,37,00')}, admin=False)
        report = ops.unload(fake, dry_run=True)
        self.assertEqual(report['steps'], [{'op': 'set', 'name': ops.FIRMWARE, 'type': 'REG_SZ',
                                            'value': "'1\\x007' (6 bytes)", 'data': '310000003700'},
                                           {'op': 'delete', 'name': ops.VBIOS}, {'op': 'delete', 'name': ops.BACKUP}])
        self.assertEqual(cli.step_text(report['steps'][0]), "set EnableGpuFirmware = REG_SZ '1\\x007' (6 bytes)")
        self.assertEqual(key.calls, [])

        # A long saved value is shortened in the step lines; the JSON steps keep all of it.
        fake, key = platform(values={ops.FIRMWARE: (REG_BINARY, bytes(ops.MAX_SAVED))}, admin=False)
        step = ops.load(fake, rom(RTX_5070), dry_run=True)['steps'][0]
        self.assertEqual(step['value'], 'hex(3):' + ','.join(['00'] * ops.MAX_SAVED))
        self.assertEqual(cli.step_text(step), 'set VixLoaderPreviousEnableGpuFirmware = REG_SZ hex(3):'
                         + '00,' * 24 + '0... (12294 characters)')

    def test_another_tools_override_is_noted_and_a_non_dword_value_kept(self):
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_SZ, b'\x11\0')})
        report = ops.load(fake, rom(RTX_5070))
        self.assertTrue(report['replaces_other_override'])
        self.assertEqual(report['message'], 'An override written by another tool (for example mVolt+) is replaced and '
                                            'will not be restored; unload restores EnableGpuFirmware to its current '
                                            "value (REG_SZ '\\x11' (2 bytes)).")
        self.assertEqual(key.get_raw(ops.FIRMWARE), (REG_SZ, b'\x11\0'))
        ops.unload(fake)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_SZ, b'\x11\0')})

    def test_a_restore_that_reads_back_differently_keeps_the_saved_value(self):
        class AlteringKey(FakeKey):
            def set_raw(self, name, kind, raw):
                super().set_raw(name, kind, raw + b'\0\0' if name == ops.FIRMWARE and kind != REG_DWORD else raw)
        fake, key = platform(values={ops.FIRMWARE: (REG_SZ, b'1\0')})
        ops.load(fake, rom(RTX_5070))
        gpu = fake.adapters()[0]
        fake.keys[gpu.instance_id] = altering = AlteringKey(key.values)
        with self.assertRaisesRegex(ops.LoaderError, 'EnableGpuFirmware read back differs from what was expected; '
                                                     'VixLoaderPreviousEnableGpuFirmware is kept, run unload again'):
            ops.unload(fake)
        self.assertEqual(altering.values[ops.BACKUP], (REG_SZ, 'hex(1):31,00'))
        self.assertNotIn(ops.VBIOS, altering.values)
        fake.keys[gpu.instance_id] = key = FakeKey(altering.values)
        ops.unload(fake)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_SZ, b'1\0')})

    def test_win32_key_reads_and_writes_exact_bytes(self):
        import ctypes
        import nvidia_vbios.windows_loader.win32 as win32  # importable without its DLLs

        class Advapi32:     # RegQueryValueExW and RegSetValueExW on a dict, as the registry keeps the data
            def __init__(self):
                self.values = {}

            def RegQueryValueExW(self, handle, name, reserved, kind, buffer, size):
                if name not in self.values:
                    return win32.ERROR_FILE_NOT_FOUND
                stored_kind, data = self.values[name]
                room, kind._obj.value, size._obj.value = size._obj.value, stored_kind, len(data)
                if len(data) > room:
                    return win32.ERROR_MORE_DATA
                ctypes.memmove(buffer, data, len(data))
                return 0

            def RegSetValueExW(self, handle, name, reserved, kind, data, size):
                # As Windows does: string data whose last character is not NUL gets one more character stored
                # when the two bytes after it are zero (memory past the caller's buffer counts as zero here).
                memory, end = bytes(data) + b'\0\0', size // 2 * 2
                if kind in (REG_SZ, REG_EXPAND_SZ, REG_MULTI_SZ) and size >= 2 \
                        and memory[end - 2:end] != b'\0\0' and memory[end:end + 2] == b'\0\0':
                    size += 2
                self.values[name] = (kind, memory[:size])
                return 0
        api = Advapi32()
        key = win32.RegistryKey(api, 1)
        self.assertIsNone(key.get_raw('missing'))
        for kind, raw, _ in EVERY_TYPE:
            with self.subTest(kind=kind, raw=raw[:16]):
                key.set_raw(ops.FIRMWARE, kind, raw)
                self.assertEqual(api.values[ops.FIRMWARE], (kind, raw))
                self.assertEqual(key.get_raw(ops.FIRMWARE), (kind, raw))
                self.assertEqual(key.get(ops.FIRMWARE), decode(kind, raw))
        key.set(ops.FIRMWARE, REG_DWORD, 0x11)
        key.set(ops.BACKUP, REG_SZ, 'hex(1):31,00')
        self.assertEqual(api.values, {ops.FIRMWARE: (REG_DWORD, b'\x11\0\0\0'),
                                      ops.BACKUP: (REG_SZ, 'hex(1):31,00\0'.encode('utf-16-le'))})
        self.assertEqual((key.get(ops.FIRMWARE), key.get(ops.BACKUP)), ((REG_DWORD, 0x11), (REG_SZ, 'hex(1):31,00')))


class SelectionTests(unittest.TestCase):
    def test_selection_by_index_or_location_and_ambiguity(self):
        gpus = [adapter(0, location='01:00.0'), adapter(1, location='02:00.0')]
        self.assertEqual(ops.select(gpus, gpu=1).index, 1)
        self.assertEqual(ops.select(gpus, pci='0000:02:00.0').index, 1)
        self.assertEqual(ops.select(gpus, pci='2:0.0').index, 1)
        self.assertEqual(ops.select(gpus[:1]).index, 0)
        for options, message in ((dict(), '2 NVIDIA GPUs'), (dict(gpu=5), 'no NVIDIA GPU matches'),
                                 (dict(gpu=0, pci='01:00.0'), 'not both'), (dict(pci='01:00'), 'not a PCI location'),
                                 (dict(pci='0001:01:00.0'), 'not a PCI location')):
            with self.subTest(options=options), self.assertRaisesRegex(ops.LoaderError, message):
                ops.select(gpus, **options)
        with self.assertRaisesRegex(ops.LoaderError, 'more than one'):
            ops.select([adapter(0), adapter(1)], pci='01:00.0')

    def test_windows_strings(self):
        self.assertEqual(nvidia_version('32.0.16.1656'), (616, 56))
        self.assertEqual(nvidia_version('31.0.15.5222'), (552, 22))
        self.assertEqual(nvidia_version('32.0.16.612'), (606, 12))
        self.assertIsNone(nvidia_version('616.56'))
        self.assertEqual(parse_instance_id(r'PCI\VEN_10DE&DEV_2C09&SUBSYS_205E1043&REV_A1\4&1A2B3C4D&0&0008'),
                         (0x10DE, 0x2C09, 0x205E1043))
        self.assertIsNone(parse_instance_id(r'ROOT\BasicDisplay\0000'))
        self.assertEqual(parse_location('PCI bus 10, device 0, function 1'), '0a:00.1')
        self.assertIsNone(parse_location(None))
        self.assertIsNone(parse_location('Bus PCI 12, dispositivo 0, función 0'))     # translated text
        self.assertEqual(pci_location(12, 0), '0c:00.0')                              # SPDRP_BUSNUMBER/ADDRESS
        self.assertEqual(pci_location(0x41, 0x1F0007), '41:1f.7')
        for bus, address in ((None, 0), (1, None), (256, 0), (1, 0x200000), (1, 8)):
            self.assertIsNone(pci_location(bus, address))
        self.assertEqual(normalize_location('0A:1f.7'), '0a:1f.7')


class CliTests(unittest.TestCase):
    def run_cli(self, argv, fake):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = cli.main(argv, platform=fake)
        return code, out.getvalue(), err.getvalue()

    def test_status_json_without_gpus_is_an_empty_list(self):
        self.assertEqual(self.run_cli(['status', '--json'], FakePlatform())[:2], (0, '[]\n'))
        self.assertEqual(self.run_cli(['status'], FakePlatform())[:2], (0, 'No NVIDIA GPU found.\n'))

    def test_status_reports_the_stored_override(self):
        fake, key = platform(values={'romOverride02': (REG_BINARY, b'x')})
        ops.load(fake, rom(RTX_5070))
        key.values['romOverride02'] = (REG_BINARY, b'x')
        code, out, _ = self.run_cli(['status', '--json'], fake)
        entry = json.loads(out)[0]
        self.assertEqual(code, 0)
        self.assertEqual((entry['location'], entry['pci_id'], entry['nvidia_driver_version']), ('01:00.0', '10de:2c09', '616.56'))
        self.assertEqual(entry['override']['vbios']['size'], len(chain(RTX_5070)))
        self.assertIsNone(entry['override']['enable_gpu_firmware'])
        self.assertEqual(entry['override']['saved_previous_enable_gpu_firmware'], 'absent')
        self.assertIs(entry['override']['gsp_forced_by_old_loader'], False)
        self.assertEqual(entry['override']['rom_override_records'], ['romOverride02'])
        out = self.run_cli(['status'], fake)[1]
        self.assertIn('  EnableGpuFirmware: not set (saved previous value: absent)\n', out)
        self.assertIn('romOverride records: 1', out)
        self.assertNotIn('GSP firmware mode', out)

    def test_status_flags_gpus_left_in_gsp_firmware_mode_by_0_3_x(self):
        line = ('  GSP firmware mode: on, set by loader 0.3.2 or older; load the file again with this version (or '
                'unload it) and restart Windows.')
        old = {ops.VBIOS: (REG_BINARY, b'x'), ops.FIRMWARE: (REG_DWORD, 0x11)}
        cases = (({**old, ops.BACKUP: (REG_SZ, ops.ABSENT)}, True),
                 ({**old, ops.BACKUP: (REG_SZ, 'hex(b):12,00,00,00,00,00,00,00')}, True),
                 ({**old, ops.BACKUP: (REG_SZ, '0x00000011')}, False),     # 0x11 before the first load
                 ({**old, ops.BACKUP: (REG_SZ, 'junk')}, False),           # unload leaves EnableGpuFirmware
                 (old, False),                                             # mVolt+, no saved value
                 ({**old, ops.BACKUP: (REG_SZ, ops.ABSENT), ops.FIRMWARE: (REG_DWORD, 0x12)}, False),
                 ({**old, ops.BACKUP: (REG_SZ, ops.ABSENT), ops.FIRMWARE: (REG_DWORD, b'\x11\0')}, False))
        for values, flagged in cases:
            with self.subTest(values=values):
                fake, _ = platform(values=values)
                code, out, _ = self.run_cli(['status', '--json'], fake)
                self.assertEqual(code, 0)
                self.assertIs(json.loads(out)[0]['override']['gsp_forced_by_old_loader'], flagged)
                lines = self.run_cli(['status'], fake)[1].splitlines()
                self.assertEqual(line in lines, flagged)
                if flagged:     # right after the EnableGpuFirmware line
                    self.assertTrue(lines[lines.index(line) - 1].startswith('  EnableGpuFirmware: 0x11 (saved '))

    def test_load_and_unload_commands(self):
        fake, key = platform()
        hint = ('\nRestart Windows to apply the change. Shutting down works too when Windows Fast Startup is off. '
                'With it on, the change waits for the next restart, and nothing has to be repeated.\n')
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'edited.rom'
            path.write_bytes(rom(RTX_5070))
            code, out, _ = self.run_cli(['load', str(path), '--pci', '01:00.0', '--dry-run'], fake)
            self.assertEqual(code, 0)
            self.assertTrue(out.endswith('\nDry run: nothing was written.\n'))
            self.assertNotIn('Restart', out)
            code, out, _ = self.run_cli(['load', str(path), '--gpu', '0', '--json'], fake)
            report = json.loads(out)
            self.assertEqual((code, report['ok'], report['next_step']), (0, True, ops.RESTART_HINT))
            self.assertNotIn('restart', report)
            code, out, _ = self.run_cli(['load', str(path)], fake)
            self.assertEqual(code, 0)
            self.assertTrue(out.endswith(hint))
            code, out, err = self.run_cli(['load', str(Path(directory) / 'missing.rom')], fake)
            self.assertEqual(code, 1)
            self.assertIn('cannot read', err)
            # --no-restart is still accepted, so older scripts and instructions keep working; it changes nothing.
            reports = []
            for extra in ([], ['--no-restart']):
                other, _ = platform()
                code, out, _ = self.run_cli(['load', str(path), '--json', *extra], other)
                reports.append((code, json.loads(out)))
            self.assertEqual(reports[0], reports[1])
        code, out, _ = self.run_cli(['unload', '--no-restart'], fake)
        self.assertEqual(code, 0)
        self.assertTrue(out.endswith(hint))
        self.assertEqual(key.values, UNRELATED)
        with contextlib.redirect_stdout(io.StringIO()) as text, self.assertRaises(SystemExit):
            cli.main(['load', '--help'], platform=fake)
        self.assertIn('--no-restart no effect, kept for compatibility: the loader no longer restarts the GPU',
                      ' '.join(text.getvalue().split()))

    @unittest.skipIf(os.name == 'nt', 'checks the behaviour on other systems')
    def test_other_systems_fail_cleanly_without_loading_windows_code(self):
        code = ('import sys; from nvidia_vbios.windows_loader import cli; '
                'code = cli.main(["status", "--json"]); '
                'assert "nvidia_vbios.windows_loader.win32" not in sys.modules; sys.exit(code)')
        result = subprocess.run([sys.executable, '-c', code], capture_output=True, text=True,
                                env=dict(os.environ, PYTHONPATH=str(Path(cli.__file__).resolve().parents[2])))
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('runs on Windows only', result.stderr)
        self.assertEqual(json.loads(result.stdout)['ok'], False)
        import nvidia_vbios.windows_loader.win32 as win32  # importable without its DLLs
        self.assertEqual(win32.ctypes.sizeof(win32.SP_DEVINFO_DATA), 32)


if __name__ == '__main__':
    unittest.main()
