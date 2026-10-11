use signal_forge::{
    config::{SerialSettings, WorkspaceConfig},
    remote_config::edit_host,
    ssh_serial::{SavedSshHost, SshHost},
    workspace::{Layout, SavedTerminal, SavedWindow},
};
fn host(name: &str, address: &str) -> SavedSshHost {
    SavedSshHost {
        name: name.into(),
        connection: SshHost::parse(&format!("ssh://{address}/dev/ttyUSB0"))
            .unwrap()
            .0,
    }
}
fn terminal(host: &SavedSshHost, device: &str, linked: bool) -> SavedTerminal {
    SavedTerminal {
        settings: SerialSettings {
            path: host.connection.uri(device),
            ..Default::default()
        },
        remote_host: linked.then(|| host.name.clone()),
        ..Default::default()
    }
}
#[test]
fn shared_host_change_preserves_ids_layout_selection_and_auth_across_restart() {
    let old = host("Workshop", "user@old-pi:2222");
    let other = host("Other", "other-pi");
    let a = terminal(&old, "/dev/ttyUSB0", true);
    let b = terminal(&old, "/dev/ttyACM0", false);
    let c = terminal(&other, "/dev/ttyUSB0", true);
    let mut config = WorkspaceConfig {
        remote_hosts: vec![old.clone(), other],
        ports: vec![a.settings.clone(), b.settings.clone(), c.settings.clone()],
        selected: Some(b.settings.path.clone()),
        layout: Some(Layout::Split {
            horizontal: true,
            fraction: 0.37,
            first: Box::new(Layout::Leaf {
                tabs: vec![a.clone()],
                active: 0,
            }),
            second: Box::new(Layout::Leaf {
                tabs: vec![c.clone()],
                active: 0,
            }),
        }),
        windows: vec![SavedWindow {
            layout: Layout::Leaf {
                tabs: vec![b.clone()],
                active: 0,
            },
            position: [10., 20.],
            size: [600., 400.],
        }],
        ..Default::default()
    };
    let mut new = host("Renamed workshop", "newuser@new-pi:2200");
    new.connection.identity_file = Some("/tmp/key with spaces".into());
    let changed = edit_host(&config, &old.name, new.clone()).unwrap();
    assert_eq!(config.ports[0].path, a.settings.path); // Input is never mutated.
    assert_eq!(changed.selected, Some(new.connection.uri("/dev/ttyACM0")));
    assert_eq!(changed.ports[2].path, c.settings.path);
    let Layout::Split {
        fraction,
        first,
        second,
        ..
    } = changed.layout.as_ref().unwrap()
    else {
        panic!()
    };
    assert_eq!(*fraction, 0.37);
    let Layout::Leaf { tabs, .. } = first.as_ref() else {
        panic!()
    };
    assert_eq!(
        tabs[0].endpoint_id,
        Some(format!("serial:{}", a.settings.path))
    );
    assert_eq!(tabs[0].remote_host, Some(new.name.clone()));
    let Layout::Leaf { tabs, .. } = second.as_ref() else {
        panic!()
    };
    assert_eq!(tabs[0].settings.path, c.settings.path);
    assert_eq!(tabs[0].endpoint_id, None);
    let Layout::Leaf { tabs, .. } = &changed.windows[0].layout else {
        panic!()
    };
    assert_eq!(
        tabs[0].endpoint_id,
        Some(format!("serial:{}", b.settings.path))
    );
    let restored = WorkspaceConfig::parse(&serde_json::to_string(&changed).unwrap()).unwrap();
    assert_eq!(restored.remote_hosts[0], new);
    config = restored;
    let next = edit_host(&config, "Renamed workshop", host("Workshop", "third-pi")).unwrap();
    let Layout::Leaf { tabs, .. } = &next.windows[0].layout else {
        panic!()
    };
    assert_eq!(
        tabs[0].endpoint_id,
        Some(format!("serial:{}", b.settings.path))
    );
}
#[test]
fn invalid_or_colliding_host_edits_leave_original_configuration_intact() {
    let a = host("A", "a");
    let b = host("B", "b");
    let config = WorkspaceConfig {
        remote_hosts: vec![a.clone(), b.clone()],
        layout: Some(Layout::Leaf {
            tabs: vec![
                terminal(&a, "/dev/ttyUSB0", true),
                terminal(&b, "/dev/ttyUSB0", true),
            ],
            active: 0,
        }),
        ..Default::default()
    };
    let before = serde_json::to_string(&config).unwrap();
    assert!(edit_host(&config, "A", host("B", "c")).is_err());
    assert!(edit_host(&config, "A", host("A", "b")).is_err());
    let mut bad = a.clone();
    bad.connection.identity_file = Some("bad\npath".into());
    assert!(edit_host(&config, "A", bad).is_err());
    assert_eq!(serde_json::to_string(&config).unwrap(), before);
}
#[test]
fn explicitly_linked_hosts_with_same_address_are_not_reassigned() {
    let a = host("A", "same");
    let b = host("B", "same");
    let config = WorkspaceConfig {
        remote_hosts: vec![a.clone(), b.clone()],
        ports: vec![terminal(&b, "/dev/ttyUSB0", true).settings],
        selected: Some(b.connection.uri("/dev/ttyUSB0")),
        layout: Some(Layout::Leaf {
            tabs: vec![terminal(&b, "/dev/ttyUSB0", true)],
            active: 0,
        }),
        ..Default::default()
    };
    let changed = edit_host(&config, "A", host("A", "elsewhere")).unwrap();
    assert_eq!(changed.ports[0].path, b.connection.uri("/dev/ttyUSB0"));
    assert_eq!(changed.selected, Some(b.connection.uri("/dev/ttyUSB0")));
    let Layout::Leaf { tabs, .. } = changed.layout.unwrap() else {
        panic!()
    };
    assert_eq!(tabs[0].settings.path, b.connection.uri("/dev/ttyUSB0"));
    assert_eq!(tabs[0].remote_host, Some("B".into()));
}

#[test]
fn duplicate_persisted_endpoint_ids_are_rejected() {
    let saved = host("A", "a");
    let mut a = terminal(&saved, "/dev/ttyUSB0", true);
    a.endpoint_id = Some("serial:stable".into());
    let mut b = terminal(&saved, "/dev/ttyUSB1", true);
    b.endpoint_id = a.endpoint_id.clone();
    let config = WorkspaceConfig {
        remote_hosts: vec![saved],
        layout: Some(Layout::Leaf {
            tabs: vec![a, b],
            active: 0,
        }),
        ..Default::default()
    };
    assert!(config
        .validate()
        .unwrap_err()
        .contains("Duplicate terminal identity"));
}

#[test]
fn reopening_an_original_address_cannot_steal_a_preserved_preset_identity() {
    use signal_forge::endpoint::EndpointId;
    let path = "ssh://original/dev/ttyUSB0";
    let original = EndpointId(format!("serial:{path}"));
    let first = signal_forge::remote_config::available_endpoint_id(path, &[original.clone()]);
    let second = signal_forge::remote_config::available_endpoint_id(
        path,
        &[original.clone(), first.clone()],
    );
    assert_ne!(first, original);
    assert_ne!(second, original);
    assert_ne!(second, first);
    assert_eq!(
        signal_forge::remote_config::available_endpoint_id("ssh://other/dev/ttyUSB0", &[original]),
        EndpointId("serial:ssh://other/dev/ttyUSB0".into())
    );
}
