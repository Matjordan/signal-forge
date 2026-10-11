use signal_forge::{
    bridge::Bridge,
    config::SerialSettings,
    endpoint::{self, ConnectionState, Endpoint},
    network::NetworkEndpoint,
    repeat::RepeatSpec,
    traffic::{Direction, TrafficBus},
};
use signal_forge_network::{BridgeConfig, Config, Ending, Framing, Mode};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    thread,
    time::{Duration, Instant},
};
fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "network endpoint timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn endpoint(config: Config, bus: TrafficBus) -> NetworkEndpoint {
    let e = NetworkEndpoint::open(&config.uri(), bus).unwrap();
    wait(|| e.state() != ConnectionState::Connecting);
    assert!(
        matches!(
            e.state(),
            ConnectionState::Connected | ConnectionState::Listening
        ),
        "{:?}",
        e.state()
    );
    e
}
#[test]
fn tcp_repeat_file_and_payload_only_recording() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bus = TrafficBus::default();
    let mut e = endpoint(
        Config {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus.clone(),
    );
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let recorded = dir.path().join("rx.bin");
    let mut recording =
        signal_forge::raw_recording::RawRecording::start(&recorded, e.id().clone(), &bus).unwrap();
    peer.write_all(&[0, 255, 10]).unwrap();
    wait(|| recording.progress().written == 3);
    recording.finish();
    assert_eq!(std::fs::read(recorded).unwrap(), [0, 255, 10]);
    let repeat = e
        .start_repeat(
            vec![0, 255],
            RepeatSpec {
                interval: Duration::from_millis(5),
                count: Some(3),
            },
        )
        .unwrap();
    let mut bytes = [0; 6];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 255, 0, 255, 0, 255]);
    wait(|| !repeat.is_active());
    let path = dir.path().join("tx.bin");
    std::fs::write(&path, [7, 0, 255, 8]).unwrap();
    let file = e
        .send_file_mode(
            &path,
            signal_forge::file_transfer::FileMode::Raw,
            2,
            Duration::from_millis(2),
        )
        .unwrap();
    let mut bytes = [0; 4];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [7, 0, 255, 8]);
    wait(|| !file.is_active());
    e.disconnect();
}
#[test]
fn udp_events_empty_boundaries_capture_and_replay() {
    let bus = TrafficBus::default();
    let events = bus.subscribe(32);
    let mut e = endpoint(
        Config {
            mode: Mode::UdpListener,
            ..Default::default()
        },
        bus.clone(),
    );
    let address = e.network().unwrap().addresses().0.unwrap();
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("capture.jsonl");
    let mut capture = signal_forge::capture::Capture::start_subscription(
        &path,
        e.id().clone(),
        e.id().clone(),
        bus.subscribe_tracked(32),
    )
    .unwrap();
    peer.send_to(b"", address).unwrap();
    peer.send_to(b"A\nB\r\n", address).unwrap();
    let first = events.recv_timeout(Duration::from_secs(2)).unwrap();
    let second = events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(first.bytes.is_empty());
    assert_eq!(&*second.bytes, b"A\nB\r\n");
    assert_eq!(
        first.network.as_ref().unwrap().source,
        peer.local_addr().unwrap()
    );
    let mut display = signal_forge::terminal_display::LineDisplay::default();
    display.receive(&first);
    display.receive(&second);
    assert_eq!(display.len(), 2);
    assert_eq!(display.row(1).unwrap().bytes, b"A\nB\r\n");
    let mut statistics = signal_forge::traffic_analysis::Statistics::default();
    statistics.observe(
        &first,
        Instant::now(),
        signal_forge::terminal_display::LineDelimiter::Auto,
    );
    statistics.observe(
        &second,
        Instant::now(),
        signal_forge::terminal_display::LineDelimiter::Auto,
    );
    assert_eq!(statistics.rx_datagrams, 2);
    assert_eq!(statistics.rx_bytes, second.bytes.len() as u64);
    capture.finish();
    e.disconnect();
    let replay_bus = TrafficBus::default();
    let replay_events = replay_bus.subscribe(8);
    let mut replay = signal_forge::replay::ReplayEndpoint::open(
        signal_forge::replay::ReplayConfig {
            path: path.display().to_string(),
            format: signal_forge::replay::ReplayFormat::Capture,
            stream: signal_forge::replay::ReplayStream::Both,
            ..Default::default()
        },
        replay_bus,
    )
    .unwrap();
    wait(|| replay.state() != ConnectionState::Connecting);
    replay.controller().set_fast(true);
    replay.controller().play();
    let a = replay_events.recv_timeout(Duration::from_secs(2)).unwrap();
    let b = replay_events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(a.bytes.is_empty());
    assert_eq!(a.network, first.network);
    assert_eq!(b.network, second.network);
    assert_eq!(b.bytes, second.bytes);
    replay.disconnect();
}
#[test]
fn tcp_udp_bridge_frames_lines_and_appends_reply_ending() {
    let bus = TrafficBus::default();
    let udp_peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp_peer
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut tcp = endpoint(
        Config {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus.clone(),
    );
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut udp = endpoint(
        Config {
            mode: Mode::UdpClient,
            host: "127.0.0.1".into(),
            port: udp_peer.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus,
    );
    let mut bridge = Bridge::start_configured(
        tcp.bridge_port().unwrap(),
        udp.bridge_port().unwrap(),
        BridgeConfig {
            stream_to_udp: Framing::default(),
            udp_to_stream: Ending::CrLf,
        },
    )
    .unwrap();
    peer.write_all(b"CMD1\r").unwrap();
    thread::sleep(Duration::from_millis(20));
    peer.write_all(b"\nCMD2\r\n").unwrap();
    let mut bytes = [0; 64];
    let (n, address) = udp_peer.recv_from(&mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"CMD1\r\n");
    let n = udp_peer.recv(&mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"CMD2\r\n");
    udp_peer.send_to(&[0, 255], address).unwrap();
    let mut bytes = [0; 4];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 255, 13, 10]);
    bridge.stop();
    tcp.disconnect();
    udp.disconnect();
}
#[test]
fn listener_survives_disconnect_and_workspace_restores_configuration() {
    let config = Config {
        mode: Mode::TcpListener,
        ..Default::default()
    };
    let settings = SerialSettings {
        path: config.uri(),
        ..Default::default()
    };
    let mut e = endpoint::open(&settings, TrafficBus::default()).unwrap();
    wait(|| e.state() == ConnectionState::Listening);
    let address = e.network().unwrap().addresses().0.unwrap();
    for _ in 0..2 {
        let peer = TcpStream::connect(address).unwrap();
        wait(|| e.state() == ConnectionState::Connected);
        drop(peer);
        wait(|| e.state() == ConnectionState::Listening);
    }
    e.disconnect();
    let workspace = signal_forge::config::WorkspaceConfig {
        ports: vec![settings],
        ..Default::default()
    };
    let encoded = serde_json::to_string(&workspace).unwrap();
    let restored = signal_forge::config::WorkspaceConfig::parse(&encoded).unwrap();
    assert_eq!(Config::parse(&restored.ports[0].path).unwrap(), config);
}
#[test]
fn udp_file_chunks_and_repeat_are_datagrams() {
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let bus = TrafficBus::default();
    let events = bus.subscribe(16);
    let mut e = endpoint(
        Config {
            mode: Mode::UdpClient,
            host: "127.0.0.1".into(),
            port: peer.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus,
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tx.bin");
    std::fs::write(&path, b"abcde").unwrap();
    let h = e
        .send_file_mode(
            &path,
            signal_forge::file_transfer::FileMode::Raw,
            2,
            Duration::from_millis(1),
        )
        .unwrap();
    let mut buffer = [0; 8];
    for expected in [b"ab".as_slice(), b"cd", b"e"] {
        let n = peer.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..n], expected);
    }
    wait(|| !h.is_active());
    let repeat = e
        .start_repeat(
            vec![],
            RepeatSpec {
                interval: Duration::from_millis(2),
                count: Some(2),
            },
        )
        .unwrap();
    assert_eq!(peer.recv(&mut buffer).unwrap(), 0);
    assert_eq!(peer.recv(&mut buffer).unwrap(), 0);
    wait(|| !repeat.is_active());
    for _ in 0..5 {
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.direction, Direction::Tx);
        assert!(event.network.is_some());
    }
    e.disconnect();
}

