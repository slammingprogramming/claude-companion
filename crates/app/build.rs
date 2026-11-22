// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Songkeys (cc-same, MIT)
// Copyright (C) 2026 SlammingProgramming and Claude Companion contributors
// Derived from cc-same under the MIT license; see NOTICE.md and LICENSES/.
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    embed_released_changelog();
    // rust-i18n reads the messages while compiling; without this, editing them alone rebuilds nothing.
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-env-changed=CLAUDE_COMPANION_SKIP_RESOURCES");
    // `CLAUDE_COMPANION_SKIP_RESOURCES=1` lets `cargo check` run for Windows from another OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CLAUDE_COMPANION_SKIP_RESOURCES").is_none()
    {
        embed_windows_resources();
    }
}

/// The changelog the app carries: every released version's section, and nothing under
/// `## Unreleased` (or `## [Unreleased]`), which stays out of every build until it is given a
/// version number. Written only when it differs, so editing the unreleased notes rebuilds nothing.
fn embed_released_changelog() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("CHANGELOG.md");
    println!("cargo:rerun-if-changed={}", source.display());
    let text = std::fs::read_to_string(&source).expect("read CHANGELOG.md");
    let mut released = String::with_capacity(text.len());
    let mut unreleased = false;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let name = heading.trim().trim_matches(['[', ']']).to_ascii_lowercase();
            unreleased = name == "unreleased";
        }
        if !unreleased {
            released.push_str(line);
            released.push('\n');
        }
    }
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("changelog.md");
    if std::fs::read_to_string(&out).ok().as_deref() != Some(released.as_str()) {
        std::fs::write(&out, released).expect("write the released changelog");
    }
}

/// The icon and version details of the `.exe`. The application manifest (Common Controls 6,
/// per-monitor DPI) comes from GPUI itself; a second one would clash at link time.
fn embed_windows_resources() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("windows");
    let icon = dir.join("claude-companion.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let esc = |p: &Path| p.to_string_lossy().replace('\\', "\\\\");
    let version = env!("CARGO_PKG_VERSION");
    let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0)).chain(std::iter::repeat(0));
    let numeric = format!(
        "{},{},{},{}",
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap()
    );
    let icon_line = if icon.exists() { format!("1 ICON \"{}\"\n", esc(&icon)) } else { String::new() };
    let rc = format!(
        r#"{icon_line}
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "Claude Companion\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "InternalName", "claude-companion-app\0"
            VALUE "OriginalFilename", "Claude Companion.exe\0"
            VALUE "ProductName", "Claude Companion\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#,
    );
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("claude-companion.rc");
    std::fs::write(&out, rc).expect("write the Windows resource script");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("compile Windows resources (rc.exe or llvm-rc is required)");
}
