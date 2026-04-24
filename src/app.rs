use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use anyhow::Result;
use tokio::signal;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use crate::audio::AudioOutput;
use crate::cli::{Cli, Commands};
use crate::config::AppConfig;
use crate::gateway::{AssistantReply, OpenClawGatewayClient, SessionSummary};
use crate::tts::{ElevenLabsClient, SpeechPreview};

pub async fn run() -> Result<()> {
    init_tracing();

    let cli = Cli::parse_args();
    let config = AppConfig::load(cli.config.as_deref())?;

    match cli.command {
        Commands::Daemon => run_daemon(config).await,
        Commands::Devices => list_devices(config).await,
        Commands::Sessions => list_sessions(config).await,
        Commands::Test { text } => test_speech(config, &text).await,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn run_daemon(config: AppConfig) -> Result<()> {
    let gateway = OpenClawGatewayClient::new(config.openclaw.clone());
    let tts = ElevenLabsClient::new(config.elevenlabs.clone());
    let audio = Arc::new(AudioOutput::new(config.audio.clone()));
    let playback = PlaybackController::new(audio.clone(), config.speech.interrupt_on_new_reply);
    let mut tracker = ReplyTracker::new(Duration::from_millis(config.speech.dedupe_window_ms));

    tracing::info!("starting openclaw-speak daemon");
    tracing::info!("gateway_url = {}", gateway.gateway_url());
    tracing::info!("session_filter = {:?}", gateway.session_filter());

    gateway.describe_connectivity()?;
    tts.describe_configuration()?;
    audio.describe_configuration()?;

    let mut connection = gateway.connect().await?;
    connection.subscribe_to_session_changes().await?;

    let mut subscribed_sessions = HashSet::new();
    refresh_session_subscriptions(&connection, &mut subscribed_sessions).await?;

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                tracing::info!("received Ctrl+C, stopping daemon");
                break;
            }
            event = connection.next_event() => {
                let Some(event) = event else {
                    tracing::warn!("gateway event stream ended");
                    break;
                };

                match event.name.as_str() {
                    "sessions.changed" => {
                        tracing::info!("sessions changed; refreshing subscriptions");
                        refresh_session_subscriptions(&connection, &mut subscribed_sessions).await?;
                    }
                    name if name.starts_with("session.message") => {
                        let Some(session_key) = event.session_key() else {
                            tracing::info!(
                                "received {name} without a recognizable session key: {}",
                                event.payload
                            );
                            continue;
                        };

                        tracing::info!("received {name} for session {session_key}");

                        if !subscribed_sessions.contains(session_key) {
                            continue;
                        }

                        match connection.fetch_latest_assistant_reply(session_key).await {
                            Ok(Some(reply)) if tracker.should_speak(session_key, &reply) => {
                                tracing::info!("speaking assistant reply for session {session_key}");
                                match tts.synthesize_preview(&reply.text).await {
                                    Ok(preview) => {
                                        playback.submit(session_key.to_string(), preview);
                                    }
                                    Err(error) => {
                                        tracing::warn!("tts synthesis failed for session {session_key}: {error}");
                                    }
                                }
                            }
                            Ok(None) => {
                                tracing::info!("no assistant reply text found yet for session {session_key}");
                            }
                            Ok(Some(_)) => {}
                            Err(error) => {
                                tracing::warn!("failed to fetch latest assistant reply for session {session_key}: {error}");
                            }
                        }
                    }
                    other => tracing::debug!("ignoring gateway event {other}"),
                }
            }
        }
    }

    Ok(())
}

async fn list_devices(config: AppConfig) -> Result<()> {
    let audio = AudioOutput::new(config.audio);
    let devices = audio.list_output_devices()?;

    if devices.is_empty() {
        println!("No output devices found.");
    } else {
        for device in devices {
            println!("{device}");
        }
    }

    Ok(())
}

async fn list_sessions(config: AppConfig) -> Result<()> {
    let gateway = OpenClawGatewayClient::new(config.openclaw);
    gateway.describe_connectivity()?;
    let connection = gateway.connect().await?;
    let sessions = connection.list_sessions().await?;

    println!("Gateway URL: {}", gateway.gateway_url());
    println!("Session filter: {:?}", gateway.session_filter());

    if sessions.is_empty() {
        println!("No sessions found.");
    } else {
        for session in sessions {
            match session.title {
                Some(title) if !title.is_empty() => println!("{} - {}", session.key, title),
                _ => println!("{}", session.key),
            }
        }
    }

    Ok(())
}

