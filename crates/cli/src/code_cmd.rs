// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! `claude-companion code …`, `mcp …`, `open` and `quit`: the terminal program (Claude Code) and
//! the desktop app (Claude) controlled from one place.

use super::{quit_watch, ShareTo};
use anyhow::{bail, Result};
use claude_companion_core::code::{self, Session};
use claude_companion_core::mcp::{self, Direction};
use claude_companion_core::{desktop, report, Ctx};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn when(ms: i64) -> String {
    if ms <= 0 {
        return "-".into();
    }
    report::ago(ms as f64 / 1000.0)
}

fn print_json(value: serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// The last two parts of a folder, to fit a table.
fn tail(path: &str) -> String {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let n = parts.len().saturating_sub(2);
    parts[n..].join("/")
}

pub fn status(ctx: &Ctx, json: bool) -> Result<()> {
    let st = code::status(ctx, true);
    if json {
        return print_json(json!({ "schemaVersion": 1, "claudeCode": st }));
    }
    match (&st.cli, &st.version) {
        (Some(p), Some(v)) => println!("Claude Code {v} at {}", p.display()),
        (Some(p), None) => println!("Claude Code at {}", p.display()),
        (None, _) => println!("Claude Code was not found (install it from https://claude.com/product/claude-code)."),
    }
    println!("Signed in as: {}", st.email.as_deref().unwrap_or("-"));
    println!("Settings: {}{}", st.settings_file.display(), if st.settings_exist { "" } else { " (none yet)" });
    println!("{} session(s) in {} folder(s); {} MCP server(s) of its own", st.sessions, st.projects, st.mcp_servers);
    println!("Claude Desktop: {}", if desktop::is_running(ctx) { "running" } else { "not running" });
    Ok(())
}

pub fn sessions(ctx: &Ctx, dir: Option<&Path>, limit: usize, terminal_only: bool, json: bool) -> Result<()> {
    // Fetch more than asked for when Desktop's own sessions are going to be dropped.
    let fetch = if terminal_only { limit.saturating_mul(4).max(40) } else { limit };
    let mut all = code::sessions(ctx, dir, fetch);
    if terminal_only {
        all.retain(|s| !s.desktop);
    }
    all.truncate(limit);
    if json {
        return print_json(json!({ "schemaVersion": 1, "sessions": all }));
    }
    if all.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }
    for s in &all {
        let from = if s.desktop { " [Desktop]" } else { "" };
        println!("  {:<8} {:<9} {:<24} {}{from}", &s.id[..s.id.len().min(8)], when(s.last_ms), tail(&s.cwd), s.title);
    }
    println!("\nContinue one with `claude-companion code resume <id>` (the start of the id is enough).");
    Ok(())
}

/// Run `claude` here, or in a new window; leave with its exit code.
fn start(dir: Option<&Path>, args: &[String], new_window: bool) -> Result<()> {
    if new_window {
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => std::env::current_dir()?,
        };
        code::open_window(&dir, args)?;
        println!("Opened Claude Code in a new terminal window ({}).", dir.display());
        return Ok(());
    }
    let status = code::run_here(dir, args)?;
    if status != 0 {
        std::process::exit(status);
    }
    Ok(())
}

pub fn resume(ctx: &Ctx, query: &str, new_window: bool) -> Result<()> {
    let session: Session = code::resolve_session(ctx, query)?;
    let (dir, args) = code::resume_target(&session)?;
    eprintln!("Continuing \"{}\" in {}", session.title, dir.display());
    start(Some(&dir), &args, new_window)
}

pub fn run(dir: Option<PathBuf>, new_window: bool, args: &[String]) -> Result<()> {
    start(dir.as_deref(), args, new_window)
}

pub fn settings(ctx: &Ctx, key: Option<&str>, value: Option<&str>, unset: bool) -> Result<()> {
    match (key, value, unset) {
        (Some(key), Some(value), false) => {
            let backup = code::settings_set(ctx, key, Some(code::parse_value(value)))?;
            println!("Set {key} in {}", ctx.paths.claude_settings.display());
            if let Some(b) = backup {
                println!("The previous file is kept as {}", b.display());
            }
        }
        (Some(key), None, true) => {
            let backup = code::settings_set(ctx, key, None)?;
            println!("Removed {key} from {}", ctx.paths.claude_settings.display());
            if let Some(b) = backup {
                println!("The previous file is kept as {}", b.display());
            }
        }
        (Some(key), None, false) => {
            let shown = code::redact(&serde_json::Value::Object(code::settings(ctx)?));
            let found = shown.as_object().and_then(|m| code::get_path(m, key));
            match found {
                Some(v) => println!("{}", serde_json::to_string_pretty(v)?),
                None => bail!("{key} is not set in {}", ctx.paths.claude_settings.display()),
            }
        }
        (None, None, false) => {
            let shown = code::redact(&serde_json::Value::Object(code::settings(ctx)?));
            println!("{}", serde_json::to_string_pretty(&shown)?);
            eprintln!("({}; secrets are hidden)", ctx.paths.claude_settings.display());
        }
        _ => bail!("use `code settings`, `code settings <key>`, `code settings <key> <value>` or `code settings <key> --unset`"),
    }
    Ok(())
}

pub fn mcp_list(ctx: &Ctx, json: bool) -> Result<()> {
    let ov = mcp::overview(ctx);
    if json {
        return print_json(json!({ "schemaVersion": 1, "mcp": ov }));
    }
    for w in &ov.warnings {
        eprintln!("warning: {w}");
    }
    if ov.entries.is_empty() {
        println!("No MCP servers in Claude Desktop or Claude Code.");
        return Ok(());
    }
    println!("  {:<24} {:<6} {:<8} {:<8} detail", "server", "kind", "Desktop", "Code");
    for e in &ov.entries {
        let mark = |b: bool| if b { "yes" } else { "-" };
        let same = if e.desktop && e.code && !e.same { "  (differs)" } else { "" };
        println!("  {:<24} {:<6} {:<8} {:<8} {}{same}", e.name, e.kind, mark(e.desktop), mark(e.code), e.detail);
    }
    println!("\nShare them with `claude-companion mcp share` (add --dry-run to look first).");
    Ok(())
}

pub fn mcp_share(ctx: &Ctx, to: Option<ShareTo>, overwrite: bool, dry_run: bool, json: bool) -> Result<()> {
    let dir = match to {
        None => Direction::Both,
        Some(ShareTo::Desktop) => Direction::ToDesktop,
        Some(ShareTo::Code) => Direction::ToCode,
    };
    if overwrite && to.is_none() {
        bail!("--overwrite needs --to desktop or --to code: say which side's server wins");
    }
    let report = mcp::share(ctx, dir, overwrite, dry_run)?;
    if json {
        return print_json(json!({ "schemaVersion": 1, "dryRun": dry_run, "report": report }));
    }
    for line in report.lines() {
        println!("{line}");
    }
    if !dry_run && !report.written.is_empty() {
        println!("Restart Claude Desktop and any open Claude Code session to load them.");
    }
    Ok(())
}

pub fn open_desktop(ctx: &Ctx) -> Result<()> {
    if desktop::is_running(ctx) {
        println!("Claude is already running.");
        return Ok(());
    }
    code::open_desktop(ctx)?;
    println!("Started Claude Desktop.");
    Ok(())
}

pub fn quit_desktop(ctx: &Ctx, force: bool) -> Result<()> {
    if !desktop::is_running(ctx) {
        println!("Claude is not running.");
        return Ok(());
    }
    desktop::quit(ctx, Duration::from_secs(60), &quit_watch(force))?;
    println!("Claude has quit.");
    Ok(())
}