#[test]
fn idle_bridge_flushes_without_another_read_and_retains_packet_metadata() {
    let tcp_peer = TcpListener::bind("127.0.0.1:0").unwrap();
    let udp_peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp_peer
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let bus = TrafficBus::default();
    let mut tcp = endpoint(
        Config {
            host: "127.0.0.1".into(),
            port: tcp_peer.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus.clone(),
    );
    let (mut stream, _) = tcp_peer.accept().unwrap();
    let mut udp = endpoint(
        Config {
            mode: Mode::UdpClient,
            host: "127.0.0.1".into(),
            port: udp_peer.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus,
    );
    let mut bridge = Bridge::start_configured(
        tcp.bridge_port().unwrap(),
        udp.bridge_port().unwrap(),
        BridgeConfig {
            stream_to_udp: Framing::Idle { gap_ms: 20 },
            ..Default::default()
        },
    )
    .unwrap();
    let events = bridge.subscribe(8);
    stream.write_all(b"no newline").unwrap();
    let mut buffer = [0; 32];
    let n = udp_peer.recv(&mut buffer).unwrap();
    assert_eq!(&buffer[..n], b"no newline");
    let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        event.network.as_ref().unwrap().destination,
        udp_peer.local_addr().unwrap()
    );
    bridge.stop();
    tcp.disconnect();
    udp.disconnect();
}

#[test]
fn serial_to_udp_and_udp_to_serial_preserve_binary() {
    use std::os::fd::AsRawFd;
    let pair = nix::pty::openpty(None, None).unwrap();
    let path = nix::unistd::ttyname(&pair.slave)
        .unwrap()
        .display()
        .to_string();
    let mut master = std::fs::File::from(pair.master);
    nix::fcntl::fcntl(
        master.as_raw_fd(),
        nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
    )
    .unwrap();
    let bus = TrafficBus::default();
    let mut serial = signal_forge::serial::SerialEndpoint::open(
        &SerialSettings {
            path,
            ..Default::default()
        },
        bus.clone(),
    )
    .unwrap();
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut udp = endpoint(
        Config {
            mode: Mode::UdpClient,
            host: "127.0.0.1".into(),
            port: peer.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus,
    );
    let mut bridge =
        Bridge::start(serial.bridge_port().unwrap(), udp.bridge_port().unwrap()).unwrap();
    master.write_all(&[0, 255, 13, 10]).unwrap();
    let mut buffer = [0; 64];
    let (n, address) = peer.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..n], &[0, 255, 13, 10]);
    peer.send_to(&[255, 0, 7], address).unwrap();
    let mut bytes = Vec::new();
    wait(|| {
        match master.read(&mut buffer) {
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("{e}"),
        };
        bytes.len() >= 3
    });
    assert_eq!(bytes, [255, 0, 7]);
    bridge.stop();
    udp.disconnect();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut tcp = endpoint(
        Config {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            ..Default::default()
        },
        TrafficBus::default(),
    );
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut bridge =
        Bridge::start(serial.bridge_port().unwrap(), tcp.bridge_port().unwrap()).unwrap();
    master.write_all(&[0, 255, 4]).unwrap();
    let mut data = [0; 3];
    peer.read_exact(&mut data).unwrap();
    assert_eq!(data, [0, 255, 4]);
    peer.write_all(&[255, 0, 5]).unwrap();
    bytes.clear();
    wait(|| {
        match master.read(&mut buffer) {
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("{e}"),
        };
        bytes.len() >= 3
    });
    assert_eq!(bytes, [255, 0, 5]);
    bridge.stop();
    serial.disconnect();
    tcp.disconnect();
}

