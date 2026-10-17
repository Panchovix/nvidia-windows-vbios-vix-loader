"""State of the loader window: selection, validation, button enablement and texts. No Tk here.

Everything goes through operations.py: the list comes from status(), a file is checked with
load(dry_run=True), and Load / Remove override run load() and unload(). gui.py only draws.
"""
from __future__ import annotations
import ctypes
import os
from .cli import step_text
from .operations import MIN_DRIVER, LoaderError, load, status, unload

MINIMUM = f'{MIN_DRIVER[0]}.{MIN_DRIVER[1]:02d}'

STRINGS = {
    'en': {
        'title': 'Windows VBIOS Vix Loader',
        'not_admin': 'Not running as administrator: you can see the status, but Load and Remove override are '
                     'disabled. Close this window and start the program as administrator.',
        'gpus': 'NVIDIA GPUs',
        'col_index': '#', 'col_name': 'Name', 'col_pci': 'PCI ID', 'col_location': 'Location',
        'col_driver': 'Driver', 'col_override': 'Override',
        'refresh': 'Refresh',
        'file': 'VBIOS file',
        'file_button': 'VBIOS file...',
        'file_types': 'VBIOS files',
        'all_files': 'All files',
        'restart': 'Restart the GPU now',
        'load': 'Load',
        'remove': 'Remove override',
        'messages': 'Messages',
        'working': 'Working... restarting the GPU can take up to 45 seconds.',
        'override_none': 'none',
        'override_loaded': 'loaded: {size} bytes, sha256 {sha}...',
        'override_other': "other tool's override: {size} bytes",
        'override_bad': 'vbios value of unexpected type {type}',
        'override_records': 'romOverride records (mVolt+): {count}',
        'override_backup': 'saved EnableGpuFirmware value only',
        'override_error': 'cannot read the driver key',
        'driver_unknown': 'unknown',
        'driver_too_old': '{version} (too old, {minimum} or newer needed)',
        'no_gpu': 'No NVIDIA GPU found.',
        'choose_gpu': 'Select a GPU.',
        'choose_file': 'Choose a VBIOS file.',
        'cannot_read': 'Cannot read {path}: {error}',
        'valid': 'OK for GPU {index}: {family}, {size} bytes to store, sha256 {sha256}.',
        'refused': 'Refused: {reason}',
        'confirm_load_title': 'Load VBIOS override',
        'confirm_load': 'Load this VBIOS on GPU {index} ({name}, PCI {location})?\n\n'
                        'File: {path}\nStored: {size} bytes of the image chain ({family}), sha256 {sha256}\n\n'
                        'Registry changes in the GPU\'s driver key:\n{steps}',
        'confirm_remove_title': 'Remove VBIOS override',
        'confirm_remove': 'Remove the override from GPU {index} ({name}, PCI {location})?\n\n'
                          'Registry changes in the GPU\'s driver key:\n{steps}',
        'other_tool': 'This GPU has an override written by another tool (for example mVolt+). It is replaced and '
                      'will not be restored.',
        'will_restart': 'The GPU is restarted afterwards; the screens on it can go black for a few seconds.',
        'no_restart': 'The GPU is not restarted: restart Windows to apply the change.',
        'risk': 'A modified VBIOS can damage hardware. If the GPU does not start, see "If the GPU does not start" '
                'in the README: boot into Safe Mode and remove the override.',
        'loaded': 'GPU {index}: VBIOS loaded, {size} bytes, sha256 {sha256}.',
        'removed': 'GPU {index}: override removed.',
        'restarted': 'GPU restarted.',
        'not_restarted': 'GPU not restarted. Restart Windows to apply the change.',
        'restart_failed': 'The values are written, but the GPU restart failed ({detail}). Restart Windows to '
                          'apply the change.',
        'pnputil_code': 'pnputil exit code {code}',
        'pnputil_timeout': 'pnputil did not finish',
        'note': 'Note: {message}',
        'error': 'Error: {message}',
        'unexpected': 'Unexpected error: {message}',
        'status_failed': 'Cannot list the GPUs: {message}',
        'hint_remove': 'In this window: click Refresh, then Remove override, before restarting Windows.',
        'hint_remove_first': 'In this window: use Remove override first.',
        'hint_refresh': 'In this window: click Refresh and select the GPU again.',
        'language': 'Language',
    },
    'es': {
        'title': 'Windows VBIOS Vix Loader',
        'not_admin': 'No se está ejecutando como administrador: puedes ver el estado, pero Cargar y Quitar '
                     'reemplazo están desactivados. Cierra esta ventana y abre el programa como administrador.',
        'gpus': 'GPUs NVIDIA',
        'col_index': '#', 'col_name': 'Nombre', 'col_pci': 'ID PCI', 'col_location': 'Ubicación',
        'col_driver': 'Driver', 'col_override': 'Reemplazo',
        'refresh': 'Actualizar',
        'file': 'Archivo VBIOS',
        'file_button': 'Archivo VBIOS...',
        'file_types': 'Archivos VBIOS',
        'all_files': 'Todos los archivos',
        'restart': 'Reiniciar la GPU ahora',
        'load': 'Cargar',
        'remove': 'Quitar reemplazo',
        'messages': 'Mensajes',
        'working': 'Trabajando... reiniciar la GPU puede tardar hasta 45 segundos.',
        'override_none': 'ninguno',
        'override_loaded': 'cargado: {size} bytes, sha256 {sha}...',
        'override_other': 'reemplazo de otra herramienta: {size} bytes',
        'override_bad': 'valor vbios de tipo inesperado {type}',
        'override_records': 'registros romOverride (mVolt+): {count}',
        'override_backup': 'solo el valor guardado de EnableGpuFirmware',
        'override_error': 'no se puede leer la clave del driver',
        'driver_unknown': 'desconocido',
        'driver_too_old': '{version} (muy antiguo, se necesita {minimum} o más nuevo)',
        'no_gpu': 'No se encontró ninguna GPU NVIDIA.',
        'choose_gpu': 'Elige una GPU.',
        'choose_file': 'Elige un archivo VBIOS.',
        'cannot_read': 'No se puede leer {path}: {error}',
        'valid': 'Válido para la GPU {index}: {family}, {size} bytes a guardar, sha256 {sha256}.',
        'refused': 'Rechazado: {reason}',
        'confirm_load_title': 'Cargar VBIOS de reemplazo',
        'confirm_load': '¿Cargar esta VBIOS en la GPU {index} ({name}, PCI {location})?\n\n'
                        'Archivo: {path}\nSe guardan: {size} bytes de la cadena de imágenes ({family}), '
                        'sha256 {sha256}\n\nCambios en la clave de registro del driver de la GPU:\n{steps}',
        'confirm_remove_title': 'Quitar VBIOS de reemplazo',
        'confirm_remove': '¿Quitar el reemplazo de la GPU {index} ({name}, PCI {location})?\n\n'
                          'Cambios en la clave de registro del driver de la GPU:\n{steps}',
        'other_tool': 'Esta GPU tiene un reemplazo escrito por otra herramienta (por ejemplo mVolt+). Se reemplaza '
                      'y no se va a restaurar.',
        'will_restart': 'Después se reinicia la GPU; las pantallas conectadas a ella pueden quedar en negro unos '
                        'segundos.',
        'no_restart': 'La GPU no se reinicia: reinicia Windows para aplicar el cambio.',
        'risk': 'Una VBIOS modificada puede dañar el hardware. Si la GPU no arranca, revisa "Si la GPU no arranca" '
                'en el README: arranca en Modo seguro y quita el reemplazo.',
        'loaded': 'GPU {index}: VBIOS cargada, {size} bytes, sha256 {sha256}.',
        'removed': 'GPU {index}: reemplazo quitado.',
        'restarted': 'GPU reiniciada.',
        'not_restarted': 'GPU sin reiniciar. Reinicia Windows para aplicar el cambio.',
        'restart_failed': 'Los valores quedaron escritos, pero falló el reinicio de la GPU ({detail}). Reinicia '
                          'Windows para aplicar el cambio.',
        'pnputil_code': 'pnputil terminó con código {code}',
        'pnputil_timeout': 'pnputil no terminó',
        'note': 'Nota: {message}',
        'error': 'Error: {message}',
        'unexpected': 'Error inesperado: {message}',
        'status_failed': 'No se pueden listar las GPUs: {message}',
        'hint_remove': 'En esta ventana: pulsa Actualizar y luego Quitar reemplazo antes de reiniciar Windows.',
        'hint_remove_first': 'En esta ventana: usa primero Quitar reemplazo.',
        'hint_refresh': 'En esta ventana: pulsa Actualizar y vuelve a elegir la GPU.',
        'language': 'Idioma',
    },
}

