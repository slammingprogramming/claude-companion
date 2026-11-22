// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! Our own long-term record of usage, kept in a SQLite file in the state folder.
//!
//! Claude Desktop keeps its plan-usage samples for 30 days and Claude Code deletes old
//! transcripts, so both fade away. This module copies what they hold into a database of ours
//! before it goes, and keeps it for as long as the user chooses (`historyKeepDays`, 730 by
//! default; 0 keeps it for as long as the user does). Nothing leaves the computer.
//!
//! Two kinds of data are kept:
//!
//! * **Plan use**: each sample Desktop recorded (percent of the 5-hour and the weekly window),
//!   with the account it belongs to. Desktop names an organization, not an account; the account
//!   is filled in as soon as [`crate::usage::owners`] can say whose organization it is, and
//!   samples taken earlier are attributed then.
//! * **Tokens**: Claude Code's transcripts add up to tokens per day, model and project folder.
//!   Transcripts are shared by every account, so these are totals, not per-account figures. A
//!   day's figures only ever grow: when old transcripts are deleted the day is not lowered.
//!
//! Collecting is cheap and repeatable: samples are keyed by time and organization, so reading the
//! same record again changes nothing, and the transcripts are only counted again when their
//! files changed.

use crate::accounts::{self, Roster};
use crate::ctx::Ctx;
use crate::paths::Paths;
use crate::transcript::{self, DayRow, Usage};
use crate::usage::{self, Sample};
use anyhow::{Context, Result};
use chrono::{Duration as Span, Local, TimeZone as _};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS plan_samples (
    at_ms     INTEGER NOT NULL,
    org       TEXT NOT NULL,
    day       TEXT NOT NULL,
    account   TEXT,
    five_hour REAL,
    weekly    REAL,
    PRIMARY KEY (at_ms, org)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS plan_samples_by_day ON plan_samples (day, account);
CREATE TABLE IF NOT EXISTS token_days (
    day         TEXT NOT NULL,
    model       TEXT NOT NULL,
    project     TEXT NOT NULL,
    input       INTEGER NOT NULL,
    output      INTEGER NOT NULL,
    cache_write INTEGER NOT NULL,
    cache_read  INTEGER NOT NULL,
    messages    INTEGER NOT NULL,
    PRIMARY KEY (day, model, project)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
";

/// The local day (`YYYY-MM-DD`) of a Unix time in seconds.
pub fn day_of(at: f64) -> String {
    match Local.timestamp_millis_opt((at * 1000.0) as i64).single() {
        Some(t) => t.format("%Y-%m-%d").to_string(),
        None => "1970-01-01".into(),
    }
}

/// The day `days` days before today.
pub fn days_ago(days: u32) -> String {
    (Local::now() - Span::days(i64::from(days))).format("%Y-%m-%d").to_string()
}

/// The highest plan use one account reached on one day.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanDay {
    pub day: String,
    /// The account's UUID; none for samples whose organization no account owns alone.
    pub account: Option<String>,
    /// The highest percent of the 5-hour window seen that day.
    pub five_hour: f64,
    /// The highest percent of the weekly window seen that day.
    pub weekly: f64,
    pub samples: u64,
}

/// One model's tokens on one day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenDay {
    pub day: String,
    pub model: String,
    pub usage: Usage,
}

/// What the database holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub plan_samples: u64,
    pub token_rows: u64,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
    pub bytes: u64,
}

/// What one [`collect`] changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Collected {
    pub new_samples: usize,
    /// How many transcript files were counted this time.
    pub files_counted: usize,
    /// Rows removed because they were older than the limit.
    pub pruned: u64,
}

pub struct History {
    conn: Connection,
}

impl History {
    /// Open (and create) the database in the state folder.
    pub fn open(paths: &Paths) -> Result<History> {
        History::open_at(&paths.history_db())
    }

