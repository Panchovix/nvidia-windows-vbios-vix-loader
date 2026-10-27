# NVIDIA Windows VBIOS Vix Loader

English | [Español](README_es.md)

Loads a modified VBIOS for an NVIDIA GPU on Windows **without flashing it**. The image is stored in the
GPU's driver registry key, where the NVIDIA driver looks for a VBIOS override when the device starts, and
the device is restarted. `verify` checks that the driver is running it, and `unload` removes it again. The
card's flash chip is never written.

Works with NVIDIA 616.x, 617.x and 621.x drivers (616.56 or newer; tested on 617.42, 617.70 and 621.01). It may
stop working in a future driver if NVIDIA decides to change or remove the registry override, so click **Verify**
after every driver update.

Thanks to **@pantsoftime** ([overclock.net](https://www.overclock.net)), whose original `vbios_override` work
this project family is built on. Without it, none of this would have been possible.

To modify the VBIOS, use [NVIDIA VBIOS Vix Editor](https://github.com/Panchovix/nvidia-vbios-vix-editor).
For Linux, see [NVIDIA Linux VBIOS Vix Loader](https://github.com/Panchovix/nvidia-linux-vbios-vix-loader).
This repository only loads the VBIOS on Windows.

This project is not affiliated with or endorsed by NVIDIA. NVIDIA and GeForce are trademarks of NVIDIA
Corporation.

> **Warning.** A modified VBIOS can raise power, clocks and voltages beyond the card's design limits. You can
> damage hardware. Keep a copy of your original ROM and use this at your own risk.

## What it does

`load` writes these values in the GPU's driver key,
`HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\NNNN`:

| Value | Type | Content |
| --- | --- | --- |
| `vbios` | REG_BINARY | The VBIOS image chain from your file: from the legacy image to the end of the last image. Bytes before and after it in the file are not stored. |
| `EnableGpuFirmware` | REG_DWORD | `0x11` (GSP firmware on, fallback allowed). |
| `VixLoaderPreviousEnableGpuFirmware` | REG_SZ | The previous `EnableGpuFirmware` value, saved on the first `load` only so `unload` can restore it: `absent`, a DWORD such as `0x00000012`, or, if it was not a 4-byte REG_DWORD (another tool or a hand edit can leave a REG_SZ, REG_QWORD, REG_BINARY or a DWORD of another size there), an exact copy of its type and bytes written the way a `.reg` file writes them, for example `hex(1):31,00,00,00`. |

It also deletes `romOverride00` to `romOverrideFF` if present, flushes the key, reads the values back and
restarts the GPU with `pnputil /restart-device`. If the restart fails, restart Windows.

These are the same `vbios` and `EnableGpuFirmware` values that mVolt+ writes when it loads a BIOS file, so a
BIOS loaded with mVolt+ on the same GPU is replaced. This project is not affiliated with mVolt+. Other values
in the key are left alone.

`unload` deletes `vbios` and any `romOverride` records, restores `EnableGpuFirmware` exactly as it was, with
the same type (or deletes it if it was absent), deletes `VixLoaderPreviousEnableGpuFirmware` and restarts the
GPU. If the saved value starts with `hex(`, remove the override with version 0.3.2 or newer: older versions
don't read that form, so they delete it and leave `EnableGpuFirmware` at `0x11`.

It does **not** flash the card, edit the VBIOS, or change clocks, fans or power limits by itself. `unload`
returns the key to its state before the first `load`, except that an override written by another tool
(mVolt+ `vbios` or `romOverride` records) is not restored; in that case `load` says so, and `unload` leaves
`EnableGpuFirmware` as that tool set it.

## Antivirus warnings

Some antivirus programs and online scanners flag the two exes (for example generic or "ML" detections, or
"Python DLL sideloading"). These are false positives: each exe is a Python program packed with Nuitka
`--onefile`, which unpacks the Python runtime into a temporary folder when it starts, and the exes are not
code-signed. The loader does not use the network. The exes are built by GitHub Actions from this repository
(see [Tests](#tests)); compare the download with `SHA256SUMS` to confirm that it matches the published asset.

I've also submitted the binaries to Microsoft so they stop being flagged. If you still don't want to run the
exes from the releases, clone the repository and run the loader from source (see the end of [Usage](#usage)),
or build the exes yourself the same way the workflow does (see [Tests](#tests)).

And lastly, if you don't want to use this loader for an edited VBIOS, you can use the BIOS override of mVolt+
(https://github.com/b00nz/mVolt) instead. Issues related to mVolt+ go there, not to this repo. Keep in mind
that its BIOS override works mostly on Blackwell.

## Status

| GPU family | State |
| --- | --- |
| Blackwell GB20x (GeForce RTX 50, RTX PRO Blackwell) | Uses the same registry values that mVolt+ uses to load BIOS files on Blackwell. Tested on an RTX 5070 (driver 617.42): an edited VBIOS loaded and worked after a GPU restart, without restarting Windows. |
| Ada AD10x (GeForce RTX 40, RTX Ada) | Tested on an RTX 4090 (VBIOS 95.02.18, driver 617.42) with an `ada-relocated-v2` payload: a raised power limit (671 W) and core clock offset range (+1200 MHz) applied. Other Ada GPUs are untested. |
| Ampere GA102–GA107 (GeForce RTX 30, RTX A series) | Tested on an RTX A2000 (driver 617.42) with an `ampere-relocated-v1` payload: a lowered power floor (5 W) applied. On an RTX 3050 6GB Laptop GPU (driver 617.42) a lowered power ceiling (70 W) applied; a raised one is untested on laptops. Other Ampere GPUs are untested. GA100 (A100, A30, A800) is not supported. |
| Turing TU102–TU117 (GeForce RTX 20, GTX 16, TITAN RTX, Quadro RTX, T series) | **Experimental.** Tested on an RTX 2080 Ti (ROG Matrix BIOS, driver 617.70): with the board and connector power raised, the card reached about 458 W under load. An RTX 2070 reached 225 W with its power limit raised from 200 W, and an RTX 2070 SUPER also ran an edited image. Other Turing GPUs are untested. If the override is refused, the card runs its stock VBIOS. See the [Turing tester guide](https://github.com/Panchovix/nvidia-vbios-vix-editor/blob/main/docs/turing.md). |

If you try it, please open an issue with the GPU, driver version and result.

## Requirements

- Windows 10 or 11, 64-bit.
- An administrator account. The window program asks for administrator rights when it starts; run the
  command-line tool from an elevated PowerShell ("Run as administrator").
- NVIDIA driver **616.x, 617.x or 621.x**, 616.56 or newer (`status` shows the version).
- A payload file for that exact GPU model (same PCI device ID).

## Which file to load

The file must contain exactly one VBIOS for this GPU's PCI ID with a valid checksum. Full flash dumps with
two banks are refused: export a single-chain payload with Vix Editor's command-line tool (`nvbios.exe`).

With Vix Editor 0.1.11 or newer, saving an edited Ada or Ampere copy also saves `<name>-loader.bin` next to
it: load that file. The commands below do the same from the command line.

- **Blackwell**: the ROM saved by Vix Editor after editing, or any edited single-bank dump (for example from
  GPU-Z). If it is refused because of two banks, run `.\nvbios.exe normalize edited.rom payload.bin`.
- **Ada**: the GSP firmware loads at most 512 KiB counted from the legacy image, so the payload must be
  relocated: `.\nvbios.exe normalize edited.rom payload.bin --representation ada-relocated-v2`. Larger
  payloads are refused.
- **Ampere GA102–GA107**: the same 512 KiB limit. Use
  `.\nvbios.exe normalize edited.rom payload.bin --representation ampere-relocated-v1`.
- **Turing TU102–TU117** (experimental): the payload fits without relocation:
  `.\nvbios.exe normalize edited.rom payload.bin`. Vix Editor 0.1.18 or newer saves it as `<name>-loader.bin`, plus
  `<name>-loader-e0end.bin` (`--representation turing-e0-end-v1`) to try if the first loads but the edit
  does not apply. Report the **Verify** result and `nvidia-smi -q -d POWER` before and after.

## Usage

### Window

1. Download `vbios-loader-gui.exe` from the releases and check it against `SHA256SUMS`.
2. Run it. Windows asks for administrator rights (it writes to the GPU's registry key).
3. Select the GPU in the list. The list shows each NVIDIA GPU with its PCI location, driver version and
   override state.
4. Click **VBIOS file...** and choose the file. The window checks it for the selected GPU right away and
   shows either OK (family, bytes to store, sha256) or why it is refused.
5. Click **Load** (the same as `load`), read the confirmation (GPU, file, registry changes) and accept. With
   **Restart the GPU now** on, the GPU is restarted (this can take up to 45 seconds); with it off, restart
   Windows afterwards. If this GPU drives your screen, turn **Restart the GPU now** off and restart Windows
   instead: restarting the GPU that drives the screen can leave it with no signal (seen on an RTX 2070 SUPER,
   where the same override then worked after a full restart).
6. To check that the driver is running the loaded VBIOS, click **Verify** (the same as `verify`, see
   [Verify](#verify)).
7. To go back to the card's own VBIOS, select the GPU and click **Remove override** (the same as
   `unload`).

Every result and warning is also written in the **Messages** area. The window is in Spanish when Windows
is; the **Language** menu, or `vbios-loader-gui.exe --lang en` / `--lang es`, changes it. It is dark when
Windows uses the dark app mode; the **Theme** menu, or `--theme light` / `--theme dark`, changes it.

### Command line

Download `vbios-loader.exe` from the releases, check it against `SHA256SUMS`, and open an elevated PowerShell
in its folder.

```powershell
# List NVIDIA GPUs: index, PCI location, driver version and override state.
.\vbios-loader.exe status

# Show what would be written, without writing anything.
.\vbios-loader.exe load .\payload.bin --gpu 0 --dry-run

# Store the override and restart the GPU.
.\vbios-loader.exe load .\payload.bin --gpu 0

# Check that the driver is running the stored override.
.\vbios-loader.exe verify --gpu 0

# Remove it and restart the GPU.
.\vbios-loader.exe unload --gpu 0
```

- Select the GPU with `--gpu INDEX` (as shown by `status`) or `--pci BUS:DEV.FN` (for example `--pci 01:00.0`).
  With a single NVIDIA GPU the selection can be omitted.
- `--no-restart` writes the values without restarting the GPU; they apply at the next restart.
- `--json` prints machine-readable output.
- Exit codes: `0` done, `1` refused or failed (nothing written unless the message says so), `2` invalid
  command-line arguments (nothing written), `3` values written but the GPU restart failed (restart Windows),
  `4` `verify` found that the driver is not running the stored override (or its two reads differed), `70`
  unexpected internal error, `130` interrupted (Ctrl+C).

The override stays in the GPU's registry key until you remove it, so it applies again at every Windows
start. A clean driver installation or a driver update can recreate the driver key and drop the override. Run
`status` after updating and `load` again if needed.

From source, with Python 3.11 or newer: `pip install .` then `vbios-loader status` or `vbios-loader-gui`, or
`python -m nvidia_vbios.windows_loader status` / `python -m nvidia_vbios.windows_loader.gui` with `src` on
`PYTHONPATH`.

## Verify

**Verify** in the window, or `verify` on the command line, reads back through NvAPI the VBIOS image that the
driver's GSP firmware is running, twice, and compares it byte by byte with the override stored in the
registry. It writes nothing and needs no administrator rights. The result (`state` in `--json`) is one of:

- `override-active`: the running image is the stored override, so the driver accepted it.
- `override-not-active`: they differ, and the first differing offset is shown. Right after a load without a
  GPU restart this is expected; otherwise the driver probably rejected the override and is running the
  card's own VBIOS.
- `no-override`: nothing is stored, so the driver should be running the card's own VBIOS (its size and
  sha256 are shown). A removed override keeps running until the GPU or Windows restarts, and Verify cannot
  tell it apart, so restart before checking a removal.
- `unstable`: the two reads differ; try again.

NvAPI returns at most 1,024,000 bytes, so a longer override is compared on those bytes only: the result then
says that the first 1,024,000 bytes match, not that it was verified byte by byte. Verify needs a driver whose
NvAPI has `NvAPI_GPU_GetVbiosImage` (617.42 and 621.01 have it); with an older one the error says so. It does not
work inside a virtual machine with a paravirtualized GPU (GPU-P).

## If the GPU does not start

Test first on a GPU that is **not** driving your display (use the integrated graphics or another card for
the screen), so you can always run `unload`.

If the screen stays black right after a load with **Restart the GPU now**, restart Windows first: the
override stays stored and may work after a full restart.

If Windows shows no image or the GPU fails in Device Manager:

1. Boot into Safe Mode: hold Shift while clicking Restart, then Troubleshoot → Advanced options → Startup
   Settings → Restart → 4 (Enable Safe Mode). Without a usable desktop, turn the PC off during boot two or
   three times (or boot Windows installation media → Repair your computer) to reach the same Troubleshoot
   menu. The NVIDIA driver does not load in Safe Mode, so the screen works there.
2. Open `vbios-loader-gui.exe`, select the GPU, turn off **Restart the GPU now** and click **Remove override**;
   or, in an elevated PowerShell, run `.\vbios-loader.exe unload --gpu 0 --no-restart` (use the GPU's index
   from `status`). Then restart Windows normally.
3. If that does not work, open `regedit` and go to
   `HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}`.
   Open the numbered subkey whose `DriverDesc` is your GPU (`status` shows it as `driver_key`). If
   `VixLoaderPreviousEnableGpuFirmware` starts with `hex(`, use step 4 instead. Otherwise delete `vbios`, set
   `EnableGpuFirmware` back to the saved value, or delete `EnableGpuFirmware` if the saved value is `absent`.
   Then delete `VixLoaderPreviousEnableGpuFirmware` and restart.
4. If the saved value starts with `hex(`, `EnableGpuFirmware` was not a 4-byte REG_DWORD before the first
   `load`, and the saved text is an exact copy of it in `.reg` file notation. The easiest way to put it back
   is a `.reg` file instead of step 3: save these lines in Notepad as `restore.reg`, with `NNNN` replaced by
   the subkey number and the text after `"EnableGpuFirmware"=` replaced by the saved value. Double-click the
   file (or run `reg import restore.reg` in an elevated PowerShell) and restart. It also deletes `vbios` and
   the saved value.

   ```
   Windows Registry Editor Version 5.00

   [HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\NNNN]
   "vbios"=-
   "EnableGpuFirmware"=hex(1):31,00,00,00
   "VixLoaderPreviousEnableGpuFirmware"=-
   ```

## Tests

```sh
PYTHONPATH=src python -m unittest discover -s tests
```

The tests use synthetic files and a fake registry; they run on Linux and Windows and never touch a GPU or
the real registry. The workflow in `.github/workflows/build.yml` runs them on Windows, builds
`vbios-loader.exe` and `vbios-loader-gui.exe` with Nuitka, checks that the window opens (with a
screenshot), and publishes tagged versions as releases with `SHA256SUMS` and the third-party license texts.

## License

Copyright (C) 2026 panchovix. GNU General Public License v3.0 or later (`GPL-3.0-or-later`), see
[LICENSE](LICENSE): you can use, study, change and share it, and changed versions you share must stay under
the same license. See also [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Version 0.1.0 was released under
the MIT license.
