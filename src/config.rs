use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dotenvy::Error as DotenvError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub openclaw: OpenClawConfig,
    pub elevenlabs: ElevenLabsConfig,
    pub audio: AudioConfig,
    pub speech: SpeechConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawConfig {
    pub gateway_url: String,
    pub gateway_token: String,
    #[serde(default)]
    pub session_filter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    pub voice_id: String,
    #[serde(default = "default_model_id")]
    pub model_id: String,
    #[serde(default = "default_output_format")]
    pub output_format: String,
    #[serde(default)]
    pub language_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default)]
    pub output_device: String,
    #[serde(default = "default_sample_rate_hz")]
    pub sample_rate_hz: u32,
    #[serde(default = "default_channel_count")]
    pub channel_count: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechConfig {
    #[serde(default = "default_interrupt_on_new_reply")]
    pub interrupt_on_new_reply: bool,
    #[serde(default = "default_dedupe_window_ms")]
    pub dedupe_window_ms: u64,
}

impl AppConfig {
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        load_dotenv()?;

        let path = resolve_config_path(explicit_path);
        let mut config = if let Some(path) = path {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read config file {}", path.display()))?;
            toml::from_str::<AppConfig>(&raw)
                .with_context(|| format!("failed to parse config file {}", path.display()))?
        } else {
            Self::default()
        };

        config.apply_env_overrides();
        Ok(config)
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(value) = env::var("OPENCLAW_GATEWAY_URL") {
            self.openclaw.gateway_url = value;
        }
        if let Ok(value) = env::var("OPENCLAW_GATEWAY_TOKEN") {
            self.openclaw.gateway_token = value;
        }
        if let Ok(value) = env::var("OPENCLAW_SESSION_FILTER") {
            self.openclaw.session_filter = value;
        }
        if let Ok(value) = env::var("ELEVENLABS_API_KEY") {
            self.elevenlabs.api_key = value;
        }
        if let Ok(value) = env::var("ELEVENLABS_VOICE_ID") {
            self.elevenlabs.voice_id = value;
        }
        if let Ok(value) = env::var("ELEVENLABS_MODEL_ID") {
            self.elevenlabs.model_id = value;
        }
        if let Ok(value) = env::var("ELEVENLABS_OUTPUT_FORMAT") {
            self.elevenlabs.output_format = value;
        }
        if let Ok(value) = env::var("ELEVENLABS_LANGUAGE_CODE") {
            self.elevenlabs.language_code = Some(value);
        }
        if let Ok(value) = env::var("AUDIO_OUTPUT_DEVICE") {
            self.audio.output_device = value;
        }
    }
}

fn load_dotenv() -> Result<()> {
    match dotenvy::from_filename(".env") {
        Ok(path) => {
            tracing::info!("loaded environment overrides from {}", path.display());
            Ok(())
        }
        Err(DotenvError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed to load .env file"),
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            openclaw: OpenClawConfig {
                gateway_url: "ws://127.0.0.1:18789".to_string(),
                gateway_token: String::new(),
                session_filter: String::new(),
            },
            elevenlabs: ElevenLabsConfig {
                api_key: String::new(),
                voice_id: String::new(),
                model_id: default_model_id(),
                output_format: default_output_format(),
                language_code: None,
            },
            audio: AudioConfig {
                output_device: String::new(),
                sample_rate_hz: default_sample_rate_hz(),
                channel_count: default_channel_count(),
            },
            speech: SpeechConfig {
                interrupt_on_new_reply: default_interrupt_on_new_reply(),
                dedupe_window_ms: default_dedupe_window_ms(),
            },
        }
    }
}

fn resolve_config_path(explicit_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit_path {
        return Some(path.to_path_buf());
    }

    if let Some(config_dir) = dirs::config_dir() {
        let path = config_dir.join("openclaw-speak").join("config.toml");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn default_model_id() -> String {
    "eleven_multilingual_v2".to_string()
}

fn default_output_format() -> String {
    "pcm_24000".to_string()
}

const fn default_sample_rate_hz() -> u32 {
    44_100
}

const fn default_channel_count() -> u16 {
    1
}

const fn default_interrupt_on_new_reply() -> bool {
    true
}

const fn default_dedupe_window_ms() -> u64 {
    5_000
}
