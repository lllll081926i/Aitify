use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const PRODUCT_NAME: &str = "ai-cli-complete-notify";

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiConfig {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub autostart: bool,
}

fn default_language() -> String { "zh-CN".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub desktop: DesktopConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourcesConfig {
    #[serde(default)]
    pub claude: SourceConfig,
    #[serde(default)]
    pub codex: SourceConfig,
    #[serde(default)]
    pub gemini: SourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub min_duration_minutes: i32,
    #[serde(default)]
    pub channels: SourceChannelsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceChannelsConfig {
    #[serde(default = "default_true")]
    pub desktop: bool,
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
    if let Ok(dir) = std::env::var("AI_CLI_COMPLETE_NOTIFY_DATA_DIR") {
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
