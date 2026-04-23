#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
uid="$(id -u)"
user="${USER:?USER is not set}"

if [[ "$uid" == "0" ]]; then
  echo "Run this script as the target user, not with sudo. The script will use sudo only for installation steps." >&2
  exit 1
fi

cargo build --release --features audio-cpal --manifest-path "$repo_root/Cargo.toml"

tmp_unit="$(mktemp)"
trap 'rm -f "$tmp_unit"' EXIT

sed \
  -e "s|@REPO_ROOT@|$repo_root|g" \
  -e "s|@USER@|$user|g" \
  -e "s|@UID@|$uid|g" \
  "$repo_root/systemd/openclaw-speak.service.in" > "$tmp_unit"

sudo install -m 0644 "$tmp_unit" /etc/systemd/system/openclaw-speak.service
sudo systemctl daemon-reload

cat <<EOF
Installed openclaw-speak.service for $user.

Next steps:
  sudo systemctl enable --now openclaw-speak.service
  sudo systemctl status openclaw-speak.service
EOF
