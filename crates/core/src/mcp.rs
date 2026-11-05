// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! MCP servers, shared between Claude Desktop and Claude Code.
//!
//! Desktop keeps its servers in `claude_desktop_config.json` (`mcpServers`, programs it starts
//! itself); Claude Code keeps its user-wide ones in `~/.claude.json` (`mcpServers`). The same
//! server entry works in both when it is a local program, so this lists both side by side and copies
//! what one is missing into the other. It never overwrites a server that differs unless asked to,
//! never touches project-scoped servers, and keeps the previous file beside the new one.
//!
//! Desktop's file is only changed while Desktop is closed (it rewrites the file when it quits).

use crate::code::backup_beside;
use crate::{desktop, fsx, Ctx};
use anyhow::{bail, Context as _, Result};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_JSON: u64 = 64 * 1024 * 1024;
const KEY: &str = "mcpServers";

/// Claude Desktop's own file of MCP servers.
pub fn desktop_file(ctx: &Ctx) -> PathBuf {
    ctx.paths.user_data.join("claude_desktop_config.json")
}

/// Claude Code's file that holds its user-wide MCP servers.
pub fn code_file(ctx: &Ctx) -> &Path {
    &ctx.paths.claude_json
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Side {
    Desktop,
    Code,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Desktop => "Claude Desktop",
            Side::Code => "Claude Code",
        }
    }
}

/// Which way servers are copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Each side gets what the other has and it lacks.
    Both,
    /// Claude Code's servers into Claude Desktop.
    ToDesktop,
    /// Claude Desktop's servers into Claude Code.
    ToCode,
}

/// One server, as the two sides have it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// `stdio` (a local program), `http` or `sse`.
    pub kind: String,
    /// The command (or address) without its arguments, which may hold secrets.
    pub detail: String,
    pub desktop: bool,
    pub code: bool,
    /// Both have it, and it is the same.
    pub same: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Overview {
    pub desktop_file: PathBuf,
    pub code_file: PathBuf,
    pub entries: Vec<Entry>,
    pub warnings: Vec<String>,
}

/// One server to copy.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Change {
    pub name: String,
    pub to: Side,
    /// It replaces a different server of that name.
    pub replaces: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// Servers that cannot go to the other side, and why.
    pub skipped: Vec<(String, String)>,
    /// Servers both sides have, differently; left as they are.
    pub differing: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub dry_run: bool,
    pub plan: Plan,
    /// Files written, with the copies of what they were.
    pub backups: Vec<PathBuf>,
    pub written: Vec<Side>,
    /// A side that could not be changed now, and why.
    pub deferred: Vec<(Side, String)>,
}

impl Report {
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let verb = if self.dry_run { "would copy" } else { "copied" };
        for side in [Side::Desktop, Side::Code] {
            let names: Vec<String> = self
                .plan
                .changes
                .iter()
                .filter(|c| c.to == side)
                .map(|c| if c.replaces { format!("{} (replacing)", c.name) } else { c.name.clone() })
                .collect();
            if !names.is_empty() {
                out.push(format!("{verb} into {}: {}", side.name(), names.join(", ")));
            }
        }
        if self.plan.changes.is_empty() {
            out.push("Nothing to copy: both sides already have each other's servers.".into());
        }
        if !self.plan.differing.is_empty() {
            out.push(format!(
                "differ between the two (left as they are; choose a side with --to and --overwrite): {}",
                self.plan.differing.join(", ")
            ));
        }
        for (name, why) in &self.plan.skipped {
            out.push(format!("skipped {name}: {why}"));
        }
        for (side, why) in &self.deferred {
            out.push(format!("{} not changed: {why}", side.name()));
        }
        for b in &self.backups {
            out.push(format!("backup: {}", b.display()));
        }
        out
    }
}

// ------------------------------------------------------------------------------------ reading

/// The `mcpServers` of a file; empty when the file does not exist.
fn read_servers(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let root = fsx::read_json(path, MAX_JSON).with_context(|| format!("cannot read {}", path.display()))?;
    match root.get(KEY) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => bail!("{} has an {KEY} that is not an object; left alone", path.display()),
    }
}

