// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
//! `claude-companion`: one place to control Claude Desktop and Claude Code (the terminal program),
//! and to keep their sessions identical across all your accounts.

mod chats_cmd;
mod code_cmd;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};
use claude_companion_core::about;
use claude_companion_core::accounts::{self, NotFound, Roster, Slot, Strategy};
use claude_companion_core::desktop::{self, Quitting};
use claude_companion_core::diagnose;
use claude_companion_core::groups;
use claude_companion_core::logins;
use claude_companion_core::report;
use claude_companion_core::retention::{self, Kept, Limit};
use claude_companion_core::service::{self, Heartbeat};
use claude_companion_core::usage::Usage;
use claude_companion_core::watch::{self, WatchOptions};
use claude_companion_core::{
    apply, fsx, plan, scan, short, snapshot, Config, Ctx, FakeDesktop, LogSink, Paths, State, Surface,
};
use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "claude-companion",
    version,
    about = "Control Claude Desktop and Claude Code (the terminal program) from one place: accounts, sessions, MCP servers, settings."
)]
struct Cli {
    /// Claude Desktop's data folder (default: this platform's location)
    #[arg(long, global = true, env = "CLAUDE_COMPANION_USER_DATA", value_name = "DIR")]
    user_data: Option<PathBuf>,
    /// Where claude-companion keeps its config, snapshots and trash
    #[arg(long, global = true, env = "CLAUDE_COMPANION_STATE_DIR", value_name = "DIR")]
    state_dir: Option<PathBuf>,
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ExportFormat {
    /// Markdown
    Md,
    /// A self-contained web page
    Html,
}

#[derive(Subcommand)]
enum ChatsCmd {
    /// Every chat in every account's list
    List {
        /// Only this account's (its number, alias, email or id)
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Chats whose history is on disk but that no account shows
    Lost {
        #[arg(long)]
        json: bool,
    },
    /// Give lost chats a card again, so Claude shows them (quit Claude first)
    Rescue {
        /// Transcript ids (or their first characters); all lost chats without any
        ids: Vec<String>,
        /// The account whose list gets them
        #[arg(long)]
        to: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Put chats of one account into another's list; newer cards win (quit Claude first)
    Copy {
        /// Chat ids (or their first characters); every chat without any
        ids: Vec<String>,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Write a chat as Markdown or a web page
    Export {
        /// The start of a chat's id (card or transcript)
        chat: String,
        #[arg(long, value_enum, default_value_t = ExportFormat::Md)]
        format: ExportFormat,
        /// Write to this file instead of the terminal
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// Which side receives MCP servers.
#[derive(Clone, Copy, ValueEnum)]
pub enum ShareTo {
    /// Claude Code's servers go into Claude Desktop
    Desktop,
    /// Claude Desktop's servers go into Claude Code
    Code,
}

#[derive(Clone, Copy, ValueEnum)]
enum OpenWhat {
    /// The Claude Desktop app
    Desktop,
    /// Claude Code in a new terminal window
    Code,
}

#[derive(Subcommand)]
enum CodeCmd {
    /// Where Claude Code is, who it is signed in as, and what it has
    Status {
        #[arg(long)]
        json: bool,
    },
    /// The terminal sessions on this computer, newest first
    Sessions {
        /// Only those that worked in this folder
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        /// Leave out sessions that Claude Desktop also shows
        #[arg(long)]
        terminal_only: bool,
        #[arg(long)]
        json: bool,
    },
    /// Continue a session (its id, the start of one, or `latest`) in the folder it worked in
    Resume {
        session: String,
        /// Open a new terminal window instead of using this one
        #[arg(long)]
        new_window: bool,
    },
    /// Start Claude Code here, or in a new window; anything after `--` goes to `claude`
    Run {
        /// Folder to start in (default: this one)
        dir: Option<PathBuf>,
        /// Open a new terminal window instead of using this one
        #[arg(long)]
        new_window: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// The account Claude Code is signed in to: save it, list saved ones, switch, forget
    Login {
        #[command(subcommand)]
        cmd: LoginCmd,
    },
    /// Show or change Claude Code's user settings (`model`, `permissions.defaultMode`, …)
    Settings {
        /// A setting, with dots for nesting: `permissions.defaultMode`
        key: Option<String>,
        /// The new value: JSON (`true`, `30`, `["a"]`) or text
        value: Option<String>,
        /// Remove the setting
        #[arg(long, requires = "key", conflicts_with = "value")]
        unset: bool,
    },
}

#[derive(Subcommand)]
enum LoginCmd {
    /// The saved sign-ins, and which one Claude Code has now
    List {
        #[arg(long)]
        json: bool,
    },
    /// Save the sign-in Claude Code has now (sign in with /login in `claude` first)
    Save,
    /// Make Claude Code use a saved sign-in (its number, email or the start of its id)
    Use { account: String },
    /// Delete a saved sign-in; Claude Code stays as it is
    Forget {
        account: String,
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// The MCP servers of Claude Desktop and Claude Code side by side
    List {
        #[arg(long)]
        json: bool,
    },
    /// Copy servers one side lacks into the other (a different server of the same name is kept)
    Share {
        /// Only this direction (default: both ways)
        #[arg(long, value_enum)]
        to: Option<ShareTo>,
        /// Replace a different server of the same name (needs --to)
        #[arg(long)]
        overwrite: bool,
        /// Show what would change without touching anything
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum Cmd {
    /// Read-only check of every account's session index
    Doctor,
    /// Show what a sync would change (dry run)
    Plan,
    /// Sync once
    Sync {
        #[arg(short, long)]
        yes: bool,
    },
    /// Keep syncing (the background agent runs this)
    Watch {
        /// Seconds between checks
        #[arg(long, default_value_t = 2.0)]
        interval: f64,
        #[arg(long, hide = true)]
        iterations: Option<u64>,
        /// Log to the terminal instead of the log file
        #[arg(long)]
        foreground: bool,
    },
    /// Sync now, then keep syncing in the background from login
    Install {
        #[arg(short, long)]
        yes: bool,
    },
    /// Stop background syncing (every account keeps its full copy)
    Uninstall,
    /// Background sync and last sync
    Status,
    /// List snapshots
    Snapshots,
    /// Put every session index back as it was in a snapshot (quit Claude first)
    Restore {
        snapshot: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Turn symlinked index folders left by other tools into real folders (quit Claude first)
    FixSymlinks {
        #[arg(short, long)]
        yes: bool,
    },
    /// Show how long Claude Code keeps transcripts; keep them, undo that, or set cleanupPeriodDays
    Retention {
        /// Set cleanupPeriodDays (terminal sessions and other data)
        days: Option<u32>,
        /// Keep the transcripts of Desktop sessions (only needed when Claude Code would delete them)
        #[arg(long, conflicts_with_all = ["days", "undo"])]
        keep: bool,
        /// Put back the setting Claude Companion changed
        #[arg(long, conflicts_with = "days")]
        undo: bool,
    },
    /// Your accounts, numbered: which one Claude is signed in to, and what each has used
    #[command(visible_alias = "list")]
    Accounts {
        #[arg(long)]
        json: bool,
    },
    /// Switch Claude to another account, or the next one (Claude restarts if it is open)
    Switch {
        /// Its number, alias or email (default: the next account in the list)
        account: Option<String>,
        /// Without an account: the next one, the next one with plan left, or the one with the most left
        #[arg(long, value_enum, conflicts_with = "account")]
        strategy: Option<StrategyArg>,
        #[arg(long)]
        json: bool,
        /// End Claude if it has not quit when asked (on Windows, nothing else quits it from outside)
        #[arg(long)]
        force: bool,
    },
    /// Restart Claude signed out, to add an account; the current one is kept
    #[command(alias = "sign-in")]
    Add {
        /// End Claude if it has not quit when asked (on Windows, nothing else quits it from outside)
        #[arg(long)]
        force: bool,
    },
    /// Take an account off the list and forget the sign-in kept for it (its sessions stay)
    #[command(alias = "forget")]
    Remove {
        /// Its number, alias or email
        account: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Give an account a short name, or list them
    Alias {
        /// Its number, alias or email
        account: Option<String>,
        /// The new alias
        name: Option<String>,
        /// Take the alias away
        #[arg(long, requires = "account", conflicts_with = "name")]
        unset: bool,
    },
    /// Leave an account out of `switch` without an account (it can still be named)
    Disable { account: String },
    /// Put an account back in the rotation
    Enable { account: String },
    /// Give an account another number (the account that has it takes the old one)
    Move { account: String, number: u32 },
    /// Chats across accounts: list, find lost ones, rescue, copy, export
    Chats {
        #[command(subcommand)]
        cmd: ChatsCmd,
    },
    /// Tokens used by model, day and folder, counted from Claude Code's local transcripts
    Stats {
        #[arg(long)]
        json: bool,
    },
    /// Claude Code, the terminal program: status, sessions, resume, run, settings
    Code {
        #[command(subcommand)]
        cmd: CodeCmd,
    },
    /// MCP servers of Claude Desktop and Claude Code: list them, share them between the two
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// Start Claude Desktop, or Claude Code in a new terminal window
    Open {
        what: OpenWhat,
        /// For `code`: the folder to open it in (default: this one)
        dir: Option<PathBuf>,
    },
    /// Version, licence, how to contact the developer and how to support the project
    About {
        #[arg(long)]
        json: bool,
    },
    /// Quit Claude Desktop (on Windows it keeps running in the tray when its window closes)
    Quit {
        /// End Claude if it has not quit when asked
        #[arg(long)]
        force: bool,
    },
    /// Merge the sidebar groups of all accounts (Claude must be closed)
    Groups {
        /// Show what would change without touching anything
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show or change settings
    Config {
        /// Keep an account (or `<account>/<org>`) separate
        #[arg(long, value_name = "ID")]
        exclude: Vec<String>,
        /// Undo an --exclude
        #[arg(long, value_name = "ID")]
        include: Vec<String>,
        /// Let accounts that appear later join automatically
        #[arg(long)]
        auto_join: Option<OnOff>,
        /// Desktop notifications from the background agent
        #[arg(long)]
        notify: Option<OnOff>,
        /// Keep the sidebar groups of all accounts the same (only while Claude is closed)
        #[arg(long)]
        groups: Option<OnOff>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum StrategyArg {
    Next,
    NextAvailable,
    Best,
}

impl From<StrategyArg> for Strategy {
    fn from(s: StrategyArg) -> Strategy {
        match s {
            StrategyArg::Next => Strategy::Next,
            StrategyArg::NextAvailable => Strategy::NextAvailable,
            StrategyArg::Best => Strategy::Best,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("claude-companion: {e:#}");
        std::process::exit(2);
    }
}

fn run(cli: Cli) -> Result<()> {
    let detected = Paths::detect();
    let paths = Paths::new(cli.user_data.unwrap_or(detected.user_data), cli.state_dir.unwrap_or(detected.state_dir));
    let config = Config::load(&paths)?;
    let mut ctx = Ctx::new(paths, config, FakeDesktop::from_env(), LogSink::Stderr);
    ctx.verbose = cli.verbose;
    let Some(cmd) = cli.cmd else {
        return doctor(&ctx);
    };
    match cmd {
        Cmd::Doctor => doctor(&ctx),
        Cmd::Plan => show_plan(&ctx, cli.verbose),
        Cmd::Sync { yes } => sync(&ctx, yes),
        Cmd::Watch { interval, iterations, foreground } => {
            if !foreground {
                ctx.logger.set_sink(LogSink::File(ctx.paths.log_file.clone()));
            }
            let opts = WatchOptions { interval: Duration::from_secs_f64(interval.max(0.2)), iterations };
            watch::run(&ctx, &opts, &AtomicBool::new(false));
            Ok(())
        }
        Cmd::Install { yes } => install(&ctx, yes),
        Cmd::Uninstall => {
            service::uninstall(&ctx)?;
            println!("Background sync removed. Every account keeps its full copy of your sessions.");
            println!("Snapshots and trash stay in {}", ctx.paths.state_dir.display());
            Ok(())
        }
        Cmd::Status => status(&ctx),
        Cmd::Snapshots => snapshots(&ctx),
        Cmd::Restore { snapshot, yes } => {
            let manifest = snapshot::list(&ctx).into_iter().find(|m| m.id == snapshot);
            let Some(m) = manifest else { bail!("no such snapshot: {snapshot} (see `claude-companion snapshots`)") };
            println!(
                "Restore {} session folder(s) from {}. The current ones go to the trash first.",
                m.partitions.len(),
                m.id
            );
            if !yes && !confirm("Continue?") {
                return Ok(());
            }
            let pre = snapshot::restore(&ctx, &m.id)?;
            println!("Restored. The state before this restore is snapshot {pre}.");
            Ok(())
        }
        Cmd::FixSymlinks { yes } => {
            let todo = snapshot::symlinked_folders(&ctx);
            if todo.is_empty() {
                println!("No symlinked session folders.");
                return Ok(());
            }
            for t in &todo {
                let target = std::fs::read_link(t).map(|p| p.display().to_string()).unwrap_or_default();
                println!("  {} -> {target}", t.strip_prefix(&ctx.paths.user_data).unwrap_or(t).display());
            }
            println!("Each link becomes a real folder holding a copy of what it points to.");
            if !yes && !confirm("Continue?") {
                return Ok(());
            }
            let fixed = snapshot::fix_symlinks(&ctx)?;
            println!("Fixed {} folder(s). Run `claude-companion sync` to merge them.", fixed.len());
            Ok(())
        }
        Cmd::Retention { days, keep, undo } => retention(&ctx, days, keep, undo),
        Cmd::Accounts { json } => accounts(&ctx, json),
        Cmd::Switch { account, strategy, json, force } => {
            switch(&ctx, account.as_deref(), strategy.map(Into::into), json, force)
        }
        Cmd::Add { force } => add(&ctx, force),
        Cmd::Remove { account, yes } => remove(&ctx, &account, yes),
        Cmd::Alias { account, name, unset } => alias(&ctx, account.as_deref(), name.as_deref(), unset),
        Cmd::Disable { account } => sit_out(&ctx, &account, true),
        Cmd::Enable { account } => sit_out(&ctx, &account, false),
        Cmd::Move { account, number } => move_account(&ctx, &account, number),
        Cmd::Chats { cmd } => match cmd {
            ChatsCmd::List { account, json } => chats_cmd::list(&ctx, account.as_deref(), json),
            ChatsCmd::Lost { json } => chats_cmd::lost(&ctx, json),
            ChatsCmd::Rescue { ids, to, yes } => chats_cmd::rescue(&ctx, &ids, &to, yes),
            ChatsCmd::Copy { ids, from, to, yes } => chats_cmd::copy(&ctx, &ids, &from, &to, yes),
            ChatsCmd::Export { chat, format, out } => chats_cmd::export(&ctx, &chat, format, out),
        },
        Cmd::Stats { json } => chats_cmd::stats(&ctx, json),
        Cmd::Code { cmd } => match cmd {
            CodeCmd::Status { json } => code_cmd::status(&ctx, json),
            CodeCmd::Sessions { dir, limit, terminal_only, json } => {
                code_cmd::sessions(&ctx, dir.as_deref(), limit, terminal_only, json)
            }
            CodeCmd::Resume { session, new_window } => code_cmd::resume(&ctx, &session, new_window),
            CodeCmd::Run { dir, new_window, args } => code_cmd::run(dir, new_window, &args),
            CodeCmd::Login { cmd } => match cmd {
                LoginCmd::List { json } => code_cmd::login_list(&ctx, json),
                LoginCmd::Save => code_cmd::login_save(&ctx),
                LoginCmd::Use { account } => code_cmd::login_use(&ctx, &account),
                LoginCmd::Forget { account, yes } => {
                    if !yes && !confirm("Delete this saved sign-in? Claude Code itself stays signed in.") {
                        return Ok(());
                    }
                    code_cmd::login_forget(&ctx, &account)
                }
            },
            CodeCmd::Settings { key, value, unset } => {
                code_cmd::settings(&ctx, key.as_deref(), value.as_deref(), unset)
            }
        },
        Cmd::Mcp { cmd } => match cmd {
            McpCmd::List { json } => code_cmd::mcp_list(&ctx, json),
            McpCmd::Share { to, overwrite, dry_run, json } => code_cmd::mcp_share(&ctx, to, overwrite, dry_run, json),
        },
        Cmd::Open { what: OpenWhat::Desktop, .. } => code_cmd::open_desktop(&ctx),
        Cmd::Open { what: OpenWhat::Code, dir } => code_cmd::run(dir, true, &[]),
        Cmd::Quit { force } => code_cmd::quit_desktop(&ctx, force),
        Cmd::About { json } => {
            if json {
                let out = serde_json::json!({
                    "schemaVersion": 1,
                    "version": claude_companion_core::VERSION,
                    "license": "AGPL-3.0-or-later",
                    "source": about::PROJECT,
                    "contactSimplex": about::CONTACT_SIMPLEX,
                    "donateKofi": about::DONATE_KOFI,
                    "crypto": about::COINS,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                print!("{}", about::text());
            }
            Ok(())
        }
        Cmd::Groups { dry_run, json } => groups_cmd(&ctx, dry_run, json),
        Cmd::Config { exclude, include, auto_join, notify, groups } => {
            let mut cfg = ctx.config();
            let before = cfg.clone();
            for id in exclude {
                if !cfg.exclude.contains(&id) {
                    cfg.exclude.push(id);
                }
            }
            cfg.exclude.retain(|e| !include.contains(e));
            if let Some(v) = auto_join {
                cfg.auto_join_new = matches!(v, OnOff::On);
            }
            if let Some(v) = notify {
                cfg.notify = matches!(v, OnOff::On);
            }
            if let Some(v) = groups {
                cfg.sync_groups = matches!(v, OnOff::On);
            }
            if cfg != before {
                cfg.save(&ctx.paths)?;
            }
            println!("{}", serde_json::to_string_pretty(&cfg)?);
            Ok(())
        }
    }
}

fn confirm(prompt: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        println!("No terminal to ask in. Review the plan above, then run again with --yes.");
        return false;
    }
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

fn doctor(ctx: &Ctx) -> Result<()> {
    print!("{}", diagnose::doctor(ctx));
    Ok(())
}

fn show_plan(ctx: &Ctx, verbose: bool) -> Result<()> {
    let (plan, app, _) = plan::build_plan(ctx, None, None);
    if app.running {
        println!("Claude is running; changes to the account it has open wait until you switch accounts or quit.");
    }
    let labels = claude_companion_core::scan::account_labels(ctx);
    for l in diagnose::describe_plan(&report::summarize(&plan), &labels) {
        println!("{l}");
    }
    if verbose {
        for a in &plan.actions {
            let from = a.src.as_ref().map(|s| format!("  <- {}", s.label())).unwrap_or_default();
            println!("  {:<16} {}/{}{from}", format!("{:?}", a.kind), a.part.label(), a.name);
        }
    }
    Ok(())
}

fn sync(ctx: &Ctx, yes: bool) -> Result<()> {
    let (plan, _, _) = plan::build_plan(ctx, None, None);
    let labels = claude_companion_core::scan::account_labels(ctx);
    for l in diagnose::describe_plan(&report::summarize(&plan), &labels) {
        println!("{l}");
    }
    if !plan.is_empty() && !yes && !confirm("Apply these changes?") {
        return Ok(());
    }
    // Run even without changes: it records which accounts are linked and what each task list
    // looked like, which later merges depend on.
    let (_, out) = apply::run_sync(ctx, "manual")?;
    if !plan.is_empty() {
        println!(
            "Done: {} change(s){}.",
            out.applied_total(),
            if out.errors.is_empty() { String::new() } else { format!(", {} error(s)", out.errors.len()) }
        );
    }
    if !out.seeded_while_loaded.is_empty() {
        println!("Quit and reopen Claude to see the sessions copied into the account it has open.");
    }
    if ctx.config().sync_groups {
        groups_cmd(ctx, false, false)?;
    }
    if !out.errors.is_empty() {
        bail!("{} change(s) failed; see above", out.errors.len());
    }
    Ok(())
}

/// `groups`: one set of sidebar groups for every account.
fn groups_cmd(ctx: &Ctx, dry_run: bool, json: bool) -> Result<()> {
    let report = groups::sync(ctx, dry_run)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    for w in &report.warnings {
        println!("warning: {w}");
    }
    println!("{}", report.summary());
    if let Some(dir) = &report.backup_dir {
        println!("Backup of Claude's Local Storage before the change: {}", dir.display());
    }
    if report.status == groups::Status::Updated {
        println!(
            "Start Claude on the account you usually use: it pushes the merged groups to claude.ai, and the other accounts pick them up when you switch to them."
        );
    }
    if !ctx.config().sync_groups {
        println!("Background sync leaves groups alone until you turn it on: `claude-companion config --groups on`.");
    }
    Ok(())
}

fn install(ctx: &Ctx, yes: bool) -> Result<()> {
    let (plan, _, _) = plan::build_plan(ctx, None, None);
    let labels = claude_companion_core::scan::account_labels(ctx);
    println!("First sync:");
    for l in diagnose::describe_plan(&report::summarize(&plan), &labels) {
        println!("{l}");
    }
    if !yes && !confirm("Sync now and keep syncing in the background?") {
        return Ok(());
    }
    if !ctx.paths.config_file().exists() {
        ctx.config().save(&ctx.paths)?;
    }
    let (_, out) = apply::run_sync(ctx, "install")?;
    let exe = std::env::current_exe()?;
    // On Windows the windowless agent build runs in the background.
    let agent = if cfg!(windows) { exe.with_file_name("claude-companion-agent.exe") } else { exe.clone() };
    let (source, name) = if agent.exists() {
        (agent, if cfg!(windows) { "claude-companion-agent.exe" } else { "claude-companion" })
    } else {
        (exe, if cfg!(windows) { "claude-companion.exe" } else { "claude-companion" })
    };
    let installed = service::install(ctx, &source, name)?;
    println!("Background sync is on ({}).", installed.display());
    println!("Log: {}", ctx.paths.log_file.display());
    if !out.seeded_while_loaded.is_empty() {
        println!("Quit and reopen Claude once to see the sessions copied into the account it has open.");
    }
    Ok(())
}

/// Every account Claude Companion knows of, by ID, with its name: the list, then the ones only seen in
/// Claude's session folders.
fn known_accounts(ctx: &Ctx, roster: &Roster) -> BTreeMap<String, String> {
    let labels = scan::account_labels(ctx);
    let mut known: BTreeMap<String, String> = roster.slots.iter().map(|s| (s.account.clone(), s.label())).collect();
    for part in scan::discover(ctx, Surface::Code) {
        let name = labels.get(&part.acct).cloned().unwrap_or_else(|| format!("Account {}", short(&part.acct)));
        known.entry(part.acct).or_insert(name);
    }
    known
}

/// The account `query` names in the list; an account only seen in Claude's session folders is
/// named in the error, with how to add it.
fn named(ctx: &Ctx, roster: &Roster, query: &str) -> Result<(String, String)> {
    match roster.find(query) {
        Ok(slot) => Ok((slot.account.clone(), slot.label())),
        Err(NotFound::Several(names)) => bail!("{query} could be {}", names.join(" or ")),
        Err(NotFound::None(_)) => {
            let query = query.trim();
            let others: Vec<(String, String)> = known_accounts(ctx, roster)
                .into_iter()
                .filter(|(id, name)| name.eq_ignore_ascii_case(query) || (query.len() >= 4 && id.starts_with(query)))
                .collect();
            match others.as_slice() {
                [(_, name)] => {
                    bail!("{name} is not in the list yet: sign in to it once in Claude (`claude-companion add`)")
                }
                [] => bail!("no account called {query} (see `claude-companion accounts`)"),
                _ => bail!(
                    "{query} could be {}",
                    others.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(" or ")
                ),
            }
        }
    }
}

fn name_in(roster: &Roster, account: &str) -> String {
    roster.slot(account).map(Slot::label).unwrap_or_else(|| format!("Account {}", short(account)))
}

/// "5h 12% · 7d 58%, 2 min ago", or nothing when Claude has no reading.
fn usage_text(usage: Option<&Usage>, now: f64) -> String {
    let Some(u) = usage else { return String::new() };
    let mut parts = Vec::new();
    if let Some(p) = u.five_hour_at(now) {
        parts.push(format!("5h {p:.0}%"));
    }
    if let Some(p) = u.weekly_at(now) {
        parts.push(format!("7d {p:.0}%"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{}, {}", parts.join(" · "), report::ago(u.at))
}

fn accounts(ctx: &Ctx, json: bool) -> Result<()> {
    let roster = accounts::observe(ctx);
    let found = logins::list(ctx);
    let usage = accounts::usage(ctx, &roster);
    let now = fsx::now_secs();
    let signed_in = found.signed_in.as_deref();
    if json {
        let rows: Vec<serde_json::Value> = roster
            .slots
            .iter()
            .map(|s| {
                let kept = found.saved(&s.account);
                let u = usage.get(&s.account);
                serde_json::json!({
                    "number": s.number,
                    "account": s.account,
                    "email": s.email,
                    "alias": s.alias,
                    "disabled": s.disabled,
                    "signedIn": signed_in == Some(s.account.as_str()),
                    "switchable": kept.is_some(),
                    "keptAt": kept.map(|k| k.set_aside_at),
                    "stale": kept.is_some_and(|k| k.stale()),
                    "usage": u.map(|u| serde_json::json!({
                        "at": u.at,
                        "fiveHour": u.five_hour_at(now),
                        "weekly": u.weekly_at(now),
                    })),
                })
            })
            .collect();
        let out = serde_json::json!({ "schemaVersion": 1, "signedIn": signed_in, "accounts": rows });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if roster.slots.is_empty() {
        println!("No accounts yet: sign in to Claude.");
        return Ok(());
    }
    if !found.supported {
        println!("Switching accounts is not supported on this system yet.\n");
    }
    let name_width = roster.slots.iter().map(|s| s.label().chars().count()).max().unwrap_or(0);
    let alias_width =
        roster.slots.iter().filter_map(|s| s.alias.as_ref()).map(|a| a.chars().count()).max().unwrap_or(0);
    for s in &roster.slots {
        let state = if signed_in == Some(s.account.as_str()) {
            "signed in".to_string()
        } else if let Some(kept) = found.saved(&s.account) {
            let stale = if kept.stale() { " (may ask to sign in again)" } else { "" };
            format!("kept, used {}{stale}", report::ago(kept.set_aside_at))
        } else {
            "sign in once to switch here (`claude-companion add`)".to_string()
        };
        let state = if s.disabled { format!("{state}, sits out") } else { state };
        let used = usage_text(usage.get(&s.account), now);
        let alias = s.alias.clone().unwrap_or_default();
        let line = format!("  {:>2}  {:name_width$}  {alias:alias_width$}  {state:<30}  {used}", s.number, s.label());
        println!("{}", line.trim_end());
    }
    let others: Vec<String> = known_accounts(ctx, &roster)
        .into_iter()
        .filter(|(id, _)| roster.slot(id).is_none())
        .map(|(_, name)| name)
        .collect();
    if !others.is_empty() {
        println!("\nAlso in your sessions: {} (sign in to them once in Claude to add them).", others.join(", "));
    }
    Ok(())
}

fn switch(ctx: &Ctx, account: Option<&str>, strategy: Option<Strategy>, json: bool, force: bool) -> Result<()> {
    let roster = accounts::observe(ctx);
    let found = logins::list(ctx);
    let target = match account {
        Some(query) => named(ctx, &roster, query)?.0,
        None => {
            let usage = accounts::usage(ctx, &roster);
            let strategy = strategy.unwrap_or_default();
            let can_switch = |s: &Slot| found.saved(&s.account).is_some();
            let current = found.signed_in.as_deref();
            match accounts::pick(&roster, current, strategy, can_switch, &usage, fsx::now_secs()) {
                Some(slot) => slot.account.clone(),
                None => return Err(accounts::Refused::NoOther.into()),
            }
        }
    };
    let name = name_in(&roster, &target);
    if found.signed_in.as_deref() == Some(target.as_str()) {
        if json {
            let out = serde_json::json!({ "schemaVersion": 1, "switched": false, "to": target, "reason": "already-signed-in" });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("Claude is already signed in to {name}.");
        }
        return Ok(());
    }
    let done = accounts::switch(ctx, &target, &quit_watch(force))?;
    if json {
        let out = serde_json::json!({
            "schemaVersion": 1, "switched": true, "from": done.from, "to": done.to, "launched": done.launched,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let restarted = if done.launched { " (restarted)" } else { "" };
    println!("Claude is signed in to {name}{restarted}.");
    Ok(())
}

/// Claude asks before it quits when it has work in progress: say so, and what Ctrl-C does then.
pub(crate) fn quit_watch(force: bool) -> desktop::Watch<'static> {
    fn tell(quitting: Quitting) {
        match quitting {
            Quitting::Asked => {}
            Quitting::Confirming => eprintln!(
                "Claude is asking whether to stop its work in progress: answer it in Claude. (Ctrl-C stops waiting; nothing changes.)"
            ),
            Quitting::AfterWork => eprintln!(
                "Claude quits once its work in progress is done, and Claude Companion carries on then. (Ctrl-C stops waiting; Claude still quits then.)"
            ),
        }
    }
    if cfg!(windows) && !force {
        eprintln!(
            "On Windows, quit Claude from its tray icon (closing its window leaves it running), or run this again with --force."
        );
    }
    desktop::Watch { progress: Some(&tell), stop: None, force, force_now: None }
}

fn add(ctx: &Ctx, force: bool) -> Result<()> {
    let switched = accounts::add(ctx, &quit_watch(force))?;
    let roster = Roster::load(&ctx.paths);
    if let Some(from) = &switched.from {
        let name = name_in(&roster, from);
        let number = roster.slot(from).map(|s| s.number.to_string()).unwrap_or_else(|| name.clone());
        println!("Kept {name}: `claude-companion switch {number}` brings it back.");
    }
    println!("Claude is open on its sign-in page: sign in to the other account there.");
    println!("To switch later, use `claude-companion switch`: Log out in Claude ends a sign-in for good.");
    Ok(())
}

fn remove(ctx: &Ctx, query: &str, yes: bool) -> Result<()> {
    let roster = accounts::observe(ctx);
    let (id, name) = named(ctx, &roster, query)?;
    println!("Remove {name} from the list? The sign-in kept for it is forgotten; its sessions stay.");
    if !yes && !confirm("Remove it?") {
        return Ok(());
    }
    accounts::remove(ctx, &id)?;
    println!("Removed {name}. Sign in to it in Claude to add it again.");
    Ok(())
}

fn alias(ctx: &Ctx, account: Option<&str>, name: Option<&str>, unset: bool) -> Result<()> {
    let roster = accounts::observe(ctx);
    let Some(query) = account else {
        let aliased: Vec<&Slot> = roster.slots.iter().filter(|s| s.alias.is_some()).collect();
        if aliased.is_empty() {
            println!("No aliases yet: `claude-companion alias <account> <name>`.");
        }
        for s in aliased {
            println!("  {:<12}  {}  {}", s.alias.as_deref().unwrap_or_default(), s.number, s.label());
        }
        return Ok(());
    };
    let (id, label) = named(ctx, &roster, query)?;
    if unset {
        accounts::set_alias(ctx, &id, None)?;
        println!("{label} has no alias now.");
    } else if let Some(name) = name {
        let roster = accounts::set_alias(ctx, &id, Some(name))?;
        let alias = roster.slot(&id).and_then(|s| s.alias.clone()).unwrap_or_default();
        println!("{label} is also {alias}: `claude-companion switch {alias}`.");
    } else {
        match roster.slot(&id).and_then(|s| s.alias.as_deref()) {
            Some(alias) => println!("{alias}"),
            None => println!("{label} has no alias."),
        }
    }
    Ok(())
}

fn sit_out(ctx: &Ctx, query: &str, disabled: bool) -> Result<()> {
    let roster = accounts::observe(ctx);
    let (id, name) = named(ctx, &roster, query)?;
    accounts::set_disabled(ctx, &id, disabled)?;
    if disabled {
        println!("{name} sits out of the rotation; `claude-companion switch {query}` still switches to it.");
    } else {
        println!("{name} is back in the rotation.");
    }
    Ok(())
}

fn move_account(ctx: &Ctx, query: &str, number: u32) -> Result<()> {
    let roster = accounts::observe(ctx);
    let (id, name) = named(ctx, &roster, query)?;
    let old = roster.slot(&id).map(|s| s.number);
    let swapped = accounts::move_to(ctx, &id, number)?;
    println!("{name} is number {number} now.");
    if let (Some(other), Some(old)) = (swapped, old) {
        println!("{} took number {old}.", name_in(&roster, &other));
    }
    Ok(())
}

fn status(ctx: &Ctx) -> Result<()> {
    let st = service::status(ctx);
    println!("Background sync: {}{}", if st.installed { "on, " } else { "off, " }, st.detail);
    if let Some(h) = Heartbeat::read(&ctx.paths) {
        println!("Agent: v{} (pid {}), last check {}", h.version, h.pid, report::ago(h.heartbeat_at));
    }
    let state = State::load(&ctx.paths);
    match &state.last_sync {
        Some(ls) => println!(
            "Last sync: {} ({}): {} change(s), {} error(s), {} waiting for Claude",
            report::ago(ls.at),
            ls.reason,
            ls.applied,
            ls.errors,
            ls.deferred
        ),
        None => println!("Never synced."),
    }
    println!("Baseline snapshot: {}", state.baseline.as_deref().unwrap_or("none"));
    for (surface, keys) in &state.known {
        let list: Vec<String> = keys.iter().map(|k| k.split('/').map(short).collect::<Vec<_>>().join("/")).collect();
        println!("Linked {surface}: {}", list.join(", "));
    }
    println!("Log: {}", ctx.paths.log_file.display());
    Ok(())
}

fn snapshots(ctx: &Ctx) -> Result<()> {
    let state = State::load(&ctx.paths);
    let list = snapshot::list(ctx);
    if list.is_empty() {
        println!("No snapshots yet. The first sync takes one.");
    }
    for m in &list {
        let baseline = if Some(&m.id) == state.baseline.as_ref() { "  (before the first sync)" } else { "" };
        println!("{}  {} folder(s){baseline}", m.id, m.partitions.len());
    }
    println!("Location: {}", ctx.paths.snapshots_dir().display());
    Ok(())
}

fn retention(ctx: &Ctx, days: Option<u32>, keep: bool, undo: bool) -> Result<()> {
    if let Some(days) = days {
        let backup = retention::set_cleanup_days(&ctx.paths, days)?;
        println!("cleanupPeriodDays = {days} in {}", ctx.paths.claude_settings.display());
        if let Some(b) = backup {
            println!("Previous file saved as {}", b.display());
        }
        return Ok(());
    }
    if keep {
        match retention::keep(&ctx.paths)? {
            Kept::Already => println!("Claude Code already keeps the transcripts of Desktop sessions at any age."),
            Kept::AnyAge => {
                println!("Removed desktopSessionCleanupPeriodDays: Desktop sessions keep their transcripts at any age.")
            }
            Kept::TenYears => {
                println!("cleanupPeriodDays = 3650. Updating Claude keeps Desktop transcripts at any age.")
            }
        }
        println!("Undo with: claude-companion retention --undo");
        return Ok(());
    }
    if undo {
        match retention::undo(&ctx.paths)? {
            true => {
                println!("Put back the setting Claude Companion changed in {}.", ctx.paths.claude_settings.display())
            }
            false => println!("Nothing to undo."),
        }
        return Ok(());
    }
    let r = retention::read(&ctx.paths);
    let version = r.claude_code.as_deref().map(|v| format!("Claude Code {v}")).unwrap_or_else(|| "Claude Code".into());
    match (r.desktop_days, r.limited_by) {
        (Some(d), Some(Limit::DesktopSetting)) => {
            println!("Desktop sessions: transcripts deleted after {d} days (desktopSessionCleanupPeriodDays).")
        }
        (Some(d), Some(Limit::Organization)) => {
            println!("Desktop sessions: transcripts deleted after {d} days (set by your organization).")
        }
        (Some(d), _) => println!("Desktop sessions: transcripts deleted after {d} days ({version} predates 2.1.248)."),
        (None, _) => println!("Desktop sessions: transcripts kept at any age ({version})."),
    }
    println!("Terminal sessions and other data: deleted after {} days (cleanupPeriodDays).", r.cleanup_days);
    if r.is_short() && r.limited_by != Some(Limit::Organization) {
        println!("Keep them: claude-companion retention --keep");
    }
    if r.undo.is_some() {
        println!("Put back what Claude Companion changed: claude-companion retention --undo");
    }
    Ok(())
}
