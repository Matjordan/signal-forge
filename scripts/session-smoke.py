"""Exercise session creation, notes, RX defaults/index, safe reopen and shutdown in Xvfb."""
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import tty
from gui_smoke_support import click, click_control, window_for

master, slave = pty.openpty()
tty.setraw(slave)
os.set_blocking(master, False)
with tempfile.TemporaryDirectory(prefix='signal-forge-session-') as directory:
    root = Path(directory)
    folder = root / 'Engineering session with spaces'
    log_path = root / 'app.log'
    log = log_path.open('w')
    process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', os.ttyname(slave)],
                               env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
    window = window_for(process)

    def key(keys):
        subprocess.run(['xdotool', 'key', '--window', window, '--clearmodifiers', keys], check=True)
        time.sleep(.2)

    def text(control, value):
        click_control(window, log_path, control)
        key('ctrl+a')
        subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', '--delay', '2', value], check=True)
        time.sleep(.2)

    def wait(predicate):
        deadline = time.monotonic() + 5
        while not predicate():
            assert process.poll() is None, log_path.read_text()
            assert time.monotonic() < deadline, log_path.read_text()
            time.sleep(.05)

    def metadata():
        return json.loads((folder / 'session.json').read_text())

    try:
        # The dedicated session shortcut keeps this test independent of toolbar width.
        key('ctrl+shift+e')
        click_control(window, log_path, 'New session')  # New session
        text('Session name', 'Engineering run')
        text('Session folder', str(folder))
        click_control(window, log_path, 'Create session')
        wait(lambda: (folder / 'session.json').exists())
        assert metadata()['name'] == 'Engineering run'
        text('Session notes', 'Pressure test: 12 kPa')
        key('Return')
        subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', 'Binary data verified.'], check=True)
        notes = 'Pressure test: 12 kPa\nBinary data verified.'
        wait(lambda: (folder / 'notes.txt').read_text() == notes)
        assert not select.select([master], [], [], .1)[0], 'Typing session notes transmitted serial data'
        key('ctrl+shift+e')
        click_control(window, log_path, 'serial:' + os.ttyname(slave) + ':Files / RX')  # Files / RX tool, single terminal at default size
        click_control(window, log_path, 'Record RX')  # Record RX, using session's generated default path
        wait(lambda: len(metadata()['artifacts']) == 1)
        artifact = metadata()['artifacts'][0]
        assert artifact['kind'] == 'rx_recording'
        assert not Path(artifact['path']).is_absolute(), artifact
        recording = folder / artifact['path']
        payload = bytes(range(256)) + b'\x00\xff\r\n'
        os.write(master, payload)
        wait(lambda: recording.read_bytes() == payload)
        key('ctrl+shift+e')
        subprocess.run(['import', '-window', window, '/tmp/signal-forge-session.png'], check=True)
        click_control(window, log_path, 'Close session')  # Close session: finalizes active RX recorder
        wait(lambda: metadata()['closed_unix_ns'] is not None)
        assert recording.read_bytes() == payload
        # Closing a session leaves the live endpoint usable.
        key('ctrl+shift+e')
        click(window, 400, 200)
        key('ctrl+l')
        subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', 'still-connected'], check=True)
        key('Return')
        assert select.select([master], [], [], 3)[0], 'Close session disconnected endpoint'
        assert os.read(master, 64) == b'still-connected'
        # Reopen explicitly, restoring the snapshot disconnected and preserving notes/index.
        key('ctrl+shift+e')
        click_control(window, log_path, 'Open session')  # Open session
        click_control(window, log_path, 'Open session folder')
        wait(lambda: metadata()['closed_unix_ns'] is None)
        assert (folder / 'notes.txt').read_text() == notes
        assert len(metadata()['artifacts']) == 1
        assert log_path.read_text().count('Opened ') == 1, 'Session reopen reconnected a device'
        key('ctrl+s')
        workspace = json.loads((folder / 'workspace.json').read_text())
        assert workspace['layout']['Leaf']['tabs'][0]['settings']['path'] == os.ttyname(slave)
        assert not select.select([master], [], [], .1)[0], 'Session reopen transmitted automatically'
        # Missing artifacts remain indexed, and reopening does not delete or recreate them.
        preserved = recording.with_suffix('.preserved')
        recording.rename(preserved)
        text('Session notes', 'Notes saved during shutdown')
        # A conflicting external edit must be preserved, and close must be cancelled.
        (folder / 'notes.txt').write_text('Edited externally')
        key('ctrl+s')
        assert (folder / 'notes.txt').read_text() == 'Edited externally'
        key('ctrl+q')
        assert process.poll() is None, 'Conflicting notes caused unsaved session shutdown'
        assert (folder / 'notes.txt').read_text() == 'Edited externally'
        # Restore the expected disk version, then retry a normal close with unsaved notes.
        (folder / 'notes.txt').write_text(notes)
        key('ctrl+q')
        assert process.wait(timeout=5) == 0
        assert (folder / 'notes.txt').read_text() == 'Notes saved during shutdown'
        assert metadata()['closed_unix_ns'] is not None
        assert preserved.read_bytes() == payload
        assert not recording.exists()
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check'],
                                   env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
        window = window_for(process)
        time.sleep(.3)
        assert log_path.read_text().count('Opened ') == 1, 'Restart reconnected a device'
        key('ctrl+shift+e')
        # No session automatically resumes; choose it explicitly, with the missing artifact retained.
        click_control(window, log_path, 'Session folder')
        text('Session folder', str(folder))
        click_control(window, log_path, 'Open session folder')
        wait(lambda: metadata()['closed_unix_ns'] is None)
        assert len(metadata()['artifacts']) == 1 and not recording.exists()
        assert not select.select([master], [], [], .1)[0]
        key('ctrl+q')
        assert process.wait(timeout=5) == 0
        print('Session creation, notes/no-TX, recording defaults/index, finalization, live connection retention, disconnected reopen/restart, external-edit preservation/cancelled close and missing artifacts passed.', flush=True)
    except BaseException:
        if process.poll() is None:
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-session-failure.png'], check=False)
        raise
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        log.close()
        os.close(master)
        os.close(slave)
