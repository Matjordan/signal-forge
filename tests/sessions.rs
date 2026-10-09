#![cfg(target_os = "linux")]
use signal_forge::{
    config::{SerialSettings, WorkspaceConfig},
    replay::ReplayConfig,
    session::{ArtifactKind, Session},
    workspace::{Layout, SavedTerminal},
};
use std::{fs, path::Path};
#[test]
fn lifecycle_notes_index_and_readable_files_survive_reopen_without_deleting_artifacts() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("Camera pressure test");
    let mut session =
        Session::create(&root, "Camera pressure test", &WorkspaceConfig::default()).unwrap();
    session.notes = "Test 3\nPressure: 12 kPa\nBinary capture attached.\n".into();
    session.save_notes().unwrap();
    let artifact = root.join("rx.bin");
    fs::write(&artifact, [0, 255, 13, 10]).unwrap();
    session
        .register(&artifact, ArtifactKind::RxRecording)
        .unwrap();
    session
        .register(&artifact, ArtifactKind::Associated)
        .unwrap();
    assert_eq!(session.metadata.artifacts.len(), 1);
    assert_eq!(session.metadata.artifacts[0].path, "rx.bin");
    session
        .save_context(&WorkspaceConfig::default(), true)
        .unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("session.json")).unwrap()).unwrap();
    assert_eq!(metadata["name"], "Camera pressure test");
    assert!(metadata["closed_unix_ns"].is_string());
    let (mut reopened, workspace) = Session::open(&root).unwrap();
    assert!(workspace.layout.is_none());
    assert_eq!(
        reopened.notes,
        "Test 3\nPressure: 12 kPa\nBinary capture attached.\n"
    );
    assert!(reopened.metadata.closed_unix_ns.is_none());
    reopened.notes.push_str("Retested: pass\n");
    reopened.save_context(&workspace, true).unwrap();
    assert_eq!(fs::read(artifact).unwrap(), [0, 255, 13, 10]);
    assert!(fs::read_to_string(root.join("notes.txt"))
        .unwrap()
        .contains("Retested: pass"));
}
fn workspace_for(path: &Path) -> WorkspaceConfig {
    let uri = ReplayConfig {
        path: path.to_str().unwrap().into(),
        ..Default::default()
    }
    .uri();
    let settings = SerialSettings {
        path: uri.clone(),
        ..Default::default()
    };
    WorkspaceConfig {
        ports: vec![settings.clone()],
        selected: Some(uri),
        layout: Some(Layout::Leaf {
            tabs: vec![SavedTerminal {
                settings,
                ..Default::default()
            }],
            active: 0,
        }),
        ..Default::default()
    }
}
#[test]
fn moved_session_keeps_relative_artifacts_and_replay_configuration() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("original");
    let mut session = Session::create(&root, "Portable", &WorkspaceConfig::default()).unwrap();
    let artifact = root.join("binary.bin");
    fs::write(&artifact, [0, 255]).unwrap();
    session
        .register(&artifact, ArtifactKind::RxRecording)
        .unwrap();
    let workspace = workspace_for(&artifact);
    session.save_context(&workspace, true).unwrap();
    let saved =
        WorkspaceConfig::parse(&fs::read_to_string(root.join("workspace.json")).unwrap()).unwrap();
    assert_eq!(
        ReplayConfig::parse(&saved.ports[0].path).unwrap().path,
        "binary.bin"
    );
    drop(session);
    let moved = temporary.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    let (session, loaded) = Session::open(&moved).unwrap();
    assert_eq!(
        ReplayConfig::parse(&loaded.ports[0].path).unwrap().path,
        moved.join("binary.bin").to_str().unwrap()
    );
    assert_eq!(loaded.selected.as_ref().unwrap(), &loaded.ports[0].path);
    assert_eq!(
        fs::read(session.artifact_path(&session.metadata.artifacts[0])).unwrap(),
        [0, 255]
    );
}
#[test]
fn external_artifacts_stay_absolute_and_missing_files_do_not_break_open() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("session");
    let artifact = temporary.path().join("outside.bin");
    fs::write(&artifact, b"outside").unwrap();
    let mut session = Session::create(&root, "External", &WorkspaceConfig::default()).unwrap();
    session
        .register(&artifact, ArtifactKind::Associated)
        .unwrap();
    assert!(Path::new(&session.metadata.artifacts[0].path).is_absolute());
    fs::remove_file(&artifact).unwrap();
    let (mut session, _) = Session::open(&root).unwrap();
    assert!(!session
        .artifact_path(&session.metadata.artifacts[0])
        .exists());
    assert!(session
        .register(&artifact, ArtifactKind::Associated)
        .is_err());
}
#[test]
fn existing_folders_and_invalid_names_never_overwrite_user_files() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("notes.txt"), "keep").unwrap();
    assert!(Session::create(temporary.path(), "Occupied", &WorkspaceConfig::default()).is_err());
    assert_eq!(
        fs::read_to_string(temporary.path().join("notes.txt")).unwrap(),
        "keep"
    );
    let root = temporary.path().join("new");
    for name in ["", "\n", "bad\nname"] {
        assert!(Session::create(&root, name, &WorkspaceConfig::default()).is_err());
        assert!(!root.exists());
    }
}
#[test]
fn external_edits_to_notes_workspace_and_metadata_are_preserved() {
    for target in ["notes.txt", "workspace.json", "session.json"] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("session");
        let mut session = Session::create(&root, "Conflict", &WorkspaceConfig::default()).unwrap();
        fs::write(root.join(target), "edited outside app").unwrap();
        session.notes = "my new notes".into();
        let workspace = WorkspaceConfig {
            ports: vec![SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(session.save_context(&workspace, true).is_err());
        assert_eq!(
            fs::read_to_string(root.join(target)).unwrap(),
            "edited outside app"
        );
    }
}
#[test]
fn malformed_future_missing_and_unsafe_sessions_fail_before_modifying_files() {
    for mode in 0..6 {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("session");
        Session::create(&root, "Invalid", &WorkspaceConfig::default()).unwrap();
        let metadata = root.join("session.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
        match mode {
            0 => value["version"] = 99.into(),
            1 => value["notes"] = "../private.txt".into(),
            2 => {
                value["artifacts"] = serde_json::json!([{"path":"../outside","kind":"associated","added_unix_ns":"1"}])
            }
            3 => {
                fs::remove_file(root.join("workspace.json")).unwrap();
            }
            4 => {
                fs::write(root.join("notes.txt"), [255]).unwrap();
            }
            _ => {
                fs::write(root.join("workspace.json"), "{broken}").unwrap();
            }
        }
        fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
        let before = fs::read(&metadata).unwrap();
        assert!(Session::open(&root).is_err());
        assert_eq!(fs::read(&metadata).unwrap(), before);
    }
}
#[test]
fn suggestions_do_not_collide_and_notes_are_bounded() {
    let temporary = tempfile::tempdir().unwrap();
    let mut session =
        Session::create(temporary.path(), "Bounded", &WorkspaceConfig::default()).unwrap();
    let first = session.suggested_path("rx", "bin");
    fs::write(&first, [255]).unwrap();
    let second = session.suggested_path("rx", "bin");
    assert_ne!(first, second);
    assert!(!second.exists());
    session.notes = "x".repeat(1024 * 1024 + 1);
    assert!(session.save_notes().is_err());
    assert!(fs::read(temporary.path().join("notes.txt"))
        .unwrap()
        .is_empty());
}
#[test]
fn untrusted_relative_replay_paths_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    Session::create(temporary.path(), "Paths", &WorkspaceConfig::default()).unwrap();
    let workspace = workspace_for(Path::new("../outside.bin"));
    workspace
        .save_to(&temporary.path().join("workspace.json"))
        .unwrap();
    let before = fs::read(temporary.path().join("session.json")).unwrap();
    assert!(Session::open(temporary.path()).is_err());
    assert_eq!(
        fs::read(temporary.path().join("session.json")).unwrap(),
        before
    );
}

#[test]
fn raw_triggered_and_bridge_artifacts_finalize_with_exact_binary_bytes() {
    use signal_forge::{
        bridge::{Bridge, BridgePort, BridgeWriter},
        capture::Capture,
        endpoint::{ConnectionState, EndpointError, EndpointId},
        raw_recording::RawRecording,
        traffic::{Direction, TrafficBus},
        triggered_capture::{CaptureOptions, TriggerState, TriggeredCapture},
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    struct Writer;
    impl BridgeWriter for Writer {
        fn state(&self) -> ConnectionState {
            ConnectionState::Connected
        }
        fn write(&self, _: &[u8], cancelled: &AtomicBool) -> Result<bool, EndpointError> {
            Ok(!cancelled.load(Ordering::Acquire))
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let mut session = Session::create(
        temporary.path(),
        "Capture artifacts",
        &WorkspaceConfig::default(),
    )
    .unwrap();
    let bus = TrafficBus::default();
    let endpoint = EndpointId("test".into());
    let payload = [0, 255, 13, 10];
    let raw = session.suggested_path("rx", "bin");
    let mut recording = RawRecording::start(&raw, endpoint.clone(), &bus).unwrap();
    session.register(&raw, ArtifactKind::RxRecording).unwrap();
    let triggered = session.suggested_path("trigger", "jsonl");
    let mut trigger = TriggeredCapture::start(
        &triggered,
        endpoint.clone(),
        &bus,
        CaptureOptions {
            post_bytes: 0,
            post_duration: Duration::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    session
        .register(&triggered, ArtifactKind::TriggeredCapture)
        .unwrap();
    bus.publish(endpoint.clone(), Direction::Rx, &payload);
    let deadline = Instant::now() + Duration::from_secs(5);
    while trigger.status().buffered_bytes != 4 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    trigger.trigger();
    while trigger.status().state != TriggerState::Completed {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    recording.finish();
    trigger.finish();
    let a = BridgePort::new(endpoint, Arc::new(Writer));
    let b = BridgePort::new(EndpointId("other".into()), Arc::new(Writer));
    let mut bridge = Bridge::start(a.clone(), b).unwrap();
    let capture_path = session.suggested_path("bridge", "jsonl");
    let mut capture = Capture::start(&capture_path, &bridge).unwrap();
    session
        .register(&capture_path, ArtifactKind::BridgeCapture)
        .unwrap();
    a.receive(&payload);
    capture.finish();
    bridge.stop();
    session
        .save_context(&WorkspaceConfig::default(), true)
        .unwrap();
    assert_eq!(fs::read(raw).unwrap(), payload);
    for path in [triggered, capture_path] {
        let rows: Vec<serde_json::Value> = fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows[1]["raw_bytes"], serde_json::json!([0, 255, 13, 10]));
        assert_eq!(rows.last().unwrap()["complete"], true);
    }
    let (session, _) = Session::open(temporary.path()).unwrap();
    assert_eq!(session.metadata.artifacts.len(), 3);
}
