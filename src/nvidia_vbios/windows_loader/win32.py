"""Windows implementation of the device interface (devices.py): SetupAPI, the registry, pnputil and NvAPI.

Importing this module is harmless on other systems; the DLLs load when Win32Platform is created,
nvapi64.dll only when vbios_image reads an image.
"""
from __future__ import annotations
import ctypes
from ctypes import wintypes
import os
import subprocess
from .devices import REG_BINARY, REG_DWORD, REG_SZ, Adapter, parse_instance_id, parse_location, pci_location
from .nvapi import read_vbios_image

DIGCF_PRESENT = 0x2
DICS_FLAG_GLOBAL = 0x1
DIREG_DRV = 0x2
KEY_READ = 0x20019
KEY_SET_VALUE = 0x0002
SPDRP_DEVICEDESC = 0x00
SPDRP_DRIVER = 0x09
SPDRP_LOCATIONINFORMATION = 0x0D
SPDRP_BUSNUMBER = 0x15
SPDRP_ADDRESS = 0x1C
ERROR_FILE_NOT_FOUND = 2
ERROR_MORE_DATA = 234
ERROR_NO_MORE_ITEMS = 259
RESTART_TIMEOUT = 45


class GUID(ctypes.Structure):
    _fields_ = [('Data1', ctypes.c_uint32), ('Data2', ctypes.c_uint16), ('Data3', ctypes.c_uint16),
                ('Data4', ctypes.c_ubyte * 8)]


class SP_DEVINFO_DATA(ctypes.Structure):
    _fields_ = [('cbSize', ctypes.c_uint32), ('ClassGuid', GUID), ('DevInst', ctypes.c_uint32),
                ('Reserved', ctypes.c_size_t)]


# {4d36e968-e325-11ce-bfc1-08002be10318}: the display adapter class.
GUID_DEVCLASS_DISPLAY = GUID(0x4D36E968, 0xE325, 0x11CE, (ctypes.c_ubyte * 8)(0xBF, 0xC1, 0x08, 0x00, 0x2B, 0xE1, 0x03, 0x18))


class RegistryKey:
    def __init__(self, advapi32, handle):
        self._api, self._handle = advapi32, handle

    def _check(self, status, what):
        if status:
            raise OSError(status, f'{what} failed: {ctypes.FormatError(status).strip()}')

    def get(self, name):
        kind, size = wintypes.DWORD(), wintypes.DWORD(0)
        while True:
            buffer = ctypes.create_string_buffer(max(size.value, 1))
            size = wintypes.DWORD(len(buffer))
            status = self._api.RegQueryValueExW(self._handle, name, None, ctypes.byref(kind), buffer, ctypes.byref(size))
            if status == ERROR_FILE_NOT_FOUND:
                return None
            if status != ERROR_MORE_DATA:
                break
        self._check(status, f'reading {name}')
        raw = buffer.raw[:size.value]
        if kind.value == REG_DWORD and len(raw) == 4:
            return REG_DWORD, int.from_bytes(raw, 'little')
        if kind.value == REG_SZ:
            return REG_SZ, raw.decode('utf-16-le', 'replace').split('\0', 1)[0]
        return kind.value, raw

    def set(self, name, kind, value):
        if kind == REG_DWORD:
            raw = int(value).to_bytes(4, 'little')
        elif kind == REG_SZ:
            raw = (value + '\0').encode('utf-16-le')
        else:
            raw = bytes(value)
        self._check(self._api.RegSetValueExW(self._handle, name, 0, kind, raw, len(raw)), f'writing {name}')

    def delete(self, name):
        status = self._api.RegDeleteValueW(self._handle, name)
        if status == ERROR_FILE_NOT_FOUND:
            return False
        self._check(status, f'deleting {name}')
        return True

    def flush(self):
        self._check(self._api.RegFlushKey(self._handle), 'flushing the driver key')

    def close(self):
        if self._handle:
            self._api.RegCloseKey(self._handle)
            self._handle = None


