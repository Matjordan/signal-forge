"""Verify the real restarted GUI confirms its version and preserves disconnected data."""
import json
import os
from pathlib import Path
import pty
import re
import select
import socket
import subprocess
import tempfile
import time
import tty
from gui_smoke_support import window_for

binary = 'target/debug/signal-forge'
version = subprocess.check_output([binary, '--version'], text=True).strip().removeprefix('Signal Forge ')
master, slave = pty.openpty()
tty.setraw(slave)
os.set_blocking(master, False)
reader, writer = socket.socketpair()
reader.settimeout(15)
process = None
with tempfile.TemporaryDirectory(prefix='signal-forge-update-') as directory:
    root = Path(directory) / 'signal-forge'
    root.mkdir()
    settings = dict(path=os.ttyname(slave), baud=19200, data_bits=8, parity='None', stop_bits=1, flow='None')
    terminal = dict(settings=settings, hex=False, receive_mode='Line', delimiter='Auto', timestamps=True, show_controls=True,
                    auto_scroll=True, encoding='Text', escapes=True, ending='None',
                    analysis=dict(visibility='Rx', direction_labels=False,
                                  search=dict(mode='Text', value='READY'), filter=dict(mode='Text', value=''),
                                  highlights=[dict(label='Fault', pattern=dict(mode='Text', value='ERROR'), emphasis='Error')],
                                  delta_displayed=True, delta_rx=False, delta_tx=False,
                                  line_span=False, response_latency=True))
    workspace = dict(version=2, ports=[settings], layout={'Leaf': {'tabs': [terminal], 'active': 0}},
                     windows=[], profile='Saved', selected=settings['path'])
    workspace_path = root / 'workspace.json'
    workspace_path.write_text(json.dumps(workspace))
    presets = b'{"version":1,"profiles":[{"name":"Saved","presets":[]}]}'
    preset_path = root / 'presets.json'
    preset_path.write_bytes(presets)
    log_path = Path(directory) / 'app.log'
    try:
        with log_path.open('w') as log:
            process = subprocess.Popen([binary, '--no-update-check'], stderr=log,
                                       pass_fds=(writer.fileno(),),
                                       env=dict(os.environ, XDG_CONFIG_HOME=directory,
                                                SIGNAL_FORGE_UPDATE_READY_FD=str(writer.fileno())))
        writer.close()
        message = b''
        while b'\n' not in message:
            chunk = reader.recv(256)
            assert chunk, log_path.read_text()
            message += chunk
        assert message == f'READY {version}\n'.encode(), message
        assert process.poll() is None, log_path.read_text()
        window = window_for(process)
        assert not select.select([master], [], [], .3)[0], 'Restart sent serial data'
        assert 'Opened ' not in log_path.read_text(), 'Restart reconnected a device'
        # Bypass xprop's icon formatter and its default property-length limit.
        icon_property = subprocess.check_output(
            ['xprop', '-id', window, '-len', str(8 * (2 + 256 * 256)),
             '-f', '_NET_WM_ICON', '32c', '_NET_WM_ICON'], text=True)
        icon_values = [int(value) for value in re.findall(r'\d+', icon_property.partition('=')[2])]
        assert icon_values[:2] == [256, 256], 'Application window icon has incorrect dimensions'
        assert len(icon_values) == 2 + 256 * 256, 'Application window icon has incomplete pixel data'
        assert any(pixel >> 24 for pixel in icon_values[2:]), 'Application window icon is transparent'
        window_class = subprocess.check_output(['xprop', '-id', window, 'WM_CLASS'], text=True)
        assert 'signal-forge' in window_class, window_class
        subprocess.run(['xdotool', 'key', '--window', window, 'ctrl+q'], check=True)
        assert process.wait(timeout=5) == 0
        assert json.loads(workspace_path.read_text()) == workspace
        assert preset_path.read_bytes() == presets
        print('Restarted GUI readiness/version, disconnected workspace, exact preset preservation, and no automatic TX passed.', flush=True)
    except Exception:
        print(log_path.read_text(), flush=True)
        raise
    finally:
        if process and process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        reader.close()
        writer.close()
        os.close(master)
        os.close(slave)
