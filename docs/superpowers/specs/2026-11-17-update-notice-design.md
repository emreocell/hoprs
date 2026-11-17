# Update notice — design

Status: approved by the author (2026-11-17) in conversation: option B ("avisar, baixar e
entregar ao instalador do sistema"), the behaviour playback ("tudo certo"), and the
non-blocking popup card decided with the ui-skills review ("sim, pode").

## Intent

People who install hoprs today never learn that a new version exists unless they visit the
site. The author wants the app to **tell the user a new version is available and make
updating close to one click**, so users don't stay on old versions.

## Goals

- Check, in the background, whether a newer release exists on GitHub.
- When one exists, show a **non-blocking popup card** once per new version, with **Update**,
  **What's new** and dismiss.
- **Update** downloads the right package for this install, verifies it against the release's
  `SHA256SUMS`, and hands it to the operating system's installer.
- After dismissing, a small persistent line in the sidebar footer keeps the update one click
  away until the user updates.
- A setting turns the automatic check off; a "Check now" button checks on demand.

## Non-goals

- Silent self-update (replacing the binary and restarting). Rejected: root-owned `/usr/bin`
  on Linux, per-machine `.msi`, unsigned builds — each would hit a password/UAC/Gatekeeper
  prompt anyway.
- Package-manager channels (apt repository, Homebrew, winget) — issue #21, separate work.
- Pre-releases, update channels, rollbacks, delta updates.
- Release notes rendered inside the app ("What's new" opens the browser).

## Anti-goals

- **Interrupting the user:** no modal, no backdrop, no keyboard-focus theft (the user may be
  typing in a terminal or a rename box), no sound, no repeated nagging for the same version.
- **Anything installed without a click**, or a downloaded file opened when its checksum does
  not match.
- **Slowing startup:** the check never delays the window or the first listing.
- **Noise on failure:** being offline or GitHub being down shows nothing to the user (log
  only), except after a manual "Check now" or a click on Update.
- Breaking the "hoprs collects no data" promise: the request carries no identifier beyond
  what any HTTP request carries (IP, a generic `User-Agent: hoprs/<version>`).

## Approaches considered

**How far the notice goes** (author chose B):
- A. Notify and open the release page in the browser. Simplest; saves only discovery.
- **B. Notify, download, verify and hand to the system installer (chosen).**
- C. Silent self-update. Rejected (non-goals).

**Where the notice appears** (author asked for a popup; decided with ui-skills):
- A modal in the centre — rejected: blocks and steals focus for a non-critical message; the
  project rule is no modal cascades; ui-skills reserves interruption for destructive actions.
- **A non-blocking floating card anchored above the sidebar footer, collapsing into a
  persistent footer line (chosen).**
- A footer line only — too easy to miss for something shown once per version.

## Design

### Checking

- **When:** a few seconds after start-up (never blocking the window) and then every 24 h
  while the app runs; plus **Check now** in Settings → General.
- **Setting:** **Check for updates automatically** in Settings → General, default **on**,
  key `update_check` in the settings file (same rules as other keys: invalid → default,
  unknown keys kept). Off means no automatic request at all; **Check now** still works.
- **Source:** GitHub's latest-release API for `engviniciusbs/hoprs` (stable releases only,
  no drafts or pre-releases). The response gives the tag (`vX.Y.Z`), the release page URL and
  the asset list. A version is newer when it is greater than the running version by semver.
- **Request:** HTTPS with certificate validation, a generic `User-Agent: hoprs/<version>`, a
  short timeout (e.g. 10 s), no cookies or tokens. Rate limits and HTTP errors are treated as
  a failed check.
