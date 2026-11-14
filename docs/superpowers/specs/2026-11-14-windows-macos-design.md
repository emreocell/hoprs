# hoprs on Windows and macOS — design

Status: approved (2026-11-14). The author delegated the open decisions ("pode ir decidindo"); the calls made after the builder check are listed at the end.

## Intent

hoprs 1.0.0 ships only as a `.deb` for Ubuntu. It is open source, and the author
wants other people on Windows and macOS to be able to download and use it. The
next release, **1.1.0**, publishes installers for all three systems from the same
tag, in the same GitHub Release, and the site at https://hoprs.online offers the
right one to each visitor.

## Goals

- Every `v*` tag publishes, all or nothing:
  - `hoprs_<v>-1_amd64.deb` (unchanged),
  - `hoprs_<v>_macos_universal.dmg` (a `hoprs.app` for Apple Silicon and Intel),
  - `hoprs_<v>_windows_x64.msi` (installer: Start menu entry, uninstall from Settings),
  - `hoprs_<v>_windows_x64.zip` (portable: `hoprs.exe` and nothing to install),
  - one `SHA256SUMS` covering all four.
- The app works on both systems as it does on Linux: credentials in the native
  keyring (Windows Credential Manager, macOS Keychain), ssh-agent, FileZilla import,
  remote editing with the system's default editor, file dialogs.
- CI runs the unit tests and clippy on Windows and macOS on every PR, so the
  portability stays real.
- The README, the site and the release notes explain how to open an unsigned app
  on each system.

## Non-goals (this round)

- Code signing and notarization (the author chose no paid signing). The macOS
  binary is only **ad-hoc signed** (`codesign -s -`), which Apple Silicon requires
  to run at all.
- Homebrew, winget, Scoop (a follow-up once the binaries exist).
- Windows ARM64, Intel-only or ARM-only macOS builds, the Mac App Store,
  Microsoft Store.
