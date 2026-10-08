# Remote serial over SSH

Open **New Port**, enable **Remote serial over SSH**, and enter a host or an SSH
config alias. Username and port are optional: leave them blank/zero to use your
normal SSH configuration. Give the host a display name and click **Save host** to
reuse it; **Remove saved host** removes the saved definition. Host definitions
contain only names, username and port, never passwords or private keys.

Click **Discover remote devices** to connect and list `/dev/serial/by-id/*`,
`/dev/ttyUSB*`, `/dev/ttyACM*` and `/dev/ttyS*`. Stable by-id paths appear first.
You can also enter an existing remote `/dev/pts/N` or other serial path manually.
Choose baud, data bits, parity, stop bits and flow control, then **Open port**.
New configurations default to 19200 baud, 8 data bits, no parity, one stop bit and
no flow control. Unsupported baud/framing settings report a configuration error
instead of falling back to different settings.

Connections appear alongside local terminals as, for example,
`ssh://pi-workbench/dev/ttyUSB0` or `ssh://bench@lab-pc:2222/dev/ttyACM0`.
The same URI works with `--port`. DNS names, IPv4 addresses and SSH aliases are
supported; use an SSH config alias for an IPv6 destination. Workspace saves keep
the URI, framing and display/send preferences. Restoring a workspace never
connects or transmits automatically. Reconnect explicitly.

## Requirements and authentication

The client needs OpenSSH (`openssh-client`). The remote machine needs a Linux
SSH server, Python 3 and permission to read/write its serial device. The helper is
embedded in Signal Forge and passed as a quoted remote command; nothing is
installed on the remote host. Python's Linux termios baud constants are supported;
arbitrary custom baud rates without a matching constant are rejected explicitly.
Remote noninteractive shell startup files must leave stdout free of banners.

OpenSSH reads the user's usual configuration and uses key files or the SSH agent.
Password prompts are disabled so authentication cannot freeze the app. Load
encrypted keys into your SSH agent before connecting. Private keys and passwords
are not stored in Signal Forge workspaces or host definitions.

Host-key verification is always strict. For a new host, connect using your normal
SSH client and verify its fingerprint before trusting it. Unknown or changed keys
produce an endpoint fault; Signal Forge does not accept or replace them. SSH
configuration can supply jump hosts, identity files and known-hosts locations.
An SSH config alias is the display-friendly way to name a bench.

## Existing tools

Line, Raw Chunk and Hex views, manual sends, repeats, presets, timing metadata,
bridges, traffic inspection and JSONL bridge captures share the normal Endpoint
pipeline. Both local-to-remote and remote-to-remote bridges work; display pause
does not stop forwarding. The SSH connection carries raw bytes without a remote
terminal allocation or newline conversion. Read timing includes SSH buffering and
network delay; calculated wire time still uses the configured serial framing.
For remote TX, traffic statistics describe bytes accepted by the SSH stream, not
an acknowledgement from the remote serial driver or device.

Select a terminal's **Files / RX** tool to send a binary file or record raw RX.
File transfers stream from disk in bounded chunks on the transport worker. A file
send cancels an active repeat; manual and repeated sends are rejected while the
file transfer is active. Disconnect cancels it. Bridge writes share the same
stream, so stop a bridge if the recipient requires an uninterrupted file.

Raw RX recording uses its own worker and is independent of display mode, clear
and pause. It creates a new file, never overwrites an existing one, and records
only that endpoint's received bytes. Stop, disconnect or normal shutdown drains
queued bytes and finishes the file. Recording is bounded best-effort monitoring:
overload or disk errors are reported in the recording status; an incomplete raw
file has no embedded metadata or footer. Bridge JSONL capture remains available
when persistent capture integrity metadata is needed.

## Reliability and tests

Connecting and discovery run independently of GUI painting. OpenSSH connection
attempts time out after 10 seconds; helper readiness and discovery are bounded
by 15 seconds. Keepalives detect lost SSH sessions. Endpoint faults include SSH
authentication/network/host-key diagnostics or remote device/configuration errors.
There is no automatic reconnect or automatic restart of repeats or bridges.
Disconnect terminates/reaps the SSH client and closes the remote device/helper.

Run the ordinary Rust and graphical test suites as documented in the README.
For the isolated real SSH suite (requires `openssh-server`, `ssh-agent`, Xvfb and
the standard GUI test utilities), run:

```sh
python3 scripts/test-remote-serial.py
```

The fixture creates temporary host/client keys, SSH configuration, known-hosts,
an empty SSH agent and a loopback SSH server. It never changes your SSH files or
trust settings. It tests key and agent authentication, aliases, unknown/changed
host rejection, authentication denial, binary/fragmented duplex traffic, repeat
sends, a file larger than a single message, exact raw recording, both bridge
types, reconnect, device disappearance, unsupported baud and abrupt SSH loss.
A real GUI test verifies remote sends/presets and disconnected workspace restore.
CI invokes this suite separately; its Rust SSH fixture test is explicitly ignored
in ordinary `cargo test` runs. Physical electrical parity/flow behavior still
requires a hardware bench.
