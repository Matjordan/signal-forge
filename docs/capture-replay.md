# Triggered capture and replay

Open **Triggered capture** in a connected terminal. **Save as…** chooses a new JSONL file; existing files are never overwritten. Configure pre/post windows, choose a trigger, and **Arm capture**. Normal RX recording and bridge capture remain available independently.

Triggers are manual (**Trigger now**), received text, hex bytes, byte regex, or the first nonempty RX chunk after an RX idle interval. Pattern triggers use the shared traffic matcher and can span adjacent RX chunks with up to 4,095 preceding bytes of context. TX does not trigger RX rules. Idle uses original RX timestamps, and waits for a second RX event; it does not trigger merely because a connection stays quiet.

Pre-trigger retention is capped at 16 MiB and 2,048 events. Its optional age limit uses monotonic worker time. Oldest whole chunks are removed to fit the byte cap; a chunk larger than that cap is not retained. Pre-trigger records precede the triggering chunk. The triggering chunk is saved whole, followed by post-trigger records until the first enabled byte/time limit. A final post chunk may be clipped to the exact byte budget. Zero time disables a timer; zero post bytes disables that limit; both post limits zero stop immediately after the trigger. **Stop capture** finishes early. Captures are one-shot: choose another filename and arm again.

Capture evaluation and writing run on a worker with a separate 64-event queue. Overload drops capture events instead of blocking serial reads or bridging; the UI and footer report drops and an incomplete capture. Stop, disconnect, and shutdown detach the subscriber and finalize queued traffic. Stopping before a trigger produces a cancelled capture with no traffic records. Disk errors are reported and may leave an unfinished file.

## Terminal capture format

The UTF-8 JSONL header uses `format: "signal-forge-terminal-capture"`, `version: 1`, `endpoint`, and decimal-string `started_unix_ns`. Event records contain `sequence`, decimal-string `timestamp_unix_ns`, `direction` (`rx` or `tx`), `source_endpoint`, and `raw_bytes` (an array of exact byte values). The footer contains `events`, `bytes`, `dropped_events`, `complete`, and `triggered`. Sequence/timestamp gaps are possible, and replay preserves original event timestamps. This format supplements the existing [bridge capture format](capture-format.md).

## Replay

Click **Replay…**, **Browse…**, choose the format and stream, then **Open paused replay**. Loading validates the entire source and snapshots it to an anonymous disk file before playback. A replay terminal is clearly labelled read-only and starts paused. Its normal Line, Raw Chunk, Hex, search, highlight, statistics, and timing tools consume ordinary traffic events.

- **Raw** preserves file bytes exactly. A rendered ASCII recording can be opened as literal text; its escapes cannot unambiguously recover the original binary bytes.
- **Hex** decodes Signal Forge's hex RX recordings and rejects invalid input.
- **Capture** supports both version-1 JSONL formats. RX maps to bridge A→B and TX to B→A. Choose one direction or both in file order. Valid captures without a footer can be replayed with a warning; malformed records, invalid footer counts, or unsupported versions fail before playback.

**Play**, **Pause playback**, **Step**, and **Stop / Reset** control playback. Captures default to original timestamp intervals; backwards timestamps cause no delay and remain visible unchanged. **Maximum speed** skips delays. Raw/Hex have no recorded timing: the default is 4,096-byte chunks, 10 ms between chunks, with synthetic timestamps anchored at loading; chunk size and pacing are configurable. Step emits one entry/chunk without its delay. Reset rewinds the snapshot and counters; retained terminal history remains available until explicitly cleared.

A replay can feed a live serial/remote endpoint through an ordinary bridge. This output is one-way: replies cannot enter the replay source, and sending into it is disabled. Both selected streams are emitted if requested. Pause allows an in-flight output chunk to finish. Reset or disconnect cancels pending bridge output; reset stops that bridge, so restart it explicitly before replaying again. A pair of read-only sources cannot form a bridge. The original file is never modified.

Workspace saving retains the replay source path/options, but restores the terminal disconnected. Reconnect explicitly to load it paused; playback and bridges never restart automatically.