    pub fn open_at(path: &Path) -> Result<History> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        // The app, the background agent and the command line may all write; wait for each other.
        conn.busy_timeout(Duration::from_secs(10))?;
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.execute_batch(SCHEMA)?;
        Ok(History { conn })
    }

    /// Keep Desktop's samples, attributing each to the account that owns its organization.
    /// Returns how many were new.
    pub fn record_plan(&mut self, samples: &[Sample], owners: &BTreeMap<String, String>) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut added = 0;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO plan_samples (at_ms, org, day, account, five_hour, weekly)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for s in samples {
                let org = s.org.clone().unwrap_or_default();
                let account = owners.get(&org);
                added += insert.execute(params![
                    (s.at * 1000.0).round() as i64,
                    org,
                    day_of(s.at),
                    account,
                    s.five_hour,
                    s.weekly
                ])?;
            }
            // Samples taken before the organization's owner was known.
            let mut attribute =
                tx.prepare_cached("UPDATE plan_samples SET account = ?1 WHERE org = ?2 AND account IS NULL")?;
            for (org, account) in owners {
                attribute.execute(params![account, org])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Keep the day's token counts, never lowering a figure already kept (Claude Code deletes old
    /// transcripts, so a later count of the same day can be smaller).
    pub fn record_tokens(&mut self, rows: &[DayRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare_cached(
                "INSERT INTO token_days (day, model, project, input, output, cache_write, cache_read, messages)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (day, model, project) DO UPDATE SET
                    input = MAX(input, excluded.input),
                    output = MAX(output, excluded.output),
                    cache_write = MAX(cache_write, excluded.cache_write),
                    cache_read = MAX(cache_read, excluded.cache_read),
                    messages = MAX(messages, excluded.messages)",
            )?;
            for r in rows {
                upsert.execute(params![
                    r.day,
                    r.model,
                    r.project,
                    r.usage.input as i64,
                    r.usage.output as i64,
                    r.usage.cache_write as i64,
                    r.usage.cache_read as i64,
                    r.usage.messages as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The highest plan use of each account on each day since `since` (`YYYY-MM-DD`), oldest first.
    pub fn plan_days(&self, since: Option<&str>) -> Result<Vec<PlanDay>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT day, account, MAX(COALESCE(five_hour, 0)), MAX(COALESCE(weekly, 0)), COUNT(*)
             FROM plan_samples WHERE day >= ?1 GROUP BY day, account ORDER BY day, account",
        )?;
        let rows = stmt.query_map(params![since.unwrap_or("0000-00-00")], |r| {
            Ok(PlanDay {
                day: r.get(0)?,
                account: r.get(1)?,
                five_hour: r.get(2)?,
                weekly: r.get(3)?,
                samples: r.get::<_, i64>(4)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tokens of each model on each day since `since`, oldest first.
    pub fn token_days(&self, since: Option<&str>) -> Result<Vec<TokenDay>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT day, model, SUM(input), SUM(output), SUM(cache_write), SUM(cache_read), SUM(messages)
             FROM token_days WHERE day >= ?1 GROUP BY day, model ORDER BY day, model",
        )?;
        let rows = stmt.query_map(params![since.unwrap_or("0000-00-00")], |r| {
            Ok(TokenDay { day: r.get(0)?, model: r.get(1)?, usage: usage_of(r, 2)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tokens of each project folder since `since`, the busiest first.
    pub fn token_projects(&self, since: Option<&str>) -> Result<Vec<(String, Usage)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT project, SUM(input), SUM(output), SUM(cache_write), SUM(cache_read), SUM(messages)
             FROM token_days WHERE day >= ?1 GROUP BY project
             ORDER BY SUM(input + output + cache_write + cache_read) DESC",
        )?;
        let rows =
            stmt.query_map(params![since.unwrap_or("0000-00-00")], |r| Ok((r.get::<_, String>(0)?, usage_of(r, 1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn summary(&self, paths: &Paths) -> Result<Summary> {
        let count = |sql: &str| -> Result<u64> { Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64) };
        let (first, last): (Option<String>, Option<String>) = self.conn.query_row(
            "SELECT MIN(d), MAX(d) FROM (SELECT day AS d FROM plan_samples UNION ALL SELECT day FROM token_days)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let bytes = std::fs::metadata(paths.history_db()).map(|m| m.len()).unwrap_or(0);
        Ok(Summary {
            plan_samples: count("SELECT COUNT(*) FROM plan_samples")?,
            token_rows: count("SELECT COUNT(*) FROM token_days")?,
            first_day: first,
            last_day: last,
            bytes,
        })
    }

    /// Delete everything older than `keep_days` days (0 deletes nothing). Returns the rows removed.
    pub fn prune(&mut self, keep_days: u32) -> Result<u64> {
        if keep_days == 0 {
            return Ok(0);
        }
        let cutoff = days_ago(keep_days);
        let tx = self.conn.transaction()?;
        let a = tx.execute("DELETE FROM plan_samples WHERE day < ?1", params![cutoff])?;
        let b = tx.execute("DELETE FROM token_days WHERE day < ?1", params![cutoff])?;
        tx.commit()?;
        Ok((a + b) as u64)
    }

    /// Delete everything.
    pub fn clear(&mut self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM plan_samples; DELETE FROM token_days; DELETE FROM meta;")?;
        Ok(())
    }

    fn meta(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).ok()
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Every plan sample as CSV: time, day, account, organization, 5-hour and weekly percent.
    pub fn write_plan_csv<W: Write + ?Sized>(&self, out: &mut W) -> Result<()> {
        writeln!(out, "time_utc,day,account,organization,five_hour_percent,weekly_percent")?;
        let mut stmt = self
            .conn
            .prepare("SELECT at_ms, day, account, org, five_hour, weekly FROM plan_samples ORDER BY at_ms, org")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let at: i64 = r.get(0)?;
            let time = chrono::DateTime::from_timestamp_millis(at).map(|t| t.to_rfc3339()).unwrap_or_default();
            let number = |i: usize| -> Result<String> {
                Ok(r.get::<_, Option<f64>>(i)?.map(|n| n.to_string()).unwrap_or_default())
            };
            writeln!(
                out,
                "{time},{},{},{},{},{}",
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, String>(3)?,
                number(4)?,
                number(5)?
            )?;
        }
        Ok(())
    }

    /// Every token row as CSV.
    pub fn write_tokens_csv<W: Write + ?Sized>(&self, out: &mut W) -> Result<()> {
        writeln!(out, "day,model,project,input,output,cache_write,cache_read,messages")?;
        let mut stmt = self.conn.prepare(
            "SELECT day, model, project, input, output, cache_write, cache_read, messages
             FROM token_days ORDER BY day, model, project",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            writeln!(
                out,
                "{},{},{},{},{},{},{},{}",
                r.get::<_, String>(0)?,
                csv(&r.get::<_, String>(1)?),
                csv(&r.get::<_, String>(2)?),
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?
            )?;
        }
        Ok(())
    }
}

fn usage_of(r: &rusqlite::Row<'_>, first: usize) -> rusqlite::Result<Usage> {
    let n = |i: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(first + i)?.max(0) as u64) };
    Ok(Usage { input: n(0)?, output: n(1)?, cache_write: n(2)?, cache_read: n(3)?, messages: n(4)? })
}

/// A CSV field: quoted when it holds a comma, a quote or a line break.
fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Copy what Desktop and Claude Code hold into the database, and trim it to the user's limit.
/// Does nothing when the user turned the history off.
pub fn collect(ctx: &Ctx) -> Result<Collected> {
    let config = ctx.config();
    if !config.history {
        return Ok(Collected::default());
    }
    let mut history = History::open(&ctx.paths)?;
    let roster = Roster::load(&ctx.paths);
    let owners = accounts::org_owners(ctx, &roster);
    let mut out =
        Collected { new_samples: history.record_plan(&usage::read(&ctx.paths), &owners)?, ..Collected::default() };

    // The first time, every transcript; after that, those changed since the last count (with a
    // day's margin), which is where new tokens can be. A day's kept figure never goes down.
    let started = std::time::SystemTime::now();
    let since = history
        .meta("tokens_counted_ms")
        .and_then(|ms| ms.parse::<u64>().ok())
        .map(|ms| std::time::UNIX_EPOCH + Duration::from_millis(ms) - Duration::from_secs(86_400));
    let (rows, files) = transcript::usage_rows(&ctx.paths.projects, since);
    if !rows.is_empty() {
        history.record_tokens(&rows)?;
    }
    let ms = started.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    history.set_meta("tokens_counted_ms", &ms.to_string())?;
    out.files_counted = files;

    let today = days_ago(0);
    if history.meta("pruned").as_deref() != Some(today.as_str()) {
        out.pruned = history.prune(config.history_keep_days)?;
        history.set_meta("pruned", &today)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "11111111-1111-4111-8111-111111111111";
    const BOB: &str = "22222222-2222-4222-8222-222222222222";
    const ADA_ORG: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const BOB_ORG: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn sample(at: f64, org: &str, five_hour: f64, weekly: f64) -> Sample {
        Sample { at, org: Some(org.into()), five_hour: Some(five_hour), weekly: Some(weekly) }
    }

    fn owners() -> BTreeMap<String, String> {
        BTreeMap::from([(ADA_ORG.to_string(), ADA.to_string()), (BOB_ORG.to_string(), BOB.to_string())])
    }

    fn open() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().unwrap();
        let history = History::open_at(&dir.path().join("h.sqlite")).unwrap();
        (dir, history)
    }

    fn now() -> f64 {
        Local::now().timestamp() as f64
    }

    #[test]
    fn samples_are_kept_once_and_split_by_account() {
        let (_dir, mut h) = open();
        let t = now();
        let samples = [
            sample(t - 60.0, ADA_ORG, 10.0, 20.0),
            sample(t - 30.0, ADA_ORG, 40.0, 25.0),
            sample(t, BOB_ORG, 5.0, 60.0),
        ];
        assert_eq!(h.record_plan(&samples, &owners()).unwrap(), 3);
        assert_eq!(h.record_plan(&samples, &owners()).unwrap(), 0, "the same record twice adds nothing");
        let days = h.plan_days(None).unwrap();
        let ada = days.iter().find(|d| d.account.as_deref() == Some(ADA)).unwrap();
        assert_eq!((ada.five_hour, ada.weekly, ada.samples), (40.0, 25.0, 2));
        let bob = days.iter().find(|d| d.account.as_deref() == Some(BOB)).unwrap();
        assert_eq!((bob.five_hour, bob.weekly), (5.0, 60.0));
    }

    #[test]
    fn samples_taken_before_their_owner_was_known_are_attributed_later() {
        let (_dir, mut h) = open();
        h.record_plan(&[sample(now() - 10.0, ADA_ORG, 1.0, 2.0)], &BTreeMap::new()).unwrap();
        assert_eq!(h.plan_days(None).unwrap()[0].account, None);
        h.record_plan(&[], &owners()).unwrap();
        assert_eq!(h.plan_days(None).unwrap()[0].account.as_deref(), Some(ADA));
    }

    fn row(day: &str, model: &str, input: u64) -> DayRow {
        DayRow {
            day: day.into(),
            model: model.into(),
            project: "/work/app".into(),
            usage: Usage { input, output: input / 2, cache_write: 0, cache_read: 0, messages: 3 },
        }
    }

    #[test]
    fn a_days_tokens_never_go_down() {
        let (_dir, mut h) = open();
        h.record_tokens(&[row("2026-01-01", "opus", 1000)]).unwrap();
        // The transcripts were partly deleted, so the same day now counts lower.
        h.record_tokens(&[row("2026-01-01", "opus", 400)]).unwrap();
        assert_eq!(h.token_days(None).unwrap()[0].usage.input, 1000);
        // Later replies the same day raise it.
        h.record_tokens(&[row("2026-01-01", "opus", 1500)]).unwrap();
        assert_eq!(h.token_days(None).unwrap()[0].usage.input, 1500);
    }

    #[test]
    fn tokens_add_up_by_day_model_and_project() {
        let (_dir, mut h) = open();
        let mut other = row("2026-01-01", "opus", 100);
        other.project = "/work/other".into();
        h.record_tokens(&[row("2026-01-01", "opus", 1000), other, row("2026-01-02", "sonnet", 50)]).unwrap();
        let days = h.token_days(None).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].usage.input, 1100, "two projects, one model, one day");
        let projects = h.token_projects(None).unwrap();
        assert_eq!(projects[0].0, "/work/app");
        assert_eq!(h.token_days(Some("2026-01-02")).unwrap().len(), 1, "since drops earlier days");
    }

    #[test]
    fn old_rows_are_trimmed_to_the_limit_and_zero_keeps_everything() {
        let (_dir, mut h) = open();
        let t = now();
        h.record_plan(&[sample(t - 400.0 * 86_400.0, ADA_ORG, 1.0, 1.0), sample(t, ADA_ORG, 2.0, 2.0)], &owners())
            .unwrap();
        h.record_tokens(&[row("2000-01-01", "opus", 10), row(&days_ago(0), "opus", 10)]).unwrap();
        assert_eq!(h.prune(0).unwrap(), 0);
        assert_eq!(h.prune(365).unwrap(), 2);
        assert_eq!(h.plan_days(None).unwrap().len(), 1);
        assert_eq!(h.token_days(None).unwrap().len(), 1);
        h.clear().unwrap();
        assert!(h.plan_days(None).unwrap().is_empty());
    }

    #[test]
    fn the_summary_and_the_csv_describe_what_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"), dir.path().join("state"));
        let mut h = History::open(&paths).unwrap();
        h.record_plan(&[sample(now(), ADA_ORG, 12.5, 30.0)], &owners()).unwrap();
        let mut project = row("2026-01-01", "opus", 7);
        project.project = "/with,comma \"quoted\"".into();
        h.record_tokens(&[project]).unwrap();
        let summary = h.summary(&paths).unwrap();
        assert_eq!((summary.plan_samples, summary.token_rows), (1, 1));
        assert_eq!(summary.first_day.as_deref(), Some("2026-01-01"));
        assert!(summary.bytes > 0);
        let (mut plan, mut tokens) = (Vec::new(), Vec::new());
        h.write_plan_csv(&mut plan).unwrap();
        h.write_tokens_csv(&mut tokens).unwrap();
        let plan = String::from_utf8(plan).unwrap();
        assert!(plan.lines().nth(1).unwrap().ends_with(&format!(",{ADA},{ADA_ORG},12.5,30")), "{plan}");
        let tokens = String::from_utf8(tokens).unwrap();
        assert!(tokens.contains("\"/with,comma \"\"quoted\"\"\",7,3,0,0,3"), "{tokens}");
    }
}
