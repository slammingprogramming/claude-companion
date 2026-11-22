// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// The features follow cc-same (MIT, Songkeys) and claude-switcher (MIT, NeezerGu and Nyxon);
// see NOTICE.md and LICENSES/.
//! What the command line does with `chats`, `stats`, `doctor` and `retention <days>`, in the app:
//! the rows of the settings sheet's *Tools* group, and the store actions behind them.

use crate::i18n::{t, tf, tn};
use crate::model::{self, Busy};
use crate::store::{Store, StoreEvent};
use claude_companion_core::about;
use claude_companion_core::{Ctx, Partition, Surface};
use claude_companion_core::{
    chats, code, code_login, desktop, diagnose, groups, logins, mcp, report, retention, scan, transcript,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::{
    Anchor, AnyElement, App, ClipboardItem, Context, Entity, IntoElement as _, ParentElement as _, SharedString,
    Styled as _,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Where an exported chat goes: the Downloads folder, else the home folder.
pub(crate) fn export_dir() -> PathBuf {
    let home = claude_companion_core::paths::home();
    let downloads = home.join("Downloads");
    if downloads.is_dir() { downloads } else { home }
}

/// `path`, or `name (2).ext`, `name (3).ext`… when it is taken: an export never replaces a file.
pub(crate) fn free_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| path.with_file_name(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("some name is free")
}

/// A file name for a chat: its title, made safe for every system, and the start of its id.
fn file_name(title: &str, id: &str, ext: &str) -> String {
    let words: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let words: String = words.chars().take(50).collect();
    let id: String = id.trim_start_matches("local_").chars().take(8).collect();
    let stem = if words.is_empty() { format!("chat {id}") } else { format!("{words} {id}") };
    format!("{stem}.{ext}")
}

/// The list that gets rescued chats: one Claude has not loaded (the signed-in account's when it
/// is free), so the write is allowed now and sync carries the cards to the others.
fn rescue_target(ctx: &Ctx) -> Option<Partition> {
    let app = desktop::detect(ctx);
    let signed_in = logins::list(ctx).signed_in;
    let free: Vec<Partition> =
        scan::discover(ctx, Surface::Code).into_iter().filter(|p| !p.is_link && !app.loaded(p)).collect();
    free.iter().find(|p| Some(&p.acct) == signed_in.as_ref()).or(free.first()).cloned()
}

/// A statistics text: tokens by model, by day (the last two weeks) and by folder.
fn stats_text(st: &transcript::Stats) -> String {
    fn k(n: u64) -> String {
        match n {
            n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
            n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
            n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
            n => n.to_string(),
        }
    }
    let row = |label: &str, w: usize, u: &transcript::Usage| {
        format!(
            "{label:<w$}  {:>7}  in {:>7}  out {:>7}  cache-w {:>7}  cache-r {:>7}  {:>6} replies\n",
            k(u.total()),
            k(u.input),
            k(u.output),
            k(u.cache_write),
            k(u.cache_read),
            u.messages
        )
    };
    let mut out = format!("{} chat(s) in {} transcript file(s), counted once per reply.\n\n", st.sessions, st.files);
    let mut models: Vec<_> = st.by_model.iter().collect();
    models.sort_by_key(|(_, u)| std::cmp::Reverse(u.total()));
    out.push_str("By model\n");
    for (m, u) in models {
        out.push_str(&row(m, 22, u));
    }
    out.push_str("\nLast 14 days\n");
    for (day, u) in st.by_day.iter().rev().take(14).collect::<Vec<_>>().into_iter().rev() {
        out.push_str(&row(day, 22, u));
    }
    out.push_str("\nBusiest folders\n");
    let mut folders: Vec<_> = st.by_project.iter().filter(|(p, _)| !p.is_empty()).collect();
    folders.sort_by_key(|(_, u)| std::cmp::Reverse(u.total()));
    for (p, u) in folders.into_iter().take(8) {
        let tail: Vec<char> = p.chars().rev().take(22).collect();
        out.push_str(&row(&tail.into_iter().rev().collect::<String>(), 22, u));
    }
    out.push_str(&row("all", 22, &st.total()));
    out
}

impl Store {
    /// Read something in the background and show it: nothing is changed.
    fn report_in_background(&mut self, cx: &mut Context<Self>, work: impl FnOnce(&Ctx) -> StoreEvent + Send + 'static) {
        let ctx = self.ctx.clone();
        cx.spawn(async move |this, cx| {
            let event = cx.background_executor().spawn(async move { work(&ctx) }).await;
            let _ = this.update(cx, |_, cx| cx.emit(event));
        })
        .detach();
    }

    /// What `claude-companion doctor` prints.
    pub fn show_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            StoreEvent::Report(t("tools.diagnostics"), format!("```text\n{}```", diagnose::doctor(ctx)))
        });
    }

    /// What `claude-companion stats` prints.
    pub fn show_stats(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            let st = transcript::usage_stats(&ctx.paths.projects);
            let body = if st.files == 0 { t("tools.stats.none") } else { format!("```text\n{}```", stats_text(&st)) };
            StoreEvent::Report(t("tools.stats"), body)
        });
    }

    /// Look for conversations no account shows; the window asks before giving them cards.
    pub fn find_lost(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            let (found, _) = chats::lost(ctx);
            if found.is_empty() {
                return StoreEvent::Done(Ok(t("toast.no_lost")));
            }
            let mut preview: Vec<String> = found.iter().take(8).map(|l| format!("• {}", l.title)).collect();
            if found.len() > 8 {
                preview.push(tn("tools.lost.more", found.len() - 8, &[]));
            }
            StoreEvent::LostFound { count: found.len(), preview: preview.join("\n") }
        });
    }

    /// Give every lost chat a card, in a list Claude has not loaded.
    pub fn rescue_lost(&mut self, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, |ctx| {
            let Some(target) = rescue_target(ctx) else {
                anyhow::bail!("{}", t("tools.lost.closed"));
            };
            let done = chats::rescue(ctx, &target, &[])?;
            Ok(tn("toast.rescued", done.created.len(), &[]))
        });
    }

    /// What `claude-companion groups [--dry-run]` prints: the sidebar groups of every account made
    /// the same, or a preview of that.
    pub fn merge_groups(&mut self, dry_run: bool, cx: &mut Context<Self>) {
        self.report_in_background(cx, move |ctx| {
            let body = match groups::sync(ctx, dry_run) {
                Ok(report) => {
                    let mut text = report.warnings.iter().map(|w| format!("warning: {w}\n")).collect::<String>();
                    text.push_str(&report.summary());
                    text.push('\n');
                    if let Some(dir) = &report.backup_dir {
                        text.push_str(&format!("Backup: {}\n", dir.display()));
                    }
                    format!("```text\n{text}```")
                }
                Err(e) => format!("```text\n{e:#}\n```"),
            };
            StoreEvent::Report(t("tools.groups"), body)
        });
    }

    /// What `claude-companion mcp share [--dry-run]` prints: MCP servers one of Claude Desktop and
    /// Claude Code lacks, copied into the other, or a preview of that.
    pub fn share_mcp(&mut self, dry_run: bool, cx: &mut Context<Self>) {
        self.report_in_background(cx, move |ctx| {
            let body = match mcp::share(ctx, mcp::Direction::Both, false, dry_run) {
                Ok(report) => format!("```text\n{}\n```", report.lines().join("\n")),
                Err(e) => format!("```text\n{e:#}\n```"),
            };
            StoreEvent::Report(t("tools.mcp"), body)
        });
    }

    /// Open Claude Code in a new terminal window: continuing `session` where it left off, or a new
    /// one in the home folder.
    pub fn open_code(&mut self, session: Option<String>, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            match session {
                Some(id) => {
                    let (dir, args) = code::resume_target(&code::resolve_session(ctx, &id)?)?;
                    code::open_window(&dir, &args)?;
                }
                None => code::open_window(&claude_companion_core::paths::home(), &[])?,
            }
            Ok(t("toast.code_opened"))
        });
    }

    /// Save the sign-in Claude Code has now, so it can be switched back to.
    pub fn save_code_login(&mut self, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, |ctx| {
            let p = code_login::save_current(ctx)?;
            Ok(tf("toast.code_login_saved", &[("email", &p.email)]))
        });
    }

    /// Make Claude Code use a saved sign-in.
    pub fn switch_code_login(&mut self, id: String, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            let p = code_login::switch_to(ctx, &id)?;
            Ok(tf("toast.code_login_switched", &[("email", &p.email)]))
        });
    }

    /// What `claude-companion chats list` prints.
    pub fn show_chats(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            let all = chats::list(ctx);
            let mut text = String::new();
            for c in all.iter().take(300) {
                let list = c.list.split_once('/').map_or(c.list.as_str(), |(a, _)| claude_companion_core::short(a));
                let archived = if c.archived { " [archived]" } else { "" };
                let when =
                    if c.last_activity_ms > 0 { report::ago(c.last_activity_ms as f64 / 1000.0) } else { "-".into() };
                text.push_str(&format!("{:<10} {:<9} {}{archived}\n", list, when, c.title));
            }
            if all.len() > 300 {
                text.push_str(&format!("... and {} more\n", all.len() - 300));
            }
            if text.is_empty() {
                text.push_str("No chats yet.\n");
            }
            StoreEvent::Report(t("tools.chats"), format!("```text\n{text}```"))
        });
    }

    /// What `claude-companion code sessions` prints.
    pub fn show_code_sessions(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            let all = code::sessions(ctx, None, 60);
            let mut text = String::new();
            for s in &all {
                let folder = s.cwd.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or_default();
                let from = if s.desktop { " [Desktop]" } else { "" };
                text.push_str(&format!(
                    "{:<8} {:<9} {:<22} {}{from}\n",
                    &s.id[..s.id.len().min(8)],
                    report::ago(s.last_ms as f64 / 1000.0),
                    folder,
                    s.title
                ));
            }
            if text.is_empty() {
                text.push_str("No sessions found.\n");
            }
            StoreEvent::Report(t("tools.sessions"), format!("```text\n{text}```"))
        });
    }

    /// What `claude-companion code settings` prints: the user settings, secrets hidden.
    pub fn show_code_settings(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            let body = match code::settings(ctx) {
                Ok(map) => {
                    let shown = code::redact(&serde_json::Value::Object(map));
                    format!(
                        "{}\n\n```json\n{}\n```",
                        ctx.paths.claude_settings.display(),
                        serde_json::to_string_pretty(&shown).unwrap_or_default()
                    )
                }
                Err(e) => format!("```text\n{e:#}\n```"),
            };
            StoreEvent::Report(t("tools.codesettings"), body)
        });
    }

    /// What `claude-companion mcp list` prints.
    pub fn show_mcp(&mut self, cx: &mut Context<Self>) {
        self.report_in_background(cx, |ctx| {
            let ov = mcp::overview(ctx);
            let mut text: String = ov.warnings.iter().map(|w| format!("warning: {w}\n")).collect();
            for e in &ov.entries {
                let mark = |b: bool| if b { "yes" } else { "-" };
                let differs = if e.desktop && e.code && !e.same { "  (differs)" } else { "" };
                text.push_str(&format!(
                    "{:<24} {:<6} Desktop {:<4} Code {:<4} {}{differs}\n",
                    e.name,
                    e.kind,
                    mark(e.desktop),
                    mark(e.code),
                    e.detail
                ));
            }
            if ov.entries.is_empty() {
                text.push_str("No MCP servers in Claude Desktop or Claude Code.\n");
            }
            StoreEvent::Report(t("tools.mcp"), format!("```text\n{text}```"))
        });
    }

    /// Set Claude Code's default permission mode (`None` removes the setting).
    pub fn set_permission_mode(&mut self, mode: Option<&'static str>, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            code::settings_set(ctx, "permissions.defaultMode", mode.map(serde_json::Value::from))?;
            Ok(t("toast.code_mode"))
        });
    }

    /// Put every chat of one account into another's list (newer cards win, nothing is deleted).
    pub fn copy_chats(&mut self, from: String, to: String, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            let one = |query: &str| -> anyhow::Result<Partition> {
                let mut lists = chats::resolve(ctx, query)?;
                if lists.len() != 1 {
                    anyhow::bail!("{}", t("tools.copy.orgs"));
                }
                Ok(lists.remove(0))
            };
            let done = chats::copy(ctx, &one(&from)?, &one(&to)?, &[])?;
            Ok(tf("toast.copied", &[("count", &(done.created.len() + done.updated))]))
        });
    }

    /// Quit Claude (on Windows it keeps running in the tray, so `force` ends it).
    pub fn quit_desktop(&mut self, force: bool, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            if desktop::is_running(ctx) {
                let watch = desktop::Watch { force, ..desktop::Watch::default() };
                desktop::quit(ctx, std::time::Duration::from_secs(60), &watch)?;
            }
            Ok(t("toast.desktop_quit"))
        });
    }

    /// Open Claude Code in a new terminal window, in `dir`.
    pub fn open_code_in(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |_| {
            code::open_window(&dir, &[])?;
            Ok(t("toast.code_opened"))
        });
    }

    /// Delete a saved Claude Code sign-in (Claude Code itself stays signed in).
    pub fn forget_code_login(&mut self, id: String, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            let p = code_login::forget(ctx, &id)?;
            Ok(tf("toast.code_login_forgot", &[("email", &p.email)]))
        });
    }

    /// Start the Claude app.
    pub fn open_desktop(&mut self, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, |ctx| {
            code::open_desktop(ctx)?;
            Ok(t("toast.desktop_opened"))
        });
    }

    /// Write a chat as Markdown or a web page into the Downloads folder.
    pub fn export_chat(&mut self, id: String, title: String, html: bool, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            let path = chats::find_transcript(ctx, &id)?;
            let (body, ext) = if html {
                (transcript::export_html(&path)?, "html")
            } else {
                (transcript::export_markdown(&path)?, "md")
            };
            let dest = free_path(export_dir().join(file_name(&title, &id, ext)));
            std::fs::write(&dest, body)?;
            Ok(tf("toast.exported", &[("path", &dest.display().to_string())]))
        });
    }

    /// Write the usage history's plan readings or token counts as CSV into the Downloads folder.
    pub fn export_history(&mut self, tokens: bool, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            let history = claude_companion_core::history::History::open(&ctx.paths)?;
            let name = if tokens { "claude-companion-tokens.csv" } else { "claude-companion-plan-use.csv" };
            let dest = free_path(export_dir().join(name));
            let mut file = std::fs::File::create(&dest)?;
            if tokens {
                history.write_tokens_csv(&mut file)?;
            } else {
                history.write_plan_csv(&mut file)?;
            }
            Ok(tf("toast.exported", &[("path", &dest.display().to_string())]))
        });
    }

    /// Delete the whole usage history.
    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, |ctx| {
            claude_companion_core::history::History::open(&ctx.paths)?.clear()?;
            Ok(t("history.cleared"))
        });
    }

    /// Give an account another number in the list; the account that has it takes the old one.
    pub fn move_account(&mut self, account: String, number: u32, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            claude_companion_core::accounts::move_to(ctx, &account, number)?;
            Ok(String::new())
        });
    }

    /// How long Claude Code keeps the transcripts of terminal sessions and other data.
    pub fn set_cleanup_days(&mut self, days: u32, cx: &mut Context<Self>) {
        self.run_task(Busy::Saving, cx, move |ctx| {
            retention::set_cleanup_days(&ctx.paths, days)?;
            Ok(t("toast.saved"))
        });
    }
}

