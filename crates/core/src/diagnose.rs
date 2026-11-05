// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
//! The plain-text report behind `claude-companion doctor`, shared by the command line and the app:
//! what every account has, what a sync would do, and anything that needs attention.

use crate::model::short;
use crate::report::{self, Overview, Warning};
use crate::retention::Limit;
use crate::Ctx;
use std::fmt::Write as _;

fn email_or_id(ov: &Overview, acct: &str) -> String {
    ov.labels.get(acct).cloned().unwrap_or_else(|| format!("account {}", short(acct)))
}

pub fn describe_plan(ov_plan: &report::PlanView, labels: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    let mut lines = Vec::new();
    if ov_plan.total == 0 {
        lines.push("Nothing to do: every linked account already has the same sessions.".to_string());
    }
    for (p, kinds) in &ov_plan.per_partition {
        let who = labels.get(&p.acct).cloned().unwrap_or_else(|| p.label());
        let what: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {k}")).collect();
        lines.push(format!("  {:<6} {:<32} {}", p.surface.as_str(), who, what.join(", ")));
    }
    if !ov_plan.deferred.is_empty() {
        lines.push("Waiting for Claude (it has these open; applied after you switch accounts or quit):".into());
        for (key, n) in &ov_plan.deferred {
            let acct = key.split('/').next().unwrap_or(key);
            let who = labels.get(acct).cloned().unwrap_or_else(|| short(acct).to_string());
            lines.push(format!("  {who}: {n} change(s)"));
        }
    }
    for n in &ov_plan.notes {
        lines.push(format!("note: {n}"));
    }
    lines
}

pub fn doctor(ctx: &Ctx) -> String {
    let mut out = String::new();
    macro_rules! p {
        ($($t:tt)*) => {{
            let _ = writeln!(out, $($t)*);
        }};
    }
    let ov = report::overview(ctx);
    p!("claude-companion {}", crate::VERSION);
    let desktop = match (&ov.desktop_version, ov.app.running) {
        (Some(v), true) => format!("Claude {v}, running"),
        (Some(v), false) => format!("Claude {v}, not running"),
        (None, true) => "Claude is running".to_string(),
        (None, false) => "Claude is not running".to_string(),
    };
    let open = ov.app.open_account.as_deref().map(|a| format!("; open account: {}", email_or_id(&ov, a)));
    p!("{desktop}{}", open.unwrap_or_default());
    p!("Background sync: {}", service_line(&ov));
    for sv in &ov.surfaces {
        p!();
        p!("{} sessions ({})", sv.surface.title(), sv.surface.dir_name());
        for pv in &sv.partitions {
            let mut flags = Vec::new();
            if ov.app.showing(&pv.part) {
                flags.push("OPEN IN CLAUDE");
            }
            if pv.excluded {
                flags.push("excluded");
            }
            if pv.part.is_link {
                flags.push("SYMLINK");
            }
            let who = pv.email.clone().unwrap_or_default();
            if let Some(err) = &pv.error {
                p!("  {}  {:<30} {}  {err}", pv.part.label(), who, flags.join(" "));
                continue;
            }
            let mut facts = vec![format!("{} sessions", pv.sessions)];
            if pv.archived > 0 {
                facts.push(format!("{} archived", pv.archived));
            }
            if pv.missing > 0 && !pv.excluded {
                facts.push(format!("missing {}", pv.missing));
            }
            if pv.unreadable > 0 {
                facts.push(format!("{} unreadable", pv.unreadable));
            }
            p!("  {}  {:<30} {}  {}", pv.part.label(), who, facts.join(" · "), flags.join(" "));
            if ctx.verbose && !pv.unmanaged.is_empty() {
                p!("      left untouched: {}", pv.unmanaged.join(", "));
            }
        }
    }
    let retention = match ov.retention.desktop_days {
        Some(d) => format!("deleted after {d} days"),
        None => "kept at any age".into(),
    };
    p!();
    p!("Transcripts of Desktop sessions: {retention}");
    if let Some(ls) = &ov.last_sync {
        p!("Last sync: {} ({}), {} change(s), {} error(s)", report::ago(ls.at), ls.reason, ls.applied, ls.errors);
    }
    if let Some(hint) = ov.restart_hint() {
        p!("\nQuit and reopen Claude to see {} session(s) copied into the account it has open.", hint.sessions);
    }
    if !ov.warnings.is_empty() {
        p!();
        for w in &ov.warnings {
            // Several organizations is the normal case, so it reads as a tip, not a problem.
            let mark = if matches!(w, Warning::SpansOrgs { .. }) { "·" } else { "!" };
            p!("{mark} {}", warning_text(w));
        }
    }
    let cc = crate::code::status(ctx, true);
    p!("\nClaude Code (terminal)");
    match (&cc.cli, &cc.version) {
        (Some(path), Some(v)) => p!("  {v} at {}", path.display()),
        (Some(path), None) => p!("  found at {}", path.display()),
        (None, _) => p!("  not found (install it from https://claude.com/product/claude-code)"),
    }
    let who =
        cc.email.as_deref().map_or("not signed in, or signed in without an account file".to_string(), str::to_string);
    p!("  signed in as: {who}");
    p!("  {} session(s) in {} folder(s); {} MCP server(s) of its own", cc.sessions, cc.projects, cc.mcp_servers);
    let shared = crate::mcp::overview(ctx);
    let (only_d, only_c) = (
        shared.entries.iter().filter(|e| e.desktop && !e.code).count(),
        shared.entries.iter().filter(|e| e.code && !e.desktop).count(),
    );
    if only_d + only_c > 0 {
        p!("  MCP servers only in Desktop: {only_d}, only in Claude Code: {only_c} (share them with `claude-companion mcp share`)");
    }
    let groups =
        if ctx.config().sync_groups { "on" } else { "off (turn on with `claude-companion config --groups on`)" };
    p!("\nSidebar groups: {groups}, merged only while Claude is closed.");
    p!("Not synced by design: pins, Cowork tasks, claude.ai chats, projects and memory, cloud sessions, connectors and Remote Control links.");
    p!("\nIf you sync now:");
    for l in describe_plan(&ov.plan, &ov.labels) {
        p!("{l}");
    }
    out
}

