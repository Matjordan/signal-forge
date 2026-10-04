use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Parity {
    None,
    Odd,
    Even,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowControl {
    None,
    Hardware,
    Software,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialSettings {
    pub path: String,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow: FlowControl,
}
impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            path: String::new(),
            baud: 115200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            flow: FlowControl::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub version: u32,
    pub ports: Vec<SerialSettings>,
    pub layout: Option<crate::workspace::Layout>,
    pub windows: Vec<crate::workspace::SavedWindow>,
    pub profile: Option<String>,
    pub selected: Option<String>,
}
impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            version: 2,
            ports: Vec::new(),
            layout: None,
            windows: Vec::new(),
            profile: None,
            selected: None,
        }
    }
}
impl WorkspaceConfig {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .map(|p| p.join("signal-forge/workspace.json"))
    }
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        Self::load_from(&path)
    }
    pub fn load_from(path: &std::path::Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| {
                format!(
                    "{}: {e}. Original file preserved; saving disabled until recovery.",
                    path.display()
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if value.get("version").and_then(|version| version.as_u64()).is_none() {
            return Err("Workspace has no valid version".into());
        }
        let mut config: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if !matches!(config.version, 1 | 2) {
            return Err(format!("Unsupported workspace version {}", config.version));
        }
        config.version = 2;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 2 || self.ports.len() > 256 || self.windows.len() > 16 {
            return Err("Invalid workspace version or size".into());
        }
        for port in &self.ports {
            port.validate()?;
        }
        let mut paths = std::collections::HashSet::new();
        if let Some(layout) = &self.layout {
            layout.validate(0, &mut paths)?;
        }
        for window in &self.windows {
            if !window.position.iter().all(|v| v.is_finite())
                || !window
                    .size
                    .iter()
                    .all(|v| v.is_finite() && *v >= 100.0 && *v <= 16000.0)
            {
                return Err("Invalid floating window geometry".into());
            }
            window.layout.validate(0, &mut paths)?;
        }
        Ok(())
    }
    /// Preserve the entire original, including any recoverable fields, before resetting.
    pub fn backup_for_recovery() -> Result<PathBuf, String> {
        let path = Self::path().ok_or("No configuration directory available")?;
        for index in 0..10000 {
            let backup = path.with_extension(format!("json.recovery-{index}"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&fs::read(&path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                    file.sync_all().map_err(|e| e.to_string())?;
                    fs::remove_file(&path).map_err(|e| e.to_string())?;
                    return Ok(backup);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("No recovery filename available".into())
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("No configuration directory available")?;
        self.save_to(&path)
    }
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        self.validate()?;
        let parent = path.parent().ok_or("Invalid configuration path")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let temporary = path.with_extension(format!("json.{}.{stamp}.tmp", std::process::id()));
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("{}: {e}", temporary.display()))?;
        if let Err(error) = file.write_all(&text).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
        drop(file);
        fs::rename(&temporary, path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            e.to_string()
        })?;
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())
    }
}

impl SerialSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.path.is_empty()
            || self.path.len() > 4096
            || self.path.contains('\0')
            || self.baud == 0
            || !(5..=8).contains(&self.data_bits)
            || !(1..=2).contains(&self.stop_bits)
        {
            Err(format!("Invalid serial settings for {}", self.path))
        } else {
            Ok(())
        }
    }
}
