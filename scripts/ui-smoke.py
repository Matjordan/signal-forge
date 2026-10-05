"""Exercise real four-terminal layouts, narrow panes, and cancelled setup dialogs."""
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import tty

pairs = [pty.openpty() for _ in range(4)]
for master, slave in pairs:
    tty.setraw(slave)
    os.set_blocking(master, False)

def command(*args):
    return subprocess.check_output(['xdotool', *args], text=True).strip()

def key(keys):
    command('key', '--window', window, keys)
    time.sleep(.2)

def snapshot(path):
    key('ctrl+s')
    return json.loads(path.read_text())

def leaf_count(layout):
    if 'Leaf' in layout:
        return 1
    split = layout['Split']
    return leaf_count(split['first']) + leaf_count(split['second'])

with tempfile.TemporaryDirectory(prefix='signal-forge-ui-') as directory:
    config = Path(directory) / 'signal-forge/workspace.json'
    log_path = Path(directory) / 'app.log'
    args = ['target/debug/signal-forge']
    for _, slave in pairs:
        args.extend(['--port', os.ttyname(slave)])
    with log_path.open('w') as log:
        process = subprocess.Popen(args, env=dict(os.environ, XDG_CONFIG_HOME=directory), stderr=log)
        try:
            deadline = time.monotonic() + 10
            window = None
            while time.monotonic() < deadline:
                assert process.poll() is None, log_path.read_text()
                result = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^Signal Forge$'], capture_output=True, text=True)
                if result.returncode == 0:
                    window = result.stdout.splitlines()[0]
                    break
                time.sleep(.1)
            assert window, 'No UI window'
            command('windowfocus', '--sync', window)
            time.sleep(.5)
            for index, (master, _) in enumerate(pairs):
                os.write(master, f'Device {index + 1}: ready at 19200\r\n'.encode())
            command('mousemove', '--window', window, '400', '200', 'click', '1')
            time.sleep(.2)
            key('ctrl+alt+4')
            initial = snapshot(config)
            assert leaf_count(initial['layout']) == 4, initial
            paths = {s['path'] for s in initial['ports']}
            assert paths == {os.ttyname(slave) for _, slave in pairs}
            assert log_path.read_text().count('Opened ') == 4
            for width, height, image in [
                (1920, 1080, '/tmp/signal-forge-four-tiles.png'),
                (2560, 1440, '/tmp/signal-forge-1440p.png'),
                (900, 600, '/tmp/signal-forge-minimum.png'),
            ]:
                command('windowsize', '--sync', window, str(width), str(height))
                time.sleep(.4)
                geometry = command('getwindowgeometry', '--shell', window)
                assert f'WIDTH={width}' in geometry and f'HEIGHT={height}' in geometry, geometry
                subprocess.run(['import', '-window', window, image], check=True)
                assert len(snapshot(config)['ports']) == 4
            # Ctrl+L still reaches the selected pane's editor at the minimum size.
            key('ctrl+l')
            command('type', '--window', window, '--clearmodifiers', 'minimum-window')
            key('Return')
            assert select.select([pairs[-1][0]], [], [], 3)[0], 'Narrow-pane send failed'
            assert os.read(pairs[-1][0], 64) == b'minimum-window'
            # Leave text editing before opening focused setup workflows.
            command('mousemove', '--window', window, '400', '200', 'click', '1')
            time.sleep(.2)
            before = snapshot(config)
            for shortcut in ['ctrl+o', 'ctrl+shift+n', 'ctrl+shift+b', 'ctrl+shift+s']:
                key(shortcut)
                key('Escape')
                assert snapshot(config) == before, shortcut
            assert 'Created PTY pair' not in log_path.read_text(), 'Cancel created a pair'
            assert log_path.read_text().count('Opened ') == 4, 'Cancel reopened a device'
            key('ctrl+alt+2')
            tiled = snapshot(config)
            assert leaf_count(tiled['layout']) == 2
            assert {s['path'] for s in tiled['ports']} == paths
            assert log_path.read_text().count('Opened ') == 4, 'Retile reopened endpoints'
            key('ctrl+q')
            assert process.wait(timeout=5) == 0
            print('Four/two live terminal layouts, 1080p/1440p/minimum sizes, narrow-pane send, and setup cancellation passed.', flush=True)
        except Exception:
            print(log_path.read_text(), flush=True)
            if window:
                subprocess.run(['import', '-window', window, '/tmp/signal-forge-minimum.png'], check=False)
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            for master, slave in pairs:
                os.close(master)
                os.close(slave)
