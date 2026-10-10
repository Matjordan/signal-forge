use crate::{
    config::WorkspaceConfig,
    repeat::RepeatSpec,
    send::{self, Encoding, LineEnding},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetTarget {
    Selected,
    Endpoint(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatSettings {
    pub interval_ms: u64,
    pub count: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub payload: String,
    pub encoding: Encoding,
    pub escapes: bool,
    pub ending: LineEnding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<crate::send::Checksum>,
    pub target: PresetTarget,
    pub repeat: Option<RepeatSettings>,
    pub description: String,
    /// Ctrl+1 through Ctrl+9, scoped to the selected profile.
    pub shortcut: String,
}
impl Default for Preset {
    fn default() -> Self {
        Self {
            name: "New command".into(),
            payload: String::new(),
            encoding: Encoding::Text,
            escapes: true,
            ending: LineEnding::None,
            checksum: None,
            target: PresetTarget::Selected,
            repeat: None,
            description: String::new(),
            shortcut: String::new(),
        }
    }
}
impl Preset {
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        let bytes = send::encode_with_checksum(
            &self.payload,
            self.encoding,
            self.escapes,
            self.ending,
            self.checksum,
        )
        .map_err(|e| e.to_string())?;
        if bytes.is_empty() || bytes.len() > 65536 {
            return Err("Preset payload must contain 1–65536 bytes".into());
        }
        Ok(bytes)
    }
    pub fn repeat_spec(&self) -> Option<RepeatSpec> {
        self.repeat.as_ref().map(|settings| RepeatSpec {
            interval: Duration::from_millis(settings.interval_ms),
            count: settings.count,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Preset name is required".into());
        }
        self.bytes()?;
        if let PresetTarget::Endpoint(id) = &self.target {
            if id.trim().is_empty() {
                return Err("Target endpoint ID is required".into());
            }
        }
        if !self.shortcut.is_empty()
            && !matches!(
                self.shortcut.strip_prefix("Ctrl+"),
                Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            )
        {
            return Err("Shortcut must be Ctrl+1 through Ctrl+9, or empty".into());
        }
        if let Some(spec) = self.repeat_spec() {
            spec.validate().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub presets: Vec<Preset>,
}
impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Profile name is required".into());
        }
        let mut shortcuts = std::collections::HashSet::new();
        for preset in &self.presets {
            preset.validate()?;
            if !preset.shortcut.is_empty() && !shortcuts.insert(&preset.shortcut) {
                return Err(format!("Duplicate shortcut: {}", preset.shortcut));
            }
        }
        Ok(())
    }
    pub fn export(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        atomic_json(
            path,
            &ProfileFile {
                version: 1,
                profile: self.clone(),
            },
        )
    }
    pub fn import(path: &Path) -> Result<Self, String> {
        let file: ProfileFile = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if file.version != 1 {
            return Err(format!("Unsupported profile version {}", file.version));
        }
        file.profile.validate()?;
        Ok(file.profile)
    }
}
#[derive(Serialize, Deserialize)]
struct ProfileFile {
    version: u32,
    profile: Profile,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetLibrary {
    pub version: u32,
    pub profiles: Vec<Profile>,
}
impl Default for PresetLibrary {
    fn default() -> Self {
        Self {
            version: 1,
            profiles: vec![Profile {
                name: "General".into(),
                presets: Vec::new(),
            }],
        }
    }
}
impl PresetLibrary {
    pub fn load() -> Result<Self, String> {
        let Some(path) = WorkspaceConfig::path().map(|p| p.with_file_name("presets.json")) else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::read_from(&path)
    }
    pub fn read_from(path: &Path) -> Result<Self, String> {
        let library: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Preset library is invalid: {e}. Original file preserved."))?;
        library.validate()?;
        Ok(library)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "Unsupported preset library version {}",
                self.version
            ));
        }
        if self.profiles.is_empty() {
            return Err("A preset library must contain at least one profile".into());
        }
        let mut names = std::collections::HashSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !names.insert(&profile.name) {
                return Err(format!("Duplicate profile name: {}", profile.name));
            }
        }
        Ok(())
    }
    pub fn save(&self) -> Result<(), String> {
        let path = WorkspaceConfig::path()
            .ok_or("No configuration directory available")?
            .with_file_name("presets.json");
        self.write_to(&path)
    }
    pub fn write_to(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        atomic_json(path, self)
    }
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_round_trip_preserves_every_send_setting_and_library_order() {
        let path =
            std::env::temp_dir().join(format!("signal-forge-profile-{}.json", std::process::id()));
        let preset = Preset {
            name: "Binary poll".into(),
            payload: "00 FF".into(),
            encoding: Encoding::Hex,
            escapes: false,
            ending: LineEnding::CrLf,
            checksum: None,
            target: PresetTarget::Endpoint("serial:/dev/ttyUSB0".into()),
            repeat: Some(RepeatSettings {
                interval_ms: 20,
                count: Some(3),
            }),
            description: "Read status".into(),
            shortcut: "Ctrl+1".into(),
        };
        assert_eq!(preset.bytes().unwrap(), [0, 255, 13, 10]);
        let profile = Profile {
            name: "Bench".into(),
            presets: vec![
                preset.clone(),
                Preset {
                    name: "Stop".into(),
                    payload: "STOP\\r\\n".into(),
                    ..Default::default()
                },
            ],
        };
        profile.export(&path).unwrap();
        assert_eq!(Profile::import(&path).unwrap(), profile);
        let library = PresetLibrary {
            version: 1,
            profiles: vec![profile],
        };
        library.write_to(&path).unwrap();
        assert_eq!(PresetLibrary::read_from(&path).unwrap(), library);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_import_and_duplicate_shortcuts_are_rejected() {
        let preset = Preset {
            payload: "valid\\xZZ".into(),
            ..Default::default()
        };
        assert!(preset.validate().is_err());
        let preset = Preset {
            payload: "PING".into(),
            shortcut: "Ctrl+1".into(),
            ..Default::default()
        };
        let profile = Profile {
            name: "Bench".into(),
            presets: vec![preset.clone(), preset],
        };
        assert!(profile.validate().is_err());
        let library = PresetLibrary {
            version: 99,
            ..Default::default()
        };
        assert!(library.validate().is_err());
    }
}
