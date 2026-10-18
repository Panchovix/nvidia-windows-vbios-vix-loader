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
from nvidia_vbios.windows_loader.devices import (REG_BINARY, REG_DWORD, REG_SZ, Adapter, normalize_location,
                                                 nvidia_version, parse_instance_id, parse_location, pci_location)
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
    def __init__(self, values=None, drop_writes=False):
        self.values = dict(values or {})
        self.calls = []
        self.flushes = 0
        self.closed = 0
        self.drop_writes = drop_writes

    def get(self, name):
        return self.values.get(name)

    def set(self, name, kind, value):
        self.calls.append(('set', name))
        if not self.drop_writes:
            self.values[name] = (kind, value)

    def delete(self, name):
        self.calls.append(('delete', name))
        return self.values.pop(name, None) is not None

    def flush(self):
        self.flushes += 1

    def close(self):
        self.closed += 1


class FakePlatform:
    def __init__(self, adapters=(), keys=None, admin=True, restart=(0, ''), image=b''):
        self._adapters, self.keys, self.admin, self.restart = list(adapters), keys or {}, admin, restart
        self.restarted, self.writable_opens = [], 0
        self.image, self.reads = image, []      # what vbios_image returns, and the devices it was read from

    def adapters(self):
        return self._adapters

    def open_key(self, adapter, write=False):
        self.writable_opens += write
        return self.keys[adapter.instance_id]

    def is_admin(self):
        return self.admin

    def restart_device(self, instance_id):
        self.restarted.append(instance_id)
        return self.restart

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
        self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x11))
        self.assertEqual(key.values[ops.BACKUP], (REG_SZ, '0x00000012'))
        self.assertFalse(any(name.startswith('romOverride') for name in key.values))
        for name, value in UNRELATED.items():
            self.assertEqual(key.values[name], value)
        self.assertEqual({name for _, name in key.calls},
                         {ops.BACKUP, ops.VBIOS, ops.FIRMWARE, 'romOverride00', 'romOverride7F'})
        self.assertEqual(key.flushes, 1)
        self.assertEqual(fake.restarted, [fake.adapters()[0].instance_id])
        self.assertTrue(report['restart']['ok'])
        self.assertEqual(report['payload']['dropped_prefix'], PREFIX)
        self.assertEqual(report['payload']['dropped_trailing'], TRAILING)

    def test_unload_restores_a_previous_value(self):
        fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x12)})
        ops.load(fake, rom(RTX_5070))
        ops.unload(fake)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x12)})
        self.assertEqual(len(fake.restarted), 2)

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
        self.assertIn('No override', ops.unload(fake)['message'])
        self.assertEqual((key.calls, fake.restarted), ([], []))

    def test_dry_run_writes_nothing_and_needs_no_admin(self):
        for operation in (lambda p: ops.load(p, rom(RTX_5070), dry_run=True), lambda p: ops.unload(p, dry_run=True)):
            fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x12), ops.VBIOS: (REG_BINARY, b'old'),
                                         'romOverride01': (REG_BINARY, b'x')}, admin=False)
            report = operation(fake)
            self.assertTrue(report['dry_run'] and report['steps'])
            self.assertEqual((key.calls, key.flushes, fake.restarted, fake.writable_opens), ([], 0, [], 0))
        self.assertIn({'op': 'delete', 'name': 'romOverride01'},
                      ops.load(platform(admin=False, values={'romOverride01': (REG_BINARY, b'x')})[0],
                               rom(RTX_5070), dry_run=True)['steps'])

    def test_refusals_write_nothing(self):
        cases = [(dict(admin=False), {}, 'administrator'),
                 (dict(version='32.0.15.6094'), {}, '560.94 is too old'),
                 (dict(version='unknown'), {}, 'cannot read the NVIDIA driver version'),
                 (dict(device=RTX_5070_TI), {}, 'payload refused.*not 10de:2c05'),
                 (dict(device=RTX_4090), {}, 'payload refused'),
                 (dict(values={ops.FIRMWARE: (REG_BINARY, b'\x11')}), {}, 'unexpected value'),
                 (dict(values={ops.FIRMWARE: (REG_DWORD, b'\x11\x00')}), {}, 'unexpected value'),
                 (dict(values={ops.BACKUP: (REG_DWORD, 0x12)}), {}, 'run unload first')]
        for options, _, message in cases:
            with self.subTest(message=message):
                fake, key = platform(**options)
                with self.assertRaisesRegex(ops.LoaderError, message):
                    ops.load(fake, rom(RTX_5070, 1100))
                self.assertEqual((key.calls, fake.restarted), ([], []))
        fake, key = platform(admin=False, values={ops.VBIOS: (REG_BINARY, b'old')})
        with self.assertRaisesRegex(ops.LoaderError, 'administrator'):
            ops.unload(fake)
        self.assertEqual(key.calls, [])

    def test_partial_write_failure_says_so_and_unload_recovers(self):
        class FailingKey(FakeKey):
            def set(self, name, kind, value):
                if name == ops.FIRMWARE:
                    raise OSError(5, 'writing EnableGpuFirmware failed: Access is denied')
                super().set(name, kind, value)
        fake, key = platform()
        gpu = fake.adapters()[0]
        fake.keys[gpu.instance_id] = key = FailingKey(key.values)
        with self.assertRaisesRegex(ops.LoaderError, 'Access is denied; the registry may be partly changed.*unload'):
            ops.load(fake, rom(RTX_5070))
        self.assertIn(ops.VBIOS, key.values)
        self.assertEqual(fake.restarted, [])
        ops.unload(fake)
        self.assertEqual(key.values, UNRELATED)

    def test_other_tools_override_is_replaced_and_its_records_removed_by_unload(self):
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        self.assertIn('another tool', ops.load(fake, rom(RTX_5070))['message'])
        ops.unload(fake)
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})
        fake, key = platform(values={'romOverride00': (REG_BINARY, b'r'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        report = ops.unload(fake)
        self.assertIn('left as it is', report['message'])
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})

    def test_malformed_values_are_reported_and_unload_still_removes_the_image(self):
        fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, b'\x11\x00')})
        self.assertEqual(ops.status(fake)[0]['override']['enable_gpu_firmware'], 'unexpected REG_DWORD')
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'x'), ops.BACKUP: (REG_SZ, 'junk'),
                                     ops.FIRMWARE: (REG_DWORD, 0x11)})
        self.assertIn('unexpected value', ops.unload(fake)['message'])
        self.assertEqual(key.values, {**UNRELATED, ops.FIRMWARE: (REG_DWORD, 0x11)})

    def test_read_back_mismatch_is_an_error_without_restart(self):
        fake, key = platform()
        key.drop_writes = True
        with self.assertRaisesRegex(ops.LoaderError, 'read back'):
            ops.load(fake, rom(RTX_5070))
        self.assertEqual(fake.restarted, [])

    def test_restart_failure_reports_the_restart_windows_hint(self):
        for result in ((3010, 'reboot required'), (None, 'timed out')):
            with self.subTest(result=result):
                fake, key = platform(restart=result)
                report = ops.load(fake, rom(RTX_5070))
                self.assertFalse(report['restart']['ok'])
                self.assertIn('Restart Windows', report['restart']['message'])
                self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x11))
        fake, _ = platform()
        self.assertEqual(ops.load(fake, rom(RTX_5070), restart=False)['restart']['requested'], False)
        self.assertEqual(fake.restarted, [])


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
        ops.load(fake, rom(RTX_5070), restart=False)
        key.values['romOverride02'] = (REG_BINARY, b'x')
        code, out, _ = self.run_cli(['status', '--json'], fake)
        entry = json.loads(out)[0]
        self.assertEqual(code, 0)
        self.assertEqual((entry['location'], entry['pci_id'], entry['nvidia_driver_version']), ('01:00.0', '10de:2c09', '616.56'))
        self.assertEqual(entry['override']['vbios']['size'], len(chain(RTX_5070)))
        self.assertEqual(entry['override']['enable_gpu_firmware'], '0x11')
        self.assertEqual(entry['override']['saved_previous_enable_gpu_firmware'], 'absent')
        self.assertEqual(entry['override']['rom_override_records'], ['romOverride02'])
        self.assertIn('romOverride records: 1', self.run_cli(['status'], fake)[1])

    def test_load_and_unload_commands(self):
        fake, key = platform(restart=(1, 'failed'))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'edited.rom'
            path.write_bytes(rom(RTX_5070))
            code, out, _ = self.run_cli(['load', str(path), '--pci', '01:00.0', '--dry-run'], fake)
            self.assertEqual(code, 0)
            self.assertIn('Dry run: nothing was written.', out)
            code, out, _ = self.run_cli(['load', str(path), '--gpu', '0', '--json'], fake)
            self.assertEqual(code, 3)
            self.assertFalse(json.loads(out)['ok'])
            code, out, err = self.run_cli(['load', str(Path(directory) / 'missing.rom')], fake)
            self.assertEqual(code, 1)
            self.assertIn('cannot read', err)
        code, out, _ = self.run_cli(['unload', '--no-restart'], fake)
        self.assertEqual(code, 0)
        self.assertIn('Restart Windows', out)
        self.assertEqual(key.values, UNRELATED)

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
