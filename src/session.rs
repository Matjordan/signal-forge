//! Portable debugging sessions stored as ordinary JSON, workspace, and notes files.
use crate::{config::WorkspaceConfig, inspector::timestamp_ns};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

const MAX_NOTES: usize = 1024 * 1024;
const MAX_METADATA: usize = 2 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    RxRecording,
    BridgeCapture,
    TriggeredCapture,
    WorkspaceExport,
    Associated,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    pub path: String,
    pub kind: ArtifactKind,
    pub added_unix_ns: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub created_unix_ns: String,
    pub opened_unix_ns: String,
    pub closed_unix_ns: Option<String>,
    pub workspace: String,
    pub notes: String,
    pub artifacts: Vec<Artifact>,
}
pub struct Session {
    pub root: PathBuf,
    pub metadata: Metadata,
    pub notes: String,
    original_metadata: Vec<u8>,
    original_notes: Vec<u8>,
    original_workspace: Vec<u8>,
}
fn now() -> String {
    timestamp_ns(SystemTime::now()).to_string()
}
fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8], expected: &[u8]) -> Result<(), String> {
    if bounded_read(path, expected.len().max(MAX_METADATA))? != expected {
        return Err(format!(
            "{} changed outside Signal Forge; reopen the session before saving",
            path.display()
        ));
    }
    let parent = path.parent().ok_or("Invalid session path")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}
