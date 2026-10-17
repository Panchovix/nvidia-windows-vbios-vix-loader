"""NvAPI (nvapi64.dll): read back the VBIOS image that the driver's GSP firmware is running.

NvAPI_GPU_GetVbiosImage returns the firmware's working copy: the override when the driver accepted it,
otherwise the card's own image. nvapi64.dll loads only when an image is read, so importing this module is
harmless on any system. Every function comes from nvapi_QueryInterface(id), which returns NULL for a function
that this driver does not have; tests pass their own query function.
"""
from __future__ import annotations
import ctypes
import os

IMAGE_SIZE = 0xFA000            # the only buffer size the driver accepts; an image this long may be cut short
IMAGE_VERSION = 0x00020010      # NV_GPU_VBIOS_IMAGE version 2, 16 bytes
MAX_PHYSICAL_GPUS = 64
OK, NO_IMPLEMENTATION = 0, -3
SHORT_STRING = 64               # NvAPI_ShortString


class NvApiError(OSError):
    pass


class NV_GPU_VBIOS_IMAGE(ctypes.Structure):
    _fields_ = [('version', ctypes.c_uint32), ('size', ctypes.c_uint32), ('buffer', ctypes.c_void_p)]


_U32 = ctypes.POINTER(ctypes.c_uint32)
FUNCTIONS = {
    'Initialize': (0x0150E828, ctypes.CFUNCTYPE(ctypes.c_int)),
    'Unload': (0xD22BDD7E, ctypes.CFUNCTYPE(ctypes.c_int)),
    'GetErrorMessage': (0x6C2D048C, ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_char))),
    'EnumPhysicalGPUs': (0xE5AC921F, ctypes.CFUNCTYPE(ctypes.c_int, ctypes.POINTER(ctypes.c_void_p), _U32)),
    'GPU_GetBusId': (0x1BE0B8E5, ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, _U32)),
    'GPU_GetBusSlotId': (0x2A0A350F, ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, _U32)),
    'GPU_GetVbiosImage': (0xFC13EE11, ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p,
                                                       ctypes.POINTER(NV_GPU_VBIOS_IMAGE))),
}


def query_interface():
    """nvapi_QueryInterface of the nvapi64.dll that the NVIDIA driver installs in System32 (by full path)."""
    path = os.path.join(os.environ.get('SystemRoot', r'C:\Windows'), 'System32', 'nvapi64.dll')
    try:
        query = ctypes.WinDLL(path).nvapi_QueryInterface
    except (AttributeError, OSError) as exc:
        raise NvApiError(f'cannot load nvapi64.dll ({exc}); is the NVIDIA driver installed?') from None
    query.restype, query.argtypes = ctypes.c_void_p, [ctypes.c_uint32]
    return query


def _missing(name):
    return NvApiError(f'this NVIDIA driver has no NvAPI_{name}; reading the running VBIOS needs a driver that '
                      'has it, for example 617.42')


class NvApi:
    def __init__(self, query):
        self.query = query

    def function(self, name):
        function_id, prototype = FUNCTIONS[name]
        address = self.query(function_id)
        if not address:
            raise _missing(name)
        return prototype(address)

    def call(self, name, *args):
        status = self.function(name)(*args)
        if status == NO_IMPLEMENTATION:
            raise _missing(name)
        if status != OK:
            raise NvApiError(f'NvAPI_{name} failed: {self.describe(status)}')

    def describe(self, status):
        text = ctypes.create_string_buffer(SHORT_STRING)
        try:
            if self.function('GetErrorMessage')(status, text) == OK and text.value:
                return f"{text.value.decode('ascii', 'replace')} ({status})"
        except NvApiError:
            pass
        return f'status {status}'

    def find(self, location):
        """The physical GPU handle at PCI location 'bb:dd.f': NvAPI gives the bus and the device (slot) number."""
        if location is None:
            raise NvApiError('the PCI location of this GPU is unknown, so it cannot be found through NvAPI')
        bus, device = int(location[:2], 16), int(location[3:5], 16)
        handles, count = (ctypes.c_void_p * MAX_PHYSICAL_GPUS)(), ctypes.c_uint32()
        self.call('EnumPhysicalGPUs', handles, ctypes.byref(count))
        found = []
        for handle in handles[:min(count.value, MAX_PHYSICAL_GPUS)]:
            ids = ctypes.c_uint32(), ctypes.c_uint32()
            if self.function('GPU_GetBusId')(handle, ctypes.byref(ids[0])) == OK and \
                    self.function('GPU_GetBusSlotId')(handle, ctypes.byref(ids[1])) == OK:
                found.append((handle, ids[0].value, ids[1].value))
        matches = [handle for handle, b, d in found if (b, d) == (bus, device)]
        if len(matches) != 1:
            listed = ', '.join(f'{b:02x}:{d:02x}' for _, b, d in found) or 'none'
            raise NvApiError(f'NvAPI does not list exactly one GPU at PCI {location[:5]} (it lists: {listed})')
        return matches[0]

    def image(self, handle):
        buffer = ctypes.create_string_buffer(IMAGE_SIZE)
        request = NV_GPU_VBIOS_IMAGE(IMAGE_VERSION, IMAGE_SIZE, ctypes.addressof(buffer))
        self.call('GPU_GetVbiosImage', handle, ctypes.byref(request))
        if not 0 < request.size <= IMAGE_SIZE:
            raise NvApiError(f'NvAPI_GPU_GetVbiosImage returned an invalid size ({request.size} bytes)')
        return buffer.raw[:request.size]


def read_vbios_image(location, query=None):
    """The VBIOS image the driver is running on the GPU at PCI location 'bb:dd.f' (Adapter.location)."""
    api = NvApi(query or query_interface())
    api.call('Initialize')
    try:
        return api.image(api.find(location))
    finally:
        try:
            api.function('Unload')()
        except NvApiError:
            pass
