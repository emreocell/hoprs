# Update notice checklist (Plan 15, 1.4.0)

One section per task; later tasks append theirs.

## Task 5: the card, the footer line and the Settings rows (2026-11-17)

| Check | Result |
| --- | --- |
| The card draws the state of the update: offer, download progress, Checking…, Windows Ready (with the transfers that stop), Windows installing, Linux command, macOS steps, error | Pass: `card_body_per_phase` |
| Before the release is known (only the remembered version), the card shows the check it waits for, or its error; once the release is known, a failed check no longer shows | Pass: `card_check_shows_only_before_the_release_is_known` |
| No card while collapsed or while no newer version is known | Pass: `no_card_without_offer` |
| The footer line reads "1.4.0 available", "Install 1.4.0" after Later on Windows; none when up to date; hidden while the card is open | Pass: `footer_label_available_then_install`, `footer_line_absent_when_up_to_date` |
| Settings → General shows Check now's result: Checking…, up to date, "hoprs X.Y.Z is available", the error | Pass: `update_status_text_per_manual_check` |
| Windows: a cancelled close leaves "Close hoprs", which asks for the normal close once, never a second installer; the opener's error shows with Try again | Pass: `cancelled_close_offers_close_hoprs_again`, `close_for_install_needs_the_installer_open`, `windows_waits_for_second_click` |
| Linux: "Run this command to install:" when the system installer did not open | Pass: `linux_intro_follows_the_open_result` |
| Every new text in English and pt-BR | Pass: `every_en_key_is_used` and the catalog parity tests |
| A new always-present layer costs no frame time (100k rows) | Pass: p95 0.80–0.84 ms against 0.71–0.76 ms at the base commit (noise between runs ±7%), criterion mean unchanged (577–594 µs both) |
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
8. **Update, per OS** (with a real newer release; on Linux from the `.deb`): progress bar
   and "Downloading… X of Y", "Checking…", then Linux: "Opened in the system installer. Or
   run:" with the `sudo apt install …` command in mono and **Copy** (pastes the command);
   macOS: the Quit / Replace / Open Anyway text; Windows: "Ready to install. hoprs will
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

## Release 1.4.0: real updates and edge cases

The first real update path is 1.4.0 → 1.4.x: 1.4.0 is the first version that can update, so
these rows wait for the release after it. Until then use the debug variables of the section
above (`HOPRS_UPDATE_URL`, `HOPRS_UPDATE_AS_VERSION`) against a test repository.

### AUTOMATED

| Check | Result |
| --- | --- |
| Release JSON parsing, version comparison, package choice per OS | Pass: unit tests in `update/` (`cargo test -p hoprs-app update`) |
| `SHA256SUMS` parsing and the checksum check (mismatch, missing line, truncated file) | Pass: unit tests in `update/` |
| Card once per version, footer line, Later / Install, cancelled close, busy and closing guards | Pass: app tests in `app/tests/update.rs` |
| Settings off means no check; Check now works either way | Pass: app tests in `app/tests/update.rs` |
| Every new text in English and pt-BR | Pass: catalog parity tests |

### NEEDS HUMAN

| Check | Result |
| --- | --- |
| Linux: real update 1.4.0 → next release from the `.deb`: App Center opens, the copied `sudo apt install` command works, hoprs keeps running | NEEDS HUMAN |
| Windows `.msi`: Update downloads and verifies; "Ready to install. hoprs will close."; with a transfer running the warning names it; **Later** then footer "Install X.Y.Z" then **Close and install**; the installer replaces the version | NEEDS HUMAN |
| Windows: cancel the close (unsent remote edit, answer Cancel): the card reads "Close hoprs", no second installer starts | NEEDS HUMAN |
| macOS `.dmg`: Update opens the disk image; the Quit / Replace / Open Anyway text matches what Gatekeeper shows | NEEDS HUMAN |
| Portable `.zip` on Windows: Update opens the release page, nothing is downloaded | NEEDS HUMAN |
| Offline start: no card, no error on screen, one line in the log; Check now offline shows the inline error | NEEDS HUMAN |
| Checksum mismatch: debug build with `HOPRS_UPDATE_URL` at a test repository whose `SHA256SUMS` is wrong: an error on the card, no file left in the updates folder, no installer opened | NEEDS HUMAN |
| The card never takes focus: type in a terminal tab and in a rename field while it appears | NEEDS HUMAN |
| The card fits inside the sidebar at 960 × 640 (command wraps, buttons visible) | NEEDS HUMAN |
| pt-BR: card, footer line, Settings rows and errors read well | NEEDS HUMAN |
| Settings off: restart, no request in the log; Check now still works | NEEDS HUMAN |

## Task 6: release 1.4.0, real-update rows (2026-11-17)

1.4.0 has nothing older that knows how to update, so the first real update path is
1.4.0 → 1.4.x. These rows wait for that release; until then use the debug variables above.

| Check | Result |
| --- | --- |
| AUTOMATED: release JSON parsing, version compare, `SHA256SUMS` parsing, package choice per OS, download and verify, app state machine, card/footer/Settings texts in en and pt-BR | Pass: the `update` and `app::tests::update` unit tests (`cargo test --workspace`) |
| AUTOMATED: the version in `Cargo.toml` matches `site/src/facts.ts` | Pass: `facts.test.ts` |
| NEEDS HUMAN: Linux `.deb`, real update 1.4.0 → 1.4.x. Update opens App Center; the shown `sudo apt install` command works; `hoprs --version` prints the new version | Open |
| NEEDS HUMAN: Windows `.msi`, real update. After Update the card says "Ready to install. hoprs will close."; with a transfer running it warns that N transfers stop; Later keeps "Install X.Y.Z" in the footer; **Close and install** closes hoprs and the installer runs | Open |
| NEEDS HUMAN: Windows, answer Cancel to the unsent-edits question after Close and install: the card reads "Close hoprs", which asks again and never opens a second installer | Open |
| NEEDS HUMAN: macOS `.dmg`, real update. The disk image opens; the steps (Quit, Replace, Open Anyway) are right for Gatekeeper | Open |
| NEEDS HUMAN: portable `.zip` (Windows) and a source build: Update opens the release page, nothing is downloaded | Open |
| NEEDS HUMAN: offline start shows nothing (a log line only); Check now offline shows the inline error in red | Open |
| NEEDS HUMAN: checksum mismatch. Debug build with `HOPRS_UPDATE_URL` at a test repository whose `SHA256SUMS` is wrong: an error, no file kept, no installer opened | Open |
| NEEDS HUMAN: the card never takes focus while typing in a terminal tab or a rename field | Open |
| NEEDS HUMAN: at 960 × 640 the card stays inside the sidebar | Open |
| NEEDS HUMAN: pt-BR texts on the card, the footer line and Settings | Open |
| NEEDS HUMAN: "Check for updates automatically" off, restart: no request (log line only) | Open |
