//! Transactional host edits preserve terminal IDs and all workspace references.
use crate::{
    config::WorkspaceConfig,
    ssh_serial::SavedSshHost,
    workspace::{Layout, SavedTerminal},
};
pub fn terminals_mut(layout: &mut Layout, action: &mut impl FnMut(&mut SavedTerminal)) {
    match layout {
        Layout::Leaf { tabs, .. } => tabs.iter_mut().for_each(action),
        Layout::Split { first, second, .. } => {
            terminals_mut(first, action);
            terminals_mut(second, action);
        }
    }
}
pub fn edit_host(
    config: &WorkspaceConfig,
    old_name: &str,
    replacement: SavedSshHost,
) -> Result<WorkspaceConfig, String> {
    let old = config
        .remote_hosts
        .iter()
        .find(|h| h.name == old_name)
        .ok_or("Saved host no longer exists")?;
    let mut result = config.clone();
    let rewrite = |path: &str| {
        crate::ssh_serial::SshHost::parse(path)
            .ok()
            .and_then(|(_, device)| {
                (old.connection.uri(&device) == path).then(|| replacement.connection.uri(&device))
            })
    };
    // Explicit links disambiguate saved hosts that share an SSH address.
    let mut foreign_paths = std::collections::HashSet::new();
    let mut foreign = |tab: &mut SavedTerminal| {
        if tab
            .remote_host
            .as_deref()
            .is_some_and(|name| name != old_name)
        {
            foreign_paths.insert(tab.settings.path.clone());
        }
    };
    if let Some(mut layout) = config.layout.clone() {
        terminals_mut(&mut layout, &mut foreign);
    }
    for mut window in config.windows.clone() {
        terminals_mut(&mut window.layout, &mut foreign);
    }
    for port in &mut result.ports {
        if foreign_paths.contains(&port.path) {
            continue;
        }
        if let Some(path) = rewrite(&port.path) {
            port.path = path;
        }
    }
    if let Some(path) = result
        .selected
        .as_ref()
        .filter(|path| !foreign_paths.contains(*path))
        .and_then(|path| rewrite(path))
    {
        result.selected = Some(path);
    }
    let mut update = |tab: &mut SavedTerminal| {
        if tab.remote_host.as_deref() == Some(old_name)
            || (tab.remote_host.is_none() && rewrite(&tab.settings.path).is_some())
        {
            let id = tab
                .endpoint_id
                .clone()
                .unwrap_or_else(|| format!("serial:{}", tab.settings.path));
            if let Ok((_, device)) = crate::ssh_serial::SshHost::parse(&tab.settings.path) {
                tab.settings.path = replacement.connection.uri(&device);
            }
            tab.endpoint_id = Some(id);
            tab.remote_host = Some(replacement.name.clone());
        }
    };
    if let Some(layout) = &mut result.layout {
        terminals_mut(layout, &mut update);
    }
    for window in &mut result.windows {
        terminals_mut(&mut window.layout, &mut update);
    }
    *result
        .remote_hosts
        .iter_mut()
        .find(|h| h.name == old_name)
        .unwrap() = replacement;
    result.validate()?;
    Ok(result)
}

/// Reusing an old address must not steal an edited endpoint's persistent identity.
pub fn available_endpoint_id(
    path: &str,
    used: &[crate::endpoint::EndpointId],
) -> crate::endpoint::EndpointId {
    let base = format!("serial:{path}");
    let mut id = crate::endpoint::EndpointId(base.clone());
    for number in 2u64.. {
        if !used.contains(&id) {
            return id;
        }
        id = crate::endpoint::EndpointId(format!("{base}#{number}"));
    }
    unreachable!("more endpoint identities than supported workspace capacity")
}
