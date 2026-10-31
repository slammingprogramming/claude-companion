// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// A Rust port of the sidebar-group merge of claude-desktop-session-sync (MIT, Copyright (c) 2026
// RasmusKD); see NOTICE.md and LICENSES/.
//! Sidebar groups: one set of custom groups for every account.
//!
//! Desktop keeps each account's custom sidebar groups in its Local Storage (a LevelDB it holds
//! open while it runs) and mirrors them into its config file. This module merges those per-account
//! scopes three ways against the result of the last run and writes the merge back into every
//! scope.
//!
//! Rules that keep it safe:
//! - Only while Claude is closed. Opening the database while Desktop runs could corrupt it, so a
//!   run then is *deferred*, never reported as done.
//! - Backup first: the database is copied before it is opened for a write, the config file before
//!   it is spliced. Nothing is opened in place until the merge says something has to change, and a
//!   write that does not read back correctly is rolled back from the backup.
//! - Three-way merge against the last result (the base): a group the base never held is an
//!   addition; a group is a deletion only when a scope lacks it *after* that scope was last seen
//!   in step with its account's server row. A deleted id is remembered and never comes back.
//!   Without a base nothing is ever deleted.
//! - claude.ai is the source of truth for each account and Local Storage only a cache of it: at
//!   start-up Desktop replaces the local list with the server's, unless the *pending marker* names
//!   the account it starts on, in which case it pushes the local state. So a write reaches an
//!   account only once Desktop has started on it with the marker set; the marker is set for the
//!   account Desktop will start on.
//! - The config mirror is output only (Desktop writes it from Local Storage, so reading it back
//!   would resurrect whatever Desktop flushed last); every byte outside the spliced value stays.

use crate::ctx::Ctx;
use crate::desktop;
use crate::fsx;
use crate::model::Surface;
use crate::scan;
use anyhow::{anyhow, bail, Context as _, Result};
use rusty_leveldb::{LdbIterator, Options, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const ORIGIN: &str = "https://claude.ai";
const SCOPES_KEY: &str = "LSS-persisted.dframe-group-scopes";
const STORE_KEY: &str = "dframe-store";
const CONFIG_PATH: [&str; 3] = ["preferences", "epitaxyPrefs", "dframe-group-scopes"];
/// Desktop's own server-sync bookkeeping, kept beside the store: the account whose row was
/// reconciled last, the push-instead-of-pull marker (value: `account/org`), and the keys it stamps
/// on logout and on a confirmed login, which date the last account switch.
const SYNC_OWNER_KEY: &str = "ccd-sync-owner";
const SYNC_PENDING_KEY: &str = "ccd-sync-pending:ccd/dframe-store";
const SESSION_MARKER_KEY: &str = "rq-cache-confirmed-session-marker";
const LOGOUT_SUFFIX: &str = "-logout-at";
const LEVELDB_BACKUPS_KEPT: usize = 5;
const CONFIG_BACKUPS_KEPT: usize = 5;
/// Microseconds between 1601-01-01 (the Windows epoch Chromium counts from) and 1970-01-01.
const WINDOWS_EPOCH_OFFSET_US: u64 = 11_644_473_600_000_000;

// ── Chromium Local Storage encoding ──────────────────────────────────────────

/// Map entries: key = `_` + origin + NUL + (0x00 + UTF-16LE | 0x01 + Latin-1); value likewise.
pub fn decode_string(buf: &[u8]) -> Result<String> {
    match buf.first() {
        None => Ok(String::new()),
        Some(0x00) => {
            let units: Vec<u16> = buf[1..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Ok(String::from_utf16_lossy(&units))
        }
        Some(0x01) => Ok(buf[1..].iter().map(|b| *b as char).collect()),
        Some(b) => bail!("unknown string format byte 0x{b:02x}"),
    }
}

pub fn encode_string(s: &str) -> Vec<u8> {
    if s.chars().all(|c| (c as u32) <= 0xff) {
        let mut out = vec![0x01];
        out.extend(s.chars().map(|c| c as u8));
        out
    } else {
        let mut out = vec![0x00];
        for unit in s.encode_utf16() {
            out.extend(unit.to_le_bytes());
        }
        out
    }
}

fn map_key_prefix(origin: &str) -> Vec<u8> {
    let mut out = format!("_{origin}").into_bytes();
    out.push(0);
    out
}

fn decode_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let (mut result, mut shift) = (0u64, 0u32);
    loop {
        let b = *buf.get(pos)?;
        pos += 1;
        result |= u64::from(b & 0x7f).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some((result, pos));
        }
        shift += 7;
    }
}

fn encode_varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n > 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
            return out;
        }
    }
}

/// `META:<origin>` holds a protobuf `{1: last modified (µs since 1601), 2: size in bytes}`; the
/// size is the byte length of every key (after the origin prefix) plus every value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Meta {
    last_modified_us: u64,
    size_bytes: u64,
}

fn decode_meta(buf: &[u8]) -> Meta {
    let mut meta = Meta::default();
    let mut pos = 0;
    while pos < buf.len() {
        let tag = buf[pos];
        pos += 1;
        let Some((val, next)) = decode_varint(buf, pos) else { break };
        pos = next;
        match tag {
            0x08 => meta.last_modified_us = val,
            0x10 => meta.size_bytes = val,
            _ => {}
        }
    }
    meta
}

fn encode_meta(meta: Meta) -> Vec<u8> {
    let mut out = vec![0x08];
    out.extend(encode_varint(meta.last_modified_us));
    out.push(0x10);
    out.extend(encode_varint(meta.size_bytes));
    out
}

// ── Three-way merge ──────────────────────────────────────────────────────────

/// One scope's entry with the three fields the merge understands pulled out; everything else
/// rides along in `extras`.
struct Norm {
    groups: Vec<Value>,
    assignments: Map<String, Value>,
    order: Map<String, Value>,
    extras: Vec<(String, Value)>,
}

