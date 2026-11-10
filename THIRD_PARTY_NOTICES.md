# Third-party notices

## NVIDIA supported GPU table

The PCI device to architecture table in `src/families.rs` lists IDs from NVIDIA's public
[supported GPU table](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/615.71.09/README.md) of the
open GPU kernel modules (MIT). Device 10de:2501, which the Vix Editor's table takes from pci.ids, is not in it and
is unknown to the loader. No NVIDIA source file is included.

## Windows programs

The released `vbios-loader.exe` and `vbios-loader-gui.exe` are compiled with Rust and the Microsoft Visual C++
toolchain. Besides this project's code (GPL-3.0-or-later, [LICENSE](LICENSE)) they contain:

| Component | License | Texts in `third_party/` |
| --- | --- | --- |
| The Rust standard library (`core`, `alloc`, `std` and the crates it is built from), Rust 1.99.1 | MIT OR Apache-2.0; the Unicode tables in `core` also Unicode-3.0. The notices of every part are in `rust-COPYRIGHT-library.html`. | `rust-COPYRIGHT-library.html`, `rust-LICENSE-MIT.txt`, `rust-LICENSE-APACHE.txt`, `rust-Unicode-3.0.txt` |
| [windows-sys](https://crates.io/crates/windows-sys) 0.61.2 and [windows-link](https://crates.io/crates/windows-link) 0.2.1 (Microsoft's Win32 API declarations, Windows only) | MIT OR Apache-2.0 | `windows-sys.license-mit.txt`, `windows-sys.license-apache-2.0.txt`, `windows-link.license-mit.txt`, `windows-link.license-apache-2.0.txt` |
| The Microsoft C runtime (vcruntime and the Universal C Runtime), linked statically | Microsoft Visual Studio license terms, which allow distributing it inside programs built with Visual C++ | none needed |
| Parts of CPython 3.12's `Lib/argparse.py` and `Lib/textwrap.py`, in `src/cli/argparse.rs` and `src/gui/args.rs`. Changes: translated to Rust and reduced to the parsing, help and error texts of these two programs' options. | PSF-2.0 (Python Software Foundation; its copyright notice and license are in the text) | `python-3.12-LICENSE.txt` (CPython 3.12's license file) |
| `src/pyfmt_table.rs`: tables of character properties generated from the Unicode Character Database 15.0, as CPython 3.12 reads it | Unicode-3.0 | `rust-Unicode-3.0.txt` (the same license text) |

The exes import only Windows system DLLs; Verify also loads the NVIDIA driver's `nvapi64.dll` from System32. The same texts, with [LICENSE](LICENSE) and this file, are in
`third-party-notices.zip`, published with each release. Up to 0.4.x the exes were built with Nuitka and contained
the CPython runtime (and Tcl/Tk in the window program); their notices are in the `third-party-notices.zip` of those
releases.
