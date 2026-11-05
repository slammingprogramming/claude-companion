// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! Claude Code, the terminal program, next to Claude Desktop: where it is, who is signed in to it,
//! its sessions, its settings, and starting it in a folder or in a new terminal window.
//!
//! Nothing here changes Claude Code's files unless a command says so (`settings_set`), and every
//! such change keeps the previous file beside it.

use crate::transcript;
use crate::{chats, desktop, fsx, Ctx};
use anyhow::{bail, Context as _, Result};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const MAX_JSON: u64 = 64 * 1024 * 1024;
/// Names (case-insensitive parts) whose values are never shown.
const SECRET_HINTS: &[&str] = &["key", "token", "secret", "password", "passwd", "credential", "auth", "bearer"];

/// What can be said about Claude Code on this computer.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    /// The `claude` program, when one was found.
    pub cli: Option<PathBuf>,
    pub version: Option<String>,
    /// The account it is signed in to, from `~/.claude.json`.
    pub email: Option<String>,
    pub config_dir: PathBuf,
    pub settings_file: PathBuf,
    pub settings_exist: bool,
    pub projects: usize,
    pub sessions: usize,
    /// MCP servers in its user scope.
    pub mcp_servers: usize,
}

/// A terminal session: one transcript of Claude Code.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub model: String,
    pub branch: String,
    /// Unix milliseconds of its last message (or of the file).
    pub last_ms: i64,
    pub size: u64,
    pub path: PathBuf,
    /// Whether a Claude Desktop chat card also uses this transcript (a Code session of Desktop).
    pub desktop: bool,
}

// ------------------------------------------------------------------------------------- program

/// Claude Desktop's `claude.exe`, which is not the Claude Code CLI that shares its name.
fn is_desktop_program(path: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase().replace('/', "\\");
    p.contains("anthropicclaude") || p.contains("\\windowsapps\\claude") || p.contains("\\app-")
}

/// The `claude` program: `CLAUDE_COMPANION_CLAUDE`, else the first one on the PATH or in the places
/// its installers use.
pub fn find_cli() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CLAUDE_COMPANION_CLAUDE").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let home = crate::paths::home();
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend([
        home.join(".local").join("bin"),
        home.join(".claude").join("local"),
        home.join(".npm-global").join("bin"),
        home.join(".bun").join("bin"),
    ]);
    if cfg!(windows) {
        if let Some(roaming) = dirs::config_dir() {
            dirs.push(roaming.join("npm"));
        }
    } else {
        dirs.extend(["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"].map(PathBuf::from));
    }
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file() && !(cfg!(windows) && is_desktop_program(p)))
}

fn require_cli() -> Result<PathBuf> {
    find_cli().context(
        "Claude Code (the `claude` program) was not found. Install it from https://claude.com/product/claude-code, \
         or point CLAUDE_COMPANION_CLAUDE at it",
    )
}

