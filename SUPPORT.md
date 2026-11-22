# Getting help

Claude Companion is made in spare time, so answers can take a few days.

- **How do I…?** Start with the [README](README.md) and [`docs/how-it-works.md`](docs/how-it-works.md).
  `claude-companion doctor` shows what every account has and anything that needs attention.
- **Something is broken:** open a [bug report](https://github.com/slammingprogramming/claude-companion/issues/new?template=bug_report.yml).
  Say which system and which version (`claude-companion --version`), and leave out tokens, cookies,
  account emails and the contents of your Claude files.
- **An idea:** open a [feature request](https://github.com/slammingprogramming/claude-companion/issues/new?template=feature_request.yml).
- **A security problem:** do not open an issue; see [SECURITY.md](SECURITY.md).
- **Talk privately:** SimpleX, no account needed:
  [contact link](https://smp14.simplex.im/a#3gZ-zeHs4QrFZKLAN0o3SC_XQJXhj1eYBVTO_c0FAtg).

If something went wrong with your chats or sign-ins, **stop and do not delete anything**. Every
change is preceded by a snapshot: `claude-companion snapshots` lists them and
`claude-companion restore <id>` puts a session list back (quit Claude first). Deleted files wait in
the trash folder for 30 days.
