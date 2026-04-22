use anyhow::Result;

use crate::config::AudioConfig;
use crate::tts::SpeechPreview;

#[cfg(feature = "audio-cpal")]
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(feature = "audio-cpal")]
use std::time::{Duration, Instant};

#[cfg(feature = "audio-cpal")]
use anyhow::{Context, bail};
#[cfg(feature = "audio-cpal")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(feature = "audio-cpal")]
use cpal::{Device, SampleFormat, SampleRate, StreamConfig};

#[cfg(feature = "audio-cpal")]
use crate::tts::AudioFormat;

#[derive(Debug, Clone)]
pub struct AudioOutput {
    config: AudioConfig,
}

impl AudioOutput {
    pub fn new(config: AudioConfig) -> Self {
        Self { config }
    }

    pub fn describe_configuration(&self) -> Result<()> {
        tracing::info!(
            "audio output configured: device='{}', sample_rate_hz={}, channel_count={}",
            self.config.output_device,
            self.config.sample_rate_hz,
            self.config.channel_count
        );
        Ok(())
    }

    pub fn list_output_devices(&self) -> Result<Vec<String>> {
        #[cfg(feature = "audio-cpal")]
        {
            let host = cpal::default_host();
            let devices = host
                .output_devices()
                .context("failed to query output devices")?;

            let mut names = Vec::new();
            for device in devices {
                let name = device
                    .name()
                    .unwrap_or_else(|_| "<unavailable-name>".to_string());
                names.push(name);
            }

            Ok(names)
        }

        #[cfg(not(feature = "audio-cpal"))]
        {
            tracing::warn!("audio-cpal feature is disabled; device enumeration is stubbed");
            Ok(Vec::new())
        }
    }

    pub fn play_preview(&self, preview: &SpeechPreview) -> Result<()> {
        self.describe_configuration()?;
        println!("Preview text: {}", preview.text);
        println!(
            "Preview audio: {} bytes, format={}, {} Hz, {} channel(s)",
            preview.audio_bytes.len(),
            preview.audio_format.label(),
            preview.sample_rate_hz,
            preview.channel_count
        );

        #[cfg(feature = "audio-cpal")]
        {
            self.play_with_cpal(preview)
        }

        #[cfg(not(feature = "audio-cpal"))]
        {
            println!("Audio playback requires the `audio-cpal` feature.");
            Ok(())
        }
    }

    #[cfg(feature = "audio-cpal")]
    fn play_with_cpal(&self, preview: &SpeechPreview) -> Result<()> {
        match preview.audio_format {
            AudioFormat::PcmS16Le => {}
            AudioFormat::Encoded(_) => {
                bail!("audio playback requires PCM output when using `audio-cpal`")
            }
        }

        let device = self.select_output_device()?;
        let stream_config = self.select_stream_config(&device)?;
        let device_name = device
            .name()
            .unwrap_or_else(|_| "<unavailable-name>".to_string());

        let decoded_samples = decode_pcm_s16le_to_f32(&preview.audio_bytes)?;
        let rendered_samples = render_audio_for_output(
            &decoded_samples,
            preview.channel_count,
            preview.sample_rate_hz,
            stream_config.config.channels,
            stream_config.config.sample_rate.0,
        );
        if rendered_samples.is_empty() {
            bail!("received empty PCM audio buffer");
        }

        tracing::info!(
            "playing {} rendered samples on '{}' at {} Hz / {} channel(s)",
            rendered_samples.len(),
            device_name,
            stream_config.config.sample_rate.0,
            stream_config.config.channels
        );

        let state = Arc::new(Mutex::new(PlaybackState {
            samples: rendered_samples,
            position: 0,
            completed: false,
        }));
        let done = Arc::new(AtomicBool::new(false));
        let error_slot = Arc::new(Mutex::new(None::<String>));

        let stream = build_stream(
            &device,
            &stream_config,
            state.clone(),
            done.clone(),
            error_slot.clone(),
        )?;
        stream.play().context("failed to start audio stream")?;

        let duration_hint = playback_duration_hint(state.clone(), stream_config.config);
        wait_for_playback(done, error_slot, duration_hint)?;

        drop(stream);
        Ok(())
    }

    #[cfg(feature = "audio-cpal")]
    fn select_output_device(&self) -> Result<Device> {
        let host = cpal::default_host();

        if self.config.output_device.trim().is_empty() {
            return host
                .default_output_device()
                .context("no default output device is available");
        }

        let wanted = self.config.output_device.trim().to_ascii_lowercase();
        let devices = host
            .output_devices()
            .context("failed to query output devices")?;

        for device in devices {
            let Ok(name) = device.name() else {
                continue;
            };
            if name.to_ascii_lowercase() == wanted || name.to_ascii_lowercase().contains(&wanted) {
                return Ok(device);
            }
        }

        bail!(
            "output device '{}' was not found",
            self.config.output_device
        )
    }

