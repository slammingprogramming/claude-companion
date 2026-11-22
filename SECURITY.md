# Security policy

Claude Companion works next to the sign-in and chat files of Claude Desktop and Claude Code, so
security reports are taken seriously. It is a spare-time project, so replies can take a few days.

## Supported versions

Only the latest release gets fixes. If you are on an older one, update first and check that the
problem is still there.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.** Use one of these instead:

- **GitHub private report:** the repository's **Security** tab, then **Report a vulnerability**.
- **SimpleX** (private, no account needed):
  [contact link](https://smp14.simplex.im/a#3gZ-zeHs4QrFZKLAN0o3SC_XQJXhj1eYBVTO_c0FAtg).

Include what you can of: the version (`claude-companion --version`) and system, what you did,
what you expected and what happened, and whether it needs a special setup. Please leave out real
tokens, cookies, account emails and the contents of your Claude files: a description or a
redacted example is enough.

You will get an acknowledgement, and then updates until it is fixed or judged not to be a
vulnerability. Fixes are released as a new version with a note in the changelog; if you would like
credit, say how you want to be named.

## What counts

In scope: anything that makes Claude Companion do more than it says it does, for example

- decoding, logging or sending a sign-in, token, cookie or key (it is meant to move or copy
  Claude's own files between folders without looking inside them);
- writing outside Claude's data folder and its own state folder, following a symlink out of them,
  or overwriting a file it did not plan to change;
- losing or corrupting chats, sign-ins or settings, or leaving them half-changed after a failed
  switch;
- a way for another program or a web page to make it run a command or change files;
- a problem in how the release builds or the update check install a file.

Out of scope: weaknesses in Claude Desktop, Claude Code or Anthropic's services themselves (report
those to Anthropic), problems that need someone who already has full control of your account or
computer, and missing hardening that has no way to be used. There is no bug bounty.

## How it is meant to behave

- Nothing is decrypted, read, refreshed or sent anywhere: sign-in files are moved between folders
  and put back as they were. The only network use is the desktop app's update check against this
  repository's releases.
- Every location can be redirected (see the Safety section of the README), which is how the tests
  and any dry run stay away from a real Claude setup.
- Writes are atomic, never follow symlinks and are preceded by a snapshot; deleted files go to a
  trash folder for 30 days.

Dependencies are watched by Dependabot, and the code is scanned by CodeQL.
