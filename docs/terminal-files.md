# Terminal file sending and RX recording

Open **Files / RX** in the terminal you want to use. Paths are on the computer
running Signal Forge, including when the serial endpoint is remote over SSH.

## Send a file

Click **Browse…** to select the source file (or enter its path), select a mode,
and click **Send file**:

- **Raw / Binary** sends the exact file bytes.
- **ASCII / Text (UTF-8)** sends valid UTF-8 text unchanged, including CR/LF and
  literal backslashes. It adds no line ending and interprets no escapes.
- **Hex** decodes hexadecimal byte pairs. Spaces, tabs, and line breaks are
  accepted; invalid characters or an unmatched digit fail before any bytes send.

Preparation and validation run in a background worker using bounded memory and
an anonymous temporary file. The snapshot requires temporary disk space equal to
the transmitted payload and prevents source edits from changing validated bytes.
Progress reports transmitted bytes and total payload bytes, with Preparing,
Sending, Completed, Cancelled, or Failed state. **Cancel file send** stops the
remaining bytes; bytes already accepted by the transport cannot be recalled.

**Chunk** sets bytes per write (1–65,536); **Delay** sets milliseconds between
chunks (0–60,000). RX continues throughout preparation and transmission. Writes
use the endpoint's normal transport and TX events. Disconnect stops the transfer.
These are byte transfers, without a receiver-side transfer protocol.

## Record received bytes

Click **Save as…** to choose a new output file (or enter its path), select a
mode, and click **Record RX**. Selecting a path does not start sending or
recording; cancelling either dialog leaves the path unchanged. Existing output
files remain protected from overwriting.

Recording modes:

- **Raw / Binary** saves exact received bytes.
- **ASCII / Text (escaped binary)** preserves printable ASCII, tabs, CR, and LF;
  other bytes appear as uppercase `\xNN` escapes. Existing backslashes remain
  unchanged. This is a readable rendering; use Raw or Hex for exact replay.
- **Hex** saves uppercase byte pairs separated by spaces, with up to 16 bytes
  per line. Send this file in Hex mode to replay the original bytes.

Recording contains only this terminal's RX payload, without timestamps, TX,
endpoint labels, or sequence numbers. It runs independently of display mode,
pause, clear, and history limits. Each terminal can record separately.

The status shows received payload bytes, written output bytes, and dropped events.
**Stop recording** detaches the subscription immediately, drains already queued
RX, and flushes the file; disconnect and application shutdown do the same.
Completed means the file was finalized. Incomplete means the recording queue
overflowed; Failed reports a file error. Existing files are never overwritten.