/// The chats most recently active, for the export menus.
fn recent_chats(ctx: &Ctx) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    chats::list(ctx)
        .into_iter()
        .filter(|c| seen.insert(c.id.clone()))
        .take(14)
        .map(|c| {
            let title = if c.title.is_empty() { c.id.clone() } else { c.title };
            (c.id, title)
        })
        .collect()
}

fn short_title(title: &str) -> String {
    let cut: String = title.chars().take(44).collect();
    if title.chars().count() > 44 { format!("{cut}…") } else { cut }
}

/// What picking a menu item does.
type Pick = Arc<dyn Fn(&mut Store, &mut Context<Store>) + Send + Sync>;

/// A button that opens a menu built when it opens: `(label, disabled, what picking it does)`. The
/// `empty` text shows when there are no items.
fn menu_button(
    store: &Entity<Store>,
    id: &'static str,
    label: String,
    enabled: bool,
    empty: String,
    items: impl Fn(&Ctx, &Store) -> Vec<(String, bool, Pick)> + 'static,
) -> AnyElement {
    let store = store.clone();
    Button::new(id)
        .small()
        .outline()
        .label(label)
        .dropdown_caret(true)
        .disabled(!enabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let ctx: Arc<Ctx> = store.read(cx).ctx.clone();
            let list = items(&ctx, store.read(cx));
            let mut menu = menu;
            if list.is_empty() {
                return menu.item(PopupMenuItem::new(empty.clone()).disabled(true));
            }
            for (text, disabled, pick) in list {
                let store = store.clone();
                menu =
                    menu.item(PopupMenuItem::new(SharedString::from(short_title(&text))).disabled(disabled).on_click(
                        move |_, _, cx| {
                            let pick = pick.clone();
                            store.update(cx, |s, cx| pick(s, cx));
                        },
                    ));
            }
            menu
        })
        .into_any_element()
}