fn kind_of(v: &Value) -> String {
    match v.get("type").and_then(Value::as_str) {
        Some(t) => t.to_lowercase(),
        None if v.get("url").is_some() => "http".into(),
        None => "stdio".into(),
    }
}

/// Compared without the `type: stdio` that only Claude Code writes.
fn normal(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        if m.get("type").and_then(Value::as_str) == Some("stdio") {
            m.remove("type");
        }
    }
    v
}

fn detail_of(v: &Value) -> String {
    if kind_of(v) == "stdio" {
        let cmd = v.get("command").and_then(Value::as_str).unwrap_or("?");
        let n = v.get("args").and_then(Value::as_array).map_or(0, Vec::len);
        if n == 0 {
            cmd.to_string()
        } else {
            format!("{cmd} (+{n} argument{})", if n == 1 { "" } else { "s" })
        }
    } else {
        // An address may carry a key in its query.
        let url = v.get("url").and_then(Value::as_str).unwrap_or("?");
        url.split(['?', '#']).next().unwrap_or(url).to_string()
    }
}

pub fn overview(ctx: &Ctx) -> Overview {
    let mut warnings = Vec::new();
    let mut read = |path: &Path| match read_servers(path) {
        Ok(m) => m,
        Err(e) => {
            warnings.push(format!("{e:#}"));
            Map::new()
        }
    };
    let d = read(&desktop_file(ctx));
    let c = read(code_file(ctx));
    let names: BTreeSet<&String> = d.keys().chain(c.keys()).collect();
    let entries = names
        .into_iter()
        .map(|name| {
            let (dv, cv) = (d.get(name), c.get(name));
            let shown = cv.or(dv).expect("the name came from one of them");
            Entry {
                name: name.clone(),
                kind: kind_of(shown),
                detail: detail_of(shown),
                desktop: dv.is_some(),
                code: cv.is_some(),
                same: matches!((dv, cv), (Some(a), Some(b)) if normal(a) == normal(b)),
            }
        })
        .collect();
    Overview { desktop_file: desktop_file(ctx), code_file: code_file(ctx).to_path_buf(), entries, warnings }
}

// ------------------------------------------------------------------------------------ planning

/// What copying `from` into the other side changes. `overwrite` lets a different server be replaced.
fn one_way(
    from: &Map<String, Value>,
    onto: &Map<String, Value>,
    to: Side,
    overwrite: bool,
    plan: &mut Plan,
    differing: &mut BTreeSet<String>,
) {
    for (name, v) in from {
        if to == Side::Desktop && kind_of(v) != "stdio" {
            plan.skipped
                .push((name.clone(), "a remote server; Claude Desktop adds those under Settings > Connectors".into()));
            continue;
        }
        match onto.get(name) {
            None => plan.changes.push(Change { name: name.clone(), to, replaces: false }),
            Some(existing) if normal(existing) == normal(v) => {}
            Some(_) if overwrite => plan.changes.push(Change { name: name.clone(), to, replaces: true }),
            Some(_) => {
                differing.insert(name.clone());
            }
        }
    }
}

pub fn plan(desktop: &Map<String, Value>, code: &Map<String, Value>, dir: Direction, overwrite: bool) -> Plan {
    let mut plan = Plan::default();
    let mut differing = BTreeSet::new();
    if matches!(dir, Direction::Both | Direction::ToCode) {
        one_way(desktop, code, Side::Code, overwrite && dir == Direction::ToCode, &mut plan, &mut differing);
    }
    if matches!(dir, Direction::Both | Direction::ToDesktop) {
        one_way(code, desktop, Side::Desktop, overwrite && dir == Direction::ToDesktop, &mut plan, &mut differing);
    }
    // A server both sides have differently is not copied either way unless one side was chosen.
    plan.differing = differing.into_iter().collect();
    plan
}

// ------------------------------------------------------------------------------------ writing

