#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
systemd_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
unit_name="openclaw-speak.service"
unit_template="$repo_root/systemd/openclaw-speak.service.in"
unit_target="$systemd_dir/$unit_name"
binary_path="$repo_root/target/release/openclaw-speak"

mkdir -p "$systemd_dir"

if [[ ! -x "$binary_path" ]]; then
  echo "Building release binary at $binary_path"
  cargo build --release --features audio-cpal --manifest-path "$repo_root/Cargo.toml"
fi

escaped_repo_root="${repo_root//\//\\/}"
escaped_binary_path="${binary_path//\//\\/}"

sed \
  -e "s/@WORKDIR@/$escaped_repo_root/g" \
  -e "s/@EXEC_START@/$escaped_binary_path/g" \
  "$unit_template" > "$unit_target"

systemctl --user daemon-reload

cat <<EOF
Installed $unit_target

Next steps:
  systemctl --user enable --now $unit_name
  systemctl --user status $unit_name
  journalctl --user -u $unit_name -f
EOF

