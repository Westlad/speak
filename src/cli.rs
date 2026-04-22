use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "openclaw-speak")]
#[command(about = "Voice OpenClaw assistant replies on Linux")]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run the long-lived daemon that will watch OpenClaw and speak replies.
    Daemon,
    /// List local output devices available through CPAL.
    Devices,
    /// Show current OpenClaw session connectivity settings.
    Sessions,
    /// Synthesize a single text string for local verification.
    Test {
        #[arg(long)]
        text: String,
    },
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