- Auto-update.
- Pageant (PuTTY's agent) on Windows; only the OpenSSH agent.
- Docker integration tests and the perf job on Windows/macOS (they stay on Linux).
- Manual testing before release: the author said ship it and look afterwards. The
  release notes say, in one line, that the Windows and macOS builds have not yet
  been tried on real machines and ask for issues.

## Anti-goals

- A release where one system's file is missing or broken but the others are
  published (keep the existing all-or-nothing draft flow).
- Breaking the Linux `.deb` or its reproducibility (pinned toolchain, remapped paths,
  the clean-Ubuntu check).
- A console window opening next to the app on Windows.
- An installer that loses the user's sites or keyring entries on upgrade or uninstall.

## Constraints

- Toolchain stays pinned by `rust-toolchain.toml` (`1.99.1`) on every OS.
- Only `hoprs-app` depends on iced; the crate graph in the main spec §4.1 does not change.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  clean, now on Linux, Windows and macOS.
- Tests use `tempfile`, never a literal `/tmp`.
- Package metadata lives in `crates/hoprs-app/Cargo.toml`, like `[package.metadata.deb]`.
- Repo is public; no secrets are needed (no signing).

## Approaches considered

1. **`cargo-packager` for macOS and Windows, `cargo-deb` stays for Linux.** One tool
   builds `.app` + `.dmg` and `.msi` (WiX) from `[package.metadata.packager]`; a script
   per OS under `scripts/` calls it, like `package-deb.sh` calls `cargo-deb`.
   Cost: one more tool with its own conventions. **Chosen by the author.**
2. Hand-written scripts: shell + `Info.plist` + `hdiutil` for macOS, `cargo-wix` and a
   `.wxs` for Windows. Full control, more files to maintain.
3. `cargo-dist`. Dropped: aimed at CLI tools, does not build a GUI `.app`/`.dmg`, and
   would replace the working `release.yml`.

## Design

### Code changes for portability

- **Build on Windows and macOS at all.** The workspace has never been compiled there;
  fix whatever `cfg` code fails (`#[cfg(unix)]` paths in `hoprs-vault/src/atomic.rs`,
  `hoprs-app/src/backend/edit.rs` and `backend/mod.rs` already have fallbacks; any
  missing one is added).
- **No console window:** `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`
  in `hoprs-app`'s `main.rs`. `--version` and `--help` output from a console then needs
  `AttachConsole(ATTACH_PARENT_PROCESS)` so they still print when run from a terminal.
- **Keyring:** `keyring` 4.2's default features already include the Windows and Apple
  stores (checked).
  A CI test on each OS stores, reads and deletes one entry (both runners have a usable
  store).
- **ssh-agent on Windows:** `try_agent` is `#[cfg(unix)]` only today. Add a Windows path:
  when `SSH_AUTH_SOCK` is unset, the OpenSSH agent's named pipe
  `\\.\pipe\openssh-ssh-agent` (russh 0.65 `connect_named_pipe`).
  macOS needs nothing: launchd sets `SSH_AUTH_SOCK`.
- **FileZilla import path:** macOS `~/.config/filezilla/sitemanager.xml` (FileZilla uses
  the XDG path there, not `Application Support`). Windows (`%APPDATA%\FileZilla`, via
  `dirs::config_dir`) and Linux are already right.
- **Remote-edit editor.** Today `EDITOR_DEFAULT = "xdg-open"` (`settings.rs`) on every
  OS, launched by `launch_editor` in `backend/edit.rs`, so remote editing fails out of
  the box on Windows and macOS. New default value `system` (the setting stays a string;
  the Settings UI shows it as "System default"):
  - Linux: `xdg-open <file>` (same behaviour as today; a saved `xdg-open` keeps working).
  - macOS: `open -t <file>` (the default *text* editor, so `.conf` or extensionless files
    open in something useful).
  - Windows: `ShellExecuteW` with the `open` verb (no console window); when the file
    type has no association (`SE_ERR_NOASSOC`) fall back to `notepad.exe <file>`.
  - Any other value is a custom command line, split with POSIX `shlex` on Linux/macOS and
    with Windows command-line rules (as `CommandLineToArgvW`) on Windows, so
    `C:\Program Files\Notepad++\notepad++.exe` works.
  The folder-opening branch in `effects.rs` stays as it is.
- Data and config directories follow `dirs` (`%APPDATA%\hoprs`,
  `~/Library/Application Support/hoprs`); nothing to change, but the README lists them.

### Icons

Generate `crates/hoprs-app/assets/icon/hoprs.icns` and `hoprs.ico` from the existing
`hoprs.svg` with a script (`scripts/make-icons.sh`), and commit the outputs, so the
release jobs do not need image tools. The `.exe` embeds the `.ico` (a `build.rs`
resource on Windows only, e.g. `winresource`), and the window icon is set as on Linux.

### Packaging

- `[package.metadata.packager]` in `crates/hoprs-app/Cargo.toml`: product name `hoprs`,
  identifier `online.hoprs.hoprs`, the icons, the description already used for the `.deb`,
  licence, homepage https://hoprs.online.
- `scripts/package-macos.sh`: builds `aarch64-apple-darwin` and `x86_64-apple-darwin`
  release binaries, joins them with `lipo`, packages the `.app`, ad-hoc signs it
  (`codesign --force --deep -s -`), builds the `.dmg` (app + Applications link), prints
  the path. Minimum macOS: 11.
- `scripts/package-windows.sh` (Git Bash on the runner): release build for
  `x86_64-pc-windows-msvc`, `.msi` through cargo-packager/WiX, and the `.zip` with the
  `.exe`, `LICENSE-*` and a short `README.txt`. The MSI has a fixed upgrade code so
  installing 1.2 replaces 1.1, and uninstalling never touches `%APPDATA%\hoprs` or the
  keyring. Install scope: per-machine (standard Program Files install, one UAC prompt).
- Both scripts remap builder paths out of the binary (`--remap-path-prefix`), as the
  `.deb` build does, and pin cargo-packager's version.
- A light check per package, in `release.yml` **and** as `package-macos` /
  `package-windows` jobs in `ci.yml` on every PR and push to `main` (like today's `.deb`
  `package` job, with the cargo cache), so a broken installer shows up in the PR, not at
  tag time: the macOS job mounts the `.dmg`, runs
  `hoprs.app/Contents/MacOS/hoprs --version`, `lipo -archs` shows both, `codesign -v` passes;
  the Windows job installs the `.msi` silently (`msiexec /i … /qn`), runs
  `hoprs.exe --version`, uninstalls, and checks the `.zip`'s `hoprs.exe --version`.

### Release workflow

`release.yml` becomes four jobs:

1. `linux` — today's steps up to the `.deb` and its clean-Ubuntu check; uploads it as an artifact.
2. `macos` (`macos-latest`) — `package-macos.sh`, the check, artifact.
3. `windows` (`windows-latest`) — `package-windows.sh`, the check, artifacts.
4. `publish` (`needs` all three) — downloads them, writes `SHA256SUMS`, then the existing
   draft → upload → download-and-verify → publish flow, with the draft deleted on failure.

The tag/version check and the release-notes extraction run first, in a small job the
others need, so a bad tag fails in seconds. Release notes gain a fixed "Installing"
block per OS, including the unsigned-app steps (below).

### Dry runs and pre-release tags

- `release.yml` gains `workflow_dispatch`: a dry run on any branch that runs the three
  build-and-check jobs and uploads the packages as workflow artifacts, but never creates
  a release (the `publish` job runs only on a tag). This is how the workflow is proven
  before tagging 1.1.0.
- Pre-release tags (`vX.Y.Z-rc.N`) still publish as prereleases. The MSI version is the
  numeric `X.Y.Z` (MSI cannot carry `-rc.N`), so an rc and its final share an MSI version:
  going from an rc to the final needs an uninstall first, which the rc's notes say. The
  site does not deploy for prereleases (above).

### CI

`ci.yml`'s `test` matrix becomes `[ubuntu-latest, windows-latest, macos-latest]`, and
clippy runs in each. `--ignored` (Docker) tests stay Linux-only. A PR that breaks
Windows or macOS fails.

### Unsigned-app instructions (README, site, release notes)

- **macOS:** open the `.dmg`, drag hoprs to Applications, open it once; macOS refuses;
  go to System Settings → Privacy & Security → "Open Anyway". Or, in Terminal,
  `xattr -dr com.apple.quarantine /Applications/hoprs.app`.
- **Windows:** run the `.msi`; on "Windows protected your PC" choose "More info" →
  "Run anyway".

### Site

- `site/src/facts.ts` gains the three new asset names, derived from the version.
- **The site never links to a release that does not exist.** `facts.version` stays tied
  to the workspace version (its test stays), but `site.yml`'s `deploy` job first checks
  that release `v<version>` is published and not a prerelease (`gh release view`); if not,
  it skips the deploy (the build and tests still run). `site.yml` also runs when the
  Release workflow completes successfully (`workflow_run`), so the site goes live with the
  new links right after a release is published.
- The install well gets a tab per OS (Linux / macOS / Windows), pre-selected from the
  visitor's platform (`navigator.userAgentData?.platform` or the user agent), defaulting
  to Linux when unknown and working with scripting off (all three visible or the
  prerendered default). Each tab: download button to the release asset, the unsigned-app
  steps, the checksum link.
- Copy and the FAQ stop saying Linux-only.

### Version and docs

- Workspace version `1.1.0`, `CHANGELOG.md` section.
- README: install sections for the three systems, data locations per OS.
- `docs/qa/release-1.1-checklist.md`: NEEDS HUMAN rows for Windows and macOS (install,
  first launch past Gatekeeper/SmartScreen, keyring save and read, ssh-agent, FileZilla
  import, remote edit opens the default editor, uninstall keeps data).
- `CLAUDE.md`: target platforms now Linux, Windows, macOS; the decision "Target
  platform v1: Linux (Ubuntu)" is updated by this spec.

## Left to the builder

- Exact cargo-packager version and config keys; the WiX template if the default needs
  changing.
- How `--version` reaches a parent console on Windows.
- The icon tool chain inside `make-icons.sh`.
- Task order.

## Calls made after the builder check (the author delegated them)

1. Remote-edit default editor: new `system` value per OS (`xdg-open`, `open -t`,
   `ShellExecuteW` + Notepad fallback); custom commands split with each OS's rules.
   Cost if wrong: a Settings model change later.
2. macOS and Windows package checks run in `ci.yml` on every PR, not only at tag time.
   Cost: longer CI (free for a public repo).
3. The site deploys only when `v<version>` is published and not a prerelease, plus once
   after each successful release. Cost: one more trigger in `site.yml`.
4. Dry runs through `workflow_dispatch` instead of rc tags; rc tags still work, MSI
   version is numeric `X.Y.Z`. Cost: an rc → final upgrade on Windows needs an uninstall.
5. macOS asks for Keychain access again after each update, because an ad-hoc signature
   changes with every build. Accepted: one line in the README, the release notes and the
   QA checklist ("click Always Allow"). Cost: a prompt per saved password per update,
   until paid signing.
