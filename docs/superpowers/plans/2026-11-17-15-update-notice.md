# Plan 15: Update notice (1.4.0) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** hoprs checks GitHub for a newer stable release, shows a non-blocking card once per new version (then a footer line), and on **Update** downloads the right package, verifies it against the release's `SHA256SUMS` and hands it to the system installer.

**Architecture:** `hoprs-app` only. Pure logic in a new `src/update/` module (version compare, release JSON, package choice, `SHA256SUMS`, the download-and-verify sequence over a `Fetch` trait); the network through a small blocking HTTPS client (`ureq` on the tree's rustls + aws-lc-rs) run on tokio's blocking pool behind two new `BackendApi` methods; the state machine in `app/update.rs` driven by `Message::Update(UpdateMsg)` and effects like everything else; the card is a stack layer like the toast, the footer line sits in the sidebar, the rows in Settings → General, a row in the Ctrl+K palette. Remembered state is two keys in `settings.toml`.

**Tech Stack:** Rust 1.99.1 (pinned), iced 0.14, tokio, `ureq` 3.5 (`rustls-no-provider`), rustls 0.23 with aws-lc-rs, rustls-native-certs 0.8, `semver` 1, `serde_json` 1, `sha2` 0.10, `dirs` 7, `shlex` 2; `rcgen` 0.14 (dev, the ignored integration test).

**Spec:** `docs/superpowers/specs/2026-11-17-update-notice-design.md` (binding, including "Builder-check rulings" 1–4).

## Global Constraints

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean on every commit; `cargo test --workspace --all-features` green on Linux, macOS and Windows (`tempfile`, no literal `/tmp`, no `/`-separator assumptions in local paths).
- Only `hoprs-app` changes. New crates in `Cargo.lock`: exactly `ureq`, `ureq-proto`, `http`, `httparse`, `utf8-zero` (verified with `cargo update --offline`). No OpenSSL, no `ring`, no second TLS stack, no `webpki-roots`. `semver`, `serde_json`, `sha2`, `rustls`, `rustls-native-certs`, `rcgen` are already in the tree.
- Request: HTTPS only with certificate validation against the system roots, `User-Agent: hoprs/<version>`, no cookies, no tokens, no other identifier; check and `SHA256SUMS` calls end within 10 s; rate limits and HTTP errors are a failed check.
- Source: `https://api.github.com/repos/engviniciusbs/hoprs/releases/latest` (stable only). Release builds always use it and their real version. Debug builds: `HOPRS_UPDATE_URL` (API URL) and `HOPRS_UPDATE_AS_VERSION` (running version) are read, and a debug build checks only when `HOPRS_UPDATE_URL` is set.
- Asset names: `hoprs_<v>-1_amd64.deb`, `hoprs_<v>_windows_x64.msi`, `hoprs_<v>_macos_universal.dmg`, `SHA256SUMS` (`.github/workflows/release.yml`).
- Anti-goals: no modal, no backdrop, no focus change, no sound; nothing installed or opened without a click; no file opened whose checksum does not match; the check never delays the window or the first listing; automatic-check failures only log.
- Mockup v2: `theme::dialog_box` look (paper_2, 1 px `rule_2`, 4 px radius, no shadow). Every user-visible text in `crates/hoprs-app/i18n/en.ftl` and `pt-BR.ftl` (`every_en_key_is_used`, `error_codes_exist_in_both_catalogs`); errors through `strings::error`. pt-BR natural, product terms in English.
- Commits: `git -c user.name="$(git log -1 --format=%an)" -c user.email="$(git log -1 --format=%ae)" commit …`, message ends with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never change git config.

### Rulings this plan makes (left to the builder)

1. **HTTP client: `ureq` 3.5**, `default-features = false, features = ["rustls-no-provider"]`, given `rustls::crypto::aws_lc_rs::default_provider()` through `TlsConfig::unversioned_rustls_crypto_provider` and the system roots as `RootCerts::Specific` (loaded with `rustls_native_certs::load_native_certs`, as `hoprs-remote/src/ftp/tls.rs` does). A hand-written client over tokio-rustls would need redirects (GitHub's asset URL answers 302 to its storage host), chunked decoding (the API answers chunked), Content-Length checks, header parsing and proxy-free edge cases — a few hundred lines of security-relevant code to own for two GET requests a day. `reqwest` with `rustls-tls` pulls hyper, hyper-util, tower, h2/http-body and more. `ureq` without its default features adds five small crates (`ureq`, `ureq-proto`, `http`, `httparse`, `utf8-zero`; its `base64`, `log`, `percent-encoding`, `rustls`, `rustls-pki-types` are already locked) and reuses the tree's rustls with aws-lc-rs; it is blocking, so calls run on `tokio::task::spawn_blocking`.
2. **Remembered state** lives in `settings.toml` beside the sidebar's section state: `update_check` (bool, default `true`) and `update_seen` (the newest version a card was shown for; omitted while unset). One key is enough: the card shows once per version whatever the user does with it, so "dismissed" equals "seen"; ✕ only collapses it for this session. The verified file is app state only; after a restart, Update finds the file already in Downloads, verifies it and reuses it instead of downloading again.
3. **No fade.** iced reports no reduced-motion preference; the toast already appears without animation for that reason (`app/edit.rs` `TOAST_TIME`). The card appears and goes the same way.
4. **Opening files and URLs** reuses the existing system opener (`explorer` / `open` / `xdg-open`, `effects.rs::open_with_system`), moved to `src/opener.rs` and generalised from folders to any target: `explorer <file.msi>` and `explorer <https URL>` open with the association, `open <file.dmg>` mounts it, `xdg-open <file.deb>` opens App Center.
5. **Check timing:** first automatic check 5 s after settings load, then every 24 h while the app runs. Each schedule carries a generation; only the latest one acts.
6. **Windows detection:** `.msi` only when `hoprs.exe` sits in `<ProgramFiles>\hoprs\` (`ProgramW6432`, `ProgramFiles` env vars); a custom install folder opens the release page (safe fallback). `.deb` and `.msi` only on `x86_64`.
7. **Download order:** `SHA256SUMS` first (a missing entry downloads nothing), then `<name>.part`, verified before it is renamed to `<name>`: the final name never holds unverified bytes.

## Review Focus

1. **GitHub answers something unexpected** (HTML error page, 403 rate limit, `tag_name` `v1.5.0-rc.1` on a non-prerelease, `1.4`, missing `assets`, an `http://` URL). Expected: automatic check shows nothing (log only); Check now shows the error. Tests: Task 1 `release_json_missing_fields_fails`, `versions_compare_by_semver`; Task 3 `automatic_check_failure_shows_nothing`.
2. **Setting toggled off/on quickly, or Check now while the automatic check runs.** Expected: one request in flight, one 24 h timer. Tests: Task 3 `toggling_setting_keeps_one_timer`, `check_now_while_checking_sends_no_second_request`.
3. **Download truncated, Downloads unwritable, or a same-named file already there.** Expected: no unverified file under the final name; a matching file is reused; a non-matching one is replaced only after the new one verifies. Tests: Task 2 `truncated_download_leaves_no_file`, `existing_verified_file_is_reused`, `existing_bad_file_kept_until_new_one_verifies`.
4. **Linux without a graphical package handler** (`xdg-open` fails). Expected: the card keeps the `sudo apt install …` command and Copy, the open error shows inline. Test: Task 4 `linux_open_failure_keeps_command`.
5. **Update clicked twice, or a newer release learned mid-download.** Expected: one download; the file being fetched is not swapped. Tests: Task 4 `second_update_click_is_ignored_while_downloading`, `newer_release_during_download_waits`.

---

### Task 1: Pure update logic and the settings keys

**Files:**
- Create: `crates/hoprs-app/src/update/mod.rs`, `crates/hoprs-app/src/update/package.rs`, `crates/hoprs-app/src/update/sums.rs`, `crates/hoprs-app/src/update/fixtures/latest-release.json`
- Modify: `crates/hoprs-app/src/lib.rs` (`pub mod update;`), `crates/hoprs-app/Cargo.toml` (`semver = "1"`, `serde_json = { workspace = true }`, `sha2 = { workspace = true }`, each with a one-line comment like its neighbours), `crates/hoprs-app/src/settings.rs`, both `.ftl`

**Interfaces — Produces:**
- `update/mod.rs`: `pub const API_URL: &str = "https://api.github.com/repos/engviniciusbs/hoprs/releases/latest";` `pub const SUMS_NAME: &str = "SHA256SUMS";` `pub const START_DELAY: Duration = 5 s;` `pub const CHECK_EVERY: Duration = 24 h;`
- `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Asset { pub name: String, pub url: String, pub size: u64 }`
- `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Release { pub version: semver::Version, pub page: String, pub assets: Vec<Asset> }` with `pub fn asset(&self, name: &str) -> Option<&Asset>`
- `pub fn parse_version(tag: &str) -> Option<Version>` (`v` prefix optional; pre-release or build metadata → `None`)
- `pub fn parse_release(json: &[u8]) -> Result<Release, Error>` (fields `tag_name`, `html_url`, `draft`, `prerelease`, `assets[].name|browser_download_url|size`; any missing/wrong type, draft or prerelease true, unparsable tag, a URL not starting with `https://` → `Error::coded(ErrorKind::Protocol, "update-bad-response", …)` with the detail)
- `pub fn is_newer(release: &Version, running: &Version) -> bool`
- `update/package.rs`: `#[derive(Copy, Clone, Debug, PartialEq, Eq)] pub enum Os { Linux, Windows, MacOs, Other }` with `pub fn current() -> Os`; `#[derive(Copy, Clone, Debug, PartialEq, Eq)] pub enum PackageKind { Deb, Msi, Dmg }`; `pub fn package_for(os: Os, x86_64: bool, exe: &str, program_files: &[&str]) -> Option<PackageKind>` (strings, so Windows paths test on every OS: Windows compares case-insensitively with `\`/`/` normalised; Linux exactly `/usr/bin/hoprs`; macOS a path ending `.app/Contents/MacOS/hoprs`); `pub fn asset_name(kind: PackageKind, version: &Version) -> String`; `pub fn current_package() -> Option<PackageKind>` (`std::env::current_exe()` canonicalised, `ProgramW6432`/`ProgramFiles`, `cfg!(target_arch = "x86_64")`).
- `update/sums.rs`: `pub fn expected_for(sums: &str, name: &str) -> Result<[u8; 32], Error>` (lines `<64 hex>  <name>` or `<64 hex> *<name>`, other lines skipped; missing → code `update-checksum-missing`, arg `name`); `pub fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]>` (streamed, 64 KiB buffer); `pub fn verify(path: &Path, expected: &[u8; 32]) -> Result<(), Error>` (mismatch → code `update-checksum-mismatch`).
- `Settings { …, pub update_check: bool, pub update_seen: Option<semver::Version> }`; file keys `update_check` (non-bool → `true`), `update_seen` (string; unparsable → `None`; in `OPTIONAL_KEYS`).

- [ ] **Step 1: Fixture.** `latest-release.json`: a trimmed real response of `GET /repos/engviniciusbs/hoprs/releases/latest` for `v1.3.1` (`tag_name`, `html_url`, `draft`, `prerelease`, `published_at`, five `assets` with `name`, `size`, `browser_download_url`).
- [ ] **Step 2: Failing tests** (in each file's `#[cfg(test)] mod tests`):
  - `versions_compare_by_semver`: `is_newer(1.4.0, 1.3.1)`, `is_newer(1.10.0, 1.9.0)`, not `is_newer(1.3.1, 1.3.1)`, not `is_newer(1.3.0, 1.3.1)`; `parse_version("v1.4.0")` and `("1.4.0")` are `Some(1.4.0)`; `"v1.5.0-rc.1"`, `"1.4"`, `"latest"`, `""`, `"v1.4.0+build"` are `None`.
  - `release_json_parses`: fixture → version 1.3.1, page `https://github.com/engviniciusbs/hoprs/releases/tag/v1.3.1`, 5 assets, `asset("hoprs_1.3.1-1_amd64.deb")` has its URL and size.
  - `release_json_missing_fields_fails`: no `tag_name`; `assets` not an array; `"draft": true`; `"prerelease": true`; `html_url` `http://…`; `<html>` body; empty body → each `Err` with `code == Some("update-bad-response")`.
  - `package_per_os_and_path`: `(Linux, true, "/usr/bin/hoprs")` → `Deb`; `(Linux, false, "/usr/bin/hoprs")` → `None`; `(Linux, true, "/home/u/src/hoprs/target/release/hoprs")` → `None`; `(Windows, true, r"C:\Program Files\hoprs\hoprs.exe", [r"C:\Program Files"])` → `Msi`; same with `c:\program files\HOPRS\hoprs.exe` → `Msi`; `r"C:\Users\u\Downloads\hoprs_1.3.1_windows_x64\hoprs.exe"` → `None`; `(MacOs, false, "/Applications/hoprs.app/Contents/MacOS/hoprs")` → `Dmg`; `(MacOs, _, "/Users/u/hoprs")` → `None`; `Other` → `None`.
  - `asset_names_follow_release_naming`: 1.4.0 → `hoprs_1.4.0-1_amd64.deb`, `hoprs_1.4.0_windows_x64.msi`, `hoprs_1.4.0_macos_universal.dmg`.
  - `sums_parse_and_verify`: a tempfile holding `hello`; sums text with a garbage line, `<sha of hello>  a.deb`, `<sha> *b.msi` → `expected_for` finds both; `verify` Ok; another file → `update-checksum-mismatch`; `expected_for(…, "c.dmg")` → `update-checksum-missing`.
  - In `settings.rs`: `update_keys_roundtrip_and_invalid`: default `update_check == true`, `update_seen == None`, and `render(&Settings::default())` has no `update_seen`; round-trip of `{false, Some(1.4.0)}`; `update_check = "no"` → `true`; `update_seen = 3` and `= "banana"` → `None`; unknown keys kept on save (as `sidebar_sections_roundtrip_and_invalid`).
- [ ] **Step 3: Run** `cargo test -p hoprs-app update:: settings::` — FAIL (missing items).
- [ ] **Step 4: Implement** the interfaces above; `serde_json::Value` walking (no derive needed). Add `error-update-bad-response`, `error-update-checksum-missing` (uses `{ $name }`), `error-update-checksum-mismatch` to both catalogs.
- [ ] **Step 5: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 6: Commit** `feat(app): update module: versions, release JSON, package choice, SHA256SUMS`.

### Task 2: HTTPS client, download sequence, backend and opener

**Files:**
- Create: `crates/hoprs-app/src/update/http.rs`, `crates/hoprs-app/src/update/download.rs`, `crates/hoprs-app/src/opener.rs`
- Modify: `crates/hoprs-app/Cargo.toml` (`ureq = { version = "3.5", default-features = false, features = ["rustls-no-provider"] }`, `rustls = { workspace = true }`, `rustls-native-certs = { workspace = true }`; dev `rcgen = { workspace = true }`), `Cargo.lock`, `crates/hoprs-app/src/lib.rs` (`pub mod opener;`), `crates/hoprs-app/src/effects.rs` (`open_folder`/`open_with_system` move to `opener.rs`; `Effect::OpenProfileFolder` calls `opener::open`), `crates/hoprs-app/src/backend/mod.rs`, `crates/hoprs-app/src/backend/fake.rs`, both `.ftl`

**Interfaces:**
- Consumes: Task 1's `Release`, `Asset`, `parse_release`, `expected_for`, `sha256_file`, `verify`, `SUMS_NAME`.
- Produces:
  - `update/http.rs`: `pub fn api_url() -> Option<String>` (release: `Some(API_URL)`; debug: `HOPRS_UPDATE_URL` if set); `pub struct Client` with `pub fn new(api_url: String, roots: Vec<CertificateDer<'static>>) -> Client`, `pub fn system(api_url: String) -> Client` (native roots, loaded once per process in a `OnceLock`), `pub fn latest(&self) -> Result<Release, Error>` (`Accept: application/vnd.github+json`, `timeout_global` 10 s, body limit 1 MiB). Agent: `https_only(true)`, `http_status_as_error(true)`, `max_redirects(5)`, `user_agent("hoprs/" + CARGO_PKG_VERSION)`, `timeout_connect` 10 s, `timeout_recv_response` 10 s. Errors: `update-check-failed` (`ErrorKind::Network`, detail = the ureq error, "rate limited" for 403/429).
  - `update/download.rs`: `pub trait Fetch { fn text(&self, url: &str) -> Result<String, Error>; fn file(&self, url: &str, to: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<u64, Error>; }` implemented by `Client` (`text`: 10 s, 64 KiB limit; `file`: `timeout_per_read` 30 s, no global limit, a body shorter than its Content-Length is `update-download-failed`); `#[derive(Clone, Debug)] pub struct DownloadJob { pub asset: Asset, pub sums_url: String }`; `#[derive(Clone, Debug)] pub enum DownloadEvent { Progress { done: u64, total: Option<u64> }, Verifying, Done(Result<PathBuf, Error>) }`; `pub fn run(fetch: &dyn Fetch, job: &DownloadJob, dir: &Path, send: &mut dyn FnMut(DownloadEvent))` (ruling 7: sums → existing `<name>` verified ⇒ `Done(Ok)` at once → `<name>.part` with `Progress` at most every 100 ms → `Verifying` → verify → rename; any failure removes the `.part`; exactly one `Done`); `pub fn download_dir() -> PathBuf` (`dirs::download_dir()` if it is a directory, else `std::env::temp_dir()`).
  - `opener.rs`: `pub async fn open(target: OsString) -> Result<(), Error>` (the moved `open_with_system` on the blocking pool, unchanged behaviour and error codes).
  - `BackendApi`: `async fn latest_release(&self) -> Result<Release>;` and `fn download_update(&self, job: DownloadJob) -> BoxStream<'static, DownloadEvent>;`. `Backend`: a lazily built `Arc<Client>` from `api_url()` (none → `Err` code `update-dev-build`); `latest` and `download::run` on `spawn_blocking`, the events through an `mpsc` channel into the stream, `dir = download_dir()`. `FakeBackend`: `with_release(Result<Release>)` (default `Err`), `set_download(Vec<DownloadEvent>)` (the stream yields them), `releases_asked() -> u32`, `downloads() -> Vec<DownloadJob>`.

- [ ] **Step 1: Failing tests** in `update/download.rs` with a test `Fetch` (maps URLs to bytes; can cut a body short):
  - `verified_download_is_renamed`: events are `Progress…`, `Verifying`, `Done(Ok(dir/name))`; `dir/name` holds the bytes; no `.part` remains.
  - `checksum_mismatch_deletes_part`: `Done(Err)` code `update-checksum-mismatch`; neither `name` nor `name.part` exists.
  - `truncated_download_leaves_no_file` (RF3): `file` fails midway → `Done(Err)` code `update-download-failed`; no file left.
  - `existing_verified_file_is_reused` (RF3): `dir/name` already holds the right bytes → `file` never called; `Done(Ok)`.
  - `existing_bad_file_kept_until_new_one_verifies` (RF3): `dir/name` holds other bytes and the new download mismatches → the old file is untouched; with a good download it is replaced.
  - `missing_sums_entry_downloads_nothing`: `Done(Err)` code `update-checksum-missing`; `file` never called.
- [ ] **Step 2: Failing integration test** in `update/http.rs`, `#[ignore]` (run with `--ignored`, like the Docker tests): `local_https_server_redirect_chunked_and_checksum`. A `std::net::TcpListener` on `127.0.0.1:0` with an `rcgen` certificate for `localhost` and `rustls::ServerConnection` + `StreamOwned` in a thread, `Connection: close` answers: `/latest` (the fixture with asset URLs rewritten to `https://localhost:<port>/dl/…`, sent `Transfer-Encoding: chunked`), `/dl/hoprs_9.9.9-1_amd64.deb` → `302 Location: /objects/1`, `/objects/1` (bytes with Content-Length), `/dl/SHA256SUMS`; and `/objects/bad` for a second run. `Client::new(url, vec![ca])`: `latest()` parses; `download::run` ends `Done(Ok)` with the bytes; the bad object ends `update-checksum-mismatch` with no file; the request headers seen include `user-agent: hoprs/` and no `cookie`/`authorization`.
- [ ] **Step 3: Run** `cargo test -p hoprs-app update::download` — FAIL.
- [ ] **Step 4: Implement** the interfaces; move the opener; wire the backend and the fake. Add `error-update-check-failed`, `error-update-download-failed`, `error-update-dev-build` ("Update checks are off in development builds (set HOPRS_UPDATE_URL)") to both catalogs. Check `Cargo.lock` gained exactly the five crates (`git diff Cargo.lock | grep '^+name'`).
- [ ] **Step 5: Run** `cargo test -p hoprs-app`, then `cargo test -p hoprs-app update::http -- --ignored`, fmt, clippy — PASS.
- [ ] **Step 6: Commit** `feat(app): HTTPS update client (ureq on rustls), verified download, backend methods`.

### Task 3: Checking, the card's state, Settings and palette

**Files:**
- Create: `crates/hoprs-app/src/app/update.rs`, `crates/hoprs-app/src/app/tests/update.rs`
- Modify: `crates/hoprs-app/src/app.rs` (`pub mod update;`, field `pub updater: update::Updater`, `Message::Update(UpdateMsg)`, effects below, `apply` arm), `app/settings.rs` (`SettingsMsg::UpdateCheck(bool)`, `SettingsMsg::CheckNow`; `settings_loaded` returns `Vec<Effect>`), `app/palette.rs` (`PaletteItem::Update`), `app/dialogs.rs` (`behind_overlay`: `Message::Update(msg) => msg.is_user()`), `app/lifecycle.rs` (`while_closing`: user `UpdateMsg`s → `Some(Vec::new())`), `effects.rs`, `app/tests.rs` (`mod update;`), `ui/strings.rs` + both `.ftl` (`update-palette` = "Update hoprs to { $version }")

**Interfaces:**
- Consumes: Task 1 `Release`, `is_newer`, `START_DELAY`, `CHECK_EVERY`, `Os`, `PackageKind`, `current_package`; Task 2 `BackendApi::latest_release`, `opener::open`, `DownloadEvent`.
- Produces:
  - `pub struct UpdateEnv { pub running: Version, pub auto: bool, pub os: Os, pub package: Option<PackageKind> }`; `UpdateEnv::from_process()` (running = `CARGO_PKG_VERSION`, or in debug `HOPRS_UPDATE_AS_VERSION` when it parses; `auto = !cfg!(debug_assertions) || api_url().is_some()`); `#[cfg(test)] UpdateEnv::test(running: &str, os: Os, package: Option<PackageKind>)` (`auto: true`).
  - `pub enum ManualCheck { Idle, Checking, UpToDate, Available(Version), Failed(Error) }`
  - `pub enum Phase { Idle, Downloading { done: u64, total: Option<u64> }, Verifying, Ready(PathBuf), HandedOff { file: PathBuf, open_error: Option<Error> }, Failed(Error) }` (Task 4 drives all but `Idle`).
  - `pub struct Updater { pub env: UpdateEnv, pub known: Option<Release>, pub card: bool, pub phase: Phase, pub manual: ManualCheck, pub page_error: Option<Error>, in_flight: bool, generation: u64 }`; `App::new()` uses `UpdateEnv::from_process()`.
  - `#[derive(Debug, Clone)] pub enum UpdateMsg { CheckDue(u64), CheckNow, Checked { manual: bool, result: Result<Release, Error> }, ShowCard, Dismiss, WhatsNew, PageOpened(Result<(), Error>), Start, Download(DownloadEvent), Install, Later, CopyCommand, InstallerOpened(Result<(), Error>) }` with `pub fn is_user(&self) -> bool` (`CheckNow`, `ShowCard`, `Dismiss`, `WhatsNew`, `Start`, `Install`, `Later`, `CopyCommand`).
  - `Effect::CheckUpdate { manual: bool }` → `backend.latest_release()` → `UpdateMsg::Checked`; `Effect::OpenPage(String)` → `opener::open` → `UpdateMsg::PageOpened`.
  - `impl App { pub fn update_offer(&self) -> Option<&Version>` (the known release if newer than running, else `settings.update_seen` if newer); `pub(super) fn update_msg(&mut self, msg: UpdateMsg) -> Vec<Effect>`; `fn schedule_update_check(&mut self, after: Duration) -> Vec<Effect>` (bumps `generation`; empty unless `env.auto && settings.update_check`) `}`.
- Rules: `CheckDue(g)` acts only if `g` is the current generation, the setting is on and nothing is in flight; an automatic result re-arms `CHECK_EVERY`; `Checked` with a newer release sets `known` (only while `phase` is `Idle` or `Failed`, see Task 4), and shows the card iff `update_seen` is `None` or older, then sets `update_seen` and saves (`save_settings_effect`); a failed automatic check only logs (`tracing::info!`); a manual one sets `ManualCheck`. `CheckNow` while in flight only marks the pending result as manual. `ShowCard` with no `known` (footer from `update_seen` before a check) shows the card and sends `CheckUpdate { manual: true }`. `Dismiss` hides the card. `WhatsNew` → `OpenPage(known.page)`. Turning the setting on schedules `START_DELAY`; off bumps the generation.

- [ ] **Step 1: Failing tests** in `app/tests/update.rs` (an app from `loaded(&FakeBackend::new())` with `app.updater = Updater::new(UpdateEnv::test("1.3.1", Os::Linux, Some(PackageKind::Deb)))`; a `release("1.4.0")` helper builds a `Release` with the three package assets and `SHA256SUMS`):
  - `settings_loaded_schedules_one_check_after_start_delay`: `SettingsLoaded` → exactly one `Effect::Later { after: START_DELAY, message: Update(CheckDue(_)) }`; its message → `[CheckUpdate { manual: false }]`.
  - `setting_off_means_no_automatic_check`: loaded with `update_check = false` → no `Later`; `env.auto = false` → none either; `CheckDue` of any generation → no effect.
  - `check_due_rearms_after_24h`: `Checked` (Ok or Err, automatic) → `Later { after: CHECK_EVERY, .. }`.
  - `toggling_setting_keeps_one_timer` (RF2): `UpdateCheck(false)`, `(true)`, `(false)`, `(true)`; delivering every `CheckDue` produced → exactly one `CheckUpdate`.
  - `check_now_while_checking_sends_no_second_request` (RF2): `CheckDue` then `CheckNow` → no second `CheckUpdate`; the result lands in `manual`.
  - `newer_release_shows_card_once_and_saves_seen`: `Checked(Ok(1.4.0))` → `card`, `settings.update_seen == Some(1.4.0)`, an `Effect::SaveSettings`.
  - `card_takes_no_focus`: those effects hold no `Unfocus`, `FocusEdit`, `FocusPalette`, `FocusTerminalSearch`, `FocusDialogField`, and `app.palette`, `app.menu`, `app.dialogs` are unchanged.
  - `restart_shows_only_the_footer_line`: `update_seen = 1.4.0` loaded, `Checked(Ok(1.4.0))` → `!card`, `update_offer() == Some(1.4.0)`.
  - `footer_line_from_seen_before_any_check`: seen 1.4.0 → offer `Some`; with running 1.4.0 → `None`; running 1.5.0 → `None`.
  - `newer_version_shows_card_again`: seen 1.4.0, `Checked(Ok(1.5.0))` → `card`.
  - `same_or_older_release_shows_nothing`: 1.3.1 and 1.2.0 → `!card`, offer `None`.
  - `automatic_check_failure_shows_nothing` (RF1): `Checked { manual: false, Err }` → `!card`, `manual == Idle`.
  - `check_now_results`: up to date → `UpToDate`; 1.4.0 → `Available(1.4.0)` and `card`; `Err` → `Failed` with that error.
  - `dismiss_then_show_card`: `Dismiss` → `!card`, offer still `Some`; `ShowCard` → `card`, no request.
  - `show_card_without_release_checks_first`: seen-only → `ShowCard` → `card` and `[CheckUpdate { manual: true }]`.
  - `whats_new_opens_the_release_page`: → `[OpenPage("https://github.com/…/v1.4.0")]`; `PageOpened(Err)` → `page_error`.
  - `palette_lists_update_while_newer_known`: an entry labelled `Update hoprs to 1.4.0` whose message is `Update(Start)`; none while nothing newer is known.
  - `update_input_ignored_while_closing_or_behind_dialog`: with `closing` → `Start` yields no effect.
- [ ] **Step 2: Run** `cargo test -p hoprs-app app::tests::update` — FAIL.
- [ ] **Step 3: Implement** the interfaces and rules; `Start`, `Download`, `Install`, `Later`, `CopyCommand`, `InstallerOpened` return no effect yet (Task 4).
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): background update check, card state, Settings and palette`.

### Task 4: Updating: download, verify, hand off

**Files:**
- Modify: `crates/hoprs-app/src/app/update.rs`, `crates/hoprs-app/src/app.rs` (effects), `crates/hoprs-app/src/effects.rs`, `crates/hoprs-app/src/app/tests/update.rs`

**Interfaces:**
- Consumes: Task 1 `asset_name`, `SUMS_NAME`; Task 2 `DownloadJob`, `DownloadEvent`, `BackendApi::download_update`, `opener::open`; Task 3 `Updater`, `Phase`, `UpdateMsg`.
- Produces:
  - `Effect::DownloadUpdate(DownloadJob)` → `Task::run(backend.download_update(job), |e| Message::Update(UpdateMsg::Download(e)))`; `Effect::OpenInstaller(PathBuf)` → `opener::open` → `UpdateMsg::InstallerOpened`; `Effect::RequestClose` → `iced::window::oldest()` then `Message::CloseRequested(id)` (the normal close path: unsent-edits question, tabs saved, shutdown).
  - `pub fn install_command(file: &Path) -> String` = `sudo apt install <file>` with the absolute path quoted by `shlex::try_quote` (always contains a `/`, so apt reads it as a file).
  - `impl App { pub fn transfers_to_stop(&self) -> usize` (`queue.counts()` Active + Queued) `}`.
- Rules: `Start` (Update, Try again, palette) with `phase` `Downloading`/`Verifying` → nothing; `Ready` → as `Install`; no `known` → as `ShowCard`; `env.package` `None`, or the asset or `SHA256SUMS` missing from the release → `OpenPage(page)` (spec: page fallback); else shows the card, `phase = Downloading { 0, Some(size) }`, `[DownloadUpdate { asset, sums_url }]`. `Download(Progress)` / `Verifying` update `phase`; `Done(Ok(file))`: Windows → `Ready(file)` (ruling 1: second click); Linux and macOS → `HandedOff { file, open_error: None }` and `[OpenInstaller(file)]`. `Done(Err)` → `Failed(err)`. `Install` on `Ready(file)` → `[OpenInstaller(file)]`; `InstallerOpened(Ok)` on Windows → `[RequestClose]`; `InstallerOpened(Err)` → Windows `Failed`, others `open_error`. `Later` → `!card`, `phase` stays `Ready`. `CopyCommand` → `[WriteClipboard(install_command(file))]`. While `phase` is `Downloading`, `Verifying`, `Ready` or `HandedOff`, a newer `Checked` release does not replace `known`.

- [ ] **Step 1: Failing tests** (append to `app/tests/update.rs`):
  - `update_without_package_opens_release_page`: package `None` → `[OpenPage(page)]`, no download; `missing_asset_opens_release_page`: release without the `.deb` (or without `SHA256SUMS`) → `[OpenPage(page)]`.
  - `linux_downloads_verifies_and_hands_off`: `Start` → `[DownloadUpdate(job)]` with `job.asset.name == "hoprs_1.4.0-1_amd64.deb"` and `sums_url` of `SHA256SUMS`; `Progress { 10, Some(100) }` → `Downloading`; `Verifying`; `Done(Ok(f))` → `[OpenInstaller(f)]`, `HandedOff`; no `RequestClose` anywhere.
  - `install_command_quotes_the_full_path`: `/home/u/My Downloads/hoprs_1.4.0-1_amd64.deb` → `sudo apt install '/home/u/My Downloads/hoprs_1.4.0-1_amd64.deb'`.
  - `linux_open_failure_keeps_command` (RF4): `InstallerOpened(Err)` → still `HandedOff` with `open_error`; `CopyCommand` → `[WriteClipboard]` of the command.
  - `macos_hands_off_without_closing`: `Os::MacOs`, `Dmg` → `hoprs_1.4.0_macos_universal.dmg`, `OpenInstaller`, never `RequestClose`.
  - `windows_waits_for_second_click`: `Done(Ok(f))` → `Ready(f)`, no effect; queue with 2 active + 1 queued → `transfers_to_stop() == 3`; `Install` → `[OpenInstaller(f)]`; `InstallerOpened(Ok)` → `[RequestClose]`; `InstallerOpened(Err)` → `Failed`, no close.
  - `windows_later_keeps_the_file`: `Later` → `!card`, `Ready(f)`; `ShowCard` then `Start` → `[OpenInstaller(f)]` (no new download).
  - `download_error_offers_try_again`: `Done(Err(e))` → `Failed(e)`; `Start` → a new `DownloadUpdate`.
  - `second_update_click_is_ignored_while_downloading` (RF5): two `Start`s → one `DownloadUpdate`.
  - `newer_release_during_download_waits` (RF5): `Checked(Ok(1.5.0))` while `Downloading` → `known` still 1.4.0, no effect.
- [ ] **Step 2: Run** `cargo test -p hoprs-app app::tests::update` — FAIL.
- [ ] **Step 3: Implement** the rules and the three effects.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): Update downloads, verifies and hands off to the system installer`.

### Task 5: The card, the footer line and the Settings rows

**Files:**
- Create: `crates/hoprs-app/src/ui/update.rs`
- Modify: `crates/hoprs-app/src/ui/mod.rs`, `ui/layout.rs` (`menu::overlay(app, update::overlay(app, base))`: the card under menus, drag, palette and dialogs), `ui/sidebar.rs` (the footer line above the footer row; `pub const FOOTER_HEIGHT: f32` for the card's offset), `ui/settings.rs` (`general`), `ui/strings.rs`, both `.ftl`

**Interfaces:**
- Consumes: Task 3/4 `App::update_offer`, `Updater`, `Phase`, `ManualCheck`, `UpdateMsg`, `install_command`, `transfers_to_stop`.
- Produces:
  - `#[derive(Debug, PartialEq)] pub enum CardBody { Offer, Progress { fraction: Option<f32>, label: String }, Checking, Ready { transfers: usize }, Linux { command: String, open_error: Option<String> }, MacOs, Failed(String) }` and `pub fn card_body(app: &App) -> Option<CardBody>` (`None` unless `updater.card` and an offer exists; `Phase::Idle` + `manual == Checking` and no `known` → `Checking`).
  - `pub fn footer_label(app: &App) -> Option<String>` (`Ready` → "Install X.Y.Z", else "X.Y.Z available"; `None` with no offer).
  - `pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message>`: always `stack![base, …]` (state kept, as `menu::overlay`); the layer is the toast's pattern (a `Length::Fill` container aligned bottom-left, padding left `theme::SPACE_XS`, bottom `sidebar::FOOTER_HEIGHT + theme::SPACE_XS`, no `mouse_area`, so the rest of the window gets every press), the card `opaque(container(..).width(sidebar::WIDTH + theme::SPACE_LG).padding(theme::SPACE_SM).style(theme::dialog_box))`. No text input in the card.
  - `pub fn footer_line(app: &App) -> Option<Element<'_, Message>>`: `Icon::ArrowDown` + label, a text button sending `Update(ShowCard)`.
- Card content: header `Icon::DownloadSimple` + `update-available` + `Icon::X` (`update-dismiss` tooltip, `Dismiss`); body per `CardBody`: `Offer` → `update-you-have`, **Update** (`theme::button_primary`, `Start`), **What's new** (`theme::button_text`, `WhatsNew`); `Progress` → `progress_bar` (`theme::progress`) + `update-downloading` with `format::size`; `Checking` → `update-verifying`; `Ready` → `update-ready-windows`, `update-transfers-stop` when `transfers > 0`, **Close and install** (`Install`), **Later** (`Later`); `Linux` → `update-linux-opened`, the command in `theme::MONO` and **Copy** (`Icon::Copy`, `CopyCommand`), the open error muted below; `MacOs` → `update-macos-steps`; `Failed` → the error in `palette().error` + **Try again** + **What's new**. `page_error` shows muted under any body.
- Settings → General, after Language: checkbox `settings-update-check` (`SettingsMsg::UpdateCheck`) with the hint `settings-update-check-hint`; **Check now** (`outlined`, disabled while `Checking`, `SettingsMsg::CheckNow`) and the inline result (`settings-update-checking`, `settings-up-to-date`, `update-available`, or the error), above the version line.
- Strings (en; pt-BR alongside): `update-available` = "hoprs { $version } is available"; `update-you-have` = "You have { $version }."; `update-action` = "Update"; `update-whats-new` = "What's new"; `update-dismiss` = "Dismiss"; `update-downloading` = "Downloading… { $done } of { $total }"; `update-verifying` = "Checking…"; `update-try-again` = "Try again"; `update-ready-windows` = "Ready to install. hoprs will close."; `update-transfers-stop` = plural on `$count`: "1 transfer will stop; it continues the next time hoprs starts." / "{ $count } transfers will stop; they continue the next time hoprs starts."; `update-close-and-install` = "Close and install"; `update-later` = "Later"; `update-linux-opened` = "Opened in the system installer. Or run:"; `update-copy` = "Copy"; `update-macos-steps` = "Quit hoprs, then drag hoprs to Applications and choose Replace. If macOS blocks it, choose Open Anyway in System Settings, Privacy & Security."; `update-footer-available` = "{ $version } available"; `update-footer-install` = "Install { $version }"; `settings-update-check` = "Check for updates automatically"; `settings-update-check-hint` = "Once a day hoprs asks GitHub for the latest release. Nothing about you is sent."; `settings-check-now` = "Check now"; `settings-update-checking` = "Checking…"; `settings-up-to-date` = "hoprs is up to date".

- [ ] **Step 1: Failing tests** in `ui/update.rs`: `card_body_per_phase` (each `Phase` and OS maps to its `CardBody`; `Ready` carries `transfers_to_stop`; the Linux command equals `install_command`); `no_card_without_offer`; `footer_label_available_then_install`; `footer_line_absent_when_up_to_date`. In `ui/settings.rs`: `update_status_text_per_manual_check` for `pub fn update_status(manual: &ManualCheck) -> Option<String>` (`Idle` → `None`, `Checking` → "Checking…", `UpToDate` → "hoprs is up to date", `Available(1.4.0)` → "hoprs 1.4.0 is available", `Failed(e)` → `strings::error(&e)`).
- [ ] **Step 2: Run** `cargo test -p hoprs-app ui::update ui::settings` — FAIL.
- [ ] **Step 3: Implement** the views and strings; pt-BR texts natural ("hoprs { $version } está disponível", "Atualizar", "Novidades", "Fechar e instalar", "Depois"…).
- [ ] **Step 4: Run** `cargo test -p hoprs-app` (catalog parity and key-use tests), the 100k-row bench once (`cargo bench -p hoprs-app --bench ui`, p95 within 20% of the base commit), fmt, clippy — PASS. Run the app in debug with `HOPRS_UPDATE_URL` pointing at a test repository's latest-release API and `HOPRS_UPDATE_AS_VERSION=1.3.0`: the card appears ~5 s after start without taking focus from a focused rename field; ✕ leaves the footer line; restart shows only the line.
- [ ] **Step 5: Commit** `feat(app): update card, footer line and Settings rows`.

### Task 6: Release 1.4.0 prep

**Files:** `Cargo.toml` (`version = "1.4.0"`), `Cargo.lock` (workspace crates), `README.md`, `CHANGELOG.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, `CLAUDE.md`, `site/src/facts.ts` (`version`, `releaseDate`), `scripts/readme-shots.sh` and `site/scripts/capture/Dockerfile` (1.4.0 names, as in `git show 961ca07`), `docs/superpowers/plans/2026-11-12-01-followups.md` (section Plan 15), Create `docs/qa/update-notice-checklist.md`.

- [ ] **Step 1: Docs.**
  - README: replace "hoprs does not update itself" in "Updating" with a lead line pointing at the new **Updates** subsection beside it: what is checked (GitHub's latest release, 5 s after start and once a day, a generic `User-Agent: hoprs/<version>`, nothing else), how to turn it off (Settings → General), what Update does per system (`.deb` → App Center or the shown `sudo apt install` command; `.msi` → hoprs closes, the installer runs; `.dmg` → drag to Applications; the `.zip` and source builds → the release page), and the trust note of ruling 4 (the checksum catches a broken download, not a replaced release; same trust as downloading from the site). Bump the version and asset names.
  - CHANGELOG: `## 1.4.0 — <release date>` with the update notice in user terms.
  - ROADMAP: "Released work" sentence for 1.4.0; under "Later", next to **Signed builds**: "**Signed `SHA256SUMS`** (minisign in `release.yml`), so the update check verifies who published a release, not only that the download is intact"; "Telemetry: not planned" links README#updates.
  - ARCHITECTURE: one paragraph for `src/update/` and `src/opener.rs` (pure logic, blocking client on the blocking pool, backend methods, `app/update.rs`).
  - followups, Plan 15 section: signed `SHA256SUMS`; a first real update 1.4.0 → 1.4.x (QA rows); reduced-motion fade if iced exposes it.
  - QA checklist `docs/qa/update-notice-checklist.md`, NEEDS HUMAN rows with steps: Linux `.deb` real update from 1.4.0 to the next release (App Center opens; the command works); Windows `.msi` (Close and install; with a transfer running; Later then Install); macOS `.dmg` (Gatekeeper "Open Anyway" wording); Windows `.zip` → release page; offline start (nothing shown, log line), Check now offline (inline error); checksum mismatch (debug `HOPRS_UPDATE_URL` at a test repository whose `SHA256SUMS` is wrong → error, no file); the card never steals focus from a terminal or rename; setting off → no request (log).
  - CLAUDE.md: Plan 15 spec/plan/QA paths in "Where things are"; a status line for 1.4.0 prepared (tag after approval).
- [ ] **Step 2: Run** `cargo test --workspace --all-features`, fmt, clippy, `npm test --prefix site` (facts.test.ts checks the version) — PASS.
- [ ] **Step 3: Commit** `release: 1.4.0`; push `update-notice` and open the PR "Update notice (1.4.0)". Tag, release and site deploy happen only after the author approves the merge (ruling 3).
