# Signal Forge

A Linux serial-port workbench implemented in Rust with egui/eframe. The initial implementation provides independently configured serial connections, dockable terminals, and validated text/hex sending.

## Current implementation

![Signal Forge running with two Linux PTYs](./docs/workbench.jpg)

## Target UI

![Signal Forge target UI](./a_detailed_widescreen_dark_themed_desktop_applicat.png)

This concept image is the design reference for the finished application. The current implementation follows its dark blue desktop shell, endpoint navigation, green connection indicators, per-pane configuration controls, black terminal surfaces, blue send/disconnect buttons, and dockable port tiles. The mockup's preset, sequence, virtual-pair, and bridge panels will be added as their issues are implemented.

## Build and run on Linux

Install a current stable Rust toolchain with Cargo. On Debian/Ubuntu:

```sh
sudo apt-get install build-essential pkg-config libudev-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libxi-dev libgl1-mesa-dev
cargo run
```

Run from a graphical X11 or Wayland session. For diagnostic logging:

```sh
RUST_LOG=signal_forge=debug cargo run
cargo test --all-targets
cargo build --release
```

The executable is `target/release/signal-forge`. To explicitly open devices at launch, pass `--port /dev/ttyUSB0 --port /dev/ttyUSB1`. Your account needs read/write access to its serial devices; on many distributions this is provided by the `dialout` or `uucp` group. The application reports permission/open errors in its status bar.

## Using the workbench

1. Refresh devices, choose a device or type an existing `/dev/pts/N` path, and set baud, data bits, parity, stop bits, and flow control.
2. Open a terminal. Opening a second device initially splits the workspace side-by-side. Drag tabs to dock and rearrange them; closing a tab disconnects its device. Each pane has Disconnect/Reconnect controls; disconnect to edit its settings, then reconnect.
3. Choose Text or Hex bytes in that terminal. Text optionally interprets `\r`, `\n`, `\t`, `\0`, `\xNN`, and `\\`. Hex accepts pairs of digits, optionally separated by whitespace, such as `00 FF 0D 0A`.
4. Choose an explicit line ending: None, CR, LF, or CRLF. Click Send or press Enter in the payload input. Invalid input is rejected before any bytes are queued. TX rows report bytes accepted by the serial writer.
5. Toggle timestamps (UTC), hex rendering, and auto-scroll. Clear removes displayed history. Pause display discards new rows while serial I/O continues.
6. Save port settings explicitly to `$XDG_CONFIG_HOME/signal-forge/workspace.json` (or `~/.config/signal-forge/workspace.json`). Saved devices are not opened or transmitted to automatically. Corrupt/unsupported configuration is reported and preserved; saving is disabled until the file is repaired.

Each terminal retains up to 2,000 rows. RX and TX have distinct labels and colors. Binary bytes are rendered with escapes in text mode. A bounded monitoring queue reports dropped events in the status bar. Monitoring is best-effort, not a lossless capture mechanism.

## Architecture

- `endpoint`: stable IDs, connection states, errors, transport-independent asynchronous TX interface.
- `traffic`: timestamped raw-byte RX/TX events, shared payloads, ordered nonblocking fan-out.
- `serial`: device discovery and a worker per open port; all physical I/O is outside widgets.
- `send`: pure, all-or-nothing escape/hex encoding and explicit line endings.
- `config`: versioned serial-setting persistence with explicit save and atomic file replacement.
- `app`: device controls, dock layout, bounded/virtualized terminal views, and send controls.

A slow subscriber loses monitoring events instead of blocking a serial worker. Sequence numbers expose gaps. Future bridges must forward through a dedicated lossless worker RX route, rather than subscribe to the lossy monitor bus. Closing a terminal stops its worker and drops its device handle; closing the application drops all terminals.

## Issue progress

This first implementation addresses the foundation (#1), event model (#2), serial management (#3), dockable terminals (#4), and manual send engine (#5). Linux build, formatting, all eight unit/PTY tests, and graphical launch with two PTYs pass in CI. Physical-device and interactive docking validation remain outstanding before closing these issues. PTY tests cover real byte flow, independent endpoints, and disconnect/reopen behavior, providing the first part of #12.

Next in issue order: repeated sending (#6), preset profiles (#7), owned PTY pairs (#8), full-duplex bridging (#9), inspector/capture (#10), full workspace persistence (#11), expanded integration tests (#12), and packaging/usability (#13). Only port settings are persisted today; dock layout and profiles are not yet saved.

## Manual smoke checklist

- Start under X11/Wayland; confirm the window and device refresh work.
- Open two devices with independent settings and rearrange their tabs.
- Send `test\r\n`, then `00 FF` in hex mode; verify bytes with a peer.
- Reject an unknown escape and malformed hex without transmitting.
- Verify RX/TX labels, hex/text modes, timestamps, pause, clear, and auto-scroll.
- Unplug a device, confirm an endpoint error, close its tab, then reconnect.
- Save settings and restart; confirm no device opens or sends automatically.

CI builds every target and runs unit and Linux PTY integration tests without physical hardware. The initial Linux CI build and all eight unit/PTY tests passed. Local builds are unavailable in the implementation environment, which lacks Rust and has an unavailable network proxy. CI also starts the app under Xvfb with two real PTY endpoints; physical-device and interactive docking smoke validation remain manual.
