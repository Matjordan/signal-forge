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
pub struct WorkspaceConfig {
    pub version: u32,
    pub ports: Vec<SerialSettings>,
}
impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            version: 1,
            ports: Vec::new(),
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
        match fs::read_to_string(&path) {
            Ok(text) => {
                let config: Self = serde_json::from_str(&text)
                    .map_err(|e| format!("{}: {e}. Original file preserved.", path.display()))?;
                if config.version != 1 {
                    return Err(format!(
                        "Unsupported workspace version {}. Original file preserved.",
                        config.version
                    ));
                }
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("No configuration directory available")?;
        let parent = path.parent().ok_or("Invalid configuration path")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text).map_err(|e| e.to_string())?;
        fs::rename(temporary, path).map_err(|e| e.to_string())
    }
}
