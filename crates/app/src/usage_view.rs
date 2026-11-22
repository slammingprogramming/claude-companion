// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! The usage history: a sheet with graphs of plan use per account and of tokens over time, from
//! the database `claude_companion_core::history` keeps, plus the settings that go with it.

use crate::i18n::{t, tf, tn};
use crate::store::Store;
use chrono::NaiveDate;
use claude_companion_core::Ctx;
use claude_companion_core::history::{self, History, PlanDay, Summary, TokenDay};
use claude_companion_core::transcript::Usage;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariant};
use gpui_kit::component::chart::{AreaChart, BarChart};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rems,
};
use std::collections::BTreeMap;

/// Open the sheet.
pub fn open(store: Entity<Store>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| UsageHistory::new(store, cx));
    window.open_sheet(cx, move |sheet, _, _| sheet.title(t("history.title")).size(px(400.)).child(view.clone()));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Range {
    Days(u32),
    All,
}

impl Range {
    const ALL: [Range; 4] = [Range::Days(30), Range::Days(90), Range::Days(365), Range::All];

    fn label(self) -> String {
        match self {
            Range::Days(365) => tn("period.years", 1, &[]),
            Range::Days(n) => tn("period.days", n as usize, &[]),
            Range::All => t("history.range.all"),
        }
    }

    fn since(self) -> Option<String> {
        match self {
            Range::Days(n) => Some(history::days_ago(n)),
            Range::All => None,
        }
    }
}

struct Data {
    summary: Summary,
    plan: Vec<PlanDay>,
    tokens: Vec<TokenDay>,
    projects: Vec<(String, Usage)>,
}

fn read(ctx: &Ctx, range: Range) -> anyhow::Result<Data> {
    let history = History::open(&ctx.paths)?;
    let since = range.since();
    Ok(Data {
        summary: history.summary(&ctx.paths)?,
        plan: history.plan_days(since.as_deref())?,
        tokens: history.token_days(since.as_deref())?,
        projects: history.token_projects(since.as_deref())?,
    })
}

pub struct UsageHistory {
    store: Entity<Store>,
    range: Range,
    data: Option<Data>,
    error: Option<String>,
    /// The database is being read, or brought up to date.
    loading: bool,
}

impl UsageHistory {
    fn new(store: Entity<Store>, cx: &mut Context<Self>) -> UsageHistory {
        let mut view = UsageHistory { store, range: Range::Days(90), data: None, error: None, loading: false };
        view.load(true, cx);
        view
    }

