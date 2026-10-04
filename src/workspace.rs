//! Versioned, transport-free workspace descriptions. Restoring these never opens a device.
use crate::{config::SerialSettings, send::{Encoding, LineEnding}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedTerminal {
    pub settings: SerialSettings,
    pub hex: bool,
    pub timestamps: bool,
    pub auto_scroll: bool,
    pub encoding: Encoding,
    pub escapes: bool,
    pub ending: LineEnding,
}
impl Default for SavedTerminal {
    fn default() -> Self {
        Self { settings: SerialSettings::default(), hex: false, timestamps: true,
            auto_scroll: true, encoding: Encoding::Text, escapes: true, ending: LineEnding::None }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Layout {
    Leaf { tabs: Vec<SavedTerminal>, active: usize },
    Split { horizontal: bool, fraction: f32, first: Box<Layout>, second: Box<Layout> },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedWindow {
    pub layout: Layout,
    pub position: [f32; 2],
    pub size: [f32; 2],
}
impl Layout {
    pub fn validate(&self, depth: usize, paths: &mut std::collections::HashSet<String>) -> Result<(), String> {
        if depth > 16 { return Err("Workspace layout exceeds 16 levels".into()); }
        match self {
            Self::Leaf { tabs, active } => {
                if tabs.is_empty() || tabs.len() > 64 || *active >= tabs.len() {
                    return Err("Invalid terminal count or active tab".into());
                }
                for tab in tabs {
                    tab.settings.validate()?;
                    if !paths.insert(tab.settings.path.clone()) { return Err(format!("Duplicate terminal {}", tab.settings.path)); }
                }
            }
            Self::Split { fraction, first, second, .. } => {
                if !fraction.is_finite() || !(0.05..=0.95).contains(fraction) { return Err("Invalid workspace split fraction".into()); }
                first.validate(depth + 1, paths)?;
                second.validate(depth + 1, paths)?;
            }
        }
        if paths.len() > 64 { return Err("Workspace exceeds 64 terminals".into()); }
        Ok(())
    }
}
