#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Songkeys (cc-same, MIT)
# Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
# Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
# Build release archives for Linux: the app with its desktop entry and icon, and the CLI.
#
#   scripts/bundle-linux.sh            # -> dist/
#
# Install the app archive with:
#   tar -xzf Claude-Companion-<version>-linux-<arch>.tar.gz -C ~/.local --strip-components=1
set -euo pipefail
cd "$(dirname "$0")/.."

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) arch="$(uname -m)" ;;
esac
out="${CLAUDE_COMPANION_OUTPUT:-dist}"
mkdir -p "$out"

cargo build --release -p claude-companion
cargo build --release --manifest-path crates/app/Cargo.toml

stage="$(mktemp -d)"
root="$stage/claude-companion-$version"
install -Dm755 target/release/claude-companion-app "$root/bin/claude-companion-app"
install -Dm755 target/release/claude-companion "$root/bin/claude-companion"
install -Dm644 crates/app/resources/linux/claude-companion.desktop "$root/share/applications/claude-companion.desktop"
install -Dm644 crates/app/resources/linux/claude-companion.png "$root/share/icons/hicolor/512x512/apps/claude-companion.png"

tar -C "$stage" -czf "$out/Claude-Companion-$version-linux-$arch.tar.gz" "claude-companion-$version"
tar -C target/release -czf "$out/claude-companion-cli-$version-linux-$arch.tar.gz" claude-companion
rm -rf "$stage"
ls -1 "$out"
