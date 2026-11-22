// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! `claude-companion history …`: the long-term record of plan use and tokens.

use super::{confirm, HistoryTable};
use anyhow::{bail, Context as _, Result};
use claude_companion_core::accounts::Roster;
use claude_companion_core::history::{self, History};
use claude_companion_core::Ctx;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

fn open(ctx: &Ctx) -> Result<History> {
    History::open(&ctx.paths)
}

fn sized(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn tokens(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=9_999_999 => format!("{:.1}K", n as f64 / 1000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// What is kept, and each account's highest use and the tokens of the last `days` days.
pub fn show(ctx: &Ctx, days: u32, json: bool) -> Result<()> {
    let history = open(ctx)?;
    let summary = history.summary(&ctx.paths)?;
    let since = history::days_ago(days);
    let roster = Roster::load(&ctx.paths);
    let name = |account: &Option<String>| match account {
        Some(id) => roster
            .slot(id)
            .map(|s| s.label())
            .unwrap_or_else(|| format!("Account {}", claude_companion_core::short(id))),
        None => "(not yet attributed)".to_string(),
    };

    // The highest each account reached in the period.
    let mut peaks: BTreeMap<Option<String>, (f64, f64, usize)> = BTreeMap::new();
    for d in history.plan_days(Some(&since))? {
        let p = peaks.entry(d.account).or_default();
        p.0 = p.0.max(d.five_hour);
        p.1 = p.1.max(d.weekly);
        p.2 += 1;
    }
    let mut by_model: BTreeMap<String, claude_companion_core::transcript::Usage> = BTreeMap::new();
    for d in history.token_days(Some(&since))? {
        by_model.entry(d.model).or_default().add(&d.usage);
    }

    if json {
        let value = serde_json::json!({
            "enabled": ctx.config().history,
            "keepDays": ctx.config().history_keep_days,
            "planSamples": summary.plan_samples,
            "tokenRows": summary.token_rows,
            "firstDay": summary.first_day,
            "lastDay": summary.last_day,
            "bytes": summary.bytes,
            "sinceDay": since,
            "accounts": peaks.iter().map(|(a, (five, week, days))| serde_json::json!({
                "account": a, "name": name(a), "peakFiveHour": five, "peakWeekly": week, "daysWithUse": days,
            })).collect::<Vec<_>>(),
            "models": by_model.iter().map(|(m, u)| serde_json::json!({
                "model": m, "input": u.input, "output": u.output, "cacheWrite": u.cache_write,
                "cacheRead": u.cache_read, "messages": u.messages,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    let config = ctx.config();
    if !config.history {
        println!("The history is off (`claude-companion history on` turns it on). What was kept stays.");
    }
    match (&summary.first_day, &summary.last_day) {
        (Some(first), Some(last)) => println!(
            "Kept: {} plan readings and {} token rows, {first} to {last} ({}).",
            summary.plan_samples,
            summary.token_rows,
            sized(summary.bytes)
        ),
        _ => println!("Nothing kept yet; `claude-companion history collect` reads what Claude has now."),
    }
    println!(
        "Limit: {}.",
        if config.history_keep_days == 0 {
            "none (kept until you clear it)".to_string()
        } else {
            format!("{} days", config.history_keep_days)
        }
    );
    if !peaks.is_empty() {
        println!("\nPlan use since {since} (highest reached):");
        for (account, (five, week, days)) in &peaks {
            println!("  {:<32} 5-hour {five:>5.1}%   weekly {week:>5.1}%   {days} day(s) with use", name(account));
        }
    }
    if !by_model.is_empty() {
        println!("\nTokens since {since}:");
        for (model, u) in &by_model {
            println!(
                "  {:<32} in {:>7}  out {:>7}  cache {:>7}  {} replies",
                model,
                tokens(u.input),
                tokens(u.output),
                tokens(u.cache_write + u.cache_read),
                u.messages
            );
        }
    }
    Ok(())
}

/// Read what Claude has now into the history.
pub fn collect(ctx: &Ctx) -> Result<()> {
    if !ctx.config().history {
        bail!("the history is off; `claude-companion history on` turns it on");
    }
    let done = history::collect(ctx)?;
    println!(
        "{} new plan reading(s); {} transcript file(s) counted; {} old row(s) trimmed.",
        done.new_samples, done.files_counted, done.pruned
    );
    Ok(())
}

/// Turn the history on or off.
pub fn switch(ctx: &Ctx, on: bool) -> Result<()> {
    let mut config = ctx.config();
    config.history = on;
    config.save(&ctx.paths)?;
    println!("The history is {}.", if on { "on" } else { "off (what was kept stays)" });
    Ok(())
}

/// Set how long the history is kept: a number of days, or `forever`.
pub fn keep(ctx: &Ctx, how_long: &str) -> Result<()> {
    let days = match how_long.trim().to_ascii_lowercase().as_str() {
        "forever" | "0" | "none" => 0,
        other => other
            .parse::<u32>()
            .ok()
            .filter(|d| (1..=36_500).contains(d))
            .with_context(|| format!("{how_long} is not a number of days (1 to 36500) or `forever`"))?,
    };
    let mut config = ctx.config();
    config.history_keep_days = days;
    config.save(&ctx.paths)?;
    if days == 0 {
        println!("The history is kept until you clear it.");
    } else {
        println!("The history keeps {days} days; older rows are trimmed once a day.");
    }
    Ok(())
}

/// Write one table as CSV, to a file or the terminal.
pub fn export(ctx: &Ctx, table: HistoryTable, out: Option<PathBuf>) -> Result<()> {
    let history = open(ctx)?;
    let write = |w: &mut dyn Write| -> Result<()> {
        match table {
            HistoryTable::Plan => history.write_plan_csv(&mut *w),
            HistoryTable::Tokens => history.write_tokens_csv(&mut *w),
        }
    };
    match out {
        Some(path) => {
            let mut file = std::fs::File::create(&path).with_context(|| format!("cannot write {}", path.display()))?;
            write(&mut file)?;
            println!("Wrote {}.", path.display());
        }
        None => write(&mut std::io::stdout().lock())?,
    }
    Ok(())
}

/// Delete the whole history.
pub fn clear(ctx: &Ctx, yes: bool) -> Result<()> {
    let mut history = open(ctx)?;
    let summary = history.summary(&ctx.paths)?;
    if !yes
        && !confirm(&format!(
            "Delete {} plan readings and {} token rows? This cannot be undone.",
            summary.plan_samples, summary.token_rows
        ))
    {
        println!("Nothing deleted.");
        return Ok(());
    }
    history.clear()?;
    println!("The history is empty.");
    Ok(())
}
