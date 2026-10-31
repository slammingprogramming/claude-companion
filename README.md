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
- A numbered account list with aliases and plan usage, and **account switching on macOS**.
- **New in this fork:** account switching on **Windows and Linux** (including ending Claude from
  the app or with `--force` where Windows keeps it in the tray). It is tested against stand-in
  data and compiles on all three systems, but it has not yet been run against a real Claude
  Desktop on Windows or Linux.
- **New in this fork:** lost-chat rescue, Markdown/HTML export, token statistics and diagnostics,
  in the app (Settings › Tools) and on the command line; copying chats between accounts on the
  command line. The app's new screens have not yet been checked by eye on every system.
- **New in this fork:** one set of **sidebar groups** for every account (off until you turn it on:
  `claude-companion config --groups on`, or Settings › Sync), re-implemented from
  claude-desktop-session-sync. It runs only while Claude is closed, backs up Claude's Local Storage
  first, and has been tested against stand-in databases only, never against a real Claude profile.
  See [how it works](docs/how-it-works.md#sidebar-groups).
- Ten interface languages, light and dark themes, and update checks against this repository's
  releases.

## Roadmap

1. **GUI and CLI parity.** Every feature reachable from one is reachable from the other. Done
   except `chats copy` in the app, which is aimed at accounts kept out of sync.
2. **Windows, macOS and Linux, fully.** Account switching, quitting and starting Claude Desktop
   now exist on all three (see above); real-world testing on Windows and Linux, and background
   sync checks on each, remain.
3. **Rename and relicense.** Done: the project is Claude Companion, `AGPL-3.0-or-later`.
4. **Features from two sibling projects**, re-implemented in Rust and credited:
   - [claude-desktop-session-sync](https://github.com/RasmusKD/claude-desktop-session-sync):
     sidebar groups: done (opt-in, see above). Its three-way scheduled-task merging, deletion
     stash and backups were already in cc-same (tasks merged three ways, a 30-day trash,
     snapshots), and its account freezing is what `config --exclude` / the account list's
     include switch do here, so nothing else is missing.
   - [claude-switcher](https://github.com/nyxon-tech/claude-switcher): recover lost chats, copy
     and merge chats between accounts, token statistics, export (command line done; app screens
     and a doctor check remain). Undo uses sync's snapshots (`restore`) instead of a journal, and
     there is no *move*, which sync's deletions would turn into deleting the chat everywhere.
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
claude-companion switch [<who>] switch Claude to an account, or the next one (--force on Windows)
claude-companion add            restart Claude signed out, to add an account (--force on Windows)
claude-companion remove <who>   take an account off the list, forgetting its saved sign-in
claude-companion alias <who> <name>, disable <who>, enable <who>, move <who> <number>
claude-companion chats list     every chat of every account (--account, --json)
claude-companion chats lost     conversations on disk that no account shows
claude-companion chats rescue   give lost chats a card again (--to <account>)
claude-companion chats copy     put chats into another account's list (--from, --to)
claude-companion chats export   a chat as Markdown or a web page (--format md|html)
claude-companion stats          tokens by model, day and folder, from local transcripts
claude-companion groups         merge the sidebar groups of all accounts (--dry-run, --json)
claude-companion config         --exclude <account-id>, --auto-join off, --groups on, …
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
- Sidebar groups touch Claude's Local Storage, so they wait until Claude is closed, copy the
  database first, check what they wrote reads back, and put the copy back if it does not.

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