/// Keep a console window from flashing up when a command runs from the app.
fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// `claude --version`, such as `2.1.250`; `None` when it does not answer in a few seconds.
pub fn version(cli: &Path) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let cli = cli.to_path_buf();
    std::thread::spawn(move || {
        let out = quiet(Command::new(&cli).arg("--version").stdin(Stdio::null()).stderr(Stdio::null())).output();
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(Duration::from_secs(8)).ok()?.ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

// -------------------------------------------------------------------------------------- status

fn read_object(path: &Path) -> Option<Map<String, Value>> {
    match fsx::read_json(path, MAX_JSON) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// The email of the account Claude Code is signed in to.
pub fn signed_in_email(ctx: &Ctx) -> Option<String> {
    let root = read_object(&ctx.paths.claude_json)?;
    root.get("oauthAccount")?.get("emailAddress")?.as_str().map(str::to_string)
}

pub fn status(ctx: &Ctx, ask_version: bool) -> Status {
    let cli = find_cli();
    let version = if ask_version { cli.as_deref().and_then(version) } else { None };
    let projects = ctx
        .paths
        .projects
        .iter()
        .map(|r| std::fs::read_dir(r).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).count())
        .sum();
    let servers = read_object(&ctx.paths.claude_json)
        .and_then(|m| m.get("mcpServers").and_then(Value::as_object).map(Map::len))
        .unwrap_or(0);
    Status {
        cli,
        version,
        email: signed_in_email(ctx),
        config_dir: ctx.paths.claude_settings.parent().map(Path::to_path_buf).unwrap_or_default(),
        settings_file: ctx.paths.claude_settings.clone(),
        settings_exist: ctx.paths.claude_settings.is_file(),
        projects,
        sessions: transcript::transcript_paths(ctx).len(),
        mcp_servers: servers,
    }
}

// ------------------------------------------------------------------------------------ sessions

fn norm_dir(s: &str) -> String {
    let s = s.replace('\\', "/");
    let s = s.trim_end_matches('/');
    if cfg!(windows) || cfg!(target_os = "macos") {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

fn mtime_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// The terminal sessions on disk, newest first: at most `limit`, optionally only those that worked in
/// `dir`.
pub fn sessions(ctx: &Ctx, dir: Option<&Path>, limit: usize) -> Vec<Session> {
    let used_by_desktop: HashSet<String> = chats::list(ctx).into_iter().filter_map(|c| c.transcript).collect();
    let mut files: Vec<(i64, String, PathBuf)> =
        transcript::transcript_paths(ctx).into_iter().map(|(id, p)| (mtime_ms(&p), id, p)).collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let wanted = dir.map(|d| norm_dir(&d.to_string_lossy()));
    let mut out = Vec::new();
    for (mtime, id, path) in files {
        if out.len() >= limit {
            break;
        }
        let Ok(m) = transcript::quick_meta(&path) else { continue };
        if !m.is_conversation() || wanted.as_ref().is_some_and(|w| norm_dir(&m.cwd) != *w) {
            continue;
        }
        out.push(Session {
            desktop: used_by_desktop.contains(&id),
            last_ms: m.end_ms.unwrap_or(mtime),
            title: if m.title.is_empty() { transcript::one_line(&m.first_prompt, 80) } else { m.title },
            cwd: m.cwd,
            model: m.model,
            branch: m.branch,
            size: m.size,
            path,
            id,
        });
    }
    out
}

/// One session named by `latest` or by its id or the start of it.
pub fn resolve_session(ctx: &Ctx, query: &str) -> Result<Session> {
    let q = query.trim().to_lowercase();
    if matches!(q.as_str(), "latest" | "last") {
        return sessions(ctx, None, 1).into_iter().next().context("there are no terminal sessions yet");
    }
    anyhow::ensure!(!q.is_empty(), "name a session by its id (or the start of it), or `latest`");
    let found: Vec<(String, PathBuf)> =
        transcript::transcript_paths(ctx).into_iter().filter(|(id, _)| id.to_lowercase().starts_with(&q)).collect();
    match found.as_slice() {
        [] => bail!("no session starts with {query} (see `claude-companion code sessions`)"),
        [(id, path)] => {
            let m = transcript::quick_meta(path)?;
            let used = chats::list(ctx).into_iter().any(|c| c.transcript.as_deref() == Some(id));
            Ok(Session {
                id: id.clone(),
                cwd: m.cwd,
                title: if m.title.is_empty() { transcript::one_line(&m.first_prompt, 80) } else { m.title },
                model: m.model,
                branch: m.branch,
                last_ms: m.end_ms.unwrap_or_else(|| mtime_ms(path)),
                size: m.size,
                path: path.clone(),
                desktop: used,
            })
        }
        many => bail!("{query} fits {} sessions; give more of the id", many.len()),
    }
}

// ----------------------------------------------------------------------------------- starting

/// What it takes to open a terminal window running a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// A command to run to open a terminal window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub program: String,
    pub args: Vec<String>,
    /// Where it runs; terminals that take no folder start there.
    pub cwd: Option<PathBuf>,
}

/// `'text'` for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The inside of an AppleScript string.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The terminal window that runs `cli args` in `dir` on `os`, using the first of the known terminals
/// that `have` finds. `None` when there is none.
pub fn terminal_command(
    os: Os,
    dir: &Path,
    cli: &Path,
    args: &[String],
    have: &dyn Fn(&str) -> bool,
) -> Option<Terminal> {
    let cli_s = cli.to_string_lossy().into_owned();
    match os {
        Os::Windows => {
            if have("wt.exe") {
                let mut a = vec!["-d".to_string(), dir.to_string_lossy().into_owned(), cli_s];
                a.extend(args.iter().cloned());
                Some(Terminal { program: "wt.exe".into(), args: a, cwd: None })
            } else {
                // `start` opens a console window of its own; `cmd /K` keeps it after Claude exits.
                let mut a = vec![
                    "/C".to_string(),
                    "start".into(),
                    "Claude Code".into(),
                    "/D".into(),
                    dir.to_string_lossy().into_owned(),
                    "cmd".into(),
                    "/K".into(),
                    cli_s,
                ];
                a.extend(args.iter().cloned());
                Some(Terminal { program: "cmd.exe".into(), args: a, cwd: None })
            }
        }
        Os::Mac => {
            let mut line = format!("cd {} && {}", sh_quote(&dir.to_string_lossy()), sh_quote(&cli_s));
            for a in args {
                line.push(' ');
                line.push_str(&sh_quote(a));
            }
            Some(Terminal {
                program: "/usr/bin/osascript".into(),
                args: vec![
                    "-e".into(),
                    format!("tell application \"Terminal\" to do script \"{}\"", applescript_escape(&line)),
                    "-e".into(),
                    "tell application \"Terminal\" to activate".into(),
                ],
                cwd: None,
            })
        }
        Os::Linux => {
            // (program, the arguments that precede the command to run)
            const KNOWN: &[(&str, &[&str])] = &[
                ("x-terminal-emulator", &["-e"]),
                ("gnome-terminal", &["--"]),
                ("konsole", &["-e"]),
                ("xfce4-terminal", &["-x"]),
                ("kitty", &[]),
                ("alacritty", &["-e"]),
                ("xterm", &["-e"]),
            ];
            let (program, lead) = KNOWN.iter().find(|(p, _)| have(p))?;
            let mut a: Vec<String> = lead.iter().map(|s| s.to_string()).collect();
            a.push(cli_s);
            a.extend(args.iter().cloned());
            Some(Terminal { program: program.to_string(), args: a, cwd: Some(dir.to_path_buf()) })
        }
    }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// Run Claude Code in this terminal, in `dir`, and wait for it; returns its exit code.
pub fn run_here(dir: Option<&Path>, args: &[String]) -> Result<i32> {
    let cli = require_cli()?;
    let mut cmd = Command::new(&cli);
    cmd.args(args);
    if let Some(dir) = dir {
        anyhow::ensure!(dir.is_dir(), "{} is not a folder", dir.display());
        cmd.current_dir(dir);
    }
    let status = cmd.status().with_context(|| format!("starting {}", cli.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// Open Claude Code in a new terminal window, in `dir`.
pub fn open_window(dir: &Path, args: &[String]) -> Result<()> {
    let cli = require_cli()?;
    anyhow::ensure!(dir.is_dir(), "{} is not a folder", dir.display());
    let term = terminal_command(Os::current(), dir, &cli, args, &on_path).context(
        "no terminal program was found to open a window in (set CLAUDE_COMPANION_CLAUDE or use `code run` here)",
    )?;
    let mut cmd = Command::new(&term.program);
    cmd.args(&term.args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(cwd) = &term.cwd {
        cmd.current_dir(cwd);
    }
    cmd.spawn().with_context(|| format!("starting {}", term.program))?;
    Ok(())
}

/// The arguments that continue `session`, and the folder to run them in (Claude Code looks for a
/// session in the folder it starts in).
pub fn resume_target(session: &Session) -> Result<(PathBuf, Vec<String>)> {
    let dir = PathBuf::from(&session.cwd);
    anyhow::ensure!(
        dir.is_dir(),
        "the folder this session worked in no longer exists ({}); Claude Code can only continue it from there",
        session.cwd
    );
    Ok((dir, vec!["--resume".into(), session.id.clone()]))
}

/// Start Claude Desktop (waiting out an update of it first).
pub fn open_desktop(ctx: &Ctx) -> Result<()> {
    desktop::wait_for_update(ctx, Duration::from_secs(60))?;
    if desktop::is_running(ctx) {
        return Ok(());
    }
    desktop::launch(ctx)
}

// -------------------------------------------------------------------------------- settings

/// A copy of `path` kept beside it as `<name>.claude-companion-backup-<time>`.
pub(crate) fn backup_beside(path: &Path) -> Result<PathBuf> {
    let mut name = path.file_name().context("no file name")?.to_os_string();
    name.push(format!(".claude-companion-backup-{}", crate::snapshot::stamp()));
    let b = fsx::unique_path(path.with_file_name(name));
    std::fs::copy(path, &b).with_context(|| format!("backing up {}", path.display()))?;
    Ok(b)
}

/// Whether a key name suggests a secret.
fn secretive(name: &str) -> bool {
    let n = name.to_lowercase();
    SECRET_HINTS.iter().any(|h| n.contains(h))
}

/// `value` with the values of secret-looking keys, and of every `env` and `headers` entry, hidden.
pub fn redact(value: &Value) -> Value {
    fn walk(v: &Value, hide: bool) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, v)| (k.clone(), walk(v, hide || secretive(k) || matches!(k.as_str(), "env" | "headers"))))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(|v| walk(v, hide)).collect()),
            Value::Null => Value::Null,
            _ if hide => Value::String("***".into()),
            other => other.clone(),
        }
    }
    walk(value, false)
}

/// Claude Code's user settings, as a JSON object (empty when the file is missing).
pub fn settings(ctx: &Ctx) -> Result<Map<String, Value>> {
    let path = &ctx.paths.claude_settings;
    if !path.exists() {
        return Ok(Map::new());
    }
    match fsx::read_json(path, MAX_JSON).with_context(|| format!("cannot read {}", path.display()))? {
        Value::Object(m) => Ok(m),
        _ => bail!("{} is not a JSON object", path.display()),
    }
}

/// The value at a dotted path such as `permissions.defaultMode`.
pub fn get_path<'a>(map: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let mut cur = map.get(parts.next()?)?;
    for p in parts {
        cur = cur.get(p)?;
    }
    Some(cur)
}

/// Set (or with `None` remove) the value at a dotted path, creating the objects on the way.
pub fn set_path(map: &mut Map<String, Value>, path: &str, value: Option<Value>) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    anyhow::ensure!(parts.iter().all(|p| !p.is_empty()), "{path:?} is not a setting name");
    let (last, init) = parts.split_last().expect("split gives one part");
    let mut cur = map;
    for p in init {
        if value.is_none() && !cur.contains_key(*p) {
            return Ok(());
        }
        let next = cur.entry(p.to_string()).or_insert_with(|| Value::Object(Map::new()));
        match next {
            Value::Object(m) => cur = m,
            _ => bail!("{p} is not an object, so {path} cannot be set"),
        }
    }
    match value {
        Some(v) => cur.insert(last.to_string(), v),
        None => cur.remove(*last),
    };
    Ok(())
}

/// A command-line value: JSON when it is JSON (`true`, `30`, `["a"]`), else text.
pub fn parse_value(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

/// Change one setting of Claude Code's user settings, keeping every other key, and the previous
/// file beside it. Returns the backup's path (`None` when there was no file).
pub fn settings_set(ctx: &Ctx, path: &str, value: Option<Value>) -> Result<Option<PathBuf>> {
    let file = &ctx.paths.claude_settings;
    let mut map = settings(ctx)?;
    let backup = if file.exists() { Some(backup_beside(file)?) } else { None };
    set_path(&mut map, path, value)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut body = serde_json::to_vec_pretty(&Value::Object(map))?;
    body.push(b'\n');
    fsx::atomic_write(file, &body, None, false)?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn have(names: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |n| names.contains(&n)
    }

    #[test]
    fn the_desktop_program_is_not_claude_code() {
        assert!(is_desktop_program(Path::new(r"C:\Users\a\AppData\Local\AnthropicClaude\app-1.0\claude.exe")));
        assert!(is_desktop_program(Path::new(r"C:\Program Files\WindowsApps\Claude_1.0_x64__x\app\claude.exe")));
        assert!(!is_desktop_program(Path::new(r"C:\Users\a\.local\bin\claude.exe")));
        assert!(!is_desktop_program(Path::new(r"C:\Users\a\AppData\Roaming\npm\claude.cmd")));
    }

    #[test]
    fn windows_prefers_windows_terminal_and_falls_back_to_a_console() {
        let (dir, cli) = (Path::new(r"C:\work"), Path::new(r"C:\bin\claude.exe"));
        let args = vec!["--resume".to_string(), "abc".to_string()];
        let wt = terminal_command(Os::Windows, dir, cli, &args, &have(&["wt.exe"])).unwrap();
        assert_eq!(wt.program, "wt.exe");
        assert_eq!(wt.args, ["-d", r"C:\work", r"C:\bin\claude.exe", "--resume", "abc"]);
        let cmd = terminal_command(Os::Windows, dir, cli, &args, &have(&[])).unwrap();
        assert_eq!(cmd.program, "cmd.exe");
        assert_eq!(&cmd.args[..4], ["/C", "start", "Claude Code", "/D"]);
        assert_eq!(cmd.args.last().unwrap(), "abc");
    }

    #[test]
    fn mac_quotes_what_it_hands_to_the_shell_and_to_applescript() {
        let t = terminal_command(
            Os::Mac,
            Path::new("/Users/a/it's \"here\""),
            Path::new("/usr/local/bin/claude"),
            &["--resume".into(), "x y".into()],
            &have(&[]),
        )
        .unwrap();
        assert_eq!(t.program, "/usr/bin/osascript");
        let script = &t.args[1];
        let expected = "cd '/Users/a/it'\\\\''s \\\"here\\\"' && '/usr/local/bin/claude' '--resume' 'x y'";
        assert!(script.contains(expected), "{script}");
    }

    #[test]
    fn linux_uses_the_first_terminal_it_finds_and_runs_in_the_folder() {
        let (dir, cli) = (Path::new("/home/a/p"), Path::new("/usr/bin/claude"));
        let t = terminal_command(Os::Linux, dir, cli, &[], &have(&["konsole", "xterm"])).unwrap();
        assert_eq!(
            (t.program.as_str(), t.args.as_slice()),
            ("konsole", &["-e".to_string(), "/usr/bin/claude".to_string()][..])
        );
        assert_eq!(t.cwd.as_deref(), Some(dir));
        assert!(terminal_command(Os::Linux, dir, cli, &[], &have(&[])).is_none());
    }

    #[test]
    fn settings_paths_are_read_and_set_without_losing_neighbours() {
        let mut m = json!({"model": "opus", "permissions": {"allow": ["Bash"], "defaultMode": "plan"}})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(get_path(&m, "permissions.defaultMode"), Some(&json!("plan")));
        set_path(&mut m, "permissions.defaultMode", Some(json!("acceptEdits"))).unwrap();
        set_path(&mut m, "env.FOO", Some(json!("1"))).unwrap();
        assert_eq!(m["permissions"]["allow"], json!(["Bash"]));
        assert_eq!(m["env"]["FOO"], json!("1"));
        set_path(&mut m, "model", None).unwrap();
        assert!(!m.contains_key("model"));
        set_path(&mut m, "nothing.here", None).unwrap();
        assert!(!m.contains_key("nothing"), "removing a missing setting creates nothing");
        assert!(set_path(&mut m, "permissions.allow.x", Some(json!(1))).is_err());
        assert!(set_path(&mut m, "a..b", Some(json!(1))).is_err());
        assert_eq!(parse_value("30"), json!(30));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("opus"), json!("opus"));
    }

    #[test]
    fn secrets_are_hidden_when_settings_are_shown() {
        let v = json!({
            "model": "opus",
            "env": {"ANTHROPIC_API_KEY": "sk-1", "FOO": "bar"},
            "apiKeyHelper": "get-key.sh",
            "mcpServers": {"a": {"command": "npx", "args": ["x"], "headers": {"X": "y"}}}
        });
        let r = redact(&v);
        assert_eq!(r["model"], json!("opus"));
        assert_eq!(r["env"]["ANTHROPIC_API_KEY"], json!("***"));
        assert_eq!(r["env"]["FOO"], json!("***"), "every env value is hidden");
        assert_eq!(r["apiKeyHelper"], json!("***"));
        assert_eq!(r["mcpServers"]["a"]["command"], json!("npx"));
        assert_eq!(r["mcpServers"]["a"]["headers"]["X"], json!("***"));
        assert!(!r.to_string().contains("sk-1"));
    }

    #[test]
    fn sessions_are_listed_newest_first_and_found_by_the_start_of_their_id() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects").join("p1");
        std::fs::create_dir_all(&projects).unwrap();
        let line = |cwd: &str, text: &str| {
            format!(
                "{}\n",
                json!({"type": "user", "cwd": cwd, "sessionId": "x", "timestamp": "2026-10-01T10:00:00Z",
                       "message": {"role": "user", "content": text}})
            )
        };
        std::fs::write(projects.join("aaaa1111-0000-0000-0000-000000000001.jsonl"), line("/w/one", "first job"))
            .unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(projects.join("bbbb2222-0000-0000-0000-000000000002.jsonl"), line("/w/two", "second job"))
            .unwrap();
        let paths = crate::Paths::new(tmp.path().join("data"), tmp.path().join("state"));
        let paths = crate::Paths { projects: vec![tmp.path().join("projects")], ..paths };
        let ctx = Ctx::new(
            paths,
            crate::Config::default(),
            crate::FakeDesktop { running: Some(false), active: None },
            crate::LogSink::Silent,
        );
        let all = sessions(&ctx, None, 10);
        assert_eq!(all.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["second job", "first job"]);
        assert_eq!(sessions(&ctx, Some(Path::new("/w/one/")), 10).len(), 1);
        assert_eq!(sessions(&ctx, None, 1).len(), 1);
        assert_eq!(resolve_session(&ctx, "aaaa").unwrap().cwd, "/w/one");
        assert_eq!(resolve_session(&ctx, "latest").unwrap().cwd, "/w/two");
        assert!(resolve_session(&ctx, "zzzz").is_err());
        assert!(resolve_session(&ctx, "").is_err());
    }
}
