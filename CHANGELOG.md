# Changelog

## 1.0.0 — 2026-11-14

First release.

- Transfer engine: persistent queue (SQLite), parallel transfers, resume after a drop or a restart, conflict handling (overwrite, skip, rename, resume) per item or for the whole batch, size and time checks, throttled progress.
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
