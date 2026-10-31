# Notices and credits

Claude Companion is free software, licensed under the **GNU Affero General Public License,
version 3 or (at your option) any later version** (`AGPL-3.0-or-later`). The full text is in
[`LICENSE`](LICENSE), also kept at [`LICENSES/AGPL-3.0-or-later.txt`](LICENSES/AGPL-3.0-or-later.txt).

Claude Companion is an independent project. It is **not affiliated with, endorsed by or sponsored
by Anthropic**. "Claude" and "Claude Code" are trademarks of Anthropic, PBC.

## How the licenses fit together

Claude Companion is a hard fork of [cc-same](https://github.com/songkeys/cc-same), and it
incorporates ideas, file-format knowledge and behaviour from two further MIT-licensed projects.
The MIT license permits this, and permits the combined work being distributed under the AGPL, on
one condition that stays in force for every copy: the original copyright and permission notice
must travel with the code. Those notices are reproduced, unmodified, in [`LICENSES/`](LICENSES/).

- Code written for Claude Companion is `Copyright (C) 2026 SlammingProgramming and Claude
  Companion contributors`, under `AGPL-3.0-or-later`.
- Code that still derives from cc-same, claude-desktop-session-sync or claude-switcher remains
  `Copyright (c)` its original authors, **and** is made available here under
  `AGPL-3.0-or-later` as part of the combined work. Anyone who takes only the original upstream
  code from its own repository still has it under the MIT license there; the AGPL applies to
  Claude Companion as a whole, including to its network use (AGPL section 13).
- Source files carry an SPDX header saying so. Files that were not changed from cc-same keep a
  note that they derive from it.

## Projects this work is based on or draws from

| Project | Author | License | What Claude Companion takes from it |
| --- | --- | --- | --- |
| [cc-same](https://github.com/songkeys/cc-same) | Songkeys | MIT, [`LICENSES/MIT-cc-same.txt`](LICENSES/MIT-cc-same.txt) | **The base of this fork**: the sync engine, snapshots and trash, background service, command-line tool, desktop app, translations, account list and (on macOS) account switching. Fork point: upstream commit `3d5183b` (v0.1.12). Full upstream history is kept in this repository's git log. |
| [claude-desktop-session-sync](https://github.com/RasmusKD/claude-desktop-session-sync) | RasmusKD | MIT, [`LICENSES/MIT-claude-desktop-session-sync.txt`](LICENSES/MIT-claude-desktop-session-sync.txt) | Knowledge of where Claude Desktop keeps its chat list, sidebar groups and scheduled tasks, and the design of safe three-way merging of groups and scheduled tasks, deletion stashing, account freezing, and backups. **`crates/core/src/groups.rs` is a Rust port of its sidebar-group merge** (`group-sync.mjs`): the Chromium Local Storage encoding, the three-way merge with tombstones and the pending-push marker, and the config-file splice. Studied at commit `5462c83`. |
| [claude-switcher](https://github.com/nyxon-tech/claude-switcher) | NeezerGu and Nyxon | MIT, [`LICENSES/MIT-claude-switcher.txt`](LICENSES/MIT-claude-switcher.txt) | The cross-platform approach to switching Claude Desktop logins, copying, moving and merging chats between accounts, recovering "lost" chats, journaled undo, usage statistics from local transcripts, a doctor check, and Markdown/web-page export. Studied at commit `53b7e53`. |
| [claude-profile-switcher](https://github.com/NeezerGu/claude-profile-switcher) | NeezerGu | MIT | The origin of claude-switcher, which worked out which of Claude Desktop's files make up a login and how to swap them safely. Credited by claude-switcher, and credited here for the same reason. |

Where Claude Companion re-implements a behaviour in Rust, it does so from reading the upstream
projects' documentation and source; the upstream authors' analysis of Claude Desktop's undocumented
internals is the foundation, and is theirs.

## Inspiration

- [claude-swap](https://github.com/realiti4/claude-swap) by realiti4: the numbered account list
  and the idea of choosing the next account by remaining plan usage, as already credited by
  cc-same. No code is taken from it.

## Libraries and frameworks

Claude Companion's desktop app is built with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
(Zed Industries, Apache-2.0) and Longbridge's [GPUI Kit](https://github.com/longbridge/gpui-kit),
and the engine and command-line tool with the Rust crates listed in `Cargo.lock`
(`Cargo.toml` of each crate names them). Each remains under its own license; all of them are
compatible with distribution inside an AGPL-3.0-or-later work. The sidebar-group sync reads and writes Claude's Local Storage with
[rusty-leveldb](https://github.com/dermesser/leveldb-rs) (Lewin Bormann, MIT), a Rust port of
LevelDB. A generated list of the exact
dependency licenses is in [`THIRD_PARTY.md`](THIRD_PARTY.md); each crate's own copyright notice
and license text travel with its source on crates.io.

## Artwork

The icon and tray artwork in this repository are inherited from cc-same (MIT) and are expected to
be replaced by Claude Companion's own artwork. Screenshots from cc-same have been removed from the
README for that reason.
