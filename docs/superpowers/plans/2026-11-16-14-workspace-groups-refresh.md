# Plan 14: workspaces as groups, refresh after transfers (1.3.1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Remove the "Open workspace" button from the expanded workspace (Open all / Close
all move to its right-click menu and the palette) and re-list a pane's folder automatically
when a transfer into it finishes.

**Architecture:** `hoprs-app` only. Workspaces: view change in `ui/sidebar.rs`, menu items in
`app/menu.rs`, strings. Refresh: `app/transfers.rs::transfer_event` learns the finished
item's destination from the queue model, then schedules a quiet re-list per matching pane
through the same quiet-listing path file operations already use, with per-pane batching.

**Spec:** `docs/superpowers/specs/2026-11-16-1.3.1-workspace-and-refresh-design.md`

## Global Constraints

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean on every commit; `cargo test --workspace` green; no literal `/tmp`.
- Only `hoprs-app` changes; no new dependency.
- Every user-visible text in `crates/hoprs-app/i18n/en.ftl` and `pt-BR.ftl`; unused keys removed (`every_en_key_is_used`).
- Commits: `git -c user.name="$(git log -1 --format=%an)" -c user.email="$(git log -1 --format=%ae)" commit …`, message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A burst of 1 000 small files finishing into the shown folder: one listing per ~500 ms per pane, never a listing storm on the FTP control connection.
2. A transfer into a folder the user has since left: no re-list of the new folder, none of the old.
3. The same remote folder open in two tabs of the same site: both re-list; a pane of a different site with the same path does not.
4. A re-list while the user has a rename editor open or rows selected: the editor and the selection survive (or the re-list waits) — never lose typed text.
5. Workspace right-click "Close all" on a workspace that is not the open one: absent or disabled, never closes another workspace's tabs.

---

### Task 1: Workspaces as groups

**Files:** `crates/hoprs-app/src/ui/sidebar.rs` (`workspace`), `crates/hoprs-app/src/app/menu.rs` (`MenuTarget::Workspace` items), `crates/hoprs-app/src/app/palette.rs` (wording only if needed), `ui/strings.rs`, both `.ftl`, `README.md` ("Using hoprs"), `CHANGELOG.md` (new `## 1.3.1 — unreleased` section).

- [ ] **Step 1: Failing tests.** In `app/tests/menu.rs`: `workspace_menu_has_open_all_close_all_edit` (closed workspace → `[OpenAll, Edit]`; the open workspace → `[OpenAll, CloseAll, Edit]`, choosing them yields `Message::OpenWorkspace(id)` / `Message::CloseWorkspace(id)`). In `ui/sidebar.rs` tests: a pure helper that lists the expanded workspace's controls (or equivalent) shows no open/close button.
- [ ] **Step 2: Implement.** Drop `open_button`/actions block from the expanded workspace; rename the menu items (`menu-open-all`, `menu-close-all`); keep the palette entries working; remove now-unused string keys.
- [ ] **Step 3: Run** tests, fmt, clippy. **Step 4:** README + CHANGELOG lines. **Step 5: Commit** `feat(app): workspaces are groups; Open all moves to the menu`.

### Task 2: Refresh after transfers

**Files:** `crates/hoprs-app/src/app/transfers.rs` (`transfer_event`, `queue_snapshot`), the pane listing code in `crates/hoprs-app/src/app/files.rs` (reuse the quiet re-list used after file operations), `crates/hoprs-app/src/app.rs` (a batching timer message if needed), tests in `crates/hoprs-app/src/app/tests/transfers.rs`.

**Interfaces:**
- `App::refresh_after_transfer(&mut self, id: TransferId) -> Vec<Effect>`: looks the item up in the queue model (destination path, side, site), computes the destination folder (parent of the destination path), and marks every matching pane (same side, same site for remote, `current folder == that folder`, pane idle, site connected) for a quiet re-list.
- Batching: a per-pane "re-list pending" flag plus one deferred tick (~500 ms, via the app's existing delayed-message mechanism, e.g. how compare debounces) that re-lists every pending pane once; a pane with a listing in flight keeps its flag and is re-listed after that listing completes.

- [ ] **Step 1: Failing tests:** `done_item_relists_the_pane_showing_its_folder`; `other_folder_other_side_other_site_not_relisted`; `failed_cancelled_skipped_do_not_relist`; `burst_of_done_items_gives_one_listing`; `listing_in_flight_defers_the_relist`; `relist_keeps_selection_and_rename_editor` (Review Focus 4); `same_remote_folder_in_two_tabs_both_relist`.
- [ ] **Step 2: Implement** (only `TransferState::Done` triggers; snapshots after a lag also trigger for items that became Done unseen).
- [ ] **Step 3: Run** tests, fmt, clippy; the 100k-row bench once (no regression expected). **Step 4:** CHANGELOG line; QA row in a new `docs/qa/release-1.3.1-checklist.md` (NEEDS HUMAN: upload, download and drag show the file without Refresh, on SFTP and FTP). **Step 5: Commit** `feat(app): re-list the destination folder when a transfer finishes`.

### Task 3: Release 1.3.1 prep (controller, after review)

Version 1.3.1 in `Cargo.toml`/`Cargo.lock`, README version/assets, `site/src/facts.ts` (releaseDate), CHANGELOG date, CLAUDE.md status; PR "Release 1.3.1"; after the author approves the merge: tag `v1.3.1`, verify assets and checksums, site.
