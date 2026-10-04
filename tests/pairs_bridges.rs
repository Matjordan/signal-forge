#![cfg(target_os = "linux")]
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    unistd::ttyname,
};
use signal_forge::{
    bridge::{Bridge, BridgeState},
    config::SerialSettings,
    endpoint::Endpoint,
    serial::SerialEndpoint,
    traffic::TrafficBus,
    virtual_pair::VirtualPair,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::fd::AsRawFd,
    thread,
    time::{Duration, Instant},
};
fn nonblocking(file: &File) {
    fcntl(file.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
}
fn client(path: &str) -> File {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    nonblocking(&file);
    file
}
fn put(file: &mut File, bytes: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut offset = 0;
    while offset < bytes.len() {
        assert!(
            Instant::now() < deadline,
            "write stalled after {offset} bytes"
        );
        match file.write(&bytes[offset..]) {
            Ok(0) => panic!("zero write"),
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => panic!("write: {e}"),
        }
    }
}
fn take(file: &mut File, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut result = Vec::new();
    let mut buffer = [0; 4096];
    while result.len() < expected.len() && Instant::now() < deadline {
        match file.read(&mut buffer) {
            Ok(0) => panic!("closed"),
            Ok(n) => result.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => panic!("read: {e}"),
        }
    }
    assert_eq!(result.len(), expected.len());
    assert_eq!(result, expected);
}
fn endpoint() -> (File, SerialEndpoint, std::os::fd::OwnedFd) {
    let pty = openpty(None, None).unwrap();
    let path = ttyname(&pty.slave).unwrap().to_string_lossy().into_owned();
    let endpoint = SerialEndpoint::open(
        &SerialSettings {
            path,
            ..Default::default()
        },
        TrafficBus::default(),
    )
    .unwrap();
    let master = File::from(pty.master);
    nonblocking(&master);
    (master, endpoint, pty.slave)
}
#[test]
fn owned_pairs_are_independent_binary_full_duplex_and_clean_up() {
    let mut first = VirtualPair::create("first", None).unwrap();
    let second = VirtualPair::create("second", None).unwrap();
    let mut a = client(&first.paths[0]);
    let mut b = client(&first.paths[1]);
    let mut c = client(&second.paths[0]);
    let mut d = client(&second.paths[1]);
    put(&mut a, b"hello\r\n\0\xff");
    put(&mut b, b"reverse\0\x80");
    put(&mut c, b"independent");
    take(&mut b, b"hello\r\n\0\xff");
    take(&mut a, b"reverse\0\x80");
    take(&mut d, b"independent");
    drop(a);
    drop(b);
    let paths = first.raw_paths.clone();
    first.stop();
    assert!(paths.iter().all(|p| !std::path::Path::new(p).exists()));
    put(&mut d, b"still alive");
    take(&mut c, b"still alive");
}
#[test]
fn pair_retains_partial_writes_under_backpressure() {
    let pair = VirtualPair::create("bulk", None).unwrap();
    let mut a = client(&pair.paths[0]);
    let mut b = client(&pair.paths[1]);
    let bytes: Vec<_> = (0..262144).map(|i| (i % 256) as u8).collect();
    let sent = bytes.clone();
    let sender = thread::spawn(move || put(&mut a, &sent));
    thread::sleep(Duration::from_millis(60));
    take(&mut b, &bytes);
    sender.join().unwrap();
}
#[test]
fn named_links_rollback_collisions_and_preserve_replacements() {
    let directory = std::env::temp_dir().join(format!("signal-forge-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("collision-b"), b"foreign").unwrap();
    assert!(VirtualPair::create("collision", Some(&directory)).is_err());
    assert!(!directory.join("collision-a").exists());
    assert_eq!(
        std::fs::read(directory.join("collision-b")).unwrap(),
        b"foreign"
    );
    let pair = VirtualPair::create("named", Some(&directory)).unwrap();
    let paths = pair.paths.clone();
    assert!(paths.iter().all(|p| std::fs::read_link(p).is_ok()));
    std::fs::remove_file(&paths[0]).unwrap();
    std::fs::write(&paths[0], b"replacement").unwrap();
    drop(pair);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"replacement");
    assert!(!std::path::Path::new(&paths[1]).exists());
    assert!(VirtualPair::create("bad/name", None).is_err());
    assert!(VirtualPair::create("no-directory", Some(&directory.join("collision-b"))).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn bridge_full_duplex_monitor_saturation_stop_and_disconnect() {
    let (mut ma, mut a, _sa) = endpoint();
    let (mut mb, b, _sb) = endpoint();
    assert!(Bridge::start(a.bridge_port().unwrap(), a.bridge_port().unwrap()).is_err());
    let mut bridge = Bridge::start(a.bridge_port().unwrap(), b.bridge_port().unwrap()).unwrap();
    assert!(Bridge::start(a.bridge_port().unwrap(), b.bridge_port().unwrap()).is_err());
    let events = bridge.subscribe(64);
    let blocked_monitor = bridge.subscribe(1);
    put(&mut ma, b"A\0\xff");
    put(&mut mb, b"B\r\n\x80");
    take(&mut mb, b"A\0\xff");
    take(&mut ma, b"B\r\n\x80");
    let first = events.recv_timeout(Duration::from_secs(2)).unwrap();
    let second = events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(first.endpoint, second.endpoint);
    assert!(first.sequence < second.sequence);
    for _ in 0..12 {
        put(&mut ma, b"monitor full");
        take(&mut mb, b"monitor full");
    }
    assert!(bridge.dropped_events() > 0);
    drop(blocked_monitor);
    bridge.stop();
    put(&mut ma, b"no forwarding");
    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        mb.read(&mut [0; 64]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    b.send(b"manual still works".to_vec()).unwrap();
    take(&mut mb, b"manual still works");
    let bridge = Bridge::start(a.bridge_port().unwrap(), b.bridge_port().unwrap()).unwrap();
    a.disconnect();
    assert!(matches!(bridge.state(), BridgeState::Fault(_)));
}
#[test]
fn serial_to_owned_pair_bridge_handles_binary_both_ways() {
    let pair = VirtualPair::create("serial-bridge", None).unwrap();
    let virtual_endpoint = SerialEndpoint::open(
        &SerialSettings {
            path: pair.paths[0].clone(),
            ..Default::default()
        },
        TrafficBus::default(),
    )
    .unwrap();
    let mut peer = client(&pair.paths[1]);
    let (mut master, serial, _slave) = endpoint();
    let bridge = Bridge::start(
        serial.bridge_port().unwrap(),
        virtual_endpoint.bridge_port().unwrap(),
    )
    .unwrap();
    put(&mut master, b"physical\0\xff\r\n");
    take(&mut peer, b"physical\0\xff\r\n");
    put(&mut peer, b"virtual\x80\0");
    take(&mut master, b"virtual\x80\0");
    drop(virtual_endpoint);
    assert!(matches!(bridge.state(), BridgeState::Fault(_)));
}

#[test]
fn bridge_large_concurrent_transfers_preserve_every_byte() {
    let (ma, a, _sa) = endpoint();
    let (mb, b, _sb) = endpoint();
    let _bridge = Bridge::start(a.bridge_port().unwrap(), b.bridge_port().unwrap()).unwrap();
    let forward: Vec<_> = (0..131072).map(|i| (i % 256) as u8).collect();
    let reverse: Vec<_> = (0..131072).map(|i| (255 - i % 256) as u8).collect();
    let mut send_a = ma.try_clone().unwrap();
    let mut send_b = mb.try_clone().unwrap();
    thread::scope(|scope| {
        scope.spawn(|| put(&mut send_a, &forward));
        scope.spawn(|| put(&mut send_b, &reverse));
        scope.spawn(|| take(&mut { ma }, &reverse));
        scope.spawn(|| take(&mut { mb }, &forward));
    });
}
