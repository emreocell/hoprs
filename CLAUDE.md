# hoprs

Desktop SSH + SFTP + FTP/FTPS client in Rust (iced GUI), open source (MIT OR Apache-2.0). Replaces FileZilla for the author: same saved-sites idea, clean UI, high performance. Repo is private for now.

## Where things are

- Design spec (source of truth): `docs/superpowers/specs/2026-11-11-hoprs-design.md`
- Approved UI mockup (open in a browser): `docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`
- Plan roadmap (6 plans, one per subsystem): `docs/superpowers/plans/2026-11-11-00-roadmap.md`
- Plan 1 (foundation + transfer engine): `docs/superpowers/plans/2026-11-11-01-foundation-transfer-engine.md`
- Plan 2 (SSH + SFTP): `docs/superpowers/plans/2026-11-12-02-ssh-sftp.md`
- Plan 3 (FTP + FTPS): `docs/superpowers/plans/2026-11-12-03-ftp-ftps.md`
- Plan 4 (vault + FileZilla import): `docs/superpowers/plans/2026-11-12-04-vault-import.md`
- Plan 5 (iced app shell): `docs/superpowers/plans/2026-11-12-05-app-shell.md`
- Plan 5 visual/manual QA checklist: `docs/qa/plan-5-checklist.md`
- Deferred follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md`

## Current status (2026-11-13)

- Plans 1 (hoprs-core, hoprs-remote, hoprs-transfer), 2 (hoprs-ssh, `SftpFs`, Docker OpenSSH test env), 3 (FTP + FTPS: `FtpFs`, `FtpSession` pool/keepalive/reconnect, certificate pinning, protocol-neutral `Connection` + `spawn_site_watcher`, Docker vsftpd test env), 4 (hoprs-vault: profiles TOML, keyring secrets, encrypted export/import; hoprs-import: FileZilla `sitemanager.xml`) and 5 (hoprs-app, PR #5) merged to `main`.
- Plan 5 (hoprs-app: iced shell, sidebar, site/workspace forms, file panes with a virtual list, transfer queue, security dialogs, reconnect/status, tab persistence and shutdown, FileZilla import, settings and backup) merged; QA checklist `docs/qa/plan-5-checklist.md` still has NEEDS HUMAN rows for the author.
- Deferred follow-ups for later plans: `docs/superpowers/plans/2026-11-12-01-followups.md` (per plan; Plan 5 section: correctness, UX, perf, Windows, later).
- **Next step:** write Plan 6 (terminal tab, remote editing, Ctrl+K palette, technical log, spec §9 benchmarks in CI) from the real signatures.
- Plan 6 is written only after Plan 5 is done, using the real signatures it produced.
- Docker integration tests: `docker compose -f docker/compose.yml up -d --build --wait`, then `cargo test --workspace --all-features -- --ignored --test-threads=1`.

## Decisions already made (do not re-litigate)

- Target platform v1: **Linux (Ubuntu)**. Code stays portable; tests must also pass on Windows (use `tempfile`, never literal `/tmp`).
- GUI: **iced**. Runtime: **tokio**. SSH/SFTP: `russh` + `russh-sftp`. FTP/FTPS: `suppaftp`. Terminal: `alacritty_terminal`.
- Sites have a protocol: `ssh` (opens terminal, files via SFTP, or both in tabs, over one connection) or `ftp` / `ftps-explicit` / `ftps-implicit` (files). FTP is first-class, not an afterthought.
- **Workspace = a bundle of sites**; "Open workspace" connects all of them and restores tabs. A site can be in several workspaces.
- Credentials: OS keyring (Secret Service) + encrypted export/import (Argon2id + XChaCha20-Poly1305). Profile TOML never holds secrets. ssh-agent preferred.
- Import from FileZilla (`sitemanager.xml`) is in v1.
- UI language: **English** (i18n-ready). Visual: mockup v2 — neutral near-black, **white accent**, Inter for UI, JetBrains Mono for data (file names, paths, permissions), 4px radius, 1px rules, no shadows, no modal cascades (errors inline).
- Unencrypted FTP is allowed, labelled `ftp (unencrypted)`, no nagging.

## Conventions

- Cargo workspace under `crates/`; only `hoprs-app` may depend on iced. Dependency graph is fixed in the spec §4.1.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` must be clean on every commit.
- Conversation with the author is in Portuguese (pt-BR); code, identifiers, UI strings and commit messages in English.