/// Every ordered pair of accounts, for copying chats from one into the other.
fn copy_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    menu_button(store, "chats-copy", t("tools.chats.copy"), enabled, t("tools.copy.none"), |_, store| {
        let accounts = store.accounts();
        let mut out: Vec<(String, bool, Pick)> = Vec::new();
        for a in &accounts {
            for b in accounts.iter().filter(|b| b.id != a.id) {
                let (from, to) = (a.id.clone(), b.id.clone());
                let pick: Pick = Arc::new(move |s, cx| s.copy_chats(from.clone(), to.clone(), cx));
                out.push((format!("{} → {}", a.name, b.name), false, pick));
            }
        }
        out
    })
}

/// Claude Code's default permission mode.
fn mode_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    menu_button(store, "code-mode", t("tools.codesettings.mode"), enabled, String::new(), |ctx, _| {
        let now = code::settings(ctx)
            .ok()
            .and_then(|m| code::get_path(&m, "permissions.defaultMode").and_then(|v| v.as_str().map(str::to_string)));
        let mut out: Vec<(String, bool, Pick)> = Vec::new();
        for mode in [None, Some("acceptEdits"), Some("plan")] {
            let on = now.as_deref() == mode;
            let name = mode.unwrap_or("default");
            let pick: Pick = Arc::new(move |s, cx| s.set_permission_mode(mode, cx));
            out.push((if on { format!("✓ {name}") } else { name.to_string() }, on, pick));
        }
        out
    })
}

