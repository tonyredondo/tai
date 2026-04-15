use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct TaiConfig {
    pub ai: AiConfig,
    pub terminal: TerminalConfig,
    pub keybindings: KeybindingsConfig,
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct AiConfig {
    pub model: String,
    pub api_key: String,
    pub auto_execute: bool,
    pub max_context_lines: usize,
    pub max_history: usize,
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct TerminalConfig {
    pub font_size: i32,
    pub scrollback: u32,
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct KeybindingsConfig {
    pub ai_toggle: String,
}

impl Default for TaiConfig {
    fn default() -> Self {
        Self {
            ai: AiConfig::default(),
            terminal: TerminalConfig::default(),
            keybindings: KeybindingsConfig::default(),
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            model: "gpt-5.4".to_string(),
            api_key: String::new(),
            auto_execute: false,
            max_context_lines: 100,
            max_history: 20,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            font_size: 16,
            scrollback: 10000000,
        }
    }
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            ai_toggle: "ctrl+/".to_string(),
        }
    }
}

impl TaiConfig {
    pub fn load() -> Self {
        let config_path = Self::config_path();
        if let Some(path) = config_path {
            if path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(config) = toml::from_str::<TaiConfig>(&contents) {
                        return config;
                    }
                    eprintln!("TAI: Failed to parse config at {}, using defaults", path.display());
                }
            }
        }
        TaiConfig::default()
    }

    pub fn api_key(&self) -> Option<String> {
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                return Some(key);
            }
        }
        if !self.ai.api_key.is_empty() {
            return Some(self.ai.api_key.clone());
        }
        None
    }

    pub fn ai_enabled(&self) -> bool {
        self.api_key().is_some()
    }

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("tai").join("config.toml"))
    }
}
