// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// The features follow cc-same (MIT, Songkeys) and claude-switcher (MIT, NeezerGu and Nyxon);
// see NOTICE.md and LICENSES/.
//! What the command line does with `chats`, `stats`, `doctor` and `retention <days>`, in the app:
//! the rows of the settings sheet's *Tools* group, and the store actions behind them.

use crate::i18n::{t, tf, tn};
use crate::model::{self, Busy};
use crate::store::{Store, StoreEvent};
use claude_companion_core::{Ctx, Partition, Surface};
use claude_companion_core::{chats, desktop, diagnose, groups, logins, retention, scan, transcript};
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, IconName, Sizable as _, h_flex};
use gpui_kit::{
    Anchor, AnyElement, App, Context, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Where an exported chat goes: the Downloads folder, else the home folder.
fn export_dir() -> PathBuf {
    let home = claude_companion_core::paths::home();
    let downloads = home.join("Downloads");
    if downloads.is_dir() { downloads } else { home }
}

/// `path`, or `name (2).ext`, `name (3).ext`… when it is taken: an export never replaces a file.
fn free_path(path: PathBuf) -> PathBuf {
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
    vec![
        row(t("tools.groups"), Some(t("tools.groups.detail")), merge, cx),
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
