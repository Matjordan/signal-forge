"""Executed by python3 over a non-PTY SSH channel; stdout is serial bytes only."""
import glob
import json
import os
import select
import sys
import termios

try:
    if sys.argv[1] == 'discover':
        paths = sorted(set(glob.glob('/dev/serial/by-id/*'))) + sorted(set(
            glob.glob('/dev/ttyUSB*') + glob.glob('/dev/ttyACM*') + glob.glob('/dev/ttyS*')))
        print(json.dumps(paths))
        sys.exit(0)
    settings = json.loads(sys.argv[1])
    try:
        device = os.open(settings['path'], os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError as error:
        raise RuntimeError('Remote device access: ' + str(error)) from error
    try:
        speed = getattr(termios, 'B' + str(settings['baud']), None)
        if speed is None:
            raise ValueError('unsupported baud rate ' + str(settings['baud']))
        attrs = termios.tcgetattr(device)
        attrs[0] = 0
        attrs[1] = 0
        attrs[2] = termios.CLOCAL | termios.CREAD | getattr(termios, 'CS' + str(settings['data_bits']))
        attrs[3] = 0
        attrs[4] = attrs[5] = speed
        attrs[6][termios.VMIN] = 1
        attrs[6][termios.VTIME] = 0
        if settings['parity'] != 'None':
            attrs[2] |= termios.PARENB
            attrs[0] |= termios.INPCK
            if settings['parity'] == 'Odd':
                attrs[2] |= termios.PARODD
        if settings['stop_bits'] == 2:
            attrs[2] |= termios.CSTOPB
        if settings['flow'] == 'Hardware':
            attrs[2] |= termios.CRTSCTS
        elif settings['flow'] == 'Software':
            attrs[0] |= termios.IXON | termios.IXOFF
        termios.tcsetattr(device, termios.TCSANOW, attrs)
        actual = termios.tcgetattr(device)
        framing_mask = termios.CSIZE | termios.PARENB | termios.PARODD | termios.CSTOPB | termios.CRTSCTS
        if (actual[0:2] != attrs[0:2] or actual[3:6] != attrs[3:6]
                or (actual[2] & framing_mask) != (attrs[2] & framing_mask)):
            raise ValueError('device rejected serial framing or flow control')
    except Exception as error:
        raise RuntimeError('Remote serial configuration: ' + str(error)) from error
    for descriptor in (0, 1):
        os.set_blocking(descriptor, False)
    print('SIGNAL_FORGE_READY', file=sys.stderr, flush=True)
    to_device = bytearray()
    to_client = bytearray()
    while True:
        reads = ([0] if len(to_device) < 65536 else []) + ([device] if len(to_client) < 65536 else [])
        writes = ([device] if to_device else []) + ([1] if to_client else [])
        readable, writable, _ = select.select(reads, writes, [])
        for descriptor in readable:
            try:
                data = os.read(descriptor, 4096)
            except BlockingIOError:
                continue
            if not data:
                if descriptor == 0:
                    sys.exit(0)
                raise RuntimeError('Remote serial device closed')
            (to_device if descriptor == 0 else to_client).extend(data)
        for descriptor in writable:
            buffer = to_device if descriptor == device else to_client
            try:
                count = os.write(descriptor, buffer)
            except BlockingIOError:
                continue
            if not count:
                raise RuntimeError('Remote serial write returned zero')
            del buffer[:count]
except Exception as error:
    print(str(error), file=sys.stderr, flush=True)
    sys.exit(1)
finally:
    if 'device' in globals():
        os.close(device)
