#![cfg(target_os = "linux")]
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    unistd::ttyname,
};
use signal_forge::{
    config::SerialSettings,
    endpoint::{ConnectionState, Endpoint, EndpointId},
    file_transfer::{FileMode, FileTransferHandle, TransferState},
    raw_recording::{RawRecording, RecordingState},
    serial::SerialEndpoint,
    traffic::{Direction, TrafficBus},
};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsRawFd,
    thread,
    time::{Duration, Instant},
};

fn endpoint(bus: TrafficBus) -> (File, SerialEndpoint, std::os::fd::OwnedFd) {
    let pair = openpty(None, None).unwrap();
    let path = ttyname(&pair.slave).unwrap().to_string_lossy().into_owned();
    fcntl(
        pair.master.as_raw_fd(),
        FcntlArg::F_SETFL(OFlag::O_NONBLOCK),
    )
    .unwrap();
    let endpoint = SerialEndpoint::open(
        &SerialSettings {
            path,
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    (File::from(pair.master), endpoint, pair.slave)
}
fn received(file: &mut File, count: usize) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut output = Vec::new();
    let mut buffer = [0; 65536];
    while output.len() < count && Instant::now() < deadline {
        match file.read(&mut buffer) {
            Ok(n) => output.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(output.len(), count);
    output
}
fn finished(handle: &FileTransferHandle) -> signal_forge::file_transfer::TransferStatus {
    let deadline = Instant::now() + Duration::from_secs(6);
    while handle.is_active() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!handle.is_active());
    handle.status()
}

#[test]
fn file_modes_preserve_bytes_text_and_hex_and_publish_tx() {
    let bus = TrafficBus::default();
    let events = bus.subscribe(4096);
    let (mut master, endpoint, _slave) = endpoint(bus);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    for (mode, source, expected) in [
        (
            FileMode::Raw,
            b"Hello\r\n\0\xff".as_slice(),
            b"Hello\r\n\0\xff".as_slice(),
        ),
        (
            FileMode::Ascii,
            "λ\r\nTEXT\\x41\n".as_bytes(),
            "λ\r\nTEXT\\x41\n".as_bytes(),
        ),
        (
            FileMode::Hex,
            b"48 65\t6c 6C\n6f 0D 0A 00 ff",
            b"Hello\r\n\0\xff",
        ),
    ] {
        std::fs::write(&path, source).unwrap();
        let handle = endpoint
            .send_file_mode(&path, mode, 3, Duration::ZERO)
            .unwrap();
        assert_eq!(received(&mut master, expected.len()), expected);
        let status = finished(&handle);
        assert_eq!(status.state, TransferState::Completed);
        assert_eq!(
            (status.sent, status.total),
            (expected.len() as u64, expected.len() as u64)
        );
        let mut tx = Vec::new();
        while tx.len() < expected.len() {
            let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
            if event.direction == Direction::Tx {
                tx.extend_from_slice(&event.bytes);
            }
        }
        assert_eq!(tx, expected);
    }
}

#[test]
fn late_invalid_hex_and_invalid_utf8_send_nothing_and_leave_endpoint_usable() {
    let (mut master, endpoint, _slave) = endpoint(TrafficBus::default());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    for (mode, bytes) in [
        (
            FileMode::Hex,
            [b"AA ".repeat(50000), b"ZZ".to_vec()].concat(),
        ),
        (FileMode::Hex, b"AA B".to_vec()),
        (FileMode::Ascii, b"valid\xff".to_vec()),
        (FileMode::Ascii, b"valid\xc3".to_vec()),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let handle = endpoint
            .send_file_mode(&path, mode, 4096, Duration::ZERO)
            .unwrap();
        let status = finished(&handle);
        assert!(matches!(status.state, TransferState::Failed(_)));
        assert_eq!(status.sent, 0);
        let mut buffer = [0; 1];
        assert_eq!(
            master.read(&mut buffer).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(endpoint.state(), ConnectionState::Connected);
    }
    endpoint.send(vec![42]).unwrap();
    assert_eq!(received(&mut master, 1), [42]);
}

#[test]
fn cancellation_throttling_rx_and_disconnect_have_exact_progress() {
    let bus = TrafficBus::default();
    let events = bus.subscribe(4096);
    let (mut master, mut endpoint, _slave) = endpoint(bus);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large");
    std::fs::write(&path, vec![0xff; 100000]).unwrap();
    let handle = endpoint
        .send_file_mode(&path, FileMode::Raw, 16, Duration::from_millis(100))
        .unwrap();
    assert_eq!(received(&mut master, 16), vec![0xff; 16]);
    master.write_all(b"RX while sending").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut rx = Vec::new();
    while rx.len() < 16 && Instant::now() < deadline {
        if let Ok(event) = events.recv_timeout(Duration::from_millis(10)) {
            if event.direction == Direction::Rx {
                rx.extend_from_slice(&event.bytes);
            }
        }
    }
    assert_eq!(rx, b"RX while sending");
    handle.cancel();
    let status = finished(&handle);
    assert_eq!(status.state, TransferState::Cancelled);
    assert!(status.sent < status.total);
    assert_eq!(endpoint.state(), ConnectionState::Connected);
    // Collect any full chunk already accepted before cancellation, then check the counter.
    let mut accepted = 16u64;
    let mut buffer = [0; 4096];
    while let Ok(n) = master.read(&mut buffer) {
        if n == 0 {
            break;
        }
        accepted += n as u64;
    }
    assert_eq!(status.sent, accepted);
    endpoint.send(vec![42]).unwrap();
    assert_eq!(received(&mut master, 1), [42]);
    let handle = endpoint
        .send_file_mode(&path, FileMode::Raw, 16, Duration::from_millis(100))
        .unwrap();
    received(&mut master, 16);
    endpoint.disconnect();
    assert_eq!(finished(&handle).state, TransferState::Cancelled);
    assert_eq!(endpoint.state(), ConnectionState::Disconnected);
}

#[test]
fn utf8_and_hex_validation_cross_block_boundaries_without_loading_ui_memory() {
    let (mut master, endpoint, _slave) = endpoint(TrafficBus::default());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut text = vec![b'A'; 65535];
    text.extend_from_slice("λ\r\n".as_bytes());
    for (mode, source, expected) in [
        (FileMode::Ascii, text.clone(), text),
        (
            FileMode::Hex,
            [vec![b' '; 65535], b"AABB\n00FF".to_vec()].concat(),
            vec![0xaa, 0xbb, 0, 255],
        ),
    ] {
        std::fs::write(&path, source).unwrap();
        let handle = endpoint
            .send_file_mode(&path, mode, 4096, Duration::ZERO)
            .unwrap();
        assert_eq!(received(&mut master, expected.len()), expected);
        assert_eq!(finished(&handle).state, TransferState::Completed);
    }
}

#[test]
fn recording_modes_are_rx_only_preserve_endings_and_hex_can_be_replayed() {
    let bus = TrafficBus::default();
    let id = EndpointId("selected".into());
    let directory = tempfile::tempdir().unwrap();
    let payload = b"Hello\r\n\0\xff\t";
    for (mode, expected) in [
        (FileMode::Raw, payload.to_vec()),
        (FileMode::Ascii, b"Hello\r\n\\x00\\xFF\t".to_vec()),
        (FileMode::Hex, b"48 65 6C 6C 6F 0D 0A 00 FF 09 \n".to_vec()),
    ] {
        let path = directory.path().join(format!("{mode:?}"));
        let mut recording = RawRecording::start_mode(&path, id.clone(), &bus, mode).unwrap();
        bus.publish(id.clone(), Direction::Rx, &payload[..3]);
        bus.publish(EndpointId("other".into()), Direction::Rx, b"excluded");
        bus.publish(id.clone(), Direction::Tx, b"excluded");
        bus.publish(id.clone(), Direction::Rx, &payload[3..]);
        recording.stop();
        bus.publish(id.clone(), Direction::Rx, b"after stop");
        recording.finish();
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        let status = recording.progress();
        assert_eq!(status.state, RecordingState::Completed);
        assert_eq!(status.received, payload.len() as u64);
        assert_eq!(status.written, expected.len() as u64);
        assert!(RawRecording::start(&path, id.clone(), &bus).is_err());
        if mode == FileMode::Hex {
            let (mut master, endpoint, _slave) = endpoint(bus.clone());
            let handle = endpoint
                .send_file_mode(&path, FileMode::Hex, 4096, Duration::ZERO)
                .unwrap();
            assert_eq!(received(&mut master, payload.len()), payload);
            assert_eq!(finished(&handle).state, TransferState::Completed);
        }
    }
}

#[test]
fn recording_overload_is_explicit_and_foreign_traffic_cannot_fill_its_queue() {
    let bus = TrafficBus::default();
    let id = EndpointId("selected".into());
    let subscription = bus.subscribe_endpoint_rx(1, id.clone());
    for _ in 0..100 {
        bus.publish(EndpointId("other".into()), Direction::Rx, b"foreign");
        bus.publish(id.clone(), Direction::Tx, b"tx");
    }
    assert_eq!(subscription.dropped_events(), 0);
    bus.publish(id.clone(), Direction::Rx, b"first");
    bus.publish(id.clone(), Direction::Rx, b"dropped");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recording");
    let mut recording =
        RawRecording::start_subscription(&path, id, FileMode::Raw, subscription).unwrap();
    recording.finish();
    assert_eq!(std::fs::read(path).unwrap(), b"first");
    assert_eq!(recording.progress().state, RecordingState::Incomplete);
    assert_eq!(recording.progress().dropped, 1);
}
