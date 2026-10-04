use super::*;
use egui_dock::{Node, Surface, SurfaceIndex, TabIndex};
use signal_forge::workspace::{Layout, SavedTerminal, SavedWindow};

struct RestoredEndpoint {
    id: EndpointId,
    path: String,
}
impl Endpoint for RestoredEndpoint {
    fn id(&self) -> &EndpointId {
        &self.id
    }
    fn display_name(&self) -> &str {
        &self.path
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
    fn saved(&self) -> SavedTerminal {
        SavedTerminal {
            settings: self.settings.clone(),
            hex: self.hex,
            timestamps: self.timestamps,
            auto_scroll: self.auto_scroll,
            encoding: self.encoding,
            escapes: self.escapes,
            ending: self.ending,
        }
    }
    fn restored(saved: &SavedTerminal) -> Self {
        let endpoint = RestoredEndpoint {
            id: EndpointId(format!("serial:{}", saved.settings.path)),
            path: saved.settings.path.clone(),
        };
        let mut tab = Self::new(endpoint, saved.settings.clone());
        tab.hex = saved.hex;
        tab.timestamps = saved.timestamps;
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
    pub(super) fn save_workspace(&mut self) {
        if !self.config_recoverable {
            return;
        }
        for (_, tab) in self.dock.iter_all_tabs() {
            self.config
                .ports
                .retain(|settings| settings.path != tab.settings.path);
            self.config.ports.push(tab.settings.clone());
        }
        self.config.ports.truncate(256);
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
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::W)) {
            let location = self.selected.as_ref().and_then(|id| {
                self.dock
                    .iter_all_tabs()
                    .find(|(_, tab)| tab.endpoint.id() == id)
                    .and_then(|((surface, node), _)| match &self.dock[surface][node] {
                        Node::Leaf { tabs, .. } => tabs.iter().position(|tab| tab.endpoint.id() == id).map(|index| (surface, node, TabIndex(index))),
                        _ => None,
                    })
            });
            if let Some(location) = location {
                if let Some(mut tab) = self.dock.remove_tab(location) {
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
