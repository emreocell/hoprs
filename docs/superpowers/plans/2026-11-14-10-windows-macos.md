# Windows and macOS release (1.1.0) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every `v*` tag publishes, all or nothing, a `.deb`, a universal macOS `.dmg`, a Windows `.msi` and a portable Windows `.zip`, and the app works on Windows and macOS as it does on Linux.

**Architecture:** Portability fixes in the crates (console subsystem, agent pipe, FileZilla path, editor default), CI on three OSes, `cargo-packager` for `.app`/`.dmg`/`.msi` driven by `scripts/package-{macos,windows}.sh`, `release.yml` split into per-OS build jobs plus one `publish` job, and the site offering one download per OS.

**Tech Stack:** Rust 1.99.1 (pinned), iced, russh 0.65, keyring 4.2, cargo-packager (pinned), WiX (via cargo-packager), GitHub Actions (`ubuntu-24.04`, `macos-latest`, `windows-latest`), Vite + React site.

**Spec:** `docs/superpowers/specs/2026-11-14-windows-macos-design.md`

## Global Constraints

- Toolchain pinned by `rust-toolchain.toml` (`1.99.1`) on every OS; workflows use `dtolnay/rust-toolchain@1.99.1` (the CI drift check enforces it).
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean on Linux, Windows and macOS.
- Only `hoprs-app` depends on iced; the crate graph does not change.
- Tests use `tempfile`, never a literal `/tmp`.
- Asset names: `hoprs_<v>-1_amd64.deb`, `hoprs_<v>_macos_universal.dmg`, `hoprs_<v>_windows_x64.msi`, `hoprs_<v>_windows_x64.zip`, plus one `SHA256SUMS` covering all four.
- Bundle identifier `online.hoprs.hoprs`; product name `hoprs`; minimum macOS 11; homepage `https://hoprs.online`.
- macOS: ad-hoc signature only (`codesign --force --deep -s -`); no paid signing or notarization anywhere.
- MSI: fixed upgrade code, per-machine scope, MSI version = numeric `X.Y.Z` of the workspace version; uninstall never touches `%APPDATA%\hoprs` or the keyring.
- No console window on Windows in release builds.
- Builder paths remapped out of every binary (`--remap-path-prefix`), as `scripts/package-deb.sh` does.
- Release stays all or nothing: a draft, every asset uploaded and verified, then published; the draft is deleted on failure.
- Commits: `git -c user.name="$(git log -1 --format=%an)" -c user.email="$(git log -1 --format=%ae)" commit …`, message ending `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; never change git config.
- Windows/macOS code cannot be run locally: CI on the pushed branch `plan-10-windows-macos` is the gate for it. Pushing that branch (never `main`) and reading its CI runs is allowed; a draft PR may be opened for CI.

## Review Focus

1. A custom editor at a path with spaces on Windows (`"C:\Program Files\Notepad++\notepad++.exe"`) must launch; Task 5 pins it.
2. No OpenSSH agent running on Windows (pipe absent) must skip the agent silently and go on to keys/password, never fail the connection; Task 3 pins it.
3. `hoprs.exe --version` run from `cmd`/PowerShell must print the version line even though the binary is a GUI-subsystem app; Task 2 pins it.
4. Installing a newer `.msi` over an older one, and uninstalling, must keep `%APPDATA%\hoprs`; Task 8's check pins it.
5. A site visitor with an unknown platform or scripting off must still reach every download (Linux pre-selected); Task 11 pins it.

---

### Task 1: CI on Windows and macOS, and the workspace builds there

**Files:**
- Modify: `.github/workflows/ci.yml` (job `test`: matrix `[ubuntu-latest, windows-latest, macos-latest]`; add a `clippy` step per OS: `cargo clippy --workspace --all-targets --all-features -- -D warnings`)
- Modify: whatever crate code fails to compile, lint or test on Windows/macOS (expected around `#[cfg(unix)]` code in `crates/hoprs-vault/src/atomic.rs`, `crates/hoprs-vault/src/lib.rs`, `crates/hoprs-app/src/backend/edit.rs`, `crates/hoprs-app/src/backend/mod.rs`, `crates/hoprs-app/benches/startup.rs`)
- Test: `crates/hoprs-vault/tests/keyring_smoke.rs` (new)

