# Plan 12: folder compare — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A file tab compares its local and remote folders recursively (automatically by default, or on demand), shows only the differences in a panel, and lets the user upload or download selected items through the queue.

**Architecture:** The walk and diff are a pure-async module `hoprs_remote::compare` over two `RemoteFs`. The app backend runs one comparison per tab with a progress channel and cancellation; `App` holds a per-tab compare state driven by messages; a new panel view sits between the panes and the queue. The transfer engine gains a per-item conflict policy so panel transfers overwrite.

**Tech Stack:** Rust 1.99.1, tokio, iced 0.14, existing crates (no new dependencies).

**Spec:** `docs/superpowers/specs/2026-11-15-folder-compare-design.md` (including its "Builder-check rulings" section, which is binding).

## Global Constraints

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean on every commit; `cargo test --workspace --all-features` green; tests portable to Linux, macOS, Windows (`tempfile`, no literal `/tmp`, no `/`-separator or LF assumptions).
- No new crate edges (graph in `docs/ARCHITECTURE.md`); no new dependencies.
- Every user-visible text in both `crates/hoprs-app/i18n/en.ftl` and `pt-BR.ftl`, looked up from `ui/strings.rs` with literal keys (`tr("…")` / `tr_args("…", …)`); new hoprs-written errors use `Error::coded` with `error-*` keys; views show errors through `strings::error`. pt-BR natural for Brazilian developers, product terms (upload, download, workspace, host…) in English, sentence case.
- Values: mtime tolerance `max(2 s, precision of either side)`; FTP `LIST` precision 60 s, default precision 1 s; limit 10 000 entries per side (ignored excluded); debounce 500 ms; SFTP listing concurrency 4, FTP 1 (and the local walk off the UI thread); default ignore list `.git, node_modules, .DS_Store`; `*` in ignore patterns matches any run of characters, matched on the entry name.
- Settings keys: `compare_auto` (bool, default `true`), `compare_ignore` (string list, default above); unknown/invalid values read as the default and unknown keys are kept on save (existing settings rules).
- Panel transfers of files use `ConflictPolicy::Overwrite` per item (persisted); folders that exist on one side only use the normal policy.
- Nothing is transferred or deleted without a click; the comparison never opens its own connection.
- Commits: `git -c user.name="$(git log -1 --format=%an)" -c user.email="$(git log -1 --format=%ae)" commit …`, message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never change git config.

## Review Focus

1. **A pane listing while a compare walks an FTP tree.** Expected: the pane lists after at most one in-flight compare listing, never after the whole walk. Test in Task 1 (one-at-a-time double) and Task 5 (app: a pane `List` effect is not blocked by the compare effect).
2. **Fast navigation with automatic on.** Expected: only the last folder pair is compared; earlier runs are cancelled and their late results dropped (by run id). Test in Task 5.
3. **A file just uploaded to a server that cannot set mtimes.** Expected: shown identical on the next run (ruling 1). Test in Task 7.
4. **The connection drops mid-run / the tab closes mid-run.** Expected: the run stops, no panic, the panel says it stopped with "Compare again" (drop) or simply goes away (close). Test in Task 5.
5. **A subfolder that cannot be listed (permission denied).** Expected: one "could not read" row with the translated error; the rest of the tree is still compared. Test in Task 1 and shown in Task 6.

---

### Task 1: The compare module

**Files:**
- Create: `crates/hoprs-remote/src/compare.rs` (and `mod compare; pub use` in `lib.rs`)
- Modify: `crates/hoprs-remote/src/lib.rs` (`RemoteFs::mtime_precision`), `crates/hoprs-remote/src/ftp/mod.rs` (60 s when listing with `LIST`)

