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
- Plan 6 (terminal, remote editing, palette, log, perf job): `docs/superpowers/plans/2026-11-13-06-terminal-polish.md`
- QA checklists (manual runs, NEEDS HUMAN rows with steps): `docs/qa/plan-5-checklist.md`, `docs/qa/plan-6-checklist.md`
- Performance targets, how they are measured, last numbers: `docs/perf/README.md` (`scripts/perf.sh`, `.github/workflows/perf.yml`)
- Deferred follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md`

## Current status (2026-11-13)

- Plans 1–6 merged to `main` (v1 feature-complete): core + transfer engine, SSH/SFTP, FTP/FTPS, vault + FileZilla import, the iced app, and Plan 6 (terminal tabs over the site's SSH connection, remote editing with re-upload on save, Ctrl+K palette, technical log panel, idle-drop fixes, perf job).
- `perf.yml` first ran green on `main` (2026-11-13): startup 84 ms, idle 33 MB, 100k rows p95 0.58 ms, SFTP 1 GiB 0.99x `scp`; small files are known gaps (FTP 1.003x, SFTP 2.0x `lftp`). Numbers and method: `docs/perf/README.md`.
- QA checklists `docs/qa/plan-5-checklist.md` and `docs/qa/plan-6-checklist.md` still have NEEDS HUMAN rows for the author (real input devices, a GUI editor, a 10-minute idle on the author's FTP server).
- Deferred follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md` (per plan; Plan 6 section first item: two queued items can write the same destination at once).
- **Next step:** prepare the v1 release (packaging, icon, `.desktop` file, version), and work the follow-ups starting with the same-destination race.
- Docker integration tests: `docker compose -f docker/compose.yml up -d --build --wait`, then `cargo test --workspace --all-features -- --ignored --test-threads=1`.

## Decisions already made (do not re-litigate)

- Target platform v1: **Linux (Ubuntu)**. Code stays portable; tests must also pass on Windows (use `tempfile`, never literal `/tmp`).
- GUI: **iced**. Runtime: **tokio**. SSH/SFTP: `russh` + `russh-sftp`. FTP/FTPS: `suppaftp`. Terminal: `alacritty_terminal`. File watching (remote editing): `notify`.
- Sites have a protocol: `ssh` (opens terminal, files via SFTP, or both in tabs, over one connection) or `ftp` / `ftps-explicit` / `ftps-implicit` (files). FTP is first-class, not an afterthought.
- **Workspace = a bundle of sites**; "Open workspace" connects all of them and restores tabs. A site can be in several workspaces.
- Credentials: OS keyring (Secret Service) + encrypted export/import (Argon2id + XChaCha20-Poly1305). Profile TOML never holds secrets. ssh-agent preferred.
- Import from FileZilla (`sitemanager.xml`) is in v1.
- UI language: **English** (i18n-ready). Visual: mockup v2 — neutral near-black, **white accent**, Inter for UI, JetBrains Mono for data (file names, paths, permissions), 4px radius, 1px rules, no shadows, no modal cascades (errors inline).
- Unencrypted FTP is allowed, labelled `ftp (unencrypted)`, no nagging.
- Terminal: Ctrl+K goes to the shell; the palette opens with Ctrl+Shift+K there. Copy/paste are Ctrl+Shift+C/V. OSC 52 (server writing the clipboard) is off.
- Remote-edit uploads go straight through `RemoteFs`, not the transfer queue. The perf job is separate and never a required PR check.

## Conventions

- Cargo workspace under `crates/`; only `hoprs-app` may depend on iced. Dependency graph is fixed in the spec §4.1.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` must be clean on every commit.
- Conversation with the author is in Portuguese (pt-BR); code, identifiers, UI strings and commit messages in English.
