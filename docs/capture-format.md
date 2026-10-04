# Signal Forge capture format, version 1

Captures are UTF-8 JSON Lines (`.jsonl`): one independent JSON object per line. They stream to the requested file on a worker thread, rather than collecting a capture in RAM. Existing files are never overwritten. Capture data is separate from workspace settings and preset profiles.

The stream contains a header, zero or more event records, and one footer on successful finalization. Process interruption or a write failure can leave an unfinished file or partial final line; a missing footer means the capture was not finalized. Completed captures can still have dropped events: check the footer's `complete` and `dropped_events` fields.

## Header

```json
{"type":"header","format":"signal-forge-capture","version":1,"started_unix_ns":"1791136800000000000","endpoint_a":"serial:/dev/ttyUSB0","endpoint_b":"serial:/dev/pts/3","queue_capacity":4096,"semantics":"successfully forwarded RX chunks; bounded best-effort capture"}
```

Endpoint IDs define the labels A and B for the lifetime of this capture. `started_unix_ns` is the header creation time; a queued first event can precede it. `queue_capacity` is measured in chunks, not bytes. Bridge RX chunks are at most 4 KiB, so the production capture queue holds at most approximately 16 MiB of raw payload. Capture does not retain the completed file's contents in memory.

## Event

```json
{"type":"event","sequence":1,"timestamp_unix_ns":"1791136800001000000","delta_ns":null,"direction":"a_to_b","source_endpoint":"serial:/dev/ttyUSB0","raw_bytes":[0,255,13,10],"missed_before":0}
```

- `sequence`: the bridge's chronological event sequence, shared across both directions. It need not begin at 1 when recording starts after a bridge was opened.
- `timestamp_unix_ns`: signed nanoseconds since the Unix epoch, captured when a complete forwarded RX chunk is published. It is stored as a decimal string to preserve precision in JSON consumers.
- `delta_ns`: signed decimal string for the difference from the previous recorded event's timestamp, or `null` for the first event. Wall-clock adjustments can produce negative deltas. This describes observed chunks, not UART byte timing or protocol-message boundaries.
- `direction`: `a_to_b` or `b_to_a`.
- `source_endpoint`: the originating endpoint's ID.
- `raw_bytes`: an array of byte values (0–255), with no text conversion. The example contains `00 FF 0D 0A` exactly.
- `missed_before`: number of sequence gaps since the previous recorded event. The footer also counts drops before the first or after the last recorded event.

The bridge publishes a chunk after its entire destination write succeeds. A write fault can accept a partial prefix before the bridge faults; that prefix is visible in destination terminal TX events but is not a complete bridge capture event.

## Footer

```json
{"type":"footer","ended_unix_ns":"1791136801000000000","events":1,"bytes":4,"dropped_events":0,"complete":true}
```

`events` and `bytes` count records and raw bytes written to this file. `dropped_events` is the capture subscription's exact dropped-chunk count, independently of the UI's monitor queue. `complete` is true only when no capture events were dropped. It describes this recording interval, not traffic before recording started.

Stopping is asynchronous: **Finishing** detaches the capture subscriber under the publication lock, drains its bounded queue, writes the footer, and flushes/synchronizes the file before reporting **Completed**. Incoming traffic cannot keep extending the drained queue. Bridge removal, endpoint faults, and normal application shutdown also finalize active captures. Display pause, clear, and direction filters do not change what is recorded; captures always include both directions.

Capture is best-effort. A full queue drops capture events and reports an incomplete capture instead of blocking endpoint forwarding. Disk write/synchronization failures are shown as **Fault** and can leave an unfinished file. A footer summarizes written records and queue drops; **Completed** additionally confirms the worker flushed and synchronized the file. The file worker and UI history are bounded; available disk space limits capture duration.

## Streaming reader example

```python
import json

with open("bridge.jsonl", encoding="utf-8") as capture:
    for line in capture:
        record = json.loads(line)
        if record["type"] == "event":
            payload = bytes(record["raw_bytes"])
            print(record["timestamp_unix_ns"], record["direction"], payload.hex(" "))
        elif record["type"] == "footer":
            print("Complete:", record["complete"], "Dropped chunks:", record["dropped_events"])
```
