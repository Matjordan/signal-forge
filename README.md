# Signal Forge

A Linux serial-port workbench implemented in Rust with egui/eframe. The initial implementation provides independently configured serial connections, dockable terminals, validated text/hex sending, independent repeated sends, saved preset profiles, owned virtual PTY pairs, and full-duplex bridges.

## Current implementation

![Signal Forge running with two Linux PTYs](./docs/workbench.jpg)

## Target UI

![Signal Forge target UI](./a_detailed_widescreen_dark_themed_desktop_applicat.png)

This concept image is the design reference for the finished application. The current implementation follows its dark blue desktop shell, endpoint navigation, green connection indicators, per-pane configuration controls, black terminal surfaces, blue send/disconnect buttons, and dockable port tiles. Preset profiles, virtual pairs, and bridges now have controls in the sidebars, with directional bridge monitors below the terminals.

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
5. Set **Repeat every** in milliseconds, choose a finite count or **Until stopped**, and click **Start repeat**. The count includes the initial send. Each terminal has one independent repeat job; its progress counts fully written payloads. The job takes a snapshot of the encoded payload, so editing the send box does not alter an active repeat. **Stop repeat**, disconnecting, closing the tab, or a device fault stops further repeated writes. Bytes already accepted by the serial driver cannot be recalled. Intervals are best-effort and begin after each full write, without catch-up bursts.
6. Toggle timestamps (UTC), hex rendering, and auto-scroll. Clear removes displayed history. Pause display discards new rows while serial I/O continues.
7. Save port settings explicitly to `$XDG_CONFIG_HOME/signal-forge/workspace.json` (or `~/.config/signal-forge/workspace.json`). Saved devices are not opened or transmitted to automatically. Corrupt/unsupported configuration is reported and preserved; saving is disabled until the file is repaired.

Each terminal retains up to 2,000 rows. RX and TX have distinct labels and colors. Binary bytes are rendered with escapes in text mode. A bounded monitoring queue reports dropped events in the status bar. Monitoring is best-effort, not a lossless capture mechanism.

## Preset profiles

The right sidebar contains the active profile and its reusable commands. Choose the preset target terminal (clicking a terminal pane also selects it), then click a preset name to send. **New preset** opens an editor for name, payload, encoding, escapes, line ending, selected/fixed endpoint target, description, shortcut, and optional repeat settings. **Edit**, **Delete**, **Up**, and **Down** update the profile immediately. A repeating preset refuses to replace a running repeat; stop that job first.

Create named device/project profiles with **Create profile**. Changes are validated and saved separately from workspace settings in `signal-forge/presets.json` under the configuration directory. Presets never send merely because a profile is loaded or imported. Invalid or unsupported files are preserved and reported rather than silently replaced.

Use **Export profile** and **Import profile** with a JSON file path. The human-readable format is `{ "version": 1, "profile": { "name": "Bench", "presets": [...] } }`; it preserves all send and repeat settings. Import replaces a profile with the same name and rejects invalid payloads, schedules, or conflicting shortcuts before modifying the library. Shortcuts are **Ctrl+1** through **Ctrl+9**, scoped to the active profile and suppressed while editing text or a preset.

## Virtual pairs and bridges

In the left sidebar, enter a pair name and click **Create PTY pair** (or **Ctrl+Shift+N**). The two raw Linux PTY paths are displayed with **Copy path** and **Open A/B** actions. External programs can open either end; bytes written to A arrive at B and vice versa. Multiple pairs are independent. Optionally enter a writable **Link directory** to create `<name>-a` and `<name>-b` symlinks. Existing paths are never overwritten, and failed creation rolls back any link already created.

**Remove pair** disconnects its app terminals and releases its relay and owned links. Closing the application does the same; **Ctrl+Q** closes the application normally. PTY paths last only for the lifetime of the pair. The internal relay uses bounded buffers and retains partial writes under backpressure; it does not require `socat`.

Open two terminals, choose endpoints **A** and **B**, then click **Start full-duplex bridge** (or **Ctrl+Shift+B**). Bridges support serial devices and PTY paths using the same endpoint interface. Each endpoint can belong to one running bridge. RX from A is sent to B and RX from B is sent to A; manual and repeated sends remain available and serialize with bridge writes.

The lower bridge monitor labels each direction **A → B** or **B → A**. **Pause display** (or **Ctrl+Shift+M** for all bridge monitors) discards new display rows while forwarding continues. **Stop / remove** leaves both endpoints open. Disconnecting either endpoint faults the bridge; restoring a connection requires starting a new bridge explicitly. Bounded monitor queues can drop display events without affecting forwarded bytes. Transport write failures fault the bridge; already accepted bytes cannot be recalled.

## Bridge traffic inspector and captures