async fn test_speech(config: AppConfig, text: &str) -> Result<()> {
    let tts = ElevenLabsClient::new(config.elevenlabs);
    let audio = AudioOutput::new(config.audio);

    let preview = tts.synthesize_preview(text).await?;
    audio.play_preview(&preview)?;
    Ok(())
}

async fn refresh_session_subscriptions(
    connection: &crate::gateway::GatewayConnection,
    subscribed_sessions: &mut HashSet<String>,
) -> Result<()> {
    let sessions = connection.list_sessions().await?;
    for session in sessions {
        if session_matches_filter(connection.session_filter(), &session)
            && subscribed_sessions.insert(session.key.clone())
        {
            connection
                .subscribe_to_session_messages(&session.key)
                .await?;
            tracing::info!("subscribed to session {}", session.key);
        }
    }

    Ok(())
}

fn session_matches_filter(filter: Option<&str>, session: &SessionSummary) -> bool {
    let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };

    let filter = filter.to_ascii_lowercase();
    if session.key.to_ascii_lowercase().contains(&filter) {
        return true;
    }

    session
        .title
        .as_deref()
        .map(|title| title.to_ascii_lowercase().contains(&filter))
        .unwrap_or(false)
}

struct ReplyTracker {
    entries: HashMap<String, TrackedReply>,
}

impl ReplyTracker {
    fn new(_dedupe_window: Duration) -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn should_speak(&mut self, session_key: &str, reply: &AssistantReply) -> bool {
        if let Some(previous) = self.entries.get(session_key) {
            if previous.fingerprint == reply.fingerprint {
                return false;
            }
        }

        self.entries.insert(
            session_key.to_string(),
            TrackedReply {
                fingerprint: reply.fingerprint.clone(),
            },
        );
        true
    }
}

struct TrackedReply {
    fingerprint: String,
}

#[derive(Clone)]
struct PlaybackController {
    audio: Arc<AudioOutput>,
    lock: Arc<Mutex<()>>,
    generation: Arc<AtomicU64>,
    interrupt_on_new_reply: bool,
}

impl PlaybackController {
    fn new(audio: Arc<AudioOutput>, interrupt_on_new_reply: bool) -> Self {
        Self {
            audio,
            lock: Arc::new(Mutex::new(())),
            generation: Arc::new(AtomicU64::new(0)),
            interrupt_on_new_reply,
        }
    }

    fn submit(&self, session_key: String, preview: SpeechPreview) {
        let audio = self.audio.clone();
        let lock = self.lock.clone();
        let generation = self.generation.clone();
        let interrupt_on_new_reply = self.interrupt_on_new_reply;
        let generation_id = generation.fetch_add(1, Ordering::SeqCst) + 1;

        tokio::spawn(async move {
            let playback_guard = lock.lock().await;
            if interrupt_on_new_reply && generation_id != generation.load(Ordering::SeqCst) {
                tracing::info!("skipping stale queued speech for session {session_key}");
                drop(playback_guard);
                return;
            }

            let task = tokio::task::spawn_blocking(move || audio.play_preview(&preview)).await;
            drop(playback_guard);

            match task {
                Ok(Ok(())) => {
                    tracing::info!("finished speech playback for session {session_key}");
                }
                Ok(Err(error)) => {
                    tracing::warn!("audio playback failed for session {session_key}: {error}");
                }
                Err(error) => {
                    tracing::warn!("audio playback task failed for session {session_key}: {error}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::ReplyTracker;
    use crate::gateway::AssistantReply;
    use std::time::Duration;

    #[test]
    fn identical_fingerprint_is_never_spoken_twice() {
        let mut tracker = ReplyTracker::new(Duration::from_secs(0));
        let reply = AssistantReply {
            text: "Earlier reply".to_string(),
            fingerprint: "id:msg-1".to_string(),
        };

        assert!(tracker.should_speak("session-1", &reply));
        assert!(!tracker.should_speak("session-1", &reply));
    }

    #[test]
    fn same_text_with_new_fingerprint_still_speaks() {
        let mut tracker = ReplyTracker::new(Duration::from_secs(60));
        let first = AssistantReply {
            text: "Repeated wording".to_string(),
            fingerprint: "id:msg-1".to_string(),
        };
        let second = AssistantReply {
            text: "Repeated wording".to_string(),
            fingerprint: "id:msg-2".to_string(),
        };

        assert!(tracker.should_speak("session-1", &first));
        assert!(tracker.should_speak("session-1", &second));
    }
}
