# openclaw-speak

`openclaw-speak` is a command-line Rust application for Linux that listens for OpenClaw assistant replies and voices them through the local sound device using ElevenLabs text-to-speech.

## Goals

- Run as a local CLI daemon on Linux.
- Intercept assistant replies from an OpenClaw Gateway session.
- Convert visible assistant text into speech with ElevenLabs.
- Play audio on the local output device.
- Support cross-compilation for Raspberry Pi 5.

## Current Status

This repository currently contains a compile-first scaffold:

- CLI entry points
- config loading from file and environment
- module boundaries for OpenClaw Gateway, ElevenLabs, and audio output
- Raspberry Pi 5 oriented cross-build settings

The live OpenClaw event subscription and ElevenLabs audio playback pipeline are the next implementation steps.

## Configuration

Configuration is loaded from:

1. `--config <path>` if provided
2. `$XDG_CONFIG_HOME/openclaw-speak/config.toml`
3. `~/.config/openclaw-speak/config.toml`
4. `.env` in the current working directory, if present
5. environment variables

Environment variables:

- `OPENCLAW_GATEWAY_URL`
- `OPENCLAW_GATEWAY_TOKEN`
- `OPENCLAW_SESSION_FILTER`
- `ELEVENLABS_API_KEY`
- `ELEVENLABS_VOICE_ID`
- `ELEVENLABS_MODEL_ID`
- `ELEVENLABS_OUTPUT_FORMAT`
- `ELEVENLABS_LANGUAGE_CODE`
- `AUDIO_OUTPUT_DEVICE`
- `RUST_LOG`

See [`config.example.toml`](./config.example.toml).
For secret values, see [`.env.example`](./.env.example).

Recommended split:

- keep stable non-secret settings in `config.toml`
- keep secrets such as `OPENCLAW_GATEWAY_TOKEN` and `ELEVENLABS_API_KEY` in `.env`
- use real exported shell env vars only when you want to override `.env`

Example `.env`:

```bash
OPENCLAW_GATEWAY_TOKEN=replace-me
ELEVENLABS_API_KEY=replace-me
```

## Remote Gateway Over LAN

Running `openclaw-speak` on a different machine from the Gateway is fine.

- Preferred secure default: keep the Gateway loopback-only and reach it through SSH, VPN, or Tailscale.
- LAN option: bind the Gateway for LAN access and require token or password auth.
- This project is already set up to send a Gateway token using `openclaw.gateway_token` or `OPENCLAW_GATEWAY_TOKEN`.

Example LAN client config:

```toml
[openclaw]
gateway_url = "ws://192.168.1.50:18789"
gateway_token = "replace-me"
```

If a `.local` hostname resolves only to a link-local IPv6 address, use either:

- an IPv4 LAN address such as `ws://192.168.1.50:18789`, or
- a scoped IPv6 literal such as `ws://[fe80::1234:5678:abcd:ef01%eth0]:18789`

Plain `ws://ada.local:18789` can fail in that case because link-local IPv6 needs an interface scope.

## Development

```bash
cargo run -- devices
cargo run -- test --text "Hello from openclaw-speak"
cargo run -- daemon
```

To enable real Linux audio device access through CPAL and ALSA:

```bash
cargo run --features audio-cpal -- devices
cargo run --features audio-cpal -- test --text "Hello from openclaw-speak"
cargo run --features audio-cpal -- daemon
```

On Debian or Ubuntu style systems, that feature typically needs:

```bash
sudo apt install pkg-config libasound2-dev
```

With `audio-cpal` enabled, the app will:

- decode ElevenLabs PCM output
- resample it to the selected device configuration when needed
- play speech through the chosen output device

## systemd User Service

This repo includes a `systemd --user` unit template and installer helper for running the daemon in the background.

Install the service files for the current checkout:

```bash
./scripts/install-systemd-user-service.sh
```

Enable and start it:

```bash
systemctl --user enable --now openclaw-speak.service
```

Useful commands:

```bash
systemctl --user status openclaw-speak.service
journalctl --user -u openclaw-speak.service -f
systemctl --user restart openclaw-speak.service
```

Notes:

- the installer builds `target/release/openclaw-speak` with `--features audio-cpal` if needed
- the unit runs with `WorkingDirectory` set to this repo root, so the local `.env` file continues to load
- if you move the repo, rerun the installer so the generated unit points at the new path

## Cross-compiling for Raspberry Pi 5

The default target for a 64-bit Raspberry Pi 5 image is:

```bash
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

You will also need an ARM64 cross linker installed on the build machine, for example `aarch64-linux-gnu-gcc`.

For audio playback on the Pi, system libraries for ALSA are expected to be present on the target system.

If you build with `--features audio-cpal`, make sure your cross toolchain also exposes the target ALSA development files.
