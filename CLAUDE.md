# hoprs

Desktop SSH + SFTP + FTP/FTPS client in Rust (iced GUI), open source (MIT OR Apache-2.0). Replaces FileZilla for the author: same saved-sites idea, clean UI, high performance. Repo is private for now.

## Where things are

- Design spec (source of truth): `docs/superpowers/specs/2026-11-11-hoprs-design.md`
- Approved UI mockup (open in a browser): `docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`
- Plan roadmap (6 plans, one per subsystem): `docs/superpowers/plans/2026-11-11-00-roadmap.md`
- Plan 1 (foundation + transfer engine): `docs/superpowers/plans/2026-11-11-01-foundation-transfer-engine.md`

## Current status (2026-11-12)

- Plan 1 implemented on branch `plan-1-foundation` (hoprs-core, hoprs-remote, hoprs-transfer), every task reviewed plus a whole-branch review; follow-ups deferred to later plans are in `docs/superpowers/plans/2026-11-12-01-followups.md`.
- **Next step:** merge Plan 1, then write Plan 2 (SSH + SFTP) from the real signatures (`RemoteFs` contract in `crates/hoprs-remote/src/lib.rs`, `TransferEngine::register_site`, `pause_site`, `resume_site`).
- Plans 2–6 are written only after the previous plan is done, using the real signatures it produced.

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
