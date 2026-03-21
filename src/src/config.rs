use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const PRODUCT_NAME: &str = "Aitify";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub silent_start: bool,
}

fn default_language() -> String { "zh-CN".to_string() }

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: default_language(),
            autostart: false,
            silent_start: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub desktop: DesktopConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

impl Default for DesktopConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourcesConfig {
    #[serde(default)]
    pub claude: SourceConfig,
    #[serde(default)]
    pub codex: SourceConfig,
    #[serde(default)]
    pub gemini: SourceConfig,
    #[serde(default)]
    pub qwen: SourceConfig,
    #[serde(default)]
    pub opencode: SourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub min_duration_minutes: i32,
    #[serde(default)]
    pub channels: SourceChannelsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceChannelsConfig {
    #[serde(default = "default_true")]
    pub desktop: bool,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_duration_minutes: 0,
            channels: SourceChannelsConfig::default(),
        }
    }
}

impl Default for SourceChannelsConfig {
    fn default() -> Self {
        Self { desktop: true }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 2,
            ui: UiConfig::default(),
            channels: ChannelsConfig::default(),
            sources: SourcesConfig::default(),
        }
    }
}

pub fn get_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AITIFY_DATA_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(app_data) = std::env::var("APPDATA") {
            return PathBuf::from(app_data).join(PRODUCT_NAME);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(PRODUCT_NAME);
        }
    }

    // Linux or fallback
    if let Ok(home) = std::env::var("HOME") {
        let product_name_lower = PRODUCT_NAME.to_lowercase();
        return PathBuf::from(home).join(format!(".config/{}", product_name_lower));
    }

    PathBuf::from(".")
}

pub fn get_settings_path() -> PathBuf {
    get_data_dir().join("settings.json")
}

pub fn load_config() -> Result<AppConfig, Box<dyn std::error::Error>> {
    let path = get_settings_path();

    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let content = fs::read_to_string(&path)?;
    let config: AppConfig = serde_json::from_str(&content)?;
    Ok(config)
}

pub fn save_config(config: &AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    let dir = get_data_dir();
    fs::create_dir_all(&dir)?;

    let path = get_settings_path();
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&path, content)?;

    Ok(())
}

pub fn get_config_path() -> PathBuf {
    get_settings_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_defaults_enable_all_sources_and_desktop_channels() {
        let config = AppConfig::default();

        assert!(config.sources.claude.enabled);
        assert!(config.sources.codex.enabled);
        assert!(config.sources.gemini.enabled);
        assert!(config.sources.qwen.enabled);
        assert!(config.sources.opencode.enabled);

        assert!(config.sources.claude.channels.desktop);
        assert!(config.sources.codex.channels.desktop);
        assert!(config.sources.gemini.channels.desktop);
        assert!(config.sources.qwen.channels.desktop);
        assert!(config.sources.opencode.channels.desktop);
    }
}
