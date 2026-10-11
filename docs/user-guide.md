# Signal Forge user guide

Open **Help** or press **F1** inside Signal Forge to read this guide offline. Search by a task, device error, control label, or topic ID. Related-topic links connect workflows and diagnostics. Controls with contextual help show a short tooltip; keep the pointer over one for ten seconds for an explanation and **Learn more**. Turn off extended hover help in the guide if you prefer. The preference lasts for this app session.

<!-- topic:getting.started -->
## Getting started
> Connect a device, choose its serial settings, and explicitly send your first message.

Signal Forge is a Linux serial workbench for observing and sending bytes. Start with a physical serial port, an SSH serial device, or a virtual pair for experiments without hardware. Opening Help never connects a port or sends traffic.

1. Launch `signal-forge` from your desktop or terminal. Use `signal-forge --port /dev/ttyUSB0` to explicitly open a device at launch; multiple `--port` arguments open multiple terminals.
2. Choose **+ New Port** or a device under **Local ports**. Enter its path and the baud, data bits, parity, stop bits, and flow control specified by the device manual. A common starting configuration is 19200, 8 data bits, no parity, 1 stop bit, no flow control, but it must match the device.
3. Select **Open port**. Check the terminal's connection status. Start in **Line** for command protocols or **Hex** for binary traffic.
4. Type a command, choose its **Ending**, then click **Send** or press Enter in the payload field. For a device expecting `STATUS\r\n`, enter `STATUS` and choose CRLF. Do not also type escaped CRLF unless two endings are intended.
5. Disconnect when finished. Reopening saved workspaces restores settings disconnected; reconnect explicitly.

Use **Virtual Pair** to create two linked endpoints. Open both in Signal Forge, or open one in an external application, then send from one and observe RX at the other.

