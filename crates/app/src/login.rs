// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
//! Opening the app at login, quietly: it starts with `--hidden` and shows only its tray icon.

use anyhow::{Context as _, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// Passed by the login item: start without a window.
pub const HIDDEN: &str = "--hidden";

pub fn started_hidden() -> bool {
    std::env::args_os().any(|a| a == HIDDEN)
}

fn launcher() -> Result<AutoLaunch> {
    // A debug build lives in `target/`; registering it would leave a stale login item.
    if cfg!(debug_assertions) && std::env::var_os("CLAUDE_COMPANION_DEV_LOGIN").is_none() {
        anyhow::bail!("login items are off in debug builds (set CLAUDE_COMPANION_DEV_LOGIN=1)");
    }
    let exe = std::env::current_exe().context("locating the app")?;
    let exe = exe.to_str().context("the app's path is not valid Unicode")?;
    AutoLaunchBuilder::new()
        .set_app_name(crate::APP_NAME)
        .set_app_path(exe)
        .set_args(&[HIDDEN])
        .set_bundle_identifiers(&[crate::APP_ID])
        .build()
        .context("setting up the login item")
}

pub fn enabled() -> bool {
    launcher().and_then(|l| l.is_enabled().map_err(Into::into)).unwrap_or(false)
}

pub fn set(on: bool) -> Result<()> {
    let launcher = launcher()?;
    if on { launcher.enable() } else { launcher.disable() }.context("changing the login item")
}
