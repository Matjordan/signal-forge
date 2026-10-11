"""Offline guide navigation and real ten-second hover help without accidental TX."""
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
path = os.ttyname(slave)
with tempfile.TemporaryDirectory(prefix='signal-forge-help-') as directory:
    log_path = Path(directory) / 'app.log'
    env = dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug')
    with log_path.open('w') as log:
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', path], env=env, stderr=log)
        window = None
        def key(value):
            subprocess.run(['xdotool', 'key', '--window', window, '--clearmodifiers', value], check=True)
            time.sleep(.2)
        def wait(predicate, timeout=6):
            deadline = time.monotonic() + timeout
            while not predicate():
                assert process.poll() is None, log_path.read_text()[-4000:]
                assert time.monotonic() < deadline, log_path.read_text()[-4000:]
                time.sleep(.05)
        def hover(control):
            matches = re.findall(r'UI control ' + re.escape(control) + r': ([\d.]+),([\d.]+)', log_path.read_text())
            assert matches, control
            subprocess.run(['xdotool', 'mousemove', '--window', window, *[str(round(float(v))) for v in matches[-1]]], check=True)
        def latest_topic():
            return re.findall(r'Help topic opened: ([\w.]+)', log_path.read_text())[-1]
        def no_tx():
            assert not select.select([master], [], [], .1)[0], 'Help transmitted serial bytes'
        try:
            window = window_for(process)
            wait(lambda: 'Opened '+path in log_path.read_text())
            key('F1')
            click_control(window, log_path, 'Help search')
            subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', 'tx.checksum.skip_first'], check=True)
            time.sleep(.3)
            click_control(window, log_path, 'Help topic tx.checksum.skip_first')
            assert latest_topic() == 'tx.checksum.skip_first'
            click_control(window, log_path, 'Help related troubleshoot.checksum')
            assert latest_topic() == 'troubleshoot.checksum'
            click_control(window, log_path, 'Help Back')
            assert latest_topic() == 'tx.checksum.skip_first'
            key('alt+Left')
            assert latest_topic() == 'getting.started'
            # Minimum-size guide remains usable, including its topic navigation.
            subprocess.run(['xdotool', 'windowsize', window, '900', '600'], check=True)
            time.sleep(.4)
            key('ctrl+f')
            subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', 'troubleshoot.virtual'], check=True)
            time.sleep(.3)
            click_control(window, log_path, 'Help topic troubleshoot.virtual')
            assert latest_topic() == 'troubleshoot.virtual'
            time.sleep(.3)
            bounds = re.findall(r'Help guide bounds: ([\d.-]+),([\d.-]+),([\d.-]+),([\d.-]+)', log_path.read_text())
            left, top, right, bottom = map(float, bounds[-1])
            assert left >= 0 and top >= 0 and right <= 900 and bottom <= 600, bounds[-1]
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-help-minimum.png'], check=True)
            key('Escape'); key('F1')
            assert latest_topic() == 'troubleshoot.virtual', 'Help did not remember its location'
            key('Escape')
            subprocess.run(['xdotool', 'windowsize', window, '1440', '900'], check=True)
            time.sleep(.4)
            click_control(window, log_path, 'Checksum options')
            click_control(window, log_path, 'Checksum XOR')
            hover('Skip first byte')
            # Ordinary hover must not immediately expose expanded action buttons.
            time.sleep(1)
            assert 'UI control Learn more:' not in log_path.read_text()
            wait(lambda: 'UI control Learn more:' in log_path.read_text(), timeout=12)
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-help-hover.png'], check=True)
            click_control(window, log_path, 'Learn more')
            wait(lambda: latest_topic() == 'tx.checksum.skip_first')
            no_tx()
            time.sleep(.3)
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-help.png'], check=True)
            key('Escape')
            key('Escape')
            # Dialog controls start a new hover scope, and Learn more remains usable above the modal.
            key('ctrl+shift+n')
            hover('Emulated baud')
            previous = log_path.read_text().count('UI control Learn more:')
            wait(lambda: log_path.read_text().count('UI control Learn more:') > previous, timeout=12)
            click_control(window, log_path, 'Learn more')
            wait(lambda: latest_topic() == 'virtual_pair.timing')
            click_control(window, log_path, 'Help related troubleshoot.timing')
            assert latest_topic() == 'troubleshoot.timing'
            no_tx()
            key('Escape'); key('Escape')
            assert 'Created PTY pair' not in log_path.read_text(), 'Help confirmed pair creation'
            # Plain sending still works after help. Reading the guide did not toggle Skip.
            key('ctrl+l')
            subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', 'A'], check=True)
            key('Return')
            assert select.select([master], [], [], 3)[0], 'Send failed after Help'
            assert os.read(master, 64) == b'A*41', 'Help changed checksum or payload'
            no_tx()
            key('ctrl+q')
            assert process.wait(timeout=5) == 0
            print('Offline guide search/deep links/back/history, minimum layout, delayed hover Learn more including modal controls, no automatic TX, and subsequent sending passed.', flush=True)
        except BaseException:
            print(log_path.read_text()[-5000:], flush=True)
            if window:
                subprocess.run(['import', '-window', window, '/tmp/signal-forge-help-failure.png'])
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
os.close(master)
os.close(slave)
