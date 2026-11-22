# How Claude Companion works

## Where Claude Desktop keeps your sessions

Claude Desktop (macOS and Windows; unofficial builds on Linux) keeps one index of local Code
sessions per **account and organization**:

```
<Claude data>/claude-code-sessions/<account-id>/<org-id>/
    local_<uuid>.json          one session: title, cwd, model, archive state, …
    deleted_<id>               "this session was deleted at <ms>" (id = session or transcript id)
    archived-sessions.idx      load-order hint: {"v":1,"archived":[…]}
    scheduled-tasks.json       Code scheduled tasks
    backlog/tasks.json         task suggestions
```

`<Claude data>` is `~/Library/Application Support/Claude` on macOS, `%APPDATA%\Claude` on
Windows, and `~/.config/Claude` on Linux. A Store build of Desktop 3 or later keeps it in
`%LOCALAPPDATA%\Claude-Data\<package family>` (it moves it there from
`%LOCALAPPDATA%\Packages\<package family>\LocalCache\Roaming\Claude`, where older versions kept
it); Claude Companion uses whichever of these Desktop wrote to last.

An account gets a folder for every organization it is used in. Signing in to another account can
leave an empty one behind as well: Desktop opens the new account with the organization it
remembered from the previous one (its `lastActiveOrg`), and moves to the right organization a moment
later. Claude Companion keeps every folder the same, and the app shows one row per account.

The conversations themselves are Claude Code transcripts in `~/.claude/projects/<project>/<id>.jsonl`,
shared by every account. A session record points at its transcript through `cliSessionId`.

