"""Bounded X11 readiness checks shared by the graphical smoke tests."""
import subprocess
import time


def click(window, x, y):
    """Let egui observe pointer movement before delivering the mouse click."""
    subprocess.run(['xdotool', 'mousemove', '--window', window, str(x), str(y)], check=True)
    time.sleep(.1)
    subprocess.run(['xdotool', 'click', '1'], check=True)
    time.sleep(.1)


def window_for(process, timeout=10):
    """Wait for this process's mapped window and confirmed keyboard focus."""
    deadline = time.monotonic() + timeout
    last_error = 'No visible application window'
    while time.monotonic() < deadline:
        assert process.poll() is None, f'Application exited: {process.returncode}'
        found = subprocess.run(
            ['xdotool', 'search', '--onlyvisible', '--pid', str(process.pid),
             '--name', '^Signal Forge$'], capture_output=True, text=True, timeout=1)
        if found.returncode == 0:
            for window in found.stdout.splitlines():
                # A named window may not yet be viewable. Retry BadMatch, and
                # avoid --sync, which can wait indefinitely for X11 focus.
                focused = subprocess.run(
                    ['xdotool', 'windowfocus', window],
                    capture_output=True, text=True, timeout=1)
                if focused.returncode != 0:
                    last_error = focused.stderr.strip()
                    continue
                actual = subprocess.run(
                    ['xdotool', 'getwindowfocus'],
                    capture_output=True, text=True, timeout=1)
                if actual.returncode == 0 and actual.stdout.strip() == window:
                    return window
                last_error = 'Application window did not retain keyboard focus'
        time.sleep(.1)
    raise AssertionError(f'Application window was not ready within {timeout}s: {last_error}')
