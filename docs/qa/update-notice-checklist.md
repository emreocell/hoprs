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
9. **pt-BR.** Switch Settings › Language to Português and repeat 1, 4 and 7: "hoprs 1.3.1
   está disponível", "Você tem a 1.3.0.", "Atualizar", "Novidades", "1.3.1 disponível",
   "Procurar atualizações automaticamente", "Procurar agora", "O hoprs está atualizado".
10. **Narrow window.** At the minimum window size (960 × 640) the card fits and the
    command wraps inside it.