/// Quit Claude, politely or not.
fn quit_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    menu_button(store, "desktop-quit", t("tools.desktop.quit"), enabled, String::new(), |_, _| {
        let ask: Pick = Arc::new(|s, cx| s.quit_desktop(false, cx));
        let end: Pick = Arc::new(|s, cx| s.quit_desktop(true, cx));
        vec![(t("tools.desktop.quit.ask"), false, ask), (t("tools.desktop.quit.end"), false, end)]
    })
}

/// The folders terminal sessions worked in, most recent first: pick one to open Claude Code there.
fn folder_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    menu_button(store, "code-folder", t("tools.code.folder"), enabled, t("tools.code.none"), |ctx, _| {
        let mut seen = std::collections::HashSet::new();
        code::sessions(ctx, None, 80)
            .into_iter()
            .filter(|s| !s.cwd.is_empty() && std::path::Path::new(&s.cwd).is_dir() && seen.insert(s.cwd.clone()))
            .take(14)
            .map(|s| {
                let dir = PathBuf::from(&s.cwd);
                let pick: Pick = Arc::new(move |st, cx| st.open_code_in(dir.clone(), cx));
                (s.cwd, false, pick)
            })
            .collect()
    })
}

/// The saved Claude Code sign-ins, to delete one.
fn forget_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    menu_button(
        store,
        "code-login-forget",
        t("tools.codelogin.forget"),
        enabled && code_login::supported(),
        t("tools.codelogin.none"),
        |ctx, _| {
            code_login::list(ctx)
                .into_iter()
                .map(|p| {
                    let id = p.id.clone();
                    let pick: Pick = Arc::new(move |s, cx| s.forget_code_login(id.clone(), cx));
                    (format!("{} {}", t("tools.codelogin.forget").trim_end_matches('…'), p.email), false, pick)
                })
                .collect()
        },
    )
}

