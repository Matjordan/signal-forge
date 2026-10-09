use signal_forge::{
    config::{SerialSettings, WorkspaceConfig},
    send::{Encoding, LineEnding},
    workspace::{Layout, SavedTerminal, SavedWindow},
};
fn terminal(path: &str) -> SavedTerminal {
    SavedTerminal {
        settings: SerialSettings {
            path: path.into(),
            baud: 9600,
            ..Default::default()
        },
        hex: true,
        timestamps: false,
        show_controls: true,
        analysis: signal_forge::traffic_analysis::ViewSettings {
            visibility: signal_forge::traffic_analysis::Visibility::Rx,
            direction_labels: false,
            delta_displayed: true,
            search: signal_forge::traffic_analysis::Pattern {
                mode: signal_forge::traffic_analysis::PatternMode::Hex,
                value: "00 FF".into(),
            },
            ..Default::default()
        },
        auto_scroll: false,
        encoding: Encoding::Hex,
        ending: LineEnding::CrLf,
        ..Default::default()
    }
}
fn leaf(path: &str) -> Layout {
    Layout::Leaf {
        tabs: vec![terminal(path)],
        active: 0,
    }
}
#[test]
fn migration_and_workspace_roundtrip_preserve_preferences_and_layout() {
    let legacy = r#"{"version":1,"ports":[{"path":"/dev/ttyUSB0","baud":9600,"data_bits":8,"parity":"None","stop_bits":1,"flow":"None"}]}"#;
    let mut config = WorkspaceConfig::parse(legacy).unwrap();
    assert_eq!(config.version, 2);
    assert_eq!(config.ports[0].baud, 9600);
    config.layout = Some(Layout::Split {
        horizontal: false,
        fraction: 0.3,
        first: Box::new(leaf("/dev/ttyUSB0")),
        second: Box::new(Layout::Leaf {
            tabs: vec![terminal("/dev/ttyUSB1"), terminal("/dev/ttyUSB2")],
            active: 1,
        }),
    });
    config.windows.push(SavedWindow {
        layout: leaf("/dev/ttyUSB3"),
        position: [25.0, 80.0],
        size: [800.0, 600.0],
    });
    config.profile = Some("Bench".into());
    let text = serde_json::to_string(&config).unwrap();
    let restored = WorkspaceConfig::parse(&text).unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(config).unwrap()
    );
    assert!(!text.contains("payload"));
    assert!(!text.contains("Connected"));
}
#[test]
fn reject_corruption_future_versions_and_invalid_layouts_without_touching_file() {
    let path =
        std::env::temp_dir().join(format!("signal-forge-config-{}.json", std::process::id()));
    std::fs::write(&path, b"{broken JSON").unwrap();
    let error = WorkspaceConfig::load_from(&path).unwrap_err();
    assert!(error.contains("Original file preserved"));
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken JSON");
    assert!(WorkspaceConfig::parse(r#"{"version":99,"ports":[]}"#).is_err());
    let mut config = WorkspaceConfig::default();
    config.layout = Some(Layout::Leaf {
        tabs: vec![terminal("/dev/test")],
        active: 10,
    });
    assert!(config.save_to(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken JSON");
    config.layout = Some(Layout::Split {
        horizontal: true,
        fraction: 0.5,
        first: Box::new(leaf("/dev/test")),
        second: Box::new(leaf("/dev/test")),
    });
    assert!(config.validate().unwrap_err().contains("Duplicate"));
    std::fs::remove_file(path).unwrap();
}
#[test]
fn atomic_save_replaces_file_and_leaves_no_temporary_file() {
    let root = std::env::temp_dir().join(format!("signal-forge-save-{}", std::process::id()));
    let path = root.join("workspace.json");
    let mut config = WorkspaceConfig::default();
    config.save_to(&path).unwrap();
    config.ports.push(terminal("/dev/test").settings);
    config.save_to(&path).unwrap();
    assert_eq!(
        WorkspaceConfig::load_from(&path).unwrap().ports[0].baud,
        9600
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_ports_default_to_19200_and_saved_custom_rates_survive() {
    assert_eq!(SerialSettings::default().baud, 19200);
    for baud in [9600, 115200, 14400, 5_000_000] {
        let mut saved = terminal("/dev/test");
        saved.settings.baud = baud;
        let config = WorkspaceConfig {
            ports: vec![saved.settings.clone()],
            layout: Some(Layout::Leaf {
                tabs: vec![saved],
                active: 0,
            }),
            ..Default::default()
        };
        let restored = WorkspaceConfig::parse(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored.ports[0].baud, baud);
        let Some(Layout::Leaf { tabs, .. }) = restored.layout else {
            panic!("Missing terminal")
        };
        assert_eq!(tabs[0].settings.baud, baud);
    }
}