    /// Read the database now, then (when `collect`) bring it up to date from Claude's own records
    /// and read it again: the first count of a large transcript folder takes a while.
    fn load(&mut self, collect: bool, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        let ctx = self.store.read(cx).ctx.clone();
        let range = self.range;
        cx.spawn(async move |this, cx| {
            let quick = {
                let ctx = ctx.clone();
                cx.background_executor().spawn(async move { read(&ctx, range) }).await
            };
            let _ = this.update(cx, |this, cx| this.show(quick, cx));
            if collect && ctx.config().history {
                let (ctx, again) = (ctx.clone(), ctx);
                let fresh = cx
                    .background_executor()
                    .spawn(async move { history::collect(&ctx).and_then(|_| read(&again, range)) })
                    .await;
                let _ = this.update(cx, |this, cx| this.show(fresh, cx));
            }
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn show(&mut self, result: anyhow::Result<Data>, cx: &mut Context<Self>) {
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
        cx.notify();
    }
}

/// One bucket of days on the x axis.
#[derive(Clone)]
struct Bucket {
    label: SharedString,
    /// Per account (in `Series::accounts` order): the highest 5-hour percent.
    five: Vec<f64>,
    /// Per account: the highest weekly percent.
    weekly: Vec<f64>,
    /// Input and output tokens.
    tokens: f64,
}

struct Series {
    accounts: Vec<Option<String>>,
    buckets: Vec<Bucket>,
}

fn date(day: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// The days the data spans, grouped so that no more than about 60 points are drawn.
fn series(plan: &[PlanDay], tokens: &[TokenDay]) -> Series {
    let days: Vec<NaiveDate> =
        plan.iter().map(|p| &p.day).chain(tokens.iter().map(|t| &t.day)).filter_map(|d| date(d)).collect();
    let (Some(first), Some(last)) = (days.iter().min().copied(), days.iter().max().copied()) else {
        return Series { accounts: Vec::new(), buckets: Vec::new() };
    };
    let span = (last - first).num_days() + 1;
    let size = match span {
        ..=60 => 1,
        61..=420 => 7,
        _ => 30,
    };
    let count = ((span + size - 1) / size) as usize;
    let at = |day: &str| date(day).map(|d| (((d - first).num_days() / size) as usize).min(count - 1));
    let mut accounts: Vec<Option<String>> = plan.iter().map(|p| p.account.clone()).collect();
    accounts.sort_by(|a, b| a.is_none().cmp(&b.is_none()).then(a.cmp(b)));
    accounts.dedup();
    let label = |i: usize| {
        let start = first + chrono::Duration::days(i as i64 * size);
        let text = if size >= 30 { start.format("%Y-%m") } else { start.format("%m-%d") };
        SharedString::from(text.to_string())
    };
    let mut buckets: Vec<Bucket> = (0..count)
        .map(|i| Bucket {
            label: label(i),
            five: vec![0.; accounts.len()],
            weekly: vec![0.; accounts.len()],
            tokens: 0.,
        })
        .collect();
    for p in plan {
        let (Some(i), Some(a)) = (at(&p.day), accounts.iter().position(|a| *a == p.account)) else { continue };
        buckets[i].five[a] = buckets[i].five[a].max(p.five_hour);
        buckets[i].weekly[a] = buckets[i].weekly[a].max(p.weekly);
    }
    for d in tokens {
        if let Some(i) = at(&d.day) {
            buckets[i].tokens += (d.usage.input + d.usage.output) as f64;
        }
    }
    // The weekly window keeps its level while Claude is idle, so a gap carries the last reading
    // (for at most a week) instead of dropping to zero.
    let reach = (7 / size).max(1) as usize;
    for a in 0..accounts.len() {
        let mut last: Option<(usize, f64)> = None;
        for (i, bucket) in buckets.iter_mut().enumerate() {
            if bucket.weekly[a] > 0.0 {
                last = Some((i, bucket.weekly[a]));
            } else if let Some((_, value)) = last.filter(|(at, _)| i - at <= reach) {
                bucket.weekly[a] = value;
            }
        }
    }
    Series { accounts, buckets }
}

/// One colour per account, far apart in hue so the lines tell apart on a light or a dark window.
/// The theme's own chart colours are all shades of one hue, which is no use for comparing accounts.
const ACCOUNT_COLORS: [(f32, f32, f32); 6] = [
    (215., 0.80, 0.55), // blue
    (28., 0.90, 0.55),  // orange
    (150., 0.60, 0.40), // green
    (320., 0.65, 0.55), // magenta
    (185., 0.70, 0.40), // teal
    (265., 0.60, 0.62), // violet
];

fn tokens_text(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.1}K", n as f64 / 1000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1_000_000.0),
        _ => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

fn size_text(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn heading(text: String, cx: &App) -> AnyElement {
    div()
        .px_1()
        .text_size(rems(0.846))
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn panel(rows: Vec<AnyElement>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .bg(theme.group_box)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius_lg)
        .overflow_hidden()
        .p_3()
        .gap_2()
        .children(rows)
        .into_any_element()
}

impl Render for UsageHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let store = self.store.clone();
        let config = store.read(cx).config();
        let names: BTreeMap<String, String> = store.read(cx).accounts().into_iter().map(|a| (a.id, a.name)).collect();
        let palette = ACCOUNT_COLORS.map(|(h, s, l)| Hsla { h: h / 360., s, l, a: 1. });

        let ranges = ButtonGroup::new("history-range")
            .small()
            .outline()
            .children(Range::ALL.iter().map(|r| Button::new(r.label()).label(r.label()).selected(*r == self.range)))
            .on_click(cx.listener(|this, picked: &Vec<usize>, _, cx| {
                if let Some(range) = picked.first().and_then(|i| Range::ALL.get(*i)).copied() {
                    this.range = range;
                    this.load(false, cx);
                }
            }));

        let mut body = v_flex().gap_4().p_1().child(
            h_flex()
                .justify_between()
                .items_center()
                .child(ranges)
                .when(self.loading, |el| el.child(Spinner::new().small())),
        );

        if let Some(error) = &self.error {
            body = body.child(div().text_color(theme.danger).child(error.clone()));
        }

        match &self.data {
            None if self.error.is_none() => {
                body = body.child(div().text_color(muted).child(t("history.loading")));
            }
            None => {}
            Some(data) => {
                let kept = match (&data.summary.first_day, &data.summary.last_day) {
                    (Some(from), Some(to)) => tf(
                        "history.kept",
                        &[
                            ("samples", &data.summary.plan_samples),
                            ("rows", &data.summary.token_rows),
                            ("from", from),
                            ("to", to),
                            ("size", &size_text(data.summary.bytes)),
                        ],
                    ),
                    _ => t("history.empty"),
                };
                body = body.child(div().px_1().text_size(rems(0.923)).text_color(muted).child(kept));

                let series = series(&data.plan, &data.tokens);
                let name_of = |account: &Option<String>| -> String {
                    match account {
                        Some(id) => names
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| tf("account.unnamed", &[("id", &claude_companion_core::short(id))])),
                        None => t("history.unattributed"),
                    }
                };

                // The accounts side by side.
                if !series.accounts.is_empty() {
                    let mut rows: Vec<AnyElement> = vec![
                        h_flex()
                            .gap_2()
                            .text_size(rems(0.846))
                            .text_color(muted)
                            .child(div().flex_1().child(t("history.col.account")))
                            .child(div().w(px(64.)).child(t("history.col.five")))
                            .child(div().w(px(64.)).child(t("history.col.weekly")))
                            .child(div().w(px(40.)).child(t("history.col.days")))
                            .into_any_element(),
                    ];
                    for (i, account) in series.accounts.iter().enumerate() {
                        let mine: Vec<&PlanDay> = data.plan.iter().filter(|p| p.account == *account).collect();
                        let five = mine.iter().map(|p| p.five_hour).fold(0., f64::max);
                        let weekly = mine.iter().map(|p| p.weekly).fold(0., f64::max);
                        rows.push(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().size_2().rounded_full().bg(palette[i % palette.len()]))
                                .child(div().flex_1().min_w_0().truncate().child(name_of(account)))
                                .child(div().w(px(64.)).child(format!("{five:.0}%")))
                                .child(div().w(px(64.)).child(format!("{weekly:.0}%")))
                                .child(div().w(px(40.)).child(mine.len().to_string()))
                                .into_any_element(),
                        );
                    }
                    body =
                        body.child(v_flex().gap_2().child(heading(t("history.accounts"), cx)).child(panel(rows, cx)));
                }

                // Plan use over time, one line per account.
                for (key, weekly) in [("weekly", true), ("five", false)] {
                    if series.accounts.is_empty() || series.buckets.len() < 2 {
                        break;
                    }
                    let title = t(if weekly { "history.chart.weekly" } else { "history.chart.five" });
                    let mut chart = AreaChart::new(series.buckets.clone())
                        .id(format!("history-{key}"))
                        .x(|b: &Bucket| b.label.clone())
                        .y_domain(0., 100.)
                        .tick_margin((series.buckets.len() / 5).max(1));
                    for (i, account) in series.accounts.iter().enumerate() {
                        let color = palette[i % palette.len()];
                        chart = chart
                            .y(move |b: &Bucket| if weekly { b.weekly[i] } else { b.five[i] })
                            .stroke(color)
                            .fill(color.opacity(0.08))
                            .linear()
                            .name(name_of(account));
                    }
                    chart = chart.tooltip_value(|_, _, v| format!("{v:.0}%").into());
                    body = body.child(
                        v_flex()
                            .gap_2()
                            .child(heading(title, cx))
                            .child(panel(vec![div().h(px(170.)).w_full().child(chart).into_any_element()], cx)),
                    );
                }

                // Tokens.
                if series.buckets.len() >= 2 && series.buckets.iter().any(|b| b.tokens > 0.0) {
                    let chart = BarChart::new(series.buckets.clone())
                        .id("history-tokens")
                        .band(|b: &Bucket| b.label.clone())
                        .value(|b: &Bucket| b.tokens)
                        .name(t("history.export.tokens"))
                        .tooltip_value(|_, v| tokens_text(v as u64).into())
                        .fill(move |_, _, _, _| gpui_kit::Background::from(palette[0]))
                        .tick_margin((series.buckets.len() / 5).max(1));
                    body = body.child(
                        v_flex()
                            .gap_2()
                            .child(heading(t("history.chart.tokens"), cx))
                            .child(panel(vec![div().h(px(170.)).w_full().child(chart).into_any_element()], cx)),
                    );
                }
                if !data.tokens.is_empty() {
                    let mut by_model: BTreeMap<&str, Usage> = BTreeMap::new();
                    for d in &data.tokens {
                        by_model.entry(d.model.as_str()).or_default().add(&d.usage);
                    }
                    let mut models: Vec<(&str, Usage)> = by_model.into_iter().collect();
                    models.sort_by_key(|(_, u)| std::cmp::Reverse(u.total()));
                    let line = |name: String, u: &Usage| -> AnyElement {
                        v_flex()
                            .child(div().truncate().child(name))
                            .child(div().text_size(rems(0.846)).text_color(muted).child(tf(
                                "history.tokens.line",
                                &[
                                    ("input", &tokens_text(u.input)),
                                    ("output", &tokens_text(u.output)),
                                    ("cache", &tokens_text(u.cache_write + u.cache_read)),
                                    ("replies", &u.messages),
                                ],
                            )))
                            .into_any_element()
                    };
                    let rows: Vec<AnyElement> = models.iter().map(|(m, u)| line(m.to_string(), u)).collect();
                    body = body.child(v_flex().gap_2().child(heading(t("history.models"), cx)).child(panel(rows, cx)));
                    let folders: Vec<AnyElement> = data
                        .projects
                        .iter()
                        .filter(|(p, _)| !p.is_empty())
                        .take(8)
                        .map(|(p, u)| {
                            let tail: String = p.chars().rev().take(34).collect::<Vec<_>>().into_iter().rev().collect();
                            line(tail, u)
                        })
                        .collect();
                    if !folders.is_empty() {
                        body = body.child(
                            v_flex().gap_2().child(heading(t("history.projects"), cx)).child(panel(folders, cx)),
                        );
                    }
                }
            }
        }

        // What the user decides: whether to keep it, for how long, and taking it out.
        let keep_choices: [(u32, String); 4] = [
            (365, tn("period.years", 1, &[])),
            (730, tn("period.years", 2, &[])),
            (1825, tn("period.years", 5, &[])),
            (0, t("history.forever")),
        ];
        let selected = keep_choices.iter().position(|(days, _)| *days == config.history_keep_days);
        let keep = ButtonGroup::new("history-keep")
            .small()
            .outline()
            .disabled(!config.history)
            .children(keep_choices.iter().map(|(days, label)| {
                Button::new(format!("keep-{days}"))
                    .label(label.clone())
                    .selected(selected == keep_choices.iter().position(|c| c.0 == *days))
            }))
            .on_click({
                let store = store.clone();
                move |picked: &Vec<usize>, _, cx| {
                    if let Some((days, _)) = picked.first().and_then(|i| keep_choices.get(*i)) {
                        let days = *days;
                        store.update(cx, |s, cx| s.update_config(cx, move |c| c.history_keep_days = days));
                    }
                }
            });
        let row = |title: String, detail: Option<String>, control: AnyElement| -> AnyElement {
            h_flex()
                .gap_3()
                .items_center()
                .child(v_flex().flex_1().min_w_0().gap_0p5().child(div().child(title)).when_some(detail, |el, d| {
                    el.child(div().text_size(rems(0.846)).line_height(rems(1.2)).text_color(muted).child(d))
                }))
                .child(control)
                .into_any_element()
        };
        let on = config.history;
        let switch = Switch::new("history-on").small().checked(on).on_change({
            let store = store.clone();
            move |on: &bool, _, cx| {
                let on = *on;
                store.update(cx, |s, cx| s.update_config(cx, move |c| c.history = on));
            }
        });
        let exports = h_flex()
            .gap_1()
            .child(Button::new("history-export-plan").small().outline().label(t("history.export.plan")).on_click({
                let store = store.clone();
                move |_, _, cx| store.update(cx, |s, cx| s.export_history(false, cx))
            }))
            .child(Button::new("history-export-tokens").small().outline().label(t("history.export.tokens")).on_click(
                {
                    let store = store.clone();
                    move |_, _, cx| store.update(cx, |s, cx| s.export_history(true, cx))
                },
            ));
        let clear = Button::new("history-clear")
            .small()
            .outline()
            .icon(Icon::new(IconName::Delete))
            .label(t("history.clear"))
            .on_click({
                let store = store.clone();
                move |_, window, cx| {
                    let store = store.clone();
                    window.open_alert_dialog(cx, move |alert, _, _| {
                        let store = store.clone();
                        alert
                            .title(t("history.clear.title"))
                            .description(t("history.clear.body"))
                            .confirm()
                            .ok_text(t("history.clear.ok"))
                            .cancel_text(t("dialog.cancel"))
                            .ok_variant(ButtonVariant::Danger)
                            .on_ok(move |_, _, cx| {
                                store.update(cx, |s, cx| s.clear_history(cx));
                                true
                            })
                    });
                }
            });
        let manage = panel(
            vec![
                row(t("history.keep"), Some(t("history.keep.detail")), switch.into_any_element()),
                row(t("history.keep.for"), None, keep.into_any_element()),
                row(t("history.export"), None, exports.into_any_element()),
                row(t("history.delete"), None, clear.into_any_element()),
            ],
            cx,
        );
        body = body.child(v_flex().gap_2().child(heading(t("history.manage"), cx)).child(manage));

        div().id("history-scroll").size_full().overflow_y_scroll().child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "ada";
    const BOB: &str = "bob";

    fn reading(day: &str, account: Option<&str>, five: f64, weekly: f64) -> PlanDay {
        PlanDay { day: day.into(), account: account.map(str::to_string), five_hour: five, weekly, samples: 1 }
    }

    fn tokens(day: &str, input: u64, output: u64) -> TokenDay {
        TokenDay {
            day: day.into(),
            model: "m".into(),
            usage: Usage { input, output, cache_write: 5, cache_read: 5, messages: 1 },
        }
    }

    #[test]
    fn nothing_kept_draws_nothing() {
        let s = series(&[], &[]);
        assert!(s.accounts.is_empty() && s.buckets.is_empty());
    }

    #[test]
    fn each_account_gets_a_line_over_the_days_and_unmatched_samples_come_last() {
        let plan = [
            reading("2026-01-01", Some(BOB), 10.0, 20.0),
            reading("2026-01-01", None, 1.0, 2.0),
            reading("2026-01-01", Some(ADA), 30.0, 40.0),
            reading("2026-01-03", Some(ADA), 50.0, 60.0),
        ];
        let s = series(&plan, &[tokens("2026-01-02", 100, 50)]);
        assert_eq!(s.accounts, [Some(ADA.to_string()), Some(BOB.to_string()), None]);
        assert_eq!(s.buckets.len(), 3, "the first to the last day, with the empty one between");
        assert_eq!(s.buckets[0].five, [30.0, 10.0, 1.0]);
        assert_eq!(s.buckets[2].five, [50.0, 0.0, 0.0]);
        assert_eq!(s.buckets[1].tokens, 150.0, "input and output; the cache is left out");
        assert_eq!(s.buckets[0].label.as_ref(), "01-01");
    }

    #[test]
    fn the_weekly_level_carries_over_a_gap_but_the_five_hour_one_does_not() {
        let plan = [reading("2026-01-01", Some(ADA), 80.0, 40.0), reading("2026-01-04", Some(ADA), 5.0, 45.0)];
        let s = series(&plan, &[]);
        assert_eq!(s.buckets.iter().map(|b| b.weekly[0]).collect::<Vec<_>>(), [40.0, 40.0, 40.0, 45.0]);
        assert_eq!(s.buckets.iter().map(|b| b.five[0]).collect::<Vec<_>>(), [80.0, 0.0, 0.0, 5.0]);
        let far = [reading("2026-01-01", Some(ADA), 1.0, 40.0), reading("2026-01-20", Some(ADA), 1.0, 10.0)];
        let s = series(&far, &[]);
        assert_eq!(s.buckets[3].weekly[0], 40.0, "within a week of the reading");
        assert_eq!(s.buckets[10].weekly[0], 0.0, "a week later it no longer holds");
    }

    #[test]
    fn long_spans_are_grouped_so_the_chart_stays_readable() {
        let plan = [reading("2026-01-01", Some(ADA), 10.0, 10.0), reading("2026-06-01", Some(ADA), 90.0, 90.0)];
        let s = series(&plan, &[]);
        assert!(s.buckets.len() < 40, "{} points", s.buckets.len());
        assert_eq!(s.buckets.iter().map(|b| b.five[0]).fold(0., f64::max), 90.0, "the peak survives the grouping");
    }
}
