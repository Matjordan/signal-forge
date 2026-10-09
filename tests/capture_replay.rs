#![cfg(target_os = "linux")]
use signal_forge::{
    endpoint::{ConnectionState, Endpoint, EndpointId},
    replay::{ReplayConfig, ReplayEndpoint, ReplayFormat, ReplayState, ReplayStream},
    traffic::{Direction, TrafficBus, TrafficEvent},
    traffic_analysis::{Pattern, PatternMode},
    triggered_capture::{CaptureOptions, RollingCapture, Trigger, TriggerState, TriggeredCapture},
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};
fn event(seq: u64, ms: u64, direction: Direction, bytes: &[u8]) -> Arc<TrafficEvent> {
    Arc::new(TrafficEvent {
        sequence: seq,
        timestamp: UNIX_EPOCH + Duration::from_millis(ms),
        endpoint: EndpointId("test".into()),
        direction,
        bytes: Arc::from(bytes),
    })
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "Timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn ready(endpoint: &ReplayEndpoint) {
    wait(|| endpoint.state() != ConnectionState::Connecting);
    assert_eq!(
        endpoint.state(),
        ConnectionState::Connected,
        "{:?}",
        endpoint.controller().status()
    );
}
fn capture_source() -> String {
    [serde_json::json!({"type":"header","format":"signal-forge-capture","version":1}),serde_json::json!({"type":"event","sequence":1,"timestamp_unix_ns":"1000000000","direction":"a_to_b","raw_bytes":[0,255,13,10]}),serde_json::json!({"type":"event","sequence":2,"timestamp_unix_ns":"1300000000","direction":"b_to_a","raw_bytes":[65,66]}),serde_json::json!({"type":"footer","events":2,"bytes":6,"dropped_events":0,"complete":true})].into_iter().map(|v|format!("{v}\n")).collect()
}
#[test]
fn rolling_limits_precede_trigger_and_post_bytes_clip_exactly_without_mutating_source() {
    let now = Instant::now();
    for (mode, value) in [
        (PatternMode::Text, "READY"),
        (PatternMode::Hex, "52 45 41 44 59"),
        (PatternMode::Regex, "REA.*DY"),
    ] {
        let mut rolling = RollingCapture::new(CaptureOptions {
            trigger: Trigger::Pattern(Pattern {
                mode,
                value: value.into(),
            }),
            pre_bytes: 8,
            pre_duration: Duration::ZERO,
            post_bytes: 3,
            post_duration: Duration::ZERO,
        })
        .unwrap();
        assert!(rolling
            .push(event(1, 0, Direction::Rx, b"oldold"), now)
            .is_empty());
        assert!(rolling
            .push(event(2, 1, Direction::Rx, b"REA"), now)
            .is_empty());
        assert_eq!(rolling.buffered(), (3, 1));
        let trigger = event(3, 2, Direction::Rx, b"DY\0\xff");
        let captured = rolling.push(trigger.clone(), now);
        assert_eq!(
            captured.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(captured[0].bytes.as_ref(), b"REA");
        assert_eq!(captured[1].bytes.as_ref(), b"DY\0\xff");
        let post = event(4, 3, Direction::Tx, b"012345");
        let tail = rolling.push(post.clone(), now);
        assert_eq!(tail[0].bytes.as_ref(), b"012");
        assert_eq!(tail[0].timestamp, post.timestamp);
        assert_eq!(post.bytes.as_ref(), b"012345");
        assert!(rolling.done());
        assert!(rolling
            .push(event(5, 4, Direction::Rx, b"late"), now)
            .is_empty());
    }
}
#[test]
fn manual_idle_time_and_metadata_bounds_work_without_incoming_traffic() {
    let now = Instant::now();
    let mut rolling = RollingCapture::new(CaptureOptions {
        pre_bytes: 16,
        pre_duration: Duration::from_millis(10),
        post_bytes: 0,
        post_duration: Duration::from_millis(20),
        ..Default::default()
    })
    .unwrap();
    rolling.push(event(1, 0, Direction::Rx, b"before"), now);
    rolling.tick(now + Duration::from_millis(11));
    assert_eq!(rolling.buffered(), (0, 0));
    rolling.push(
        event(2, 12, Direction::Rx, b"recent"),
        now + Duration::from_millis(12),
    );
    assert_eq!(
        rolling.manual(now + Duration::from_millis(13))[0].sequence,
        2
    );
    rolling.tick(now + Duration::from_millis(34));
    assert!(rolling.done());
    let mut rolling = RollingCapture::new(CaptureOptions {
        trigger: Trigger::RxIdle(Duration::from_millis(100)),
        pre_bytes: 16,
        post_bytes: 0,
        post_duration: Duration::ZERO,
        ..Default::default()
    })
    .unwrap();
    rolling.push(event(1, 0, Direction::Rx, b"first"), now);
    rolling.push(
        event(2, 1000, Direction::Tx, b"does not reset RX idle"),
        now,
    );
    let output = rolling.push(event(3, 100, Direction::Rx, b"after idle"), now);
    assert_eq!(output.last().unwrap().sequence, 3);
    assert!(rolling.done());
    let mut rolling = RollingCapture::new(CaptureOptions {
        pre_bytes: 16,
        pre_duration: Duration::ZERO,
        ..Default::default()
    })
    .unwrap();
    for seq in 1..10000 {
        rolling.push(event(seq, 0, Direction::Rx, b""), now);
    }
    assert_eq!(rolling.buffered(), (0, 2048));
    assert!(RollingCapture::new(CaptureOptions {
        pre_bytes: 16 * 1024 * 1024 + 1,
        ..Default::default()
    })
    .is_err());
}
#[test]
fn triggered_worker_records_only_selected_endpoint_then_can_be_replayed() {
    let bus = TrafficBus::default();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trigger.jsonl");
    let mut capture = TriggeredCapture::start(
        &path,
        EndpointId("test".into()),
        &bus,
        CaptureOptions {
            trigger: Trigger::Pattern(Pattern {
                mode: PatternMode::Hex,
                value: "00 FF".into(),
            }),
            post_bytes: 2,
            post_duration: Duration::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    for _ in 0..100 {
        bus.publish(EndpointId("foreign".into()), Direction::Rx, b"ignore");
    }
    bus.publish(EndpointId("test".into()), Direction::Tx, b"QUERY");
    bus.publish(EndpointId("test".into()), Direction::Rx, b"\0\xff");
    bus.publish(EndpointId("test".into()), Direction::Rx, b"XYlate");
    wait(|| capture.status().state == TriggerState::Completed);
    capture.finish();
    let records: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records[0]["format"], "signal-forge-terminal-capture");
    assert_eq!(records.len(), 5);
    assert_eq!(
        records[1]["raw_bytes"],
        serde_json::json!([81, 85, 69, 82, 89])
    );
    assert_eq!(records[2]["raw_bytes"], serde_json::json!([0, 255]));
    assert_eq!(records[3]["raw_bytes"], serde_json::json!([88, 89]));
    assert_eq!(records[4]["complete"], true);
    assert_eq!(capture.status().dropped, 0);
    assert!(
        TriggeredCapture::start(&path, EndpointId("test".into()), &bus, Default::default())
            .is_err()
    );
    let output = bus.subscribe_endpoint(
        16,
        EndpointId(format!(
            "serial:{}",
            ReplayConfig {
                path: path.to_str().unwrap().into(),
                format: ReplayFormat::Capture,
                ..Default::default()
            }
            .uri()
        )),
    );
    let replay = ReplayEndpoint::open(
        ReplayConfig {
            path: path.to_str().unwrap().into(),
            format: ReplayFormat::Capture,
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    ready(&replay);
    let control = replay.controller();
    control.set_fast(true);
    control.play();
    wait(|| control.status().state == ReplayState::Completed);
    assert_eq!(
        output
            .receiver
            .try_iter()
            .flat_map(|event| event.bytes.to_vec())
            .collect::<Vec<_>>(),
        b"\0\xffXY"
    );
}
#[test]
fn manual_trigger_without_post_data_finalizes_on_timer_and_untriggered_stop_is_cancelled() {
    let directory = tempfile::tempdir().unwrap();
    let bus = TrafficBus::default();
    let mut capture = TriggeredCapture::start(
        &directory.path().join("manual.jsonl"),
        EndpointId("test".into()),
        &bus,
        CaptureOptions {
            post_bytes: 0,
            post_duration: Duration::from_millis(20),
            ..Default::default()
        },
    )
    .unwrap();
    bus.publish(EndpointId("test".into()), Direction::Rx, b"before");
    wait(|| capture.status().buffered_bytes == 6);
    capture.trigger();
    wait(|| capture.status().state == TriggerState::Completed);
    assert_eq!(capture.status().bytes, 6);
    capture.finish();
    let mut cancelled = TriggeredCapture::start(
        &directory.path().join("cancelled.jsonl"),
        EndpointId("test".into()),
        &bus,
        Default::default(),
    )
    .unwrap();
    cancelled.finish();
    assert_eq!(cancelled.status().state, TriggerState::Cancelled);
}
#[test]
fn raw_and_hex_replay_are_paused_read_only_and_exact_with_reset_step_and_fast_play() {
    let directory = tempfile::tempdir().unwrap();
    for (format, data) in [
        (ReplayFormat::Raw, b"\0\xff\r\nABC".as_slice()),
        (ReplayFormat::Hex, b"00 ff 0D\n0A 41 42 43".as_slice()),
    ] {
        let path = directory.path().join(format!("{format:?}"));
        std::fs::write(&path, data).unwrap();
        let bus = TrafficBus::default();
        let events = bus.subscribe(16);
        let mut replay = ReplayEndpoint::open(
            ReplayConfig {
                path: path.to_str().unwrap().into(),
                format,
                chunk: 2,
                delay_ms: 30,
                ..Default::default()
            },
            bus,
        )
        .unwrap();
        ready(&replay);
        let control = replay.controller();
        assert_eq!(control.status().state, ReplayState::Paused);
        assert!(events.try_recv().is_err());
        assert!(replay.read_only());
        assert!(replay.send(vec![1]).is_err());
        assert!(replay.send_file(&path).is_err());
        control.step();
        wait(|| control.status().events == 1);
        assert_eq!(
            events
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .bytes
                .as_ref(),
            b"\0\xff"
        );
        assert_eq!(control.status().state, ReplayState::Paused);
        control.play();
        let second = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(second.bytes.as_ref(), b"\r\n");
        control.pause();
        thread::sleep(Duration::from_millis(10));
        assert_eq!(control.status().state, ReplayState::Paused);
        control.reset();
        assert_eq!(control.status().bytes, 0);
        control.set_fast(true);
        control.play();
        wait(|| control.status().state == ReplayState::Completed);
        let bytes: Vec<_> = events
            .try_iter()
            .flat_map(|event| event.bytes.to_vec())
            .collect();
        assert_eq!(bytes, b"\0\xff\r\nABC");
        assert_eq!(std::fs::read(&path).unwrap(), data);
        replay.disconnect();
        assert_eq!(replay.state(), ConnectionState::Disconnected);
    }
}
#[test]
fn capture_streams_timestamps_original_pacing_and_pause_are_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capture.jsonl");
    std::fs::write(&path, capture_source()).unwrap();
    let bus = TrafficBus::default();
    let events = bus.subscribe(16);
    let replay = ReplayEndpoint::open(
        ReplayConfig {
            path: path.to_str().unwrap().into(),
            format: ReplayFormat::Capture,
            stream: ReplayStream::Both,
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    ready(&replay);
    let control = replay.controller();
    control.play();
    let first = events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(first.timestamp, UNIX_EPOCH + Duration::from_secs(1));
    assert_eq!(first.direction, Direction::Rx);
    assert!(events.recv_timeout(Duration::from_millis(30)).is_err());
    control.pause();
    thread::sleep(Duration::from_millis(320));
    assert!(events.try_recv().is_err());
    control.play();
    let before = Instant::now();
    let second = events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(before.elapsed() > Duration::from_millis(150));
    assert_eq!(second.timestamp, UNIX_EPOCH + Duration::from_millis(1300));
    assert_eq!(second.direction, Direction::Tx);
    assert_eq!(second.bytes.as_ref(), b"AB");
    wait(|| control.status().state == ReplayState::Completed);
    for (stream, expected) in [
        (ReplayStream::Rx, b"\0\xff\r\n".as_slice()),
        (ReplayStream::Tx, b"AB".as_slice()),
    ] {
        let bus = TrafficBus::default();
        let events = bus.subscribe(16);
        let endpoint = ReplayEndpoint::open(
            ReplayConfig {
                path: path.to_str().unwrap().into(),
                format: ReplayFormat::Capture,
                stream,
                ..Default::default()
            },
            bus,
        )
        .unwrap();
        ready(&endpoint);
        endpoint.controller().set_fast(true);
        endpoint.controller().play();
        wait(|| endpoint.controller().status().state == ReplayState::Completed);
        assert_eq!(
            events
                .try_iter()
                .flat_map(|e| e.bytes.to_vec())
                .collect::<Vec<_>>(),
            expected
        );
    }
}
#[test]
fn invalid_sources_fail_before_any_playback_and_unfinalized_captures_warn() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source");
    for (format, data) in [
        (ReplayFormat::Hex, "AA BB Z".to_string()),
        (
            ReplayFormat::Capture,
            capture_source().replace("\"version\":1", "\"version\":99"),
        ),
        (
            ReplayFormat::Capture,
            capture_source().replace("\"bytes\":6", "\"bytes\":7"),
        ),
        (ReplayFormat::Capture, capture_source() + "{broken"),
    ] {
        std::fs::write(&path, data).unwrap();
        let bus = TrafficBus::default();
        let events = bus.subscribe(16);
        let endpoint = ReplayEndpoint::open(
            ReplayConfig {
                path: path.to_str().unwrap().into(),
                format,
                ..Default::default()
            },
            bus,
        )
        .unwrap();
        wait(|| endpoint.state() != ConnectionState::Connecting);
        assert!(matches!(endpoint.state(), ConnectionState::Fault(_)));
        endpoint.controller().play();
        assert!(events.try_recv().is_err());
    }
    let text = capture_source();
    let unfinalized = text.lines().take(3).collect::<Vec<_>>().join("\n");
    std::fs::write(&path, unfinalized).unwrap();
    let endpoint = ReplayEndpoint::open(
        ReplayConfig {
            path: path.to_str().unwrap().into(),
            format: ReplayFormat::Capture,
            ..Default::default()
        },
        TrafficBus::default(),
    )
    .unwrap();
    ready(&endpoint);
    assert!(endpoint
        .controller()
        .status()
        .warning
        .unwrap()
        .contains("footer"));
}

fn serial_peer(
    bus: TrafficBus,
) -> (
    std::fs::File,
    signal_forge::serial::SerialEndpoint,
    std::os::fd::OwnedFd,
) {
    use std::os::fd::AsRawFd;
    let pair = nix::pty::openpty(None, None).unwrap();
    let path = nix::unistd::ttyname(&pair.slave)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    nix::fcntl::fcntl(
        pair.master.as_raw_fd(),
        nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
    )
    .unwrap();
    let endpoint = signal_forge::serial::SerialEndpoint::open(
        &signal_forge::config::SerialSettings {
            path,
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    (std::fs::File::from(pair.master), endpoint, pair.slave)
}
#[test]
fn replay_bridge_outputs_exact_binary_ignores_replies_and_reset_cancels_backpressure() {
    use signal_forge::bridge::{Bridge, BridgeState};
    use std::io::{Read, Write};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("binary");
    let original: Vec<u8> = (0..100000).map(|i| (i % 256) as u8).collect();
    std::fs::write(&path, &original).unwrap();
    let bus = TrafficBus::default();
    let (mut master, serial, _slave) = serial_peer(bus.clone());
    let mut replay = ReplayEndpoint::open(
        ReplayConfig {
            path: path.to_str().unwrap().into(),
            chunk: 4096,
            delay_ms: 0,
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    ready(&replay);
    let mut bridge =
        Bridge::start(replay.bridge_port().unwrap(), serial.bridge_port().unwrap()).unwrap();
    let control = replay.controller();
    control.set_fast(true);
    control.play();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut buffer = [0; 65536];
    while output.len() < original.len() {
        assert!(Instant::now() < deadline);
        match master.read(&mut buffer) {
            Ok(n) => output.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(output, original);
    wait(|| control.status().state == ReplayState::Completed);
    master
        .write_all(b"device reply must not enter recording")
        .unwrap();
    thread::sleep(Duration::from_millis(30));
    assert_eq!(bridge.state(), BridgeState::Running);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    bridge.stop();
    control.reset();
    let _bridge =
        Bridge::start(replay.bridge_port().unwrap(), serial.bridge_port().unwrap()).unwrap();
    control.play();
    // Do not drain the peer: the replay worker must be cancellable during a write.
    thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    control.reset();
    wait(|| control.status().state == ReplayState::Paused);
    replay.disconnect();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(std::fs::read(path).unwrap(), original);
}
#[test]
fn replay_factory_workspace_roundtrip_and_source_to_source_bridge_rejection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty");
    std::fs::write(&path, b"").unwrap();
    let config = ReplayConfig {
        path: path.to_str().unwrap().into(),
        ..Default::default()
    };
    let uri = config.uri();
    assert_eq!(ReplayConfig::parse(&uri).unwrap().path, config.path);
    let settings = signal_forge::config::SerialSettings {
        path: uri,
        ..Default::default()
    };
    settings.validate().unwrap();
    let mut endpoint = signal_forge::endpoint::open(&settings, TrafficBus::default()).unwrap();
    wait(|| endpoint.state() != ConnectionState::Connecting);
    assert!(endpoint.read_only());
    assert_eq!(
        endpoint.playback().unwrap().status().state,
        ReplayState::Paused
    );
    let second =
        ReplayEndpoint::open(ReplayConfig { chunk: 1, ..config }, TrafficBus::default()).unwrap();
    ready(&second);
    assert!(signal_forge::bridge::Bridge::start(
        endpoint.bridge_port().unwrap(),
        second.bridge_port().unwrap()
    )
    .is_err());
    endpoint.playback().unwrap().play();
    wait(|| endpoint.playback().unwrap().status().state == ReplayState::Completed);
    endpoint.disconnect();
    let saved = signal_forge::workspace::SavedTerminal {
        settings,
        ..Default::default()
    };
    let json = serde_json::to_string(&saved).unwrap();
    let roundtrip: signal_forge::workspace::SavedTerminal = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtrip.settings.path, saved.settings.path);
    assert!(!json.contains("Playing"));
}
#[test]
fn backwards_timestamps_keep_original_values_and_do_not_stall_playback() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("backwards.jsonl");
    std::fs::write(&path, capture_source().replace("1300000000", "500000000")).unwrap();
    let bus = TrafficBus::default();
    let events = bus.subscribe(16);
    let endpoint = ReplayEndpoint::open(
        ReplayConfig {
            path: path.to_str().unwrap().into(),
            format: ReplayFormat::Capture,
            stream: ReplayStream::Both,
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    ready(&endpoint);
    endpoint.controller().play();
    wait(|| endpoint.controller().status().state == ReplayState::Completed);
    let records: Vec<_> = events.try_iter().collect();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].timestamp,
        UNIX_EPOCH + Duration::from_millis(500)
    );
    assert_eq!(records[1].bytes.as_ref(), b"AB");
}