**Interfaces — Produces:**
- `RemoteFs::mtime_precision(&self) -> Duration` (default `Duration::from_secs(1)`; FTP: 60 s when `MLSD` is not in use).
- `pub struct CompareOptions { pub ignore: Vec<String>, pub limit: Option<usize>, pub concurrency_a: usize, pub concurrency_b: usize, pub known_synced: HashMap<RelPath, (u64, u64)> }` (`known_synced`: relative path → (local size, remote size) from ruling 1).
- `pub type RelPath = String` (relative, `/`-separated, no leading slash).
- `pub enum Status { OnlyLocal, OnlyRemote, LocalNewer, RemoteNewer, SizeDiffers, KindDiffers, Unreadable { side: Side2, error: Error } }` and `pub enum Side2 { Local, Remote }` (name it to avoid clashing with app's `Side`; builder may pick a better name).
- `pub struct Difference { pub path: RelPath, pub kind: EntryKind, pub status: Status, pub local: Option<RemoteEntry>, pub remote: Option<RemoteEntry>, pub entries_inside: Option<u64> }`.
- `pub enum Progress { Seen { local: usize, remote: usize }, Found(Difference), LimitReached { local: usize, remote: usize } }`.
- `pub async fn compare(local: Arc<dyn RemoteFs>, local_root: RemotePath, remote: Arc<dyn RemoteFs>, remote_root: RemotePath, options: CompareOptions, progress: mpsc::UnboundedSender<Progress>, resume: watch::Receiver<bool>) -> Result<Summary>` where `Summary { identical: u64, differences: usize, partial: bool }`. On `LimitReached` the walk waits until `resume` turns `true` (continue without limit). **Cancellation is dropping the future** (no new dependency such as tokio-util in hoprs-remote): nothing more is listed once it is dropped. Errors listing a root fail the call; errors listing a subfolder become `Unreadable` differences.
- `pub fn ignored(name: &str, patterns: &[String]) -> bool`.

- [ ] **Step 1: Failing tests** in `compare.rs` with `MemFs` on both sides (and one with `LocalFs` on a `tempfile` dir): `only_local_and_only_remote`; `size_differs`; `local_newer` / `remote_newer` beyond 2 s; `within_tolerance_is_identical` (1.5 s); `minute_precision_widens_tolerance` (a `MemFs` wrapper reporting 60 s precision, 45 s apart → identical); `missing_mtime_compares_size_only`; `one_sided_folder_reported_once_not_walked` (a listing counter shows it was never listed); `file_vs_folder_is_kind_differs`; `symlink_by_name_only`; `ignored_at_depth_and_wildcard` (`node_modules` deep, `*.log`); `limit_pauses_then_continues` (limit 3, `LimitReached` sent, nothing more listed until `resume=true`, then completes with `partial=false`); `dropping_the_future_stops_listing`; `unreadable_subfolder_reported_and_walk_continues`; `one_listing_at_a_time_with_concurrency_1` (a `RemoteFs` double that fails if two `list` calls overlap); `known_synced_mtime_only_counts_identical` (sizes match `known_synced` → identical despite mtime; sizes changed → different); `ignored()` unit cases (`*.log`, `a*b`, exact, no match).
- [ ] **Step 2: Run** `cargo test -p hoprs-remote compare` — fails.
- [ ] **Step 3: Implement.** Walk both trees level by level from the roots, pairing children by name; per side a semaphore of `concurrency_*` permits around `list`; send `Found` as soon as a difference is known and `Seen` counts at least every 200 entries; a tolerance of `max(2 s, local.mtime_precision(), remote.mtime_precision())`. `FtpFs::mtime_precision` returns 60 s when the session lists with `LIST`.
- [ ] **Step 4: Run** tests, fmt, clippy — pass.
- [ ] **Step 5: Commit** `feat(remote): recursive folder compare`.

### Task 2: Per-item conflict policy in the engine

**Files:** `crates/hoprs-transfer/src/engine/mod.rs`, `engine/actor.rs` / `control.rs` as needed, `crates/hoprs-transfer/src/store.rs` (only if `next_policy` needs a setter at enqueue), `crates/hoprs-app/src/backend/mod.rs` + `backend/fake.rs` (`BackendApi::enqueue_with_policy`).

**Interfaces — Produces:** `TransferEngine::enqueue_with_policy(&self, spec: TransferSpec, policy: ConflictPolicy) -> Result<TransferId>` (stores `policy` as the item's `next_policy` so it survives a restart); `BackendApi::enqueue_with_policy(spec, policy) -> Result<TransferId>`; `FakeBackend` records `(spec, Some(policy))`.

- [ ] **Step 1: Failing tests:** `enqueue_with_overwrite_replaces_existing_without_asking` (global policy `Ask`, destination exists → item completes, no `Conflict` event); `per_item_policy_survives_restart` (enqueue, shut down before it runs, restart engine on the same store → still overwrites); existing `enqueue` keeps using the global policy.
- [ ] **Step 2–4:** run (fail), implement, run tests/fmt/clippy.
- [ ] **Step 5: Commit** `feat(transfer): per-item conflict policy at enqueue`.

### Task 3: Settings

**Files:** `crates/hoprs-app/src/settings.rs`, `app/settings.rs`, `ui/settings.rs`, `ui/strings.rs`, both `.ftl`.

**Interfaces — Produces:** `Settings::compare_auto: bool`, `Settings::compare_ignore: Vec<String>`; `SettingsMsg::CompareAuto(bool)`, `SettingsMsg::CompareIgnore(String)` (comma-separated as typed; trimmed, empty items dropped; saved when it parses); a "Folder compare" group in the settings page with a toggle and a text field (placeholder shows the default).

- [ ] **Step 1: Failing tests:** both keys round-trip; defaults when missing; invalid types read as defaults and unknown keys kept; `CompareIgnore(" a, ,*.log ")` → `["a", "*.log"]`; app test: the messages update state and emit the save effect.
- [ ] **Step 2–4:** run (fail), implement, run tests/fmt/clippy.
- [ ] **Step 5: Commit** `feat(app): folder compare settings`.

### Task 4: Running a comparison from the backend

**Files:** `crates/hoprs-app/src/backend/mod.rs` (new `BackendApi::compare`), `backend/fake.rs`, `crates/hoprs-app/src/effects.rs`, `crates/hoprs-app/src/app.rs` (`Effect::Compare`, `Effect::CompareResume`, `Effect::CompareCancel`, `Message::Compare(CompareMsg)`).

**Interfaces — Produces:**
- `BackendApi::compare(&self, session: SessionId, local_root: RemotePath, remote_root: RemotePath, options: CompareOptions, progress: mpsc::UnboundedSender<Progress>, resume: watch::Receiver<bool>) -> Result<Summary>` — uses the session's existing remote `RemoteFs` and the local `LocalFs`; concurrency 4 for SFTP, 1 for FTP/FTPS, local 4.
- `pub struct CompareRunId(pub u64)`; `Effect::Compare { tab: TabId, run: CompareRunId, session, local_root, remote_root, options }`; the effect keeps an abort handle (iced `Task::abortable` or a tokio `AbortHandle`; `hoprs-app` already depends on tokio-util if a token is simpler there) and the `watch::Sender` in a registry keyed by `(tab, run)` inside `effects.rs` (or the backend) so `Effect::CompareResume { tab, run }` and `Effect::CompareCancel { tab, run }` reach them.
- `CompareMsg::{Progress { tab, run, progress: Progress }, Finished { tab, run, result: Result<Summary> }}` streamed back with `Task::run`.
- `FakeBackend::script_compare(...)` to script progress items and the result for app tests.

- [ ] **Step 1: Failing tests:** backend test with `MemFs` remote and a temp local dir returns the expected differences and summary; cancel ends the stream with no `Finished` error noise beyond a "cancelled" result the app ignores; resume after `LimitReached` completes; a closed session yields the coded "session closed" error.
- [ ] **Step 2–4:** run (fail), implement, run tests/fmt/clippy.
- [ ] **Step 5: Commit** `feat(app): run folder comparisons in the backend`.

### Task 5: Compare state and triggers in the app

**Files:** `crates/hoprs-app/src/app/compare.rs` (new), `app.rs` (`TabState`/file-tab state gains `compare: CompareState`), `app/files.rs` (hooks on listing done / folder change / refresh), `app/tabs.rs` (tab close cancels), `app/palette.rs` (action "Compare folders"), `app/tests/compare.rs` (new).

**Interfaces — Produces:**
- `pub struct CompareState { pub run: Option<CompareRunId>, pub roots: Option<(RemotePath, RemotePath)>, pub phase: Phase, pub rows: Vec<Row>, pub identical: u64, pub hidden: bool, pub synced: HashMap<RelPath, (u64, u64)>, pub queued: HashMap<TransferId, RelPath> }`, `pub enum Phase { Idle, Debouncing, Running { seen: (usize, usize) }, Paused { seen: (usize, usize) }, Done { partial: bool }, Stopped(Error) }`, `pub struct Row { pub diff: Difference, pub selected: bool, pub state: RowState }`, `pub enum RowState { Idle, Queued(TransferId), Done, Failed(Error), Skipped(String) }`.
- `CompareMsg` gains `Start { tab }` (button/palette), `Debounced { tab, generation }`, `Continue { tab }`, `Stop { tab }`, `Hide { tab }`, `ToggleAuto(bool)` (emits `SettingsMsg::CompareAuto`'s save), `Select { tab, index, on }`, `SelectGroup { tab, group, on }`, `Open { tab, index }`.
- Debounce: a 500 ms timer effect carrying a generation number; only the latest generation starts a run.

- [ ] **Step 1: Failing app tests (FakeBackend):** auto run when both panes of a new tab finish listing; a folder change starts a new run after the debounce and cancels the previous (`CompareCancel` emitted, late `Progress` of the old run dropped); refresh (F5) re-runs; no run with `compare_auto=false`; `Start` runs regardless of the setting; `ToggleAuto(false)` saves the setting and keeps the current rows; `Hide` hides until the next run; `LimitReached` → `Paused`, `Continue` emits `CompareResume`; tab close emits `CompareCancel`; a session drop sets `Stopped(error)`; a pane `List` effect is still emitted while a run is in progress; palette action "Compare folders" exists for file tabs only.
- [ ] **Step 2–4:** run (fail), implement, run tests/fmt/clippy.
- [ ] **Step 5: Commit** `feat(app): folder compare state and triggers`.

### Task 6: The panel

**Files:** `crates/hoprs-app/src/ui/compare.rs` (new), `ui/files.rs` (place the panel between panes and queue; "Compare" toolbar button), `ui/fit.rs` if widths need it, `ui/strings.rs`, both `.ftl`.

- [ ] **Step 1: Failing tests:** view-level tests in the existing style (content builders that return the texts shown, as `dialogs::content` does): header shows both roots and the toggle state; groups in order Only on this computer / Only on the server / Different, empty groups hidden, counts right; row texts for each `Status` (sizes with `format::size`, "local is newer", "file vs folder", "could not read: <translated error>"); identical summary; running line with counts; paused line with "Compare everything"; stopped line with "Compare again"; Upload enabled only when the selection has local-side items, Download only for remote-side items; pt-BR rendering of the panel's labels via a pt-BR catalog.
- [ ] **Step 2: Implement** with `virtual_list` for rows, checkboxes, group select-all, keyboard (arrows, Space, Enter), mockup v2 styling (1 px rules, no shadows, mono for paths), collapsible like the queue. Render it offscreen in pt-BR and English at 1280 px (as Plan 11 Task 10 did, throwaway harness, not committed) and fix anything cut.
- [ ] **Step 3:** tests, fmt, clippy. **Commit** `feat(app): folder compare panel`.

### Task 7: Acting on differences

**Files:** `app/compare.rs`, `app/transfers.rs` (route transfer events to compare rows), `app/files.rs` (navigate both panes), `app/tests/compare.rs`.

- [ ] **Step 1: Failing tests:** Upload selected enqueues one spec per eligible row (files with `Overwrite` via `enqueue_with_policy`, one-sided folders via the normal enqueue), rows go `Queued(id)`, checkboxes clear; Download selected likewise in the other direction; ineligible rows (KindDiffers, Unreadable, symlinks, wrong side) are skipped; transfer events move rows to Done / Failed / Skipped and back to Idle on cancel; a Done file is added to `synced` with its sizes, and the next run treats an mtime-only difference as identical (ruling 1, via `CompareOptions::known_synced`); with auto on, one re-run starts when every row queued in that batch has finished; `Open` navigates each pane to the parent folder if it exists on that side, else the nearest existing ancestor up to the root, without starting a new run.
- [ ] **Step 2–4:** run (fail), implement, run tests/fmt/clippy.
- [ ] **Step 5: Commit** `feat(app): upload, download and open from the compare panel`.

### Task 8: Integration test and docs

**Files:** `crates/hoprs-remote/tests/sftp.rs` and `tests/ftp.rs` (or a new `tests/compare.rs`, `#[ignore]`, Docker), `README.md`, `CHANGELOG.md`, `docs/ROADMAP.md`, `docs/qa/folder-compare-checklist.md` (new), `CLAUDE.md`, `docs/superpowers/plans/2026-11-12-01-followups.md`.

- [ ] **Step 1:** Docker integration tests: plant a tree on the OpenSSH and vsftpd containers and a local temp tree with one only-local, one only-remote, one size-different file; `compare` finds exactly those over SFTP and FTP (and FTP `LIST` precision path if the vsftpd config allows `MLSD` off; otherwise note it).
- [ ] **Step 2:** README "Using hoprs → Compare folders" (what it shows, automatic toggle, ignore list, limit, upload/download overwrite the other side), CHANGELOG 1.2.0 entry, ROADMAP moves folder compare to done-on-main, QA checklist (NEEDS HUMAN: real project over SFTP and FTP, large tree prompt, panel at 1280 px in both languages, keyboard use), CLAUDE.md Plan 12 entries, follow-ups "Plan 12" (delete one-sided, content diff, mirror, checksums, case-insensitive local FS matching, FTP `LIST` timezone offsets).
- [ ] **Step 3:** `cargo test --workspace --all-features` (and the Docker tests if Docker is available), fmt, clippy. **Commit** `test, docs: folder compare integration tests and docs`.
