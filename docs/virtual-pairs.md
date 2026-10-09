# Virtual pair compatibility and timing

Signal Forge creates two owned Linux PTYs with a bounded full-duplex relay. Creation, opening, binary transfer, timing emulation, diagnostics, and recovery use ordinary user permissions. Running Signal Forge as root, adding capabilities/groups, changing device modes, kernel modules, and real-time scheduling privileges are unnecessary for these features. Links belong in a folder you can write; protected system directories are not needed.

## Connecting external applications

Create a pair, copy both paths, and open **one side** in Signal Forge. Configure the external application with the **other side**. Alternatively, two external programs can use the two sides while Signal Forge owns only the relay. Avoid opening both ends in Signal Forge when another program needs an end.

A serial client can take an exclusive lock (`TIOCEXCL` and/or an advisory file lock). A second opener of that same side then receives `EBUSY` or a locking error, even for the same user. It does not mean the peer is unavailable or that sudo is needed. Disconnect the client occupying that side; do not bypass it with elevated privileges.

**Check paths/access** shows the advertised path, its raw `/dev/pts/N` target, permissions/owner, and whether an ordinary read/write open and advisory lock check succeeds. Missing/replaced paths and exclusive access failures appear explicitly. This is an on-demand access check, not a count of arbitrary external clients. Clients that do not take locks cannot reliably be identified as connected. The sidebar separately identifies sides in use by Signal Forge. Pair **Open** actions start with ordinary 8N1/no-flow-control PTY settings instead of inheriting a physical port's hardware/software flow-control configuration.

### Stale exclusivity after an external program exits

The compatibility investigation reproduced GNU screen leaving `TIOCEXCL` set after its serial client exited. New opens continued to fail with `EBUSY`. This happens with both Signal Forge and the socat baseline; closing the client alone need not clear the flag while the PTY master remains alive. Normal Signal Forge disconnection clears its own serial-port exclusivity.

After the external program has **exited**, click **Release stale exclusive flag** on the affected side and check access again. Signal Forge uses `TIOCNXCL` on a descriptor it already owns, so this recovery needs no sudo or added privilege. It does not change permissions and does not override advisory locks held by an active process. It is disabled on sides currently used by Signal Forge. Do not release a live external client's exclusive flag: doing so permits conflicting opens. Recovery is always explicit; the relay never automatically breaks clients' locks.

## Comparison evidence

The baseline command is:

```
socat PTY,raw,echo=0,link=/tmp/ttyV0 PTY,raw,echo=0,link=/tmp/ttyV1
```

`python3 scripts/test-virtual-compat.py` compares the actual Signal Forge relay process against socat. It requires an ordinary, non-root account and the optional comparison tools socat, picocom, minicom, and screen. It verifies:

- Two independent external processes exchange every byte value in both directions.
- Picocom, minicom, and screen open an endpoint and exchange traffic with the peer.
- Same-side exclusivity blocks opening that side while the peer remains usable.
- Stale screen exclusivity can be cleared without elevation and the endpoint reopened.
- Advertised paths disappear after normal owner shutdown.

The comparison uses `minicom -o` to skip modem dialing/initialization and lockfile startup behavior. Screen uses `-ln` and a private user-owned `SCREENDIR`, avoiding system socket-directory and login-record requirements. These are terminal-program settings, not Signal Forge privilege requirements. Raw PTYs preserve binary bytes; a terminal program can still intentionally translate input, display control bytes, or restore termios settings. Choose raw/no-echo settings for raw external clients. A PTY supplies terminal/byte-stream semantics; it does not supply USB device discovery, physical modem-control pins, or hardware UART errors. Programs requiring those hardware features may behave the same way with socat.

Signal Forge retains both slave descriptors for the intended pair lifetime, preventing relay EIO when applications close/reopen and allowing explicit lock recovery. Creation applies raw/no-echo termios, masters are nonblocking, and each direction has a 64 KiB relay queue. Named links never overwrite existing paths and cleanup preserves paths replaced by another process. PTYs and links are session resources and are not recreated automatically after restart. Place disposable named links in a temporary directory; forced process termination can leave dangling user-created links.

## Baud/framing emulation

Pair setup offers **Unlimited** (the existing fast relay) or **Emulated baud**. Emulated mode configures one shared link with baud, data bits, parity, and stop bits; the default is 19200 8N1. Each direction has its own queue and target timeline, allowing concurrent full-duplex transfer. Recreate the pair to change its link configuration.

Frame length is `1 start + data bits + optional parity bit + stop bits`. At 19200 8N1 it is 10 bits, so one byte takes approximately 0.521 ms and 100 bytes take 52.1 ms. At 19200 8E2 it is 12 bits, so 100 bytes take 62.5 ms. The relay and calculated wire-time diagnostics share the same framing calculation.

Bytes become available progressively, with small groups determined by desktop scheduling. The worker schedules against an absolute rational timeline instead of sleeping a rounded interval for every byte; it does not accumulate per-byte sleep/rounding drift. The first byte becomes eligible after one frame interval. An idle queue starts a fresh timeline. A backpressured receiver pauses that direction's timeline, and queued writes can be cancelled promptly by removing/stopping the pair.

This is software timing emulation. Desktop scheduler jitter and PTY/application read coalescing affect observed chunks. At rates beyond the host's scheduling/transport throughput, delivery cannot reach hardware-level timing precision. Unlimited remains available for fast bulk testing; no real-time privilege is requested.

Framing changes **delay only**, preserving all eight bits of every byte even when modeling a 5–7-bit frame. It does not create parity bits or frame errors. The underlying PTY can remain ordinary 8N1; changing an external client's termios does not change the configured virtual link. The UI labels the emulated link independently from endpoint settings. Received events use link framing for calculated wire time, so normal Line/Raw Chunk/Hex views, observed timing, analysis/statistics, recording/capture, and bridging continue to operate on unchanged bytes with paced receipt timestamps.

Automated PTY tests cover progressive delivery, 19200 8N1 and alternate framing, longer simultaneous duplex transfers, Unlimited speed, exact binary bytes, close/reopen, diagnostics, unprivileged stale-lock recovery, and prompt stop with queued data. The Xvfb virtual-pair smoke test exercises the real setup controls, safe endpoint defaults, peer ownership, paced binary RX recording, and cleanup.
