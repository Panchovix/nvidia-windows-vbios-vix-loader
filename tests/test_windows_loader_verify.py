"""Verify: NvAPI readback through a fake nvapi_QueryInterface, and the comparison with the stored override.

The fake NvAPI functions are real ctypes callbacks, so the structures and pointers go through ctypes as they
would on Windows. No Windows, GPU or NVIDIA DLL is needed.
"""
import contextlib
import ctypes
import io
import json
import os
import unittest
from unittest import mock

from nvidia_vbios.windows_loader import cli, nvapi, operations as ops
from nvidia_vbios.windows_loader.devices import REG_BINARY, REG_SZ
from test_windows_loader import RTX_5070, chain, platform, rom

MESSAGES = {-174: b'NVAPI_INSUFFICIENT_BUFFER', -101: b'NVAPI_EXPECTED_PHYSICAL_GPU_HANDLE',
            -6: b'NVAPI_NVIDIA_DEVICE_NOT_FOUND'}


class FakeNvApi:
    """GPUs as (bus, slot, image); `missing` functions are absent, `status` makes a function fail."""
    def __init__(self, gpus, missing=(), status=None):
        self.gpus, self.missing, self.status, self.calls = list(gpus), set(missing), dict(status or {}), []
        functions = {'Initialize': self.initialize, 'Unload': self.unload, 'GetErrorMessage': self.error_message,
                     'EnumPhysicalGPUs': self.enum, 'GPU_GetBusId': self.bus, 'GPU_GetBusSlotId': self.slot,
                     'GPU_GetVbiosImage': self.image}
        self.names = {nvapi.FUNCTIONS[name][0]: name for name in functions}
        self.callbacks = {name: nvapi.FUNCTIONS[name][1](function) for name, function in functions.items()}

    def query(self, function_id):
        name = self.names.get(function_id)
        if name is None or name in self.missing:
            return None
        return ctypes.cast(self.callbacks[name], ctypes.c_void_p).value

    def enter(self, name):
        self.calls.append(name)
        return self.status.get(name, 0)

    def initialize(self):
        return self.enter('Initialize')

    def unload(self):
        return self.enter('Unload')

    def error_message(self, status, text):
        if status not in MESSAGES:
            return -5
        ctypes.memmove(text, MESSAGES[status] + b'\0', len(MESSAGES[status]) + 1)
        return 0

    def enum(self, handles, count):
        for index in range(len(self.gpus)):
            handles[index] = 0x1000 + index
        count[0] = len(self.gpus)
        return self.enter('EnumPhysicalGPUs')

    def gpu(self, handle):
        index = (handle or 0) - 0x1000
        return self.gpus[index] if 0 <= index < len(self.gpus) else None

    def bus(self, handle, value):
        value[0] = self.gpu(handle)[0]
        return self.status.get(('GPU_GetBusId', handle), 0)

    def slot(self, handle, value):
        value[0] = self.gpu(handle)[1]
        return self.status.get(('GPU_GetBusSlotId', handle), 0)

    def image(self, handle, request):
        status, request = self.enter('GPU_GetVbiosImage'), request.contents
        if status:
            return status
        if self.gpu(handle) is None:
            return -101
        if request.version != 0x00020010:
            return -9
        if not request.buffer or request.size != 0xFA000:
            request.size = 0xFA000
            return -174
        data = self.gpu(handle)[2][:0xFA000]
        ctypes.memmove(request.buffer, data, len(data))
        request.size = len(data)
        return 0


