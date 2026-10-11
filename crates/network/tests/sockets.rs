use signal_forge_network::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};
fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !condition() {
        assert!(Instant::now() < deadline, "socket operation timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn receive(t: &Transport) -> (Vec<u8>, Option<Metadata>) {
    let mut buffer = [0; 65536];
    let mut result = None;
    wait(|| match t.receive(&mut buffer) {
        Ok((n, m)) => {
            result = Some((buffer[..n].to_vec(), m));
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(e) => panic!("{e}"),
    });
    result.unwrap()
}
#[test]
fn tcp_client_binary_and_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let t = Transport::open(Config {
        port: listener.local_addr().unwrap().port(),
        host: "127.0.0.1".into(),
        ..Default::default()
    })
    .unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    wait(|| t.state() == State::Connected);
    peer.write_all(&[0, 255, 10]).unwrap();
    assert_eq!(receive(&t).0, [0, 255, 10]);
    t.send(&[255, 0], &AtomicBool::new(false)).unwrap();
    let mut b = [0; 2];
    peer.read_exact(&mut b).unwrap();
    assert_eq!(b, [255, 0]);
    drop(peer);
    wait(|| {
        let _ = t.receive(&mut [0; 8]);
        t.state() == State::Disconnected
    });
    t.disconnect();
}
#[test]
fn tcp_listener_accepts_replacement() {
    let t = Transport::open(Config {
        mode: Mode::TcpListener,
        ..Default::default()
    })
    .unwrap();
    wait(|| t.state() == State::Listening);
    let address = t.addresses().0.unwrap();
    for payload in [b"first".as_slice(), b"second"] {
        let mut peer = TcpStream::connect(address).unwrap();
        peer.write_all(payload).unwrap();
        assert_eq!(receive(&t).0, payload);
        drop(peer);
        wait(|| {
            let _ = t.receive(&mut [0; 8]);
            t.state() == State::Listening
        });
    }
    t.disconnect();
}
#[test]
fn udp_empty_binary_multiple_peers_and_explicit_reply() {
    let t = Transport::open(Config {
        mode: Mode::UdpListener,
        ..Default::default()
    })
    .unwrap();
    wait(|| t.state() == State::Connected);
    let address = t.addresses().0.unwrap();
    assert!(t
        .send(b"x", &AtomicBool::new(false))
        .unwrap_err()
        .to_string()
        .contains("explicit"));
    let a = UdpSocket::bind("127.0.0.1:0").unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").unwrap();
    a.send_to(b"", address).unwrap();
    b.send_to(&[0, 255, 10], address).unwrap();
    let (empty, meta) = receive(&t);
    assert!(empty.is_empty());
    assert_eq!(meta.unwrap().source, a.local_addr().unwrap());
    let (data, meta) = receive(&t);
    assert_eq!(data, [0, 255, 10]);
    assert_eq!(meta.unwrap().source, b.local_addr().unwrap());
    assert_eq!(t.addresses().1, None);
    t.set_peer(a.local_addr().unwrap()).unwrap();
    t.send(b"reply", &AtomicBool::new(false)).unwrap();
    a.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut bytes = [0; 16];
    assert_eq!(a.recv(&mut bytes).unwrap(), 5);
    assert!(t.send(&vec![0; 65508], &AtomicBool::new(false)).is_err());
    t.disconnect();
}
#[test]
fn udp_client_ipv6() {
    let Ok(peer) = UdpSocket::bind("[::1]:0") else {
        return;
    };
    let t = Transport::open(Config {
        mode: Mode::UdpClient,
        host: "::1".into(),
        port: peer.local_addr().unwrap().port(),
        bind: "[::1]:0".into(),
        ..Default::default()
    })
    .unwrap();
    wait(|| t.state() == State::Connected);
    let meta = t.send(b"ipv6", &AtomicBool::new(false)).unwrap().unwrap();
    assert!(meta.destination.is_ipv6());
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    assert_eq!(peer.recv(&mut [0; 8]).unwrap(), 4);
    t.disconnect();
}
#[test]
fn failures_and_cancel() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let t = Transport::open(Config {
        host: "127.0.0.1".into(),
        port,
        timeout_ms: 100,
        ..Default::default()
    })
    .unwrap();
    wait(|| matches!(t.state(), State::Fault(_)));
    t.disconnect();
    let t = Transport::open(Config {
        host: "192.0.2.1".into(),
        port: 5000,
        timeout_ms: 1000,
        ..Default::default()
    })
    .unwrap();
    t.disconnect();
    thread::sleep(Duration::from_millis(120));
    assert_eq!(t.state(), State::Disconnected);
}
#[test]
fn uri_roundtrip() {
    for mode in [
        Mode::TcpClient,
        Mode::TcpListener,
        Mode::UdpClient,
        Mode::UdpListener,
    ] {
        let c = Config {
            mode,
            host: "::1".into(),
            bind: "[::1]:0".into(),
            reply: mode.udp().then(|| "[::1]:5001".parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(Config::parse(&c.uri()).unwrap().mode, c.mode);
        assert_eq!(Config::parse(&c.uri()).unwrap().reply, c.reply);
    }
}
#[test]
fn line_framing_fragmented_crlf_and_empty() {
    let mut p = Packetizer::new(Framing::default());
    let now = Instant::now();
    assert!(p.push(b"CMD1\r", now).unwrap().is_empty());
    assert_eq!(
        p.push(b"\nCMD2\r\n", now).unwrap(),
        [b"CMD1\r\n".to_vec(), b"CMD2\r\n".to_vec()]
    );
    let mut p = Packetizer::new(Framing::Line { strip_ending: true });
    assert_eq!(p.push(b"\r\nX\n", now).unwrap(), [vec![], b"X".to_vec()]);
}
#[test]
fn fixed_idle_raw_and_bounds() {
    let now = Instant::now();
    let mut p = Packetizer::new(Framing::Fixed { size: 3 });
    assert_eq!(
        p.push(b"abcdefg", now).unwrap(),
        [b"abc".to_vec(), b"def".to_vec()]
    );
    let mut p = Packetizer::new(Framing::Idle { gap_ms: 10 });
    assert!(p.push(b"abc", now).unwrap().is_empty());
    assert!(p.tick(now + Duration::from_millis(9)).is_empty());
    assert_eq!(p.tick(now + Duration::from_millis(10)), [b"abc".to_vec()]);
    let mut p = Packetizer::new(Framing::Raw);
    assert_eq!(p.push(b"a\nb", now).unwrap(), [b"a\nb".to_vec()]);
    let mut p = Packetizer::new(Framing::default());
    assert!(p.push(&vec![b'a'; 65508], now).is_err());
}

#[test]
fn tcp_ipv6_client_and_listener() {
    let Ok(server) = TcpListener::bind("[::1]:0") else {
        return;
    };
    let transport = Transport::open(Config {
        host: "::1".into(),
        port: server.local_addr().unwrap().port(),
        ..Default::default()
    })
    .unwrap();
    let (mut peer, _) = server.accept().unwrap();
    wait(|| transport.state() == State::Connected);
    peer.write_all(b"IPv6 TCP").unwrap();
    assert_eq!(receive(&transport).0, b"IPv6 TCP");
    transport.disconnect();
    let transport = Transport::open(Config {
        mode: Mode::TcpListener,
        bind: "[::1]:0".into(),
        ..Default::default()
    })
    .unwrap();
    wait(|| transport.state() == State::Listening);
    let mut peer = TcpStream::connect(transport.addresses().0.unwrap()).unwrap();
    peer.write_all(b"listener").unwrap();
    assert_eq!(receive(&transport).0, b"listener");
    transport.disconnect();
}

#[test]
fn cancellation_under_tcp_backpressure_preserves_observed_partial_writes() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let transport = Transport::open(Config {
        host: "127.0.0.1".into(),
        port: listener.local_addr().unwrap().port(),
        ..Default::default()
    })
    .unwrap();
    let (_peer, _) = listener.accept().unwrap();
    wait(|| transport.state() == State::Connected);
    let cancelled = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicUsize::new(0));
    let worker_transport = transport.clone();
    let worker_cancelled = cancelled.clone();
    let worker_observed = observed.clone();
    let worker = thread::spawn(move || {
        worker_transport.send_observed(
            &vec![0xA5; 16 * 1024 * 1024],
            &worker_cancelled,
            |bytes, metadata| {
                assert!(metadata.is_none());
                worker_observed.fetch_add(bytes.len(), Ordering::Release);
            },
        )
    });
    wait(|| observed.load(Ordering::Acquire) > 0);
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        worker.join().unwrap().unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
    assert!(observed.load(Ordering::Acquire) < 16 * 1024 * 1024);
    assert_eq!(transport.state(), State::Connected);
    transport.disconnect();
}

#[test]
fn saved_explicit_udp_target_survives_reconnect_and_overrides_original_hostname() {
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let saved = Config {
        mode: Mode::UdpClient,
        host: "unused.invalid".into(),
        reply: Some(peer.local_addr().unwrap()),
        ..Default::default()
    }
    .uri();
    for _ in 0..2 {
        let transport = Transport::open(Config::parse(&saved).unwrap()).unwrap();
        wait(|| transport.state() == State::Connected);
        assert_eq!(transport.addresses().1, Some(peer.local_addr().unwrap()));
        transport.send(b"chosen", &AtomicBool::new(false)).unwrap();
        let mut bytes = [0; 8];
        let n = peer.recv(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"chosen");
        transport.disconnect();
    }
}
