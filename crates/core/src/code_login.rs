// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
//! Switching the account Claude Code (the terminal program) is signed in to.
//!
//! On Windows and Linux Claude Code keeps its sign-in in `<config dir>/.credentials.json` (the
//! config dir is `~/.claude`, or `$CLAUDE_CONFIG_DIR`), and says which account that is in the
//! `oauthAccount` entry of `.claude.json`. A saved sign-in is a copy of those two things, kept in
//! this tool's private state folder; the credentials file is copied byte for byte and never read,
//! parsed or printed. Switching saves the current sign-in first (Claude Code refreshes its tokens
//! from time to time, so the newest copy is the one worth keeping), then puts the other's in place.
//!
//! macOS keeps the sign-in in the Keychain, which cannot be moved without handing the secret
//! to another program on a command line, so switching is not offered there.
//!
//! Close running Claude Code sessions first: a running one holds its sign-in in memory and writes
//! it back when it refreshes.

use crate::apply::Lock;
use crate::{fsx, Ctx};
use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_JSON: u64 = 64 * 1024 * 1024;
const KEEP_BACKUPS: usize = 5;

/// Whether sign-ins can be switched on this system.
pub fn supported() -> bool {
    !cfg!(target_os = "macos")
}

/// A saved sign-in.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    /// A folder name: derived from the account id.
    pub id: String,
    pub email: String,
    /// Unix seconds of the last save.
    pub saved_at: f64,
    /// It is the sign-in Claude Code has now.
    #[serde(default)]
    pub current: bool,
}

fn dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.state_dir.join("code-logins")
}

fn credentials_file(ctx: &Ctx) -> PathBuf {
    ctx.paths.claude_settings.parent().map(|p| p.join(".credentials.json")).unwrap_or_default()
}

fn require_supported() -> Result<()> {
    if !supported() {
        bail!(
            "macOS keeps Claude Code's sign-in in the Keychain, which Claude Companion does not move; \
             switching it is not available there"
        );
    }
    Ok(())
}

fn read_root(ctx: &Ctx) -> Result<Map<String, Value>> {
    let path = &ctx.paths.claude_json;
    if !path.exists() {
        return Ok(Map::new());
    }
    match fsx::read_json(path, MAX_JSON).with_context(|| format!("cannot read {}", path.display()))? {
        Value::Object(m) => Ok(m),
        _ => bail!("{} is not a JSON object; left alone", path.display()),
    }
}

/// The `oauthAccount` Claude Code has now.
fn current_account(ctx: &Ctx) -> Option<Value> {
    read_root(ctx).ok()?.remove("oauthAccount").filter(Value::is_object)
}

fn email_of(account: &Value) -> String {
    account.get("emailAddress").and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A folder name that names the account and nothing else.
fn id_of(account: &Value) -> Option<String> {
    let raw = account
        .get("accountUuid")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| account.get("emailAddress").and_then(Value::as_str))?;
    let id: String =
        raw.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    (!id.is_empty()).then_some(id)
}

fn profile_dir(ctx: &Ctx, id: &str) -> PathBuf {
    dir(ctx).join(id)
}

fn read_profile(ctx: &Ctx, id: &str) -> Option<Profile> {
    let p: Profile = serde_json::from_slice(&std::fs::read(profile_dir(ctx, id).join("meta.json")).ok()?).ok()?;
    (p.id == id && profile_dir(ctx, id).join("credentials.json").is_file()).then_some(p)
}

