#![cfg(target_os = "linux")]
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    unistd::ttyname,
};
use signal_forge::{
    bridge::Bridge,
    config::SerialSettings,
    endpoint::{ConnectionState, Endpoint},
    raw_recording::RawRecording,
    repeat::RepeatSpec,
    serial::SerialEndpoint,
    ssh_serial::{SshHost, SshSerialEndpoint},
    traffic::{Direction, TrafficBus},
};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, OwnedFd},
    thread,
    time::{Duration, Instant},
};

fn pty() -> (File, String, OwnedFd) {
    let pair = openpty(None, None).unwrap();
    fcntl(
        pair.master.as_raw_fd(),
        FcntlArg::F_SETFL(OFlag::O_NONBLOCK),
    )
    .unwrap();
    let path = ttyname(&pair.slave).unwrap().to_string_lossy().into_owned();
    (File::from(pair.master), path, pair.slave)
}
fn host() -> SshHost {
    SshHost {
        host: std::env::var("SIGNAL_FORGE_TEST_SSH_HOST").unwrap(),
        username: None,
        port: None,
    }
}
fn ready(endpoint: &dyn Endpoint) {
    let start = Instant::now();
    while endpoint.state() == ConnectionState::Connecting
        && start.elapsed() < Duration::from_secs(16)
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(endpoint.state(), ConnectionState::Connected);
}
fn read(file: &mut File, expected: &[u8]) {
    let start = Instant::now();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    while bytes.len() < expected.len() && start.elapsed() < Duration::from_secs(8) {
        match file.read(&mut buffer) {
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(bytes, expected);
}
fn remote(path: &str, bus: TrafficBus) -> SshSerialEndpoint {
    let endpoint = SshSerialEndpoint::open(
        &SerialSettings {
            path: host().uri(path),
            ..Default::default()
        },
        bus,
    )
    .unwrap();
    ready(&endpoint);
    endpoint
}

#[test]
fn rejects_credentials_options_and_invalid_remote_paths() {
    for uri in [
        "ssh://-o/dev/ttyUSB0",
        "ssh://host:0/dev/ttyUSB0",
        "ssh://user:password@host/dev/ttyUSB0",
        "ssh://host/tmp/file",
        "ssh://host;touch/dev/ttyUSB0",
    ] {
        assert!(SshHost::parse(uri).is_err(), "{uri}");
    }
    let (host, device) =
        SshHost::parse("ssh://bench-user@pi-workbench:2222/dev/serial/by-id/USB").unwrap();
    assert_eq!(
        host.uri(&device),
        "ssh://bench-user@pi-workbench:2222/dev/serial/by-id/USB"
    );
}

#[test]
#[ignore = "real SSH fixture: python3 scripts/test-remote-serial.py"]
fn real_ssh_binary_repeat_file_recording_bridges_and_failures() {
    let bus = TrafficBus::default();
    let events = bus.subscribe(4096);
    let (mut master, path, slave) = pty();
    let mut endpoint = remote(&path, bus.clone());
    endpoint.send(vec![0, 255, 13, 10]).unwrap();
    read(&mut master, &[0, 255, 13, 10]);
    let temp = tempfile::tempdir().unwrap();
    let recording_path = temp.path().join("rx.bin");
    let mut recording = RawRecording::start(&recording_path, endpoint.id().clone(), &bus).unwrap();
    for chunk in [b"STA".as_slice(), b"TUS\0\xff\r\n".as_slice()] {
        master.write_all(chunk).unwrap();
        thread::sleep(Duration::from_millis(20));
    }
    let mut rx = Vec::new();
    let mut lines = signal_forge::terminal_display::LineDisplay::default();
    while rx.len() < 10 {
        let event = events.recv_timeout(Duration::from_secs(3)).unwrap();
        if event.direction == Direction::Rx {
            lines.receive_with_framing(
                &event,
                signal_forge::terminal_display::SerialFraming::from(&SerialSettings::default()),
            );
            rx.extend_from_slice(&event.bytes);
        }
    }
    assert_eq!(rx, b"STATUS\0\xff\r\n");
    assert_eq!(lines.rows[0].bytes, b"STATUS\0\xff");
    assert!(lines.rows[0]
        .timing
        .as_ref()
        .unwrap()
        .tooltip()
        .contains("19200"));
    recording.finish();
    assert_eq!(std::fs::read(recording_path).unwrap(), rx);
    assert!(recording.status().starts_with("Completed"));
    let repeat = endpoint
        .start_repeat(
            vec![0, 255],
            RepeatSpec {
                interval: Duration::from_millis(20),
                count: Some(3),
            },
        )
        .unwrap();
    read(&mut master, &[0, 255, 0, 255, 0, 255]);
    assert_eq!(repeat.status().sent, 3);
    let file_path = temp.path().join("payload.bin");
    let payload: Vec<u8> = (0..100_000).map(|i| i as u8).collect();
    std::fs::write(&file_path, &payload).unwrap();
    endpoint.send_file(&file_path).unwrap();
    read(&mut master, &payload);
    let (mut local_master, local_path, _local_slave) = pty();
    let local = SerialEndpoint::open(
        &SerialSettings {
            path: local_path,
            ..Default::default()
        },
        bus.clone(),
    )
    .unwrap();
    let mut bridge = Bridge::start(
        endpoint.bridge_port().unwrap(),
        local.bridge_port().unwrap(),
    )
    .unwrap();
    master.write_all(&[0, 255, 1]).unwrap();
    read(&mut local_master, &[0, 255, 1]);
    local_master.write_all(&[255, 0, 2]).unwrap();
    read(&mut master, &[255, 0, 2]);
    bridge.stop();
    let (mut other_master, other_path, _other_slave) = pty();
    let other = remote(&other_path, bus.clone());
    let mut bridge = Bridge::start(
        endpoint.bridge_port().unwrap(),
        other.bridge_port().unwrap(),
    )
    .unwrap();
    master.write_all(b"remote A\0\xff").unwrap();
    read(&mut other_master, b"remote A\0\xff");
    other_master.write_all(b"remote B\r\n").unwrap();
    read(&mut master, b"remote B\r\n");
    bridge.stop();
    endpoint.disconnect();
    assert_eq!(endpoint.state(), ConnectionState::Disconnected);
    endpoint = remote(&path, bus.clone());
    endpoint.send(vec![42]).unwrap();
    read(&mut master, &[42]);
    drop(master);
    drop(slave);
    let start = Instant::now();
    while !matches!(endpoint.state(), ConnectionState::Fault(_))
        && start.elapsed() < Duration::from_secs(4)
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(endpoint.state(), ConnectionState::Fault(_)));
    let missing = SshSerialEndpoint::open(
        &SerialSettings {
            path: host().uri("/dev/signal-forge-missing"),
            ..Default::default()
        },
        bus.clone(),
    )
    .unwrap();
    let start = Instant::now();
    while !matches!(missing.state(), ConnectionState::Fault(_))
        && start.elapsed() < Duration::from_secs(4)
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        matches!(missing.state(), ConnectionState::Fault(_)),
        "{:?}",
        missing.state()
    );
    if let ConnectionState::Fault(message) = missing.state() {
        assert!(message.contains("Remote device access"), "{message}");
    }
    let invalid_baud = SshSerialEndpoint::open(
        &SerialSettings {
            path: host().uri(&other_path),
            baud: 12345,
            ..Default::default()
        },
        bus.clone(),
    )
    .unwrap();
    let start = Instant::now();
    while !matches!(invalid_baud.state(), ConnectionState::Fault(_))
        && start.elapsed() < Duration::from_secs(4)
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        matches!(invalid_baud.state(), ConnectionState::Fault(ref message) if message.contains("unsupported baud")),
        "{:?}",
        invalid_baud.state()
    );
    // Kill only this test process's SSH children to exercise abrupt transport loss.
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let directory = entry.path();
        if std::fs::read_to_string(directory.join("comm"))
            .unwrap_or_default()
            .trim()
            != "ssh"
        {
            continue;
        }
        let status = std::fs::read_to_string(directory.join("status")).unwrap_or_default();
        if status.lines().any(|line| {
            line.strip_prefix("PPid:")
                .is_some_and(|value| value.trim() == std::process::id().to_string())
        }) {
            unsafe {
                nix::libc::kill(pid, nix::libc::SIGTERM);
            }
        }
    }
    let start = Instant::now();
    while !matches!(other.state(), ConnectionState::Fault(_))
        && start.elapsed() < Duration::from_secs(4)
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(other.state(), ConnectionState::Fault(_)));
    let discovered = signal_forge::ssh_serial::discover(&host()).unwrap();
    assert!(discovered.iter().all(|p| p.starts_with("/dev/")));
    let known_path = std::env::var("SIGNAL_FORGE_TEST_KNOWN_HOSTS").unwrap();
    let known_contents = std::fs::read_to_string(&known_path).unwrap();
    std::fs::write(
        &known_path,
        std::env::var("SIGNAL_FORGE_TEST_CHANGED_KNOWN_HOSTS").unwrap(),
    )
    .unwrap();
    let changed = signal_forge::ssh_serial::discover(&host())
        .unwrap_err()
        .to_string();
    assert!(
        changed.contains("HOST IDENTIFICATION HAS CHANGED"),
        "{changed}"
    );
    std::fs::write(&known_path, &known_contents).unwrap();
    // Key-file authentication above worked with an empty agent. Now use only the agent.
    let key_path = std::env::var("SIGNAL_FORGE_TEST_CLIENT_KEY").unwrap();
    assert!(std::process::Command::new("ssh-add")
        .arg(&key_path)
        .status()
        .unwrap()
        .success());
    let config_path = std::env::var("SIGNAL_FORGE_TEST_SSH_CONFIG").unwrap();
    let original_config = std::fs::read_to_string(&config_path).unwrap();
    let agent_config: String = original_config
        .lines()
        .filter(|line| !line.contains("IdentityFile") && !line.contains("IdentitiesOnly"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&config_path, &agent_config).unwrap();
    signal_forge::ssh_serial::discover(&host()).unwrap();
    // An unrelated private key and an empty agent must be rejected.
    assert!(std::process::Command::new("ssh-add")
        .arg("-D")
        .status()
        .unwrap()
        .success());
    std::fs::write(
        &config_path,
        original_config.replace(
            &key_path,
            &std::env::var("SIGNAL_FORGE_TEST_UNAUTHORIZED_KEY").unwrap(),
        ),
    )
    .unwrap();
    let denied = signal_forge::ssh_serial::discover(&host())
        .unwrap_err()
        .to_string();
    assert!(denied.contains("Permission denied"), "{denied}");
    std::fs::write(&config_path, &original_config).unwrap();
    // Removing our isolated trust entry must fail rather than accepting a new key.
    std::fs::write(std::env::var("SIGNAL_FORGE_TEST_KNOWN_HOSTS").unwrap(), "").unwrap();
    assert!(signal_forge::ssh_serial::discover(&host())
        .unwrap_err()
        .to_string()
        .contains("Host key verification failed"));
}

