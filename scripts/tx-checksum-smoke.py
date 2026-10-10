"""Verify checksum controls, exact manual/preset/repeat TX, recall and persistence."""
import json
import os
from pathlib import Path
import pty
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
with tempfile.TemporaryDirectory(prefix='signal-forge-tx-checksum-') as directory:
    root = Path(directory) / 'signal-forge'
    root.mkdir()
    settings = dict(path=path, baud=19200, data_bits=8, parity='None', stop_bits=1, flow='None')
    workspace = root / 'workspace.json'
    workspace.write_text(json.dumps(dict(version=2, ports=[settings], layout={'Leaf': {'tabs': [dict(settings=settings, ending='CrLf')], 'active': 0}})))
    common = dict(escapes=True, description='Checksum fixture', target='Selected')
    presets = [dict(common, name='Raw checksum', payload='00 FF', encoding='Hex', ending='None', checksum=dict(skip_first=False, output='Raw'), shortcut='Ctrl+1', repeat=None),
               dict(common, name='Repeated checksum', payload='#0,MED', encoding='Text', ending='CrLf', checksum=dict(skip_first=True, output='StarHex'), shortcut='Ctrl+2', repeat=dict(interval_ms=100, count=3))]
    (root / 'presets.json').write_text(json.dumps(dict(version=1, profiles=[dict(name='Bench', presets=presets)])))
    log_path = root / 'app.log'
    with log_path.open('w') as log:
        process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', path], env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
        window = None
        def key(value):
            subprocess.run(['xdotool', 'key', '--window', window, '--clearmodifiers', value], check=True)
            time.sleep(.2)
        def read(expected):
            result = b''
            deadline = time.monotonic() + 4
            while len(result) < len(expected) and time.monotonic() < deadline:
                if select.select([master], [], [], .1)[0]:
                    result += os.read(master, 4096)
            assert result == expected, (result, expected)
            assert not select.select([master], [], [], .15)[0], 'Unexpected extra TX'
        def payload(text):
            key('Escape')
            key('ctrl+l')
            key('ctrl+a')
            subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', text], check=True)
            time.sleep(.2)
        def option(control):
            click_control(window, log_path, 'Checksum options')
            click_control(window, log_path, control)
            key('Escape')
        try:
            window = window_for(process)
            time.sleep(.3)
            assert not select.select([master], [], [], .1)[0], 'Startup transmitted'
            payload('#0,MED')
            key('Return')
            read(b'#0,MED\r\n')
            option('Checksum XOR')
            payload('#0,MED')
            key('Return')
            read(b'#0,MED*73\r\n')
            option('Skip first byte')
            payload('#0,MED')
            key('Return')
            read(b'#0,MED*50\r\n')
            option('Checksum Hex')
            payload('A')
            key('Return')
            read(b'A00\r\n')
            option('Checksum Raw')
            payload('A')
            key('Return')
            read(b'A\0\r\n')
            option('Checksum Off')
            payload('A')
            key('Return')
            read(b'A\r\n')
            key('Up')  # Newest entry is the checksum-Off send.
            key('Up')  # Recall raw checksum configuration, not just payload text.
            key('Return')
            read(b'A\0\r\n')
            key('Escape')
            key('ctrl+1')
            read(b'\0\xff\xff')
            key('ctrl+2')
            read(b'#0,MED*50\r\n' * 3)
            click_control(window, log_path, 'serial:' + path + ':Repeat')
            click_control(window, log_path, 'Start repeat')
            read(b'#0,MED*50\r\n' * 3)
            # Manual repeat reuses the same checksum transform and captured settings.
            key('ctrl+s')
            saved = json.loads(workspace.read_text())
            assert saved['layout']['Leaf']['tabs'][0]['checksum'] == dict(skip_first=True, output='StarHex')
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-tx-checksum.png'], check=True)
            key('ctrl+q')
            assert process.wait(timeout=5) == 0
            process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', path], env=dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug'), stderr=log)
            window = window_for(process)
            time.sleep(.3)
            assert not select.select([master], [], [], .1)[0], 'Restoring checksum settings transmitted'
            payload('Z')
            key('Return')
            read(b'Z*00\r\n')
            key('ctrl+q')
            assert process.wait(timeout=5) == 0
            print('Checksum UI Off/XOR, skip, Hex/*Hex/Raw outputs, exact terminators, command recall, binary presets, repeating presets, persistence and no automatic TX passed.', flush=True)
        except BaseException:
            print(log_path.read_text()[-5000:], flush=True)
            if window and process.poll() is None:
                subprocess.run(['import', '-window', window, '/tmp/signal-forge-tx-checksum-failure.png'], check=False)
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            os.close(master)
            os.close(slave)
