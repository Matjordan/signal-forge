#![cfg(target_os = "linux")]
use signal_forge::{
    config::{Parity, SerialSettings},
    endpoint::Endpoint,
    serial::SerialEndpoint,
    terminal_display::SerialFraming,
    traffic::{Direction, TrafficBus},
    virtual_pair::{LinkTiming, PairState, VirtualPair},
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    thread,
    time::{Duration, Instant},
};
fn client(path: &str) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOCTTY | nix::libc::O_NONBLOCK)
        .open(path)
        .unwrap()
}
fn frame(baud: u32, parity: Parity, stop_bits: u8) -> SerialFraming {
    SerialFraming {
        baud,
        data_bits: 8,
        parity,
        stop_bits,
    }
}
fn receive(mut file: File, payload: &[u8]) -> (Duration, Duration, usize) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(8);
    let mut output = Vec::new();
    let mut first = None;
    let mut chunks = 0;
    let mut buffer = [0; 4096];
    while output.len() < payload.len() {
        assert!(
            Instant::now() < deadline,
            "stalled {} of {}",
            output.len(),
            payload.len()
        );
        match file.read(&mut buffer) {
            Ok(0) => panic!("closed"),
            Ok(count) => {
                first.get_or_insert(start.elapsed());
                chunks += 1;
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_micros(200))
            }
            Err(error) => panic!("{error}"),
        }
    }
    assert_eq!(output, payload);
    (first.unwrap(), start.elapsed(), chunks)
}
#[test]
fn paced_delivery_is_progressive_binary_exact_and_uses_framing() {
    for framing in [frame(19200, Parity::None, 1), frame(19200, Parity::Even, 2)] {
        let pair =
            VirtualPair::create_with_timing("paced", None, LinkTiming::Emulated(framing)).unwrap();
        let mut a = client(&pair.paths[0]);
        let b = client(&pair.paths[1]);
        let payload: Vec<u8> = (0..768).map(|i| (i % 256) as u8).collect();
        let start = Instant::now();
        a.write_all(&payload).unwrap();
        let (first, _last, chunks) = receive(b, &payload);
        let total = start.elapsed();
        let expected = framing.wire_seconds(payload.len() as u64).unwrap();
        assert!(
            first.as_secs_f64() < expected * 0.4,
            "first {first:?} expected {expected}"
        );
        assert!(
            total.as_secs_f64() > expected * 0.92,
            "total {total:?} expected {expected}"
        );
        assert!(
            total.as_secs_f64() < expected * 1.5 + 0.05,
            "total {total:?} expected {expected}"
        );
        assert!(chunks > 20, "only {chunks} delivery chunks");
    }
}
#[test]
fn duplex_directions_transmit_concurrently_and_long_transfer_does_not_drift() {
    let framing = frame(19200, Parity::None, 1);
    let pair =
        VirtualPair::create_with_timing("duplex", None, LinkTiming::Emulated(framing)).unwrap();
    let mut a = client(&pair.paths[0]);
    let mut b = client(&pair.paths[1]);
    let forward: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
    let reverse: Vec<u8> = forward.iter().map(|byte| !byte).collect();
    let start = Instant::now();
    a.write_all(&forward).unwrap();
    b.write_all(&reverse).unwrap();
    let reader_a = thread::spawn(move || receive(a, &reverse));
    let reader_b = thread::spawn(move || receive(b, &forward));
    reader_a.join().unwrap();
    reader_b.join().unwrap();
    let expected = framing.wire_seconds(4096).unwrap();
    assert!(start.elapsed().as_secs_f64() > expected * 0.8);
    assert!(
        start.elapsed().as_secs_f64() < expected * 1.35 + 0.1,
        "elapsed {:?}, expected {expected}",
        start.elapsed()
    );
}
#[test]
fn unlimited_keeps_fast_delivery_and_queued_stop_is_prompt() {
    let pair = VirtualPair::create("unlimited", None).unwrap();
    let mut a = client(&pair.paths[0]);
    let b = client(&pair.paths[1]);
    let payload = vec![255; 4096];
    let start = Instant::now();
    a.write_all(&payload).unwrap();
    receive(b, &payload);
    assert!(start.elapsed() < Duration::from_millis(500));
    let mut pair = VirtualPair::create_with_timing(
        "slow",
        None,
        LinkTiming::Emulated(frame(300, Parity::None, 1)),
    )
    .unwrap();
    let mut a = client(&pair.paths[0]);
    a.write_all(&[42; 4096]).unwrap();
    thread::sleep(Duration::from_millis(20));
    let start = Instant::now();
    pair.stop();
    assert!(start.elapsed() < Duration::from_millis(100));
    assert_eq!(pair.state(), PairState::Stopped);
    assert!(pair
        .raw_paths
        .iter()
        .all(|path| !std::path::Path::new(path).exists()));
}
#[test]
fn diagnostics_report_exclusive_client_and_peer_works_without_privilege() {
    assert_ne!(
        unsafe { nix::libc::geteuid() },
        0,
        "Run compatibility checks without sudo"
    );
    let directory = tempfile::tempdir().unwrap();
    let pair = VirtualPair::create("access", Some(directory.path())).unwrap();
    let bus = TrafficBus::default();
    let sub = bus.subscribe_tracked(64);
    let mut endpoint = SerialEndpoint::open(
        &SerialSettings {
            path: pair.paths[0].clone(),
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    let diagnostics = pair.diagnostics();
    assert!(
        diagnostics[0].access.contains("Cannot open"),
        "{:?}",
        diagnostics[0]
    );
    assert!(diagnostics[1].access.contains("Can open"));
    assert!(diagnostics[0].permissions.contains("uid "));
    let mut peer = client(&pair.paths[1]);
    peer.write_all(&[0, 255, 13, 10]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    while bytes.len() < 4 {
        assert!(Instant::now() < deadline);
        let event = sub.receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        if event.direction == Direction::Rx {
            bytes.extend_from_slice(&event.bytes);
        }
    }
    assert_eq!(bytes, [0, 255, 13, 10]);
    endpoint.send(vec![128, 0, 255]).unwrap();
    receive(peer, &[128, 0, 255]);
    endpoint.disconnect();
    assert!(pair.diagnostics()[0].access.contains("Can open"));
}
#[test]
fn pair_survives_external_client_close_reopen_and_validates_timing() {
    let pair = VirtualPair::create("reopen", None).unwrap();
    for _ in 0..3 {
        let mut a = client(&pair.paths[0]);
        let b = client(&pair.paths[1]);
        a.write_all(b"exact\0\xff").unwrap();
        receive(b, b"exact\0\xff");
        drop(a);
        thread::sleep(Duration::from_millis(10));
        assert_eq!(pair.state(), PairState::Running);
    }
    for framing in [
        frame(0, Parity::None, 1),
        SerialFraming {
            data_bits: 255,
            stop_bits: 255,
            ..frame(19200, Parity::None, 1)
        },
    ] {
        assert!(
            VirtualPair::create_with_timing("invalid", None, LinkTiming::Emulated(framing))
                .is_err()
        );
    }
}

#[test]
fn stale_exclusive_flag_recovers_through_owned_descriptor_without_sudo() {
    use std::os::fd::AsRawFd;
    let pair = VirtualPair::create("stale", None).unwrap();
    let client = client(&pair.paths[0]);
    assert_eq!(
        unsafe { nix::libc::ioctl(client.as_raw_fd(), nix::libc::TIOCEXCL) },
        0
    );
    drop(client);
    assert!(pair.diagnostics()[0].access.contains("Cannot open"));
    pair.release_exclusive(0).unwrap();
    assert!(pair.diagnostics()[0].access.contains("Can open"));
    assert!(pair.release_exclusive(2).is_err());
}