#[test]
fn remote_workspace_roundtrip_preserves_nonsecret_settings() {
    let config = signal_forge::config::WorkspaceConfig {
        remote_hosts: vec![signal_forge::ssh_serial::SavedSshHost {
            name: "Pi workbench".into(),
            connection: SshHost::parse("ssh://bench@pi-workbench:2222/dev/ttyUSB0")
                .unwrap()
                .0,
        }],
        ports: vec![SerialSettings {
            path: "ssh://bench@pi-workbench:2222/dev/ttyUSB0".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let text = serde_json::to_string(&config).unwrap();
    let restored = signal_forge::config::WorkspaceConfig::parse(&text).unwrap();
    assert_eq!(restored.ports[0].path, config.ports[0].path);
    assert_eq!(restored.ports[0].baud, 19200);
    assert_eq!(restored.remote_hosts[0].name, "Pi workbench");
    assert!(!text.contains("password"));
}

#[test]
#[ignore = "real SSH fixture: python3 scripts/test-remote-serial.py"]
fn ssh_handshake_timeout_and_discovery_cancellation_are_bounded() {
    // TCP connects, but no server sends an SSH banner: exercise ConnectTimeout.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stalled = SshHost {
        host: "127.0.0.1".into(),
        username: None,
        port: Some(listener.local_addr().unwrap().port()),
    };
    let started = Instant::now();
    let endpoint = SshSerialEndpoint::open(
        &SerialSettings {
            path: stalled.uri("/dev/ttyUSB0"),
            ..Default::default()
        },
        TrafficBus::default(),
    )
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "SSH connection blocked its caller"
    );
    let discovery = signal_forge::ssh_serial::Discovery::start(stalled);
    thread::sleep(Duration::from_millis(100));
    let cancelled = Instant::now();
    drop(discovery);
    assert!(
        cancelled.elapsed() < Duration::from_secs(2),
        "Discovery cancellation blocked its caller"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !matches!(endpoint.state(), ConnectionState::Fault(_)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        matches!(endpoint.state(), ConnectionState::Fault(ref message) if message.to_lowercase().contains("timed out")),
        "{:?}",
        endpoint.state()
    );
}
