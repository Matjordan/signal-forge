"""Compare Signal Forge with socat using independent clients and common terminals, without elevation."""
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import subprocess
import sys
import tempfile
import termios
import time
import tty

assert os.geteuid() != 0, 'Run this compatibility suite without sudo/root'
for executable in ['socat', 'picocom', 'minicom', 'screen']:
    assert shutil.which(executable), f'Missing test tool: {executable}'
subprocess.run(['cargo', 'build', '--locked', '--example', 'virtual-pair-fixture'], check=True)

def read(fd, timeout=.2):
    output = b''
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if select.select([fd], [], [], .02)[0]:
            try:
                chunk = os.read(fd, 65536)
            except BlockingIOError:
                continue
            except OSError:
                break
            if not chunk:
                break
            output += chunk
    return output

client = r'''
import os,sys,base64,time,tty,select
fd=os.open(sys.argv[1],os.O_RDWR|os.O_NOCTTY|os.O_NONBLOCK)
tty.setraw(fd)
payload=base64.b64decode(sys.argv[2]);expected=base64.b64decode(sys.argv[3])
print('READY',flush=True);sys.stdin.readline()
assert os.write(fd,payload)==len(payload)
output=b'';deadline=time.monotonic()+5
while len(output)<len(expected) and time.monotonic()<deadline:
 if select.select([fd],[],[],.05)[0]:output+=os.read(fd,4096)
assert output==expected,(len(output),len(expected),output[:32])
print('DONE',flush=True);sys.stdin.readline()
os.close(fd)
'''
with tempfile.TemporaryDirectory(prefix='signal-forge-compat-') as directory:
    root = Path(directory)
    for backend in ['socat', 'signal-forge']:
        folder = root / backend
        folder.mkdir()
        log_path = folder / 'relay.log'
        log = log_path.open('w')
        if backend == 'socat':
            paths = [str(folder / 'a'), str(folder / 'b')]
            pair = subprocess.Popen(['socat', '-d', '-d', *[f'PTY,raw,echo=0,link={path}' for path in paths]], stderr=log)
            deadline = time.monotonic() + 5
            while not all(Path(path).exists() for path in paths):
                assert time.monotonic() < deadline and pair.poll() is None, log_path.read_text()
                time.sleep(.02)
        else:
            pair = subprocess.Popen(['target/debug/examples/virtual-pair-fixture', str(folder)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True)
            paths = json.loads(pair.stdout.readline())['paths']
        try:
            # Two independent external processes, all byte values, simultaneous bidirectional exchange.
            payload = bytes(range(256)) * 8
            reverse = bytes(reversed(range(256))) * 8
            enc = lambda value: base64.b64encode(value).decode()
            processes = [subprocess.Popen([sys.executable, '-c', client, path, enc(sent), enc(expected)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                         for path, sent, expected in [(paths[0], payload, reverse), (paths[1], reverse, payload)]]
            for process in processes:
                assert select.select([process.stdout], [], [], 5)[0]
                assert process.stdout.readline() == b'READY\n'
            for process in processes:
                process.stdin.write(b'go\n'); process.stdin.flush()
            for process in processes:
                assert select.select([process.stdout], [], [], 5)[0]
                assert process.stdout.readline() == b'DONE\n'
            for process in processes:
                process.stdin.write(b'exit\n'); process.stdin.flush()
                output, error = process.communicate(timeout=8)
                assert process.returncode == 0, (backend, output, error)
            for tool in ['picocom', 'minicom', 'screen']:
                master, slave = pty.openpty()
                os.set_blocking(master, False)
                screen_dir = folder / 'screen'
                screen_dir.mkdir(mode=0o700, exist_ok=True)
                environment = dict(os.environ, TERM='xterm', SCREENRC='/dev/null', SCREENDIR=str(screen_dir))
                name = f'forge-compat-{backend}-{os.getpid()}'
                args = {
                    'picocom': ['picocom', '--baud', '19200', paths[1]],
                    'minicom': ['minicom', '-o', '-b', '19200', '-D', paths[1]],
                    'screen': ['screen', '-S', name, '-ln', paths[1], '19200'],
                }[tool]
                def controlling_terminal():
                    os.setsid()
                    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
                recovery_fd = os.open(paths[1], os.O_RDWR | os.O_NOCTTY) if backend == 'socat' else None
                process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_terminal, env=environment)
                peer = None
                try:
                    time.sleep(.4)
                    startup = read(master)
                    assert process.poll() is None, (backend, tool, startup)
                    peer = os.open(paths[0], os.O_RDWR | os.O_NONBLOCK | os.O_NOCTTY)
                    tty.setraw(peer)
                    os.write(master, b'client-to-device\r')
                    assert read(peer, .5) == b'client-to-device\r', (backend, tool, startup)
                    os.write(peer, b'device-to-client\r\n')
                    assert b'device-to-client' in read(master, .5), (backend, tool)
                    print(f'{backend}: {tool} bidirectional exchange passed (uid {os.geteuid()})', flush=True)
                finally:
                    if tool == 'screen':
                        subprocess.run(['screen', '-S', name, '-X', 'quit'], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=2, check=False)
                    if process.poll() is None:
                        process.terminate()
                    try:
                        process.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                    if peer is not None:
                        os.close(peer)
                    os.close(master)
                    os.close(slave)
                    try:
                        reopened = os.open(paths[1], os.O_RDWR | os.O_NOCTTY)
                    except OSError as error:
                        assert error.errno == 16, error
                        if backend == 'signal-forge':
                            pair.stdin.write('reset 1\n'); pair.stdin.flush()
                            assert json.loads(pair.stdout.readline())['released']
                        else:
                            fcntl.ioctl(recovery_fd, termios.TIOCNXCL)
                        reopened = os.open(paths[1], os.O_RDWR | os.O_NOCTTY)
                        print(f'{backend}: {tool} stale exclusive flag recovered without privilege', flush=True)
                    os.close(reopened)
                    if recovery_fd is not None:
                        os.close(recovery_fd)
            # Same-side TIOCEXCL blocks another opener on both backends. Explicit release restores access.
            fd = os.open(paths[0], os.O_RDWR | os.O_NOCTTY)
            try:
                fcntl.ioctl(fd, termios.TIOCEXCL)
                try:
                    unexpected = os.open(paths[0], os.O_RDWR | os.O_NOCTTY)
                except OSError as error:
                    assert error.errno == 16, error
                else:
                    os.close(unexpected)
                    raise AssertionError('Exclusive opener did not exclude another same-side client')
                peer = os.open(paths[1], os.O_RDWR | os.O_NOCTTY)
                os.close(peer)
                fcntl.ioctl(fd, termios.TIOCNXCL)
            finally:
                os.close(fd)
            reopened = os.open(paths[0], os.O_RDWR | os.O_NOCTTY)
            os.close(reopened)
        finally:
            if backend == 'signal-forge':
                pair.stdin.close()
            else:
                pair.terminate()
            pair.wait(timeout=3)
            log.close()
        assert not any(Path(path).exists() for path in paths), (backend, 'stale paths')
print('Socat comparison, two external binary clients, picocom/minicom/screen, same-side exclusive access, reopen and cleanup passed without elevated privileges.', flush=True)
