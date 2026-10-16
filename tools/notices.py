"""Collect the license texts for the Windows programs: usage `python tools/notices.py OUTPUT_DIRECTORY`.

vbios-loader.exe and vbios-loader-gui.exe contain the CPython runtime and the Nuitka runtime; the window
program also contains Tcl/Tk (Nuitka's tk-inter plugin). The texts are copied from the Python installation
and from Nuitka's package metadata. Missing or unreviewed texts fail the build.
"""
from pathlib import Path
import platform
import re
import shutil
import sys
import tkinter

sys.path.insert(0, str(Path(__file__).resolve().parent))
from license_texts import copy_nuitka_licenses, copy_nuitka_onefile_licenses  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
TCL_TK_TERMS = re.compile(r'This software is copyrighted by the Regents of the University of\s+California.*?'
                          r'terms specified in\s+this\s+license\.', re.S)


def tcl_tk_license():
    """The Tcl and Tk license terms shipped with this Python: in its LICENSE.txt (Windows builds append
    them there) and in any license.terms under its tcl directory. Both texts must be found."""
    base = Path(sys.base_prefix)
    sources = [base / 'LICENSE.txt', *sorted((base / 'tcl').rglob('license.terms'))]
    texts = []
    for source in sources:
        if source.is_file():
            for match in TCL_TK_TERMS.finditer(source.read_text(encoding='utf-8', errors='replace')):
                if match.group(0) not in texts:
                    texts.append(match.group(0))
    if not any('Apple Inc.' in text for text in texts) or not any('Apple Inc.' not in text for text in texts):
        raise SystemExit('the Tcl and Tk license terms were not found in the Python installation')
    return '\n\n----------------------------------------\n\n'.join(texts) + '\n'


def main(argv):
    if len(argv) != 2:
        print(__doc__.split('\n')[0], file=sys.stderr)
        return 2
    output = Path(argv[1])
    third_party = output / 'third_party'
    third_party.mkdir(parents=True)
    terms = copy_nuitka_licenses(third_party)
    copy_nuitka_onefile_licenses(third_party)
    shutil.copyfile(Path(sys.base_prefix) / 'LICENSE.txt', third_party / 'python.LICENSE.txt')
    (third_party / 'tcl-tk.LICENSE.txt').write_text(tcl_tk_license(), encoding='utf-8')
    shutil.copyfile(ROOT / 'LICENSE', output / 'LICENSE')
    source = (ROOT / 'THIRD_PARTY_NOTICES.md').read_text(encoding='utf-8')
    source_notices = '\n' + source.split('\n', 1)[1].split('## Windows program')[0].strip() + '\n'
    (output / 'THIRD_PARTY_NOTICES.md').write_text(
        '# Third-party notices\n\n'
        '## Windows program\n\n'
        f'vbios-loader.exe and vbios-loader-gui.exe contain the CPython {platform.python_version()} runtime and '
        'standard library (Python Software Foundation License, `third_party/python.LICENSE.txt`; that file also '
        'covers the libraries and Microsoft C runtime files distributed with the Windows build of CPython). '
        f'They are compiled with Nuitka; the Nuitka runtime compiled into them is licensed under {terms} '
        '(`third_party/nuitka.*`), and their single-file bootstrap includes Zstandard '
        '(BSD, `third_party/nuitka-inline-zstd.LICENSE`). '
        f'vbios-loader-gui.exe also contains Tcl/Tk {tkinter.Tcl().eval("info patchlevel")} from the same Python '
        'installation (`_tkinter`, the Tcl and Tk libraries and their script libraries; Tcl/Tk license, '
        '`third_party/tcl-tk.LICENSE.txt`).\n' + source_notices, encoding='utf-8')
    return 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv))
