use signal_forge::{
    capture::Capture,
    endpoint::EndpointId,
    terminal_display::{line_text, LineDelimiter, LineDisplay, LINE_BYTE_LIMIT, ROW_LIMIT},
    traffic::{Direction, TrafficBus, TrafficEvent},
};
use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
fn event(sequence: u64, direction: Direction, bytes: &[u8]) -> TrafficEvent {
    TrafficEvent {
        sequence,
        timestamp: UNIX_EPOCH + Duration::from_millis(sequence),
        endpoint: EndpointId("A".into()),
        direction,
        bytes: Arc::from(bytes),
    }
}
fn feed(display: &mut LineDisplay, chunks: &[&[u8]]) {
    for (index, chunk) in chunks.iter().enumerate() {
        display.receive(&event(index as u64 + 1, Direction::Rx, chunk));
    }
}
fn rows(display: &LineDisplay) -> Vec<Vec<u8>> {
    display.rows.iter().map(|row| row.bytes.clone()).collect()
}
#[test]
fn fragmented_line_and_split_crlf_have_no_spurious_rows() {
    for delimiter in [LineDelimiter::Auto, LineDelimiter::CrLf] {
        let mut display = LineDisplay::new(delimiter);
        feed(&mut display, &[b"STA", b"TUS=OK\r", b"\n"]);
        assert_eq!(rows(&display), vec![b"STATUS=OK".to_vec()]);
        assert!(display.pending.is_none());
        assert_eq!(
            display.rows[0].timestamp,
            UNIX_EPOCH + Duration::from_millis(1)
        );
        display.clear();
        feed(&mut display, &[b"HELLO\r", b"\nWORLD\r", b"\n"]);
        assert_eq!(rows(&display), vec![b"HELLO".to_vec(), b"WORLD".to_vec()]);
    }
}
#[test]
fn combined_lines_empty_lines_and_partial_next_line() {
    let mut display = LineDisplay::default();
    feed(&mut display, &[b"ONE\r\nTWO\r\nTHREE\r\n\r\nPART"]);
    assert_eq!(
        rows(&display),
        vec![b"ONE".to_vec(), b"TWO".to_vec(), b"THREE".to_vec(), vec![]]
    );
    assert_eq!(display.pending.as_ref().unwrap().bytes, b"PART");
    assert!(!display.pending.as_ref().unwrap().complete);
    feed(&mut display, &[b"IAL\n"]);
    assert_eq!(display.rows.back().unwrap().bytes, b"PARTIAL");
    assert!(display.pending.is_none());
}
#[test]
fn line_assembly_is_independent_of_all_chunk_boundaries() {
    let bytes = b"\r\nONE\r\n\nTWO\rTHREE\n\r\nPART";
    let mut whole = LineDisplay::default();
    feed(&mut whole, &[bytes]);
    for split in 0..=bytes.len() {
        let mut fragmented = LineDisplay::default();
        feed(&mut fragmented, &[&bytes[..split], &bytes[split..]]);
        assert_eq!(rows(&fragmented), rows(&whole));
        assert_eq!(fragmented.pending.as_ref().unwrap().bytes, b"PART");
    }
    let mut single_bytes = LineDisplay::default();
    for byte in bytes {
        feed(&mut single_bytes, &[std::slice::from_ref(byte)]);
    }
    assert_eq!(rows(&single_bytes), rows(&whole));
}
#[test]
fn explicit_delimiters_only_consume_their_own_terminators() {
    for (delimiter, input, expected) in [
        (
            LineDelimiter::Lf,
            b"A\r\nB\n".as_slice(),
            vec![b"A\r".to_vec(), b"B".to_vec()],
        ),
        (
            LineDelimiter::Cr,
            b"A\n\rB\r".as_slice(),
            vec![b"A\n".to_vec(), b"B".to_vec()],
        ),
        (
            LineDelimiter::CrLf,
            b"A\nB\rC\r\n".as_slice(),
            vec![b"A\nB\rC".to_vec()],
        ),
    ] {
        let mut display = LineDisplay::new(delimiter);
        for byte in input {
            feed(&mut display, &[std::slice::from_ref(byte)]);
        }
        assert_eq!(rows(&display), expected);
        assert!(display.pending.is_none());
    }
}
#[test]
fn tx_does_not_flush_rx_or_break_split_crlf() {
    let mut display = LineDisplay::default();
    display.receive(&event(1, Direction::Rx, b"HE"));
    display.receive(&event(2, Direction::Tx, b"request\r\n"));
    assert_eq!(display.pending.as_ref().unwrap().bytes, b"HE");
    display.receive(&event(3, Direction::Rx, b"LLO\r"));
    display.receive(&event(4, Direction::Tx, b"another"));
    display.receive(&event(5, Direction::Rx, b"\nWORLD\n"));
    let rx: Vec<_> = display
        .rows
        .iter()
        .filter(|row| row.direction == Direction::Rx)
        .map(|row| row.bytes.as_slice())
        .collect();
    assert_eq!(rx, vec![b"HELLO".as_slice(), b"WORLD".as_slice()]);
    assert_eq!(display.rows[0].bytes, b"request\r\n");
}
#[test]
fn unicode_survives_fragmentation_and_invalid_bytes_are_escaped() {
    let mut display = LineDisplay::default();
    feed(
        &mut display,
        &[b"caf\xc3", b"\xa9 \xf0\x9f", b"\x98\x80\x00\xff\n"],
    );
    assert_eq!(line_text(&display.rows[0].bytes), "café 😀\\x00\\xFF");
    assert_eq!(line_text(b"\xff\xc3\t\r\n"), "\\xFF\\xC3\\t\\r\\n");
}
#[test]
fn bounds_clear_and_display_gaps_are_explicit() {
    let mut display = LineDisplay::default();
    feed(&mut display, &[&vec![b'X'; LINE_BYTE_LIMIT + 100]]);
    assert_eq!(
        display.pending.as_ref().unwrap().bytes.len(),
        LINE_BYTE_LIMIT
    );
    assert!(display.pending.as_ref().unwrap().truncated);
    feed(&mut display, &[b"\n"]);
    assert!(display.rows[0].truncated);
    for _ in 0..ROW_LIMIT + 1 {
        feed(&mut display, &[b"line\n"]);
    }
    assert_eq!(display.rows.len(), ROW_LIMIT);
    feed(&mut display, &[b"old partial"]);
    display.discard_pending();
    feed(&mut display, &[b"new\n"]);
    assert_eq!(display.rows.back().unwrap().bytes, b"new");
    display.clear();
    assert!(display.is_empty());
    feed(&mut display, &[b"\n"]);
    assert_eq!(rows(&display), vec![Vec::<u8>::new()]);
}
#[test]
fn presentation_leaves_raw_recording_events_and_capture_bytes_exact() {
    let bus = TrafficBus::default();
    let id = EndpointId("A".into());
    let display_rx = bus.subscribe(32);
    let raw_rx = bus.subscribe(32);
    let path = std::env::temp_dir().join(format!(
        "signal-forge-line-capture-{}.jsonl",
        std::process::id()
    ));
    let mut capture = Capture::start_subscription(
        &path,
        id.clone(),
        EndpointId("B".into()),
        bus.subscribe_tracked(32),
    )
    .unwrap();
    let chunks: &[&[u8]] = &[b"STA", b"TUS=OK\r", b"\nONE\r\nPART", b"\x00\xff\n"];
    let mut display = LineDisplay::default();
    for chunk in chunks {
        bus.publish(id.clone(), Direction::Rx, chunk);
        let event = display_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let raw = raw_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        display.receive(&event);
        assert!(Arc::ptr_eq(&raw, &event));
        assert_eq!(raw.bytes.as_ref(), *chunk);
    }
    capture.finish();
    let text = std::fs::read_to_string(&path).unwrap();
    let recorded: Vec<Vec<u8>> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|row| row["type"] == "event")
        .map(|row| serde_json::from_value(row["raw_bytes"].clone()).unwrap())
        .collect();
    assert_eq!(
        recorded,
        chunks
            .iter()
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        rows(&display),
        vec![
            b"STATUS=OK".to_vec(),
            b"ONE".to_vec(),
            b"PART\x00\xff".to_vec()
        ]
    );
    std::fs::remove_file(path).unwrap();
}