/// The terminal sessions most recently active, for the continue menu.
fn recent_code_sessions(ctx: &Ctx) -> Vec<(String, String)> {
    code::sessions(ctx, None, 14)
        .into_iter()
        .map(|s| {
            let folder = s.cwd.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or_default().to_string();
            let title = if s.title.is_empty() { s.id.clone() } else { s.title };
            (s.id, format!("{folder}: {title}"))
        })
        .collect()
}

/// A button that opens a menu of the saved Claude Code sign-ins; picking one switches to it.
fn login_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    let store = store.clone();
    Button::new("code-login-switch")
        .small()
        .outline()
        .label(t("tools.codelogin.switch"))
        .dropdown_caret(true)
        .disabled(!enabled || !code_login::supported())
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let ctx: Arc<Ctx> = store.read(cx).ctx.clone();
            let mut menu = menu;
            let saved = code_login::list(&ctx);
            if saved.is_empty() {
                return menu.item(PopupMenuItem::new(t("tools.codelogin.none")).disabled(true));
            }
            for p in saved {
                let store = store.clone();
                let label = if p.current { format!("✓ {}", p.email) } else { p.email.clone() };
                menu = menu.item(PopupMenuItem::new(SharedString::from(label)).disabled(p.current).on_click(
                    move |_, _, cx| {
                        let id = p.id.clone();
                        store.update(cx, |s, cx| s.switch_code_login(id, cx));
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// A button that opens a menu of recent terminal sessions; picking one continues it in a new window.
fn continue_button(store: &Entity<Store>, enabled: bool) -> AnyElement {
    let store = store.clone();
    Button::new("code-continue")
        .small()
        .outline()
        .label(t("tools.code.continue"))
        .dropdown_caret(true)
        .disabled(!enabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let ctx: Arc<Ctx> = store.read(cx).ctx.clone();
            let mut menu = menu;
            let recent = recent_code_sessions(&ctx);
            if recent.is_empty() {
                return menu.item(PopupMenuItem::new(t("tools.code.none")).disabled(true));
            }
            for (id, label) in recent {
                let store = store.clone();
                menu =
                    menu.item(PopupMenuItem::new(SharedString::from(short_title(&label))).on_click(move |_, _, cx| {
                        let id = id.clone();
                        store.update(cx, |s, cx| s.open_code(Some(id), cx));
                    }));
            }
            menu
        })
        .into_any_element()
}

/// A button that opens a menu of recent chats; picking one exports it.
fn export_button(store: &Entity<Store>, id: &'static str, label: String, html: bool, enabled: bool) -> AnyElement {
    let store = store.clone();
    Button::new(id)
        .small()
        .outline()
        .icon(IconName::FileText)
        .label(label)
        .disabled(!enabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let ctx: Arc<Ctx> = store.read(cx).ctx.clone();
            let mut menu = menu;
            let recent = recent_chats(&ctx);
            if recent.is_empty() {
                return menu.item(PopupMenuItem::new(t("tools.export.none")).disabled(true));
            }
            for (chat, title) in recent {
                let store = store.clone();
                let label = SharedString::from(short_title(&title));
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let (chat, title) = (chat.clone(), title.clone());
                    store.update(cx, |s, cx| s.export_chat(chat, title, html, cx));
                }));
            }
            menu
        })
        .into_any_element()
}

