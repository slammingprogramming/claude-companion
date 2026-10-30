<p align="center">
  <img src="assets/icon.png" width="112" alt="">
</p>

<h1 align="center">Claude Companion</h1>

<p align="center">
  A desktop app and command-line tool that keep Claude Desktop's local sessions identical across
  all your accounts, on Windows, macOS and Linux.<br>
  Sign out, sign in with another account, and everything is still there.
</p>

> **Status: early fork.** Claude Companion is a hard fork of [cc-same](https://github.com/songkeys/cc-same)
> by Songkeys, with the goals listed under [Roadmap](#roadmap). Today it does what cc-same 0.1.12
> does, under a new name and license. The roadmap items are not done yet; this README says so
> plainly rather than promising them.

## Why

Claude Desktop keeps a separate list of local Code sessions for every account and organization.
Switch accounts and the sidebar is empty, even though every conversation is still on disk in
`~/.claude/projects`. Claude Companion keeps every account's list in step: sessions, titles,
archive state, deletions, scheduled tasks and task suggestions.

It comes as a desktop app and a command-line tool that share one engine.

## What works today

Inherited from cc-same 0.1.12 (see [NOTICE.md](NOTICE.md)):

- Sync of local Code sessions, deletions, scheduled tasks and task suggestions across accounts,
  with snapshots, a 30-day trash, and atomic writes. Syncing needs no network access.
- Background sync from login, with a tray / menu bar icon.
- A numbered account list with aliases, plan usage, and **account switching on macOS**.
- Ten interface languages, light and dark themes, and update checks against this repository's
  releases.

## Roadmap

1. **GUI and CLI parity.** Every feature reachable from one is reachable from the other.
2. **Windows, macOS and Linux, fully.** Account switching, quitting and starting Claude Desktop,
   and background sync on all three (switching is macOS-only in the inherited code).
3. **Rename and relicense.** Done: the project is Claude Companion, `AGPL-3.0-or-later`.
4. **Features from two sibling projects**, re-implemented in Rust and credited:
   - [claude-desktop-session-sync](https://github.com/RasmusKD/claude-desktop-session-sync):
     sidebar groups, three-way scheduled-task merging, account freezing.
   - [claude-switcher](https://github.com/nyxon-tech/claude-switcher): copy / move / merge chats
     between accounts, recover lost chats, journaled undo, usage statistics, export.
5. **Credits kept honest**: see [NOTICE.md](NOTICE.md).

## Using the command line

```text
claude-companion doctor         what every account has, and anything that needs attention
claude-companion plan           what a sync would change (dry run)
claude-companion sync           sync once
claude-companion install        sync, then keep syncing in the background from login
claude-companion uninstall      stop background syncing (every account keeps its full copy)
claude-companion status         background sync and the last sync
claude-companion snapshots      list snapshots
claude-companion restore <id>   put every session list back as it was (quit Claude first)
claude-companion retention      how long Claude Code keeps transcripts (--keep, --undo)
claude-companion accounts       your accounts, numbered, with their plan usage (--json)
claude-companion switch [<who>] switch Claude to an account, or the next one (macOS for now)
claude-companion add            restart Claude signed out, to add an account
claude-companion remove <who>   take an account off the list, forgetting its saved sign-in
claude-companion alias <who> <name>, disable <who>, enable <who>, move <who> <number>
claude-companion config         --exclude <account-id>, --auto-join off, …
```

The first time a sync copies sessions into the account Claude has open, quit and reopen Claude
once: it only reads its session list at startup.

## Safety

- Claude Companion only reads and writes inside Claude Desktop's data folder and its own state
  folder. Every location can be redirected with `CLAUDE_COMPANION_USER_DATA`,
  `CLAUDE_COMPANION_STATE_DIR`, `CLAUDE_COMPANION_DESKTOP_LOG_DIR` and `CLAUDE_COMPANION_PROJECTS`,
  which is how the tests (and any dry run you want) avoid your real Claude setup.
- A copy-on-write snapshot of every index is taken before the first change and at most hourly
  afterwards; nothing is deleted without going to a trash folder for 30 days.
- Writes are atomic, never follow symlinks, and never overwrite a file that appeared or changed
  since planning.

## Building from source

```bash
cargo build --release                        # CLI (Rust 1.85+)
cargo test                                   # engine + CLI tests
cargo app                                    # run the app
cargo app-test                               # app tests
```

The app lives in `crates/app` as a workspace of its own, so the CLI never pulls in the GUI
framework. Interface text is in [`crates/app/locales/app.yml`](crates/app/locales/app.yml).
Linux needs `clang cmake libfontconfig-dev libssl-dev libvulkan1 libwayland-dev libx11-xcb-dev
libxkbcommon-x11-dev libzstd-dev pkg-config` for the app. [`docs/how-it-works.md`](docs/how-it-works.md)
describes how Claude Desktop's data is laid out and why the sync is safe.

## License and credits

Copyright (C) 2026 SlammingProgramming and Claude Companion contributors.

Licensed under the [GNU Affero General Public License, version 3 or later](LICENSE). Large parts
derive from MIT-licensed work whose notices are kept in [`LICENSES/`](LICENSES/); who did what is
set out in **[NOTICE.md](NOTICE.md)**:

- [cc-same](https://github.com/songkeys/cc-same) by Songkeys, the base of this fork;
- [claude-desktop-session-sync](https://github.com/RasmusKD/claude-desktop-session-sync) by RasmusKD;
- [claude-switcher](https://github.com/nyxon-tech/claude-switcher) by NeezerGu and Nyxon, and
  [claude-profile-switcher](https://github.com/NeezerGu/claude-profile-switcher) by NeezerGu before it;
- [claude-swap](https://github.com/realiti4/claude-swap) by realiti4, an inspiration.

Claude Companion is not affiliated with or endorsed by Anthropic. Claude is a trademark of Anthropic.