fn group_id(g: &Value) -> &str {
    g.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn norm(entry: Option<&Value>) -> Norm {
    let empty = Map::new();
    let obj = entry.and_then(Value::as_object).unwrap_or(&empty);
    let groups = match obj.get("groups") {
        Some(Value::Array(a)) => a.iter().filter(|g| g.get("id").is_some_and(Value::is_string)).cloned().collect(),
        _ => Vec::new(),
    };
    let map_of = |k: &str| match obj.get(k) {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let extras = obj
        .iter()
        .filter(|(k, _)| !["groups", "assignments", "order"].contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Norm { groups, assignments: map_of("assignments"), order: map_of("order"), extras }
}

fn uniq<T: Clone + Eq + std::hash::Hash>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|x| seen.insert(x.clone())).collect()
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    match v {
        Some(Value::Array(a)) => Some(a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
        _ => None,
    }
}

/// What the last run left behind, per scope and overall.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Base {
    pub version: u32,
    pub scopes: Vec<String>,
    pub entry: Value,
    pub written_at: String,
    /// Per scope: the entry last seen in step with the account's server row.
    pub published: BTreeMap<String, Stamped>,
    /// Per scope: the entry we wrote under a pending marker.
    pub pending: BTreeMap<String, Stamped>,
    /// Deleted group ids and when; they never come back.
    pub tombstones: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stamped {
    pub at: String,
    pub entry: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub groups_added: Vec<String>,
    pub groups_removed: Vec<String>,
    pub groups_changed: Vec<String>,
    pub groups_withheld: Vec<String>,
    pub assignments_changed: usize,
}

/// Equalize `scopes`.
///
/// `present`: the entry actually found in Local Storage per scope (absent scopes are seed
/// targets). `base`: the last result. `prefer`: scope order that breaks ties, most recent first.
/// `reconciled`: per scope, the entry it was last known to hold after Desktop reconciled it with
/// its account's server row; only a departure from that entry is a user edit, and a scope without
/// one contributes additions only.
pub fn merge_scopes(
    scopes: &[String],
    present: &BTreeMap<String, Value>,
    base: Option<&Base>,
    prefer: &[String],
    reconciled: &BTreeMap<String, Value>,
) -> (Value, Changes) {
    let pref: Vec<String> =
        uniq(prefer.iter().filter(|s| scopes.contains(s)).chain(scopes.iter()).cloned().collect::<Vec<_>>());
    let b = norm(base.map(|b| &b.entry));
    let no_tombstones = BTreeMap::new();
    let tombstones = base.map_or(&no_tombstones, |b| &b.tombstones);
    let present_scopes: Vec<&String> = pref.iter().filter(|s| present.contains_key(*s)).collect();
    let cur: HashMap<&String, Norm> = present_scopes.iter().map(|s| (*s, norm(present.get(*s)))).collect();
    let known: HashMap<&String, Norm> =
        present_scopes.iter().filter_map(|s| reconciled.get(*s).map(|r| (*s, norm(Some(r))))).collect();
    let mut changes = Changes::default();

    let base_groups: HashMap<&str, &Value> = b.groups.iter().map(|g| (group_id(g), g)).collect();
    let ids: Vec<String> = uniq(
        b.groups
            .iter()
            .map(|g| group_id(g).to_string())
            .chain(present_scopes.iter().flat_map(|s| cur[*s].groups.iter().map(|g| group_id(g).to_string())))
            .collect::<Vec<_>>(),
    );
    let holds = |s: &&String, id: &str| cur[*s].groups.iter().any(|g| group_id(g) == id);
    let mut merged: HashMap<String, Value> = HashMap::new();
    for id in &ids {
        let holders: Vec<&&String> = present_scopes.iter().filter(|s| holds(s, id)).collect();
        // A deleted id never returns: an account that has not logged in since the deletion still
        // carries it in its server row and hands it back on its next start.
        if tombstones.contains_key(id) {
            if !holders.is_empty() {
                changes.groups_withheld.push(id.clone());
            }
            continue;
        }
        let find = |s: &&String| cur[*s].groups.iter().find(|g| group_id(g) == id).cloned();
        if let Some(in_base) = base_groups.get(id.as_str()) {
            let deleted_by = present_scopes.iter().any(|s| {
                !holders.contains(&s) && known.get(*s).is_some_and(|k| k.groups.iter().any(|g| group_id(g) == id))
            });
            if deleted_by {
                changes.groups_removed.push(id.clone());
                continue;
            }
            // The base is memory, not a source: a group no scope holds any more is gone from
            // every account and is not resurrected from the last run.
            if holders.is_empty() {
                changes.groups_withheld.push(id.clone());
                continue;
            }
            let changed = holders.iter().filter_map(|s| find(s)).find(|g| g != *in_base);
            if changed.is_some() {
                changes.groups_changed.push(id.clone());
            }
            merged.insert(id.clone(), changed.unwrap_or_else(|| (*in_base).clone()));
        } else if let Some(first) = holders.first() {
            changes.groups_added.push(id.clone());
            if let Some(g) = find(first) {
                merged.insert(id.clone(), g);
            }
        }
    }

    // Group list order: the first preferred scope whose sequence departs from the base decides;
    // ids it does not know are appended in preference order.
    let base_seq: Vec<String> = b.groups.iter().map(|g| group_id(g).to_string()).collect();
    let mut seq = base_seq.clone();
    for s in &present_scopes {
        let s_seq: Vec<String> = cur[*s].groups.iter().map(|g| group_id(g).to_string()).collect();
        if s_seq != base_seq {
            seq = s_seq;
            break;
        }
    }
    let group_order: Vec<String> = uniq(
        seq.into_iter()
            .chain(present_scopes.iter().flat_map(|s| cur[*s].groups.iter().map(|g| group_id(g).to_string())))
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .filter(|id| merged.contains_key(id))
    .collect();
    let groups: Vec<Value> = group_order.iter().map(|id| merged[id].clone()).collect();

    let mut assignments = Map::new();
    let akeys: Vec<String> = uniq(
        b.assignments
            .keys()
            .cloned()
            .chain(present_scopes.iter().flat_map(|s| cur[*s].assignments.keys().cloned()))
            .collect::<Vec<_>>(),
    );
    for key in &akeys {
        let base_val = b.assignments.get(key);
        let val: Option<Value> = if let Some(bv) = base_val {
            // A scope moves the chat when it assigns it elsewhere, or when it lacks an assignment
            // its reconciled entry held (the user removed it there).
            let mover = present_scopes.iter().find(|s| {
                cur[**s].assignments.get(key) != Some(bv)
                    && (cur[**s].assignments.contains_key(key)
                        || known.get(**s).is_some_and(|k| k.assignments.get(key) == Some(bv)))
            });
            match mover {
                Some(s) => cur[*s].assignments.get(key).cloned(),
                None => Some(bv.clone()),
            }
        } else {
            present_scopes.iter().find_map(|s| cur[*s].assignments.get(key).cloned())
        };
        if let Some(v) = &val {
            if v.as_str().is_some_and(|id| merged.contains_key(id)) {
                assignments.insert(key.clone(), v.clone());
            }
        }
        if val.as_ref() != base_val {
            changes.assignments_changed += 1;
        }
    }

    let mut order = Map::new();
    for id in &group_order {
        let base_list = strings(b.order.get(id)).unwrap_or_default();
        let mut list = base_list.clone();
        for s in &present_scopes {
            if let Some(s_list) = strings(cur[*s].order.get(id)) {
                if s_list != base_list {
                    list = s_list;
                    break;
                }
            }
        }
        let in_group = |k: &String| assignments.get(k).and_then(Value::as_str) == Some(id.as_str());
        let members = assignments.keys().filter(|k| in_group(k)).cloned();
        let from_scopes = present_scopes.iter().flat_map(|s| strings(cur[*s].order.get(id)).unwrap_or_default());
        let ordered: Vec<String> = uniq(list.into_iter().chain(from_scopes).chain(members).collect::<Vec<_>>())
            .into_iter()
            .filter(|k| in_group(k))
            .collect();
        // An order key nobody wrote is not invented: an empty list the app never stored would
        // read as a change on every run.
        let existed = b.order.contains_key(id) || present_scopes.iter().any(|s| cur[*s].order.contains_key(id));
        if !ordered.is_empty() || existed {
            order.insert(id.clone(), json!(ordered));
        }
    }

    // Unknown top-level fields ride along from the most recently active holder so a future app
    // field is never silently dropped from the scope we rewrite.
    let mut extras: Vec<(String, Value)> = Vec::new();
    for s in present_scopes.iter().rev() {
        for (k, v) in &cur[*s].extras {
            extras.retain(|(ek, _)| ek != k);
            extras.push((k.clone(), v.clone()));
        }
    }
    let mut entry = Map::new();
    for (k, v) in extras {
        entry.insert(k, v);
    }
    entry.insert("groups".into(), Value::Array(groups));
    entry.insert("assignments".into(), Value::Object(assignments));
    entry.insert("order".into(), Value::Object(order));
    (Value::Object(entry), changes)
}

/// The LSS wrapper is the app's pruned projection of the store (assignments to unknown groups and
/// groups nobody references are dropped), so it legitimately holds less. It holds MORE only when
/// the two have diverged.
fn lss_exceeds_store(lss: &Value, store: &Value) -> bool {
    let (l, s) = (norm(Some(lss)), norm(Some(store)));
    let ids: HashSet<&str> = s.groups.iter().map(group_id).collect();
    l.groups.iter().any(|g| !ids.contains(group_id(g)))
        || l.assignments
            .iter()
            .any(|(k, v)| v.as_str().is_some_and(|id| ids.contains(id)) && s.assignments.get(k) != Some(v))
}

// ── Config mirror: a raw-text splice with a walk from the document root ──────

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// End of the JSON value starting at `i` (after whitespace), or `None` if it is cut off.
fn skip_value(s: &[u8], i: usize) -> Option<usize> {
    let mut i = skip_ws(s, i);
    let c = *s.get(i)?;
    match c {
        b'"' => {
            i += 1;
            while i < s.len() {
                match s[i] {
                    b'\\' => i += 2,
                    b'"' => return Some(i + 1),
                    _ => i += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let close = if c == b'{' { b'}' } else { b']' };
            let mut depth = 0usize;
            while i < s.len() {
                let ch = s[i];
                if ch == b'"' {
                    i = skip_value(s, i)?;
                    continue;
                }
                if ch == c {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                i += 1;
            }
            None
        }
        _ => {
            while i < s.len() && !matches!(s[i], b',' | b']' | b'}') && !s[i].is_ascii_whitespace() {
                i += 1;
            }
            Some(i)
        }
    }
}

#[derive(Clone, Copy)]
struct Span {
    key_start: usize,
    value_start: usize,
    value_end: usize,
}

fn find_key_span(s: &[u8], obj_start: usize, key: &str) -> Option<Span> {
    let mut i = obj_start + 1;
    while i < s.len() {
        while i < s.len() && (s[i].is_ascii_whitespace() || s[i] == b',') {
            i += 1;
        }
        if i >= s.len() || s[i] != b'"' {
            return None;
        }
        let k_end = skip_value(s, i)?;
        let k: String = serde_json::from_slice(&s[i..k_end]).ok()?;
        let mut j = skip_ws(s, k_end);
        if s.get(j) != Some(&b':') {
            return None;
        }
        j = skip_ws(s, j + 1);
        let v_end = skip_value(s, j)?;
        if k == key {
            return Some(Span { key_start: i, value_start: j, value_end: v_end });
        }
        i = v_end;
    }
    None
}

fn resolve_value_span(s: &[u8], path: &[&str]) -> Option<Span> {
    let mut obj_start = s.iter().position(|b| *b == b'{')?;
    let mut span = None;
    for (n, p) in path.iter().enumerate() {
        let found = find_key_span(s, obj_start, p)?;
        obj_start = found.value_start;
        if n + 1 < path.len() && s.get(obj_start) != Some(&b'{') {
            return None;
        }
        span = Some(found);
    }
    span
}

fn render_for_config(text: &str, span: Span, value: &Value) -> String {
    let line_start = text[..span.key_start].rfind('\n').map_or(0, |i| i + 1);
    let indent = &text[line_start..span.key_start];
    if !indent.chars().all(|c| c == ' ' || c == '\t') || line_start == 0 {
        return value.to_string();
    }
    let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
    pretty
        .split('\n')
        .enumerate()
        .map(|(i, l)| if i == 0 { l.to_string() } else { format!("{indent}{l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What became of the config mirror.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigState {
    Skipped,
    Missing,
    MissingKey,
    Unchanged,
    WouldUpdate,
    Updated,
    Error,
}

/// Put `value` into the mirror key of `text` and say what the new text is, leaving every other
/// byte alone. `Ok(None)` when it already holds the value.
fn splice(text: &str, value: &Value) -> std::result::Result<Option<String>, ConfigState> {
    if serde_json::from_str::<Value>(text).is_err() {
        return Err(ConfigState::Skipped);
    }
    let Some(span) = resolve_value_span(text.as_bytes(), &CONFIG_PATH) else { return Err(ConfigState::MissingKey) };
    let before = &text[span.value_start..span.value_end];
    let rendered = render_for_config(text, span, value);
    if before == rendered {
        return Ok(None);
    }
    if serde_json::from_str::<Value>(before).is_ok_and(|current| &current == value) {
        return Ok(None);
    }
    let out = format!("{}{}{}", &text[..span.value_start], rendered, &text[span.value_end..]);
    if serde_json::from_str::<Value>(&out).is_err() {
        return Err(ConfigState::Error);
    }
    Ok(Some(out))
}

fn splice_config(
    path: &Path,
    backups: &Path,
    value: &Value,
    dry_run: bool,
    warnings: &mut Vec<String>,
) -> Result<ConfigState> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ConfigState::Missing),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let bom = raw.starts_with(b"\xEF\xBB\xBF");
    let text = String::from_utf8_lossy(&raw[if bom { 3 } else { 0 }..]).into_owned();
    let out = match splice(&text, value) {
        Ok(Some(out)) => out,
        Ok(None) => return Ok(ConfigState::Unchanged),
        Err(state) => {
            match state {
                ConfigState::Skipped => warnings.push("config is not valid JSON; mirror skipped".into()),
                ConfigState::Error => warnings.push("config splice produced invalid JSON; write aborted".into()),
                _ => {}
            }
            return Ok(state);
        }
    };
    if dry_run {
        return Ok(ConfigState::WouldUpdate);
    }
    fs::create_dir_all(backups)?;
    fs::copy(path, backups.join(format!("config-backup-{}.json", stamp())))?;
    rotate(backups, "config-backup-", CONFIG_BACKUPS_KEPT);
    let mut bytes = if bom { b"\xEF\xBB\xBF".to_vec() } else { Vec::new() };
    bytes.extend(out.as_bytes());
    // Lost-update guard: Desktop rewrites this file on its own schedule.
    if fs::read(path)? != raw {
        warnings.push("config changed under us; mirror retried next run".into());
        return Ok(ConfigState::Skipped);
    }
    fsx::atomic_write(path, &bytes, None, false)?;
    Ok(ConfigState::Updated)
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Delete all but the newest `keep` entries of `dir` whose names start with `prefix`.
fn rotate(dir: &Path, prefix: &str, keep: usize) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && !n.ends_with(".tmp"))
        .collect();
    names.sort();
    names.reverse();
    for n in names.into_iter().skip(keep) {
        let p = dir.join(n);
        let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
    }
}

// ── Local Storage ────────────────────────────────────────────────────────────

/// One origin's entries, as found.
struct Origin {
    prefix: Vec<u8>,
    /// Decoded name → (full key, value).
    entries: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    size_bytes: u64,
    meta_key: Vec<u8>,
    meta: Option<Meta>,
}

impl Origin {
    fn text(&self, name: &str) -> Option<String> {
        self.entries.get(name).and_then(|(_, v)| decode_string(v).ok())
    }
}

fn options() -> Options {
    Options { create_if_missing: false, ..Options::default() }
}

fn read_origin(db: &mut DB, origin: &str) -> Result<Origin> {
    let prefix = map_key_prefix(origin);
    let meta_key = format!("META:{origin}").into_bytes();
    let mut entries = BTreeMap::new();
    let mut size_bytes = 0u64;
    let mut meta = None;
    let mut it = db.new_iter().map_err(|e| anyhow!("cannot read Local Storage: {e}"))?;
    while let Some((k, v)) = LdbIterator::next(&mut it) {
        if k == meta_key {
            meta = Some(decode_meta(&v));
        } else if k.starts_with(&prefix) {
            size_bytes += (k.len() - prefix.len() + v.len()) as u64;
            if let Ok(name) = decode_string(&k[prefix.len()..]) {
                entries.insert(name, (k, v));
            }
        }
    }
    Ok(Origin { prefix, entries, size_bytes, meta_key, meta })
}

fn open_db(dir: &Path) -> Result<DB> {
    DB::open(dir, options()).map_err(|e| anyhow!("cannot open Local Storage at {}: {e}", dir.display()))
}

/// Copy a database directory without its lock file.
fn copy_db_dir(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for e in fs::read_dir(src)?.flatten() {
        if e.file_name() == "LOCK" || !e.path().is_file() {
            continue;
        }
        fs::copy(e.path(), dest.join(e.file_name()))?;
    }
    Ok(())
}

/// Put a backup copy back over a database directory.
fn restore_db_dir(backup: &Path, dir: &Path) -> Result<()> {
    for e in fs::read_dir(dir)?.flatten() {
        if e.file_name() != "LOCK" && e.path().is_file() {
            fs::remove_file(e.path())?;
        }
    }
    copy_db_dir(backup, dir)
}

fn json_of(origin: &Origin, name: &str) -> Option<Value> {
    serde_json::from_str(&origin.text(name)?).ok()
}

// ── The run ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Nothing to change.
    Unchanged,
    Updated,
    WouldUpdate,
    /// Claude is running; try again when it is closed.
    Deferred,
    /// There is nothing to work on (no Local Storage, fewer than two scopes, switched off).
    Off,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Publish {
    pub owner: Option<String>,
    /// `none`, `set`, `current`, `kept` or `foreign`.
    pub marker: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub status: Status,
    pub reason: Option<String>,
    pub config_state: ConfigState,
    pub warnings: Vec<String>,
    pub changes: Option<Changes>,
    pub scopes_rewritten: Vec<String>,
    pub publish: Option<Publish>,
    pub backup_dir: Option<PathBuf>,
}

impl Report {
    fn new(status: Status, reason: impl Into<String>) -> Report {
        Report {
            status,
            reason: Some(reason.into()),
            config_state: ConfigState::Skipped,
            warnings: Vec::new(),
            changes: None,
            scopes_rewritten: Vec::new(),
            publish: None,
            backup_dir: None,
        }
    }

    /// One line for the log and the command line.
    pub fn summary(&self) -> String {
        match self.status {
            Status::Off | Status::Deferred => self.reason.clone().unwrap_or_default(),
            Status::Unchanged if self.config_state == ConfigState::Updated => {
                "groups already equal; the config file mirror was refreshed".into()
            }
            Status::Unchanged => self.reason.clone().unwrap_or_else(|| "groups already equal".into()),
            Status::Updated | Status::WouldUpdate => {
                let c = self.changes.clone().unwrap_or_default();
                format!(
                    "{} {} scope(s): +{} -{} ~{} group(s), {} withheld, {} assignment(s){}",
                    if self.status == Status::Updated { "merged into" } else { "would merge into" },
                    self.scopes_rewritten.len(),
                    c.groups_added.len(),
                    c.groups_removed.len(),
                    c.groups_changed.len(),
                    c.groups_withheld.len(),
                    c.assignments_changed,
                    self.publish
                        .as_ref()
                        .and_then(|p| p.owner.as_ref().map(|o| format!("; marker {} for {o}", p.marker)))
                        .unwrap_or_default(),
                )
            }
        }
    }
}

/// Where a run reads and writes.
#[derive(Clone, Debug)]
pub struct Target {
    /// `…/Local Storage/leveldb`.
    pub leveldb: PathBuf,
    /// Desktop's `claude_desktop_config.json`, mirrored when present.
    pub config: Option<PathBuf>,
    /// Our own folder for the base, backups and config copies.
    pub state: PathBuf,
}

impl Target {
    pub fn of(ctx: &Ctx) -> Target {
        Target {
            leveldb: ctx.paths.user_data.join("Local Storage").join("leveldb"),
            config: Some(ctx.paths.user_data.join("claude_desktop_config.json")),
            state: ctx.paths.state_dir.join("groups"),
        }
    }
}

fn account(scope: &str) -> &str {
    scope.split('/').next().unwrap_or(scope)
}

/// Merge the sidebar groups of `scopes` (`account/org` keys). Claude must be closed; the caller
/// decides that (see [`sync`]).
pub fn run(target: &Target, scopes: &[String], dry_run: bool) -> Result<Report> {
    if scopes.len() < 2 {
        return Ok(Report::new(Status::Off, "fewer than two accounts take part, so there is nothing to equalize"));
    }
    if !target.leveldb.is_dir() {
        return Ok(Report::new(Status::Off, format!("Local Storage not found at {}", target.leveldb.display())));
    }
    fs::create_dir_all(&target.state)?;
    let mut warnings = Vec::new();
    let base_path = target.state.join("groups-base.json");
    let base: Option<Base> = match fs::read(&base_path) {
        Ok(raw) => match serde_json::from_slice::<Base>(&raw) {
            Ok(b) if !b.scopes.is_empty() && b.entry.is_object() => Some(b),
            Ok(_) => None,
            Err(e) => {
                warnings.push(format!("base unreadable ({e}); merging without deletions"));
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warnings.push(format!("base unreadable ({e}); merging without deletions"));
            None
        }
    };
    let empty = BTreeMap::new();
    let published = base.as_ref().map_or(&empty, |b| &b.published);
    let pending_writes = base.as_ref().map_or(&empty, |b| &b.pending);

    // The pristine backup, and a second copy to read from: opening a database changes the files
    // it is opened in, so nothing is opened in place until a write is certain.
    let tag = format!("{}-{}", stamp(), std::process::id());
    let backup_tmp = target.state.join(format!("leveldb-backup-{tag}.tmp"));
    let read_tmp = target.state.join(format!("leveldb-read-{tag}.tmp"));
    let cleanup = |p: &Path| {
        let _ = fs::remove_dir_all(p);
    };
    copy_db_dir(&target.leveldb, &backup_tmp)?;
    copy_db_dir(&target.leveldb, &read_tmp)?;
    let origin = (|| -> Result<Origin> {
        let mut db = open_db(&read_tmp)?;
        let origin = read_origin(&mut db, ORIGIN);
        let _ = db.close();
        origin
    })();
    cleanup(&read_tmp);
    let origin = match origin {
        Ok(o) => o,
        Err(e) => {
            cleanup(&backup_tmp);
            return Err(e);
        }
    };

    let analysis = match analyse(scopes, dry_run, &origin, base.as_ref(), published, pending_writes) {
        Ok(a) => a,
        Err(e) => {
            cleanup(&backup_tmp);
            return Err(e);
        }
    };
    let Analysis { mut report, plan } = analysis;
    warnings.append(&mut report.warnings);
    report.warnings = warnings;
    let Some(plan) = plan else {
        cleanup(&backup_tmp);
        return Ok(report);
    };

    let mut kept_backup = false;
    if !dry_run {
        let written = (|| -> Result<()> {
            if plan.db_write {
                write_db(&target.leveldb, &plan.ops, &backup_tmp)?;
                report.status = Status::Updated;
                kept_backup = true;
            }
            fsx::atomic_write(&base_path, &serde_json::to_vec_pretty(&plan.base_out)?, None, false)?;
            Ok(())
        })();
        if let Err(e) = written {
            cleanup(&backup_tmp);
            return Err(e);
        }
    }
    if kept_backup {
        let name = format!("leveldb-backup-{}", stamp());
        let mut dest = target.state.join(&name);
        for i in 2.. {
            if !dest.exists() {
                break;
            }
            dest = target.state.join(format!("{name}-{i}"));
        }
        match fs::rename(&backup_tmp, &dest) {
            Ok(()) => {
                rotate(&target.state, "leveldb-backup-", LEVELDB_BACKUPS_KEPT);
                report.backup_dir = Some(dest);
            }
            Err(e) => report.warnings.push(format!("could not keep the backup: {e}")),
        }
    } else {
        cleanup(&backup_tmp);
    }
    // The config mirror follows what Local Storage now holds.
    if let Some(config) = &target.config {
        match splice_config(config, &target.state, &plan.mirror, dry_run, &mut report.warnings) {
            Ok(state) => report.config_state = state,
            Err(e) => {
                report.warnings.push(format!("config mirror: {e}"));
                report.config_state = ConfigState::Error;
            }
        }
    }
    Ok(report)
}

/// What a run decided to write.
struct Plan {
    /// Puts into Local Storage, in order; the last is the `META` record.
    ops: Vec<(Vec<u8>, Vec<u8>)>,
    db_write: bool,
    base_out: Base,
    /// The whole value of the config mirror key.
    mirror: Value,
}

struct Analysis {
    report: Report,
    plan: Option<Plan>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn analyse(
    scopes: &[String],
    dry_run: bool,
    origin: &Origin,
    base: Option<&Base>,
    published: &BTreeMap<String, Stamped>,
    pending_writes: &BTreeMap<String, Stamped>,
) -> Result<Analysis> {
    let nothing = |why: &str| Ok(Analysis { report: Report::new(Status::Unchanged, why), plan: None });
    let mut warnings = Vec::new();
    let (scopes_rec, store_rec) = (origin.entries.get(SCOPES_KEY), origin.entries.get(STORE_KEY));
    if scopes_rec.is_none() && store_rec.is_none() {
        return nothing("no group keys in Local Storage yet");
    }
    let parse = |name: &str, warnings: &mut Vec<String>| {
        let v = json_of(origin, name);
        if v.is_none() && origin.entries.contains_key(name) {
            warnings.push(format!("{name} is not JSON; ignored"));
        }
        v
    };
    let wrapper = parse(SCOPES_KEY, &mut warnings);
    let store = parse(STORE_KEY, &mut warnings);
    let from_lss: Option<&Map<String, Value>> =
        wrapper.as_ref().and_then(|w| w.get("value")).and_then(Value::as_object);
    let from_store: Option<&Map<String, Value>> =
        store.as_ref().and_then(|s| s.pointer("/state/customGroupsByScope")).and_then(Value::as_object);
    if from_lss.is_none() && from_store.is_none() {
        return nothing("group keys present but hold no scopes");
    }

    // The store is what the sidebar hydrates from; the LSS wrapper is the app's projection of it
    // and is only reported when it holds more.
    let mut present: BTreeMap<String, Value> = BTreeMap::new();
    for s in scopes {
        let (a, b) = (from_store.and_then(|m| m.get(s)), from_lss.and_then(|m| m.get(s)));
        if let (Some(a), Some(b)) = (a, b) {
            if lss_exceeds_store(b, a) {
                warnings.push(format!("scope {s}: LSS mirror holds groups the store lacks; store wins"));
            }
        }
        if let Some(chosen) = a.or(b) {
            present.insert(s.clone(), chosen.clone());
        }
    }
    let last_scope = store.as_ref().and_then(|s| s.pointer("/state/lastSidebarScopeKey")).and_then(Value::as_str);
    let prefer: Vec<String> = last_scope.map(str::to_string).into_iter().collect();

    let owner = origin.text(SYNC_OWNER_KEY);
    let marker = origin.text(SYNC_PENDING_KEY);
    let mut switched_at = 0.0f64;
    for (name, (_, v)) in &origin.entries {
        if !name.ends_with(LOGOUT_SUFFIX) && name != SESSION_MARKER_KEY {
            continue;
        }
        if let Some(at) = decode_string(v).ok().and_then(|t| t.trim().parse::<f64>().ok()).filter(|n| n.is_finite()) {
            switched_at = switched_at.max(at);
        }
    }
    // The scope the app will reconcile at its next start: the owner account, in the org it showed
    // last when that account has several.
    let owner_scope: Option<String> = owner.as_ref().and_then(|o| {
        scopes
            .iter()
            .find(|s| Some(s.as_str()) == last_scope && account(s) == o)
            .or_else(|| scopes.iter().find(|s| account(s) == o))
            .cloned()
    });
    // A scope's entry is reconciled when the app has run on its account since the last write to
    // it: the owner scope with no marker outstanding. Without server sync (no owner key) every
    // present scope is what the app shows.
    let reconciled_now = |s: &String| {
        present.contains_key(s) && (owner.is_none() || (owner_scope.as_ref() == Some(s) && marker.is_none()))
    };
    let mut reconciled: BTreeMap<String, Value> = BTreeMap::new();
    for s in scopes.iter().filter(|s| reconciled_now(s)) {
        // A write of ours counts as reconciled once the app has pushed it: the marker is gone and
        // no account switch (which discards it unpushed) has happened since. Without a dated
        // switch the push is not assumed.
        let pushed = pending_writes.get(s).filter(|p| {
            !p.entry.is_null()
                && switched_at > 0.0
                && chrono::DateTime::parse_from_rfc3339(&p.at).is_ok_and(|t| switched_at < t.timestamp_millis() as f64)
        });
        if let Some(p) = pushed {
            reconciled.insert(s.clone(), p.entry.clone());
        } else if let Some(p) = published.get(s).filter(|p| !p.entry.is_null()) {
            reconciled.insert(s.clone(), p.entry.clone());
        }
    }
    let (entry, changes) = merge_scopes(scopes, &present, base, &prefer, &reconciled);
    let rewritten: Vec<String> = scopes.iter().filter(|s| present.get(*s) != Some(&entry)).cloned().collect();

    let now = iso_now();
    let (mut published_out, mut pending_out) = (BTreeMap::new(), BTreeMap::new());
    for s in scopes {
        if reconciled_now(s) {
            published_out.insert(s.clone(), Stamped { at: now.clone(), entry: present[s].clone() });
        } else {
            if let Some(p) = published.get(s) {
                published_out.insert(s.clone(), p.clone());
            }
            if let Some(p) = pending_writes.get(s) {
                pending_out.insert(s.clone(), p.clone());
            }
        }
    }
    let mut tombstones = base.map(|b| b.tombstones.clone()).unwrap_or_default();
    for id in &changes.groups_removed {
        tombstones.insert(id.clone(), now.clone());
    }
    // A tombstone stays until every account has been seen reconciled after the deletion, so the
    // id can no longer come back from a server row.
    tombstones.retain(|id, at| {
        let held = scopes
            .iter()
            .any(|s| present.get(s).is_some_and(|e| norm(Some(e)).groups.iter().any(|g| group_id(g) == id)));
        held || !scopes.iter().all(|s| published_out.get(s).is_some_and(|p| p.at > *at))
    });
    // The owner scope reaches its server row only through the app's push, so the marker is set
    // whenever what it holds after this run is not what the server was last seen to hold. A marker
    // the app wrote for another identity (a seed migration) is not touched.
    let owner_needs_push = owner_scope.as_ref().is_some_and(|o| published_out.get(o).is_none_or(|p| p.entry != entry));
    let marker_write = owner_needs_push && marker.is_none();
    let marker_state = match (&owner_scope, &marker) {
        (None, _) => "none",
        (Some(_), None) if marker_write => "set",
        (Some(_), None) => "current",
        (Some(o), Some(m)) if m == o => "kept",
        _ => "foreign",
    };
    if let (true, Some(m), Some(o)) = (owner_needs_push, &marker, &owner_scope) {
        if m != o {
            warnings.push(format!("pending marker names {m}; the app will not push {o} until it clears"));
        }
    }
    if let (true, Some(o)) = (owner_needs_push, &owner_scope) {
        pending_out.insert(o.clone(), Stamped { at: now.clone(), entry: entry.clone() });
    }
    let db_write = !rewritten.is_empty() || marker_write;

    // What goes into Local Storage.
    let mut ops: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut size_delta = 0i64;
    let set_all = |m: &mut Map<String, Value>| {
        for s in scopes {
            m.insert(s.clone(), entry.clone());
        }
    };
    if let (Some(store), Some(rec)) = (&store, store_rec) {
        let mut next = store.clone();
        if let Some(m) = next.pointer_mut("/state/customGroupsByScope").and_then(Value::as_object_mut) {
            set_all(m);
        }
        let value = encode_string(&next.to_string());
        size_delta += value.len() as i64 - rec.1.len() as i64;
        ops.push((rec.0.clone(), value));
    }
    if let (Some(w), Some(rec), true) = (&wrapper, scopes_rec, from_lss.is_some()) {
        let mut next = w.clone();
        if let Some(m) = next.get_mut("value").and_then(Value::as_object_mut) {
            set_all(m);
        }
        next["timestamp"] = json!(now_ms());
        let value = encode_string(&next.to_string());
        size_delta += value.len() as i64 - rec.1.len() as i64;
        ops.push((rec.0.clone(), value));
    } else if from_store.is_some() {
        let mut value = Map::new();
        set_all(&mut value);
        let wrapper = json!({ "value": value, "tabId": "", "timestamp": now_ms() });
        let key = [origin.prefix.clone(), encode_string(SCOPES_KEY)].concat();
        let value = encode_string(&wrapper.to_string());
        size_delta += (key.len() - origin.prefix.len() + value.len()) as i64;
        ops.push((key, value));
    }
    if let (true, Some(o)) = (marker_write, &owner_scope) {
        let key = [origin.prefix.clone(), encode_string(SYNC_PENDING_KEY)].concat();
        let value = encode_string(o);
        size_delta += (key.len() - origin.prefix.len() + value.len()) as i64;
        ops.push((key, value));
    }
    let mut meta = origin.meta.unwrap_or(Meta { last_modified_us: 0, size_bytes: origin.size_bytes });
    meta.size_bytes = (origin.size_bytes as i64 + size_delta).max(0) as u64;
    meta.last_modified_us = now_ms() * 1000 + WINDOWS_EPOCH_OFFSET_US;
    ops.push((origin.meta_key.clone(), encode_meta(meta)));

    let mut mirror = from_store.or(from_lss).cloned().unwrap_or_default();
    for s in scopes {
        mirror.insert(s.clone(), entry.clone());
    }

    let report = Report {
        status: if dry_run && db_write { Status::WouldUpdate } else { Status::Unchanged },
        reason: None,
        config_state: ConfigState::Skipped,
        warnings,
        changes: Some(changes),
        scopes_rewritten: rewritten,
        publish: Some(Publish { owner: owner_scope.clone(), marker: marker_state.into() }),
        backup_dir: None,
    };
    let base_out = Base {
        version: 2,
        scopes: scopes.to_vec(),
        entry: entry.clone(),
        written_at: now,
        published: published_out,
        pending: pending_out,
        tombstones,
    };
    Ok(Analysis { report, plan: Some(Plan { ops, db_write, base_out, mirror: Value::Object(mirror) }) })
}

/// Put `ops` into the database in `dir` and check they read back; on any failure put the backup
/// back so the app finds the database as it left it.
fn write_db(dir: &Path, ops: &[(Vec<u8>, Vec<u8>)], backup: &Path) -> Result<()> {
    let attempt = || -> Result<()> {
        let mut db = open_db(dir)?;
        let mut batch = WriteBatch::default();
        for (k, v) in ops {
            batch.put(k, v);
        }
        db.write(batch, true).map_err(|e| anyhow!("cannot write Local Storage: {e}"))?;
        db.flush().map_err(|e| anyhow!("cannot flush Local Storage: {e}"))?;
        db.close().map_err(|e| anyhow!("cannot close Local Storage: {e}"))?;
        drop(db);
        let mut db = open_db(dir)?;
        for (k, v) in ops {
            if db.get(k).as_deref() != Some(v.as_slice()) {
                bail!("what was written did not read back");
            }
        }
        let _ = db.close();
        Ok(())
    };
    match attempt() {
        Ok(()) => Ok(()),
        Err(e) => match restore_db_dir(backup, dir) {
            Ok(()) => Err(e.context("Local Storage was put back as it was")),
            Err(r) => {
                Err(e
                    .context(format!("and the backup could not be restored ({r}); it is kept at {}", backup.display())))
            }
        },
    }
}

/// Every list that takes part in sync, as `account/org` scope keys.
pub fn scopes_of(ctx: &Ctx) -> Vec<String> {
    let cfg = ctx.config();
    scan::discover(ctx, Surface::Code).iter().filter(|p| !cfg.is_excluded(&p.acct, &p.org)).map(|p| p.key()).collect()
}

/// Cheap change detector for the background agent: Local Storage's files, the config mirror and
/// the lists taking part.
pub fn fingerprint(ctx: &Ctx) -> u64 {
    use std::hash::{Hash, Hasher};
    let target = Target::of(ctx);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(rd) = fs::read_dir(&target.leveldb) {
        let mut files: Vec<(String, i128, u64)> = rd
            .flatten()
            .filter(|e| e.file_name() != "LOCK")
            .filter_map(|e| {
                let md = e.metadata().ok()?;
                Some((e.file_name().to_string_lossy().into_owned(), fsx::mtime_ns(&md), md.len()))
            })
            .collect();
        files.sort();
        files.hash(&mut h);
    }
    if let Some(md) = target.config.as_ref().and_then(|c| fs::metadata(c).ok()) {
        (fsx::mtime_ns(&md), md.len()).hash(&mut h);
    }
    scopes_of(ctx).hash(&mut h);
    h.finish()
}

/// Run if `ctx` allows: Claude closed, and the lock ours.
pub fn sync(ctx: &Ctx, dry_run: bool) -> Result<Report> {
    let target = Target::of(ctx);
    if desktop::detect(ctx).running {
        return Ok(Report::new(Status::Deferred, "Claude is running; groups are merged when it is closed"));
    }
    let scopes = scopes_of(ctx);
    let _lock = crate::apply::Lock::acquire(ctx, std::time::Duration::from_secs(30))?;
    let report = run(&target, &scopes, dry_run)?;
    for w in &report.warnings {
        ctx.log(format!("warning: groups: {w}"));
    }
    if matches!(report.status, Status::Updated) || report.config_state == ConfigState::Updated {
        ctx.log(format!("groups: {}", report.summary()));
    }
    Ok(report)
}

/// For the background agent: [`sync`] when the setting is on, never fatal.
pub fn sync_if_enabled(ctx: &Ctx) -> Option<Report> {
    if !ctx.config().sync_groups {
        return None;
    }
    match sync(ctx, false) {
        Ok(r) => Some(r),
        Err(e) => {
            ctx.log(format!("warning: group sync failed: {e:#}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, FakeDesktop, LogSink, Paths};

    fn entry(groups: &[(&str, &str)], assign: &[(&str, &str)]) -> Value {
        let groups: Vec<Value> = groups.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
        let assign: Map<String, Value> = assign.iter().map(|(k, g)| (k.to_string(), json!(g))).collect();
        json!({ "groups": groups, "assignments": assign, "order": {} })
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn ids(entry: &Value) -> Vec<String> {
        norm(Some(entry)).groups.iter().map(|g| group_id(g).to_string()).collect()
    }

    // ── encoding ────────────────────────────────────────────────────────────

    #[test]
    fn strings_round_trip_in_both_chromium_forms() {
        for s in ["plain", "caf\u{e9}", "Gruppe \u{2013} \u{4e2d}\u{6587} \u{1f600}", ""] {
            assert_eq!(decode_string(&encode_string(s)).unwrap(), s);
        }
        assert_eq!(encode_string("abc")[0], 0x01, "ASCII and Latin-1 use the one-byte form");
        assert_eq!(encode_string("\u{4e2d}")[0], 0x00, "anything wider uses UTF-16");
        assert!(decode_string(&[0x07, 1]).is_err());
    }

    #[test]
    fn meta_round_trips_and_survives_garbage() {
        let meta = Meta { last_modified_us: 13_400_000_000_000_000, size_bytes: 123_456 };
        assert_eq!(decode_meta(&encode_meta(meta)), meta);
        assert_eq!(decode_meta(&[0x08, 0xff]), Meta::default());
    }

    // ── the merge ───────────────────────────────────────────────────────────

    fn present(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn base_of(entry: Value, tombstones: &[&str]) -> Base {
        Base {
            version: 2,
            scopes: names(&["a/o", "b/o"]),
            entry,
            tombstones: tombstones.iter().map(|id| (id.to_string(), "2026-01-01T00:00:00.000Z".into())).collect(),
            ..Base::default()
        }
    }

    #[test]
    fn a_group_one_account_has_reaches_the_other() {
        let scopes = names(&["a/o", "b/o"]);
        let p = present(&[("a/o", entry(&[("g1", "Work")], &[("c1", "g1")])), ("b/o", entry(&[], &[]))]);
        let (e, ch) = merge_scopes(&scopes, &p, None, &[], &BTreeMap::new());
        assert_eq!(ids(&e), ["g1"]);
        assert_eq!(e["assignments"]["c1"], "g1");
        assert_eq!(ch.groups_added, ["g1"]);
    }

    #[test]
    fn without_a_base_nothing_is_ever_deleted() {
        let scopes = names(&["a/o", "b/o"]);
        let p = present(&[("a/o", entry(&[("g1", "Work")], &[])), ("b/o", entry(&[("g2", "Home")], &[]))]);
        let reconciled = present(&[("a/o", entry(&[("g1", "Work"), ("g2", "Home")], &[]))]);
        let (e, ch) = merge_scopes(&scopes, &p, None, &[], &reconciled);
        assert_eq!(ids(&e), ["g1", "g2"]);
        assert!(ch.groups_removed.is_empty());
    }

    #[test]
    fn a_deletion_counts_only_after_the_account_was_seen_holding_the_group() {
        let scopes = names(&["a/o", "b/o"]);
        let base = base_of(entry(&[("g1", "Work")], &[]), &[]);
        let p = present(&[("a/o", entry(&[], &[])), ("b/o", entry(&[("g1", "Work")], &[]))]);
        // a was never seen holding g1 (its server row may simply not have it yet): no deletion.
        let (e, ch) = merge_scopes(&scopes, &p, Some(&base), &[], &BTreeMap::new());
        assert_eq!(ids(&e), ["g1"]);
        assert!(ch.groups_removed.is_empty());
        // a was reconciled while holding g1 and now lacks it: the user deleted it.
        let reconciled = present(&[("a/o", entry(&[("g1", "Work")], &[]))]);
        let (e, ch) = merge_scopes(&scopes, &p, Some(&base), &[], &reconciled);
        assert!(ids(&e).is_empty());
        assert_eq!(ch.groups_removed, ["g1"]);
    }

    #[test]
    fn a_deleted_id_never_comes_back() {
        let scopes = names(&["a/o", "b/o"]);
        let base = base_of(entry(&[], &[]), &["g1"]);
        let p = present(&[("a/o", entry(&[], &[])), ("b/o", entry(&[("g1", "Work")], &[]))]);
        let (e, ch) = merge_scopes(&scopes, &p, Some(&base), &[], &BTreeMap::new());
        assert!(ids(&e).is_empty());
        assert_eq!(ch.groups_withheld, ["g1"]);
    }

    #[test]
    fn a_rename_wins_over_the_base_and_the_chat_follows_its_move() {
        let scopes = names(&["a/o", "b/o"]);
        let base = base_of(entry(&[("g1", "Work"), ("g2", "Home")], &[("c1", "g1")]), &[]);
        let p = present(&[
            ("a/o", entry(&[("g1", "Work"), ("g2", "Home")], &[("c1", "g1")])),
            ("b/o", entry(&[("g1", "Job"), ("g2", "Home")], &[("c1", "g2")])),
        ]);
        let (e, ch) = merge_scopes(&scopes, &p, Some(&base), &[], &BTreeMap::new());
        assert_eq!(e["groups"][0]["name"], "Job");
        assert_eq!(e["assignments"]["c1"], "g2");
        assert_eq!(ch.groups_changed, ["g1"]);
        assert_eq!(ch.assignments_changed, 1);
    }

    #[test]
    fn the_preferred_scope_decides_the_group_order() {
        let scopes = names(&["a/o", "b/o"]);
        let base = base_of(entry(&[("g1", "x"), ("g2", "y")], &[]), &[]);
        let p = present(&[
            ("a/o", entry(&[("g1", "x"), ("g2", "y")], &[])),
            ("b/o", entry(&[("g2", "y"), ("g1", "x")], &[])),
        ]);
        let (e, _) = merge_scopes(&scopes, &p, Some(&base), &[], &BTreeMap::new());
        assert_eq!(ids(&e), ["g2", "g1"], "the one scope that departs from the base is followed");
    }

    #[test]
    fn fields_the_merge_does_not_know_are_kept() {
        let scopes = names(&["a/o", "b/o"]);
        let mut a = entry(&[("g1", "x")], &[]);
        a["collapsed"] = json!(["g1"]);
        let p = present(&[("a/o", a), ("b/o", entry(&[], &[]))]);
        let (e, _) = merge_scopes(&scopes, &p, None, &[], &BTreeMap::new());
        assert_eq!(e["collapsed"], json!(["g1"]));
    }

    // ── the config mirror ───────────────────────────────────────────────────

    const CONFIG: &str = "{\n  \"decoy\": { \"dframe-group-scopes\": 1 },\n  \"preferences\": {\n    \"epitaxyPrefs\": {\n      \"dframe-group-scopes\": { \"old\": 1 },\n      \"other\": true\n    }\n  },\n  \"tail\": [1, 2]\n}\n";

    #[test]
    fn the_splice_changes_only_the_mirror_value() {
        let value = json!({ "a/o": { "groups": [] } });
        let out = splice(CONFIG, &value).unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.pointer("/preferences/epitaxyPrefs/dframe-group-scopes").unwrap(), &value);
        assert_eq!(parsed["decoy"]["dframe-group-scopes"], 1, "a look-alike elsewhere is left alone");
        assert_eq!(parsed["tail"], json!([1, 2]));
        assert!(out.starts_with("{\n  \"decoy\": { \"dframe-group-scopes\": 1 },"));
        assert!(out.ends_with("\"other\": true\n    }\n  },\n  \"tail\": [1, 2]\n}\n"));
        assert!(splice(&out, &value).unwrap().is_none(), "a second splice finds nothing to do");
    }

    #[test]
    fn the_splice_refuses_what_it_cannot_place() {
        let v = json!({});
        assert_eq!(splice("{ not json", &v), Err(ConfigState::Skipped));
        assert_eq!(splice("{\"preferences\": {}}", &v), Err(ConfigState::MissingKey));
        assert_eq!(splice("{\"preferences\": 3}", &v), Err(ConfigState::MissingKey));
    }

    #[test]
    fn a_byte_order_mark_and_a_missing_file_are_handled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let mut warnings = Vec::new();
        let value = json!({ "a/o": 1 });
        assert_eq!(splice_config(&path, dir.path(), &value, false, &mut warnings).unwrap(), ConfigState::Missing);
        let mut raw = b"\xEF\xBB\xBF".to_vec();
        raw.extend(CONFIG.as_bytes());
        fs::write(&path, &raw).unwrap();
        assert_eq!(splice_config(&path, dir.path(), &value, true, &mut warnings).unwrap(), ConfigState::WouldUpdate);
        assert_eq!(fs::read(&path).unwrap(), raw, "a dry run writes nothing");
        assert_eq!(splice_config(&path, dir.path(), &value, false, &mut warnings).unwrap(), ConfigState::Updated);
        let after = fs::read(&path).unwrap();
        assert!(after.starts_with(b"\xEF\xBB\xBF"), "the byte order mark stays");
        assert!(fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("config-backup-")));
    }

    // ── against a real LevelDB, in a temp dir ───────────────────────────────

    struct Fixture {
        _root: tempfile::TempDir,
        target: Target,
    }

    impl Fixture {
        fn new(entries: &[(&str, Value)], extra: &[(&str, &str)], last: &str) -> Fixture {
            let root = tempfile::tempdir().unwrap();
            let target = Target {
                leveldb: root.path().join("Local Storage").join("leveldb"),
                config: Some(root.path().join("claude_desktop_config.json")),
                state: root.path().join("state"),
            };
            fs::create_dir_all(&target.leveldb).unwrap();
            fs::write(target.config.as_ref().unwrap(), CONFIG).unwrap();
            let f = Fixture { _root: root, target };
            f.write(entries, extra, last);
            f
        }

        /// Replace what the fixture database holds under the origin.
        fn write(&self, entries: &[(&str, Value)], extra: &[(&str, &str)], last: &str) {
            let by_scope: Map<String, Value> = entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            let store =
                json!({ "state": { "customGroupsByScope": by_scope, "lastSidebarScopeKey": last }, "version": 0 });
            let lss = json!({ "value": by_scope, "tabId": "t", "timestamp": 1 });
            let prefix = map_key_prefix(ORIGIN);
            let mut db = DB::open(&self.target.leveldb, Options::default()).unwrap();
            let mut size = 0u64;
            let mut put = |name: &str, value: &str| {
                let (k, v) = ([prefix.clone(), encode_string(name)].concat(), encode_string(value));
                size += (k.len() - prefix.len() + v.len()) as u64;
                db.put(&k, &v).unwrap();
            };
            put(STORE_KEY, &store.to_string());
            put(SCOPES_KEY, &lss.to_string());
            for (k, v) in extra {
                put(k, v);
            }
            let meta = encode_meta(Meta { last_modified_us: 1, size_bytes: size });
            db.put(format!("META:{ORIGIN}").as_bytes(), &meta).unwrap();
            db.close().unwrap();
        }

        fn read(&self) -> Origin {
            let copy = tempfile::tempdir().unwrap();
            copy_db_dir(&self.target.leveldb, copy.path()).unwrap();
            let mut db = open_db(copy.path()).unwrap();
            read_origin(&mut db, ORIGIN).unwrap()
        }

        fn store_scope(&self, scope: &str) -> Value {
            let o = self.read();
            json_of(&o, STORE_KEY).unwrap()["state"]["customGroupsByScope"][scope].clone()
        }

        fn lss_scope(&self, scope: &str) -> Value {
            let o = self.read();
            json_of(&o, SCOPES_KEY).unwrap()["value"][scope].clone()
        }

        fn files(&self) -> BTreeMap<String, Vec<u8>> {
            fs::read_dir(&self.target.leveldb)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name() != "LOCK")
                .map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap()))
                .collect()
        }
    }

    #[test]
    fn groups_spread_to_every_account_and_everything_stays_consistent() {
        let scopes = names(&["a/o", "b/o"]);
        let fx =
            Fixture::new(&[("a/o", entry(&[("g1", "Work")], &[("c1", "g1")])), ("b/o", entry(&[], &[]))], &[], "a/o");
        let report = run(&fx.target, &scopes, false).unwrap();
        assert_eq!(report.status, Status::Updated, "{report:?}");
        assert_eq!(report.config_state, ConfigState::Updated);
        // b gets the group; a only gains the member order the merge derives for its chat.
        assert_eq!(report.scopes_rewritten, ["a/o", "b/o"]);
        assert_eq!(fx.store_scope("a/o")["order"]["g1"], json!(["c1"]));
        for scope in ["a/o", "b/o"] {
            assert_eq!(ids(&fx.store_scope(scope)), ["g1"], "store, {scope}");
            assert_eq!(ids(&fx.lss_scope(scope)), ["g1"], "LSS mirror, {scope}");
        }
        let o = fx.read();
        assert_eq!(o.meta.unwrap().size_bytes, o.size_bytes, "the META size matches what the origin holds");
        assert!(report.backup_dir.is_some_and(|d| d.is_dir()), "a backup was kept");
        let config: Value = serde_json::from_slice(&fs::read(fx.target.config.as_ref().unwrap()).unwrap()).unwrap();
        let mirrored = &config["preferences"]["epitaxyPrefs"]["dframe-group-scopes"];
        assert_eq!(ids(&mirrored["b/o"]), ["g1"]);
        assert_eq!(config["decoy"]["dframe-group-scopes"], 1);
        // A second run finds nothing to do and leaves the database alone.
        let again = run(&fx.target, &scopes, false).unwrap();
        assert_eq!(again.status, Status::Unchanged, "{again:?}");
        assert_eq!(again.config_state, ConfigState::Unchanged);
        assert!(again.backup_dir.is_none());
    }

    #[test]
    fn a_dry_run_changes_nothing_at_all() {
        let scopes = names(&["a/o", "b/o"]);
        let fx = Fixture::new(&[("a/o", entry(&[("g1", "Work")], &[])), ("b/o", entry(&[], &[]))], &[], "a/o");
        let (files, config) = (fx.files(), fs::read(fx.target.config.as_ref().unwrap()).unwrap());
        let report = run(&fx.target, &scopes, true).unwrap();
        assert_eq!(report.status, Status::WouldUpdate);
        assert_eq!(report.config_state, ConfigState::WouldUpdate);
        assert_eq!(fx.files(), files, "not one byte of the database moved");
        assert_eq!(fs::read(fx.target.config.as_ref().unwrap()).unwrap(), config);
        assert!(!fx.target.state.join("groups-base.json").exists());
        let leftovers = fs::read_dir(&fx.target.state).unwrap().flatten().count();
        assert_eq!(leftovers, 0, "no temporary copies are left behind");
    }

    #[test]
    fn a_deletion_made_in_one_account_reaches_the_others() {
        let scopes = names(&["a/o", "b/o"]);
        let fx = Fixture::new(&[("a/o", entry(&[("g1", "Work")], &[])), ("b/o", entry(&[], &[]))], &[], "a/o");
        run(&fx.target, &scopes, false).unwrap();
        // The user deletes g1 in a and adds g2 in b while the sync is idle.
        fx.write(&[("a/o", entry(&[], &[])), ("b/o", entry(&[("g1", "Work"), ("g2", "Home")], &[]))], &[], "a/o");
        let report = run(&fx.target, &scopes, false).unwrap();
        let changes = report.changes.unwrap();
        assert_eq!(changes.groups_removed, ["g1"]);
        assert_eq!(changes.groups_added, ["g2"]);
        for scope in ["a/o", "b/o"] {
            assert_eq!(ids(&fx.store_scope(scope)), ["g2"], "{scope}");
        }
        let base: Base = serde_json::from_slice(&fs::read(fx.target.state.join("groups-base.json")).unwrap()).unwrap();
        assert!(base.tombstones.contains_key("g1"), "the deleted id is remembered");
    }

    #[test]
    fn the_account_claude_starts_on_gets_a_pending_marker_so_it_pushes() {
        let scopes = names(&["a/o", "b/o"]);
        let fx = Fixture::new(
            &[("a/o", entry(&[], &[])), ("b/o", entry(&[("g2", "Home")], &[]))],
            &[(SYNC_OWNER_KEY, "a")],
            "a/o",
        );
        let report = run(&fx.target, &scopes, false).unwrap();
        let publish = report.publish.unwrap();
        assert_eq!((publish.owner.as_deref(), publish.marker.as_str()), (Some("a/o"), "set"));
        assert_eq!(fx.read().text(SYNC_PENDING_KEY).as_deref(), Some("a/o"));
        let again = run(&fx.target, &scopes, false).unwrap();
        assert_eq!(again.publish.unwrap().marker, "kept", "an outstanding marker is left as it is");
        assert_eq!(again.status, Status::Unchanged);
    }

    #[test]
    fn a_database_without_group_keys_and_too_few_accounts_are_left_alone() {
        let fx = Fixture::new(&[], &[], "");
        {
            let mut db = DB::open(&fx.target.leveldb, Options::default()).unwrap();
            db.delete(&[map_key_prefix(ORIGIN), encode_string(STORE_KEY)].concat()).unwrap();
            db.delete(&[map_key_prefix(ORIGIN), encode_string(SCOPES_KEY)].concat()).unwrap();
            db.close().unwrap();
        }
        let r = run(&fx.target, &names(&["a/o", "b/o"]), false).unwrap();
        assert_eq!(r.status, Status::Unchanged);
        let r = run(&fx.target, &names(&["a/o"]), false).unwrap();
        assert_eq!(r.status, Status::Off);
    }

    #[test]
    fn a_backup_puts_the_database_back() {
        let fx = Fixture::new(&[("a/o", entry(&[("g1", "x")], &[])), ("b/o", entry(&[], &[]))], &[], "a/o");
        let backup = fx.target.state.join("backup");
        copy_db_dir(&fx.target.leveldb, &backup).unwrap();
        let before = fx.store_scope("a/o");
        fx.write(&[("a/o", entry(&[("zzz", "other")], &[])), ("b/o", entry(&[], &[]))], &[], "a/o");
        assert_ne!(fx.store_scope("a/o"), before);
        restore_db_dir(&backup, &fx.target.leveldb).unwrap();
        assert_eq!(fx.store_scope("a/o"), before);
    }

    #[test]
    fn nothing_happens_while_claude_runs() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path().join("data"), root.path().join("state"));
        let fake = FakeDesktop { running: Some(true), active: None };
        let ctx = Ctx::new(paths, Config::default(), fake, LogSink::Silent);
        let r = sync(&ctx, false).unwrap();
        assert_eq!(r.status, Status::Deferred);
        assert!(!root.path().join("state").join("groups").exists(), "not even a folder was made");
        assert!(sync_if_enabled(&ctx).is_none(), "off unless switched on");
    }
}
