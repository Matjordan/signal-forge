"""Save, restart disconnected, reconnect explicitly, and recover corrupt config."""
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import tty

pairs = [pty.openpty(), pty.openpty()]
for master, slave in pairs:
    tty.setraw(slave)
    os.set_blocking(master, False)

def window_for(process):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        assert process.poll() is None, process.returncode
        result = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^Signal Forge$'], capture_output=True, text=True)
        if result.returncode == 0:
            window = result.stdout.splitlines()[0]
            focused = subprocess.run(['xdotool', 'windowfocus', '--sync', window], capture_output=True)
            if focused.returncode == 0:
                time.sleep(.5)
                return window
        time.sleep(.1)
    raise AssertionError('No application window')

def key(window, keys):
    subprocess.run(['xdotool', 'key', '--window', window, keys], check=True)
    time.sleep(.2)

def quit_app(process, window):
    key(window, 'ctrl+q')
    assert process.wait(timeout=5) == 0

with tempfile.TemporaryDirectory(prefix='signal-forge-workspace-') as directory:
    env = dict(os.environ, XDG_CONFIG_HOME=directory)
    config_path = Path(directory) / 'signal-forge/workspace.json'
    log = open(Path(directory) / 'app.log', 'w')
    process = None
    try:
        args = ['target/debug/signal-forge', '--no-update-check']
        for _, slave in pairs:
            args.extend(['--port', os.ttyname(slave)])
        process = subprocess.Popen(args, env=env, stderr=log)
        window = window_for(process)
        # RX fragments and independent receive modes must survive a workspace restart.
        for chunk in [b'STA', b'TUS=OK\r', b'\nPART']:
            os.write(pairs[0][0], chunk)
            time.sleep(.05)
        for x in [330, 385, 268, 330]:  # Left: Raw Chunks -> Hex -> Line -> Raw Chunks.
            subprocess.run(['xdotool', 'mousemove', '--window', window, str(x), '97', 'click', '1'], check=True)
            time.sleep(.1)
        subprocess.run(['xdotool', 'mousemove', '--window', window, '985', '97', 'click', '1'], check=True)
        time.sleep(.1)
        # Change split ratio with the visible divider, then save via keyboard.
        subprocess.run(['xdotool', 'mousemove', '--window', window, '844', '350', 'mousedown', '1', 'sleep', '0.2', 'mousemove', '--window', window, '740', '350', 'sleep', '0.2', 'mouseup', '1'], check=True)
        key(window, 'ctrl+s')
        initial = json.loads(config_path.read_text())
        assert initial['version'] == 2
        assert initial['layout']['Split']['horizontal'] is True
        assert abs(initial['layout']['Split']['fraction'] - .5) > .02, initial
        assert len(initial['ports']) == 2
        left = initial['layout']['Split']['first']['Leaf']['tabs'][0]
        right = initial['layout']['Split']['second']['Leaf']['tabs'][0]
        assert left['settings']['baud'] == right['settings']['baud'] == 19200
        assert left['receive_mode'] == 'RawChunks', left
        assert right['receive_mode'] == 'Hex', right
        assert left['delimiter'] == right['delimiter'] == 'Auto'
        quit_app(process, window)
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check'], env=env, stderr=log)
        window = window_for(process)
        assert not select.select([p[0] for p in pairs], [], [], .3)[0], 'Restore transmitted bytes'
        log.flush()
        assert (Path(directory) / 'app.log').read_text().count('Opened ') == 2, 'Restore opened a device'
        key(window, 'ctrl+s')
        restored = json.loads(config_path.read_text())
        assert restored == initial, (restored, initial)
        key(window, 'ctrl+shift+o')  # Explicitly reconnect selected terminal.
        key(window, 'ctrl+l')
        subprocess.run(['xdotool', 'type', '--clearmodifiers', 'restored'], check=True)
        key(window, 'Return')
        assert select.select([pairs[1][0]], [], [], 3)[0], 'Explicit reconnect/send failed'
        assert os.read(pairs[1][0], 64) == b'restored'
        key(window, 'ctrl+w')
        key(window, 'ctrl+s')
        assert 'Leaf' in json.loads(config_path.read_text())['layout'], 'Selected terminal did not close'
        quit_app(process, window)
        # Corrupt config must be preserved through ordinary save and clean quit.
        broken = b'{broken workspace JSON'
        config_path.write_bytes(broken)
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check'], env=env, stderr=log)
        window = window_for(process)
        key(window, 'ctrl+s')
        assert config_path.read_bytes() == broken
        # Recovery action appears immediately below the bottom error line.
        subprocess.run(['xdotool', 'mousemove', '--window', window, '150', '890', 'click', '1'], check=True)
        time.sleep(.3)
        # Coordinates are verified by the required backup, not assumed successful.
        backups = list(config_path.parent.glob('workspace.json.recovery-*'))
        assert backups and backups[0].read_bytes() == broken, 'Recovery backup missing'
        key(window, 'ctrl+s')
        assert json.loads(config_path.read_text())['version'] == 2
        quit_app(process, window)
        print('Workspace save/restart, disconnected restore, no automatic TX, corrupt-file preservation and backup recovery passed.', flush=True)
    except Exception:
        log.flush()
        print((Path(directory) / 'app.log').read_text(), flush=True)
        subprocess.run(['import', '-window', window, '/tmp/workspace-failure.png'], check=False)
        subprocess.run(['convert', '/tmp/workspace-failure.png', '-resize', '1280x', '-quality', '80', '/tmp/workspace-failure.jpg'], check=False)
        raise
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        log.close()
        for master, slave in pairs:
            os.close(master)
            os.close(slave)
