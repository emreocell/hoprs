# Update notice checklist (Plan 15, 1.4.0)

Automated rows first, then the steps that need a person on a real display.

## The card, the footer line and the Settings rows (2026-11-17)

| Check | Result |
| --- | --- |
| The card draws the state of the update: offer, download progress, Checking…, Windows Ready (with the transfers that stop), Windows installing, Linux command, macOS steps (or, when the disk image did not open, the error with Try again), error | Pass: `card_body_per_phase` |
| A finished download opens a card closed with ✕ (Linux, macOS); macOS Try again opens the verified file again; a newer release clears an old failure | Pass: `hand_off_opens_a_collapsed_card`, `macos_open_failure_tries_again`, `newer_release_clears_a_failure` |
| Before the release is known (only the remembered version), the card shows the check it waits for, or its error; once the release is known, a failed check no longer shows | Pass: `card_check_shows_only_before_the_release_is_known` |
| No card while collapsed or while no newer version is known | Pass: `no_card_without_offer` |
| The footer line reads "1.4.0 available", "Install 1.4.0" after Later on Windows; none when up to date; hidden while the card is open | Pass: `footer_label_available_then_install`, `footer_line_absent_when_up_to_date` |
| Settings → General shows Check now's result: Checking…, up to date, "hoprs X.Y.Z is available", the error | Pass: `update_status_text_per_manual_check` |
| Windows: a cancelled close leaves "Close hoprs", which asks for the normal close once, never a second installer; the opener's error shows with Try again | Pass: `cancelled_close_offers_close_hoprs_again`, `close_for_install_needs_the_installer_open`, `windows_waits_for_second_click` |
| Linux: "Run this command to install:" when the `.deb` did not open | Pass: `linux_intro_follows_the_open_result` |
| Every new text in English and pt-BR | Pass: `every_en_key_is_used` and the catalog parity tests |
| The always-present card layer stays within the plan's 20% frame budget (100k rows) | Pass: p95 0.80–0.84 ms against 0.71–0.76 ms at the base commit (noise between runs ±7%), criterion mean unchanged (577–594 µs both) |
| The card's look, position, focus and click-through on a real display, in English and pt-BR | NEEDS HUMAN: steps below |

### NEEDS HUMAN

Start a debug build against a release newer than the one it pretends to run:
`HOPRS_UPDATE_URL=https://api.github.com/repos/engviniciusbs/hoprs/releases/latest HOPRS_UPDATE_AS_VERSION=1.3.0 cargo run -p hoprs-app`.

1. **Card.** About 5 s after start a card appears bottom left, over the sidebar, just
   above "Import from FileZilla" (which stays visible and clickable): download icon,
   "hoprs 1.3.1 is available", "You have 1.3.0.", **Update** (white), **What's new** (text),
   ✕. Look: `paper_2`, 1 px border, 4 px corners, no shadow, no fade.
2. **No focus taken.** Before the 5 s are up, start a rename in a file pane (F2) and keep
   typing: the card appears and every key still goes to the rename field.
3. **Click-through.** With the card shown, click files, tabs and the sidebar: they work,
   and the card stays. A right-click menu, the Ctrl+K palette and a dialog cover the card.
4. **✕.** Click ✕ (tooltip "Dismiss"): the card goes and "↓ 1.3.1 available" shows above
   "Import from FileZilla". Click it: the card comes back.
5. **Restart.** Close hoprs and start it again with the same variables: no card, only the
   footer line.
6. **What's new** opens the release page in the browser.
7. **Settings.** Settings → General: "Check for updates automatically" (checked) with its
   hint, **Check now**; click it: "Checking…" (button dashed and disabled), then "hoprs 1.3.1
   is available" and the card opens. Turn the network off and click again: the error, in
   red. With `HOPRS_UPDATE_AS_VERSION` set to the latest release: "hoprs is up to date".
8. **Update, per OS** (add `HOPRS_UPDATE_AS_PACKAGE=deb`, `dmg` or `msi` to the variables
   above; see "Update and hand-off" below): progress bar and "Downloading… X of Y",
   "Checking…", then Linux: "Opened with the default app for .deb files. Or run:" with the
   `sudo apt install …` command in mono and **Copy** (pastes the command); macOS: "Close the
   hoprs window, then drag hoprs to Applications…"; Windows: "Ready to install. hoprs will
   close." (with "N transfers will stop…" while a transfer runs), **Close and install**,
   **Later** (the footer line then reads "Install 1.3.1").
   With a remote file open in an editor and edited (unsent), click **Close and install**,
   then answer Cancel to the unsent-edits question: the card reads "The installer is open.
   Close hoprs to finish." with **Close hoprs**, which asks the same question again (no
   second installer); ✕ leaves "Install 1.3.1" in the footer.
9. **pt-BR.** Switch Settings › Language to Português and repeat 1, 4 and 7: "hoprs 1.3.1
   está disponível", "Você tem a 1.3.0.", "Atualizar", "Novidades", "1.3.1 disponível",
   "Procurar atualizações automaticamente", "Procurar agora", "O hoprs está atualizado".
