mod app;
mod audio;
mod cli;
mod config;
mod gateway;
mod tts;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    app::run().await
}
