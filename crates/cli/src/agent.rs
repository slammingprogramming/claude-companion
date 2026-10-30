// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
#![cfg_attr(windows, windows_subsystem = "windows")]
//! Background agent: `claude-companion-agent [--user-data <dir>] [--state-dir <dir>] [watch]`.
//! Same as `claude-companion watch`, built without a console window on Windows.

fn main() {
    claude_companion_core::watch::run_agent(std::env::args_os().skip(1));
}