/// A small button that runs one store action.
fn action_button(
    store: &Entity<Store>,
    id: &'static str,
    icon: IconName,
    label: String,
    enabled: bool,
    run: impl Fn(&mut Store, &mut Context<Store>) + 'static,
) -> AnyElement {
    let store = store.clone();
    Button::new(id)
        .small()
        .outline()
        .icon(icon)
        .label(label)
        .disabled(!enabled)
        .on_click(move |_, _, cx: &mut App| store.update(cx, |s, cx| run(s, cx)))
        .into_any_element()
}

/// A button that copies `text` and says so.
fn copy_button(store: &Entity<Store>, id: SharedString, label: String, text: &'static str, done: String) -> AnyElement {
    let store = store.clone();
    Button::new(id)
        .small()
        .outline()
        .icon(IconName::Copy)
        .label(label)
        .on_click(move |_, _, cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
            let done = done.clone();
            store.update(cx, |_, cx| cx.emit(StoreEvent::Done(Ok(done))));
        })
        .into_any_element()
}

/// The rows of the *About and support* group: how to reach the developer and how to support the
/// project (`claude-companion about` on the command line).
pub fn about_rows(
    store: &Entity<Store>,
    cx: &App,
    row: impl Fn(String, Option<String>, AnyElement, &App) -> AnyElement,
) -> Vec<AnyElement> {
    let contact = h_flex()
        .gap_1()
        .child(
            Button::new("about-contact")
                .small()
                .outline()
                .icon(IconName::ExternalLink)
                .label(t("about.open"))
                .on_click(|_, _, cx: &mut App| cx.open_url(about::CONTACT_SIMPLEX)),
        )
        .child(copy_button(
            store,
            "about-contact-copy".into(),
            t("about.copy_link"),
            about::CONTACT_SIMPLEX,
            t("toast.link_copied"),
        ))
        .into_any_element();
    let donate = Button::new("about-donate")
        .small()
        .outline()
        .icon(IconName::ExternalLink)
        .label("Ko-fi")
        .on_click(|_, _, cx: &mut App| cx.open_url(about::DONATE_KOFI))
        .into_any_element();
    let crypto = about::COINS.iter().fold(h_flex().gap_1(), |flex, coin| {
        flex.child(copy_button(
            store,
            SharedString::from(format!("about-{}", coin.ticker)),
            coin.ticker.to_string(),
            coin.address,
            tf("toast.address_copied", &[("coin", &coin.name)]),
        ))
    });
    vec![
        row(t("about.contact"), Some(t("about.contact.detail")), contact, cx),
        row(t("about.donate"), Some(t("about.donate.detail")), donate, cx),
        row(t("about.crypto"), Some(t("about.crypto.detail")), crypto.into_any_element(), cx),
    ]
}

