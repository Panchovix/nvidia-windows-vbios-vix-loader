"""Entry point of the single-file Windows program built by .github/workflows/build.yml."""
import sys

from nvidia_vbios.windows_loader.cli import main

try:
    code = main()
except KeyboardInterrupt:
    code = 130
except Exception as exc:  # unexpected: one line, no traceback
    print(f'vbios-loader: unexpected error: {type(exc).__name__}: {exc}', file=sys.stderr)
    code = 70
raise SystemExit(code)
