# hoprs

Desktop SSH + SFTP + FTP/FTPS client in Rust (iced GUI), open source (MIT OR Apache-2.0). Replaces FileZilla for the author: same saved-sites idea, clean UI, high performance. Repo is private for now.

## Where things are

- Design spec (source of truth): `docs/superpowers/specs/2026-11-11-hoprs-design.md`
- Approved UI mockup (open in a browser): `docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`
- Plan roadmap (6 plans, one per subsystem): `docs/superpowers/plans/2026-11-11-00-roadmap.md`
- Plan 1 (foundation + transfer engine): `docs/superpowers/plans/2026-11-11-01-foundation-transfer-engine.md`
- Plan 2 (SSH + SFTP): `docs/superpowers/plans/2026-11-12-02-ssh-sftp.md`
- Plan 3 (FTP + FTPS): `docs/superpowers/plans/2026-11-12-03-ftp-ftps.md`
- Deferred follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md`

## Current status (2026-11-12)

- Plans 1 (hoprs-core, hoprs-remote, hoprs-transfer) and 2 (hoprs-ssh, `SftpFs`, Docker OpenSSH test env) merged to `main`.
- Plan 3 (FTP + FTPS: `FtpFs`, `FtpSession` pool/keepalive/reconnect, explicit and implicit FTPS with certificate pinning, protocol-neutral `Connection` + `spawn_site_watcher`, Docker vsftpd test env) implemented on branch `plan-3-ftp-ftps`; every task reviewed, whole-branch review pending.
- Deferred follow-ups for later plans: `docs/superpowers/plans/2026-11-12-01-followups.md` (per plan: before Plan 5/6, perf, Windows, Later).
- **Next step:** merge Plan 3, then write Plan 4 (vault + FileZilla import) from the real signatures; it implements `FtpPrompter::store_pin` by saving the pin into the site profile (`crates/hoprs-remote/src/ftp/prompt.rs`).
- Plans 4–6 are written only after the previous plan is done, using the real signatures it produced.
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