/// The rows of the *Tools* group.
pub fn rows(
    store: &Entity<Store>,
    idle: bool,
    cx: &App,
    row: impl Fn(String, Option<String>, AnyElement, &App) -> AnyElement,
) -> Vec<AnyElement> {
    let find = action_button(store, "find-lost", IconName::Search, t("tools.find"), idle, |s, cx| s.find_lost(cx));
    let stats =
        action_button(store, "show-stats", IconName::ChartPie, t("settings.show"), true, |s, cx| s.show_stats(cx));
    let doctor = action_button(store, "show-diagnostics", IconName::Info, t("settings.show"), true, |s, cx| {
        s.show_diagnostics(cx)
    });
    let exports = h_flex()
        .gap_1()
        .child(export_button(store, "export-md", t("tools.export_md"), false, idle))
        .child(export_button(store, "export-html", t("tools.export_html"), true, idle))
        .into_any_element();
    let cleanup = cleanup_menu(store, idle);
    let merge = h_flex()
        .gap_1()
        .child(action_button(store, "groups-preview", IconName::Eye, t("tools.groups.preview"), idle, |s, cx| {
            s.merge_groups(true, cx)
        }))
        .child(action_button(store, "groups-merge", IconName::Check, t("tools.groups.merge"), idle, |s, cx| {
            s.merge_groups(false, cx)
        }))
        .into_any_element();
    let terminal = h_flex()
        .gap_1()
        .child(action_button(store, "code-open", IconName::SquareTerminal, t("tools.code.open"), idle, |s, cx| {
            s.open_code(None, cx)
        }))
        .child(continue_button(store, idle))
        .child(folder_menu(store, idle))
        .into_any_element();
    let logins = h_flex()
        .gap_1()
        .child(action_button(
            store,
            "code-login-save",
            IconName::Check,
            t("tools.codelogin.save"),
            idle && code_login::supported(),
            |s, cx| s.save_code_login(cx),
        ))
        .child(login_menu(store, idle))
        .child(forget_menu(store, idle))
        .into_any_element();
    let servers = h_flex()
        .gap_1()
        .child(action_button(store, "mcp-preview", IconName::Eye, t("tools.groups.preview"), idle, |s, cx| {
            s.share_mcp(true, cx)
        }))
        .child(action_button(store, "mcp-share", IconName::Check, t("tools.mcp.share"), idle, |s, cx| {
            s.share_mcp(false, cx)
        }))
        .child(action_button(store, "mcp-list", IconName::Info, t("tools.mcp.list"), true, |s, cx| s.show_mcp(cx)))
        .into_any_element();
    let claude = h_flex()
        .gap_1()
        .child(action_button(store, "desktop-open", IconName::ExternalLink, t("tools.desktop.open"), idle, |s, cx| {
            s.open_desktop(cx)
        }))
        .child(quit_menu(store, idle))
        .into_any_element();
    let chats_row = h_flex()
        .gap_1()
        .child(action_button(store, "chats-list", IconName::Info, t("settings.show"), true, |s, cx| s.show_chats(cx)))
        .child(copy_menu(store, idle))
        .into_any_element();
    let sessions = action_button(store, "sessions-list", IconName::Info, t("settings.show"), true, |s, cx| {
        s.show_code_sessions(cx)
    });
    let code_settings = h_flex()
        .gap_1()
        .child(action_button(store, "code-settings", IconName::Info, t("settings.show"), true, |s, cx| {
            s.show_code_settings(cx)
        }))
        .child(mode_menu(store, idle))
        .into_any_element();
    let history = Button::new("open-history")
        .small()
        .outline()
        .icon(Icon::empty().path("icons/chart-line.svg"))
        .label(t("settings.show"))
        .on_click({
            let store = store.clone();
            move |_, window, cx| crate::usage_view::open(store.clone(), window, cx)
        })
        .into_any_element();
    vec![
        row(t("history.title"), Some(t("history.detail")), history, cx),
        row(t("tools.desktop"), Some(t("tools.desktop.detail")), claude, cx),
        row(t("tools.code"), Some(t("tools.code.detail")), terminal, cx),
        row(t("tools.sessions"), Some(t("tools.sessions.detail")), sessions, cx),
        row(t("tools.codesettings"), Some(t("tools.codesettings.detail")), code_settings, cx),
        row(t("tools.codelogin"), Some(t("tools.codelogin.detail")), logins, cx),
        row(t("tools.mcp"), Some(t("tools.mcp.detail")), servers, cx),
        row(t("tools.groups"), Some(t("tools.groups.detail")), merge, cx),
        row(t("tools.chats"), Some(t("tools.chats.detail")), chats_row, cx),
        row(t("tools.lost"), Some(t("tools.lost.detail")), find, cx),
        row(t("tools.export"), Some(t("tools.export.detail")), exports, cx),
        row(t("tools.stats"), Some(t("tools.stats.detail")), stats, cx),
        row(t("tools.diagnostics"), Some(t("tools.diagnostics.detail")), doctor, cx),
        row(t("settings.cleanup"), Some(t("settings.cleanup.detail")), cleanup, cx),
    ]
}