The bridge panel shows full UTC timestamps, direction, ASCII escapes, and raw hex together. **Delta times** adds the time since the previous observed chunk; it uses the complete chronological stream even when direction filters hide rows. Choose **Both directions**, **A → B**, or **B → A** without discarding the other direction's retained history. Sequence gaps are marked explicitly. The inspector retains at most 1,000 chunks per bridge and renders only visible rows; **Auto-scroll**, **Clear display**, and **Pause display** control presentation.

Enter a writable **Capture file** path and click **Start capture**. **Ctrl+Shift+R** starts/stops recording on the first bridge. Recording always includes both directions, independently of display filters, clear, and pause. A dedicated worker streams raw bytes and metadata directly to disk through its own bounded queue. Existing files are never overwritten; choose a new filename to start another capture. **Stop capture** changes the state to **Finishing**, then **Completed** after queued events and the end summary are saved. Endpoint faults, removing a bridge, and normal shutdown also finish recording.

Captures use [versioned JSON Lines](./docs/capture-format.md), with raw byte arrays, exact Unix nanosecond timestamps, signed deltas, source IDs, and direction. The footer records the capture's own dropped-event count; overloaded captures report incomplete data while forwarding continues. UI history limits do not limit file length. Files without a footer were interrupted or encountered a write failure.

## Architecture

- `endpoint`: stable IDs, connection states, errors, transport-independent asynchronous TX interface.
- `traffic`: timestamped raw-byte RX/TX events, shared payloads, ordered nonblocking fan-out.
- `serial`: device discovery and a worker per open port; all physical I/O is outside widgets.
- `send`: pure, all-or-nothing escape/hex encoding and explicit line endings.
- `presets`: validated command/profile models, atomic persistence, and versioned JSON import/export.
- `repeat`: transport-independent scheduling, finite counts, progress, and cancellation tokens; serial workers drive timers independently of GUI repainting.
- `virtual_pair`: owned raw Linux PTYs, optional named links, and a bounded full-duplex relay.
- `bridge`: transport-neutral RX routes, serialized destination writes, lifecycle control, and independent directional monitoring.
- `inspector`: bounded display history, UTC timestamp formatting, chronological deltas, and direction filters.
- `capture`: streaming JSON Lines writer, independent bounded subscriptions, integrity summaries, and controlled shutdown.
- `config`: versioned serial-setting persistence with explicit save and atomic file replacement.
- `app`: device controls, dock layout, bounded/virtualized terminal views, and send controls.

A slow subscriber loses monitoring events instead of blocking a serial worker. Sequence numbers expose gaps. Bridges forward through a dedicated transport RX route into the destination writer, independently of the lossy monitor bus. Independent read handles keep both RX directions active while writes are serialized. Closing a terminal stops its worker and drops its device handle; closing the application drops all terminals.

## Issue progress

The foundation (#1–#5), repeated sending (#6), and preset profiles (#7) are merged. Owned PTY pairs (#8) and full-duplex bridging (#9) are merged; bridge inspection and capture export (#10) are implemented in the current pass. Serial-device (#3) and interactive docking (#4) checks remain open for manual validation with physical hardware.

Next in issue order: full workspace persistence (#11), expanded integration tests (#12), and packaging/usability (#13). Port settings and preset profiles are persisted today; dock layout, virtual-pair definitions, bridges, and display preferences are not yet saved.

## Manual smoke checklist

- Start under X11/Wayland; confirm the window and device refresh work.
- Open two devices with independent settings and rearrange their tabs.
- Send `test\r\n`, then `00 FF` in hex mode; verify bytes with a peer.
- Reject an unknown escape and malformed hex without transmitting.
- Run a finite repeat, verify its total bytes/count, then run and stop a continuous repeat on each of two ports.
- Disconnect/close a repeating port and confirm the peer receives no further payloads.
- Verify RX/TX labels, hex/text modes, timestamps, pause, clear, and auto-scroll.
- Unplug a device, confirm an endpoint error, close its tab, then reconnect.
- Create/edit/reorder/delete presets, export/import a profile, and verify one-click and keyboard sends to the selected terminal.
- Create several virtual pairs, copy/open their paths in another program, exchange bytes both ways, then remove them and verify cleanup.
- Bridge two serial devices or a serial device and a virtual pair; exchange binary data both ways, pause the bridge monitor, and verify forwarding continues.
- Record a bridge capture while pausing/filtering the inspector; verify raw bytes and directions in JSONL and a complete footer after stopping.
- Disconnect either bridged endpoint and verify the bridge faults without restarting automatically.
- Save settings and restart; confirm no device opens or sends automatically.

CI builds every target and runs unit and Linux PTY integration tests without physical hardware. The initial Linux CI build and unit/PTY tests passed. Local builds are unavailable in the implementation environment, which lacks Rust and has an unavailable network proxy. CI also starts the app under Xvfb with two real PTY endpoints; physical-device and interactive docking smoke validation remain manual.