- **Remembered state** (in the settings file or app state, builder's call): the newest
  version seen and the version the user dismissed, so the card shows once per version and the
  footer line survives restarts.
- **Not checked:** debug builds (`cfg!(debug_assertions)`), and installs where no package
  kind can be determined (see below) still get the notice but **Update** opens the release
  page.

### Which package this install uses

- **Linux:** running from `/usr/bin/hoprs` (or the `.deb`'s install path) → `.deb`
  (`hoprs_<v>-1_amd64.deb`). Otherwise (built from source, other distribution packaging) →
  open the release page.
- **Windows:** executable under the Program Files install directory created by the `.msi` →
  `.msi` (`hoprs_<v>_windows_x64.msi`). Otherwise (the portable `.zip`) → open the release
  page.
- **macOS:** running from an `.app` bundle → `.dmg` (`hoprs_<v>_macos_universal.dmg`).
- Asset names follow the release's stable naming (README, ROADMAP); an expected asset missing
  from the release → open the release page.

### Updating

1. **Download** the asset over HTTPS (following GitHub's redirect to its asset storage) into
   the user's Downloads folder (`dirs::download_dir()`, falling back to the temp dir), as
   `<name>.part`, renamed when complete. Progress shown in the card.
2. **Verify:** download `SHA256SUMS` from the same release, compute the file's SHA-256 and
   compare. Mismatch or missing entry → delete the file, show the error in the card, open
   nothing.
3. **Hand to the installer:**
   - **Windows:** say in the card that hoprs will close so the installer can replace it,
     then open the `.msi` with the system handler and quit hoprs gracefully (the normal
     close path: tabs saved, unsent-edits question if any — if the user cancels the close,
     the installer is still open and will ask to close hoprs).
   - **macOS:** open the `.dmg` (Finder shows the drag-to-Applications window); the card
     says to quit hoprs before replacing it and that Gatekeeper may ask "Open anyway".
   - **Linux:** open the `.deb` with the system handler (App Center on Ubuntu 24.04) and show,
     in the card, the command `sudo apt install ./<file>` (with the full path) and a **Copy**
     button — works where no graphical installer exists.
4. A failure at any step shows an inline error in the card (through `strings::error`) with
   **Try again** and **What's new** still available.

### The card

- Position: floating above the sidebar footer, bottom-left of the window, over the content
  (a layer like the context menu), not covering the footer itself. Width about the sidebar's
  plus a margin; mockup v2 look (paper_2, 1 px `rule_2` border, 4 px radius, no shadow).
- Content: icon + "hoprs X.Y.Z is available" + "You have A.B.C." + **Update** (primary) +
  **What's new** (link) + ✕. During update: progress, then "Checking…", then the per-OS
  message (Linux: command + Copy).
- **Never takes keyboard focus** on appearing and has no backdrop; clicks elsewhere don't
  close it. Appears with a short fade (ease-out, no spring), none under reduced motion if the
  platform exposes it.
- Shown **once per new version**, the first time the app learns of it. ✕ collapses it to the
  footer line. Next start-ups show only the footer line for that version.
- **Footer line:** "↓ X.Y.Z available" above "Import from FileZilla"; clicking it reopens the
  card. Present while a newer version than the running one is known, until the app runs that
  version (or a newer one).
- The palette (Ctrl+K) gets "Update hoprs to X.Y.Z" while a newer version is known, so the
  keyboard can reach it.

### Settings → General

- **Check for updates automatically** (checkbox).
- **Check now** button with an inline result: "hoprs is up to date", "hoprs X.Y.Z is
  available" (and the card opens), or the error.

### Text and docs

- All text in `en.ftl` and `pt-BR.ftl`.
- README: a short "Updates" note (what the app checks, how often, how to turn it off) next to
  the "Updating" section; ROADMAP keeps "Telemetry: not planned" and links that note.
- CHANGELOG under the next version; QA checklist with NEEDS HUMAN rows per OS (real update
  from the previous release on Linux `.deb`, Windows `.msi`, macOS `.dmg`, portable `.zip`
  opening the page; offline; checksum mismatch).

## Tests

- Version comparison (`1.3.1` vs `1.4.0`, `1.10.0` vs `1.9.0`, equal, older, malformed tags).
- Parsing the release JSON (tag, page URL, assets; missing fields → failed check).
- Package choice per OS and install path (pure function over an exe path and OS).
- `SHA256SUMS` parsing and verification (match, mismatch, missing entry).
- Card state machine (FakeBackend): hidden → shown once per version → dismissed → footer
  line; next start shows only the line; a newer version shows the card again; Update →
  downloading → verifying → handed off / error; Check now results; setting off → no
  automatic check effect.
- Settings key round-trip and invalid values.
- No keyboard focus change when the card appears (the focused pane/terminal keeps it).
- Integration (ignored by default, like the Docker tests): a local HTTPS test server or a
  recorded response for the API and an asset, covering redirect and checksum.

## Constraints

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` clean; tests on Linux, macOS and Windows; no literal `/tmp`.
- Only `hoprs-app` changes unless the builder argues otherwise. **Dependencies:** reuse what
  is in the tree (rustls/tokio-rustls, rustls-native-certs, serde_json, sha2, semver, dirs);
  a small HTTP client crate is acceptable only if it builds on the existing rustls stack (no
  OpenSSL, no second TLS stack) — record the choice and its added crates in the plan.
- Mockup v2 look; every text in both catalogs; errors through `strings::error`.

## Builder-check rulings (2026-11-17, the controller's calls)

1. **Transfers when updating on Windows.** Download and verification run whenever the user
   clicks Update. The hand-off step on Windows (which closes hoprs) waits for a second click:
   the card says "Ready to install. hoprs will close." and, if transfers are running or
   queued, adds "N transfers will stop; they continue the next time hoprs starts" (the queue
   is persisted), with **Close and install** and **Later** (keeps the verified file; the
   footer line then reads "Install X.Y.Z"). macOS and Linux don't close hoprs, so no wait.
2. **Testing against another server.** The API base URL and the "running version" can be
   overridden only in debug builds (`HOPRS_UPDATE_URL`, `HOPRS_UPDATE_AS_VERSION`), and a
   debug build checks only when `HOPRS_UPDATE_URL` is set. Release builds always use GitHub
   and their real version. Automated tests use a fake backend and recorded responses; the
   manual "real update" QA rows run when the release after this one exists.
3. **Version and release.** The feature ships as **1.4.0**; the plan ends with the release
   PR, and tag/publish happen only after the author approves the merge. 1.4.0 itself has
   nothing older that knows how to update; the first real update path is 1.4.0 → 1.4.x.

4. **Integrity, not authenticity (the author's choice, 2026-11-17: "só integridade").** The
   checksum comes from the release's own `SHA256SUMS`: it catches a corrupted or truncated
   download, not a release replaced by someone with access to the GitHub account. That is the
   same trust as downloading from the site today; the README "Updates" note says so. Signing
   `SHA256SUMS` (e.g. minisign in `release.yml`) is recorded as a follow-up next to signed
   builds in the ROADMAP.

## Left to the builder

The HTTP client choice within the dependency rule; where the remembered versions live; the
exact check delay after start-up; card width and spacing; how the per-OS open-with-handler is
implemented (the app already opens folders with the system opener); wording within the
meaning above.
