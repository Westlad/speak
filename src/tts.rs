use anyhow::{Context, Result, bail};
use bytes::Bytes;
use reqwest::Client;
use serde_json::json;

use crate::config::ElevenLabsConfig;

#[derive(Debug, Clone)]
pub struct ElevenLabsClient {
    config: ElevenLabsConfig,
    http: Client,
}

impl ElevenLabsClient {
    pub fn new(config: ElevenLabsConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    pub fn describe_configuration(&self) -> Result<()> {
        if self.config.voice_id.is_empty() {
            tracing::warn!("ElevenLabs voice_id is not configured yet");
        }

        if self.config.api_key.is_empty() {
            tracing::warn!("ElevenLabs api_key is not configured yet");
        }

        tracing::info!(
            "ElevenLabs client configured with model {} and output format {}",
            self.config.model_id,
            self.config.output_format
        );
        Ok(())
    }

    pub async fn synthesize_preview(&self, text: &str) -> Result<SpeechPreview> {
        if text.trim().is_empty() {
            bail!("test text cannot be empty");
        }

        if self.config.api_key.trim().is_empty() {
            bail!("ELEVENLABS_API_KEY is not configured");
        }

        if self.config.voice_id.trim().is_empty() {
            bail!("ELEVENLABS_VOICE_ID is not configured");
        }

        tracing::info!(
            "preview synthesis requested for {} characters using model {}",
            text.len(),
            self.config.model_id
        );

        let url = format!(
            "https://api.elevenlabs.io/v1/text-to-speech/{}",
            self.config.voice_id
        );

        let mut body = json!({
            "text": text,
            "model_id": self.config.model_id,
        });
        if let Some(language_code) = self
            .config
            .language_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body["language_code"] = json!(language_code);
        }

        let response = self
            .http
            .post(url)
            .query(&[("output_format", self.config.output_format.as_str())])
            .header("xi-api-key", &self.config.api_key)
            .json(&body)
            .send()
            .await
            .context("failed to call ElevenLabs text-to-speech endpoint")?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unavailable>".to_string());
            bail!("ElevenLabs request failed with {status}: {error_body}");
        }

        let audio_bytes = response
            .bytes()
            .await
            .context("failed to read ElevenLabs audio response")?;

        let audio_format = AudioFormat::from_output_format(&self.config.output_format)?;

        Ok(SpeechPreview {
            text: text.to_string(),
            audio_bytes,
            audio_format,
            sample_rate_hz: infer_sample_rate_hz(&self.config.output_format)?,
            channel_count: 1,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SpeechPreview {
    pub text: String,
    pub audio_bytes: Bytes,
    pub audio_format: AudioFormat,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
}

#[derive(Debug, Clone)]
pub enum AudioFormat {
    PcmS16Le,
    Encoded(String),
}

impl AudioFormat {
    fn from_output_format(output_format: &str) -> Result<Self> {
        let codec = output_format
            .split('_')
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid ElevenLabs output_format: {output_format}"))?;

        if codec.eq_ignore_ascii_case("pcm") {
            Ok(Self::PcmS16Le)
        } else {
            Ok(Self::Encoded(codec.to_string()))
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::PcmS16Le => "pcm_s16le",
            Self::Encoded(codec) => codec.as_str(),
        }
    }
}

fn infer_sample_rate_hz(output_format: &str) -> Result<u32> {
    let sample_rate = output_format
        .split('_')
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("output_format is missing sample rate: {output_format}"))?;

    sample_rate.parse::<u32>().with_context(|| {
        format!("failed to parse sample rate from ElevenLabs output_format {output_format}")
    })
}