class NvApiTests(unittest.TestCase):
    def test_structure_and_function_ids(self):
        self.assertEqual(ctypes.sizeof(nvapi.NV_GPU_VBIOS_IMAGE), 16 if ctypes.sizeof(ctypes.c_void_p) == 8 else 12)
        self.assertEqual((nvapi.IMAGE_SIZE, nvapi.IMAGE_VERSION, nvapi.MAX_PHYSICAL_GPUS), (1024000, 16 | 2 << 16, 64))
        self.assertEqual({name: function_id for name, (function_id, _) in nvapi.FUNCTIONS.items()},
                         {'Initialize': 0x0150E828, 'Unload': 0xD22BDD7E, 'GetErrorMessage': 0x6C2D048C,
                          'EnumPhysicalGPUs': 0xE5AC921F, 'GPU_GetBusId': 0x1BE0B8E5, 'GPU_GetBusSlotId': 0x2A0A350F,
                          'GPU_GetVbiosImage': 0xFC13EE11})

    def test_the_gpu_is_found_by_bus_and_slot(self):
        fake = FakeNvApi([(1, 0, b'first'), (2, 0, b'second'), (0x41, 3, b'third')])
        self.assertEqual(nvapi.read_vbios_image('02:00.0', fake.query), b'second')
        self.assertEqual(nvapi.read_vbios_image('41:03.0', fake.query), b'third')
        self.assertEqual((fake.calls[0], fake.calls[-1]), ('Initialize', 'Unload'))
        fake.status[('GPU_GetBusId', 0x1000)] = -101         # a GPU that cannot answer is skipped
        self.assertEqual(nvapi.read_vbios_image('02:00.0', fake.query), b'second')
        fake = FakeNvApi([(1, 0, b'slot 0'), (1, 1, b'slot 1')])     # same bus: the slot decides
        self.assertEqual(nvapi.read_vbios_image('01:01.0', fake.query), b'slot 1')
        self.assertEqual(nvapi.read_vbios_image('01:00.0', fake.query), b'slot 0')
        fake.status[('GPU_GetBusSlotId', 0x1000)] = -101
        with self.assertRaisesRegex(nvapi.NvApiError, r'at PCI 01:00 \(it lists: 01:01\)'):
            nvapi.read_vbios_image('01:00.0', fake.query)

    def test_a_long_image_is_cut_at_the_buffer_size(self):
        fake = FakeNvApi([(1, 0, bytes(range(256)) * 5000)])
        data = nvapi.read_vbios_image('01:00.0', fake.query)
        self.assertEqual((len(data), data), (nvapi.IMAGE_SIZE, (bytes(range(256)) * 5000)[:nvapi.IMAGE_SIZE]))

    def test_errors_name_the_cause_and_unload(self):
        gpus = [(1, 0, b'first'), (2, 0, b'second')]
        cases = [(dict(missing={'GPU_GetVbiosImage'}), '01:00.0', 'has no NvAPI_GPU_GetVbiosImage.*617.42'),
                 (dict(status={'GPU_GetVbiosImage': -3}), '01:00.0', 'has no NvAPI_GPU_GetVbiosImage'),
                 (dict(status={'GPU_GetVbiosImage': -174}), '01:00.0',
                  r'NvAPI_GPU_GetVbiosImage failed: NVAPI_INSUFFICIENT_BUFFER \(-174\)'),
                 (dict(status={'GPU_GetVbiosImage': -104}), '01:00.0', 'GetVbiosImage failed: status -104'),
                 (dict(status={'GPU_GetVbiosImage': -174}, missing={'GetErrorMessage'}), '01:00.0', 'status -174'),
                 (dict(status={'EnumPhysicalGPUs': -6}), '01:00.0', 'NVAPI_NVIDIA_DEVICE_NOT_FOUND'),
                 ({}, '03:00.0', r'not list exactly one GPU at PCI 03:00 \(it lists: 01:00, 02:00\)'),
                 ({}, None, 'PCI location of this GPU is unknown')]
        for options, location, message in cases:
            with self.subTest(message=message):
                fake = FakeNvApi(gpus, **options)
                with self.assertRaisesRegex(nvapi.NvApiError, message):
                    nvapi.read_vbios_image(location, fake.query)
                self.assertEqual(fake.calls[-1], 'Unload')
        fake = FakeNvApi(gpus, status={'Initialize': -1})
        with self.assertRaisesRegex(nvapi.NvApiError, 'NvAPI_Initialize failed: status -1'):
            nvapi.read_vbios_image('01:00.0', fake.query)
        self.assertEqual(fake.calls, ['Initialize'])           # nothing to unload
        self.assertIsInstance(nvapi.NvApiError('x'), OSError)  # reported like the other platform errors

    @unittest.skipIf(os.name == 'nt', 'must not load the real nvapi64.dll')
    def test_without_nvapi64_dll_the_error_is_clear(self):
        with self.assertRaisesRegex(nvapi.NvApiError, 'cannot load nvapi64.dll'):
            nvapi.read_vbios_image('01:00.0')

    def test_nvapi64_dll_is_loaded_from_system32_only(self):
        loaded = []

        def windll(path):
            loaded.append(path)
            raise OSError('not loaded in tests')
        with mock.patch.object(ctypes, 'WinDLL', windll, create=True), \
                mock.patch.dict(os.environ, {'SystemRoot': 'D:\\Win'}):
            with self.assertRaisesRegex(nvapi.NvApiError, 'cannot load nvapi64.dll'):
                nvapi.query_interface()
        self.assertEqual(loaded, [os.path.join('D:\\Win', 'System32', 'nvapi64.dll')])