Related: [Serial settings](#serial-settings), [Receive modes](#receive-modes), [Text and hex sending](#text-and-hex-sending), [Installation and updates](#installation-and-updates).

<!-- topic:serial.settings -->
## Serial settings
> Baud and framing must match your device; changing them is not a protocol negotiation.

Baud is the link rate. Data bits, parity, and stop bits define each serial frame. Flow control can pause transmission: Hardware uses RTS/CTS, Software uses XON/XOFF, and None uses neither. Match all settings to the connected equipment before troubleshooting payloads.

Use **+ New Port** to configure a local device or enable **Remote serial over SSH**. The device path is the serial port, not a firmware image or log file. Prefer `/dev/serial/by-id/...` for USB adapters whose `/dev/ttyUSB0` number changes after reconnecting.

The terminal **Settings** control exposes its serial configuration. Saved settings survive workspace saves; they do not cause automatic transmission. An 8N1 frame uses 10 bits per byte: one start bit, eight data bits, one stop bit. At 19200 baud, 100 bytes take about 52.1 ms on a physical wire without extra gaps. PTY timing is a separate link option.

Do not run Signal Forge as root to work around a device access error. Check the device permissions and your access policy instead; an administrator may need to grant ordinary-user access.

Related: [Port access problems](#port-access-problems), [No traffic or garbled traffic](#no-traffic-or-garbled-traffic), [Virtual timing](#virtual-timing).

<!-- topic:terminal.modes -->
## Receive modes
> Line groups by terminators; Raw Chunks shows reads; Hex exposes exact binary bytes.

**Line** assembles incoming data into lines using the selected receive delimiter. **Raw Chunks** shows the application's read chunks. **Hex** shows byte values and a text representation. These are views of received bytes, not changes to the connected device's protocol.

Use Line for newline-delimited commands and logs. Choose **Display…** to set Auto, LF, CRLF, or CR receive delimiters. Auto recognizes CR, LF, and CRLF; a pending partial line can appear before its terminator arrives. TX is displayed separately.

Use Raw Chunks (shown as Chunks in the narrow selector) to see how reads were delivered, but do not treat a read chunk as a protocol packet: the OS can split one message or combine several. Choose Hex for non-UTF-8 data, missing terminators, bootloader commands, and exact byte comparisons. Changing views does not send, reframe, or repair traffic.

Related: [Control characters and visibility](#control-characters-and-visibility), [Selection and copying](#selection-and-copying), [Timing and statistics](#timing-and-statistics).

<!-- topic:terminal.control_characters -->
## Control characters and visibility
> Display hidden CR/LF and control bytes without changing the payload.

**Display… → Show Control Characters** makes otherwise invisible control bytes visible. A displayed `\r`, `\n`, `\0`, or `\xNN` represents bytes, while timestamps, RX/TX labels, and UI row separators are annotations. Use Hex when you need to remove ambiguity between escaped display text and literal backslash characters.

Use **RX + TX**, **RX only**, or **TX only** to choose visible directions; enable **Show RX/TX labels** to distinguish received from sent traffic. Timestamps describe the application's traffic events, not hardware timestamps. **Auto-scroll** follows new rows. **Pause display** pauses monitoring display; it does not pause the serial peer or act as hardware flow control. Data received during that pause is not added to the display history.

**Clear** clears terminal display history. It does not reset the device. Hidden line endings are not included in a selection unless made visible, so enable control characters before selecting an entire frame for checksum analysis.

Related: [Receive modes](#receive-modes), [Selection Inspector](#selection-inspector), [Recording RX](#recording-rx).

<!-- topic:selection.copy -->
## Selection and copying
> Select traffic glyphs to copy text; inspector copy actions export payload representations.

Drag over terminal traffic to select a range. Click traffic then Ctrl+A to select retained displayed traffic; dragging beyond the viewport scrolls through retained history. Starting a selection disables auto-scroll. **Ctrl+C** copies the terminal's selected display text using its normal copy behavior. **Display… → Copy All** copies the displayed traffic. Neither action transmits data.

The Selection Inspector follows the selected terminal. Its **Copy ASCII**, **Copy Hex**, and **Copy XOR** buttons export payload-only representations. Selecting part of a displayed escape, hex byte, or Unicode glyph selects its underlying complete byte or byte sequence. UI timestamps, labels, spacing, and timing annotations have no payload bytes.

Filtering rows or hiding line endings can leave gaps. Selection XOR includes only selected bytes; it does not fill in hidden gaps. For an exact complete frame, show both directions as needed, enable control characters, remove filters, and select the intended payload. Display history is bounded, so use recording or capture when you need an enduring record.

Related: [Selection Inspector](#selection-inspector), [Search and highlighting](#search-and-highlighting), [Recording RX](#recording-rx).

<!-- topic:selection.inspector -->
## Selection Inspector
> Inspect the selected bytes, XOR, source timing, and possible NMEA sentences.

Highlight traffic to open the Selection Inspector. It shows byte count, UTF-8 character count when valid, escaped ASCII, raw hex, Selection XOR, event timing, and calculated wire time. It does not send, edit, or validate arbitrary device protocols.

Move, resize, collapse, or close the inspector. It does not take mouse input while you drag a terminal selection. A new selection reopens a dismissed inspector. Previews are limited to 4096 bytes; copy buttons include the complete selection.

**Selection XOR** XORs every selected byte, including any selected delimiters and CR/LF. Selecting `#0,MED` gives `0x73`; including its CRLF gives `0x74`. NMEA validation separately recognizes complete `$...*XX` or `!...*XX` sentences in verified contiguous segments. It excludes the start delimiter, `*XX`, and CR/LF, and compares the calculated checksum to the received hex value.

RX/TX changes, filtered gaps, pauses, and missing source metadata prevent a claim of a single contiguous sentence. Missing timing metadata is reported as unavailable. A zero observed span often means all selected bytes came from one OS read, not simultaneous physical arrival.

Related: [Checksum problems](#checksum-problems), [Timing and statistics](#timing-and-statistics), [Selection and copying](#selection-and-copying).

<!-- topic:tx.payload -->
## Text and hex sending
> Parse text or hex into bytes, append the selected ending, and send explicitly.

**Text** sends UTF-8 text. Enable **Escapes** to interpret supported byte escapes such as `\r`, `\n`, `\t`, `\0`, `\\`, and `\xNN`. **Hex bytes** parses pairs of hexadecimal digits into binary bytes. Invalid input is rejected rather than partly sent.

For a binary command containing `02 00 FF`, choose Hex bytes and enter those three byte pairs. With Text and Escapes enabled, `PING\r\n` sends four letters followed by CR and LF. With Escapes disabled, the backslashes and letters are literal text. Choose **Ending: None** when the payload already contains its terminator.

Use **Send** or Enter in the payload editor. Each terminal has its own command history: Up and Down recall previous sends without sending them. Drafts and recalled settings need an explicit Send. A successful TX event reports application output; it is not proof that a remote instrument accepted or acted on the command.

Related: [Line endings](#line-endings), [TX checksums](#tx-checksums), [Presets](#presets), [No traffic or garbled traffic](#no-traffic-or-garbled-traffic).

<!-- topic:tx.ending -->
## Line endings
> Append None, CR, LF, or CRLF after the payload and any generated checksum.

The **Ending** control appends bytes after parsing your payload and adding any checksum. CR is byte `0D`, LF is `0A`, and CRLF is `0D 0A`. Choose the ending required by your device; there is no universal serial line ending.

For `AT` followed by CR, type `AT` and choose CR. If you type `AT\r` with Escapes enabled and also choose CR, you send two CR bytes. Configured endings are excluded from automatic XOR; explicit escaped endings inside the payload are included.

The receive delimiter under **Display…** controls line presentation independently. It does not alter outgoing endings. Use Show Control Characters or Hex to compare transmitted and received terminators.

Related: [Text and hex sending](#text-and-hex-sending), [TX checksums](#tx-checksums), [No traffic or garbled traffic](#no-traffic-or-garbled-traffic).

<!-- topic:tx.checksum -->
## TX checksums
> Append an XOR checksum before the configured line ending.

Open **Checksum: Off** (or **XOR: Off** in a narrow pane) and select **XOR**. Choose Hex for two uppercase ASCII digits, *Hex for `*` plus those digits, or Raw for one binary checksum byte. The control previews the value before transmission.

The order is parsed payload, generated checksum, then configured ending. `#0,MED` with *Hex and CRLF sends `#0,MED*73\r\n`. With Skip first byte enabled it sends `#0,MED*50\r\n`. An empty checksum range yields zero. Existing checksum text in the input remains ordinary payload; it is not stripped or replaced.

Normal Send, Enter sends, repeats, and presets use this transform. Presets have their own checksum settings. Active repeats retain the bytes captured at Start. File sending preserves its separate byte-transform rules and does not inject this checksum. Select Off when the device protocol does not require it.

Related: [Skip first byte](#skip-first-byte), [Checksum problems](#checksum-problems), [Line endings](#line-endings).

<!-- topic:tx.checksum.skip_first -->
## Skip first byte
> Exclude the first parsed byte from XOR while still transmitting it.

**Skip first byte** excludes exactly the first transmitted payload byte from the checksum calculation while still sending that byte. Use it when that byte is a framing delimiter such as `#` or `$`. It skips one byte, not one Unicode character, and does not check which delimiter you used.

For `$ABC`, XOR over `ABC` is `0x40`. With Skip first byte, *Hex, and CRLF, the wire message is `$ABC*40\r\n`. CRLF added by Ending is excluded. CRLF explicitly embedded in the input participates in XOR.

For NMEA-style output, supply only the start delimiter and payload; do not enter an existing `*XX`. Disabling this option includes the delimiter in XOR. Skipping the only byte produces a zero checksum without removing that byte from the sent payload.

Related: [TX checksums](#tx-checksums), [Checksum problems](#checksum-problems), [Selection Inspector](#selection-inspector).

<!-- topic:tx.presets -->
## Presets
> Save reusable messages with explicit encoding, endings, checksum, and target.

Use **Presets…** or **Manage…** to create profiles of named sends. Save payload, Text or Hex bytes encoding, escape handling, ending, optional checksum, shortcut, and target. A preset can address the selected terminal or a fixed endpoint; verify the target before sending to connected equipment.

Activate a preset or its shortcut to send explicitly. A repeated preset starts its configured repeat behavior; saved presets never transmit just because a workspace was loaded. Remote endpoint IDs survive host/device edits so fixed-target presets keep their intended terminal. Changing a terminal's send options does not rewrite an existing preset.

Use profile import/export for reusable command collections. Exporting does not send. Presets are stored under the configuration directory in `presets.json`; keep a backup for important bench procedures. Avoid placing secrets in portable profiles.

Related: [Repeats](#repeats), [Text and hex sending](#text-and-hex-sending), [SSH serial](#ssh-serial), [Workspaces](#workspaces).

<!-- topic:tx.repeat -->
## Repeats
> Schedule identical messages at an interval, with a count or until stopped.

Choose the terminal **Repeat** tool, set interval and count or Continuous, and select Start. The message bytes are captured when the repeat starts, including parsed escapes, checksum, and ending. Editing the draft while it runs does not change the active repeat.

Use repeats for polling or reproducible stimulus. Finite repeats stop at the requested count; continuous repeats need Stop. Disconnecting an endpoint stops its repeat. Workspace/session restore does not restart repeats. A repeat interval is application scheduling, not a precise hardware clock; serial transmission time and backpressure can limit throughput.

Before starting, verify the target, message, and rate. At 19200 8N1, a 100-byte message needs approximately 52.1 ms of physical wire time before any device processing. Sending it every millisecond cannot make the physical link transmit that fast.

Related: [Timing and statistics](#timing-and-statistics), [TX checksums](#tx-checksums), [Serial settings](#serial-settings).

<!-- topic:tx.file -->
## File sending
> Choose a file and explicitly stream its content with chunk and delay controls.

Use **Files / RX → Browse…** to select a source file, choose its mode, chunk size, and inter-chunk delay, then start the transfer. File selection alone never sends. Raw preserves bytes, ASCII validates UTF-8 text and sends its bytes unchanged, and Hex parses hexadecimal byte text. Check the selected mode against how the file was produced.

Use Raw for firmware or other binary images when the device protocol explicitly accepts a raw stream. This is not a protocol-specific firmware flasher: a bootloader may require framing, acknowledgements, packet checksums, or a dedicated tool. Hex mode parses byte values rather than sending the ASCII characters of a hex dump. ASCII does not unescape an ASCII RX recording: visible `\xNN` text in that file remains literal characters. Use Raw recordings when binary round-trip fidelity matters.

Progress and errors appear in the terminal. Cancel file send stops the job; bytes already written cannot be undone. Chunk size and delay affect scheduling, not hardware baud. Terminal draft XOR settings are not applied to file transfers. Avoid another sender competing for the same endpoint during a transfer.

Related: [Recording RX](#recording-rx), [Timing and statistics](#timing-and-statistics), [Port access problems](#port-access-problems).

<!-- topic:recording.rx -->
## Recording RX
> Save incoming bytes independently of the displayed representation.

Open **Files / RX**, use **Save as…** for an output path, select Raw, ASCII, or Hex, and click **Record RX**. Recording starts only after this explicit action. **Stop recording** finalizes the output. Existing files are protected against overwriting; choose a new filename.

Raw preserves received binary bytes. ASCII writes an escaped, readable representation; Hex writes hexadecimal byte text. These exports do not all have the same on-disk representation. Raw RX recording contains received payload, not outgoing bytes or replay timing metadata. Capture JSONL is the option for structured traffic events.

Recording runs independently of the terminal's view and display filters. Check status and errors rather than assuming every displayed row reached disk. Disconnect finalizes recording. In a session, paths default to that session's folder and successfully started recordings are indexed as artifacts. Outside a session, your chosen output path determines the location.

Related: [Capture and replay files](#capture-and-replay-files), [Sessions](#sessions), [Recording and capture problems](#recording-and-capture-problems).

<!-- topic:analysis.search -->
## Search and highlighting
> Find, filter, and emphasize displayed traffic without modifying device bytes.

Open **Analysis** to configure search, filters, and highlight rules. Pattern modes include text, hex bytes, and regular expressions where offered. Search locates matches; filtering hides nonmatching rows; highlighting applies visual emphasis to matching traffic.

Use Hex patterns for binary signatures, such as `02 FF`, and text patterns for readable replies. Check the displayed pattern error if a regular expression or hex pattern is invalid. Search the retained terminal history; it is not an unlimited file-search service. RX/TX visibility and filters affect what is shown, not what the device sends.

A filtered selection may contain gaps. The inspector marks those segments and does not invent missing bytes. Clear the filter and enable control characters before validating a whole protocol frame. Capture triggers have independent patterns and buffering; a display search is not a recording trigger.

Related: [Selection and copying](#selection-and-copying), [Triggered capture](#triggered-capture), [Timing and statistics](#timing-and-statistics).

<!-- topic:analysis.timing -->
## Timing and statistics
> Compare observed event timing with calculated wire time; neither is per-byte hardware timing.

Use **Statistics…** (or **Stats** in narrow panes) and the **Analysis…** timing options to inspect traffic counts, rates, gaps, and request/response measurements. Application timestamps reflect when events were observed. Calculated wire time uses byte count, baud, data bits, parity, and stop bits; it is an estimate of serial occupancy without idle gaps.

At 19200 8N1, each byte occupies ten bits. Ten bytes need approximately 5.21 ms on a physical wire. A zero observed span can mean all ten arrived in one read. It does not mean the physical transfer took zero time. OS scheduling, buffering, SSH, and read chunk boundaries can broaden or hide real gaps.

Request/response timing associates monitored TX/RX according to the configured analysis model. It is not proof of a protocol acknowledgement or round-trip network latency. Selection Inspector uses source events and captured framing, and reports unavailable metadata rather than fabricating exact timing. Do not infer packet boundaries from raw read chunks.

Related: [Virtual timing](#virtual-timing), [Timing problems](#timing-problems), [Selection Inspector](#selection-inspector).

<!-- topic:capture.trigger -->
## Triggered capture
> Retain traffic around a matching event using a bounded rolling buffer.

Choose **Triggered capture**, set an output path and pre/post-trigger limits, choose Manual, RX pattern, or RX after idle, then arm the capture. For RX pattern choose text, hex, or byte regex; for Manual use Fire after arming. The rolling buffer holds bounded recent events before a match; after the trigger, the configured post-trigger collection completes the capture. Arming does not send a command.

For a fault log, trigger on text `ERROR` and keep preceding context plus following replies. For a binary marker use a Hex pattern. Validate the pattern and choose limits appropriate to the expected event rate. Bounded buffers can discard older context; large payloads and dropped monitor events affect completeness.

Check capture status and finalized output. Existing files are not silently replaced. Session paths default inside the active session. Disconnects or explicit stopping can finalize partial captures; inspect file metadata and event/drop information before treating the artifact as a complete test record.

Related: [Capture and replay files](#capture-and-replay-files), [Search and highlighting](#search-and-highlighting), [Recording and capture problems](#recording-and-capture-problems).

<!-- topic:capture.replay -->
## Capture and replay files
> Replay structured JSONL traffic offline; raw RX files have no event timing.

Bridge and triggered captures use versioned JSONL with headers, traffic events, and finalization metadata. It records endpoint identity, directions, payload, and event timestamps; inspect the footer/status for dropped events or an incomplete run. RX recording is a separate raw/ASCII/hex payload export.

Use **Replay…** to open Raw, Hex, or Signal Forge JSONL. Choose the stream (RX, TX, or both where supported), then **Open paused replay**. Playback presents saved events without reopening their original devices. Use Play/Pause, Reset, or Step; JSONL can follow original event timing or maximum speed. Raw/Hex use the chosen chunk size and pacing delay because they have no original event timestamps. A read-only replay can feed a connected device through an explicitly started one-way bridge; this sends real bytes only after you start playback/forwarding. Loading a workspace does not automatically start replay or send captured bytes to hardware.

Keep the original capture intact and use new output names for new runs. Raw binary files cannot supply missing directions or timestamps; converting a hex dump to bytes does not create genuine timing metadata. Rendered ASCII recordings can be opened as Raw text, but their escapes are not decoded back into original binary bytes. Use a session folder to keep captures, notes, and workspace context together.

Related: [Triggered capture](#triggered-capture), [Recording RX](#recording-rx), [Sessions](#sessions), [Recording and capture problems](#recording-and-capture-problems).

<!-- topic:virtual_pair.create -->
## Virtual pairs
> Create two local linked ports for experiments and external serial applications.

**Virtual Pair** creates two linked Linux pseudo-terminal endpoints, or PTYs. A PTY is a software serial-like byte stream, not a USB adapter. Bytes written at one end arrive at the other. No root privileges are required to create a pair.

Enter a unique pair name and optional **Link directory**. Without friendly links, use the displayed `/dev/pts/N` device paths. With a link directory, Signal Forge creates named symlinks pointing to those actual PTYs; it does not copy a device into the directory. Use the path displayed by Signal Forge in an external program, such as `serial.Serial('/dev/pts/7', 19200)`.

Multiple pairs are independent. New dialogs propose an unused name; existing names must not be reused. Open each intended side explicitly in Signal Forge or an external application. The creating Signal Forge process owns the pair: shutting it down removes its owned links and ends that pair. Paths are ephemeral, and restarting can allocate different numbers.

Related: [Virtual timing](#virtual-timing), [Virtual ports and containers](#virtual-ports-and-containers), [Bridges](#bridges).

<!-- topic:virtual_pair.timing -->
## Virtual timing
> Unlimited PTYs transfer immediately; baud emulation adds an approximate shared link schedule.

Normal PTYs have no physical UART clock and are effectively instantaneous relative to a hardware serial link. Configuring a baud rate in an external application does not itself impose serial wire timing on a PTY. Choose the pair's baud-emulated timing option when you need a rate-limited approximation.

The virtual pair's shared baud and framing model controls forwarding time for both directions. It is separate from terminal connection settings. At 19200 8N1, 100 bytes imply about 52.1 ms of calculated wire occupancy. Changing a PTY application's baud does not renegotiate Signal Forge's shared model.

Emulation is useful for exercising timeout and load behavior, but OS scheduling, buffers, and read sizes determine observed arrival chunks. It cannot reproduce electrical parity errors, hardware RTS/CTS, exact inter-byte timing, or a real device's processing delays. Compare the selected model, observed timestamps, and calculated wire time; do not treat them as interchangeable measurements.

Related: [Timing problems](#timing-problems), [Virtual pairs](#virtual-pairs), [Timing and statistics](#timing-and-statistics).

<!-- topic:bridge.create -->
## Bridges
> Forward bytes in both directions between two explicitly connected endpoints.

Open two endpoints, choose **Bridge**, select each side, and start the bridge. A full-duplex bridge forwards traffic both ways while Signal Forge can monitor it. Use a physical serial port and a virtual endpoint to interpose on an application, or a local endpoint and an SSH endpoint to reach a remote instrument.

Verify the endpoints before starting: a bridge can expose commands to real equipment. Avoid another bridge creating a loop or multiple writers competing for one endpoint. Forwarded data is byte traffic, not protocol translation. Capture bridge traffic to JSONL when you need directions and event metadata.

Stop the bridge explicitly when finished. Endpoint disconnects stop attached forwarding; workspace/session restoration does not recreate active bridges. Bridge monitor display pause is not a pause of the underlying byte forwarding. SSH latency and buffering affect timing even when the physical device settings match.

Related: [Virtual pairs](#virtual-pairs), [SSH serial](#ssh-serial), [Capture and replay files](#capture-and-replay-files).

<!-- topic:ssh.serial -->
## SSH serial
> Open a remote Linux serial device over SSH with keys and strict host verification.

Choose **+ New Port → Remote serial over SSH**. Enter the host or SSH config alias, optional username/SSH port, and the remote device path. Save the host for reuse; expand **Remote hosts** to discover devices or use **Refresh ports**. Discovery is asynchronous and does not interrupt live terminals.

Signal Forge uses the installed SSH client, keys or agent, SSH configuration, and strict known-host verification. The remote host needs Python 3 and permission to open its serial device. It does not need Signal Forge or pyserial installed. Establish trusted SSH access normally before opening a device; Signal Forge does not prompt for passwords or silently accept unknown host keys.

Use a stable `/dev/serial/by-id/...` remote path where possible. **Edit Host** updates shared connection settings and disconnects linked endpoints; **Edit remote endpoint** changes its device, framing, or friendly name. History and drafts remain, endpoint IDs stay stable, and reconnecting is explicit. A remote endpoint can participate in a bridge like a local one.

Related: [SSH connection problems](#ssh-connection-problems), [Bridges](#bridges), [Workspaces](#workspaces).

<!-- topic:workspace.save -->
## Workspaces
> Save layouts and settings; restore disconnected without replaying commands.

**Save Workspace** or **Ctrl+S** saves the default workspace. **Workspace…** offers Save to file and Load workspace. Normal application shutdown also saves the default workspace. Saved state includes splits, tabs, floating windows, per-device settings, display/send options, checksum configuration, saved SSH hosts, and selected preset profile.

Restoring a workspace disconnects current resources and restores saved terminals disconnected. It does not send drafts, restart repeats, reconnect devices, resume recording, or recreate session-owned PTYs and bridges. Ephemeral virtual paths may no longer exist; create a new pair and use its current paths.

The default file is `$XDG_CONFIG_HOME/signal-forge/workspace.json` or `~/.config/signal-forge/workspace.json`. Corrupt or unsupported files are preserved; automatic saving stays disabled until repair or the explicit backup/reset recovery action. **2 Tiles** and **4 Tiles** arrange terminals; they do not change serial settings or transmit.

Related: [Sessions](#sessions), [Installation and updates](#installation-and-updates), [Virtual ports and containers](#virtual-ports-and-containers).

<!-- topic:session.manage -->
## Sessions
> Group notes, workspace context, recordings, captures, and external artifacts in a folder.

Open **Session…** or **Ctrl+Shift+E**, select New session, and choose a new or empty folder. A session contains `session.json`, `workspace.json`, `notes.txt`, and indexed artifacts. Creating one snapshots the workspace and finalizes pre-existing outputs while keeping live connections open.

Edit notes in the session panel; they autosave after one second and also save with Save session, Ctrl+S, and normal close. Recordings and captures default to session paths and are indexed when successfully started. **Add artifact…** associates an existing file without copying or deleting it. Use Copy path or Open containing folder to find an artifact.

**Close session** finalizes outputs and saves context, while leaving endpoints connected. **Open session** validates files before replacing the workspace, then restores terminals disconnected. Explicitly reconnect. Moving the whole folder preserves relative artifact paths, but external files, devices, and SSH hosts can be machine-specific.

External edits are preserved rather than overwritten. On a conflict, copy unsaved notes before choosing Reload session files from disk. Missing artifacts remain indexed and do not prevent reopening; reassociate moved files explicitly.

Related: [Workspaces](#workspaces), [Recording RX](#recording-rx), [Recording and capture problems](#recording-and-capture-problems).

<!-- topic:installation.update -->
## Installation and updates
> Install in a writable user prefix and review offered updates before applying them.

Download the Linux release archive, verify its supplied SHA-256 checksum, extract it, and run its `install.sh` with a user-owned installation prefix. The package provides a desktop entry and icon. From the extracted package directory, run `bash install.sh "$HOME/.local"`, then `"$HOME/.local/bin/signal-forge"`. Launch through the desktop or its installed `bin/signal-forge`. Source builds require the Rust toolchain and the repository's Linux native build dependencies.

The application checks for releases and presents an update offer. Download/install needs explicit approval in that prompt; the application then restarts with saved workspace settings and disconnected terminals. A writable user installation avoids needing runtime sudo. Development/source binaries may not support the same packaged update flow. Use `--no-update-check` to skip the launch check or `--version` to identify the running build.

Configuration lives in `$XDG_CONFIG_HOME/signal-forge` or `~/.config/signal-forge`. Workspace and preset files are separate. Logs, recordings, captures, and session artifacts are at the output locations you selected; they are not automatically all in the configuration directory. Help is bundled with the executable and requires no network connection.

Related: [Getting started](#getting-started), [Workspaces](#workspaces), [Recording and capture problems](#recording-and-capture-problems).

<!-- topic:troubleshoot.port -->
## Port access problems
> Check the exact device path, ordinary-user access, and other programs holding it.

For permission denied, inspect `ls -l /dev/ttyUSB0` and `id` using the actual path. Confirm the owning group and your access to it. Distribution access rules differ; request the appropriate administrator-managed access instead of launching Signal Forge with sudo. A running container has its own device visibility and permissions.

For device already in use, close competing serial terminals, bootloaders, and background services. Where available, `fuser /dev/ttyUSB0` or `lsof /dev/ttyUSB0` can identify owners accessible to your user. Do not force-kill an unknown device service. Check the terminal connection state and error to distinguish opening failure from a protocol with no reply.

After a USB reconnect, use Refresh and verify the current path. `/dev/ttyUSB0` can become `/dev/ttyUSB1`. Compare `/dev/serial/by-id/` links and reconnect using the stable adapter identity. Confirm the symlink still resolves; an unplugged device has no valid target. For virtual ports, the creating process must still be running.

Related: [Serial settings](#serial-settings), [Virtual ports and containers](#virtual-ports-and-containers), [No traffic or garbled traffic](#no-traffic-or-garbled-traffic).

<!-- topic:troubleshoot.traffic -->
## No traffic or garbled traffic
> Separate connection, visibility, framing, and protocol problems before changing the payload.

1. Check Connected versus Disconnected and the error text. A displayed TX byte count means bytes were written, not that the device replied.
2. Choose RX + TX, remove filters, unpause display, and inspect Hex. Binary data can look garbled in Text without being corrupt.
3. Verify baud, data bits, parity, stop bits, and flow control against the device manual. Wrong baud commonly produces nonsense bytes; RTS/CTS or XON/XOFF can stall output if the peer does not cooperate.
4. Show Control Characters and inspect the actual TX ending. Try the documented CR, LF, or CRLF, avoiding duplicated endings in the payload.
5. Confirm wiring, adapter voltage levels, device state, and whether the protocol needs a framed request or dedicated bootloader application.

Use a virtual pair to verify your application send/receive workflow independently of hardware. Use a raw recording or JSONL capture to preserve the actual bytes before changing view settings. Do not assume an OS read chunk is a complete reply.

Related: [Port access problems](#port-access-problems), [Line endings](#line-endings), [Receive modes](#receive-modes), [Checksum problems](#checksum-problems).

<!-- topic:troubleshoot.virtual -->
## Virtual ports and containers
> A PTY must exist in the reader's device namespace; a friendly link does not expose it to Docker.

If pyserial reports No such file or directory for `/dev/pts/7`, run `ls -l /dev/pts/7` in the same environment as that application. Confirm Signal Forge is still running and copy the current displayed path. A PTY can be on the same computer but invisible inside a container's private `/dev/pts` namespace.

Try the external program on the host first to distinguish a Signal Forge pair problem from container isolation. A friendly link directory is just symlinks: inspect `readlink` and `readlink -f` on the link inside the external program's environment. Mounting only that directory into Docker does not make its host `/dev/pts/N` target appear in the container. Container configuration must explicitly expose the appropriate host device/namespace and ordinary-user permissions. There is no universal safe Docker command for every namespace setup; use your application's container policy and do not add blanket privileged mode as a workaround.

Some serial dropdowns enumerate physical USB devices but omit PTYs. Try opening the copied path directly, for example `python3 -c "import serial; s=serial.Serial('/dev/pts/7',19200); print(s.is_open); s.close()"`, with pyserial installed in that Python environment. A successful open does not send a test command.

Use the pair sidebar **Check paths/access** action to inspect links, owners, permissions, and access results. If an external client exited but left an exclusive-open flag, use **Release stale exclusive flag** only after confirming that client is no longer using the side. For duplicate names, choose a unique pair name. Stop pairs normally to clean up owned links. After a crash, inspect any stale symlink before removing it and choose a new name; do not delete unrelated device paths. Restored workspace PTY paths are not recreated automatically.

Related: [Virtual pairs](#virtual-pairs), [Port access problems](#port-access-problems), [Bridges](#bridges).

<!-- topic:troubleshoot.timing -->
## Timing problems
> Check whether you are measuring read events, emulated forwarding, or physical wire time.

A normal PTY can deliver a large payload almost instantly even if both applications set 19200 baud. Check whether the pair is Unlimited or baud-emulated. In emulated mode, verify the pair's shared baud/framing rather than only the terminal settings.

Compare three quantities separately: calculated wire time from bytes/framing, event timestamps from application reads/writes, and any protocol request/response measurement. One OS read containing a whole packet can have zero observed span. SSH buffering, CPU scheduling, timer granularity, and device processing can make observed gaps larger than calculated transmission time.

Use Hex and Selection Inspector to confirm byte counts before calculating timing. For 8N1, time in seconds is bytes multiplied by 10 divided by baud. Parity and a second stop bit increase frame size. Signal Forge does not provide hardware per-byte timestamps or electrical timing verification; use appropriate bench equipment when those measurements are required.

Related: [Virtual timing](#virtual-timing), [Timing and statistics](#timing-and-statistics), [Selection Inspector](#selection-inspector).

<!-- topic:troubleshoot.checksum -->
## Checksum problems
> Verify the exact checksum range, output encoding, and ending order.

Selection XOR and NMEA validation answer different questions. Selection XOR includes every byte you selected. NMEA XOR excludes `$` or `!`, `*XX`, and CR/LF and only validates complete contiguous sentences. `#0,MED` is not automatically a standard NMEA sentence.

For TX, inspect the checksum preview, Skip first byte, output format, and Ending. Including `#` in `#0,MED` gives `73`; excluding it gives `50`. Hex output sends two ASCII digits; Raw sends one byte; *Hex also prefixes `*`. Endings added by the Ending control come after XOR and are excluded. Escaped CR/LF inside the payload are included.

Use Hex or Show Control Characters to select exactly the transmitted range. Hidden endings and filtered gaps can change Selection XOR. Existing `*XX` in your draft is included as ordinary input; remove it when generating a fresh checksum. Presets have independent checksum options, and an active repeat keeps its original fully encoded bytes. File sends do not inherit draft XOR settings.

Related: [TX checksums](#tx-checksums), [Skip first byte](#skip-first-byte), [Selection Inspector](#selection-inspector).

<!-- topic:troubleshoot.ssh -->
## SSH connection problems
> Diagnose SSH access separately from remote device access and serial protocol settings.

Check the displayed discovery or connection error. Test ordinary SSH to the same host/alias as the same local user. Unknown or changed host-key errors require verifying the server identity through your trusted process; Signal Forge uses strict verification and does not bypass it. Authentication errors usually involve the selected user, SSH config, key file, or agent availability.

On the remote host, confirm Python 3 exists and that the serial path resolves. Check its permissions as the SSH user, not your local desktop user. A saved identity-file path points to a local key; it does not upload a private key. Avoid passwords or private-key contents in workspace files.

Refresh the saved host to distinguish an empty device list from a connection failure. A missing remote device can be unplugged or renumbered; prefer stable by-id paths. A disconnected SSH endpoint stops active work and requires explicit reconnect. Host/endpoint edits also intentionally disconnect affected terminals while preserving drafts/history. A connected endpoint with no reply still needs framing, flow, and protocol diagnostics.

Related: [SSH serial](#ssh-serial), [Port access problems](#port-access-problems), [No traffic or garbled traffic](#no-traffic-or-garbled-traffic).

<!-- topic:troubleshoot.recording -->
## Recording and capture problems
> Locate the chosen output and check writer status, format, and finalization before replaying.

Confirm the output path in Files / RX, Triggered capture, the bridge capture controls, or the session artifact list. Outside a session there is no universal logs folder: the selected path determines location. Inside a session, use Copy path or Open containing folder. A missing artifact may have been moved externally.

If recording cannot start, check the parent directory and ordinary-user write access, choose a new filename, and read the error. Writers refuse to silently overwrite existing files. Check free disk space and worker status for failures during a run. Stop recording/capture or close the session normally to finalize outputs; an interrupted JSONL file may lack its completion footer.

Distinguish Raw binary, ASCII escaped text, Hex byte text, and structured JSONL capture. Match the file-send parser to the representation. For replay select the matching Raw, Hex, or JSONL format. Raw/Hex require synthetic chunk pacing; they cannot reproduce original timing. Recorded timestamps are event-level, not precise byte arrival times. Dropped monitor events, rolling-buffer limits, and missing finalization can limit completeness even if the file exists.

Related: [Recording RX](#recording-rx), [Capture and replay files](#capture-and-replay-files), [Sessions](#sessions), [Timing and statistics](#timing-and-statistics).

<!-- topic:network.setup -->
## Network endpoints
> Open TCP or UDP clients and listeners with Network; saved endpoints restore disconnected.

Choose **Network…**, select TCP client, TCP listener, UDP client, or UDP listener, and enter the remote host or local bind address. Clients accept hostnames and IPv4/IPv6 addresses. Bind addresses must be numeric IP:port; write IPv6 binds as `[::1]:5000`. The terminal shows the actual local address and current peer. Wildcard addresses in packet metadata identify the bound socket; they do not infer which interface carried a packet. Port 0 chooses a free local port each time you reconnect.

TCP clients connect asynchronously within the configured timeout. Disconnect cancels application work immediately; any OS lookup or connection attempt still in progress finishes in the background and its result is discarded. TCP listeners start in Listening, accept one active client, return to Listening when it disconnects, and can accept another client. Repeats, file sends, and bridges stop when the active client disconnects. TCP is a byte stream: reads can split or combine protocol messages.

UDP clients send to the configured peer and can receive datagrams from other senders. UDP listeners receive from arbitrary peers. **UDP send / reply target IP:port** and **Set UDP target** choose an explicit destination; receiving traffic never changes it. Each send, preset send, repeat iteration, or file chunk produces one datagram, including an empty payload. File chunk size is therefore packet size; the last chunk may be shorter. The portable send limit is 65507 bytes.

UDP traffic rows remain separate messages in every receive mode, even if a packet is empty or contains multiple line endings. Rows identify source and destination. Raw RX recording concatenates payload bytes only; use JSONL capture to retain packet boundaries and addresses. Capture version 1 has an optional `network` event field; older captures remain readable, and capture replay preserves this metadata and empty packets. Triggered post-byte limits retain the entire final datagram, so the byte count may exceed the requested limit by one packet.

Network endpoints use ordinary-user sockets. Use a high port such as 5000. An address-in-use error means another socket owns the bind; permission errors require choosing an allowed address/port. Binding `0.0.0.0` or `[::]` makes the listener reachable on all interfaces; `127.0.0.1` and `::1` restrict it to this machine. UDP does not guarantee delivery or ordering. Network terminals show observed timing; UART wire-time calculations are unavailable.

Related: [Network bridge framing](#network-bridge-framing), [Bridges](#bridges), [Recording RX](#recording-rx).

<!-- topic:network.framing -->
## Network bridge framing
> Stream-to-UDP bridges default to complete LF-terminated lines; UDP-to-stream forwards payloads unchanged.

Serial, SSH serial, and TCP endpoints are streams. A stream-to-UDP bridge must choose message boundaries. The default **Line** policy buffers until LF; `CMD1\r\nCMD2\r\n` becomes two packets containing `CMD1\r\n` and `CMD2\r\n`. **Strip LF / CRLF from datagrams** removes these endings; otherwise they are retained exactly. A blank line with stripping enabled sends an empty datagram.

For binary protocols choose **Fixed byte count**. **Idle gap** sends buffered bytes after the selected quiet period. **Raw chunks (advanced)** sends one transport read per packet and depends on OS read boundaries. Incomplete lines and fixed packets are discarded when a bridge stops. Messages larger than the UDP payload limit stop the bridge with an error rather than silently splitting a packet.

UDP-to-stream bridges write each received payload in receive order. Choose None, LF, CRLF, or CR to append an ending after each datagram; None preserves payloads unchanged. UDP-to-UDP bridges preserve packet boundaries and payloads. Destinations use their configured peer or explicit reply target.

Bridge framing choices save in the workspace. Select the same endpoints and choose **Use saved framing** to restore them; loading a workspace never starts a listener, connection, bridge, or send automatically.

Related: [Network endpoints](#network-endpoints), [Bridges](#bridges).
