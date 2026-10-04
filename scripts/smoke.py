"""Headless Linux launch smoke using real PTYs. Run under xvfb-run."""
import os
import pty
import subprocess
import time
import tty

pairs = [pty.openpty(), pty.openpty()]
for master, slave in pairs:
    tty.setraw(slave)
args = ["target/debug/signal-forge"]
for _, slave in pairs:
    args.extend(["--port", os.ttyname(slave)])
process = subprocess.Popen(args)
try:
    for index in range(40):
        if process.poll() is not None:
            raise RuntimeError(f"Application exited unexpectedly: {process.returncode}")
        for port, (master, _) in enumerate(pairs):
            os.write(master, f"Device {port + 1}: status OK, sample {index}\r\n".encode())
        time.sleep(0.1)
    subprocess.run(["import", "-window", "root", "/tmp/signal-forge-smoke.png"], check=True)
    subprocess.run(["convert", "/tmp/signal-forge-smoke.png", "-resize", "1280x", "-quality", "80", "/tmp/signal-forge-smoke.jpg"], check=True)
    print("Graphical launch succeeded with two PTYs; screenshot: /tmp/signal-forge-smoke.png", flush=True)
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