**Interfaces:**
- Produces: a green `test` job on all three OSes; later tasks add tests that run there.

- [ ] **Step 1:** Change the matrix and add clippy per OS; `--ignored` (Docker) tests stay in `integration` (Linux).
- [ ] **Step 2:** Add `keyring_smoke` test: on Windows and macOS only (`#[cfg(any(windows, target_os = "macos"))]`), store an entry under service `hoprs-ci-smoke` and a random account, read it back equal, delete it, read fails with not-found. Linux has no Secret Service on the runner: the test file compiles to nothing there.
- [ ] **Step 3:** Push the branch, read the failures (`gh run view --log-failed`), fix each in the crate with a portable fallback (never by disabling a test, unless the test is about Unix permissions; then `#[cfg(unix)]` on that test with a one-line reason). Repeat until `test` is green on all three.
- [ ] **Step 4:** Locally on Linux: `cargo fmt --all --check`, clippy and `cargo test --workspace --all-features` still pass.
- [ ] **Step 5: Commit** `ci: test and lint on Windows and macOS`.

### Task 2: No console window on Windows, `--version` still prints

**Files:**
- Modify: `crates/hoprs-app/src/main.rs`
- Modify: `crates/hoprs-app/Cargo.toml` (`windows-sys` with `Win32_System_Console`, Windows-only target dependency)

**Interfaces:**
- Produces: `fn attach_parent_console()` in `main.rs` (no-op off Windows).

- [ ] **Step 1:** `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]` at the top of `main.rs`.
- [ ] **Step 2:** Before printing for `Cli::Version`, `Cli::Help` and `Cli::Unknown`, call `attach_parent_console()`: on Windows `AttachConsole(ATTACH_PARENT_PROCESS)`, ignoring failure (double-clicked: no console, nothing to print to). `Cli::Run` never attaches.
- [ ] **Step 3:** CI check in the `test` job on `windows-latest`, after the tests: `cargo build --release -p hoprs-app` then `target/release/hoprs.exe --version | grep -q '^hoprs '` (pins Review Focus 3).
- [ ] **Step 4: Commit** `fix(app): no console window on Windows`.

### Task 3: ssh-agent over the OpenSSH named pipe on Windows

**Files:**
- Modify: `crates/hoprs-ssh/src/auth.rs:304-388` (replace the `#[cfg(not(unix))]` stub with a Windows implementation)
- Modify: `crates/hoprs-ssh/src/config.rs:44` (doc: on Windows `None` means `SSH_AUTH_SOCK`, else `\\.\pipe\openssh-ssh-agent`)
- Test: in `auth.rs` tests

**Interfaces:**
- Produces: `fn agent_endpoint(configured: Option<&Path>, env_sock: Option<OsString>) -> Option<AgentEndpoint>` with `enum AgentEndpoint { Socket(PathBuf), Pipe(String) }` (pure, testable on every OS; `Pipe` only built on Windows). `pub const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";`

- [ ] **Step 1: Tests** `agent_endpoint`: configured path wins; non-empty env wins next; empty or unset env gives `None` on Unix and `Some(Pipe(WINDOWS_AGENT_PIPE))` on Windows (`#[cfg(windows)]` assertion). A Windows-only test `missing_pipe_is_skipped`: connecting to a pipe name that does not exist yields `Ok(None)` from the connect helper and records the skip in `agent_skipped`, no error (pins Review Focus 2).
- [ ] **Step 2:** Refactor `try_agent` so the identity loop is shared, and only the connection differs: Unix socket via `AgentClient::connect_uds`, Windows pipe via `AgentClient::connect_named_pipe` (russh 0.65). Same budget, same skip rules as today.
- [ ] **Step 3:** Linux tests + clippy pass locally; CI on Windows green.
- [ ] **Step 4: Commit** `feat(ssh): ssh-agent through the OpenSSH pipe on Windows`.

