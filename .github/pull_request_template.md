## What and why

<!-- What changes, and what problem it solves. Link the issue: "Closes #123". -->

## Checks

- [ ] `cargo fmt --all --check` and `cargo fmt --manifest-path crates/app/Cargo.toml --all --check`
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo app-clippy -- -D warnings`
- [ ] `cargo test` and `cargo app-test`
- [ ] I added or changed a test for this
- [ ] The command line and the app match, or the README says why not
- [ ] New app text is in all ten languages in `crates/app/locales/app.yml`
- [ ] I added a line to `CHANGELOG.md` under `## Unreleased`
- [ ] I did not test on my real Claude setup, and nothing personal is in the code, tests or logs
- [ ] Nothing reads inside a sign-in (see CONTRIBUTING.md)

## Tried on

<!-- Windows / macOS / Linux, and what you actually ran. Say what you could not try. -->