Desktop reads the index of the account it signs in to, **only when that account loads**. So after
an account switch the sidebar is empty although every transcript is still on disk. It has been
reported since April 2026 ([anthropics/claude-code#48511](https://github.com/anthropics/claude-code/issues/48511),
closed as not planned; [#74662](https://github.com/anthropics/claude-code/issues/74662)), and it still
reproduces on Desktop 2.9939.2.

## Why not symlink the folders together

"Merge the folders, then symlink every account's folder to one of them" looks like the simplest fix,
and several tools do it. On current Desktop it breaks saving:

- Before writing a session, Desktop makes sure its storage folder exists with a hardened
  `mkdir`, which ends with `open(dir, O_RDONLY|O_DIRECTORY|O_NOFOLLOW)`.
- If the folder is a symlink, that `open` fails (`ENOTDIR` on macOS).
- The save routine catches the error, writes `Failed to save session …` to its log, and carries on.
  The session stays in memory and is gone after a restart.

Desktop also refuses (or warns about) files with more than one hard link, so hard links are out too.
Every account's index therefore has to stay a real folder with real files, and keeping them
identical means copying. `claude-companion fix-symlinks` turns symlinked folders left by other tools
back into real ones.

## One writer at a time

Copying between folders that Desktop also writes would race it, except that Desktop only ever writes
**one** of them: the account it has loaded. Claude Companion builds on that:

1. It works out which account Desktop has loaded, from the newest
   `[LocalSessionManager] Initialization succeeded — accountId=…, orgId=…` line in Desktop's
   `main.log`, and `lastKnownAccountUuid` in its `config.json`. If Desktop is running but neither
   can be read, every index counts as loaded. What the app calls open is narrower: the account
   in `lastKnownAccountUuid`, since the newest log line can predate the last sign-in.
2. It never modifies an index Desktop has loaded. It copies that index's changes to all the
   others as they happen.
3. After you switch accounts or quit, the index you left is caught up. A just-left account stays
   read-only for 30 seconds while Desktop flushes it.
4. An index Claude Companion has never seen (an account signing in for the first time, or every account
   on the very first run) is the one exception: sessions it lacks can be added while it is loaded,
   because nothing in it is overwritten. Desktop shows them after one restart, and the app tells
   you when that is needed.

## Deciding what each index should contain

For every session:

- **Newest wins.** The copy with the newest file modification time wins, ties going to the
  greatest partition key, so every run agrees. Copies keep the source's mtime, so a copy never
  looks newer than what it was copied from, and nothing ping-pongs.
- **Healthy beats damaged.** If the newest copy lost its transcript link (`cliSessionId` gone or
  `transcriptUnavailable`) while an older copy points at a transcript that is still on disk, the
  older copy wins. Clearing or rewinding a conversation is not damage: Desktop first lists the
  transcript it moves off in `priorCliSessionIds` (clearing can also keep it in
  `preClearCliSessionId`), so a newest copy that left the older copy's transcript behind still
  wins. A transcript Desktop could not resume is dropped without that trace.
- **Deletions.** A `deleted_<session id>` marker newer than the session's newest copy means the
  session was deleted after its last change. It is moved to the trash everywhere, and the markers
  are copied everywhere, which also stops Desktop's importer from offering the session again. A
  session newer than its marker has come back, so the stale markers go to the trash, as Desktop
  does itself when a session returns.
- **Account-bound fields stay home.** These are never copied:
  - `remoteMcpServersConfig`, `enabledMcpTools` and `withheldConnectorHosts`: the organization's
    connectors.
  - `bridgeSessionId(s)`, `steeredByRemoteClient` and `remoteControl*`: Remote Control mirrors are
    server sessions owned by an account.
  - `publishedArtifacts`.
  - `isStarred`: pins are star-synced with each account's server settings.
  - `sshRemoteProcessId` and `sshReattach`: a session over SSH leaves Claude running on the host,
    signed in to the account that started it. Another account starts a process of its own there
    and carries on from the same transcript (`sshRemoteTranscriptPath` travels).

  Each account keeps its own values, and a new copy starts without them. Desktop reads all of
  these with defaults, so absent is safe. `remoteMcpServersConfig` is typically about 97% of a
  record, so the engine keeps only the portable part in memory and reads the target's own fields
  back at write time.
- **Collection files.** `scheduled-tasks.json` and `backlog/tasks.json` get a three-way merge
  against what each account had at the last sync. Lists of objects with an `id` merge item by item.
  An account's edits since its last sync win; an account that has never been synced can add items
  but not remove any; an item removed from the group stays removed. Desktop's indentation is kept.
- **Archive hint.** `archived-sessions.idx` is regenerated byte-for-byte the way Desktop writes it.

## Writing safely

- **Snapshots.** Before its first change, Claude Companion takes a baseline snapshot of every index and
  keeps it forever. After that it snapshots at most hourly and keeps the 20 newest. Each snapshot
  is a copy-on-write clone (APFS, Btrfs/XFS, ReFS; a plain copy elsewhere). `restore` puts a
  snapshot back (Claude must be closed) after taking a `pre-restore` snapshot.
- **Nothing is deleted.** Removed or replaced files go to `<state>/trash/<time>/…` for 30 days.
- **Atomic writes.** Every write goes to a temporary file in the same folder, is fsynced, gets its
  mtime, and is renamed into place. New files use a no-clobber rename, so a file Desktop created
  meanwhile is never overwritten. An existing file is only rewritten if its mtime still matches
  what was planned.
- **Hands off the unexpected.** Symlinks are never followed. Records whose `sessionId` does not
  match their file name, or that cannot be parsed, are left alone.
- **One run at a time**, via an exclusive lock (`flock` / `LockFileEx`) on `<state>/lock`.
- **Re-checking.** While applying, Desktop's state is re-checked every 250 ms, in case it launches
  or switches accounts mid-run.

## The background agent

`claude-companion install` (or the app's switch) copies the binary to `<state>/bin` and registers it to run
`watch` at login: a LaunchAgent on macOS, a `Run` value on Windows (a windowless build, so nothing
flashes), and a systemd user service on Linux, with an XDG autostart fallback. The Mac app is the
exception: it registers its own executable where it is, because the app's signature covers that
file only inside `Claude Companion.app`, and macOS refuses to run a copy of it anywhere else (0.1.0 and
0.1.1 made that copy; the app sets such an agent up again when it opens). For the same reason it
won't set the agent up while it runs from a disk image. The agent:

- polls a cheap fingerprint of the index folders and Desktop's `config.json` every 2 seconds;
- waits up to 6 seconds for a burst of writes to settle, then syncs;
- runs a full pass every minute anyway;
- writes a heartbeat to `<state>/agent.json` every 15 seconds, which the app reads.

On macOS the LaunchAgent names the app it belongs to (`AssociatedBundleIdentifiers`), so System
Settings lists it as Claude Companion rather than under the name on the signing certificate.

It needs no network access. A full pass over three accounts with about 200 sessions each peaks at
roughly 25 MB of memory.

`<state>` is `~/Library/Application Support/claude-companion` (macOS), `%APPDATA%\claude-companion` (Windows)
or `~/.local/share/claude-companion` (Linux).

## The desktop app

The app is a window over the same engine, plus a menu bar (macOS) or notification area (Windows,
Linux) icon. The icon keeps running when the window is closed and shows the headline, *Sync now*
and the background switch; it is a `tray-icon` status item on macOS and Windows and a
StatusNotifierItem (`ksni`) on Linux. With the icon hidden, closing the window quits the app.
*Open at login* registers the app to start with `--hidden`, so it comes up in the tray only.
On macOS the app is an `LSUIElement`: it takes the regular activation policy (a Dock icon, a
place in ⌘-Tab) while its window is open and goes back to accessory when the window closes,
unless *Always show in the Dock* keeps it regular.

The app never syncs by itself: it reads, and runs the engine when you press *Sync now*. The
background switch installs the same agent as `claude-companion install`.

Its preferences live next to the sync settings in `<state>/config.json`: `appearance`
(`system`, `light`, `dark`), `language` (`system` or a code such as `zh-CN`), `tray`, `dock`
(macOS), `checkUpdates` and `autoUpdate`.

### Updates

The app is the only part that goes online, and only for this. A few seconds after it starts and
then once a day it asks GitHub's API for the latest release. When there is a newer one:

1. It downloads the archive for this system (`Claude-Companion-<version>-<platform>`) into
   `<state>/updates` and checks its size and SHA-256 against the digest GitHub recorded at upload.
2. It unpacks the archive and starts the new program with `--version`, which must answer with
   the expected version. On macOS the new bundle must also pass `codesign --verify --deep --strict`
   and be signed by the same team as the running copy (a copy without a team, built locally,
   accepts only what Gatekeeper accepts).
3. It swaps the new copy in: on macOS by renaming the bundle, on Windows by renaming the running
   program aside (Windows allows that, not overwriting it), on Linux by replacing
   `bin/claude-companion-app`, and `bin/claude-companion` next to it. Without permission to write there, it offers
   the download page instead.
4. It starts the new copy with `--after-update <pid>` and quits; the new copy waits for the old
   one to exit, then removes what is left of it.

With *Install updates automatically* on (the default), step 3 waits until the window is closed or
the app quits; otherwise **Update** does it all at once. A development build does not update
itself. The background agent keeps running the version it started with, so after an update the
app sets it up again the next time it starts. *What's new* shows the
release notes, from [CHANGELOG.md](../CHANGELOG.md).

## Transcripts

Claude Companion copies session records, not transcripts: every account already reads the same
`~/.claude/projects`. Since Claude Code 2.1.248 the transcript of a session started or last
continued in Claude Desktop or Cowork is kept at any age. `desktopSessionCleanupPeriodDays` can give
those an age limit, and a managed `cleanupPeriodDays` applies to them too. `cleanupPeriodDays`
(30 days by default) still covers terminal sessions and other data, and before 2.1.248 it covered
Desktop sessions as well. A session whose transcript is gone still shows in the list but opens empty.

Claude Companion warns only when Desktop sessions would lose their transcripts within a year.
**Keep them** in the app, or `claude-companion retention --keep`, removes the Desktop limit (on an older
Claude Code it raises `cleanupPeriodDays` to ten years instead), backs up the settings file first,
and can be undone with **Undo** or `claude-companion retention --undo`. Don't set `cleanupPeriodDays` to
`0`: Claude Code rejects it.

## Switching accounts

Claude Desktop keeps one sign-in: the claude.ai web session (the `Cookies` database and the
`Local Storage`, `Session Storage`, `IndexedDB` and `WebStorage` folders) and the tokens it caches
in `config.json` (`oauth:tokenCache`, `oauth:tokenCacheV2`, `lastKnownAccountUuid`), all encrypted
with a key in the login keychain. Signing out ends that sign-in on the server, so switching by
signing out and in again loses it.

Claude Companion switches without signing out (macOS for now):

1. It asks Claude to quit, the way the Dock does (SIGTERM, so no "control Claude" permission is
   needed), waits for it and its helpers to exit and for its updater to finish, and takes the
   sync lock. The other programs in Claude's app don't count: its crash reporter, for one, can
   keep running after Claude quits. With work in progress, Claude asks first ("Claude is still
   working"); Claude Companion reads the answer from Claude's `main.log`
   (`vetoed by before-quit interceptor`, then `[updater-guard] restart deferred` for Wait for
   Claude). While Claude asks, Claude Companion says so and can stop waiting (Cancel in Claude leaves no
   trace to read; after ten minutes it gives up anyway); when the answer is to wait, it waits as
   long as the work takes. Stopping changes nothing.
2. It moves Claude's sign-in, as it is, to `<state>/logins/<account>` (a rename on the same disk;
   the account is `lastKnownAccountUuid`), and moves the target account's back in its place: the
   files as they were, and the `config.json` keys written back in Claude's own format.
   `bridge-state.json`, tied to the old account, is dropped; Claude makes it again.
3. It starts Claude again if it was running. So does a switch that fails after Claude quit, with
   nothing changed. A switch from the window or the menu bar opens Claude even when it was closed;
   `claude-companion switch` leaves a closed Claude closed.

Nothing is decrypted, read, refreshed or sent anywhere: Claude's own files move between two
folders on the same Mac. Tokens rotate while Claude runs, so the sign-in being left is saved afresh
at every switch, and a saved copy is put back only once. Every move is written to a journal first:
a failed switch is undone, and one cut short (a crash) is undone or finished the next time.

To add an account, Claude Companion sets the current sign-in aside and restarts Claude on its sign-in page;
signing in there goes through Anthropic's own flow as usual, and Claude Companion takes note once Claude has
a new sign-in. A sign-in left unused for about four weeks expires on the server; Claude then asks
to sign in again, and nothing else is lost.

Why not trade only the token, as [claude-swap](https://github.com/realiti4/claude-swap) does for
the Claude Code command line? Desktop does not use that login at all: it hands its Code and Cowork
sessions its own token (`CLAUDE_CODE_OAUTH_TOKEN`), and keeps its sign-in in a running web session
that a restart is the only way to change. Trading Desktop's tokens would also mean decrypting and
handling them, which Claude Companion never does. It keeps the files whole and unread instead, and moves
rather than copies them, so a sign-in whose tokens rotate is never put back twice. The Claude Code
command line has a sign-in of its own, which Claude Companion leaves alone; one `CLAUDE_CONFIG_DIR` per
account is Anthropic's way to keep several there.

### The list of accounts

The accounts to switch between are a numbered list in `<state>/accounts.json`, the way claude-swap
keeps one. An account joins by itself the first time Claude Companion sees Claude signed in to it (the app,
the background agent and the command line all look), or when a sign-in is kept for it, and keeps
its number until it is removed. Accounts that only have session folders, signed in to before CC
Same kept sign-ins, are shown apart until Claude signs in to them once more.

- **Next Account** (the menu bar, or `claude-companion switch` without an account) goes to the next number
  after the account in use that has a sign-in kept, starting over after the last.
  `--strategy next-available` skips accounts whose plan was used up when Claude last read it, and
  `--strategy best` picks the one with the most left.
- An **alias** (`claude-companion alias 2 work`, or the pencil beside an account's name in the window) is a
  lowercase name of up to 24 letters of any script, digits, `-`, `_` and `.`, not only digits. The
  window shows it in place of the email (or of the ID, when Claude Companion doesn't know the email).
  Anywhere an account is named, its number, alias, email or the start of its ID works.
- An account can **sit out** of Next Account (`claude-companion disable`, or the check mark in its row's
  menu) and still be switched to by name, or **move** to another number (`claude-companion move`), trading
  places with the account that had it.
- **Removing** an account (`claude-companion remove`, or its row's menu) forgets the sign-in kept for it. Its
  sessions stay; the account Claude is signed in to cannot be removed, since it would join again at
  once.

### Plan usage

While it runs, Desktop asks claude.ai how much of the open organization's plan is used, and keeps
the answers for 30 days in `plan-usage-history.json` (`fh` is the 5-hour window and `sd` the
weekly one, in percent). It asks when it starts, when the account changes, when its menu bar menu
opens (a right-click on its icon), and every 15 minutes in between. Since October 2026 (Desktop
2.19675) a switch on Anthropic's side can pause those in-between checks until the menu has been
opened in the last 24 hours: `main.log` then says `[plan-usage] background poll paused: tray not
opened recently`, and even the open account's numbers stand still until Claude restarts, switches
accounts or its menu opens.

Claude Companion shows the newest answer for each account. It never asks Anthropic itself, so an account's
numbers are as fresh as the last time Claude asked: a reading older than half an hour shows its
age (for the account Claude is signed in to, with how to have Claude check again), a 5-hour
reading older than five hours counts as a window that has started over (or as unknown for the
account Claude is signed in to, which may be filling a new one), and a weekly one older than a
week is not shown.

The samples name an organization, not an account. Desktop only gets an answer for an organization
the signed-in account belongs to, so the samples taken after an account loads (the time of its
`Initialization succeeded` line in `main.log`) belong to its organizations, and Claude Companion notes them
in the list. Other samples are attributed by organization: one used by a single account belongs to
it (from what was noted, else from the session folders). An organization several accounts share,
such as a team, is left out rather than guessed.

### Usage history

Both sources of usage forget: Desktop keeps `plan-usage-history.json` for 30 days, and Claude Code
deletes old transcripts (`cleanupPeriodDays`). Claude Companion copies them into its own SQLite
file, `usage-history.sqlite` in its state folder, so they can be kept as long as the user wants.
The background agent does it every five minutes on a thread of its own, and the app does it when
the history sheet opens; either one, or the command line (`history collect`), gives the same result.

* **Plan readings** are keyed by time and organization, so reading the same file again adds
  nothing. Each is stored with the account that owns its organization (the attribution described
  above); a reading taken before the owner was known is attributed later. Readings of an
  organization several accounts share stay unattributed.
* **Tokens** are counted from the transcripts the way `stats` counts them (each reply once, however
  many files it was written to) and kept per local day, model and project folder. Transcripts are
  shared by all accounts, so these are totals, not per-account figures. The first count reads every
  transcript; after that only files changed since the last count (with a day's margin) are read.
  A day's kept figure never goes down: when Claude Code deletes old transcripts, the recount of that
  day is lower than what was kept, and the larger value wins.
* **Keeping.** `historyKeepDays` in the configuration (730 by default; 0 keeps everything) trims
  older rows once a day; `history` turns collecting off (what was kept stays). Nothing is ever
  deleted other than by that limit or an explicit *Delete* (`history clear`).

The app's graphs compare accounts as one line each (daily peak of the 5-hour and weekly percent,
grouped by week or month for long ranges, with the weekly level carried across idle days for up
to a week), then tokens per day, by model and by folder.

## Sidebar groups

*Opt-in (`config --groups on`). A Rust port of the group merge in
[claude-desktop-session-sync](https://github.com/RasmusKD/claude-desktop-session-sync); the code
is `crates/core/src/groups.rs`.*

Custom sidebar groups (a name, the chats in it, their order) are stored per account and
organization in Claude's **Local Storage**, a LevelDB under `Local Storage/leveldb` in Claude's
data folder, in two keys of the `https://claude.ai` origin: `dframe-store` (what the sidebar
loads) and `LSS-persisted.dframe-group-scopes` (a pruned copy). Chromium stores strings as a
format byte plus Latin-1 or UTF-16LE, and keeps a `META:` record with the origin's size and
modification time, which has to be kept right when a value changes. Claude also mirrors the value
into `claude_desktop_config.json`.

A run, only when Claude is closed:

1. Copy the database twice: one pristine backup, and one to read from. (Opening a LevelDB changes
   its files, so nothing is opened in place until a write is certain.)
2. Read each included account's entry and merge them **three ways** against the result of the last
   run (`groups/groups-base.json` in the state folder). A group no base knew is an addition; a
   rename or reorder in any one account wins over the base; a chat follows the account that moved
   it.
3. A group counts as **deleted** only when an account lacks it *after that account was last seen
   in step with its server copy* — otherwise an account that simply had not received the group yet
   would delete it everywhere. A deleted id is remembered and never returns, even if an account
   that missed the deletion hands it back later. With no base, nothing is deleted.
4. The server copy is the truth for each account: at start Claude replaces the local list with it,
   unless a **pending marker** (`ccd-sync-pending:ccd/dframe-store`) names the account it starts
   on, in which case it pushes the local list instead. So the merge sets that marker for the
   account Claude will start on; the other accounts pick the merged groups up when you switch to
   them. Start Claude on your usual account after a merge.
5. Write both keys and the `META` record in one batch, reopen the database and check the values
   read back; if anything is wrong, put the backup back. Keep the last five backups.
6. Splice the merged value into the config file's mirror key as raw text (a walk from the document
   root, so a look-alike key elsewhere is never touched; every other byte, including a byte order
   mark, stays), after copying the file.

What it does not do: it never runs while Claude does (on Windows, Claude keeps running in the tray
when its window is closed, so quit it from the tray first), and it has been tested against
databases built to the same layout, not against a real Claude profile. If a future Claude changes
the layout, the run finds no group keys, or JSON it cannot read, and changes nothing.

## Claude Code and MCP servers

Claude Code (the `claude` program) keeps its own files, apart from Claude Desktop's: user settings
in `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR`), the account it is signed in as and its
user-wide MCP servers in `~/.claude.json`, and one transcript per session under
`~/.claude/projects/<folder>/<id>.jsonl`. Desktop's Code sessions use those same transcripts, so a
session in the list can belong to both; `code sessions` marks the ones a Desktop chat card names.

- **Finding it.** `CLAUDE_COMPANION_CLAUDE`, else `claude` on the PATH and the places its installers
  use. On Windows Claude Desktop's own `claude.exe` (under `AnthropicClaude\app-…` or a Store
  package) is skipped, since it is not the terminal program.
- **Continuing a session.** Claude Code looks a session up in the folder it starts in, so `resume`
  starts it in the folder the transcript says it worked in, and refuses when that folder is gone.
  A new window is Windows Terminal (or a console), `Terminal.app` through `osascript`, or the first
  known terminal on Linux; every argument is quoted for the shell it passes through.
- **Settings.** A change reads the whole file, sets one dotted path, writes it back atomically and
  keeps the previous file next to it. Shown settings hide the values of anything that looks like a
  secret and of every `env` and `headers` entry.
- **Claude Code's sign-in.** On Windows and Linux it is `<config dir>/.credentials.json` plus the
  `oauthAccount` entry of `.claude.json` (inside `$CLAUDE_CONFIG_DIR` when that is set, else in the
  home folder). `code login save` copies both into `<state>/code-logins/<account id>/`, the
  credentials byte for byte. `use` first saves what is signed in now (Claude Code rotates its
  tokens, so the newest copy is the one to keep), backs up both files, writes the other
  account's, reads them back, and restores both files on any mismatch. macOS keeps the sign-in in
  the Keychain; moving it would mean passing the secret to `security` on a command line, so it is
  not offered. The Desktop account list is separate: switching one does not switch the other.
- **MCP servers.** Desktop's `claude_desktop_config.json` and Claude Code's `~/.claude.json` both hold
  an `mcpServers` object, and a local-program entry (`command`, `args`, `env`) means the same in
  both. `mcp share` copies what one lacks into the other, takes Claude Code's `type: "stdio"` out
  for Desktop, leaves remote (`http`/`sse`) servers out of Desktop, and keeps a server that differs
  unless a side and `--overwrite` are given. Desktop's file is written only while Desktop is
  closed, since it rewrites the file when it quits. Both writes keep a backup and are read back; a
  file that does not read back as written is put back. Claude Code rewrites `~/.claude.json` often,
  so run `mcp share` while no Claude Code session is starting up.

## What cannot be synced from the outside

- **Sidebar groups and order, while Claude runs.** Desktop keeps them per account and organization
  in its web storage (`dframe-store`) and syncs them with the server as the user setting
  `ccd/dframe-store`. At every start and account switch the server's copy replaces the local one,
  and the storage is a database Claude holds open. They can only be merged while Claude is closed
  (see [Sidebar groups](#sidebar-groups)), and reach an account's server copy only once Claude has
  started on it.
- **Pins.** They are star-synced with each account's server settings, so they stay per account.
- **claude.ai chats, projects, memory, cloud Code and Cowork sessions, connectors, routines.**
  These live in each account on Anthropic's servers.
- **Cowork tasks.** Since October 6, 2026, new Cowork tasks on Pro and Max plans run in the cloud,
  so they belong to the list above. Claude Companion leaves local ones (`local-agent-mode-sessions/`) alone
  as well; it only reads their records to learn each account's email address.

## Desktop's own importer

Desktop 2.9939 has **Import Claude Code CLI Sessions…** in its menu. It finds sessions from other
organizations too. It is a one-off copy, though: archive state and permission grants are not
restored, and later changes do not follow. Claude Companion keeps every account identical continuously,
so the importer finds nothing left to import.