# operations.py names command-line steps in its errors; the window adds what to do here instead.
HINTS = (('registry may be partly changed', 'hint_remove'), ('read back differ', 'hint_remove'),
         ('run unload first', 'hint_remove_first'), ('run status to list them', 'hint_refresh'))


def language_for(langid):
    """'es' when the primary language of a Windows LANGID is Spanish (0x0a), else 'en'."""
    return 'es' if langid & 0x3FF == 0x0A else 'en'


def system_language():
    if os.name != 'nt':
        return 'en'
    return language_for(ctypes.windll.kernel32.GetUserDefaultUILanguage())


# Each language is named in itself in the Language menu.
LANGUAGES = (('en', 'English'), ('es', 'Español'))


class Controller:
    def __init__(self, platform, language='en'):
        self.platform, self.language, self.text = platform, language, STRINGS[language]
        self.gpus, self.admin = [], False
        self.selected = None          # instance id of the selected GPU
        self.path = self.data = None
        self.file_error = None
        self.validation = None        # ('ok', dry-run report) or ('error', reason); None until both are chosen
        self.restart = True
        self.busy = False

    # -- state --------------------------------------------------------------------------------------------
    def refresh(self):
        """Read status and admin rights again; returns message lines."""
        lines = []
        try:
            self.admin = bool(self.platform.is_admin())
            self.gpus = status(self.platform)
        except (LoaderError, OSError) as exc:
            self.gpus = []
            lines.append(self.text['status_failed'].format(message=exc))
        if self.selected is None and self.gpus:     # only before any choice: a GPU that went away stays chosen
            self.selected = self.gpus[0]['instance_id']
        self.validate()
        return lines

    def set_language(self, language):
        """Switch the texts; the file is checked again so its refusal reads in the new language."""
        if self.busy or language == self.language:
            return
        self.language, self.text = language, STRINGS[language]
        if self.path is not None:
            self.choose_file(self.path)
        else:
            self.validate()

    @property
    def gpu(self):
        return next((gpu for gpu in self.gpus if gpu['instance_id'] == self.selected), None)

    def select(self, index):
        if self.busy:
            return
        chosen = next((gpu['instance_id'] for gpu in self.gpus if gpu['index'] == index), None)
        if chosen != self.selected:
            self.selected = chosen
            self.validate()

    def choose_file(self, path):
        self.path, self.data, self.file_error = path, None, None
        try:
            with open(path, 'rb') as stream:
                self.data = stream.read()
        except OSError as exc:
            self.file_error = self.text['cannot_read'].format(path=path, error=exc.strerror or exc)
        self.validate()

    def validate(self):
        """The same checks as `load --dry-run`, on the selected GPU and the chosen file."""
        self.validation = None
        if self.file_error:
            self.validation = ('error', self.file_error)
        elif self.gpu is not None and self.data is not None:
            try:
                self.validation = ('ok', load(self.platform, self.data, instance_id=self.selected, dry_run=True))
            except (LoaderError, OSError) as exc:
                self.validation = ('error', self._reason(exc))

    def _reason(self, exc):
        text = str(exc)
        hint = next((self.text[key] for phrase, key in HINTS if phrase in text), None)
        return f'{text} {hint}' if hint else text

    # -- what the window shows ------------------------------------------------------------------------------
    def override_text(self, gpu):
        t = self.text
        if gpu.get('error'):
            return t['override_error']
        override, parts = gpu['override'], []
        vbios, saved = override['vbios'], override['saved_previous_enable_gpu_firmware']
        if vbios is not None:
            if vbios['size'] is None:
                parts.append(t['override_bad'].format(type=vbios['type']))
            elif saved is not None:
                parts.append(t['override_loaded'].format(size=vbios['size'], sha=vbios['sha256'][:12]))
            else:
                parts.append(t['override_other'].format(size=vbios['size']))
        if override['rom_override_records']:
            parts.append(t['override_records'].format(count=len(override['rom_override_records'])))
        if not parts and saved is not None:
            parts.append(t['override_backup'])
        return '; '.join(parts) or t['override_none']

    def driver_text(self, gpu):
        version = gpu['nvidia_driver_version']
        if version is None:
            return self.text['driver_unknown']
        return version if gpu['driver_supported'] else \
            self.text['driver_too_old'].format(version=version, minimum=MINIMUM)

    def rows(self):
        """(index, name, PCI ID, location, driver, override) per GPU."""
        return [(gpu['index'], gpu['name'], gpu['pci_id'], gpu['location'] or '?', self.driver_text(gpu),
                 self.override_text(gpu)) for gpu in self.gpus]

    def validation_text(self):
        """(ok: bool, text) for the line under the file."""
        t = self.text
        if not self.gpus:
            return False, t['no_gpu']
        if self.gpu is None:
            return False, t['choose_gpu']
        if self.validation is None:
            return False, t['choose_file']
        kind, value = self.validation
        if kind == 'error':
            return False, t['refused'].format(reason=value)
        payload = value['payload']
        return True, t['valid'].format(index=value['gpu']['index'], family=payload['family'], size=payload['size'],
                                       sha256=payload['sha256'])

    def can_load(self):
        return self.admin and not self.busy and self.validation is not None and self.validation[0] == 'ok'

    def can_remove(self):
        gpu = self.gpu
        if not self.admin or self.busy or gpu is None or gpu.get('error'):
            return False
        override = gpu['override']
        return bool(override['vbios'] is not None or override['rom_override_records']
                    or override['saved_previous_enable_gpu_firmware'] is not None)

    def _restart_note(self):
        return self.text['will_restart'] if self.restart else self.text['no_restart']

    def _steps(self, report):
        return '\n'.join('  ' + step_text(step) for step in report['steps'])

    def confirm_load(self):
        """The question asked before Load, from a fresh dry run; None when Load is not possible now."""
        self.validate()
        if not self.can_load():
            return None
        report, t = self.validation[1], self.text
        gpu = report['gpu']
        payload = report['payload']
        parts = [t['confirm_load'].format(index=gpu['index'], name=gpu['name'], location=gpu['location'] or '?',
                                          path=self.path, size=payload['size'], family=payload['family'],
                                          sha256=payload['sha256'], steps=self._steps(report))]
        if report.get('message'):
            parts.append(t['other_tool'])
        parts += [self._restart_note(), t['risk']]
        return '\n\n'.join(parts)

    def confirm_remove(self):
        """(question, None) before Remove override, or (None, error line) when the dry run is refused."""
        t = self.text
        try:
            report = unload(self.platform, instance_id=self.selected, dry_run=True)
        except (LoaderError, OSError) as exc:
            return None, t['error'].format(message=self._reason(exc))
        gpu = report['gpu']
        parts = [t['confirm_remove'].format(index=gpu['index'], name=gpu['name'], location=gpu['location'] or '?',
                                            steps=self._steps(report))]
        if report.get('message'):
            parts.append(t['note'].format(message=report['message']))
        parts.append(self._restart_note())
        return '\n\n'.join(parts), None

    # -- actions (run in a worker thread while busy) ------------------------------------------------------------
    def run(self, action):
        """Load or remove on the selected GPU; returns (ok, message lines). Never raises."""
        try:
            return self._run(action)
        except (LoaderError, OSError) as exc:
            return False, [self.text['error'].format(message=self._reason(exc))]
        except Exception as exc:  # the window must survive anything
            return False, [self.text['unexpected'].format(message=f'{type(exc).__name__}: {exc}')]

    def _run(self, action):
        t = self.text
        if action == 'load':
            report = load(self.platform, self.data, instance_id=self.selected, restart=self.restart)
        else:
            report = unload(self.platform, instance_id=self.selected, restart=self.restart)
        index, lines = report['gpu']['index'], []
        if action == 'load':
            payload = report['payload']
            lines.append(t['loaded'].format(index=index, size=payload['size'], sha256=payload['sha256']))
            if report.get('message'):
                lines.append(t['other_tool'])
        else:
            if report['steps']:
                lines.append(t['removed'].format(index=index))
            if report.get('message'):
                lines.append(t['note'].format(message=report['message']))
        restart = report.get('restart')
        if restart is None:
            return True, lines
        if not restart['requested']:
            lines.append(t['not_restarted'])
        elif restart['ok']:
            lines.append(t['restarted'])
        else:
            code = restart.get('exit_code')
            detail = t['pnputil_timeout'] if code is None else t['pnputil_code'].format(code=code)
            lines.append(t['restart_failed'].format(detail=detail))
            if restart.get('output'):
                lines.append(restart['output'])
            return False, lines
        return True, lines
