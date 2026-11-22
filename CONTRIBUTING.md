# Contributing to Claude Companion

Thanks for wanting to help. Bug reports, ideas, translations, documentation fixes and code are all
welcome. Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Before you start

- **Bugs and ideas:** open an [issue](https://github.com/slammingprogramming/claude-companion/issues/new/choose).
  For anything bigger than a small fix, please open an issue first so we can agree on the shape
  before you spend time on it.
- **Security problems:** do not open an issue; see [SECURITY.md](SECURITY.md).
- **Questions:** see [SUPPORT.md](SUPPORT.md).

## What the project will and will not do

Claude Companion moves and merges the files of Claude Desktop and Claude Code on your own
computer. Changes are judged against these rules:

1. **Never look inside a sign-in.** Sign-in files, tokens, cookies and keys are moved or copied as
   they are and never decoded, decrypted, refreshed, logged or sent anywhere.
2. **Stay inside Claude's data folders and the project's own state folder.** Never follow symlinks
   out of them, and never overwrite a file that appeared or changed after planning.
3. **Be undoable.** A snapshot or a backup comes before a change, writes are atomic, and deletions
   go to the trash folder.
4. **The command line and the app match.** A feature added to one side gets its counterpart in the
   other, or the README's side-by-side table says why it is on one side only.
5. **It works on Windows, macOS and Linux**, or says plainly which it does not.

## Setting up

You need Rust (the repository pins the toolchain in `rust-toolchain.toml`; the command line builds
on Rust 1.89 and later). The repository is two Cargo workspaces that share one `target` folder:
the engine and command line at the root, and the desktop app in `crates/app`.

```sh
cargo build                 # the engine and command line
cargo test                  # their tests
cargo app                   # run the desktop app
cargo app-test              # the app's tests
```

On Linux the app needs a few system libraries; `.github/workflows/ci.yml` lists them.

## Before you open a pull request

Run what CI runs, and fix what it says:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo fmt --manifest-path crates/app/Cargo.toml --all --check   # a separate workspace
cargo app-clippy -- -D warnings
cargo app-test
```

Plus:

- **Add a test** for what you changed; the engine's tests use temporary folders and stand-in data.
- **Do not test against your real Claude setup.** Point the program at throwaway folders with
  `CLAUDE_COMPANION_USER_DATA`, `CLAUDE_COMPANION_STATE_DIR`, `CLAUDE_COMPANION_DESKTOP_LOG_DIR`,
  `CLAUDE_COMPANION_PROJECTS` and `CLAUDE_CONFIG_DIR`. If you try something on a copy of real
  files, never put any of it in the repository, a test, a log or an issue.
- **Add a line to `CHANGELOG.md`** under `## Unreleased`, written for people who use the program,
  not for the code. Notes under `Unreleased` are left out of every build (the app carries only
  released sections) until the maintainer gives them a version number at release time.
- **Text the app shows goes in `crates/app/locales/app.yml`** in all ten languages (English,
  Simplified and Traditional Chinese, Japanese, Korean, French, German, Spanish, Brazilian
  Portuguese, Russian). Machine translations are fine as a start if you say so in the pull request.
- **Keep it small.** One change per pull request is easier to review and to take or leave.
- **Commits** need a clear message saying what and why. Signed commits are appreciated but not
  required from contributors.

## Licence

Claude Companion is licensed under `AGPL-3.0-or-later`, and some files derive from MIT-licensed
projects (see [NOTICE.md](NOTICE.md)). By sending a contribution you agree that it is licensed the
same way, and that you have the right to send it. Files taken from another project keep that
project's copyright header and need an entry in `NOTICE.md`.

## Sending changes upstream

This project is a hard fork of [cc-same](https://github.com/songkeys/cc-same). Fixes that also
apply there are best offered there too; a change that only exists here (Claude Code, MCP servers,
Windows and Linux account switching and the rest of what the README lists) belongs in this
repository.