    #[cfg(feature = "audio-cpal")]
    fn select_stream_config(&self, device: &Device) -> Result<ResolvedStreamConfig> {
        let default = device
            .default_output_config()
            .context("failed to query default output config")?;
        let preferred_rate = self.config.sample_rate_hz;
        let preferred_channels = self.config.channel_count;

        let chosen = device
            .supported_output_configs()
            .ok()
            .and_then(|configs| {
                configs.into_iter().find_map(|range| {
                    if range.channels() != preferred_channels {
                        return None;
                    }
                    if range.min_sample_rate().0 <= preferred_rate
                        && preferred_rate <= range.max_sample_rate().0
                    {
                        Some(ResolvedStreamConfig {
                            sample_format: range.sample_format(),
                            config: StreamConfig {
                                channels: preferred_channels,
                                sample_rate: SampleRate(preferred_rate),
                                buffer_size: cpal::BufferSize::Default,
                            },
                        })
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| {
                if default.sample_rate().0 != preferred_rate || default.channels() != preferred_channels
                {
                    tracing::info!(
                        "preferred output config {} Hz / {} channel(s) is unavailable; using device default {} Hz / {} channel(s)",
                        preferred_rate,
                        preferred_channels,
                        default.sample_rate().0,
                        default.channels()
                    );
                }

                ResolvedStreamConfig {
                    sample_format: default.sample_format(),
                    config: default.config(),
                }
            });

        Ok(chosen)
    }
}

#[cfg(feature = "audio-cpal")]
#[derive(Clone)]
struct ResolvedStreamConfig {
    sample_format: SampleFormat,
    config: StreamConfig,
}

#[cfg(feature = "audio-cpal")]
struct PlaybackState {
    samples: Vec<f32>,
    position: usize,
    completed: bool,
}

#[cfg(feature = "audio-cpal")]
fn decode_pcm_s16le_to_f32(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() % 2 != 0 {
        bail!("PCM audio byte length must be even, got {}", bytes.len());
    }

    let mut samples = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(sample as f32 / i16::MAX as f32);
    }

    Ok(samples)
}

#[cfg(feature = "audio-cpal")]
fn render_audio_for_output(
    input_samples: &[f32],
    input_channels: u16,
    input_rate_hz: u32,
    output_channels: u16,
    output_rate_hz: u32,
) -> Vec<f32> {
    if input_samples.is_empty() || input_channels == 0 || output_channels == 0 {
        return Vec::new();
    }

    let input_channels = usize::from(input_channels);
    let output_channels = usize::from(output_channels);
    let input_frame_count = input_samples.len() / input_channels;
    if input_frame_count == 0 {
        return Vec::new();
    }

    let output_frame_count = if input_rate_hz == output_rate_hz {
        input_frame_count
    } else {
        ((input_frame_count as u64 * output_rate_hz as u64) / input_rate_hz as u64).max(1) as usize
    };

    let mut rendered = Vec::with_capacity(output_frame_count * output_channels);
    for output_frame_idx in 0..output_frame_count {
        let source_position = if output_frame_count <= 1 {
            0.0
        } else {
            output_frame_idx as f64 * input_rate_hz as f64 / output_rate_hz as f64
        };
        let source_frame_idx = source_position.floor() as usize;
        let next_frame_idx = (source_frame_idx + 1).min(input_frame_count - 1);
        let fraction = (source_position - source_frame_idx as f64) as f32;

        for output_channel_idx in 0..output_channels {
            let source_channel_idx = if input_channels == 1 {
                0
            } else {
                output_channel_idx.min(input_channels - 1)
            };

            let a = input_samples
                [source_frame_idx.min(input_frame_count - 1) * input_channels + source_channel_idx];
            let b = input_samples[next_frame_idx * input_channels + source_channel_idx];
            rendered.push(a + (b - a) * fraction);
        }
    }

    rendered
}

#[cfg(feature = "audio-cpal")]
fn build_stream(
    device: &Device,
    stream_config: &ResolvedStreamConfig,
    state: Arc<Mutex<PlaybackState>>,
    done: Arc<AtomicBool>,
    error_slot: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream> {
    let config = stream_config.config.clone();
    let error_done = done.clone();
    let error_slot_for_callback = error_slot.clone();
    let err_fn = move |error: cpal::StreamError| {
        if let Ok(mut slot) = error_slot_for_callback.lock() {
            *slot = Some(error.to_string());
        }
        error_done.store(true, Ordering::SeqCst);
    };

    match stream_config.sample_format {
        SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| write_samples_f32(data, &state, &done),
                err_fn,
                None,
            )
            .context("failed to build f32 output stream"),
        SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| write_samples_i16(data, &state, &done),
                err_fn,
                None,
            )
            .context("failed to build i16 output stream"),
        SampleFormat::U16 => device
            .build_output_stream(
                &config,
                move |data: &mut [u16], _| write_samples_u16(data, &state, &done),
                err_fn,
                None,
            )
            .context("failed to build u16 output stream"),
        SampleFormat::U8 => device
            .build_output_stream(
                &config,
                move |data: &mut [u8], _| write_samples_u8(data, &state, &done),
                err_fn,
                None,
            )
            .context("failed to build u8 output stream"),
        other => bail!("unsupported output sample format: {other:?}"),
    }
}

#[cfg(feature = "audio-cpal")]
fn write_samples_f32(
    buffer: &mut [f32],
    state: &Arc<Mutex<PlaybackState>>,
    done: &Arc<AtomicBool>,
) {
    write_samples_generic(buffer, state, done, |sample| sample);
}

#[cfg(feature = "audio-cpal")]
fn write_samples_i16(
    buffer: &mut [i16],
    state: &Arc<Mutex<PlaybackState>>,
    done: &Arc<AtomicBool>,
) {
    write_samples_generic(buffer, state, done, f32_to_i16);
}

#[cfg(feature = "audio-cpal")]
fn write_samples_u16(
    buffer: &mut [u16],
    state: &Arc<Mutex<PlaybackState>>,
    done: &Arc<AtomicBool>,
) {
    write_samples_generic(buffer, state, done, f32_to_u16);
}

#[cfg(feature = "audio-cpal")]
fn write_samples_u8(buffer: &mut [u8], state: &Arc<Mutex<PlaybackState>>, done: &Arc<AtomicBool>) {
    write_samples_generic(buffer, state, done, f32_to_u8);
}

#[cfg(feature = "audio-cpal")]
fn write_samples_generic<T>(
    buffer: &mut [T],
    state: &Arc<Mutex<PlaybackState>>,
    done: &Arc<AtomicBool>,
    convert: impl Fn(f32) -> T,
) {
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(_) => {
            done.store(true, Ordering::SeqCst);
            return;
        }
    };

    for slot in buffer.iter_mut() {
        let sample = if state.position < state.samples.len() {
            let sample = state.samples[state.position];
            state.position += 1;
            sample
        } else {
            state.completed = true;
            0.0
        };
        *slot = convert(sample.clamp(-1.0, 1.0));
    }

    if state.completed {
        done.store(true, Ordering::SeqCst);
    }
}

#[cfg(feature = "audio-cpal")]
fn f32_to_i16(sample: f32) -> i16 {
    (sample * i16::MAX as f32) as i16
}

#[cfg(feature = "audio-cpal")]
fn f32_to_u16(sample: f32) -> u16 {
    ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16
}

#[cfg(feature = "audio-cpal")]
fn f32_to_u8(sample: f32) -> u8 {
    ((sample * 0.5 + 0.5) * u8::MAX as f32) as u8
}

#[cfg(feature = "audio-cpal")]
fn playback_duration_hint(state: Arc<Mutex<PlaybackState>>, config: StreamConfig) -> Duration {
    let sample_count = state.lock().map(|state| state.samples.len()).unwrap_or(0);
    if sample_count == 0 || config.channels == 0 || config.sample_rate.0 == 0 {
        return Duration::from_secs(1);
    }

    let frames = sample_count as f64 / config.channels as f64;
    let seconds = frames / config.sample_rate.0 as f64;
    Duration::from_secs_f64(seconds.max(0.1) + 0.75)
}

#[cfg(feature = "audio-cpal")]
fn wait_for_playback(
    done: Arc<AtomicBool>,
    error_slot: Arc<Mutex<Option<String>>>,
    duration_hint: Duration,
) -> Result<()> {
    let deadline = Instant::now() + duration_hint;
    while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }

