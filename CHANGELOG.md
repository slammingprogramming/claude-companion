# Changelog

What changed in each version of Claude Companion. The app shows these notes after it updates, and
under Settings › Updates.

Versions up to 0.1.13 are the history of [cc-same](https://github.com/songkeys/cc-same) by
Songkeys, kept as written (so they say "CC Same"); Claude Companion forks from 0.1.12 and merged
0.1.13 afterwards.

## Unreleased

### Added

- Community files: a Code of Conduct (Contributor Covenant 2.1), a security policy, contributing
  and support guides, issue forms for bugs and ideas, and a pull request checklist.

### Changed

- **Plan use shows as soon as Claude writes it.** While the window is open, the app watches Claude's
  plan-usage record every 1.5 seconds instead of waiting for the 12-second refresh (from
  Arcaneology's cc-same pull request 9).
- The app carries only released changelog sections: `build.rs` leaves out `## Unreleased`, so notes
  there reach no build until they have a version number. A release warns if one is left behind.
- CI and the CodeQL scan no longer run for changes that only touch documents, licence texts or
  GitHub's community files. CodeQL is now a workflow (`.github/workflows/codeql.yml`: actions, Python
  and Rust, default queries, weekly) instead of GitHub's default setup, which cannot skip by path.
  Editing `CHANGELOG.md` alone starts neither.

## 0.2.3 - 2026-11-22

### Added

- **Weekly reset countdown.** Claude keeps no reset time on disk, so you give it once: in the app, the
  pencil on an account (*Edit…*) takes a weekday and a time; on the command line,
  `claude-companion reset <account> <day> <time>` (or `--clear`; no account lists them). The account
  list then shows the time left (`3d 4h`) with the day and time on hover, and `accounts` adds
  `resets in 3d 4h` (and `weeklyReset` / `nextWeeklyReset` in `--json`). A weekly reading from before
  the last renewal counts as 0%. Taken from Arcaneology's cc-same pull request 9 (credited in NOTICE.md);
  nothing is read or sent, and accounts without one are unchanged.

### Fixed

- **The macOS build.** The 0.1.13 merge left forced quit using constants upstream had replaced, so
  the macOS app and command line did not compile and the 0.2.2 release was never published. This
  release is the first with the 0.1.13 changes built on macOS.

### Changed

- Dependencies: the Dependabot updates for clap, dirs 7, libc, reflink-copy, serde_json, uuid,
  windows-sys 0.61, sha2 0.11, tray-icon 0.26 and the rest of the weekly groups, with the code they
  needed. `rusty-leveldb` stays on 3.x (it writes Claude's sidebar-group database) until 4.x is tried
  against a real Claude.
- GitHub Actions run on Node 24: checkout 7, upload-artifact 7, download-artifact 8, action-gh-release 3.

## 0.2.2 - 2026-11-21

Tagged, but its macOS builds did not compile, so no release was published; the fix is in the next
version.

### Changed

- **Merged upstream cc-same 0.1.13.** Switching from the window or the menu bar now opens Claude
  even if it was closed (`claude-companion switch` still leaves a closed Claude closed), and the
  confirmation says so.

### Security

- **rustls 0.23.46** in the app (four Dependabot advisories: session Debug output exposing TLS
  secrets, Brotli certificate decompression memory, ECH rejection, and a key-exchange poisoning case).

### Fixed

- **macOS: switching accounts could stop at "Switching...".** Claude's crash reporter can keep
  running after Claude quits, and the switch waited for it. It now waits for Claude and its
  helpers only, waits for Claude's updater to finish, and opens Claude again if a switch fails
  after Claude quit. (From upstream 0.1.13; not testable on Windows, so unverified here.)

## 0.2.1 - 2026-11-20

### Changed

- **New icon and tray artwork** (two overlapping window outlines with a prompt chevron), replacing
  cc-same's rings, with the script that draws it all (`scripts/make-art.py`).
- Switching accounts was checked against copies of a real Windows setup, not only stand-in data:
  Claude Desktop's sign-in (63 files) went away on `add` and came back identical on `switch`, and
  Claude Code's sign-in switched back and forth with its credentials byte for byte the same.
  Nothing of the original was touched. Real Claude opening the results is still untested.

### Fixed

- **The minimum Rust version is 1.89**, not the 1.85 the manifest, README and CI claimed: a locked
  dependency (`uuid`) needs 1.89. Found by building on Linux in a container.
- Two tests failed on Linux (not the product): one relied on file times that Linux does not tell
  apart within a few milliseconds, the other on a read-only folder that root ignores. Both are
  fixed, and the engine and command-line tests now pass on Linux, as root and as a normal user.
- **Rewriting Claude Code's `.claude.json` no longer alters numbers.** The file holds long
  decimals (costs, timings); they used to come back one digit off in the last place after a
  switch, MCP share or settings change. Every JSON file this tool rewrites now keeps its numbers
  exactly (`float_roundtrip`).

## 0.2.0 - 2026-11-20

### Added

- **Switching accounts on Windows and Linux**, as on macOS: `claude-companion switch`, `add`,
  and the app's account list now work there too. The sign-in files are moved as they are, never
  read or decrypted. Windows also keeps the sign-in's cookies in `Network`, which is handled, and
  Claude is started again from its Store package or its installer build, whichever holds the
  data in use. How Claude's sign-in is laid out on Windows and Linux follows the research in
  [claude-switcher](https://github.com/nyxon-tech/claude-switcher) by NeezerGu and Nyxon.
- **Quitting Claude on Windows.** Windows keeps Claude running in the tray when its window
  closes, and nothing from outside quits it politely. The app says so and offers **End Claude**;
  on the command line, `switch --force` and `add --force` do the same after a few seconds.
  Linux sends the quit request (SIGTERM) to the app, as macOS does.

- **Lost chats, copying and export** (`claude-companion chats …`), after
  [claude-switcher](https://github.com/nyxon-tech/claude-switcher):
  - `chats list` shows every chat of every account; `chats lost` finds conversations whose
    history is on disk but that no account's list shows, and `chats rescue --to <account>` gives
    them a card again so Claude shows them (your other accounts get them through sync).
  - `chats copy --from <a> --to <b>` puts one chat, or all of them, into another account's list
    (the newer card wins, deleted chats stay deleted): for accounts you keep out of sync.
    There is no *move*: a deleted card travels to every account that syncs, so a move would
    delete the chat everywhere.
  - `chats export <id> [--format md|html]` writes a chat as Markdown or a self-contained page.
  - Every write goes through the same engine as sync: never into a list Claude has open, only
    after a snapshot (`restore <id>` undoes it), and never over a newer card.
- **Token statistics** (`claude-companion stats`): tokens by model, day and folder, counted once
  per reply from Claude Code's local transcripts. Nothing is sent anywhere.

- **The same in the app.** Settings has a new **Tools** group: *Lost chats* (find them, then
  rescue them after a confirmation), *Export a chat* (Markdown or a web page, into Downloads),
  *Token statistics*, *Diagnostics* (what `doctor` prints) and how long Claude Code keeps other
  transcripts (`retention <days>`). The account list has arrows to reorder accounts (`move`).
  With these, nothing the command line does is missing from the app, and the command line has
  `doctor` and `plan` for what the window shows as its headline. The app's new rows were checked by
  building, linting and unit tests, not yet by eye on every system.

- **One set of sidebar groups for every account** (`claude-companion groups`, off until you turn
  it on with `config --groups on` or Settings › Sync; Settings › Tools has *Preview* and *Merge
  now*). Claude keeps each account's custom groups in its Local Storage; this merges them three
  ways against the last result, so a group made, renamed, reordered or deleted in one account
  reaches the others, and a deleted group does not come back. Ported from the group merge in
  [claude-desktop-session-sync](https://github.com/RasmusKD/claude-desktop-session-sync) by
  RasmusKD, which also taught the order of things: groups live on claude.ai for each account, so
  the merge marks the account Claude starts on as the one to push, and the others pick the groups
  up when you switch. Only while Claude is closed; Local Storage and Claude's config file are
  backed up first, and the write is checked and rolled back if it does not read back. Tested
  against stand-in databases, not yet against a real Claude profile.

- **Control Claude Desktop and Claude Code from one place.**
  - `claude-companion code status | sessions | resume | run | settings`: find Claude Code
    (`CLAUDE_COMPANION_CLAUDE` overrides, Claude Desktop's own `claude.exe` is never mistaken for
    it), see who it is signed in as, list its terminal sessions, continue one in the folder it
    worked in (in this terminal or `--new-window`: Windows Terminal or a console on Windows,
    Terminal on macOS, the first known terminal on Linux), and read or change its user settings
    by dotted name. Shown settings hide secrets; a change keeps the previous file beside it.
  - `claude-companion mcp list | share`: the MCP servers of Claude Desktop
    (`claude_desktop_config.json`) and Claude Code (user scope in `~/.claude.json`) side by side,
    and a copy of what one lacks into the other. Servers that differ are kept unless you pick a
    side with `--to` and `--overwrite`; remote servers do not go into Desktop; Desktop's file is
    only written while Desktop is closed; backups and a read-back check as for groups.
  - `claude-companion open desktop | code [dir]` and `quit`; `doctor` now reports on Claude Code too.
  - In the app, Settings › Tools has *Claude Desktop* (open), *Claude Code (terminal)* (a new
    terminal, or continue a recent session) and *Share MCP servers* (preview, share).
  - `claude-companion code login save | list | use | forget`: switch the account **Claude Code**
    is signed in to (Windows and Linux; the app has *Claude Code sign-in* in Settings › Tools).
    A saved sign-in is a byte-for-byte copy of `.credentials.json` plus the `oauthAccount` entry of
    `.claude.json`, kept in Claude Companion's private state folder and never read or printed.
    Switching saves the current sign-in first (so refreshed tokens are kept), checks the result
    reads back and puts both files back if it does not. `CLAUDE_CONFIG_DIR` is honoured. Not on
    macOS, where the sign-in is in the Keychain. Tested with stand-in files only, never against a
    real Claude Code sign-in; close running Claude Code sessions before switching.
  - **App and command line match.** Settings › Tools gained what it lacked: *All chats* (show,
    and copy one account's chats into another's), *Terminal sessions*, *Claude Code settings*
    (show, and the permission mode), *Show servers* for MCP, *Quit…* for Claude Desktop, *New in
    folder…* and *Forget…* for Claude Code. What stays on one side only is listed in the README.
  - Opening terminal windows is untested on macOS and Linux.

- **Contact and support.** `claude-companion about` (and Settings › About and support in the
  app) shows the version and licence, a SimpleX contact link for the developer, the Ko-fi page and
  crypto donation addresses (buttons copy each address; the Bitcoin one is a Silent Payments
  address). It only shows them: nothing is sent anywhere. A `.github/FUNDING.yml` adds a Sponsor
  button on GitHub.

> Switching on Windows and Linux is tested against stand-in data and compiles on all three
> systems, but it has not yet been run against a real Claude Desktop there. Treat it as a
> first release of that feature, and report what you find.

### Changed

- **Claude Companion.** This is a hard fork of cc-same 0.1.12, renamed to Claude Companion. The
  command is `claude-companion`, settings and state live in a `claude-companion` folder, the
  environment variables start with `CLAUDE_COMPANION_`, and updates come from this project's
  releases. It runs alongside a cc-same install without sharing its state, but run only one of
  the two background agents.
- **New license: AGPL-3.0-or-later.** The inherited code stays available under its original MIT
  terms upstream; its notices are kept in `LICENSES/`, and `NOTICE.md` credits every project this
  one builds on.

## 0.1.13 - 2026-11-03

### Changed

- **Switching opens Claude.** Switching from the window or the menu bar restarted Claude only when
  it was open: with Claude closed, it changed the sign-in and left Claude closed. Now Claude opens
  on the new account either way. `cc-same switch` still leaves a closed Claude closed.

### Fixed

- **macOS: switching accounts could stop at "Switching…".** CC Same waited for every program in
  Claude's app to exit, and Claude's crash reporter can keep running after Claude quits. Claude
  closed, nothing happened for a minute, then the switch gave up and left Claude closed. CC Same
  now waits for Claude and its helpers only, and a switch that fails after Claude quit opens
  Claude again. Claude's updater went unnoticed too; a switch now waits for an update to finish
  before touching Claude's files.

## 0.1.12 - 2026-10-28

### Changed

- **macOS: the Dock icon goes with the window.** CC Same used to stay in the Dock for as long as
  it ran, even when all you used was its menu bar icon. Now it's in the Dock while its window is
  open and leaves when you close it, and at login it starts in the menu bar alone. To keep it in
  the Dock, turn on **Always show in the Dock** in Settings.

## 0.1.11 - 2026-10-27

### Fixed

- **SSH sessions could reconnect to another account's Claude.** A Code session over SSH leaves
  Claude running on the host, signed in to the account that started it. CC Same copied the link
  to that process along with the session, so after a switch another account could pick it up
  and use the first account's plan. Each account now keeps its own; the conversation still
  carries over, and the other account starts a process of its own on the host.
- **Windows: no sessions with Claude Desktop 3 from the Store.** Desktop 3 moves a Store build's
  data to `%LOCALAPPDATA%\Claude-Data`, where CC Same didn't look, so it found no sessions and
  stopped syncing. It looks there now, and in the older folder of either of Desktop's Store
  packages.
- **Windows: the Settings button didn't open Settings.** Windows took a click on it for the start
  of dragging the window, so only Ctrl+, worked.

## 0.1.10 - 2026-10-05

### Added

- **Name an account**: point at it in the window and click the pencil beside its name (or
  right-click it and choose **Name…**). The list then shows that name instead of the email, or
  instead of the ID of an account whose email CC Same can't find; a name given with
  `cc-same alias` now shows the same way, rather than in a tag beside the email. Names can use
  letters of any script, like 工作 (up to 24 characters), and the command line takes them too.

### Removed

- **Cowork sync.** From October 6, 2026, new Cowork tasks on Pro and Max plans run in the cloud,
  where each account keeps its own and CC Same can't reach them, so the experimental option is
  gone from Settings and from `cc-same config`. Cowork sessions copied earlier stay where they are,
  and Code sessions sync as before.

### Fixed

- **Plan usage could look up to date when it wasn't.** Claude checks an account's plan every 15
  minutes while it runs, but since Desktop 2.19675 Anthropic can pause those checks until Claude's
  menu in the menu bar (or system tray) has been opened in the last day, so a reading could be a
  day old. A reading older than half an hour now shows its age, in the window and in the menu bar.
  For the account Claude is signed in to, a 5-hour reading that old shows as unknown, and pointing
  at it tells you how to have Claude check again.

## 0.1.9 - 2026-10-01

### Changed

- **Your accounts are a numbered list, the way claude-swap keeps them.** An account joins by
  itself the first time Claude signs in to it, and keeps its number. In the menu bar, Switch
  Account starts with **Next Account**, which goes round the list; an account's menu in the window
  (right-click) leaves it out of that, or removes it from the list. The command line follows
  claude-swap too: `cc-same switch` goes to the next account, `cc-same switch 2` or
  `cc-same switch work` to one (aliases come from `cc-same alias`), and `--strategy best` to the
  one with the most of its plan left; `add`, `remove`, `disable`, `enable`, `move` and
  `accounts --json` are new. `sign-in` and `forget` still work.

### Added

- **Plan usage beside each account**: the 5-hour and weekly windows, as Claude last read them.
  CC Same reads them from Claude's own record and asks Anthropic nothing, so an account's numbers
  are as fresh as the last time Claude used it.

### Fixed

- **A switch could hang on “Restarting Claude…”.** When Claude has work in progress, it asks
  before it quits (“Claude is still working”). Choosing Cancel there left CC Same waiting for a
  minute, and choosing Wait for Claude made it give up while Claude went on to quit later, still
  signed in to the same account and not reopened. CC Same now follows the answer: while Claude
  asks, it says so and offers **Stop Waiting**; when the answer is to wait, it waits for the work
  to finish and then switches.

## 0.1.8 - 2026-10-01

### Fixed

- **A cleared conversation could come back.** If you cleared a conversation and didn't write in
  it again, say you cleared it and then archived it, CC Same mistook the cleared session for a
  damaged one and kept the copy from before the clear. Your other accounts kept the old
  conversation, the window said an update would land when you switch accounts or quit Claude, and
  that update gave the account you had cleared it in the old conversation back too, unarchived if
  you had archived it. A cleared conversation now reaches every account like any other change.

## 0.1.7 - 2026-09-30

### Fixed

- **Switch Account was missing from the menu bar.** The menu is built when the app starts, before
  it has read your accounts, and it added Switch Account only if it already knew then that switching
  works here, so it never did. On macOS it is now always there; the accounts fill in once they are
  read.

## 0.1.6 - 2026-09-30

### Changed

- **Switch Account in the menu bar lists every account, and asks first.** It used to list only the
  accounts CC Same had kept a sign-in for, so until your first switch it showed just the account in
  use. Now every account is there. Choosing one asks before Claude restarts, since a menu item is
  easy to hit by mistake; an account whose sign-in isn't kept yet offers to sign in to it instead.

## 0.1.5 - 2026-09-30

### Fixed

- **An account showed up once per organization**, the same email on two rows, and every row said
  “Open in Claude”. Claude keeps a session list for each organization an account is used in, and
  signing in to one account after another can leave an empty list in the other account’s
  organization. The window now shows each account once, reporting the list Claude shows for it,
  and marks only the account Claude is signed in to as open. CC Same still keeps every list the
  same. `cc-same doctor` also names only that account as open.
- **Updates to an account could wait until Claude restarted** when Claude had been open for a few
  days. CC Same read only the end of Claude’s log to see which account is in use, and could fall
  back to an older log’s line about another account. It now reads the whole log.

## 0.1.4 - 2026-09-30

### Fixed

- **Background sync stopped after updating** on macOS, with “launchctl bootstrap failed: 5:
  Input/output error”. After an update the app restarts the background agent: it now restarts it
  in place, waits for launchd to let go of the old one when it has to load it again, and retries.
  An agent that is switched on but not loaded is set up again by itself when the app opens, and a
  failure there no longer shows an error: the background switch says how it stands.

## 0.1.3 - 2026-09-29

### New

- **Switch accounts in one click** (macOS). Each account in the window has **Switch**, and the menu
  bar has **Switch Account**: Claude restarts signed in to the other account, without signing out
  of either. CC Same keeps each account’s sign-in on this Mac, as Claude stored it, and trades them
  while Claude restarts. To add an account, **Sign in** (or **+**) restarts Claude on its sign-in
  page; the account you were using stays one click away. On the command line: `cc-same accounts`,
  `cc-same switch`, `cc-same sign-in` and `cc-same forget`.

### Fixed

- **Check for Updates… in the app menu opened a second window.** It now brings the one window
  forward.
- **Messages shown while Settings is open** were pushed off the left edge of the window. They now
  appear centered at the bottom, over the settings.

## 0.1.2 - 2026-09-29

### Fixed

- **Background sync on macOS never started.** The switch stayed at “starting…” and then said “On,
  but not running”. The app set the agent up as a copy of its own program, and macOS refuses to run
  that program outside the app. The agent now runs the app’s program where it is, and an agent set
  up by 0.1.0 or 0.1.1 is set up again the next time the app opens. Run from a disk image, the app
  first asks to be moved to the Applications folder.
- **System Settings lists the background item as CC Same**, with its icon. 0.1.1 meant to, but
  the copy stood in the way.

## 0.1.1 - 2026-09-29

### New

- **Updates inside the app.** CC Same looks for a new version on GitHub once a day, downloads it in
  the background, checks it, and installs it when its window is closed or it quits. **Update** in
  the window installs it right away. Settings › Updates turns either off and shows what changed in
  each version.
- **Check for Updates…** in the menu bar / tray menu and in the app menu.

### Fixed

- **No false alarm about transcripts.** Since Claude Code 2.1.248, the transcripts of Claude Desktop
  sessions are kept at any age unless a setting limits them. CC Same now reads the settings that
  apply (yours, your organization’s, and which Claude Code you have) and warns only when a
  transcript would really be deleted. **Keep them** can be undone from Settings, and
  `cc-same retention` explains the same, with `--keep` and `--undo`.
- **“On, but not running” right after switching on.** The background switch now says the agent is
  starting, and the agent checks in every 15 seconds, even during a long sync.
- **The background item in System Settings** is listed as CC Same, with its icon, instead of under
  the name on the developer’s certificate. The background agent is updated along with the app.

## 0.1.0 - 2026-09-29

The first release: keep Claude Desktop’s local sessions identical across all your accounts. Sign
out, sign in with another account, and everything is still there.

- Every account's local Code sessions stay in step: titles, archive state, deletions, scheduled
  tasks and task suggestions. Local Cowork sessions are optional (experimental).
- The index Claude Desktop has loaded is never touched. CC Same mirrors it to the other accounts,
  and catches up the one you left after you switch accounts or quit.
- Snapshots before the first change and at most hourly afterwards, restorable from Settings or with
  `cc-same restore`. Anything removed or replaced goes to a trash folder for 30 days.
- A small app with a menu bar / tray icon and an optional quiet start at login, in light and dark,
  and in 10 languages.
- A command-line tool on the same engine: `doctor`, `plan`, `sync`, `install`, `restore` and more.
