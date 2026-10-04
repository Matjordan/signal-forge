"""Headless Linux launch and preset shortcuts using real PTYs, under xvfb-run."""
import base64
import json
import os
from pathlib import Path
import pty
import re
import select
import subprocess
import sys
import tempfile
import time
import tty


def read_bytes(master, expected):
    deadline = time.monotonic() + 3
    output = b""
    while len(output) < len(expected) and time.monotonic() < deadline:
        if select.select([master], [], [], 0.1)[0]:
            output += os.read(master, 4096)
    assert output == expected, (output, expected)


pairs = [pty.openpty(), pty.openpty()]
for master, slave in pairs:
    tty.setraw(slave)
    os.set_blocking(master, False)
args = ["target/debug/signal-forge"]
for _, slave in pairs:
    args.extend(["--port", os.ttyname(slave)])
with tempfile.TemporaryDirectory(prefix="signal-forge-smoke-") as config_dir:
    common = dict(escapes=True, ending="None", description="CI smoke fixture", shortcut="Ctrl+1")
    presets = [
        dict(common, name="Get Status", payload="STATUS\\r\\n", encoding="Text", target="Selected", repeat=None),
        dict(common, name="Binary Poll", payload="00 FF", encoding="Hex", shortcut="Ctrl+2",
             target={"Endpoint": "serial:" + os.ttyname(pairs[0][1])}, repeat={"interval_ms": 100, "count": 3}),
    ]
    profile_path = Path(config_dir) / "signal-forge" / "presets.json"
    profile_path.parent.mkdir()
    profile_path.write_text(json.dumps({"version": 1, "profiles": [{"name": "Bench", "presets": presets}]}))
    log_path = Path(config_dir) / "app.log"
    app_log = log_path.open("w")
    process = subprocess.Popen(args, env=dict(os.environ, XDG_CONFIG_HOME=config_dir), stderr=app_log)
    try:
        for index in range(40):
            if process.poll() is not None:
                raise RuntimeError(f"Application exited unexpectedly: {process.returncode}")
            for port, (master, _) in enumerate(pairs):
                os.write(master, f"Device {port + 1}: status OK, sample {index}\r\n".encode())
            time.sleep(0.1)
        # Loading profiles must never transmit automatically.
        assert not select.select([master for master, _ in pairs], [], [], 0.1)[0]
        window = subprocess.check_output(["xdotool", "search", "--name", "^Signal Forge$"]).decode().splitlines()[0]
        subprocess.run(["xdotool", "windowfocus", "--sync", window], check=True)
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+1"], check=True)
        read_bytes(pairs[1][0], b"STATUS\r\n")
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+2"], check=True)
        read_bytes(pairs[0][0], bytes([0, 255]) * 3)
        assert not select.select([master for master, _ in pairs], [], [], 0.15)[0]
        def key(keys):
            subprocess.run(["xdotool", "key", "--window", window, keys], check=True)
            time.sleep(0.06)

        def payload(x, text):
            # Fixed 1440x900 smoke viewport, before a bridge panel is opened.
            subprocess.run(["xdotool", "mousemove", "--window", window, str(x), "713", "click", "1"], check=True)
            key("ctrl+a")
            subprocess.run(["xdotool", "type", "--window", window, "--clearmodifiers", "--delay", "5", text], check=True)

        payload(950, r"alpha\r\n")
        key("Return")
        read_bytes(pairs[1][0], b"alpha\r\n")
        # Enter keeps focus, so another press sends exactly once again.
        key("Return")
        read_bytes(pairs[1][0], b"alpha\r\n")
        key("ctrl+a")
        subprocess.run(["xdotool", "type", "--window", window, "beta"], check=True)
        key("Return")
        read_bytes(pairs[1][0], b"beta")
        payload(950, "unsent draft")
        key("Up")  # beta
        key("Up")  # alpha
        key("Down")  # beta
        key("Down")  # restore unsent draft
        key("Return")
        read_bytes(pairs[1][0], b"unsent draft")
        key("Up")  # unsent draft
        key("Up")  # beta
        key("Up")  # alpha
        key("Return")
        read_bytes(pairs[1][0], b"alpha\r\n")
        payload(950, r"invalid\q")
        key("Return")
        assert not select.select([master for master, _ in pairs], [], [], 0.1)[0]
        key("Up")  # alpha; rejected input was not added
        key("Up")  # unsent draft
        key("Return")
        read_bytes(pairs[1][0], b"unsent draft")
        # The other terminal has independent hex history, including its preset.
        payload(480, "AA BB")
        key("Return")
        read_bytes(pairs[0][0], bytes([170, 187]))
        key("Up")  # AA BB
        key("Up")  # 00 FF, recalled from the earlier binary preset
        key("Return")
        read_bytes(pairs[0][0], bytes([0, 255]))
        assert not select.select([master for master, _ in pairs], [], [], 0.1)[0]
        # Leave the editor; Return and Up elsewhere cannot send a payload.
        subprocess.run(["xdotool", "mousemove", "--window", window, "950", "200", "click", "1"], check=True)
        key("Return")
        key("Up")
        assert not select.select([master for master, _ in pairs], [], [], 0.1)[0]
        # Create an owned pair through the same action as the UI button.
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+shift+n"], check=True)
        deadline = time.monotonic() + 3
        match = None
        while time.monotonic() < deadline:
            match = re.search(r"Created PTY pair bench: (\S+) <-> (\S+)", log_path.read_text())
            if match:
                break
            time.sleep(0.05)
        assert match, log_path.read_text()
        pair_paths = match.groups()
        clients = [os.open(path, os.O_RDWR | os.O_NONBLOCK) for path in pair_paths]
        try:
            os.write(clients[0], b"owned A\x00\xff")
            read_bytes(clients[1], b"owned A\x00\xff")
            os.write(clients[1], b"owned B\r\n")
            read_bytes(clients[0], b"owned B\r\n")
        finally:
            for client in clients:
                os.close(client)
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+shift+b"], check=True)
        time.sleep(0.15)
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+shift+r"], check=True)
        deadline = time.monotonic() + 3
        capture_match = None
        while time.monotonic() < deadline:
            capture_match = re.search(r"Capture started: (.+)", log_path.read_text())
            if capture_match:
                break
            time.sleep(0.05)
        assert capture_match, log_path.read_text()
        capture_path = Path(capture_match.group(1).strip())
        os.write(pairs[0][0], b"bridge A -> B\x00\xff")
        os.write(pairs[1][0], b"bridge B -> A\r\n")
        read_bytes(pairs[1][0], b"bridge A -> B\x00\xff")
        read_bytes(pairs[0][0], b"bridge B -> A\r\n")
        time.sleep(0.2)  # Let the live inspector display these chunks before pausing.
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+shift+m"], check=True)
        time.sleep(0.1)
        os.write(pairs[0][0], b"forward while display paused")
        read_bytes(pairs[1][0], b"forward while display paused")
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+shift+m"], check=True)
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+shift+r"], check=True)
        deadline = time.monotonic() + 3
        capture_lines = []
        while time.monotonic() < deadline:
            try:
                capture_lines = [json.loads(line) for line in capture_path.read_text().splitlines()]
            except json.JSONDecodeError:  # A worker flush may still be writing the last line.
                time.sleep(0.05)
                continue
            if capture_lines and capture_lines[-1]["type"] == "footer":
                break
            time.sleep(0.05)
        assert capture_lines[-1]["type"] == "footer", capture_lines
        assert capture_lines[0]["format"] == "signal-forge-capture"
        assert capture_lines[-1]["complete"] is True
        traffic = [line for line in capture_lines if line["type"] == "event"]
        for direction, expected in [("a_to_b", b"bridge A -> B\x00\xffforward while display paused"), ("b_to_a", b"bridge B -> A\r\n")]:
            actual = b"".join(bytes(line["raw_bytes"]) for line in traffic if line["direction"] == direction)
            assert actual == expected, (actual, expected)
        assert all(int(line["timestamp_unix_ns"]) > 0 for line in traffic)
        time.sleep(0.2)
        subprocess.run(["import", "-window", "root", "/tmp/signal-forge-smoke.png"], check=True)
        subprocess.run(["convert", "/tmp/signal-forge-smoke.png", "-resize", "1280x", "-quality", "80", "/tmp/signal-forge-smoke.jpg"], check=True)
        print("Graphical launch, selected/fixed preset targets, binary repeat, owned PTY creation, duplex bridging, paused monitoring, JSONL capture export, Enter sends, per-terminal Up/Down recall, and no-auto-send checks passed.", flush=True)
        if os.environ.get("SIGNAL_FORGE_REVIEW_IMAGE") == "1":
            print("SMOKE_IMAGE_BEGIN", flush=True)
            print(base64.b64encode(Path("/tmp/signal-forge-smoke.jpg").read_bytes()).decode(), flush=True)
            print("SMOKE_IMAGE_END", flush=True)
    finally:
        if sys.exc_info()[0] is not None and "window" in locals():
            subprocess.run(["import", "-window", "root", "/tmp/signal-forge-smoke.png"], check=False)
            subprocess.run(["convert", "/tmp/signal-forge-smoke.png", "-resize", "1280x", "/tmp/signal-forge-smoke-failure.jpg"], check=False)
            print("SMOKE_FAILURE_IMAGE_BEGIN", flush=True)
            print(base64.b64encode(Path("/tmp/signal-forge-smoke-failure.jpg").read_bytes()).decode(), flush=True)
            print("SMOKE_FAILURE_IMAGE_END", flush=True)
        subprocess.run(["xdotool", "key", "--window", window, "ctrl+q"], check=False) if "window" in locals() else process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        if "window" in locals() and "pair_paths" in locals():
            assert process.returncode == 0, log_path.read_text()
        app_log.close()
        if "capture_path" in locals():
            capture_path.unlink(missing_ok=True)
        if "pair_paths" in locals():
            assert all(not Path(path).exists() for path in pair_paths), "Owned PTYs survived application shutdown"
        for master, slave in pairs:
            os.close(master)
            os.close(slave)