10. **Narrow window.** At the minimum window size (960 × 640) the card fits and the
    command wraps inside it.

## Update and hand-off, release 1.4.0 (2026-11-17)

1.4.0 is the first version that can update itself, so the first real update path is
1.4.0 → 1.4.x. Until that release exists, a debug build exercises the same download,
check and hand-off against the latest real release (1.3.1 today; set
`HOPRS_UPDATE_AS_VERSION` below it):

```sh
HOPRS_UPDATE_URL=https://api.github.com/repos/engviniciusbs/hoprs/releases/latest \
HOPRS_UPDATE_AS_VERSION=1.3.0 \
HOPRS_UPDATE_AS_PACKAGE=deb \
cargo run -p hoprs-app
```

`HOPRS_UPDATE_AS_PACKAGE` is `deb` on Linux, `dmg` on macOS, `msi` on Windows (PowerShell:
`$env:NAME = "value"` for each, then `cargo run -p hoprs-app`). All three variables are read by
debug builds only; an unknown package value is ignored with a log line. The file goes to the
Downloads folder; delete it afterwards. On Windows **Close and install** really installs
that release's `.msi`; use a test machine.

**Checksum mismatch.** Asset URLs must be under
`https://github.com/engviniciusbs/hoprs/releases/download/`, so the mismatch uses real
assets in the wrong place: save the 1.3.1 release JSON
(`curl -s https://api.github.com/repos/engviniciusbs/hoprs/releases/tags/v1.3.1`), change the
`.deb` asset's `browser_download_url` and `size` to those of the `.msi` asset (keep its
`name`), put the file in a secret gist and point `HOPRS_UPDATE_URL` at its raw URL
(`https://gist.githubusercontent.com/…/raw/…`). Update then downloads the `.msi` bytes under
the `.deb` name, and `SHA256SUMS` rejects them.

### AUTOMATED

| Check | Result |
| --- | --- |
| Release JSON parsing (asset URLs pinned to this project's release downloads), version comparison, package choice per OS | Pass: `release_json_parses`, `release_json_missing_fields_fails`, `asset_urls_are_pinned_to_the_release_downloads`, `versions_compare_by_semver`, `package_per_os_and_path` |
| `SHA256SUMS` parsing and the checksum check (mismatch, missing line, truncated file) | Pass: `update::sums` and `update::download` unit tests |
| Download folder choice, private fallback, stale part files of this user removed (never a fresh one) | Pass: `download_dir_choice`, `private_dir_is_private`, `stale_part_files_are_removed`, `part_file_names` |
| `HOPRS_UPDATE_AS_PACKAGE` parsed strictly, read by debug builds only | Pass: `package_override_parses_strictly`, `package_override_follows_the_build` |
| Card once per version, footer line, Later / Install, cancelled close, busy and closing guards | Pass: `app::tests::update` |
| Settings off means no check; Check now works either way | Pass: `app::tests::update` |
| The version in `Cargo.toml` matches `site/src/facts.ts` | Pass: `facts.test.ts` |
| Every new text in English and pt-BR | Pass: catalog parity tests |

### NEEDS HUMAN

| Check | Result |
| --- | --- |
| Linux `.deb` (debug build with `HOPRS_UPDATE_AS_PACKAGE=deb` now, the installed 1.4.0 → 1.4.x later): Update opens App Center (or the default app for `.deb` files); the copied `sudo apt install` command works; `hoprs --version` prints the new version; hoprs keeps running | NEEDS HUMAN |
| Windows `.msi` (`HOPRS_UPDATE_AS_PACKAGE=msi`): Update downloads and verifies; "Ready to install. hoprs will close."; with a transfer running it warns that N transfers stop; **Later** leaves "Install X.Y.Z" in the footer; **Close and install** closes hoprs and the installer replaces the version | NEEDS HUMAN |
| Windows: answer Cancel to the unsent-edits question after **Close and install**: the card reads "Close hoprs", which asks again and never opens a second installer | NEEDS HUMAN |
| macOS `.dmg` (`HOPRS_UPDATE_AS_PACKAGE=dmg`): Update opens the disk image; the "Close the hoprs window, then drag hoprs to Applications" steps work, and Gatekeeper's prompt (if any) matches the Open Anyway text | NEEDS HUMAN |
| Portable `.zip` on Windows and a source build without `HOPRS_UPDATE_AS_PACKAGE`: Update opens the release page, nothing is downloaded | NEEDS HUMAN |
| Offline start: no card, no error on screen, one line in the log; Check now offline shows the inline error in red | NEEDS HUMAN |
| Checksum mismatch (the gist above): an error on the card, no file and no part file left in the Downloads folder, no installer opened | NEEDS HUMAN |
| The card never takes focus: type in a terminal tab and in a rename field while it appears | NEEDS HUMAN |
| At 960 × 640 the card stays inside the sidebar (command wraps, buttons visible) | NEEDS HUMAN |
| pt-BR: card, footer line, Settings rows and errors read well | NEEDS HUMAN |
| "Check for updates automatically" off, restart: no request (log line only); Check now still works | NEEDS HUMAN |
