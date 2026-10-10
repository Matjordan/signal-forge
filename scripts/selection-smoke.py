"""Real PTY selection inspection and clipboard copies across all terminal views."""
import functools
import json
import os
from pathlib import Path
import pty
import re
import select
import subprocess
import tempfile
import time
import tty
from gui_smoke_support import click_control, window_for

master, slave = pty.openpty()
tty.setraw(slave)
os.set_blocking(master, False)
path = os.ttyname(slave)
payload = b'$ABC*40\r\n!ABC*41\r\n#0,MED\x00\xff\r\n'
with tempfile.TemporaryDirectory(prefix='signal-forge-selection-') as directory:
    root = Path(directory) / 'signal-forge'
    root.mkdir()
    settings = dict(path=path, baud=19200, data_bits=8, parity='None', stop_bits=1, flow='None')
    (root / 'workspace.json').write_text(json.dumps(dict(version=2, ports=[settings], layout={'Leaf': {'tabs': [dict(settings=settings, show_controls=True)], 'active': 0}})))
    log_path = root / 'app.log'
    with log_path.open('w') as log:
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', path],
                                   env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
        window = window_for(process)
        def key(value):
            subprocess.run(['xdotool', 'key', '--window', window, '--clearmodifiers', value], check=True)
            time.sleep(.2)
        def clipboard():
            return subprocess.check_output(['xclip', '-selection', 'clipboard', '-o'], text=True, timeout=3)
        def copy(control, expected):
            click_control(window, log_path, control)
            deadline = time.monotonic() + 3
            actual = clipboard()
            while actual != expected and time.monotonic() < deadline:
                time.sleep(.05)
                actual = clipboard()
            assert actual == expected, (control, actual, expected)

        try:
            time.sleep(.3)
            os.write(master, payload[:3])
            time.sleep(.1)
            os.write(master, payload[3:])
            time.sleep(.3)
            for mode in ['Line', 'Raw Chunks', 'Hex']:
                click_control(window, log_path, 'serial:' + path + ':' + mode)
                click_control(window, log_path, 'serial:' + path + ':Traffic row 0')
                key('ctrl+a')
                key('ctrl+c')
                assert 'RX' in clipboard(), 'Inspector stole focus from normal terminal copy'
                copy('Copy Hex', payload.hex(' ').upper())
                copy('Copy ASCII', '$ABC*40\\r\\n!ABC*41\\r\\n#0,MED\\0\\xFF\\r\\n')
                copy('Copy XOR', f'0x{functools.reduce(int.__xor__, payload):02X}')
                copy('Copy NMEA result 0', 'NMEA VALID · calculated 0x40 · received 0x40')
                copy('Copy NMEA result 9', 'NMEA INVALID · calculated 0x40 · received 0x41')
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-selection.png'], check=True)
            click_control(window, log_path, 'serial:' + path + ':Line')
            pattern = re.compile(r'UI traffic origin ' + re.escape('serial:' + path) + r':2: ([\d.]+),([\d.]+),([\d.]+)')
            match = list(pattern.finditer(log_path.read_text()))[-1]
            x, y, advance = map(float, match.groups())
            # Timestamp (13), RX (2), and separating spaces (2) precede the payload.
            start, end = round(x + 17 * advance), round(x + 23 * advance)
            subprocess.run(['xdotool', 'mousemove', '--window', window, str(start), str(round(y)),
                            'mousedown', '1', 'sleep', '.15', 'mousemove', '--window', window, str(end), str(round(y)),
                            'sleep', '.15', 'mouseup', '1'], check=True)
            time.sleep(.2)
            copy('Copy Hex', '23 30 2C 4D 45 44')
            copy('Copy XOR', '0x73')
            assert not select.select([master], [], [], .1)[0], 'Selection/copy transmitted serial data'
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-selection-xor.png'], check=True)
            key('ctrl+q')
            assert process.wait(timeout=5) == 0
            print('Exact binary Line/Raw/Hex selections, ASCII/hex/XOR clipboard copies, valid/invalid NMEA results, and no automatic TX passed.', flush=True)
        except BaseException:
            Path("/tmp/selection-app-failure.log").write_text(log_path.read_text())
            print(log_path.read_text()[-5000:], flush=True)
            if process.poll() is None:
                subprocess.run(['import', '-window', window, '/tmp/signal-forge-selection-failure.png'], check=False)
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            os.close(master)
            os.close(slave)
