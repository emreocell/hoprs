# Plan 11 checklist: Brazilian Portuguese UI

One run per release that touches the catalogs. AUTOMATED rows run in `cargo test --workspace`;
NEEDS HUMAN rows need a person and a real display.

## AUTOMATED

| Check | Result |
| --- | --- |
| Every key `strings.rs` looks up exists in `en.ftl` (`strings_rs_keys_exist`) | Pass |
| Every key in `en.ftl` is used, and no `error-*` key is dead (`every_en_key_is_used`) | Pass |
| `en.ftl` and `pt-BR.ftl` have the same keys and the same `{ $var }` names | Pass |
| Every error code in the source has an `error-<code>` message in both catalogs (`error_codes_exist_in_both_catalogs`) | Pass |
| Views show errors through `strings::error` (`views_show_errors_through_strings_error`) | Pass |
| pt-BR values differ from English except the allowed list (`pt_br_values_differ_from_english`) | Pass |
| Numbers and dates in pt-BR (`4,1 KB`, `1.000`, `11 nov 14:02`); OS locale mapping; unknown `language` setting kept | Pass |
| Queue schema v2 migrates to v3; old errors show their English message; known skip reasons are translated | Pass |
| Layout in pt-BR rendered offscreen at 1280, 1240, 1100 and 960 px (sidebar, file tab, queue, settings, site form, host-key dialog): no cut labels after the fix in `ui/fit.rs` | Pass: one-off render (Plan 11 task 10 report), not repeated in CI |
| The `.deb` still passes `scripts/check-deb.sh` with the `pt_BR` desktop entry keys | NEEDS HUMAN: `desktop-file-validate` runs in `check-deb.sh` on a clean Ubuntu; run it once for the new keys |

## NEEDS HUMAN

**Run 2026-11-16 (author, Linux, release build of `main` with the UI in pt-BR):** the main flows passed — switching the language live in Settings, folder compare over SFTP and FTP with upload/download from the panel, identical files not shown as newer on FTP, enabled and disabled panel buttons distinguishable. Rows below not covered by that run stay open (macOS and Windows, large-tree limit, keyboard-only use).

1. **First launch in Portuguese.** Start with `LANG=pt_BR.UTF-8 hoprs` (or an OS set to
   Portuguese) and no `settings.toml`. Expect: Portuguese everywhere, Settings shows
   "Idioma: Sistema".
2. **Switch live, both ways.** In Settings choose English, then Português (Brasil), then
   Sistema, with a site tab, the queue and the log open. Expect: every label changes at
   once, no restart, nothing half translated; the choice survives a restart.
3. **Every screen without truncation.** In Portuguese at 1280 x 800 and at the 960 px
   minimum width: sidebar, a file tab (local and remote toolbars, column headers), the
   queue (all four tabs, a conflict row), the log, Settings, the site and workspace
   forms, the import flow, the host-key and certificate dialogs, the palette, a terminal
   tab. Expect: no cut or overlapping labels; the remote toolbar keeps its key hints at
   1280 px and shortens (no hints) from about 1240 px.
4. **Accents render.** File names and paths in JetBrains Mono (`relatório-março.pdf`,
   `configuração`) and UI text in Inter show accents and `ç` correctly, in the file
   list, the terminal and the queue.
5. **Errors read in Portuguese.** A wrong password, a refused connection (a closed port)
   and a changed host key each show a Portuguese message, inline.
6. **Dates and numbers.** A listing shows `14 nov 2023`, sizes like `4,1 KB`, a queue with
   `1.000` items groups with a dot.
7. **Windows and macOS with the OS in Portuguese.** First launch picks Portuguese; the
   native file dialogs (import, export, restore) have their own language (the OS's), the
   dialog titles are Portuguese.
8. **Linux launcher.** On a Portuguese desktop the application menu shows "Cliente SSH e
   FTP" and the tooltip text in Portuguese after installing the `.deb`.
9. **A queue written by 1.1.** Open 1.2 on a profile that has a 1.1 queue with a failed
   item: it migrates, the failed row still shows its message (English), new errors follow
   the language.
