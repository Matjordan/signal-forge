use signal_forge::{
    endpoint::EndpointId,
    terminal_display::LineDelimiter,
    traffic::{Direction, TrafficEvent},
    traffic_analysis::{
        duration_text, Matcher, Pattern, PatternMode, Statistics, Timeline, Visibility,
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};
fn event(sequence: u64, ms: u64, direction: Direction, bytes: &[u8]) -> TrafficEvent {
    TrafficEvent {
        sequence,
        timestamp: UNIX_EPOCH + Duration::from_millis(ms),
        endpoint: EndpointId("test".into()),
        direction,
        bytes: Arc::from(bytes),
    }
}
#[test]
fn matching_is_shared_binary_safe_unicode_and_validated() {
    for (mode, pattern) in [
        (PatternMode::Text, "λ"),
        (PatternMode::Hex, "00 ff 0D\n0a"),
        (PatternMode::Regex, r"\x00\xFF\r\n"),
    ] {
        let matcher = Pattern {
            mode,
            value: pattern.into(),
        }
        .compile()
        .unwrap()
        .unwrap();
        assert!(matcher.is_match("λ\0".as_bytes()) || matcher.is_match(b"prefix\0\xff\r\nsuffix"));
        assert!(!matcher.is_match(b"unrelated"));
    }
    for (mode, pattern) in [
        (PatternMode::Hex, "0"),
        (PatternMode::Hex, "ZZ"),
        (PatternMode::Hex, " \t"),
        (PatternMode::Regex, "["),
    ] {
        assert!(Pattern {
            mode,
            value: pattern.into()
        }
        .compile()
        .is_err());
    }
    assert!(Pattern::default().compile().unwrap().is_none());
    assert!(Pattern {
        mode: PatternMode::Text,
        value: "x".repeat(4097)
    }
    .compile()
    .is_err());
    assert!(!Matcher::Bytes(vec![]).is_match(b"anything"));
}
#[test]
fn hidden_rows_update_direction_clocks_not_displayed_clock_and_reversed_time_is_unavailable() {
    let mut timeline = Timeline::default();
    let at = |ms| UNIX_EPOCH + Duration::from_millis(ms);
    assert!(timeline
        .observe(at(10), Direction::Rx, true)
        .displayed
        .is_none());
    timeline.observe(at(20), Direction::Tx, false);
    let row = timeline.observe(at(30), Direction::Rx, true);
    assert_eq!(row.displayed, Some(Duration::from_millis(20)));
    assert_eq!(row.rx, Some(Duration::from_millis(20)));
    assert_eq!(row.tx, Some(Duration::from_millis(10)));
    let backwards = timeline.observe(at(5), Direction::Rx, true);
    assert!(backwards.displayed.is_none() && backwards.rx.is_none() && backwards.tx.is_none());
    assert!(Visibility::Rx.includes(Direction::Rx));
    assert!(!Visibility::Rx.includes(Direction::Tx));
    assert!(Visibility::Tx.includes(Direction::Tx));
    assert!(Visibility::Both.includes(Direction::Rx) && Visibility::Both.includes(Direction::Tx));
    assert_eq!(duration_text(Duration::from_micros(123)), "123 µs");
    assert_eq!(duration_text(Duration::from_millis(14)), "14.00 ms");
    assert_eq!(duration_text(Duration::from_secs(2)), "2.000 s");
}
#[test]
fn latency_latest_tx_wins_first_rx_only_and_clock_changes_do_not_fabricate_samples() {
    let now = Instant::now();
    let mut stats = Statistics::new(now);
    for e in [
        event(1, 100, Direction::Tx, b"first"),
        event(2, 105, Direction::Tx, b"second"),
        event(3, 119, Direction::Rx, b"reply"),
        event(4, 120, Direction::Rx, b"more"),
    ] {
        stats.observe(&e, now, LineDelimiter::Auto);
    }
    let sample = stats.last_latency.as_ref().unwrap();
    assert_eq!((sample.tx_sequence, sample.rx_sequence), (2, 3));
    assert_eq!(sample.duration, Duration::from_millis(14));
    assert_eq!(sample.tx_timestamp, UNIX_EPOCH + Duration::from_millis(105));
    assert_eq!(stats.latency_count, 1);
    assert_eq!(stats.superseded_tx, 1);
    stats.observe(
        &event(5, 200, Direction::Tx, b"repeat"),
        now,
        LineDelimiter::Auto,
    );
    stats.observe(
        &event(6, 220, Direction::Rx, b"reply"),
        now,
        LineDelimiter::Auto,
    );
    assert_eq!(stats.average_latency(), Some(Duration::from_millis(17)));
    stats.observe(
        &event(7, 300, Direction::Tx, b"request"),
        now,
        LineDelimiter::Auto,
    );
    stats.observe(
        &event(8, 250, Direction::Rx, b"clock backwards"),
        now,
        LineDelimiter::Auto,
    );
    assert_eq!(stats.latency_count, 2);
    assert!(stats.clock_regressions > 0);
    stats.observe(
        &event(9, 400, Direction::Rx, b"unsolicited"),
        now,
        LineDelimiter::Auto,
    );
    assert_eq!(stats.latency_count, 2);
}
#[test]
fn rates_decay_and_reset_resets_only_statistics_with_bounded_long_running_state() {
    let now = Instant::now();
    let mut stats = Statistics::new(now);
    let a = event(1, 0, Direction::Rx, b"ONE\r");
    stats.observe(&a, now, LineDelimiter::Auto);
    stats.observe(
        &event(2, 100, Direction::Tx, b"TX"),
        now + Duration::from_millis(100),
        LineDelimiter::Auto,
    );
    stats.observe(
        &event(3, 200, Direction::Rx, b"\nTWO\r\nPART"),
        now + Duration::from_millis(200),
        LineDelimiter::Auto,
    );
    assert_eq!(stats.rx_lines, 2);
    assert_eq!(stats.longest_rx_gap, Some(Duration::from_millis(200)));
    assert_eq!(stats.rates(now + Duration::from_millis(200)), (14, 2));
    assert_eq!(stats.rates(now + Duration::from_secs(2)), (0, 0));
    for i in 0..10000 {
        stats.observe(
            &event(10 + i * 2, 300 + i * 10, Direction::Tx, b"Q"),
            now + Duration::from_millis(300 + i * 10),
            LineDelimiter::Auto,
        );
        stats.observe(
            &event(11 + i * 2, 301 + i * 10, Direction::Rx, b"R"),
            now + Duration::from_millis(301 + i * 10),
            LineDelimiter::Auto,
        );
    }
    assert_eq!(stats.latencies.len(), 2000);
    assert_eq!(stats.latency_count, 10001);
    stats.reset(now + Duration::from_secs(200));
    assert_eq!((stats.rx_bytes, stats.tx_bytes, stats.rx_lines), (0, 0, 0));
    assert!(stats.latencies.is_empty() && stats.last_latency.is_none());
    assert_eq!(stats.rates(now + Duration::from_secs(200)), (0, 0));
    assert_eq!(a.bytes.as_ref(), b"ONE\r");
}
#[test]
fn line_counts_respect_delimiter_and_tx_does_not_break_fragmented_crlf() {
    let now = Instant::now();
    for delimiter in [LineDelimiter::Auto, LineDelimiter::CrLf] {
        let mut stats = Statistics::new(now);
        stats.observe(&event(1, 0, Direction::Rx, b"\r"), now, delimiter);
        stats.observe(&event(2, 1, Direction::Tx, b"\n"), now, delimiter);
        stats.observe(&event(3, 2, Direction::Rx, b"\nA\r\nB\r"), now, delimiter);
        assert_eq!(
            stats.rx_lines,
            if delimiter == LineDelimiter::Auto {
                3
            } else {
                2
            }
        );
    }
}

#[test]
fn statistics_reset_preserves_only_line_assembly_state_across_split_crlf() {
    let now = Instant::now();
    for delimiter in [LineDelimiter::Auto, LineDelimiter::CrLf] {
        let mut stats = Statistics::new(now);
        stats.observe(&event(1, 0, Direction::Rx, b"DONE\r"), now, delimiter);
        stats.reset(now);
        stats.observe(&event(2, 1, Direction::Rx, b"\n"), now, delimiter);
        assert_eq!(
            stats.rx_lines,
            if delimiter == LineDelimiter::Auto {
                0
            } else {
                1
            }
        );
        assert_eq!(stats.rx_bytes, 1);
        assert!(stats.longest_rx_gap.is_none());
    }
}