fn service_line(ov: &Overview) -> String {
    let beat = ov.heartbeat.as_ref().filter(|h| h.is_fresh());
    match (ov.service.installed, beat) {
        (true, Some(h)) => format!("on (agent v{}, last check {})", h.version, report::ago(h.heartbeat_at)),
        (true, None) => format!("on ({})", ov.service.detail),
        (false, _) if ov.service.legacy => {
            "the original Python agent is still installed; `claude-companion install` replaces it".into()
        }
        (false, _) => "off (turn on with `claude-companion install`)".into(),
    }
}

fn warning_text(w: &Warning) -> String {
    match w {
        Warning::SymlinkedFolders { surface, count } => format!(
            "{count} {surface} session folder(s) are symlinks. Claude silently stops saving sessions there. Quit Claude, then run: claude-companion fix-symlinks"
        ),
        Warning::LeftoverFolders { dirs, .. } => format!("Leftover backup folders from other tools: {} (safe to archive by hand)", dirs.join(", ")),
        Warning::SpansOrgs { orgs, .. } => format!(
            "Your accounts span {orgs} organizations, and synced sessions appear in all of them. To keep one separate: claude-companion config --exclude <account-id>"
        ),
        Warning::ShortRetention { days, limited_by, path } => {
            let why = match limited_by {
                Limit::DesktopSetting => format!("desktopSessionCleanupPeriodDays in {}", path.display()),
                Limit::Organization => "your organization's managed settings".into(),
                Limit::OlderClaudeCode => "a Claude Code before 2.1.248; updating Claude keeps them at any age".into(),
            };
            let fix = match limited_by {
                Limit::Organization => "",
                _ => " Keep them: claude-companion retention --keep",
            };
            format!("Claude Code deletes the transcripts of Desktop sessions after {days} days ({why}), and a session opens empty without one.{fix}")
        }
        Warning::MissingTranscripts { missing, total } => {
            format!("{missing} of {total} sessions no longer have a transcript on disk; they open empty or not at all")
        }
        Warning::DesktopDataMissing { path } => format!("Claude Desktop's data folder was not found at {}", path.display()),
    }
}
