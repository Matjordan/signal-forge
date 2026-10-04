"""Headless Linux launch and preset shortcuts using real PTYs, under xvfb-run."""
import base64
import json
import os
from pathlib import Path
import pty
import select
import subprocess
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
    process = subprocess.Popen(args, env=dict(os.environ, XDG_CONFIG_HOME=config_dir))
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
        time.sleep(0.2)
        subprocess.run(["import", "-window", "root", "/tmp/signal-forge-smoke.png"], check=True)
        subprocess.run(["convert", "/tmp/signal-forge-smoke.png", "-resize", "1280x", "-quality", "80", "/tmp/signal-forge-smoke.jpg"], check=True)
        print("Graphical launch, selected/fixed preset targets, binary repeat, and no-auto-send checks passed.", flush=True)
        if os.environ.get("SIGNAL_FORGE_REVIEW_IMAGE") == "1":
            print("SMOKE_IMAGE_BEGIN", flush=True)
            print(base64.b64encode(Path("/tmp/signal-forge-smoke.jpg").read_bytes()).decode(), flush=True)
            print("SMOKE_IMAGE_END", flush=True)
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        for master, slave in pairs:
            os.close(master)
            os.close(slave)
