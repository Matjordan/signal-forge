"""Create an emulated pair in the GUI, open only its peer, record paced binary RX and clean up."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
from gui_smoke_support import click, window_for

assert os.geteuid() != 0, 'Run virtual-pair smoke without sudo/root'
with tempfile.TemporaryDirectory(prefix='signal-forge-virtual-ui-') as directory:
    root = Path(directory)
    config = root / 'signal-forge' / 'workspace.json'
    config.parent.mkdir()
    # Opening an owned PTY must not inherit flow control/framing from this physical port.
    config.write_text(json.dumps({'version': 2, 'ports': [{'path': '/dev/previous-device', 'baud': 9600,
        'data_bits': 7, 'parity': 'Even', 'stop_bits': 2, 'flow': 'Software'}]}))
    log_path = root / 'app.log'
    with log_path.open('w') as log:
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check'],
                                   env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
        window = window_for(process)
        paths = None
        peer = None
        def key(keys):
            subprocess.run(['xdotool', 'key', '--window', window, '--clearmodifiers', keys], check=True)
            time.sleep(.2)
        def wait(predicate):
            deadline = time.monotonic() + 5
            while not predicate():
                assert process.poll() is None, log_path.read_text()
                assert time.monotonic() < deadline, log_path.read_text()
                time.sleep(.02)
        try:
            key('ctrl+shift+n')
            click(window, 650, 490)  # Emulated baud, default 19200 8N1
            key('ctrl+Return')
            wait(lambda: 'Created PTY pair bench:' in log_path.read_text())
            match = re.search(r'Created PTY pair bench: (\S+) <-> (\S+)', log_path.read_text())
            paths = match.groups()
            assert 'Emulated · 19200 baud · 8N1' in log_path.read_text()
            # Sidebar height varies with path lengths and wrapped device labels.
            # Use the rendered control's pixel coordinates, not a fixed row.
            control_pattern = re.compile(r'PTY open control ' + re.escape(paths[0]) + r': ([\d.]+),([\d.]+)')
            wait(lambda: control_pattern.search(log_path.read_text()))
            control = list(control_pattern.finditer(log_path.read_text()))[-1]
            click(window, *(round(float(value)) for value in control.groups()))
            wait(lambda: 'Opened ' + paths[0] in log_path.read_text())
            key('ctrl+s')
            saved = json.loads(config.read_text())
            port = next(item for item in saved['ports'] if item['path'] == paths[0])
            assert (port['baud'], port['data_bits'], port['parity'], port['stop_bits'], port['flow']) == (19200, 8, 'None', 1, 'None'), port
            # Ordinary same-side ownership must be busy; external clients use B.
            try:
                unexpected = os.open(paths[0], os.O_RDWR | os.O_NOCTTY)
            except OSError as error:
                assert error.errno == 16, error
            else:
                os.close(unexpected)
                raise AssertionError('Signal Forge did not hold expected same-side exclusivity')
            peer = os.open(paths[1], os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
            click(window, 430, 786)  # Files / RX
            recording = root / 'paced.bin'
            click(window, 330, 734)
            key('ctrl+a')
            subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', str(recording)], check=True)
            click(window, 525, 734)
            wait(recording.exists)
            payload = bytes(range(256)) * 3
            started = time.monotonic()
            os.write(peer, payload)
            wait(lambda: recording.stat().st_size >= 32)
            assert recording.stat().st_size < len(payload), 'Emulated delivery arrived in one burst'
            wait(lambda: recording.read_bytes() == payload)
            elapsed = time.monotonic() - started
            assert .36 < elapsed < .8, elapsed  # 768 * 10 / 19200 = .4 seconds
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-virtual-timing.png'], check=True)
            key('ctrl+q')
            assert process.wait(timeout=5) == 0
            assert recording.read_bytes() == payload
            assert all(not Path(path).exists() for path in paths)
            print('Unprivileged GUI timing configuration, safe PTY settings, same-side ownership/peer access, progressive exact binary RX recording, and cleanup passed.', flush=True)
        except BaseException:
            if process.poll() is None:
                subprocess.run(['import', '-window', window, '/tmp/signal-forge-virtual-failure.png'], check=False)
            raise
        finally:
            if peer is not None:
                os.close(peer)
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