    if let Some(error) = error_slot.lock().ok().and_then(|slot| slot.clone()) {
        bail!("audio stream failed: {error}");
    }

    if !done.load(Ordering::SeqCst) {
        bail!("audio playback timed out after {:?}", duration_hint);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "audio-cpal")]
    use super::{decode_pcm_s16le_to_f32, render_audio_for_output};

    #[test]
    #[cfg(feature = "audio-cpal")]
    fn pcm_decode_reads_little_endian_samples() {
        let decoded = decode_pcm_s16le_to_f32(&[0x00, 0x00, 0xff, 0x7f, 0x00, 0x80]).unwrap();
        assert_eq!(decoded.len(), 3);
        assert!(decoded[0].abs() < 0.0001);
        assert!(decoded[1] > 0.99);
        assert!(decoded[2] < -0.99);
    }

    #[test]
    #[cfg(feature = "audio-cpal")]
    fn render_audio_duplicates_mono_to_stereo() {
        let rendered = render_audio_for_output(&[0.0, 0.5, 1.0], 1, 24_000, 2, 24_000);
        assert_eq!(rendered, vec![0.0, 0.0, 0.5, 0.5, 1.0, 1.0]);
    }

    #[test]
    #[cfg(feature = "audio-cpal")]
    fn render_audio_resamples_frame_count() {
        let rendered = render_audio_for_output(&[0.0, 1.0], 1, 24_000, 1, 48_000);
        assert_eq!(rendered.len(), 4);
        assert!(rendered[1] > 0.0);
        assert!(rendered[2] > rendered[1]);
    }
}
