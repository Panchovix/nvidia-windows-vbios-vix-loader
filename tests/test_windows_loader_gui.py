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
from nvidia_vbios.windows_loader.devices import REG_BINARY, REG_DWORD, REG_QWORD, REG_SZ
from nvidia_vbios.windows_loader.nvapi import NvApiError
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
        applies = ('The change applies when you restart Windows. Shutting down works too when Windows Fast Startup '
                   'is off. With it on, the change waits for the next restart, and nothing has to be repeated.')
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_DWORD, 0x11)})
        c = self.controller(fake)
        path = self.file(rom(RTX_5070))
        c.choose_file(path)
        text = c.confirm_load()
        for part in ('GPU 0 (GPU 0, PCI 01:00.0)', f'File: {path}', f'Stored: {len(chain(RTX_5070))} bytes',
                     'set vbios = REG_BINARY', 'set VixLoaderPreviousEnableGpuFirmware = REG_SZ 0x00000011',
                     'another tool', STRINGS['en']['firmware_gsp_kept'],
                     applies,
                     'If the GPU does not start'):
            self.assertIn(part, text)
        self.assertNotIn('set EnableGpuFirmware', text)
        question, error = c.confirm_remove()
        self.assertIsNone(error)
        self.assertIn('delete vbios', question)
        self.assertIn('Note: EnableGpuFirmware was not saved by this tool', question)
        self.assertTrue(question.endswith('\n\n' + applies))
        self.assertEqual(key.calls, [])

    def test_load_and_remove_results(self):
        restart = ('Restart Windows to apply the change. Shutting down works too when Windows Fast Startup is off. '
                   'With it on, the change waits for the next restart, and nothing has to be repeated.')
        fake, key = platform()
        c = self.controller(fake)
        self.assertFalse(hasattr(c, 'restart'))         # no "restart the GPU" choice any more
        c.choose_file(self.file(rom(RTX_5070)))
        ok, lines = c.run('load')
        self.assertTrue(ok)
        self.assertEqual(lines[1:], [restart])
        self.assertTrue(lines[0].startswith(f'GPU 0: VBIOS loaded, {len(chain(RTX_5070))} bytes'))
        self.assertEqual(key.values[ops.VBIOS], (REG_BINARY, chain(RTX_5070)))
        c.refresh()
        self.assertTrue(c.can_remove())
        self.assertTrue(c.rows()[0][5].startswith('loaded: '))
        ok, lines = c.run('unload')
        self.assertEqual((ok, lines), (True, ['GPU 0: override removed.', restart]))
        self.assertEqual(key.values, UNRELATED)

    def test_a_non_dword_enable_gpu_firmware_is_saved_and_put_back_in_both_languages(self):
        qword = (0x11).to_bytes(8, 'little')
        for language, removed in (('en', 'GPU 0: override removed.'), ('es', 'GPU 0: reemplazo quitado.')):
            with self.subTest(language=language):
                fake, key = platform(values={ops.FIRMWARE: (REG_QWORD, qword)})
                c = self.controller(fake, language)
                c.choose_file(self.file(rom(RTX_5070)))
                self.assertTrue(c.validation_text()[0])
                question = c.confirm_load()
                self.assertIn('set VixLoaderPreviousEnableGpuFirmware = REG_SZ hex(b):11,00,00,00,00,00,00,00',
                              question)
                self.assertNotIn('set EnableGpuFirmware', question)
                self.assertNotIn(STRINGS[language]['other_tool'], question)
                ok, lines = c.run('load')
                self.assertTrue(ok)
                self.assertEqual(lines[1:], [STRINGS[language]['restart_windows']])
                self.assertEqual(key.get_raw(ops.FIRMWARE), (REG_QWORD, qword))      # left as it is
                c.refresh()
                self.assertTrue(c.rows()[0][5].startswith(STRINGS[language]['override_loaded'].split(':')[0]))
                question, error = c.confirm_remove()
                self.assertIsNone(error)
                self.assertIn('set EnableGpuFirmware = REG_QWORD 0x11', question)
                ok, lines = c.run('unload')
                self.assertEqual((ok, lines[0]), (True, removed))
                self.assertEqual(key.get_raw(ops.FIRMWARE), (REG_QWORD, qword))
                self.assertEqual(set(key.values), {*UNRELATED, ops.FIRMWARE})

        # With another tool's override as well, only that note is shown: EnableGpuFirmware is not changed.
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, b'mvolt'), ops.FIRMWARE: (REG_SZ, b'1\0')})
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        ok, lines = c.run('load')
        self.assertEqual(lines[1:], [STRINGS['en']['other_tool'], STRINGS['en']['restart_windows']])
        # A value too large to save is refused before anything is written.
        fake, key = platform(values={ops.FIRMWARE: (REG_BINARY, bytes(5000))})
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        self.assertEqual(c.validation_text(), (False, 'Refused: EnableGpuFirmware holds a REG_BINARY value of 5000 '
                                                      'bytes, too large to save (at most 4096); not changing it'))
        self.assertIsNone(c.confirm_load())
        self.assertEqual(key.calls, [])

    def test_gpus_left_in_gsp_firmware_mode_by_0_3_x_are_named_and_a_load_undoes_it(self):
        old = {ops.VBIOS: (REG_BINARY, chain(RTX_5070)), ops.FIRMWARE: (REG_DWORD, 0x11),
               ops.BACKUP: (REG_SZ, '0x00000012')}
        gpus = [adapter(i, location=f'0{i + 1}:00.0') for i in range(3)]
        keys = {gpus[0].instance_id: FakeKey(old), gpus[1].instance_id: FakeKey(UNRELATED),
                gpus[2].instance_id: FakeKey({**old, ops.BACKUP: (REG_SZ, ops.ABSENT)})}
        self.assertEqual(Controller(FakePlatform(gpus, keys)).refresh(),
                         [STRINGS['en']['gsp_old_loader'].format(index=index) for index in (0, 2)])
        texts = {
            'en': ('GPU 0: GSP firmware mode is on (set by loader 0.3.2 or older). Load the VBIOS file again with this '
                   'version (or remove the override) and restart Windows.',
                   'EnableGpuFirmware goes back to its value before the first load: 0x00000012. Loader 0.3.2 or older '
                   "set it to 0x11, which turns on the driver's GSP firmware mode; this version no longer turns that "
                   'mode on.'),
            'es': ('GPU 0: el modo de firmware GSP está activado (lo activó el loader 0.3.2 o anterior). Vuelve a '
                   'cargar el archivo VBIOS con esta versión (o quita el reemplazo) y reinicia Windows.',
                   'EnableGpuFirmware vuelve a su valor de antes de la primera carga: 0x00000012. El loader 0.3.2 o '
                   'anterior lo puso en 0x11, lo que activa el modo de firmware GSP del driver; esta versión ya no '
                   'activa ese modo.'),
        }
        for language, (line, restored) in texts.items():
            with self.subTest(language=language):
                fake, key = platform(values=old)
                c = Controller(fake, language)
                self.assertEqual(c.refresh(), [line])
                c.choose_file(self.file(rom(RTX_5070)))
                question = c.confirm_load()
                self.assertIn('  set EnableGpuFirmware = REG_DWORD 0x12', question)
                self.assertIn(restored, question.split('\n\n'))
                ok, lines = c.run('load')
                self.assertEqual((ok, lines[1:]), (True, [restored, STRINGS[language]['restart_windows']]))
                self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x12))
                self.assertEqual(c.refresh(), [])
        # 0x11 set before the first load (by mVolt+, for example) is not 0.3.x's: a note when loading, no line.
        for language in ('en', 'es'):
            with self.subTest(language=language):
                fake, key = platform(values={ops.FIRMWARE: (REG_DWORD, 0x11)})
                c = self.controller(fake, language)
                c.choose_file(self.file(rom(RTX_5070)))
                self.assertIn(STRINGS[language]['firmware_gsp_kept'], c.confirm_load().split('\n\n'))
                ok, lines = c.run('load')
                self.assertEqual(lines[1:], [STRINGS[language]['firmware_gsp_kept'],
                                             STRINGS[language]['restart_windows']])
                self.assertEqual(c.refresh(), [])
                self.assertEqual(key.values[ops.FIRMWARE], (REG_DWORD, 0x11))

    def test_worker_errors_are_reported_not_raised(self):
        fake, key = platform()
        c = self.controller(fake)
        c.choose_file(self.file(rom(RTX_5070)))
        fake.admin = False
        self.assertEqual(c.run('unload'), (False, ['Error: administrator rights are required; run this from an '
                                                   'elevated PowerShell']))

        def broken(adapter, write=False):
            raise RuntimeError('boom')
        fake.admin, fake.open_key = True, broken
        self.assertEqual(c.run('unload'), (False, ['Unexpected error: RuntimeError: boom']))
        self.assertEqual(key.calls, [])

    def test_verify_needs_only_a_selected_gpu_and_reports_each_state(self):
        fake, key = platform(admin=False, image=b'card image')
        c = self.controller(fake)
        self.assertTrue(c.can_verify())                 # no file and no administrator rights needed
        ok, lines = c.run('verify')
        self.assertEqual((ok, lines[0]), (True, "GPU 0: no override stored: the driver should be using the card's "
                                                'own VBIOS. If you just removed one, restart Windows first.'))
        self.assertTrue(lines[1].startswith('Running VBIOS: 10 bytes, sha256 '))
        key.values = {ops.VBIOS: (REG_BINARY, b'card image'), 'romOverride00': (REG_BINARY, b'r')}
        ok, lines = c.run('verify')
        self.assertEqual((ok, lines[0]), (True, 'GPU 0: the driver is using the loaded override (verified byte by '
                                                'byte).'))
        self.assertEqual(len(lines), 3)                 # no romOverride note: it is only for no-override
        self.assertTrue(lines[2].startswith('Stored override: 10 bytes, sha256 '))
        key.values[ops.VBIOS] = (REG_BINARY, b'card imagf')
        ok, lines = c.run('verify')
        self.assertFalse(ok)
        self.assertEqual(lines[0], 'GPU 0: the driver is NOT using the stored override. If you just loaded it, '
                                   'restart Windows; otherwise the driver may have rejected it and is running the '
                                   "card's own VBIOS.")
        self.assertEqual(lines[1], 'First difference at offset 0x9.')
        self.assertEqual(key.calls, [])
        c.busy = True
        self.assertFalse(c.can_verify())
        self.assertFalse(self.controller(FakePlatform()).can_verify())

    def test_verify_texts_in_spanish_prefix_unstable_and_errors(self):
        big = bytes(range(256)) * 4001
        fake, key = platform(values={ops.VBIOS: (REG_BINARY, big)}, image=big[:1024000])
        c = self.controller(fake, 'es')
        ok, lines = c.run('verify')
        self.assertEqual(lines[:2], ['GPU 0: los primeros 1024000 bytes de la VBIOS en uso coinciden con el '
                                     'reemplazo cargado; el resto no se puede leer.',
                                     'Solo se compararon los primeros 1024000 bytes: NvAPI entrega como máximo esa '
                                     f'cantidad, y el reemplazo tiene {len(big)}.'])
        self.assertTrue(ok)
        self.assertEqual(self.controller(fake).run('verify')[1][0], 'GPU 0: the first 1024000 bytes the driver is '
                                                                    'running match the loaded override; the rest '
                                                                    'cannot be read back.')
        fake.image = b'\xff' + big[1:1024000]
        ok, lines = c.run('verify')
        self.assertEqual((ok, lines[0], lines[2]),
                         (False, 'GPU 0: el driver NO está usando el reemplazo guardado. Si lo acabas de cargar, '
                                 'reinicia Windows; si no, puede que el driver lo haya rechazado y esté usando la '
                                 'VBIOS de la tarjeta.', 'Primera diferencia en el offset 0x0.'))
        key.values = {'romOverride00': (REG_BINARY, b'r')}
        self.assertEqual(c.run('verify')[1][:2], ['GPU 0: sin reemplazo guardado: el driver debería usar la VBIOS de '
                                                  'la tarjeta. Si acabas de quitar uno, reinicia Windows primero.',
                                                  'También hay registros romOverride (mVolt+) guardados; el driver '
                                                  'puede aplicarlos.'])
        reads = iter([b'a', b'b'])
        fake.vbios_image = lambda adapter: next(reads)
        ok, lines = c.run('verify')
        self.assertEqual((ok, lines[0]), (False, 'GPU 0: dos lecturas de la VBIOS en uso dieron resultados distintos; '
                                                 'inténtalo de nuevo.'))

        def failing(adapter):
            raise NvApiError('this NVIDIA driver has no NvAPI_GPU_GetVbiosImage')
        fake.vbios_image = failing
        self.assertEqual(c.run('verify'), (False, ['Error: this NVIDIA driver has no NvAPI_GPU_GetVbiosImage']))

    def test_window_verify_button(self):
        if not importlib.util.find_spec('tkinter'):
            self.skipTest('no tkinter')
        import tkinter
        try:
            root = tkinter.Tk()
        except tkinter.TclError as exc:                  # no display
            self.skipTest(f'no Tk window: {exc}')
        self.addCleanup(root.destroy)
        root.withdraw()
        from nvidia_vbios.windows_loader import gui
        window = gui.Window(root, Controller(platform(admin=False)[0], 'es', 'dark'))
        self.assertEqual(str(window.verify_button.cget('text')), 'Verificar')
        window.update_controls()
        self.assertTrue(window.verify_button.instate(['disabled']))      # no GPU listed yet
        window.refresh()
        self.assertTrue(window.verify_button.instate(['!disabled']))
        self.assertTrue(window.load_button.instate(['disabled']))
        started = []
        window.start = started.append
        window.verify_button.invoke()
        self.assertEqual(started, ['verify'])
        window.c.busy, window.action = True, 'verify'
        window.update_controls()
        self.assertTrue(window.verify_button.instate(['disabled']))
        self.assertEqual(str(window.working.cget('text')), STRINGS['es']['verifying'])
        window.action = 'load'
        window.update_controls()
        self.assertEqual(str(window.working.cget('text')), 'Trabajando...')
        self.assertFalse(hasattr(window, 'restart_box'))       # no "restart the GPU" checkbox

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