#[test]
fn udp_to_udp_forwards_two_packets_including_empty_without_line_framing() {
    let a = UdpSocket::bind("127.0.0.1:0").unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").unwrap();
    b.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let bus = TrafficBus::default();
    let mut left = endpoint(
        Config {
            mode: Mode::UdpClient,
            host: "127.0.0.1".into(),
            port: a.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus.clone(),
    );
    let mut right = endpoint(
        Config {
            mode: Mode::UdpClient,
            host: "127.0.0.1".into(),
            port: b.local_addr().unwrap().port(),
            ..Default::default()
        },
        bus,
    );
    let mut bridge =
        Bridge::start(left.bridge_port().unwrap(), right.bridge_port().unwrap()).unwrap();
    let address = left.network().unwrap().addresses().0.unwrap();
    a.send_to(b"", address).unwrap();
    a.send_to(b"A\nB", address).unwrap();
    let mut buffer = [0; 8];
    assert_eq!(b.recv(&mut buffer).unwrap(), 0);
    let n = b.recv(&mut buffer).unwrap();
    assert_eq!(&buffer[..n], b"A\nB");
    bridge.stop();
    left.disconnect();
    right.disconnect();
}

#[test]
fn listener_peer_loss_cancels_repeat_and_does_not_send_to_replacement() {
    let mut e = endpoint(
        Config {
            mode: Mode::TcpListener,
            ..Default::default()
        },
        TrafficBus::default(),
    );
    let address = e.network().unwrap().addresses().0.unwrap();
    let mut first = TcpStream::connect(address).unwrap();
    first
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    wait(|| e.state() == ConnectionState::Connected);
    let repeat = e
        .start_repeat(
            b"ping".to_vec(),
            RepeatSpec {
                interval: Duration::from_millis(5),
                count: None,
            },
        )
        .unwrap();
    let mut bytes = [0; 4];
    first.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"ping");
    drop(first);
    wait(|| e.state() == ConnectionState::Listening);
    wait(|| !repeat.is_active());
    let mut second = TcpStream::connect(address).unwrap();
    second
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    wait(|| e.state() == ConnectionState::Connected);
    assert!(
        second.read(&mut bytes).is_err(),
        "An old repeat continued on a replacement client"
    );
    e.send(b"new".to_vec()).unwrap();
    second
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    second.read_exact(&mut bytes[..3]).unwrap();
    assert_eq!(&bytes[..3], b"new");
    e.disconnect();
}

