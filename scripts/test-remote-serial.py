#!/usr/bin/env python3
"""Run real SSH/PTY integration tests with isolated keys, config and known_hosts."""
import getpass
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='signal-forge-ssh-') as directory:
    temporary = Path(directory)
    for name in ('host', 'client', 'untrusted-host', 'unauthorized-client'):
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(temporary / name)], check=True)
    authorized = temporary / 'authorized_keys'
    authorized.write_bytes((temporary / 'client.pub').read_bytes())
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    config = temporary / 'sshd_config'
    config.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {temporary / 'host'}
PidFile {temporary / 'sshd.pid'}
AuthorizedKeysFile {authorized}
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM no
PermitRootLogin prohibit-password
AllowUsers {getpass.getuser()}
LogLevel VERBOSE
''')
    host_key = (temporary / 'host.pub').read_text().split()
    known = temporary / 'known_hosts'
    known.write_text(f'[127.0.0.1]:{port} {host_key[0]} {host_key[1]}\n')
    client_config = temporary / 'ssh_config'
    client_config.write_text(f'''Host forge-test
  HostName 127.0.0.1
  Port {port}
  User {getpass.getuser()}
  IdentityFile {temporary / 'client'}
  IdentitiesOnly yes
  UserKnownHostsFile {known}
  GlobalKnownHostsFile /dev/null
''')
    # Use a real OpenSSH client with an isolated config, never the user's files.
    wrapper = temporary / 'ssh'
    client = shutil.which('ssh')
    wrapper.write_text(f'#!/bin/sh\nexec "{client}" -F "{client_config}" "$@"\n')
    wrapper.chmod(0o755)
    env = dict(os.environ, PATH=str(temporary) + os.pathsep + os.environ['PATH'],
               SIGNAL_FORGE_TEST_SSH_HOST='forge-test', SIGNAL_FORGE_TEST_KNOWN_HOSTS=str(known),
               SIGNAL_FORGE_TEST_SSH_CONFIG=str(client_config),
               SIGNAL_FORGE_TEST_CLIENT_KEY=str(temporary / 'client'),
               SIGNAL_FORGE_TEST_UNAUTHORIZED_KEY=str(temporary / 'unauthorized-client'))
    alternate_key = (temporary / 'untrusted-host.pub').read_text().split()
    env['SIGNAL_FORGE_TEST_CHANGED_KNOWN_HOSTS'] = f'[127.0.0.1]:{port} {alternate_key[0]} {alternate_key[1]}\n'
    agent_socket = temporary / 'agent.sock'
    env['SSH_AUTH_SOCK'] = str(agent_socket)
    agent = subprocess.Popen(['ssh-agent', '-D', '-a', str(agent_socket)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 5
    while not agent_socket.exists():
        if agent.poll() is not None or time.monotonic() > deadline:
            agent.terminate()
            agent.wait(timeout=5)
            raise RuntimeError('Isolated SSH agent did not start')
        time.sleep(.05)
    log_path = temporary / 'sshd.log'
    with log_path.open('w') as log:
        server = None
        try:
            server = subprocess.Popen([shutil.which('sshd') or '/usr/sbin/sshd', '-D', '-e', '-f', str(config)], stderr=log)
            deadline = time.monotonic() + 5
            while True:
                if server.poll() is not None:
                    raise RuntimeError(log_path.read_text())
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=.2):
                        break
                except OSError:
                    if time.monotonic() > deadline:
                        raise RuntimeError('Test SSH server did not start: ' + log_path.read_text())
                    time.sleep(.1)
            subprocess.run(['cargo', 'test', '--locked', '--test', 'remote_serial', '--', '--ignored', '--test-threads=1', '--nocapture'], cwd=root, env=env, check=True)
            # The trust-rejection test intentionally empties this isolated file.
            known.write_text(f'[127.0.0.1]:{port} {host_key[0]} {host_key[1]}\n')
            # Restore key-file authentication after the agent/authentication tests.
            client_config.write_text(f'''Host forge-test
  HostName 127.0.0.1
  Port {port}
  User {getpass.getuser()}
  IdentityFile {temporary / 'client'}
  IdentitiesOnly yes
  UserKnownHostsFile {known}
  GlobalKnownHostsFile /dev/null
''')
            subprocess.run(['cargo', 'build', '--locked', '--bin', 'signal-forge'], cwd=root, env=env, check=True)
            subprocess.run(['xvfb-run', '-a', '-s', '-screen 0 2560x1440x24 -noreset',
                            'python3', 'scripts/remote-ui-smoke.py'], cwd=root, env=env, check=True)
        except Exception:
            print(log_path.read_text(), flush=True)
            raise
        finally:
            if server is not None:
                server.terminate()
                server.wait(timeout=5)
            agent.terminate()
            agent.wait(timeout=5)