BIG = bytes(range(256)) * 4001       # longer than what NvAPI returns


class VerifyTests(unittest.TestCase):
    def verify(self, values=None, image=b'', **options):
        fake, key = platform(values=values, image=image, admin=False, **options)
        report = ops.verify(fake)
        self.assertEqual((key.calls, fake.writable_opens), ([], 0))    # read-only, no admin
        self.assertEqual(len(fake.reads), 2)
        return report

    def test_the_loaded_override_is_active(self):
        fake, key = platform(image=chain(RTX_5070))
        ops.load(fake, rom(RTX_5070))
        report = ops.verify(fake)
        self.assertEqual((report['state'], report['match'], report['compared'], report['first_difference']),
                         ('override-active', True, 'full', None))
        self.assertEqual(report['readback'], report['configured'] | {'truncated': False})
        self.assertEqual(report['configured']['size'], len(chain(RTX_5070)))
        self.assertIn('verified byte by byte', report['message'])
        stored = chain(RTX_5070)
        report = self.verify({ops.VBIOS: (REG_BINARY, stored), 'romOverride00': (REG_BINARY, b'r')}, stored)
        self.assertEqual(report['state'], 'override-active')
        self.assertNotIn('romOverride', report['message'])    # the records note is only for no-override

    def test_a_different_image_is_not_active_with_the_first_difference(self):
        stored = chain(RTX_5070)
        running = stored[:0x123] + b'\x00' + stored[0x124:]
        report = self.verify({ops.VBIOS: (REG_BINARY, stored)}, running)
        self.assertEqual((report['state'], report['match'], report['first_difference']),
                         ('override-not-active', False, 0x123))
        self.assertEqual(report['message'], 'The driver is NOT using the stored override (first difference at offset '
                                            "0x123). If it was just loaded, restart Windows; otherwise the driver may "
                                            "have rejected it and is running the card's own VBIOS.")
        report = self.verify({ops.VBIOS: (REG_BINARY, stored)}, stored[:1024])
        self.assertEqual((report['state'], report['first_difference']), ('override-not-active', 1024))

    def test_without_an_override_the_card_vbios_runs(self):
        report = self.verify(image=b'card image')
        self.assertEqual((report['state'], report['configured'], report['match']), ('no-override', None, None))
        self.assertEqual(report['readback']['size'], 10)
        self.assertNotIn('romOverride', report['message'])
        self.assertIn("should be running the card's own VBIOS", report['message'])

    def test_a_removed_override_still_running_gets_the_restart_hint(self):
        fake, _ = platform(image=chain(RTX_5070))
        ops.load(fake, rom(RTX_5070))
        ops.unload(fake)                       # the driver keeps the removed image until Windows restarts
        report = ops.verify(fake)
        self.assertEqual(report['state'], 'no-override')
        self.assertNotIn('the driver is running', report['message'])
        self.assertEqual(report['message'], "No override is stored, so the driver should be running the card's own "
                                            'VBIOS. If one was just removed, restart Windows first.')
        report = self.verify({'romOverride00': (REG_BINARY, b'r'), ops.VBIOS: (REG_SZ, 'text')}, b'card image')
        self.assertEqual(report['state'], 'no-override')
        self.assertIn('romOverride records are stored too', report['message'])

    def test_two_different_reads_are_unstable(self):
        fake, _ = platform(values={ops.VBIOS: (REG_BINARY, b'override')})
        reads = iter([b'override', b'overridf'])
        fake.vbios_image = lambda adapter: next(reads)
        report = ops.verify(fake)
        self.assertEqual((report['state'], report['match']), ('unstable', None))
        self.assertEqual(report['second_readback']['size'], 8)
        self.assertIn('try again', report['message'])

    def test_a_cut_readback_compares_the_prefix_and_says_so(self):
        size = nvapi.IMAGE_SIZE
        report = self.verify({ops.VBIOS: (REG_BINARY, BIG)}, BIG[:size])
        self.assertEqual((report['state'], report['compared'], report['readback']['truncated']),
                         ('override-active', 'prefix', True))
        self.assertEqual(report['message'], f'The first {size} bytes the driver is running match the stored '
                                            f'override; NvAPI returns no more, so the last {len(BIG) - size} bytes '
                                            'cannot be read back.')
        report = self.verify({ops.VBIOS: (REG_BINARY, BIG)}, b'\xff' + BIG[1:size])
        self.assertEqual((report['state'], report['compared'], report['first_difference']),
                         ('override-not-active', 'prefix', 0))
        report = self.verify({ops.VBIOS: (REG_BINARY, BIG[:1000])}, BIG[:size])     # shorter override: whole
        self.assertEqual((report['state'], report['compared'], report['first_difference']),
                         ('override-not-active', 'full', 1000))

    def test_selection_and_readback_errors(self):
        fake, _ = platform()
        with self.assertRaisesRegex(ops.LoaderError, 'not a PCI location'):
            ops.verify(fake, pci='1')

        def failing(adapter):
            raise nvapi.NvApiError('this NVIDIA driver has no NvAPI_GPU_GetVbiosImage')
        fake.vbios_image = failing
        with self.assertRaisesRegex(nvapi.NvApiError, 'no NvAPI_GPU_GetVbiosImage'):
            ops.verify(fake)