#[test]
fn tcp_to_tcp_bridge_is_duplex_and_binary_safe() {
    let listeners = [
        TcpListener::bind("127.0.0.1:0").unwrap(),
        TcpListener::bind("127.0.0.1:0").unwrap(),
    ];
    let bus = TrafficBus::default();
    let mut a = endpoint(
        Config {
            host: "127.0.0.1".into(),
            port: listeners[0].local_addr().unwrap().port(),
            ..Default::default()
        },
        bus.clone(),
    );
    let mut b = endpoint(
        Config {
            host: "127.0.0.1".into(),
            port: listeners[1].local_addr().unwrap().port(),
            ..Default::default()
        },
        bus,
    );
    let (mut peer_a, _) = listeners[0].accept().unwrap();
    let (mut peer_b, _) = listeners[1].accept().unwrap();
    peer_a
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    peer_b
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bridge = Bridge::start(a.bridge_port().unwrap(), b.bridge_port().unwrap()).unwrap();
    peer_a.write_all(&[0, 255, 13, 10]).unwrap();
    let mut bytes = [0; 4];
    peer_b.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 255, 13, 10]);
    peer_b.write_all(&[255, 0, 10, 13]).unwrap();
    peer_a.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [255, 0, 10, 13]);
    bridge.stop();
    a.disconnect();
    b.disconnect();
}
