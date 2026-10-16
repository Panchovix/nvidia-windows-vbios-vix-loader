# Third-party notices

## NVIDIA supported GPU table

The PCI device to architecture table in `src/nvidia_vbios/families.py` lists IDs from NVIDIA's public
[supported GPU table](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/615.71.09/README.md) of the
open GPU kernel modules (MIT). Its `EDITOR_ONLY` entry (10de:2501, from pci.ids) is treated as unknown by
the loader. No NVIDIA source file is included.

## Windows program

The released `vbios-loader.exe` and `vbios-loader-gui.exe` contain the CPython runtime and the Nuitka
runtime; `vbios-loader-gui.exe` also contains Tcl/Tk. Their license texts are in `third-party-notices.zip`,
published with each release.
