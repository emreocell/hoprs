# Changelog

## 1.1.0 — 2026-11-14

hoprs now ships for macOS and Windows as well as Linux.

- macOS: a universal `.dmg` (Apple Silicon and Intel, macOS 11 or newer), ad-hoc signed only. The first launch needs "Open Anyway" in System Settings, Privacy & Security (or `xattr -dr com.apple.quarantine /Applications/hoprs.app`). Passwords live in the Keychain; after each update macOS asks for access again, choose "Always Allow".
- Windows: an `.msi` (per machine, Start menu and desktop shortcut) and a portable `.zip` (x64). The C runtime is linked statically, so no Visual C++ redistributable is needed. On "Windows protected your PC" choose "More info", then "Run anyway". Passwords live in the Credential Manager.
- No console window on Windows; `hoprs --version` and `--help` still print into the cmd or PowerShell window that ran them.
- ssh-agent on Windows through the OpenSSH agent pipe (`\\.\pipe\openssh-ssh-agent`); Pageant is not supported.
- Remote editing: the default editor `system` opens files with `xdg-open` on Linux, `open -t` on macOS and the associated app on Windows (the Open With dialog if there is none). On Windows a custom editor must be a windowed program, for example `code.cmd --wait`.
- Command palette: Cmd+K on macOS (Ctrl+K elsewhere); in a focused terminal Ctrl+Shift+K everywhere.
- FileZilla import also reads `~/.config/filezilla` on macOS and `%APPDATA%\FileZilla` on Windows.
- Your data lives in `~/Library/Application Support/hoprs` on macOS and `%APPDATA%\hoprs` on Windows. Uninstalling never removes it or the saved passwords.
- Release workflow: builds on three runners and publishes all or nothing; a manual run is a dry run. The website's download buttons follow the visitor's system and go live only once the release is published.

## 1.0.0 — 2026-11-14

First release.

- Transfer engine: persistent queue (SQLite), parallel transfers, resume after a drop or a restart, conflict handling (ask, overwrite, overwrite if newer, skip, resume) per item or for the whole batch, size and time checks, throttled progress.
- SSH and SFTP: agent, key, password and keyboard-interactive authentication, `known_hosts` verification, keepalive and automatic reconnection with backoff, pipelined SFTP.
- FTP, FTPS explicit and FTPS implicit: connection pool, active and passive modes, PASV workaround for NAT, pinned server certificates, keepalive and automatic reconnection. Unencrypted FTP is labelled as such.
- Saved sites and workspaces: a workspace is a bundle of sites; opening it connects them all and restores its tabs. Profiles are plain TOML without secrets.
- Credentials in the system keyring (Secret Service); encrypted backup export and import (Argon2id and XChaCha20-Poly1305).
- Import of FileZilla `sitemanager.xml`, with a report of what was imported and what was skipped.
- Desktop app: sidebar, tabs, dual-pane file browser with virtualised lists (100 000 entries), transfer queue with progress, and inline error and security dialogs (host key and certificate changes) instead of modal cascades.
- Terminal for SSH sites: `xterm-256color` with truecolor, scrollback, copy and paste, configurable font size, reconnect bar. Terminal and files tabs share one connection.
- Remote editing: open a remote file in your editor, upload on save, with a conflict strip if the file changed on the server.
- Command palette (Ctrl+K) to jump to sites, workspaces and actions.
- Technical log view and `RUST_LOG` support.
- Idle-drop handling: connections that the server closes while idle are reconnected on next use.
- Two queue items never write the same destination file at once; the second waits and then goes through the normal conflict check.
- `hoprs --version` and `--help`; desktop entry, AppStream metadata and app icon for Linux.