/// Put `servers` into the `mcpServers` of `path` (made if need be), keeping every other key. The
/// previous file is kept beside it, and put back if the new one cannot be read back correctly.
fn write_servers(path: &Path, servers: Map<String, Value>) -> Result<Option<PathBuf>> {
    let (mut root, backup) = if path.exists() {
        let root = match fsx::read_json(path, MAX_JSON).with_context(|| format!("cannot read {}", path.display()))? {
            Value::Object(m) => m,
            _ => bail!("{} is not a JSON object; left alone", path.display()),
        };
        (root, Some(backup_beside(path)?))
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        (Map::new(), None)
    };
    root.insert(KEY.into(), Value::Object(servers.clone()));
    let mut body = serde_json::to_vec_pretty(&Value::Object(root))?;
    body.push(b'\n');
    fsx::atomic_write(path, &body, None, false)?;
    let back = read_servers(path);
    if !back.as_ref().is_ok_and(|m| *m == servers) {
        match &backup {
            Some(b) => {
                let _ = std::fs::copy(b, path);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        bail!("{} did not read back as written; the previous file was put back", path.display());
    }
    Ok(backup)
}

/// Copy servers between the two as `dir` says. With `dry_run` nothing is changed.
pub fn share(ctx: &Ctx, dir: Direction, overwrite: bool, dry_run: bool) -> Result<Report> {
    let (dfile, cfile) = (desktop_file(ctx), code_file(ctx).to_path_buf());
    let (d, c) = (read_servers(&dfile)?, read_servers(&cfile)?);
    let plan = plan(&d, &c, dir, overwrite);
    let mut report = Report { dry_run, plan, backups: Vec::new(), written: Vec::new(), deferred: Vec::new() };
    if dry_run || report.plan.changes.is_empty() {
        return Ok(report);
    }
    let _lock = crate::apply::Lock::acquire(ctx, Duration::from_secs(30))?;
    for side in [Side::Code, Side::Desktop] {
        let mine: Vec<&Change> = report.plan.changes.iter().filter(|ch| ch.to == side).collect();
        if mine.is_empty() {
            continue;
        }
        if side == Side::Desktop && desktop::is_running(ctx) {
            report.deferred.push((
                Side::Desktop,
                "it is open and rewrites its file when it quits; quit it (`claude-companion quit`), then run this again".into(),
            ));
            continue;
        }
        let (mut target, source) = if side == Side::Code { (c.clone(), &d) } else { (d.clone(), &c) };
        for ch in &mine {
            let mut v = source[&ch.name].clone();
            if side == Side::Desktop {
                v = normal(&v);
            }
            target.insert(ch.name.clone(), v);
        }
        let path = if side == Side::Code { &cfile } else { &dfile };
        if let Some(b) = write_servers(path, target)? {
            report.backups.push(b);
        }
        report.written.push(side);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Env {
        _tmp: tempfile::TempDir,
        ctx: Ctx,
    }

    fn env(desktop_running: bool, desktop: Option<Value>, code: Option<Value>) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let state = tmp.path().join("state");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let paths = crate::Paths::new(data.clone(), state);
        let paths = crate::Paths { claude_json: tmp.path().join(".claude.json"), ..paths };
        if let Some(v) = desktop {
            std::fs::write(data.join("claude_desktop_config.json"), v.to_string()).unwrap();
        }
        if let Some(v) = code {
            std::fs::write(&paths.claude_json, v.to_string()).unwrap();
        }
        let ctx = Ctx::new(
            paths,
            crate::Config::default(),
            crate::FakeDesktop { running: Some(desktop_running), active: None },
            crate::LogSink::Silent,
        );
        Env { _tmp: tmp, ctx }
    }

    fn servers(path: &Path) -> Map<String, Value> {
        read_servers(path).unwrap()
    }

    #[test]
    fn each_side_gets_what_the_other_has_and_other_keys_stay() {
        let e = env(
            false,
            Some(json!({"theme": "dark", "mcpServers": {"files": {"command": "npx", "args": ["fs"]}}})),
            Some(json!({"numStartups": 7, "mcpServers": {
                "git": {"type": "stdio", "command": "uvx", "args": ["git"], "env": {"A": "b"}},
                "remote": {"type": "http", "url": "https://x.test/mcp?key=1"}
            }})),
        );
        let r = share(&e.ctx, Direction::Both, false, false).unwrap();
        assert_eq!(r.written, [Side::Code, Side::Desktop]);
        assert_eq!(r.plan.skipped.len(), 1, "a remote server cannot go to Desktop");
        let desktop = servers(&desktop_file(&e.ctx));
        assert_eq!(
            desktop["git"],
            json!({"command": "uvx", "args": ["git"], "env": {"A": "b"}}),
            "no type for Desktop"
        );
        assert!(!desktop.contains_key("remote"));
        let code = servers(code_file(&e.ctx));
        assert_eq!(code["files"]["command"], json!("npx"));
        // Everything else in both files is as it was.
        let d: Value = serde_json::from_slice(&std::fs::read(desktop_file(&e.ctx)).unwrap()).unwrap();
        assert_eq!(d["theme"], json!("dark"));
        let c: Value = serde_json::from_slice(&std::fs::read(code_file(&e.ctx)).unwrap()).unwrap();
        assert_eq!(c["numStartups"], json!(7));
        assert_eq!(r.backups.len(), 2);
        assert!(r.backups.iter().all(|b| b.exists()));
        // A second run has nothing left to do.
        let again = share(&e.ctx, Direction::Both, false, false).unwrap();
        assert!(again.plan.changes.is_empty() && again.written.is_empty());
    }

    #[test]
    fn a_server_that_differs_is_kept_unless_a_side_is_chosen() {
        let e = env(
            false,
            Some(json!({"mcpServers": {"s": {"command": "old"}}})),
            Some(json!({"mcpServers": {"s": {"command": "new"}}})),
        );
        let r = share(&e.ctx, Direction::Both, true, false).unwrap();
        assert_eq!(r.plan.differing, ["s"], "both ways with --overwrite still has no winner");
        assert!(r.written.is_empty());
        let r = share(&e.ctx, Direction::ToDesktop, true, false).unwrap();
        assert_eq!(r.plan.changes, [Change { name: "s".into(), to: Side::Desktop, replaces: true }]);
        assert_eq!(servers(&desktop_file(&e.ctx))["s"]["command"], json!("new"));
        assert_eq!(servers(code_file(&e.ctx))["s"]["command"], json!("new"));
    }

    #[test]
    fn dry_runs_change_nothing_and_an_open_desktop_is_not_written() {
        let e = env(
            true,
            Some(json!({"mcpServers": {"a": {"command": "x"}}})),
            Some(json!({"mcpServers": {"b": {"command": "y"}}})),
        );
        let before = std::fs::read(code_file(&e.ctx)).unwrap();
        let dry = share(&e.ctx, Direction::Both, false, true).unwrap();
        assert_eq!(dry.plan.changes.len(), 2);
        assert_eq!(std::fs::read(code_file(&e.ctx)).unwrap(), before);
        let real = share(&e.ctx, Direction::Both, false, false).unwrap();
        assert_eq!(real.written, [Side::Code], "Code's file may change; Desktop's waits until it is closed");
        assert_eq!(real.deferred.len(), 1);
        assert!(!servers(&desktop_file(&e.ctx)).contains_key("b"));
        assert!(servers(code_file(&e.ctx)).contains_key("a"));
    }

    #[test]
    fn a_missing_file_is_made_and_a_broken_one_is_left_alone() {
        let e = env(false, Some(json!({"mcpServers": {"a": {"command": "x"}}})), None);
        share(&e.ctx, Direction::ToCode, false, false).unwrap();
        assert!(servers(code_file(&e.ctx)).contains_key("a"));
        std::fs::write(code_file(&e.ctx), "{ not json").unwrap();
        assert!(share(&e.ctx, Direction::ToCode, false, false).is_err());
        assert_eq!(std::fs::read_to_string(code_file(&e.ctx)).unwrap(), "{ not json");
        assert!(!overview(&e.ctx).warnings.is_empty());
    }

    #[test]
    fn the_overview_shows_commands_but_not_arguments_or_keys() {
        let e = env(
            false,
            Some(json!({"mcpServers": {"s": {"command": "npx", "args": ["--token=SECRET"]}}})),
            Some(json!({"mcpServers": {"r": {"type": "sse", "url": "https://h.test/sse?key=SECRET"}}})),
        );
        let ov = overview(&e.ctx);
        let text = serde_json::to_string(&ov.entries).unwrap();
        assert!(!text.contains("SECRET"), "{text}");
        assert_eq!(ov.entries[0].detail, "https://h.test/sse");
        assert_eq!(ov.entries[1].detail, "npx (+1 argument)");
        assert_eq!((ov.entries[1].desktop, ov.entries[1].code), (true, false));
    }
}
