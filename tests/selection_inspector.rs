use signal_forge::{
    config::SerialSettings,
    endpoint::EndpointId,
    selection_inspector::{map_text, nmea_sentences, Inspection, RenderKind},
    terminal_display::{LineDelimiter, LineDisplay, SerialFraming, SourceRun, LINE_BYTE_LIMIT},
    traffic::{self, Direction, TrafficEvent},
};
use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
fn frame() -> SerialFraming {
    SerialFraming::from(&SerialSettings::default())
}
fn event(sequence: u64, direction: Direction, bytes: &[u8]) -> TrafficEvent {
    TrafficEvent {
        sequence,
        timestamp: UNIX_EPOCH + Duration::from_millis(sequence * 10),
        endpoint: EndpointId("serial:test".into()),
        direction,
        bytes: Arc::from(bytes),
    }
}
fn inspection(bytes: &[u8]) -> Inspection {
    let source = SourceRun {
        start: 0,
        len: bytes.len(),
        sequence: 1,
        timestamp: UNIX_EPOCH,
        framing: frame(),
        stream_offset: 0,
        epoch: 0,
    };
    let mut result = Inspection::default();
    result.append(bytes, 0..bytes.len(), Direction::Rx, &[source]);
    result
}
#[test]
fn rendered_glyphs_match_existing_views_and_partial_escapes_are_atomic() {
    let data = b"a\0\r\n\t\xff\\xFF\xc2\x85\xce\xbb";
    let unicode = map_text(data, RenderKind::Unicode { controls: true });
    assert_eq!(
        unicode.text,
        signal_forge::terminal_display::control_text(data)
    );
    assert_eq!(
        map_text(data, RenderKind::Unicode { controls: false }).text,
        signal_forge::terminal_display::line_text(data)
    );
    assert_eq!(map_text(data, RenderKind::Ascii).text, traffic::ascii(data));
    assert_eq!(map_text(data, RenderKind::Hex).text, traffic::hex(data));
    let mapped = map_text(b"\xffZ", RenderKind::Ascii);
    assert_eq!(mapped.selected(1..2), Some(0..1));
    assert_eq!(
        map_text(b"\\xFF", RenderKind::Ascii).selected(1..2),
        Some(1..2)
    );
    let hex = map_text(b"\0\xff", RenderKind::Hex);
    assert_eq!(hex.selected(2..3), None); // whitespace alone is decoration
    assert_eq!(hex.selected(4..5), Some(1..2));
    assert_eq!(
        map_text("AλB".as_bytes(), RenderKind::Unicode { controls: false }).selected(1..2),
        Some(1..3)
    );
    assert_eq!(
        map_text(b"\xf0\x9f", RenderKind::Unicode { controls: false }).text,
        "\\xF0\\x9F"
    );
}
#[test]
fn split_reads_partial_range_uses_only_contributing_events_and_framing() {
    let mut lines = LineDisplay::default();
    lines.receive(&event(1, Direction::Rx, b"LEFT"));
    let mut changed = frame();
    changed.baud = 9600;
    lines.receive_with_framing(&event(2, Direction::Rx, b"RIGHT\r"), changed);
    lines.receive_with_framing(&event(3, Direction::Rx, b"\n"), changed);
    let row = lines.row(0).unwrap();
    let mut bytes = row.bytes.clone();
    bytes.extend_from_slice(&row.terminator);
    assert_eq!(bytes, b"LEFTRIGHT\r\n");
    let mut selected = Inspection::default();
    selected.append(&bytes, 4..9, Direction::Rx, &row.sources);
    assert_eq!(selected.bytes, b"RIGHT");
    assert_eq!(
        selected.event_times(),
        Some((
            UNIX_EPOCH + Duration::from_millis(20),
            UNIX_EPOCH + Duration::from_millis(20)
        ))
    );
    assert_eq!(selected.observed_span(), Some(Duration::ZERO));
    assert!((selected.wire_seconds().unwrap() - 5.0 * 10.0 / 9600.0).abs() < 1e-12);
    selected = Inspection::default();
    selected.append(&bytes, 2..bytes.len(), Direction::Rx, &row.sources);
    assert_eq!(selected.observed_span(), Some(Duration::from_millis(20)));
    assert_eq!(selected.framing().len(), 2);
    assert_eq!(selected.segments(), vec![0..9]);
}
#[test]
fn nmea_validation_and_arbitrary_selection_xor_are_separate() {
    let valid = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n";
    let nmea = nmea_sentences(valid);
    assert_eq!(nmea.len(), 1);
    assert!(nmea[0].valid());
    assert_eq!(nmea[0].calculated, 0x47);
    assert_eq!(nmea_sentences(b"!ABC*40\r\n")[0].calculated, 0x40);
    assert!(nmea_sentences(b"!ABC*40\r\n")[0].valid());
    assert!(!nmea_sentences(b"$ABC*41")[0].valid());
    assert_eq!(nmea_sentences(b"$ABC*41")[0].received, 0x41);
    let arbitrary = inspection(b"#0,MED");
    assert_eq!(arbitrary.xor(), 0x73);
    assert!(arbitrary.nmea().is_empty());
    assert_eq!(inspection(b"#0,MED\r\n").xor(), 0x74);
    for bytes in [
        b"$ABC".as_slice(),
        b"$ABC*G0",
        b"$ABC*4",
        b"ABC*40",
        b"$ABC*400",
        b"$A\0B*00",
    ] {
        assert!(nmea_sentences(bytes).is_empty(), "{bytes:?}");
    }
    assert_eq!(nmea_sentences(b"$ABC*40\r\n!ABC*40\r\n").len(), 2);
}
#[test]
fn direction_gaps_epochs_and_unknown_provenance_do_not_form_fake_packets() {
    let mut result = inspection(b"$ABC");
    let source = SourceRun {
        start: 0,
        len: 3,
        sequence: 2,
        timestamp: UNIX_EPOCH + Duration::from_millis(2),
        framing: frame(),
        stream_offset: 0,
        epoch: 0,
    };
    result.append(b"*40", 0..3, Direction::Tx, &[source.clone()]);
    assert_eq!(result.segments(), vec![0..4, 4..7]);
    assert!(result.nmea().is_empty());
    let mut gap = inspection(b"$ABC");
    gap.append(
        b"*40",
        0..3,
        Direction::Rx,
        &[SourceRun {
            stream_offset: 5,
            ..source.clone()
        }],
    );
    assert_eq!(gap.segments().len(), 2);
    assert!(gap.nmea().is_empty());
    let mut epoch = inspection(b"$ABC");
    epoch.append(
        b"*40",
        0..3,
        Direction::Rx,
        &[SourceRun {
            stream_offset: 4,
            epoch: 1,
            ..source
        }],
    );
    assert!(epoch.nmea().is_empty());
    epoch.append(b"\xff", 0..1, Direction::Rx, &[]);
    assert!(epoch.missing_metadata());
    assert!(epoch.event_times().is_none());
    assert!(epoch.wire_seconds().is_none());
    assert!(epoch.character_count().is_none());
}
#[test]
fn retained_provenance_is_bounded_and_truncated_lines_keep_true_terminator_offsets() {
    let mut lines = LineDisplay::default();
    for i in 1..=1100 {
        lines.receive(&event(i, Direction::Rx, b"A"));
    }
    let row = lines.row(0).unwrap();
    assert!(row.sources.len() <= 1024);
    let mut selected = Inspection::default();
    selected.append(&row.bytes, 1050..1100, Direction::Rx, &row.sources);
    assert_eq!(selected.bytes.len(), 50);
    assert!(selected.missing_metadata());
    assert!(selected.wire_seconds().is_none());
    let mut lines = LineDisplay::new(LineDelimiter::CrLf);
    let mut bytes = vec![b'A'; LINE_BYTE_LIMIT + 100];
    bytes.extend_from_slice(b"\r\n");
    lines.receive(&event(1, Direction::Rx, &bytes));
    let row = lines.row(0).unwrap();
    assert!(row.truncated);
    let mut bytes = row.bytes.clone();
    bytes.extend_from_slice(&row.terminator);
    let mut selected = Inspection::default();
    selected.append(&bytes, 0..bytes.len(), Direction::Rx, &row.sources);
    assert_eq!(selected.bytes.len(), LINE_BYTE_LIMIT + 2);
    assert_eq!(selected.segments().len(), 2);
    assert!(!selected.missing_metadata());
}
#[test]
fn backwards_clock_single_read_and_invalid_framing_are_honest() {
    let mut value = inspection(b"a");
    assert_eq!(value.observed_span(), Some(Duration::ZERO));
    let source = SourceRun {
        start: 0,
        len: 1,
        sequence: 2,
        timestamp: UNIX_EPOCH - Duration::from_secs(1),
        framing: SerialFraming { baud: 0, ..frame() },
        stream_offset: 1,
        epoch: 0,
    };
    value.append(b"b", 0..1, Direction::Rx, &[source]);
    assert!(value.observed_span().is_none());
    assert!(value.wire_seconds().is_none());
}

#[test]
fn unicode_glyph_split_across_reads_keeps_both_byte_origins() {
    let mut lines = LineDisplay::default();
    lines.receive(&event(1, Direction::Rx, b"A\xce"));
    lines.receive(&event(2, Direction::Rx, b"\xbbB\r\n"));
    let row = lines.row(0).unwrap();
    let map = map_text(&row.bytes, RenderKind::Unicode { controls: false });
    assert_eq!(map.text, "AλB");
    let mut selected = Inspection::default();
    selected.append(
        &row.bytes,
        map.selected(1..2).unwrap(),
        Direction::Rx,
        &row.sources,
    );
    assert_eq!(selected.bytes, "λ".as_bytes());
    assert_eq!(selected.character_count(), Some(1));
    assert_eq!(selected.observed_span(), Some(Duration::from_millis(10)));
    assert_eq!(selected.pieces.len(), 2);
}
