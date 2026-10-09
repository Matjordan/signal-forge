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
from gui_smoke_support import window_for, click_control

pairs = [pty.openpty() for _ in range(4)]
for master, slave in pairs:
    tty.setraw(slave)
    os.set_blocking(master, False)

def command(*args):
    return subprocess.check_output(['xdotool', *args], text=True).strip()

def key(keys):
    command('key', '--window', window, '--clearmodifiers', keys)
    time.sleep(.2)

def snapshot(path):
    previous = path.stat().st_mtime_ns if path.exists() else None
    key('ctrl+s')
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        assert process.poll() is None, log_path.read_text()
        if path.exists() and path.stat().st_mtime_ns != previous:
            return json.loads(path.read_text())
        time.sleep(.05)
    raise AssertionError('Workspace save did not complete: ' + log_path.read_text())

def leaf_count(layout):
    if 'Leaf' in layout:
        return 1
    split = layout['Split']
    return leaf_count(split['first']) + leaf_count(split['second'])

with tempfile.TemporaryDirectory(prefix='signal-forge-ui-') as directory:
    config = Path(directory) / 'signal-forge/workspace.json'
    log_path = Path(directory) / 'app.log'
    args = ['target/debug/signal-forge', '--no-update-check']
    for _, slave in pairs:
        args.extend(['--port', os.ttyname(slave)])
    with log_path.open('w') as log:
        process = subprocess.Popen(args, env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
        try:
            window = window_for(process)
            time.sleep(.5)
            for index, (master, _) in enumerate(pairs):
                os.write(master, f'Device {index + 1}: ready at 19200\r\n'.encode())
            command('mousemove', '--window', window, '400', '200', 'click', '1')
            time.sleep(.2)
            click_control(window, log_path, '4 Tiles')
            time.sleep(.3)
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
            selected_master = next(master for master, slave in pairs if os.ttyname(slave) == initial['selected'])
            assert select.select([selected_master], [], [], 3)[0], 'Narrow-pane send failed'
            assert os.read(selected_master, 64) == b'minimum-window'
            key('ctrl+a')
            command('type', '--window', window, '--clearmodifiers', 'minimum-button')
            click_control(window, log_path, 'serial:' + initial['selected'] + ':Send payload')
            assert select.select([selected_master], [], [], 3)[0], 'Narrow-pane Send button failed'
            assert os.read(selected_master, 64) == b'minimum-button'
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
            click_control(window, log_path, '2 Tiles')
            time.sleep(.3)
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
