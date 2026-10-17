"""The loader window's controller (no Tk): selection, validation, enablement, confirmations and results."""
from dataclasses import replace
import importlib.util
import os
from pathlib import Path
import re
import string
import subprocess
import sys
import tempfile
import unittest

from nvidia_vbios.windows_loader import controller as gui_controller, operations as ops
from nvidia_vbios.windows_loader.controller import STRINGS, Controller, language_for, resolve_theme
from nvidia_vbios.windows_loader.devices import REG_BINARY, REG_DWORD, REG_SZ
from test_windows_loader import RTX_5070, RTX_5070_TI, UNRELATED, FakeKey, FakePlatform, adapter, chain, platform, rom


class ControllerTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)

    def file(self, data, name='edited.rom'):
        path = self.directory / name
        path.write_bytes(data)
        return str(path)

    def controller(self, fake, language='en'):
        c = Controller(fake, language)
        self.assertEqual(c.refresh(), [])
        return c

    def test_rows_show_the_override_state_from_status(self):
        gpus = [adapter(0, location='01:00.0'), adapter(1, location='02:00.0'),
                adapter(2, location='03:00.0', version='32.0.15.6094'), adapter(3, location='04:00.0')]
        keys = {gpus[0].instance_id: FakeKey(UNRELATED),
                gpus[1].instance_id: FakeKey({ops.VBIOS: (REG_BINARY, b'mvolt'), 'romOverride00': (REG_BINARY, b'r')}),
                gpus[2].instance_id: FakeKey({ops.VBIOS: (REG_BINARY, b'ours'), ops.BACKUP: (REG_SZ, ops.ABSENT)})}
        fake = FakePlatform(gpus, keys)

        def open_key(gpu, write=False):
            if gpu.instance_id not in keys:
                raise OSError(5, 'Access is denied')
            return keys[gpu.instance_id]
        fake.open_key = open_key
        rows = self.controller(fake).rows()
        self.assertEqual(rows[0], (0, 'GPU 0', '10de:2c09', '01:00.0', '616.56', 'none'))
        self.assertEqual(rows[1][5], "other tool's override: 5 bytes; romOverride records (mVolt+): 1")
        self.assertEqual(rows[2][4], '560.94 (too old, 616.56 or newer needed)')
        self.assertTrue(rows[2][5].startswith('loaded: 4 bytes, sha256 '))
        self.assertEqual(rows[3][5], 'cannot read the driver key')

    def test_selection_and_validation_use_the_dry_run(self):
        gpus = [adapter(0, device=RTX_5070, location='01:00.0'), adapter(1, device=RTX_5070_TI, location='02:00.0')]
        keys = {gpu.instance_id: FakeKey(UNRELATED) for gpu in gpus}
        fake = FakePlatform(gpus, keys)
        c = self.controller(fake)
        self.assertEqual(c.gpu['index'], 0)             # the first GPU is selected
        self.assertEqual(c.validation_text(), (False, 'Choose a VBIOS file.'))
        c.choose_file(self.file(rom(RTX_5070)))
        ok, text = c.validation_text()
        self.assertTrue(ok)
        self.assertIn(f'OK for GPU 0: blackwell-gb20x, {len(chain(RTX_5070))} bytes to store, sha256 ', text)
        c.select(1)
        ok, text = c.validation_text()
        self.assertFalse(ok)
        self.assertRegex(text, r'^Refused: payload refused for GPU 1 \(10de:2c05\): .*not 10de:2c05')
        self.assertFalse(c.can_load())
        c.refresh()
        self.assertEqual(c.gpu['index'], 1)             # kept across a refresh
        c.choose_file(str(self.directory / 'missing.rom'))
        self.assertTrue(c.validation_text()[1].startswith('Refused: Cannot read '))
        self.assertEqual(sum(key.calls != [] for key in keys.values()), 0)
        self.assertEqual(fake.writable_opens, 0)
        self.assertEqual(self.controller(FakePlatform()).validation_text(), (False, 'No NVIDIA GPU found.'))

    def test_old_driver_is_listed_and_refused_with_the_existing_message(self):
        fake, _ = platform(version='32.0.15.6094')
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        self.assertEqual(c.validation_text(),
                         (False, 'Refused: NVIDIA driver 560.94 is too old; 616.56 or newer is required'))

    def test_buttons_follow_admin_validation_override_and_busy(self):
        fake, key = platform(admin=False)
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        self.assertTrue(c.validation_text()[0])         # status and checks work without admin
        self.assertEqual((c.admin, c.can_load(), c.can_remove()), (False, False, False))
        fake.admin = True
        c.refresh()
        self.assertEqual((c.can_load(), c.can_remove()), (True, False))
        for values in ({ops.VBIOS: (REG_BINARY, b'x')}, {'romOverride05': (REG_BINARY, b'x')},
                       {ops.BACKUP: (REG_SZ, '0x00000012')}):
            key.values = {**UNRELATED, **values}
            c.refresh()
            self.assertTrue(c.can_remove(), values)
        c.busy = True
        self.assertEqual((c.can_load(), c.can_remove()), (False, False))
        c.select(5)
        self.assertEqual(c.gpu['index'], 0)

    def test_confirmation_texts(self):
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        c = self.controller(fake)
        path = self.file(rom(RTX_5070))
        c.choose_file(path)
        text = c.confirm_load()
        for part in ('GPU 0 (GPU 0, PCI 01:00.0)', f'File: {path}', f'Stored: {len(chain(RTX_5070))} bytes',
                     'set vbios = REG_BINARY', 'set EnableGpuFirmware = REG_DWORD 0x11',
                     'set VixLoaderPreviousEnableGpuFirmware = REG_SZ 0x00000011', 'another tool',
                     'The GPU is restarted', 'If the GPU does not start'):
            self.assertIn(part, text)
        c.restart = False
        self.assertIn('restart Windows to apply', c.confirm_load())
        question, error = c.confirm_remove()
        self.assertIsNone(error)
        self.assertIn('delete vbios', question)
        self.assertIn('Note: EnableGpuFirmware was not saved by this tool', question)
        self.assertEqual(key.calls, [])

    def test_load_and_remove_results(self):
        fake, key = platform(restart=(0, ''))
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        ok, lines = c.run('load')
        self.assertTrue(ok)
        self.assertEqual(lines[1], 'GPU restarted.')
        self.assertTrue(lines[0].startswith(f'GPU 0: VBIOS loaded, {len(chain(RTX_5070))} bytes'))
        self.assertEqual(key.values[ops.VBIOS], (REG_BINARY, chain(RTX_5070)))
        c.refresh()
        self.assertTrue(c.can_remove())
        self.assertTrue(c.rows()[0][5].startswith('loaded: '))
        c.restart = False
        ok, lines = c.run('unload')
        self.assertEqual((ok, lines), (True, ['GPU 0: override removed.',
                                              'GPU not restarted. Restart Windows to apply the change.']))
        self.assertEqual(key.values, UNRELATED)
        self.assertEqual(len(fake.restarted), 1)

    def test_worker_errors_are_reported_not_raised(self):
        fake, key = platform(restart=(None, 'timed out'))
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        ok, lines = c.run('load')
        self.assertFalse(ok)
        self.assertIn('restart failed (pnputil did not finish)', lines[-2])
        self.assertEqual(lines[-1], 'timed out')
        fake.admin = False
        self.assertEqual(c.run('unload'), (False, ['Error: administrator rights are required; run this from an '
                                                   'elevated PowerShell']))

        def broken(instance_id):
            raise RuntimeError('boom')
        fake.admin, fake.restart_device = True, broken
        self.assertEqual(c.run('unload'), (False, ['Unexpected error: RuntimeError: boom']))

    def same_model(self):
        gpus = [adapter(i, location=f'0{i + 1}:00.0') for i in range(3)]
        keys = {gpu.instance_id: FakeKey(UNRELATED) for gpu in gpus}
        return gpus, keys, FakePlatform(gpus, keys)

    def test_the_selected_device_is_written_even_when_the_indexes_shift(self):
        gpus, keys, fake = self.same_model()
        c = self.controller(fake)
        c.select(1)
        c.choose_file(self.file(rom(RTX_5070)))
        self.assertIn('PCI 02:00.0', c.confirm_load())
        fake._adapters = [replace(gpus[1], index=0), replace(gpus[2], index=1)]    # 01:00.0 fell off the bus
        ok, lines = c.run('load')
        self.assertTrue(ok)
        self.assertTrue(lines[0].startswith('GPU 0: VBIOS loaded'))
        self.assertEqual([ops.VBIOS in keys[gpu.instance_id].values for gpu in gpus], [False, True, False])
        self.assertEqual(fake.restarted, [gpus[1].instance_id])

    def test_a_gpu_that_goes_away_stays_selected_and_is_not_replaced(self):
        gpus, _, fake = self.same_model()
        c = self.controller(fake)
        c.select(1)
        c.choose_file(self.file(rom(RTX_5070)))
        fake._adapters = [replace(gpus[0]), replace(gpus[2], index=1)]
        c.refresh()
        self.assertIsNone(c.gpu)
        self.assertEqual((c.validation_text(), c.can_load(), c.confirm_load()), ((False, 'Select a GPU.'), False, None))
        fake._adapters = gpus
        c.refresh()
        self.assertEqual((c.gpu['location'], c.can_load()), ('02:00.0', True))

    def test_load_confirmation_comes_from_a_fresh_dry_run(self):
        fake, key = platform()
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        key.values.update({ops.VBIOS: (REG_BINARY, b'mvolt'), 'romOverride00': (REG_BINARY, b'r')})
        text = c.confirm_load()
        self.assertIn('another tool', text)
        self.assertIn('delete romOverride00', text)
        key.values[ops.BACKUP] = (REG_DWORD, 0x12)
        self.assertIsNone(c.confirm_load())
        self.assertFalse(c.can_load())

    def test_malformed_backup_can_be_removed_and_hints_name_the_window_buttons(self):
        fake, key = platform(values={ops.BACKUP: (REG_DWORD, 0x12)})
        c = self.controller(fake, 'es')
        c.choose_file(self.file(rom(RTX_5070)))
        self.assertTrue(c.validation_text()[1].endswith('run unload first En esta ventana: usa primero Quitar '
                                                        'reemplazo.'))
        self.assertTrue(c.can_remove())
        self.assertEqual(c.rows()[0][5], 'solo el valor guardado de EnableGpuFirmware')
        self.assertIn('delete VixLoaderPreviousEnableGpuFirmware', c.confirm_remove()[0])

        class FailingKey(FakeKey):
            def set(self, name, kind, value):
                raise OSError(5, 'Access is denied')
        fake, key = platform()
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        fake.keys[fake.adapters()[0].instance_id] = FailingKey(key.values)
        ok, lines = c.run('load')
        self.assertFalse(ok)
        self.assertTrue(lines[0].endswith('before restarting Windows In this window: click Refresh, then Remove '
                                          'override, before restarting Windows.'), lines)

    def test_run_never_raises(self):
        fake, _ = platform()
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        original = gui_controller.load
        gui_controller.load = lambda *args, **kwargs: {}
        self.addCleanup(setattr, gui_controller, 'load', original)
        self.assertEqual(c.run('load'), (False, ["Unexpected error: KeyError: 'gpu'"]))

    def test_status_failure_is_a_message(self):
        fake = FakePlatform()
        def adapters():
            raise OSError(13, 'SetupAPI failed')
        fake.adapters = adapters
        c = Controller(fake)
        self.assertEqual(c.refresh(), ['Cannot list the GPUs: [Errno 13] SetupAPI failed'])
        self.assertEqual(c.rows(), [])

    def test_spanish_has_every_key_with_the_same_placeholders(self):
        def fields(text):
            return {name for _, name, _, _ in string.Formatter().parse(text) if name}
        self.assertEqual(set(STRINGS['es']), set(STRINGS['en']))
        for name, text in STRINGS['en'].items():
            self.assertEqual(fields(STRINGS['es'][name]), fields(text), name)
        fake, _ = platform(admin=False)
        c = self.controller(fake, 'es')
        self.assertEqual(c.rows()[0][5], 'ninguno')
        self.assertEqual(c.validation_text(), (False, 'Elige un archivo VBIOS.'))
        self.assertEqual((language_for(0x0C0A), language_for(0x080A), language_for(0x0409)), ('es', 'es', 'en'))

    def test_language_switch_rewrites_texts_and_the_refusal(self):
        fake, _ = platform()
        c = self.controller(fake, 'es')
        c.choose_file(self.file(b'\x00' * 0x400))
        ok, text = c.validation_text()
        self.assertFalse(ok)
        self.assertTrue(text.startswith('Rechazado: '))
        c.set_language('en')
        self.assertEqual((c.language, c.rows()[0][5]), ('en', 'none'))
        self.assertTrue(c.validation_text()[1].startswith('Refused: '))
        c.choose_file(str(self.directory / 'missing.rom'))
        c.set_language('es')
        self.assertTrue(c.validation_text()[1].startswith('Rechazado: No se puede leer '))
        c.busy = True                     # not while a load or unload runs
        c.set_language('en')
        self.assertEqual(c.language, 'es')

    def test_theme_auto_follows_the_windows_app_mode(self):
        self.assertEqual(Controller(FakePlatform()).theme, 'auto')
        self.assertEqual((resolve_theme('auto', lambda: True), resolve_theme('auto', lambda: False)), ('dark', 'light'))
        for mode in ('light', 'dark'):
            self.assertEqual(resolve_theme(mode, lambda: mode == 'light'), mode)
        with self.assertRaises(ValueError):
            resolve_theme('blue')
        if os.name != 'nt':
            self.assertEqual(resolve_theme('auto'), 'light')     # no Windows registry: light

    @unittest.skipIf(os.name == 'nt', 'checks the behaviour on other systems')
    def test_controller_needs_no_tk_and_the_window_fails_cleanly_off_windows(self):
        code = ('import sys; from nvidia_vbios.windows_loader import controller; '
                'assert "tkinter" not in sys.modules; assert controller.system_language() == "en"')
        if importlib.util.find_spec('tkinter'):
            code += '; from nvidia_vbios.windows_loader import gui; sys.exit(gui.main([]))'
        result = subprocess.run([sys.executable, '-c', code], capture_output=True, text=True,
                                env=dict(os.environ, PYTHONPATH=str(Path(gui_controller.__file__).resolve().parents[2])))
        if importlib.util.find_spec('tkinter'):
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn('runs on Windows only', result.stderr)
        else:
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(re.search('Traceback', result.stderr), result.stderr)


if __name__ == '__main__':
    unittest.main()
