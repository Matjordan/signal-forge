"""Live TCP/UDP GUI sends, explicit reply target, and disconnected restoration."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from gui_smoke_support import click_control, window_for

with tempfile.TemporaryDirectory(prefix='signal-forge-network-') as directory:
    log_path = Path(directory) / 'app.log'
    env = dict(os.environ, XDG_CONFIG_HOME=directory, RUST_LOG='signal_forge=debug')
    tcp = socket.socket(); tcp.bind(('127.0.0.1', 0)); tcp.listen(); tcp.settimeout(4)
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); udp.bind(('127.0.0.1', 0)); udp.settimeout(4)
    uri = f'tcp://127.0.0.1:{tcp.getsockname()[1]}?bind=127.0.0.1:0&timeout=5000'
    udp_uri = f'udp://127.0.0.1:{udp.getsockname()[1]}?bind=127.0.0.1:0&timeout=5000'
    reply = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); reply.bind(('127.0.0.1', 0)); reply.settimeout(3)
    alternate = socket.socket(); alternate.bind(('127.0.0.1', 0)); alternate.listen(); alternate.settimeout(4)
    process = None
    def key(value):
        subprocess.run(['xdotool', 'key', '--window', window, '--clearmodifiers', value], check=True)
        time.sleep(.15)
    def replace(control, text):
        click_control(window, log_path, control); key('ctrl+a'); key('BackSpace')
        subprocess.run(['xdotool', 'type', '--window', window, '--clearmodifiers', text], check=True)
        time.sleep(.15)
    def wait(predicate):
        deadline = time.monotonic() + 6
        while not predicate():
            assert process.poll() is None, log_path.read_text()[-4000:]
            assert time.monotonic() < deadline, log_path.read_text()[-4000:]
            time.sleep(.05)
    def tabs(layout):
        return layout['Leaf']['tabs'] if 'Leaf' in layout else tabs(layout['Split']['first']) + tabs(layout['Split']['second'])
    with log_path.open('w') as log:
        try:
            process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check', '--port', uri, '--port', udp_uri], env=env, stderr=log)
            stream, _ = tcp.accept(); stream.settimeout(3)
            window = window_for(process)
            wait(lambda: f'UI control serial:{uri}:Payload editor' in log_path.read_text())
            replace(f'serial:{uri}:Payload editor', r'hello\r\n'); key('Return')
            assert stream.recv(100) == b'hello\r\n'
            stream.sendall(b'TCP response\r\n')
            replace(f'serial:{udp_uri}:Payload editor', r'\x00\xFF'); key('Return')
            data, address = udp.recvfrom(100); assert data == bytes([0, 255]), data
            udp.sendto(b'', address); udp.sendto(b'A\nB\r\n', address)
            replace(f'serial:{udp_uri}:Payload editor', ''); key('Return')
            assert udp.recvfrom(100)[0] == b'', 'Empty UDP send was suppressed'
            subprocess.run(['xdotool', 'windowsize', window, '900', '600'], check=True)
            time.sleep(.4)
            # A foreign sender must not change the chosen destination.
            reply.sendto(b'foreign sender', address)
            replace(f'serial:{udp_uri}:Payload editor', 'original'); key('Return')
            assert udp.recvfrom(100)[0] == b'original'
            replace('UDP reply target', f'127.0.0.1:{reply.getsockname()[1]}')
            click_control(window, log_path, 'Set UDP target')
            replace(f'serial:{udp_uri}:Payload editor', 'reply'); key('Return')
            assert reply.recvfrom(100)[0] == b'reply'
            saved_udp_uri = udp_uri + f'&reply=127.0.0.1:{reply.getsockname()[1]}'
            click_control(window, log_path, 'Network…')
            click_control(window, log_path, 'Cancel network')
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-network-minimum.png'], check=True)
            subprocess.run(['xdotool', 'windowsize', window, '1440', '900'], check=True)
            time.sleep(.4)
            subprocess.run(['import', '-window', window, '/tmp/signal-forge-network.png'], check=True)
            # Create another client through the actual setup dialog.
            click_control(window, log_path, 'Network…')
            replace('Network host', '127.0.0.1')
            replace('Network remote port', str(alternate.getsockname()[1]))
            click_control(window, log_path, 'Open network')
            alternate_stream, _ = alternate.accept(); alternate_stream.settimeout(3)
            alternate_uri = f'tcp://127.0.0.1:{alternate.getsockname()[1]}?bind=127.0.0.1:0&timeout=5000'
            wait(lambda: f'UI control serial:{alternate_uri}:Payload editor' in log_path.read_text())
            replace(f'serial:{alternate_uri}:Payload editor', 'dialog'); key('Return')
            assert alternate_stream.recv(100) == b'dialog'
            key('ctrl+s')
            workspace = Path(directory) / 'signal-forge' / 'workspace.json'
            wait(workspace.exists)
            saved = json.loads(workspace.read_text())
            assert {t['settings']['path'] for t in tabs(saved['layout'])} == {uri, saved_udp_uri, alternate_uri}
            process.terminate(); process.wait(timeout=4); stream.close(); alternate_stream.close()
            process = subprocess.Popen(['target/debug/signal-forge', '--no-update-check'], env=env, stderr=log)
            window = window_for(process)
            time.sleep(.4)
            tcp.settimeout(.3)
            try:
                unexpected, _ = tcp.accept(); unexpected.close(); raise AssertionError('Restore opened TCP automatically')
            except socket.timeout:
                pass
            udp.settimeout(.3)
            try:
                udp.recvfrom(100); raise AssertionError('Restore transmitted UDP automatically')
            except socket.timeout:
                pass
            key('ctrl+s')
            assert {t['settings']['path'] for t in tabs(json.loads(workspace.read_text())['layout'])} == {uri, saved_udp_uri, alternate_uri}
        finally:
            if process and process.poll() is None:
                process.terminate(); process.wait(timeout=4)
            tcp.close(); udp.close(); reply.close(); alternate.close()
print('Live TCP/UDP GUI sends, binary and empty datagrams, setup cancellation, and disconnected/no-TX restoration passed.')
