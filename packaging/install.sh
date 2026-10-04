#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
prefix="${1:-${HOME}/.local}"
install -Dm755 bin/signal-forge "$prefix/bin/signal-forge"
mkdir -p "$prefix/share/applications" "$prefix/share/doc/signal-forge"
# Desktop Exec is an absolute, quoted path so a nonstandard prefix works.
python3 - "$prefix" <<'PY'
from pathlib import Path
import sys
prefix = Path(sys.argv[1]).absolute()
exe = str(prefix / 'bin/signal-forge')
if any(c in exe for c in '\n\r'):
    raise SystemExit('Installation path cannot contain a newline')
exe = exe.replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
s = Path('share/applications/signal-forge.desktop').read_text()
s = s.replace('Exec=signal-forge', 'Exec="' + exe + '"')
(prefix / 'share/applications/signal-forge.desktop').write_text(s)
PY
cp -R share/doc/signal-forge/. "$prefix/share/doc/signal-forge/"
printf 'Installed Signal Forge in %s/bin\n' "$prefix"