fn metadata_valid(value: &Metadata) -> Result<(), String> {
    if value.format != "signal-forge-session" || value.version != 1 {
        return Err("Unsupported session format/version".into());
    }
    if value.name.trim().is_empty()
        || value.name.len() > 256
        || value.name.chars().any(char::is_control)
    {
        return Err("Session name must contain 1–256 bytes without control characters".into());
    }
    if value.workspace != "workspace.json"
        || value.notes != "notes.txt"
        || value.artifacts.len() > 4096
    {
        return Err("Invalid session references or artifact count".into());
    }
    for stamp in [&value.created_unix_ns, &value.opened_unix_ns]
        .into_iter()
        .chain(value.closed_unix_ns.iter())
    {
        stamp
            .parse::<i128>()
            .map_err(|_| "Invalid session timestamp")?;
    }
    for artifact in &value.artifacts {
        let path = Path::new(&artifact.path);
        if artifact.path.len() > 4096
            || artifact.path.contains('\0')
            || (!path.is_absolute() && !relative(path))
        {
            return Err("Invalid artifact path".into());
        }
        artifact
            .added_unix_ns
            .parse::<i128>()
            .map_err(|_| "Invalid artifact timestamp")?;
    }
    Ok(())
}
impl Session {
    pub fn create(folder: &Path, name: &str, workspace: &WorkspaceConfig) -> Result<Self, String> {
        workspace.validate()?;
        let metadata = Metadata {
            format: "signal-forge-session".into(),
            version: 1,
            name: name.trim().into(),
            created_unix_ns: now(),
            opened_unix_ns: now(),
            closed_unix_ns: None,
            workspace: "workspace.json".into(),
            notes: "notes.txt".into(),
            artifacts: Vec::new(),
        };
        metadata_valid(&metadata)?;
        if folder.exists() {
            if fs::read_dir(folder)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
            {
                return Err(
                    "Choose a new or empty session folder; existing files are preserved".into(),
                );
            }
        } else {
            fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        let root = fs::canonicalize(folder).map_err(|e| e.to_string())?;
        let original_workspace =
            serde_json::to_vec_pretty(&workspace_paths(workspace.clone(), &root, false)?)
                .map_err(|e| e.to_string())?;
        let original_metadata = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?;
        let files = [
            ("notes.txt", &[][..]),
            ("workspace.json", original_workspace.as_slice()),
            ("session.json", original_metadata.as_slice()),
        ];
        let mut created = Vec::new();
        for (name, bytes) in files {
            let path = root.join(name);
            let result = (|| {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                created.push(path.clone());
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())
            })();
            if let Err(error) = result {
                for path in created {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
        }
        fs::File::open(&root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            metadata,
            notes: String::new(),
            original_metadata,
            original_notes: Vec::new(),
            original_workspace,
        })
    }
    /// Validates every session file before the caller replaces its workspace.
    pub fn open(folder: &Path) -> Result<(Self, WorkspaceConfig), String> {
        let root = fs::canonicalize(folder).map_err(|e| e.to_string())?;
        let original_metadata = bounded_read(&root.join("session.json"), MAX_METADATA)?;
        let mut metadata: Metadata =
            serde_json::from_slice(&original_metadata).map_err(|e| e.to_string())?;
        metadata_valid(&metadata)?;
        let original_notes = bounded_read(&root.join("notes.txt"), MAX_NOTES)?;
        let notes =
            String::from_utf8(original_notes.clone()).map_err(|_| "Session notes must be UTF-8")?;
        let original_workspace = bounded_read(&root.join("workspace.json"), MAX_METADATA)?;
        let workspace = WorkspaceConfig::parse(
            std::str::from_utf8(&original_workspace).map_err(|_| "Workspace must be UTF-8")?,
        )?;
        let workspace = workspace_paths(workspace, &root, true)?;
        metadata.opened_unix_ns = now();
        metadata.closed_unix_ns = None;
        let mut session = Self {
            root,
            metadata,
            notes,
            original_metadata,
            original_notes,
            original_workspace,
        };
        session.save_metadata()?;
        Ok((session, workspace))
    }
    pub fn save_notes(&mut self) -> Result<(), String> {
        if self.notes.len() > MAX_NOTES {
            return Err("Session notes exceed 1 MiB".into());
        }
        if self.notes.as_bytes() == self.original_notes {
            return Ok(());
        }
        atomic_write(
            &self.root.join("notes.txt"),
            self.notes.as_bytes(),
            &self.original_notes,
        )?;
        self.original_notes = self.notes.as_bytes().to_vec();
        Ok(())
    }
    fn save_metadata(&mut self) -> Result<(), String> {
        metadata_valid(&self.metadata)?;
        let bytes = serde_json::to_vec_pretty(&self.metadata).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_METADATA {
            return Err("Session metadata exceeds 2 MiB".into());
        }
        atomic_write(
            &self.root.join("session.json"),
            &bytes,
            &self.original_metadata,
        )?;
        self.original_metadata = bytes;
        Ok(())
    }
    pub fn save_context(
        &mut self,
        workspace: &WorkspaceConfig,
        closed: bool,
    ) -> Result<(), String> {
        self.save_notes()?;
        let workspace = workspace_paths(workspace.clone(), &self.root, false)?;
        let bytes = serde_json::to_vec_pretty(&workspace).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_METADATA {
            return Err("Session workspace exceeds 2 MiB".into());
        }
        if bytes != self.original_workspace {
            atomic_write(
                &self.root.join("workspace.json"),
                &bytes,
                &self.original_workspace,
            )?;
            self.original_workspace = bytes;
        }
        self.metadata.closed_unix_ns = if closed { Some(now()) } else { None };
        self.save_metadata()
    }
    pub fn register(&mut self, path: &Path, kind: ArtifactKind) -> Result<(), String> {
        let path = fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !path.is_file() {
            return Err("Artifacts must be regular files".into());
        }
        let portable = path
            .strip_prefix(&self.root)
            .unwrap_or(&path)
            .to_str()
            .ok_or("Artifact path must be UTF-8")?
            .to_owned();
        if self
            .metadata
            .artifacts
            .iter()
            .any(|artifact| artifact.path == portable)
        {
            return Ok(());
        }
        if self.metadata.artifacts.len() >= 4096 {
            return Err("Session index is limited to 4096 artifacts".into());
        }
        self.metadata.artifacts.push(Artifact {
            path: portable,
            kind,
            added_unix_ns: now(),
        });
        if let Err(error) = self.save_metadata() {
            self.metadata.artifacts.pop();
            return Err(error);
        }
        Ok(())
    }
    pub fn artifact_path(&self, artifact: &Artifact) -> PathBuf {
        self.root.join(&artifact.path)
    }
    /// Returns a collision-free suggestion; the writer still uses create_new.
    pub fn suggested_path(&self, stem: &str, extension: &str) -> PathBuf {
        for index in 1..=10000 {
            let path = self.root.join(format!("{stem}-{index}.{extension}"));
            if !path.exists() {
                return path;
            }
        }
        self.root.join(format!("{stem}-{}.{}", now(), extension))
    }
}
/// Only replay files within the session become relative; device/SSH identities remain unchanged.
fn workspace_paths(
    mut workspace: WorkspaceConfig,
    root: &Path,
    expand: bool,
) -> Result<WorkspaceConfig, String> {
    fn convert(path: &mut String, root: &Path, expand: bool) -> Result<(), String> {
        if !path.starts_with("replay://") {
            return Ok(());
        }
        let mut config = crate::replay::ReplayConfig::parse(path)?;
        let source = Path::new(&config.path);
        if expand && !source.is_absolute() {
            if !relative(source) {
                return Err("Invalid relative replay path in session".into());
            }
            config.path = root
                .join(source)
                .to_str()
                .ok_or("Session path must be UTF-8")?
                .into();
        } else if !expand && source.is_absolute() {
            if let Ok(short) = source.strip_prefix(root) {
                config.path = short.to_str().ok_or("Replay path must be UTF-8")?.into();
            }
        }
        *path = config.uri();
        Ok(())
    }
    fn layout(
        value: &mut crate::workspace::Layout,
        root: &Path,
        expand: bool,
    ) -> Result<(), String> {
        match value {
            crate::workspace::Layout::Leaf { tabs, .. } => {
                for tab in tabs {
                    convert(&mut tab.settings.path, root, expand)?;
                }
            }
            crate::workspace::Layout::Split { first, second, .. } => {
                layout(first, root, expand)?;
                layout(second, root, expand)?;
            }
        }
        Ok(())
    }
    for settings in &mut workspace.ports {
        convert(&mut settings.path, root, expand)?;
    }
    if let Some(path) = &mut workspace.selected {
        convert(path, root, expand)?;
    }
    if let Some(value) = &mut workspace.layout {
        layout(value, root, expand)?;
    }
    for window in &mut workspace.windows {
        layout(&mut window.layout, root, expand)?;
    }
    workspace.validate()?;
    Ok(workspace)
}