### Task 4: FileZilla's sitemanager.xml on macOS

**Files:**
- Modify: `crates/hoprs-import/src/filezilla.rs:72-75`

**Interfaces:**
- Produces: `default_sitemanager_path()` unchanged signature; new pure `fn sitemanager_path_for(os: &str, home: Option<PathBuf>, config: Option<PathBuf>) -> Option<PathBuf>` it delegates to.

- [ ] **Step 1: Tests** `sitemanager_path_for("macos", Some("/Users/a"), Some("/Users/a/Library/Application Support"))` == `/Users/a/.config/filezilla/sitemanager.xml`; `("linux", …, Some("/home/a/.config"))` == `/home/a/.config/filezilla/sitemanager.xml`; `("windows", …, Some(r"C:\Users\a\AppData\Roaming"))` == `…\Roaming\filezilla\sitemanager.xml` (Windows paths are case-insensitive, FileZilla's folder is `FileZilla`; keep lowercase join). Build paths with `PathBuf::join` in the tests so they pass on every OS.
- [ ] **Step 2:** Implement; `default_sitemanager_path()` passes `std::env::consts::OS`, `dirs::home_dir()`, `dirs::config_dir()`. Fix the doc comment.
- [ ] **Step 3: Commit** `fix(import): FileZilla's macOS config path`.

### Task 5: Remote-edit `system` editor per OS, Windows command splitting

**Files:**
- Modify: `crates/hoprs-app/src/settings.rs:34` (`EDITOR_DEFAULT = "system"`), and wherever the Settings UI shows the editor field (show `system` as "System default" placeholder/help text)
- Modify: `crates/hoprs-app/src/backend/edit.rs:1632-1677` (`editor_argv`, `launch_editor`)
- Modify: `crates/hoprs-app/Cargo.toml` (Windows-only: `windows-sys` features `Win32_UI_Shell`, `Win32_Foundation`, `Win32_UI_WindowsAndMessaging`; `winsplit` or an equivalent `CommandLineToArgvW`-rules splitter)

**Interfaces:**
- Produces: `pub enum EditorLaunch { System, Argv(Vec<OsString>) }`; `pub fn editor_launch(command: &str, path: &Path) -> Result<EditorLaunch>` (`"system"` trimmed, case-insensitive → `System`; else split with `split_command`); `fn split_command(command: &str) -> Option<Vec<String>>` (POSIX `shlex` on Unix, Windows rules on Windows); `fn system_argv(path: &Path) -> Vec<OsString>` (Linux `["xdg-open", path]`, macOS `["open", "-t", path]`). `launch_editor` keeps its signature.

- [ ] **Step 1: Tests**: `editor_launch("system", p)` and `(" System ", p)` → `System`; `("xdg-open", p)` → `Argv(["xdg-open", p])` (old saved value keeps working); `("code --wait", p)` → `Argv(["code","--wait",p])`; unmatched quote → error "The editor command has an unmatched quote"; blank → error "No editor command is set". `#[cfg(target_os = "macos")]` `system_argv(p) == ["open","-t",p]`; `#[cfg(target_os = "linux")]` `== ["xdg-open",p]`. `#[cfg(windows)]` `split_command(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst"#) == [r"C:\Program Files\Notepad++\notepad++.exe", "-multiInst"]` and `split_command(r"C:\Tools\vim.exe") == [r"C:\Tools\vim.exe"]` (backslashes kept; pins Review Focus 1). Settings default test: a fresh `Settings::default().editor_command == "system"`.
- [ ] **Step 2:** Implement. `launch_editor`: `System` on Linux/macOS spawns `system_argv` the way `Argv` is spawned today; on Windows calls `ShellExecuteW(null, "open", path, null, null, SW_SHOWNORMAL)`, and when it returns `SE_ERR_NOASSOC` (31) spawns `notepad.exe <path>`; any other failure is the existing "Cannot start the editor" error. `Argv` on Windows spawns with `CREATE_NO_WINDOW` not set (a GUI editor) but no inherited console. `editor_argv` is replaced by `editor_launch` at its call sites.
- [ ] **Step 3:** Local Linux tests + clippy; CI green on all three.
- [ ] **Step 4: Commit** `feat(app): system default editor on every OS`.

### Task 6: Icons for macOS and Windows

**Files:**
- Modify: `scripts/icons.sh` (also writes `crates/hoprs-app/assets/icon/hoprs.icns` and `hoprs.ico` from `hoprs.svg`)
- Create: `crates/hoprs-app/assets/icon/hoprs.icns`, `crates/hoprs-app/assets/icon/hoprs.ico` (committed outputs)
- Create/Modify: `crates/hoprs-app/build.rs` (Windows only: embed `hoprs.ico` and version info with `winresource` as a Windows-only build dependency)

**Interfaces:**
- Produces: the two icon files at those paths, used by Tasks 7 and 8.

- [ ] **Step 1:** Extend `icons.sh`: `.ico` with 16, 24, 32, 48, 64, 128, 256 px PNG frames; `.icns` with 16–1024 px (`ic04`…`ic10` sizes) built without macOS tools (e.g. `png2icns` from `icnsutils`, or a small Python writer); document the tools the script needs at its top.
- [ ] **Step 2:** Test in `crates/hoprs-app/src/icon.rs` tests: `hoprs.ico` starts with bytes `00 00 01 00` and lists a 256 px entry; `hoprs.icns` starts with `icns` and its length field equals the file size.
- [ ] **Step 3:** `build.rs`: on `CARGO_CFG_TARGET_OS == "windows"` only, compile the resource; Linux builds unchanged (check `cargo build -p hoprs-app` locally is unaffected).
- [ ] **Step 4: Commit** `feat(app): macOS and Windows icons`.

### Task 7: macOS `.dmg`

**Files:**
- Modify: `crates/hoprs-app/Cargo.toml` (`[package.metadata.packager]`: `product-name = "hoprs"`, `identifier = "online.hoprs.hoprs"`, icons, description from `[package.metadata.deb]`, licence, homepage, `macos.minimum-system-version = "11.0"`)
- Create: `scripts/package-macos.sh`, `scripts/check-macos.sh`
- Modify: `.github/workflows/ci.yml` (job `package-macos` on `macos-latest`, `Swatinem/rust-cache@v2`, runs both scripts)

**Interfaces:**
- Produces: `scripts/package-macos.sh` prints the absolute path of `target/packager/hoprs_<v>_macos_universal.dmg`; `scripts/check-macos.sh <dmg>` exits 0 or fails with `FAIL  <reason>` lines (same style as `check-deb.sh`). Version via `scripts/workspace-version.sh`.

- [ ] **Step 1:** `package-macos.sh`: pinned cargo-packager install if missing; `cargo build --release --locked -p hoprs-app` for `aarch64-apple-darwin` and `x86_64-apple-darwin` with `--remap-path-prefix` for `$HOME`, `$CARGO_HOME` and the repo; `lipo -create` into `target/universal-apple-darwin/release/hoprs`; cargo-packager builds the `.app` from that binary; `codesign --force --deep -s -` the `.app`; `.dmg` with the app and an `Applications` link; rename to the asset name.
- [ ] **Step 2:** `check-macos.sh`: mount read-only, `lipo -archs …/hoprs.app/Contents/MacOS/hoprs` contains `arm64` and `x86_64`; `codesign -v` passes; `Info.plist` `CFBundleIdentifier == online.hoprs.hoprs`, `CFBundleShortVersionString == <v>`, `LSMinimumSystemVersion == 11.0`; `…/hoprs --version` prints `hoprs <v>`; `strings` finds none of the builder paths; detach.
- [ ] **Step 3:** Add the CI job; push; green.
- [ ] **Step 4: Commit** `feat(release): macOS universal .dmg`.

### Task 8: Windows `.msi` and portable `.zip`

**Files:**
- Modify: `crates/hoprs-app/Cargo.toml` (`[package.metadata.packager]` Windows/WiX part: fixed `upgrade-code` GUID generated once and written here, per-machine)
- Create: `scripts/package-windows.sh` (Git Bash), `scripts/check-windows.ps1`
- Modify: `.github/workflows/ci.yml` (job `package-windows` on `windows-latest`)

**Interfaces:**
- Produces: `package-windows.sh` prints two absolute paths, one per line: `…/hoprs_<v>_windows_x64.msi` then `…/hoprs_<v>_windows_x64.zip`. `check-windows.ps1 -Msi <path> -Zip <path>` exits 0 or throws with the failing check.

- [ ] **Step 1:** `package-windows.sh`: release build for `x86_64-pc-windows-msvc` (remapped paths); cargo-packager `.msi` with MSI version = numeric `X.Y.Z` (strip any `-…`); `.zip` holding `hoprs.exe`, `LICENSE-MIT`, `LICENSE-APACHE` and a `README.txt` (what it is, the SmartScreen step, data at `%APPDATA%\hoprs`, homepage).
- [ ] **Step 2:** `check-windows.ps1`: create `%APPDATA%\hoprs\keep.txt`; `msiexec /i <msi> /qn /l*v install.log` exit 0; `"$env:ProgramFiles\hoprs\hoprs.exe" --version` prints `hoprs <v>`; a Start menu shortcut exists; reinstall the same `.msi` with `REINSTALL=ALL REINSTALLMODE=vomus` succeeds (upgrade path); `msiexec /x <msi> /qn` exit 0; the exe is gone and `keep.txt` is still there (pins Review Focus 4); the `.zip`'s `hoprs.exe --version` prints the same.
- [ ] **Step 3:** Add the CI job; push; green.
- [ ] **Step 4: Commit** `feat(release): Windows .msi and portable .zip`.

### Task 9: Release workflow for three systems, with a dry run

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `scripts/release-notes.sh` (appends the fixed "Installing" block)

**Interfaces:**
- Consumes: the scripts of Tasks 7 and 8, `scripts/package-deb.sh`, `scripts/check-deb.sh`.

- [ ] **Step 1:** Jobs: `prepare` (tag/version check and `notes.md`, both only on a tag; outputs `version`), `linux`, `macos`, `windows` (each `needs: prepare`, builds with its script, runs its check, uploads its files with `actions/upload-artifact@v4`), `publish` (`needs` the three, `if: startsWith(github.ref, 'refs/tags/v')`): download all, `SHA256SUMS` over the four files, then today's draft → upload → download-and-verify → publish → delete-draft-on-failure steps, unchanged in behaviour, over all five files.
- [ ] **Step 2:** `on:` gains `workflow_dispatch`; on a dispatch `prepare` skips the tag check and takes the version from `scripts/workspace-version.sh`, and `publish` does not run.
- [ ] **Step 3:** `release-notes.sh` appends, after the CHANGELOG section, a block "Installing" with: Linux (`sudo apt install ./hoprs_<v>-1_amd64.deb`), macOS (drag to Applications; first open is refused; System Settings → Privacy & Security → Open Anyway, or `xattr -dr com.apple.quarantine /Applications/hoprs.app`; Keychain asks again after each update, choose Always Allow), Windows (`.msi`: "Windows protected your PC" → More info → Run anyway; or the portable `.zip`), and the line "The Windows and macOS builds are new and not yet tried on real machines; please open an issue if something is off." Test it the way the script is tested today (run it for a sample version and grep the block).
- [ ] **Step 4:** Run the dry run on the branch: `gh workflow run release.yml --ref plan-10-windows-macos`; all three build jobs green, `publish` skipped, artifacts present.
- [ ] **Step 5: Commit** `ci(release): build and publish for Linux, macOS and Windows`.

### Task 10: Site: one download per system, deploy only after a release

**Files:**
- Modify: `site/src/facts.ts` (asset names and URLs for `.dmg`, `.msi`, `.zip`)
- Modify: `site/src/sections/Install.tsx`, `site/src/components/InstallWell.tsx` and its test, `site/src/sections/Hero.tsx` (sentence and the primary button), `site/src/sections/Faq.tsx` (no "Linux only"; unsigned-app steps)
- Modify: `.github/workflows/site.yml` (deploy gating, `workflow_run`)
- Test: `site/src/facts.test.ts`, `InstallWell.test.tsx`, `App.test.tsx`, `prerender.test.ts`

**Interfaces:**
- Produces: `facts.dmgName/dmgUrl`, `facts.msiName/msiUrl`, `facts.zipName/zipUrl`; `export function detectOs(ua: string, platform?: string): "linux" | "macos" | "windows"` in `site/src/platform.ts` (unknown → `"linux"`).

- [ ] **Step 1: Tests**: `facts` names equal the Global Constraints patterns for `facts.version`; `detectOs` maps a macOS UA (`Macintosh; Intel Mac OS X`), `platform "macOS"`, a Windows UA (`Windows NT 10.0`), a Linux UA, and `""` → `linux`; the install section renders three tabs Linux / macOS / Windows, the macOS one with the `.dmg` link and the Gatekeeper steps, the Windows one with `.msi` and `.zip` links and the SmartScreen step; the prerendered HTML contains all three download URLs (scripting-off reachability, pins Review Focus 5); the hero's primary button follows `detectOs` after hydration and is the Linux `.deb` in the prerender.
- [ ] **Step 2:** Implement; hero sentence becomes "A fast, native file client for Linux, macOS and Windows." (≤ 12 words rule from the Hallmark stamp holds: check `sloplint` reports no new FAIL beyond the two recorded ones).
- [ ] **Step 3:** `site.yml`: `deploy` gets a first step that runs `gh release view "v$VERSION" --json isDraft,isPrerelease` (version from `facts.ts` or `scripts/workspace-version.sh`) and skips the remaining steps unless the release exists, is not a draft and not a prerelease. Add `on: workflow_run: workflows: [Release], types: [completed]`, with the job running only when `github.event.workflow_run.conclusion == 'success'` and checking out `main`.
- [ ] **Step 4:** `npm run lint && npm run typecheck && npm run build && npm test` pass.
- [ ] **Step 5: Commit** `feat(site): downloads for Linux, macOS and Windows`.

### Task 11: Version 1.1.0 and the docs

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version = "1.1.0"`), `Cargo.lock`, `site/src/facts.ts` (`version = "1.1.0"`)
- Modify: `CHANGELOG.md` (section `1.1.0`), `README.md` (install for three systems, unsigned-app steps, data locations per OS: `~/.config/hoprs` etc. as `dirs` gives them, `%APPDATA%\hoprs`, `~/Library/Application Support/hoprs`), `CLAUDE.md` (target platforms Linux, Windows, macOS; Plan 10 in "Where things are"; status), `crates/hoprs-app/Cargo.toml` metainfo/description if it says Linux-only
- Create: `docs/qa/release-1.1-checklist.md` (NEEDS HUMAN rows for Windows and macOS: install, first launch past Gatekeeper/SmartScreen, keyring save and read, Keychain "Always Allow" after update, ssh-agent, FileZilla import, remote edit opens the default editor, upgrade, uninstall keeps data)
- Modify: `docs/superpowers/plans/2026-11-12-01-followups.md` (section Plan 10: signing/notarization, Homebrew/winget/Scoop, Windows ARM64, Pageant)

- [ ] **Step 1:** Bump and write; `facts.test.ts` passes with 1.1.0; `cargo test --workspace` passes.
- [ ] **Step 2: Commit** `chore: 1.1.0 for Linux, macOS and Windows`.
