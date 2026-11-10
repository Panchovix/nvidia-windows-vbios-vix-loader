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

The exes use only Windows system DLLs at run time. The same texts, with [LICENSE](LICENSE) and this file, are in
`third-party-notices.zip`, published with each release. Up to 0.4.x the exes were built with Nuitka and contained
the CPython runtime (and Tcl/Tk in the window program); their notices are in the `third-party-notices.zip` of those
releases.