/// A menu of how long Claude Code may keep the transcripts of terminal sessions.
fn cleanup_menu(store: &Entity<Store>, enabled: bool) -> AnyElement {
    let store = store.clone();
    Button::new("cleanup-days")
        .small()
        .outline()
        .label(t("settings.cleanup.choose"))
        .dropdown_caret(true)
        .disabled(!enabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu;
            for days in [30u32, 90, 365, 3650] {
                let store = store.clone();
                menu = menu.item(
                    PopupMenuItem::new(model::retention_label(Some(days as f64)))
                        .on_click(move |_, _, cx| store.update(cx, |s, cx| s.set_cleanup_days(days, cx))),
                );
            }
            menu
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_files_get_safe_names() {
        assert_eq!(file_name("Fix: the <build>/now?", "local_5a1d3c07-8f2e", "md"), "Fix the build now 5a1d3c07.md");
        assert_eq!(file_name("", "abcdef0123456789", "html"), "chat abcdef01.html");
        assert!(file_name(&"long ".repeat(40), "x", "md").chars().count() < 70);
        assert!(!file_name("a/b\\c:d", "i", "md").contains(['/', '\\', ':']));
    }

    #[test]
    fn long_titles_are_cut_for_menus() {
        assert_eq!(short_title("short"), "short");
        assert_eq!(short_title(&"x".repeat(60)).chars().count(), 45);
    }
}