class Win32Platform:
    def __init__(self):
        self.setupapi = ctypes.WinDLL('setupapi', use_last_error=True)
        self.advapi32 = ctypes.WinDLL('advapi32', use_last_error=True)
        self.shell32 = ctypes.WinDLL('shell32')
        s, a = self.setupapi, self.advapi32
        HANDLE, DWORD, PDWORD = wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)
        PDATA = ctypes.POINTER(SP_DEVINFO_DATA)
        s.SetupDiGetClassDevsW.argtypes = [ctypes.POINTER(GUID), wintypes.LPCWSTR, wintypes.HWND, DWORD]
        s.SetupDiGetClassDevsW.restype = HANDLE
        s.SetupDiEnumDeviceInfo.argtypes = [HANDLE, DWORD, PDATA]
        s.SetupDiEnumDeviceInfo.restype = wintypes.BOOL
        s.SetupDiGetDeviceInstanceIdW.argtypes = [HANDLE, PDATA, wintypes.LPWSTR, DWORD, PDWORD]
        s.SetupDiGetDeviceInstanceIdW.restype = wintypes.BOOL
        s.SetupDiGetDeviceRegistryPropertyW.argtypes = [HANDLE, PDATA, DWORD, PDWORD, ctypes.c_void_p, DWORD, PDWORD]
        s.SetupDiGetDeviceRegistryPropertyW.restype = wintypes.BOOL
        s.SetupDiOpenDevRegKey.argtypes = [HANDLE, PDATA, DWORD, DWORD, DWORD, DWORD]
        s.SetupDiOpenDevRegKey.restype = wintypes.HKEY
        s.SetupDiDestroyDeviceInfoList.argtypes = [HANDLE]
        a.RegQueryValueExW.argtypes = [wintypes.HKEY, wintypes.LPCWSTR, PDWORD, PDWORD, ctypes.c_void_p, PDWORD]
        a.RegSetValueExW.argtypes = [wintypes.HKEY, wintypes.LPCWSTR, DWORD, DWORD, ctypes.c_char_p, DWORD]
        a.RegDeleteValueW.argtypes = [wintypes.HKEY, wintypes.LPCWSTR]
        a.RegFlushKey.argtypes = [wintypes.HKEY]
        a.RegCloseKey.argtypes = [wintypes.HKEY]
        for function in (a.RegQueryValueExW, a.RegSetValueExW, a.RegDeleteValueW, a.RegFlushKey, a.RegCloseKey):
            function.restype = wintypes.LONG

    def is_admin(self):
        return bool(self.shell32.IsUserAnAdmin())

    def _devices(self):
        """(info set, SP_DEVINFO_DATA, instance id) of each present NVIDIA display device."""
        info = self.setupapi.SetupDiGetClassDevsW(ctypes.byref(GUID_DEVCLASS_DISPLAY), None, None, DIGCF_PRESENT)
        if info in (None, ctypes.c_void_p(-1).value):
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            index = 0
            while True:
                data = SP_DEVINFO_DATA(cbSize=ctypes.sizeof(SP_DEVINFO_DATA))
                if not self.setupapi.SetupDiEnumDeviceInfo(info, index, ctypes.byref(data)):
                    if ctypes.get_last_error() == ERROR_NO_MORE_ITEMS:
                        return
                    raise ctypes.WinError(ctypes.get_last_error())
                index += 1
                buffer = ctypes.create_unicode_buffer(512)
                if not self.setupapi.SetupDiGetDeviceInstanceIdW(info, ctypes.byref(data), buffer, len(buffer), None):
                    continue
                parsed = parse_instance_id(buffer.value)
                if parsed and parsed[0] == 0x10DE:
                    yield info, data, buffer.value
        finally:
            self.setupapi.SetupDiDestroyDeviceInfoList(info)

    def _property(self, info, data, prop):
        buffer = ctypes.create_unicode_buffer(1024)
        if not self.setupapi.SetupDiGetDeviceRegistryPropertyW(info, ctypes.byref(data), prop, None, buffer,
                                                               ctypes.sizeof(buffer), None):
            return None
        return buffer.value or None

    def _dword_property(self, info, data, prop):
        value = wintypes.DWORD()
        if not self.setupapi.SetupDiGetDeviceRegistryPropertyW(info, ctypes.byref(data), prop, None, ctypes.byref(value),
                                                               ctypes.sizeof(value), None):
            return None
        return value.value

    def _location(self, info, data):
        # The location text is translated ('Bus PCI 1, dispositivo 0, función 0'); the numbers are not.
        return (pci_location(self._dword_property(info, data, SPDRP_BUSNUMBER),
                             self._dword_property(info, data, SPDRP_ADDRESS))
                or parse_location(self._property(info, data, SPDRP_LOCATIONINFORMATION)))

    def _open(self, info, data, write):
        access = KEY_READ | (KEY_SET_VALUE if write else 0)
        handle = self.setupapi.SetupDiOpenDevRegKey(info, ctypes.byref(data), DICS_FLAG_GLOBAL, 0, DIREG_DRV, access)
        if handle in (None, ctypes.c_void_p(-1).value):
            raise ctypes.WinError(ctypes.get_last_error())
        return RegistryKey(self.advapi32, handle)

    def adapters(self):
        found = []
        for info, data, instance_id in self._devices():
            vendor, device, subsystem = parse_instance_id(instance_id)
            name, version = self._property(info, data, SPDRP_DEVICEDESC), None
            try:
                key = self._open(info, data, False)
            except OSError:
                key = None
            if key is not None:
                try:
                    value = key.get('DriverDesc')
                    name = value[1] if value and value[0] == REG_SZ else name
                    value = key.get('DriverVersion')
                    version = value[1] if value and value[0] == REG_SZ else None
                finally:
                    key.close()
            found.append(dict(name=name or 'NVIDIA display adapter', vendor=vendor, device=device,
                              subsystem=subsystem, instance_id=instance_id, driver_version=version,
                              location=self._location(info, data),
                              driver_key=self._property(info, data, SPDRP_DRIVER)))
        found.sort(key=lambda a: (a['location'] is None, a['location'] or '', a['instance_id']))
        return [Adapter(index=i, **fields) for i, fields in enumerate(found)]

    def open_key(self, adapter, write=False):
        for info, data, instance_id in self._devices():
            if instance_id == adapter.instance_id:
                return self._open(info, data, write)
        raise OSError(f'device {adapter.instance_id} is no longer present')

    def vbios_image(self, adapter):
        return read_vbios_image(adapter.location)

    def restart_device(self, instance_id):
        pnputil = os.path.join(os.environ.get('SystemRoot', r'C:\Windows'), 'System32', 'pnputil.exe')
        try:
            result = subprocess.run([pnputil, '/restart-device', instance_id], capture_output=True, text=True,
                                    errors='replace', timeout=RESTART_TIMEOUT,
                                    creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        except (OSError, subprocess.TimeoutExpired) as exc:
            return None, str(exc)
        return result.returncode, (result.stdout + result.stderr).strip()
