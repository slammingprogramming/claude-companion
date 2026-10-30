// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
//! Desktop notifications from the background agent (best effort).

use crate::ctx::Ctx;
#[cfg(unix)]
use std::process::{Command, Stdio};

pub fn notify(ctx: &Ctx, text: &str) {
    ctx.log(format!("notice: {text}"));
    if ctx.config().notify && ctx.fake.running.is_none() {
        show(text);
    }
}

#[cfg(target_os = "macos")]
fn show(text: &str) {
    let quoted = format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let script = format!("display notification {quoted} with title \"Claude Companion\"");
    let _ =
        Command::new("/usr/bin/osascript").args(["-e", &script]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show(text: &str) {
    let _ = Command::new("notify-send")
        .args(["Claude Companion", text])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Windows has no toast API without an app identity; the app shows the same hint in its window.
#[cfg(not(unix))]
fn show(_text: &str) {}
