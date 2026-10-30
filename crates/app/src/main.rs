// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    // The updater checks that a new copy starts and is the version it expects.
    if std::env::args_os().nth(1).is_some_and(|a| a == claude_companion_app::update::PRINT_VERSION) {
        println!("{} {}", claude_companion_app::APP_NAME, claude_companion_app::update::VERSION);
        return;
    }
    // The background agent is this same binary started with `watch`: no window, no UI.
    if std::env::args_os().skip(1).any(|a| a == "watch") {
        claude_companion_core::watch::run_agent(std::env::args_os().skip(1));
        return;
    }
    claude_companion_app::run();
}
