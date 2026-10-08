"""Exercise the real remote terminal, presets and disconnected workspace restore."""
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import tty
from gui_smoke_support import window_for

master, slave = pty.openpty()
tty.setraw(slave)
uri = 'ssh://' + os.environ['SIGNAL_FORGE_TEST_SSH_HOST'] + os.ttyname(slave)

def read_bytes(expected):
    output = bytearray()
    deadline = time.monotonic() + 5
    while len(output) < len(expected) and time.monotonic() < deadline:
        if select.select([master], [], [], .1)[0]:
            output.extend(os.read(master, 65536))
    assert output == expected, output

with tempfile.TemporaryDirectory(prefix='signal-forge-remote-ui-') as directory:
    root = Path(directory) / 'signal-forge'
    root.mkdir()
    preset = dict(name='Remote poll', payload='00 FF', encoding='Hex', escapes=False,
                  ending='None', description='SSH fixture', shortcut='Ctrl+1',
                  target={'Endpoint': 'serial:' + uri}, repeat={'interval_ms': 30, 'count': 3})
    (root / 'presets.json').write_text(json.dumps({'version': 1, 'profiles': [{'name': 'Remote', 'presets': [preset]}]}))
    env = dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug')
    log_path = root / 'app.log'
    process = None
    with log_path.open('w') as log:
        try:
            process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', uri], env=env, stderr=log)
            window = window_for(process)
            deadline = time.monotonic() + 15
            while 'SSH remote serial connected' not in log_path.read_text():
                assert process.poll() is None, log_path.read_text()
                assert time.monotonic() < deadline, log_path.read_text()
                time.sleep(.1)
            os.write(master, b'REMOTE\r\n\0\xff')
            for keys in ('ctrl+l', 'ctrl+a'):
                subprocess.run(['xdotool', 'key', '--window', window, keys], check=True)
            subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', r'GUI\x00\xff'], check=True)
            subprocess.run(['xdotool', 'key', '--window', window, 'Return'], check=True)
            read_bytes(b'GUI\0\xff')
            subprocess.run(['xdotool', 'mousemove', '--window', window, '300', '200', 'click', '1'], check=True)
            subprocess.run(['xdotool', 'key', '--window', window, 'ctrl+1'], check=True)
            read_bytes(bytes([0, 255]) * 3)
            subprocess.run(['xdotool', 'key', '--window', window, 'ctrl+s'], check=True)
            deadline = time.monotonic() + 3
            while not (root / 'workspace.json').exists() and time.monotonic() < deadline:
                time.sleep(.05)
            workspace = json.loads((root / 'workspace.json').read_text())
            assert workspace['ports'][0]['path'] == uri
            subprocess.run(['xdotool', 'key', '--window', window, 'ctrl+q'], check=True)
            assert process.wait(timeout=5) == 0
            connected_count = log_path.read_text().count('SSH remote serial connected')
            process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check'], env=env, stderr=log)
            window = window_for(process)
            time.sleep(.5)
            assert log_path.read_text().count('SSH remote serial connected') == connected_count, 'Restore connected automatically'
            assert not select.select([master], [], [], .3)[0], 'Restore transmitted data'
            subprocess.run(['xdotool', 'key', '--window', window, 'ctrl+q'], check=True)
            assert process.wait(timeout=5) == 0
            print('Remote GUI send, binary repeating preset, workspace identity and disconnected/no-TX restore passed.', flush=True)
        finally:
            if process and process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            os.close(master)
            os.close(slave)
