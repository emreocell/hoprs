# Plan 12 checklist: folder compare

One run per release that touches `hoprs_remote::compare` or the compare panel. AUTOMATED rows
run in `cargo test --workspace`; NEEDS HUMAN rows need a person and a real display.

## AUTOMATED

| Check | Result |
| --- | --- |
| Walk, diff rules, tolerance, ignore patterns, limit pause/resume, cooperative stop, unreadable subfolder (`crates/hoprs-remote/src/compare.rs` tests, `MemFs`, `FaultFs`, `LocalFs`) | Pass |
| FTP walk lists one folder at a time; a pane listing waits for at most one compare listing | Pass |
| App state machine: auto on/off, debounce, folder-change cancel, stale results dropped by run id, connection drop, tab close, Open suppression | Pass |
| Upload/Download from the panel use the overwrite policy per file; a server that keeps an old mtime shows the file identical after the transfer | Pass |
| Settings `compare_auto` and `compare_ignore` round-trip, defaults, unknown values | Pass |
| Both catalogs have every `compare-*` key (`cargo test -p hoprs-app`) | Pass |
| Docker: a planted tree compared over SFTP and over FTP (`LIST`, minute precision) finds exactly the only-local, only-remote, folder-only-remote and size-different entries (`crates/hoprs-remote/tests/compare.rs`, `--ignored`) | Pass |

## NEEDS HUMAN

**Run 2026-11-16 (author, Linux, release build of `main` with the UI in pt-BR):** the main flows passed — switching the language live in Settings, folder compare over SFTP and FTP with upload/download from the panel, identical files not shown as newer on FTP, enabled and disabled panel buttons distinguishable. Rows below not covered by that run stay open (macOS and Windows, large-tree limit, keyboard-only use).

1. **A real project over SFTP.** Open a local project and its deployed copy on an SSH
   site. Expect: the panel lists plausible differences, `node_modules`/`.git` are absent, the
   summary is correct, Upload selected makes the row "Queued" then "done", and the next run
   shows it identical.
2. **A real project over FTP.** Same on an FTP/FTPS site (ideally one without `MFMT`).
   Expect: no file you just uploaded comes back as "local is newer"; no long freeze in the
   panes while it walks.
3. **Large tree.** Compare folders with more than 10 000 entries on a side. Expect: the walk
   pauses, the panel says how many items it has seen and that the list is partial,
   "Compare everything" continues, "Stop" keeps the partial list.
4. **Panel at 1280 x 800, English and Portuguese (Brasil).** Header, group headers, rows,
   footer and buttons are not cut off, the panel height follows the window, and the long
   Portuguese labels fit. Also at the 960 px minimum width.
5. **Keyboard.** Tab into the panel; arrows move the selection, Space toggles a row,
   Enter opens it (both panes go to the item's folder), and the row stays visible while
   moving through a long list.
6. **Enabled and disabled buttons are clearly distinguishable.** Select rows that exist on
   both sides, then only one-sided ones: "Upload selected" and "Download selected" must look
   clearly enabled or disabled on a real display (parked finding: the outline button's
   disabled state may be too close to the enabled one; see the Plan 12 follow-ups).
7. **Automatic toggle.** With automatic on, change folders quickly in either pane: only the
   last pair is compared. Turn it off in the panel and in Settings (they stay in step): no
   run starts on folder change; Compare still works.
8. **Connection drop mid-run.** Disconnect the network while a run walks. Expect: the panel
   says the comparison stopped, with Compare again; no hang, no crash.
9. **Nothing is deleted.** After several uploads and downloads, confirm nothing was removed
   from either side.
