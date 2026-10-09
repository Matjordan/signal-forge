use super::*;
use egui_dock::{Node, Surface, SurfaceIndex, TabIndex};
use signal_forge::workspace::{Layout, SavedTerminal, SavedWindow};

struct RestoredEndpoint {
    id: EndpointId,
    path: String,
    name: String,
}
impl Endpoint for RestoredEndpoint {
    fn read_only(&self) -> bool {
        self.path.starts_with("replay://")
    }
    fn id(&self) -> &EndpointId {
        &self.id
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ConnectionState {
        ConnectionState::Disconnected
    }
    fn send(&self, _: Vec<u8>) -> Result<(), EndpointError> {
        Err(EndpointError::Disconnected)
    }
    fn start_repeat(&self, _: Vec<u8>, _: RepeatSpec) -> Result<RepeatHandle, EndpointError> {
        Err(EndpointError::Disconnected)
    }
    fn bridge_port(&self) -> Result<signal_forge::bridge::BridgePort, EndpointError> {
        Err(EndpointError::Disconnected)
    }
    fn disconnect(&mut self) {}
}
impl Terminal {
    pub(super) fn saved(&self) -> SavedTerminal {
        SavedTerminal {
            settings: self.settings.clone(),
            hex: self.receive_mode == ReceiveMode::Hex,
            receive_mode: Some(self.receive_mode),
            delimiter: self.lines.delimiter,
            timestamps: self.timestamps,
            analysis: self.analysis.clone(),
            show_controls: self.show_controls,
            auto_scroll: self.auto_scroll,
            encoding: self.encoding,
            escapes: self.escapes,
            ending: self.ending,
        }
    }
    pub(super) fn restored(saved: &SavedTerminal) -> Self {
        let endpoint = RestoredEndpoint {
            id: EndpointId(format!("serial:{}", saved.settings.path)),
            path: saved.settings.path.clone(),
            name: if let Ok(config) =
                signal_forge::replay::ReplayConfig::parse(&saved.settings.path)
            {
                format!("Replay: {}", config.path)
            } else {
                saved.settings.path.clone()
            },
        };
        let mut tab = Self::new(endpoint, saved.settings.clone());
        tab.receive_mode = saved.receive_mode.unwrap_or(if saved.hex {
            ReceiveMode::Hex
        } else {
            ReceiveMode::Line
        });
        tab.lines = LineDisplay::new(saved.delimiter);
        tab.timestamps = saved.timestamps;
        tab.analysis = saved.analysis.clone();
        tab.show_controls = saved.show_controls;
        tab.auto_scroll = saved.auto_scroll;
        tab.encoding = saved.encoding;
        tab.escapes = saved.escapes;
        tab.ending = saved.ending;
        tab
    }
}
fn snapshot(tree: &egui_dock::Tree<Terminal>, node: NodeIndex) -> Option<Layout> {
    if node.0 >= tree.len() {
        return None;
    }
    match &tree[node] {
        Node::Empty => None,
        Node::Leaf { tabs, .. } if tabs.is_empty() => None,
        Node::Leaf { tabs, active, .. } => Some(Layout::Leaf {
            tabs: tabs.iter().map(Terminal::saved).collect(),
            active: active.0,
        }),
        Node::Horizontal { fraction, .. } | Node::Vertical { fraction, .. } => {
            let first = snapshot(tree, NodeIndex(node.0 * 2 + 1));
            let second = snapshot(tree, NodeIndex(node.0 * 2 + 2));
            match (first, second) {
                (Some(first), Some(second)) => Some(Layout::Split {
                    horizontal: matches!(&tree[node], Node::Horizontal { .. }),
                    fraction: fraction.clamp(0.05, 0.95),
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (first, second) => first.or(second),
            }
        }
    }
}
fn restore_tree(layout: &Layout) -> egui_dock::Tree<Terminal> {
    fn seed(layout: &Layout) -> Terminal {
        match layout {
            Layout::Leaf { tabs, .. } => Terminal::restored(&tabs[0]),
            Layout::Split { first, .. } => seed(first),
        }
    }
    fn fill(tree: &mut egui_dock::Tree<Terminal>, node: NodeIndex, layout: &Layout) {
        match layout {
            Layout::Leaf { tabs, active } => {
                tree[node] = Node::leaf_with(tabs.iter().map(Terminal::restored).collect());
                tree.set_active_tab(node, TabIndex(*active));
            }
            Layout::Split {
                horizontal,
                fraction,
                first,
                second,
            } => {
                let children = if *horizontal {
                    tree.split_right(node, *fraction, vec![seed(second)])
                } else {
                    tree.split_below(node, *fraction, vec![seed(second)])
                };
                fill(tree, children[0], first);
                fill(tree, children[1], second);
            }
        }
    }
    let mut tree = egui_dock::Tree::new(vec![seed(layout)]);
    fill(&mut tree, NodeIndex::root(), layout);
    tree
}
impl Workbench {
    pub(super) fn restore_workspace(&mut self) {
        if let Some(layout) = &self.config.layout {
            *self.dock.main_surface_mut() = restore_tree(layout);
        }
        for saved in &self.config.windows {
            let surface = self.dock.add_window(vec![]);
            self.dock[surface] = restore_tree(&saved.layout);
            if let Some(state) = self.dock.get_window_state_mut(surface) {
                state.set_position(egui::pos2(saved.position[0], saved.position[1]));
                state.set_size(egui::vec2(saved.size[0], saved.size[1]));
            }
        }
        self.profile_index = self
            .config
            .profile
            .as_ref()
            .and_then(|name| {
                self.library
                    .profiles
                    .iter()
                    .position(|profile| &profile.name == name)
            })
            .unwrap_or(0);
        self.selected = self
            .config
            .selected
            .as_ref()
            .map(|path| EndpointId(format!("serial:{path}")));
    }
    pub(super) fn snapshot_workspace(&mut self) {
        for (_, tab) in self.dock.iter_all_tabs() {
            self.config
                .ports
                .retain(|settings| settings.path != tab.settings.path);
            self.config.ports.push(tab.settings.clone());
        }
        let excess = self.config.ports.len().saturating_sub(256);
        self.config.ports.drain(..excess);
        self.config.layout = snapshot(self.dock.main_surface(), NodeIndex::root());
        self.config.windows.clear();
        for index in 1..self.dock.surfaces_count() {
            let surface = SurfaceIndex(index);
            let layout = match self.dock.get_surface(surface) {
                Some(Surface::Window(tree, _)) => snapshot(tree, NodeIndex::root()),
                _ => None,
            };
            if let Some(layout) = layout {
                let rect = self
                    .dock
                    .get_window_state(surface)
                    .map(|state| state.rect());
                let (position, size) = rect
                    .filter(|rect| rect.is_finite())
                    .map(|rect| {
                        (
                            [rect.min.x, rect.min.y],
                            [
                                rect.width().clamp(100.0, 16000.0),
                                rect.height().clamp(100.0, 16000.0),
                            ],
                        )
                    })
                    .unwrap_or(([100.0, 100.0], [700.0, 500.0]));
                self.config.windows.push(SavedWindow {
                    layout,
                    position,
                    size,
                });
            }
        }
        self.config.profile = self
            .library
            .profiles
            .get(self.profile_index)
            .map(|profile| profile.name.clone());
        self.config.selected = self.selected.as_ref().and_then(|id| {
            self.dock
                .iter_all_tabs()
                .find(|(_, tab)| tab.endpoint.id() == id)
                .map(|(_, tab)| tab.settings.path.clone())
        });
    }
    pub(super) fn save_workspace(&mut self) {
        if !self.config_recoverable {
            return;
        }
        self.snapshot_workspace();
        if let Err(error) = self.save_session_context(false) {
            self.error = Some(error);
            return;
        }
        match self.config.save() {
            Ok(()) => {
                self.error = None;
                log::info!("Workspace saved");
            }
            Err(error) => {
                log::error!("Workspace save: {error}");
                self.error = Some(error);
            }
        }
    }
    pub(super) fn workspace_shortcuts(&mut self, ctx: &egui::Context) {
        if self.setup.is_some() {
            return;
        }
        if !ctx.wants_keyboard_input() {
            if ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S)
            }) {
                self.open_setup(workbench_ui::SetupKind::Workspace);
                return;
            }
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save_workspace();
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::D)
        }) {
            if let Some(id) = &self.selected {
                if let Some((_, tab)) = self
                    .dock
                    .iter_all_tabs_mut()
                    .find(|(_, tab)| tab.endpoint.id() == id)
                {
                    tab.stop_repeat();
                    tab.endpoint.disconnect();
                }
            }
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O)
        }) {
            if let Some(id) = &self.selected {
                if let Some((_, tab)) = self
                    .dock
                    .iter_all_tabs_mut()
                    .find(|(_, tab)| tab.endpoint.id() == id)
                {
                    if tab.endpoint.state() != ConnectionState::Connected {
                        if let Err(error) = tab.baud_control.validate() {
                            tab.error = Some(error.into());
                        } else {
                            tab.stop_repeat();
                            tab.endpoint.disconnect();
                            match signal_forge::endpoint::open(&tab.settings, self.bus.clone()) {
                                Ok(endpoint) => {
                                    tab.active_framing = SerialFraming::from(&tab.settings);
                                    tab.rx_gap = true;
                                    tab.lines.discard_pending();
                                    tab.endpoint = endpoint;
                                    tab.error = None;
                                }
                                Err(error) => tab.error = Some(error.to_string()),
                            }
                        }
                    }
                }
            }
        }
        if !ctx.wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O))
        {
            self.open_setup(workbench_ui::SetupKind::Port);
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::W)) {
            let location = self.selected.as_ref().and_then(|id| {
                self.dock
                    .iter_all_tabs()
                    .find(|(_, tab)| tab.endpoint.id() == id)
                    .and_then(|((surface, node), _)| match &self.dock[surface][node] {
                        Node::Leaf { tabs, .. } => tabs
                            .iter()
                            .position(|tab| tab.endpoint.id() == id)
                            .map(|index| (surface, node, TabIndex(index))),
                        _ => None,
                    })
            });
            if let Some(location) = location {
                if let Some(mut tab) = self.dock.remove_tab(location) {
                    self.config
                        .ports
                        .retain(|settings| settings.path != tab.settings.path);
                    self.config.ports.push(tab.settings.clone());
                    tab.stop_repeat();
                    tab.endpoint.disconnect();
                }
                self.selected = self
                    .dock
                    .iter_all_tabs()
                    .next()
                    .map(|(_, tab)| tab.endpoint.id().clone());
            }
        }
        if !ctx.wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::L))
        {
            if let Some(id) = &self.selected {
                ctx.memory_mut(|memory| {
                    memory.request_focus(egui::Id::new(("terminal-payload", id.0.clone())))
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restored_dock_is_disconnected_and_retains_active_tabs_and_split_ratios() {
        fn saved(path: &str) -> SavedTerminal {
            SavedTerminal {
                settings: SerialSettings {
                    path: path.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }
        let layout = Layout::Split {
            horizontal: true,
            fraction: 0.35,
            first: Box::new(Layout::Leaf {
                tabs: vec![saved("/dev/a")],
                active: 0,
            }),
            second: Box::new(Layout::Leaf {
                tabs: vec![saved("/dev/b"), saved("/dev/c")],
                active: 1,
            }),
        };
        let tree = restore_tree(&layout);
        let snapshot = snapshot(&tree, NodeIndex::root()).unwrap();
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::to_value(layout).unwrap()
        );
        for tab in tree.tabs() {
            assert_eq!(tab.endpoint.state(), ConnectionState::Disconnected);
            assert!(tab.endpoint.send(vec![1]).is_err());
            assert!(tab
                .endpoint
                .start_repeat(
                    vec![1],
                    RepeatSpec {
                        interval: Duration::from_millis(1),
                        count: Some(1)
                    }
                )
                .is_err());
            assert!(tab.repeat.is_none());
            assert!(tab.input.is_empty());
        }
    }
    #[test]
    fn empty_dock_can_be_saved_without_indexing_missing_root() {
        assert!(snapshot(&egui_dock::Tree::new(vec![]), NodeIndex::root()).is_none());
    }
}

#[cfg(test)]
mod receive_display_tests {
    use super::*;
    fn saved() -> SavedTerminal {
        SavedTerminal {
            settings: SerialSettings {
                path: "/dev/test".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    #[test]
    fn mode_switches_keep_exact_history_pending_rx_and_statistics() {
        let mut tab = Terminal::restored(&saved());
        let chunks: &[&[u8]] = &[b"STA", b"TUS=OK\r", b"\nPART"];
        let mut originals = Vec::new();
        for (index, bytes) in chunks.iter().enumerate() {
            let event = Arc::new(TrafficEvent {
                sequence: index as u64 + 1,
                timestamp: UNIX_EPOCH,
                endpoint: tab.endpoint.id().clone(),
                direction: Direction::Rx,
                bytes: Arc::from(*bytes),
            });
            tab.receive(event.clone());
            originals.push(event);
        }
        for mode in [ReceiveMode::Hex, ReceiveMode::RawChunks, ReceiveMode::Line] {
            tab.receive_mode = mode;
            assert_eq!(tab.history.len(), 3);
            assert_eq!(
                tab.rx_bytes,
                chunks.iter().map(|bytes| bytes.len() as u64).sum::<u64>()
            );
            assert_eq!(tab.lines.rows[0].bytes, b"STATUS=OK");
            assert_eq!(tab.lines.pending.as_ref().unwrap().bytes, b"PART");
            for (raw, original) in tab.history.iter().zip(&originals) {
                assert!(Arc::ptr_eq(raw, original));
            }
        }
        assert_eq!(Terminal::restored(&saved()).receive_mode, ReceiveMode::Line);
    }
    #[test]
    fn timing_replay_retains_receipt_settings_and_history_bounds() {
        let mut tab = Terminal::restored(&saved());
        let original = tab.active_framing;
        for sequence in 0..HISTORY_LIMIT as u64 + 3 {
            tab.receive(Arc::new(TrafficEvent {
                sequence,
                timestamp: UNIX_EPOCH + Duration::from_millis(sequence),
                endpoint: tab.endpoint.id().clone(),
                direction: Direction::Rx,
                bytes: Arc::from(b"A\r\n".as_slice()),
            }));
        }
        tab.settings.baud = 9600;
        tab.settings.parity = Parity::Even;
        tab.settings.stop_bits = 2;
        assert_eq!(tab.history.len(), HISTORY_LIMIT);
        assert_eq!(tab.history_framing.len(), HISTORY_LIMIT);
        tab.lines.delimiter = LineDelimiter::CrLf;
        tab.rebuild_lines();
        for row in &tab.lines.rows {
            let timing = row.timing.as_ref().unwrap();
            assert_eq!(timing.framing, original);
            assert_eq!(timing.byte_count, 3);
            assert_eq!(timing.read_count, 1);
            assert!((timing.calculated_wire_seconds().unwrap() - 30.0 / 19200.0).abs() < 1e-12);
        }
        let raw = tab.history.back().unwrap().clone();
        assert_eq!(raw.bytes.as_ref(), b"A\r\n");
        assert_eq!(
            tab.lines
                .rows
                .back()
                .unwrap()
                .timing
                .as_ref()
                .unwrap()
                .last_timestamp,
            raw.timestamp
        );
    }
    #[test]
    fn replay_preserves_framing_changes_and_pause_gaps() {
        let mut tab = Terminal::restored(&saved());
        let event = |sequence, bytes: &[u8]| {
            Arc::new(TrafficEvent {
                sequence,
                timestamp: UNIX_EPOCH + Duration::from_millis(sequence),
                endpoint: EndpointId("serial:/dev/test".into()),
                direction: Direction::Rx,
                bytes: Arc::from(bytes),
            })
        };
        tab.receive(event(1, b"OLD\npartial"));
        tab.paused = true;
        tab.receive(event(2, b"hidden"));
        tab.paused = false;
        tab.settings.baud = 9600;
        tab.settings.parity = Parity::Even;
        tab.settings.stop_bits = 2;
        tab.active_framing = SerialFraming::from(&tab.settings);
        tab.receive(event(3, b"NEW\n"));
        tab.rebuild_lines();
        assert_eq!(tab.lines.rows.len(), 2);
        assert_eq!(tab.lines.rows[0].bytes, b"OLD");
        assert_eq!(tab.lines.rows[1].bytes, b"NEW");
        let old = tab.lines.rows[0].timing.as_ref().unwrap();
        let new = tab.lines.rows[1].timing.as_ref().unwrap();
        assert_eq!(old.framing.baud, 19200);
        assert_eq!(new.framing.baud, 9600);
        assert_eq!(new.byte_count, 4);
        assert_eq!(new.read_count, 1);
        assert_eq!(new.first_timestamp, UNIX_EPOCH + Duration::from_millis(3));
        assert!((new.calculated_wire_seconds().unwrap() - 48.0 / 9600.0).abs() < 1e-12);
        assert_eq!(tab.history_framing.len(), tab.history.len());
    }
    #[test]
    fn modes_and_delimiters_persist_and_legacy_hex_settings_migrate() {
        for mode in [ReceiveMode::Line, ReceiveMode::RawChunks, ReceiveMode::Hex] {
            let mut tab = Terminal::restored(&saved());
            tab.receive_mode = mode;
            tab.lines.delimiter = LineDelimiter::CrLf;
            let serialized = serde_json::to_string(&tab.saved()).unwrap();
            let restored = Terminal::restored(&serde_json::from_str(&serialized).unwrap());
            assert_eq!(restored.receive_mode, mode);
            assert_eq!(restored.lines.delimiter, LineDelimiter::CrLf);
            assert!(restored.lines.is_empty());
        }
        let mut legacy = serde_json::to_value(saved()).unwrap();
        legacy.as_object_mut().unwrap().remove("receive_mode");
        legacy.as_object_mut().unwrap().remove("delimiter");
        legacy["hex"] = true.into();
        let restored = Terminal::restored(&serde_json::from_value(legacy).unwrap());
        assert_eq!(restored.receive_mode, ReceiveMode::Hex);
        assert_eq!(restored.lines.delimiter, LineDelimiter::Auto);
    }
}
