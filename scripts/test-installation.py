#!/usr/bin/env python3
"""Exercise a real release installer, desktop entries, icon ownership and uninstall."""
import ctypes
import ctypes.util
import json
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile

package = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='signal-forge-install-') as directory:
    prefix = Path(directory) / 'prefix with spaces $dollar "quote" %field `tick` \\slash'
    for _ in range(2):
        subprocess.run(['bash', str(package / 'install.sh'), str(prefix)], check=True)
        manifest_path = prefix / 'share/signal-forge/install-manifest.json'
        manifest = json.loads(manifest_path.read_text())
        assert manifest['owner'] == 'signal-forge' and manifest['version'] == 1
        assert len(manifest['files']) == len(set(manifest['files']))
        assert all((prefix / path).is_file() for path in manifest['files'])
        launcher = prefix / 'share/applications/signal-forge.desktop'
        subprocess.run(['desktop-file-validate', str(launcher)], check=True)
        lines = dict(line.split('=', 1) for line in launcher.read_text().splitlines() if '=' in line)
        assert lines['Name'] == 'Signal Forge' and lines['Icon'] == 'signal-forge'
        assert lines['StartupWMClass'] == 'signal-forge'
        # Decode the desktop string layer, then the quoted Exec argument layer.
        glib = ctypes.CDLL(ctypes.util.find_library('glib-2.0'))
        glib.g_shell_parse_argv.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.POINTER(ctypes.c_char_p)), ctypes.c_void_p]
        count = ctypes.c_int()
        arguments = ctypes.POINTER(ctypes.c_char_p)()
        desktop_exec = lines['Exec'].replace('\\\\', '\\')
        assert glib.g_shell_parse_argv(desktop_exec.encode(), ctypes.byref(count), ctypes.byref(arguments), None)
        assert count.value == 1
        executable = arguments[0].decode().replace('%%', '%')
        glib.g_strfreev(arguments)
        assert executable == str(prefix / 'bin/signal-forge'), executable
        subprocess.run([executable, '--version'], check=True)
        for size in (16, 24, 32, 48, 64, 128, 256, 512):
            path = prefix / f'share/icons/hicolor/{size}x{size}/apps/signal-forge.png'
            data = path.read_bytes()
            assert data[:8] == b'\x89PNG\r\n\x1a\n'
            assert struct.unpack('>II', data[16:24]) == (size, size)
            assert data == (package / path.relative_to(prefix)).read_bytes()
            assert stat.S_IMODE(path.stat().st_mode) == 0o644
        assert stat.S_IMODE((prefix / 'bin/signal-forge').stat().st_mode) == 0o755
        assert stat.S_IMODE((prefix / 'share/signal-forge/uninstall.sh').stat().st_mode) == 0o755
    unrelated = prefix / 'share/applications/other.desktop'
    unrelated.write_text('unrelated app')
    settings = prefix / 'workspace.json'
    settings.write_text('user data')
    owned = list(manifest['files'])
    subprocess.run(['bash', str(prefix / 'share/signal-forge/uninstall.sh')], check=True)
    assert not (prefix / 'bin/signal-forge').exists()
    assert all(not (prefix / path).exists() for path in owned)
    assert unrelated.read_text() == 'unrelated app'
    assert settings.read_text() == 'user data'
print('Fresh/repeated installation, special prefix paths, desktop validation, eight icons and owned-only uninstall passed.')