class VerifyCliTests(unittest.TestCase):
    def run_cli(self, argv, fake):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = cli.main(argv, platform=fake)
        return code, out.getvalue(), err.getvalue()

    def test_states_and_exit_codes(self):
        fake, key = platform(image=b'card image', admin=False)
        code, out, _ = self.run_cli(['verify'], fake)
        self.assertEqual(code, 0)
        self.assertIn('  running VBIOS (NvAPI): 10 bytes, sha256 ', out)
        self.assertIn('  stored override: none', out)
        self.assertTrue(out.endswith('If one was just removed, restart Windows first.\n'))
        key.values[ops.VBIOS] = (REG_BINARY, b'override')
        code, out, _ = self.run_cli(['verify', '--gpu', '0', '--json'], fake)
        report = json.loads(out)
        self.assertEqual((code, report['ok'], report['state'], report['first_difference']),
                         (4, False, 'override-not-active', 0))
        fake.image = BIG[:nvapi.IMAGE_SIZE]
        key.values[ops.VBIOS] = (REG_BINARY, BIG)
        code, out, _ = self.run_cli(['verify', '--pci', '01:00.0'], fake)
        self.assertEqual(code, 0)
        self.assertIn(f'{nvapi.IMAGE_SIZE} bytes, sha256 ', out)
        self.assertIn(f'(NvAPI returns at most {nvapi.IMAGE_SIZE} bytes)', out)
        self.assertIn(f'  stored override: {len(BIG)} bytes', out)
        self.assertNotIn('verified byte by byte', out)
        reads = iter([b'override', b'overridf'])
        fake.vbios_image = lambda adapter: next(reads)
        key.values[ops.VBIOS] = (REG_BINARY, b'override')
        code, out, _ = self.run_cli(['verify'], fake)
        self.assertEqual(code, 4)
        self.assertIn('  running VBIOS (NvAPI): 8 bytes, sha256 ', out)
        self.assertIn('  second read: 8 bytes, sha256 ', out)

    def test_errors_and_options(self):
        fake, _ = platform()

        def failing(adapter):
            raise nvapi.NvApiError('cannot load nvapi64.dll')
        fake.vbios_image = failing
        code, out, err = self.run_cli(['verify', '--json'], fake)
        self.assertEqual((code, json.loads(out)), (1, {'ok': False, 'error': 'cannot load nvapi64.dll'}))
        self.assertIn('vbios-loader: cannot load nvapi64.dll', err)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            cli.main(['verify', '--dry-run'], platform=fake)


if __name__ == '__main__':
    unittest.main()