/// The saved sign-ins, ordered by email, with the one in use marked.
pub fn list(ctx: &Ctx) -> Vec<Profile> {
    let now = current_account(ctx).and_then(|a| id_of(&a));
    let mut out: Vec<Profile> = std::fs::read_dir(dir(ctx))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read_profile(ctx, &e.file_name().to_string_lossy()))
        .map(|mut p| {
            p.current = now.as_deref() == Some(p.id.as_str());
            p
        })
        .collect();
    out.sort_by(|a, b| a.email.to_lowercase().cmp(&b.email.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    out
}

/// The saved sign-in `query` names: its number in the list, its email, or the start of its id.
pub fn find(ctx: &Ctx, query: &str) -> Result<Profile> {
    let all = list(ctx);
    let q = query.trim().to_lowercase();
    if let Ok(n) = q.parse::<usize>() {
        if (1..=all.len()).contains(&n) {
            return Ok(all[n - 1].clone());
        }
    }
    let hits: Vec<&Profile> =
        all.iter().filter(|p| p.email.to_lowercase() == q || (!q.is_empty() && p.id.starts_with(&q))).collect();
    match hits.as_slice() {
        [p] => Ok((*p).clone()),
        [] => bail!("no saved Claude Code sign-in matches {query} (see `claude-companion code login list`)"),
        _ => bail!("{query} fits more than one saved sign-in; give more of it"),
    }
}

/// Keep a copy of `path` in this tool's backup folder, and only the newest few.
fn backup(ctx: &Ctx, path: &Path, label: &str) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let folder = dir(ctx).join("backups");
    fsx::create_private_dir_all(&folder)?;
    let name = format!("{}-{label}", crate::snapshot::stamp());
    let dest = fsx::unique_path(folder.join(name));
    std::fs::copy(path, &dest).with_context(|| format!("backing up {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)?.flatten().map(|e| e.path()).collect();
    files.sort();
    for old in files.iter().take(files.len().saturating_sub(KEEP_BACKUPS * 2)) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

/// Save the sign-in Claude Code has now, replacing the saved copy of the same account.
pub fn save_current(ctx: &Ctx) -> Result<Profile> {
    require_supported()?;
    let _lock = Lock::acquire(ctx, Duration::from_secs(30))?;
    save_locked(ctx)
}

fn save_locked(ctx: &Ctx) -> Result<Profile> {
    let creds = credentials_file(ctx);
    anyhow::ensure!(
        creds.is_file(),
        "Claude Code is not signed in (no {} found); run `claude` and sign in with /login first",
        creds.display()
    );
    let account = current_account(ctx)
        .context("Claude Code's account file does not say which account is signed in; sign in again with /login")?;
    let id = id_of(&account).context("the signed-in account has no id or email")?;
    let bytes = fsx::read_limited(&creds, MAX_JSON)?;
    let folder = profile_dir(ctx, &id);
    fsx::create_private_dir_all(&dir(ctx))?;
    fsx::create_private_dir_all(&folder)?;
    fsx::atomic_write(&folder.join("credentials.json"), &bytes, None, false)?;
    fsx::atomic_write(&folder.join("account.json"), &serde_json::to_vec_pretty(&account)?, None, false)?;
    let profile = Profile { id, email: email_of(&account), saved_at: fsx::now_secs(), current: true };
    fsx::atomic_write(&folder.join("meta.json"), &serde_json::to_vec_pretty(&profile)?, None, false)?;
    Ok(profile)
}

/// Put the saved sign-in `query` names in place. The current one is saved first, so switching back
/// brings it back with its newest tokens. Returns the account now signed in.
pub fn switch_to(ctx: &Ctx, query: &str) -> Result<Profile> {
    require_supported()?;
    let target = find(ctx, query)?;
    let _lock = Lock::acquire(ctx, Duration::from_secs(30))?;
    let creds = credentials_file(ctx);
    let now = current_account(ctx);
    if now.as_ref().and_then(id_of).as_deref() == Some(target.id.as_str()) {
        return Ok(Profile { current: true, ..target });
    }
    // Save what is signed in now. With credentials but no account to name them, keep a backup only.
    if creds.is_file() {
        if now.is_some() {
            save_locked(ctx)?;
        } else {
            backup(ctx, &creds, "credentials-unnamed")?;
        }
    }
    let folder = profile_dir(ctx, &target.id);
    let saved_creds = fsx::read_limited(&folder.join("credentials.json"), MAX_JSON)?;
    let saved_account: Value = fsx::read_json(&folder.join("account.json"), MAX_JSON)?;

    let json_path = ctx.paths.claude_json.clone();
    backup(ctx, &creds, "credentials")?;
    backup(ctx, &json_path, "claude-json")?;
    let before_json = if json_path.is_file() { Some(std::fs::read(&json_path)?) } else { None };
    let before_creds = if creds.is_file() { Some(std::fs::read(&creds)?) } else { None };

    let result = (|| -> Result<()> {
        let mut root = read_root(ctx)?;
        root.insert("oauthAccount".into(), saved_account.clone());
        let mut body = serde_json::to_vec_pretty(&Value::Object(root))?;
        body.push(b'\n');
        if let Some(parent) = creds.parent() {
            std::fs::create_dir_all(parent)?;
        }
        fsx::atomic_write(&creds, &saved_creds, None, false)?;
        fsx::atomic_write(&json_path, &body, None, false)?;
        // Read back: the account file names the target and the credentials are what was saved.
        let ok = current_account(ctx).and_then(|a| id_of(&a)).as_deref() == Some(target.id.as_str())
            && std::fs::read(&creds)? == saved_creds;
        anyhow::ensure!(ok, "the new sign-in did not read back as written");
        Ok(())
    })();
    if let Err(e) = result {
        // Put both files back as they were.
        match &before_creds {
            Some(b) => {
                let _ = fsx::atomic_write(&creds, b, None, false);
            }
            None => {
                let _ = std::fs::remove_file(&creds);
            }
        }
        match &before_json {
            Some(b) => {
                let _ = fsx::atomic_write(&json_path, b, None, false);
            }
            None => {
                let _ = std::fs::remove_file(&json_path);
            }
        }
        return Err(e.context("Claude Code's sign-in was put back as it was"));
    }
    Ok(Profile { current: true, ..target })
}

/// Delete a saved sign-in (the one Claude Code has now stays signed in).
pub fn forget(ctx: &Ctx, query: &str) -> Result<Profile> {
    let target = find(ctx, query)?;
    let _lock = Lock::acquire(ctx, Duration::from_secs(30))?;
    std::fs::remove_dir_all(profile_dir(ctx, &target.id))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Env {
        _tmp: tempfile::TempDir,
        ctx: Ctx,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let state = tmp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let paths = crate::Paths::new(tmp.path().join("data"), state);
        let paths = crate::Paths {
            claude_settings: home.join(".claude").join("settings.json"),
            claude_json: home.join(".claude.json"),
            ..paths
        };
        let ctx = Ctx::new(
            paths,
            crate::Config::default(),
            crate::FakeDesktop { running: Some(false), active: None },
            crate::LogSink::Silent,
        );
        Env { _tmp: tmp, ctx }
    }

    /// Stand in for `/login`: the files Claude Code writes for an account.
    fn sign_in(e: &Env, uuid: &str, email: &str, token: &str) {
        std::fs::write(
            credentials_file(&e.ctx),
            json!({"claudeAiOauth": {"accessToken": token, "refreshToken": format!("r-{token}")}}).to_string(),
        )
        .unwrap();
        let mut root = read_root(&e.ctx).unwrap();
        root.insert("oauthAccount".into(), json!({"accountUuid": uuid, "emailAddress": email}));
        root.insert("numStartups".into(), json!(3));
        std::fs::write(&e.ctx.paths.claude_json, Value::Object(root).to_string()).unwrap();
    }

    fn token(e: &Env) -> String {
        let v: Value = serde_json::from_slice(&std::fs::read(credentials_file(&e.ctx)).unwrap()).unwrap();
        v["claudeAiOauth"]["accessToken"].as_str().unwrap().to_string()
    }

    fn who(e: &Env) -> String {
        email_of(&current_account(&e.ctx).unwrap())
    }

    #[test]
    fn switching_back_and_forth_keeps_each_sign_in_whole_and_the_newest_tokens() {
        if !supported() {
            return;
        }
        let e = env();
        sign_in(&e, "aaaa-1", "ada@example.com", "ada-1");
        let ada = save_current(&e.ctx).unwrap();
        assert_eq!(ada.email, "ada@example.com");
        sign_in(&e, "bbbb-2", "bob@example.com", "bob-1");
        save_current(&e.ctx).unwrap();
        let all = list(&e.ctx);
        assert_eq!(all.iter().map(|p| p.email.as_str()).collect::<Vec<_>>(), ["ada@example.com", "bob@example.com"]);
        assert_eq!((all[0].current, all[1].current), (false, true));

        // Bob's session refreshed its token before the switch: that newer copy is the one kept.
        sign_in(&e, "bbbb-2", "bob@example.com", "bob-2");
        let now = switch_to(&e.ctx, "1").unwrap();
        assert_eq!(now.email, "ada@example.com");
        assert_eq!((who(&e), token(&e)), ("ada@example.com".to_string(), "ada-1".to_string()));
        let back = switch_to(&e.ctx, "bob@example.com").unwrap();
        assert_eq!(back.id, "bbbb-2");
        assert_eq!((who(&e), token(&e)), ("bob@example.com".to_string(), "bob-2".to_string()));

        // Everything else in the account file stays.
        assert_eq!(read_root(&e.ctx).unwrap()["numStartups"], json!(3));
        // Already there: nothing to do.
        assert!(switch_to(&e.ctx, "bbbb").unwrap().current);
        assert!(switch_to(&e.ctx, "nobody").is_err());
    }

    #[test]
    fn saved_sign_ins_are_only_copies_and_forgetting_one_leaves_claude_code_signed_in() {
        if !supported() {
            return;
        }
        let e = env();
        sign_in(&e, "aaaa-1", "ada@example.com", "ada-1");
        let p = save_current(&e.ctx).unwrap();
        let saved = std::fs::read(profile_dir(&e.ctx, &p.id).join("credentials.json")).unwrap();
        assert_eq!(saved, std::fs::read(credentials_file(&e.ctx)).unwrap(), "copied byte for byte");
        forget(&e.ctx, "ada@example.com").unwrap();
        assert!(list(&e.ctx).is_empty());
        assert_eq!(who(&e), "ada@example.com");
        assert_eq!(token(&e), "ada-1");
    }

    #[test]
    fn nothing_signed_in_or_a_broken_account_file_changes_nothing() {
        if !supported() {
            return;
        }
        let e = env();
        assert!(save_current(&e.ctx).is_err(), "no credentials yet");
        sign_in(&e, "aaaa-1", "ada@example.com", "ada-1");
        save_current(&e.ctx).unwrap();
        sign_in(&e, "bbbb-2", "bob@example.com", "bob-1");
        std::fs::write(&e.ctx.paths.claude_json, "{ broken").unwrap();
        let creds_before = std::fs::read(credentials_file(&e.ctx)).unwrap();
        assert!(switch_to(&e.ctx, "ada").is_err());
        assert_eq!(std::fs::read(credentials_file(&e.ctx)).unwrap(), creds_before);
        assert_eq!(std::fs::read_to_string(&e.ctx.paths.claude_json).unwrap(), "{ broken");
    }
}
