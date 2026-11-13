# Plan 6 visual checklist

Manual check against spec §8 and mockup v2 (`docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`).
One line per check; later tasks append theirs.

## Task 4: terminal tab (2026-11-13)

Setup: release build, Docker sshd (`127.0.0.1:2222`, alice, key `docker/fixtures/id_ed25519`), site opened as Both.
A throwaway driver (not committed) sent the app's own messages (keys as `TerminalMsg::Key`, window resize,
search, select + Ctrl+Shift+C / V) and took iced `window::screenshot` captures; it ran with an isolated
`HOME`/`XDG_CONFIG_HOME` so no real profile, keyring or `known_hosts` was touched. Under native Wayland the
captures went stale once the window was in the background (no frame callbacks), so the final pass ran under
XWayland (1000×700 logical @2x after the resize; 1280×800 before).

| Check | Result |
| --- | --- |
| Tab title | `docker ssh` + mono `ssh` tag; the files tab `docker ssh` + `sftp`. The remote OSC title (`alice@…: ~`) shows as a tooltip only, never replaces the name | Pass (title state checked; tooltip hover is NEEDS HUMAN: hover the terminal tab) |
| Prompt and colours | JetBrains Mono at 13 px; bash prompt in its green/blue; bold, dim, underline, inverse, 256-colour green, 24-bit orange and a 64-step 24-bit gradient bar all render; `中文` takes two cells each | Pass. Italic renders upright (no italic face is embedded) |
| Full-screen program | `top` (no htop/vim in the container): header, reverse-video column bar across the width, process table; `q` returns to the prompt | Pass |
| 50 MB of output | `head -c 37500000 /dev/urandom \| base64 -w 200` (50 MB): finished (server `real 0.96 s`), the shell never overflowed, the model ends on the `time` output and the prompt. Meanwhile a 100 ms UI timer kept ticking: 100 ticks during and after the flood, gaps 100–124 ms (median 101); switching to the files tab and back mid-flood worked | Pass |
| 50 MB `cat` (fix round 2) | After the consumer credits (2 batches of at most 2 MiB in flight per shell): `time cat /tmp/big.txt` (50 250 000 bytes of 200-column base64) on Docker sshd, release build, native Wayland. The shell stayed open (no "Output was too fast"), the model reached `FLOOD-DONE` and the prompt 0.77 s after the command (server `real 0.51 s`); a 50 ms UI timer ticked throughout (median 82 ms, max 124 ms) and switching to the files tab mid-flood worked; `echo still-alive` ran afterwards | Pass |
| Resize | Window 1280×800 → 1000×700: grid 129×36 → 93×30 in the model at once; `stty size` printed `36 129` before and `30 93` after (PTY follows after the 100 ms debounce) | Pass. Drag-resizing with the mouse is NEEDS HUMAN: drag the window edge, run `stty size` |
| Search | Ctrl+Shift+F opens the bar under the terminal (mono field, white border while focused, Previous / Next / Close on the right); typing `stty` highlights the newest match after the 150 ms pause; the terminal cursor turns to an outline while the field has the keyboard; Escape / Close gives the keyboard back | Pass |
| Copy and paste | A line selection (`accent_wash` background) copied with Ctrl+Shift+C; `echo pasted:` + Ctrl+Shift+V ran as `echo pasted:30 93` → `pasted:30 93` (system clipboard round trip) | Pass via messages. NEEDS HUMAN with a real mouse and keyboard: drag to select, double click (word), triple click (line), Ctrl+Shift+C, paste into another app, Ctrl+Shift+V from another app |
| Shell exit | `exit 3`: thin `paper_2` bar "Shell exited (code 3)" in the error colour with an outlined "Restart shell"; the scrollback stays | Pass |
| Settings | New "Terminal" section: Font size stepper (10–24, default 13), Scrollback lines (1000…100000, default 10000, "Applies to terminals opened from now on."), Theme "hoprs dark" | Pass |
| Typing by hand | Real keyboard typing, Ctrl+C / Ctrl+K / arrows in bash, Tab completion, vim-like apps | NEEDS HUMAN: open a terminal tab, type `ls -la`, Tab-complete a path, Ctrl+R history, Ctrl+K kills the rest of the line (the palette must NOT open), Ctrl+Shift+K does nothing in the shell |
| Wheel | Scrolling back through history; in `less` the wheel moves the pager (alternate scroll) | NEEDS HUMAN: `seq 1 500` then wheel up/down; `seq 1 500 \| less` then wheel |
| Connection lost | Covered by `connection_lost_banner_then_new_shell` (banner, Reconnect, new shell, "— reconnected —") | NEEDS HUMAN visually: `docker compose -f docker/compose.yml restart sshd` with a terminal open, wait for the "Connection lost" bar, click Reconnect, check the "— reconnected —" line and that the new shell works |
| Overflow | Covered by `overflow_shows_inline_and_reopen` and the hoprs-ssh pump test; did not trigger with the real 50 MB flood (the tab keeps up) | Pass (not reproducible by hand on a fast machine) |

## Task 6: technical log panel (2026-11-13)

Setup: debug build under XWayland (2560×1600 physical @2x), isolated `HOME`/`XDG_*`, default filter (no `RUST_LOG`).
A throwaway driver (not committed) emitted eight sample lines like the real ones (connection transitions with
`since_last_request_secs`, a keepalive warning, an error, a debug line), sent the panel's own messages and took
iced `window::screenshot` captures 2.5 s apart (shorter waits caught half-drawn frames).

| Check | Result |
| --- | --- |
| Closed by default | Status bar shows "Log" at the right; no panel until it is clicked; "Hide" closes it and gives the room back | Pass |
| Panel open | Under the transfer queue: 1 px strong rule, 36 px head ("Log", Error / Warn / Info / Debug segments with a hint tooltip, "Filter log" field, "N lines", Copy shown, Clear (tooltip: panel only), Hide), six 30 px rows in JetBrains Mono: time `HH:MM:SS.mmm` muted, level (WARN/ERROR in the error colour, INFO in ink_2), target muted, message + `key=value` fields in ink, clipped on one line; scrolled to the newest line on open | Pass |
| Default filter | The startup produced no iced / wgpu / sctk lines (the debug sample line was filtered out too: 7 sample lines + 1 `winit` "Guessed window scale factor" = 8 lines) | Pass. `winit` itself still logs one info line under XWayland (not in the brief's list) |
| Level filter | Warn: only the WARN and ERROR lines, "2 lines"; the chosen segment on `paper_3` | Pass |
| Text filter | `reconnect`: the three matching lines (case-insensitive, over level, target, message and fields), "3 lines"; `zzz`: "No lines match", Copy shown disabled. Fix round 1: "Copy shown" label rechecked in a new capture | Pass |
| Copy shown / Clear | Covered by `panel_filters_by_level_and_text` (clipboard text, one line each, oldest first; Clear empties, new lines still come) | Pass via messages. NEEDS HUMAN: click Copy shown, paste into an editor; hover Clear and the level buttons for their hints |
| Following | New lines scroll the list only while it is at its end | Pass in tests. NEEDS HUMAN: scroll up with the panel open while connecting a site; the list must stay put |

## Task 8: remote editing UI (2026-11-13)

Setup: release build under XWayland (2560×1600 @2x, captures scaled to 1280×800), isolated `HOME`/`XDG_*`
(`known_hosts` seeded with `ssh-keyscan`), Docker sshd (`127.0.0.1:2222`, alice, key), site opened as Files at
`/home/alice/edit-demo` (`notes.txt`, folder `sub/`). A throwaway driver (not committed) sent the app's own
messages and took iced `window::screenshot` captures. The editor was a script standing in for a person saving
three times in a row: `sh -c 'sleep 1; for i in 1 2 3; do printf "save %s\n" "$i" >> "$0"; sleep 0.15; done'`.

| Check | Result |
| --- | --- |
| Three saves in a row | 3 saves 150 ms apart → exactly one `remote edit uploaded` (size 27 = `hello` + the 3 lines), ~1.8 s after F4 (1 s script delay + 0.3 s of saves + 500 ms debounce); the server holds all three lines | Pass |
| Edit action | Remote toolbar: `Edit F4` after Download (local pane has none). On `sub/` it looks off; F4 (or a click) shows "Folders can't be edited" in the thin bar above the table with Dismiss; hovering shows the same as a tooltip. Links and files over 50 MB likewise (unit test) | Pass. Tooltip hover NEEDS HUMAN |
| Remote toolbar width | With Edit the full remote toolbar needs ~540 px: at the default 1280 px window the remote toolbar is compact (no key hints), full from 1346 px. The local toolbar is unchanged | Pass (visual change, for Task 11 to judge) |
| Strip and toast | Thin `paper_2` strip across the files tab: "Editing notes.txt — saved 13:42"; toast "Uploaded notes.txt" at the bottom right of the center (`dialog_box` style, no shadow), gone after 3 s, no animation | Pass |
| Conflict choice | After a foreign change on the server, the next save: "notes.txt: Changed on the server since you opened it" with outlined Overwrite / Keep remote / Save a copy on the right, inline, no dialog; the server's change is kept | Pass |
| Close the last tab | × on the tab: inline at the top of the tab, "Files are open for editing", `/home/alice/edit-demo/notes.txt  changed on the server`, "Unsent changes are copied to …/hoprs/recovered", white "Upload pending changes and close", outlined "Close and keep local copies" and "Cancel"; Cancel keeps the tab | Pass |
| Close the window | Window close: the center shows "Some remote edits are not uploaded", the file and its state, where copies go, "Upload and quit" / "Quit and keep local copies" / "Cancel", "The window stays open until you choose. Nothing is deleted."; Cancel returns. "Quit and keep local copies" then closed the app: the recovered copy `…/recovered/<site id>/home/alice/edit-demo/notes.txt.local-<ts>` holds the local version and the edit file is still in `$XDG_RUNTIME_DIR/hoprs/edit/1/…` | Pass |
| Settings | New "Remote editing" section: "Editor" (mono field, default `xdg-open`), the hint; emptied, "Type a command" in the error colour and the last valid command stays in force | Pass |
| Real GUI editor | `gedit` and VS Code are not installed here; `xdg-open` maps text files to GNOME Text Editor | NEEDS HUMAN: with the Docker site open, set Editor to `gnome-text-editor` (or `gedit`, `code --wait`), F4 on a text file, save three times quickly (Ctrl+S ×3): the log panel shows one "remote edit uploaded" per 500 ms of quiet and one toast; then edit on the server (`docker exec hoprs-test-sshd-1 sh -c 'echo x >> /home/alice/<file>'`), save again, check the inline choice and each button |
| Keep remote / Save a copy | Covered by `remote_changed_choice_inline` and the backend tests | NEEDS HUMAN visually: click "Keep remote", check the line says to reload the editor and shows "Your version was saved to …"; reload the editor |
| Upload and quit | Covered by `window_close_with_pending_edits_asks_before_saving` (edits settled before the save) | NEEDS HUMAN: with a saved, unsent change and the network cut (`docker compose -f docker/compose.yml stop sshd`), close the window, click "Upload and quit": after ≤10 s the question stays with "1 file could not be uploaded"; start sshd, click again: the app closes and the server has the change |

### Task 8 fix round 1 (2026-11-13)

Not re-captured (the driver was not rebuilt); covered by tests, visual check pending.

| Check | Result |
| --- | --- |
| Strip and toast | File names now a separate JetBrains Mono span ("Editing `notes.txt` — saved 13:42", "Uploaded `notes.txt`"); the strip also shows in terminal tabs of the session | NEEDS HUMAN: open a file for editing, look at the strip in the files tab and in a terminal tab of the same site, and at the toast |
| Kept notice | After a close that kept unsent edits (delete the site, or "Close and keep local copies"): a `paper_2` bar above the center "Kept 1 unsent edit as a recovered copy in" + the recovered root in mono + Dismiss; visible from any tab and form; upload toasts don't replace it | NEEDS HUMAN visually |
| Close workspace | With an unsent edit on one of its sites: the center question "Some remote edits of this workspace are not uploaded", Upload pending changes and close / Close and keep local copies / Cancel; an open site form comes back after Cancel | NEEDS HUMAN visually |
| Delete site | The delete confirmation adds "1 file of this site is open for editing: its unsent changes will be kept as a recovered copy, never deleted." | NEEDS HUMAN visually |
| Stop and keep | While "Upload…" runs, only "Stop and keep local copies" and "Uploading… (Stop to keep local copies instead)"; Stop returns at once, and a file whose upload was under way shows "Server copy of `<path>` may be incomplete" in the kept notice | NEEDS HUMAN: stop sshd, save, close the window, click Upload and quit, then Stop and keep local copies |

## Task 9: Ctrl+K palette (2026-11-13)

Release build under XWayland at 2560×1600 @2x, isolated `HOME`/`XDG_*` (`known_hosts` seeded with `ssh-keyscan`), Docker
sshd (`127.0.0.1:2222`, alice, key) saved as `shop-web` (Open as: both), plus `nas`, `blog` and a workspace `Shop`. A
throwaway driver (not committed) sent the app's own messages and took iced `window::screenshot` captures.

| Check | Result |
| --- | --- |
| Open (terminal tab active) | Box at the top center over the dimmed window (dialog backdrop, `dialog_box` surface, 1 px `rule_2`, 4 px corners, no shadow); search field focused (white outline). Rows: Paste `Ctrl Shift V`, Find in scrollback `Ctrl Shift F`, New terminal tab, New files tab, Close tab + `shop-web · terminal` in mono, the two tabs in mono, New site, Open workspace…, Toggle queue, Toggle log, Import from FileZilla (12 rows, the rest by typing). Group tag muted on the right, keys in key caps | Pass |
| Filtering | `sh`: Shop (Workspace), `shop-web alice@127.0.0.1` (Site, user@host muted mono), `shop-web · files`, `shop-web · terminal` (Tab); equal scores alphabetical | Pass |
| Selected item | ↓ ↓: the third row on the accent wash (`row_selected`); the first row is highlighted by default | Pass |
| Files tab | Remote pane, a folder selected: New folder `F7`, Rename `F2`, Delete `Del`, Download selection `Ctrl ↵` (no Edit: a folder), then the session actions | Pass |
| No match | `zzz`: "No matches" in muted text under the field | Pass |
| Top bar button | "Go to anything `Ctrl K`" is now an outlined button (`button_outline`) | NEEDS HUMAN: click it, hover |
| Real keyboard | Ctrl+K from the file panes and the sidebar filter; Ctrl+Shift+K in a focused terminal (plain Ctrl+K reaches the shell: `echo abc` then Ctrl+A, Ctrl+K kills the line); Esc; click outside closes | NEEDS HUMAN (covered by unit tests through messages) |

## Task 11: final pass (2026-11-13)

Setup: release build under XWayland (2560×1600 @2x, captures scaled to 1280×800), isolated `HOME`/`XDG_*`
(`known_hosts` seeded with `ssh-keyscan`, nothing in the real keyring), Docker sshd (`127.0.0.1:2222`, alice, key)
saved as `shop-web` (Open as: both) in a workspace `Shop` whose saved tabs are Terminal + Files, plus four
unconnected sites. `vim.tiny`, `htop`, `less` and `tmux` were installed into the running test container for the
pass (the container was recreated afterwards). A throwaway driver (not committed) sent the app's own messages
(terminal keys as `TerminalMsg::Key`) and took iced `window::screenshot` captures, waiting 2.5 s before each.

Spec §11 scenarios, one row each (earlier rows above are referenced, not repeated):

| Check | Result |
| --- | --- |
| Workspace with a saved terminal tab | "Open workspace" on `Shop`: a terminal tab (`shop-web ssh`, prompt ready) then the files tab (`shop-web sftp` at `/home/alice/edit-demo`); one `connecting` → `connected` pair in the log, so both tabs ride one connection | OK |
| Files and terminal on one session | Same run: status bar "Connected alice@127.0.0.1:2222 ed25519 key"; the terminal ran vim/htop/tmux while the files tab listed; switching tabs kept both. Closing one tab keeps the other: covered by `closing_one_tab_keeps_the_session_for_the_other` | OK |
| `vim` | `vim.tiny /tmp/qa.txt`, `i`, typed text, Esc, `:wq`: full-screen editor, `~` column, `-- INSERT --` and `1,15 All` in the last row; `:wq` returned to the prompt | OK |
| `htop` | Coloured CPU/memory meters, the green header bar across the width, the cyan selected row, the F-key bar at the bottom; `q` back to the prompt | OK |
| `less` | `seq 1 300 \| less`: lines 1–35 and the `:` prompt on the last row; `q` back. Wheel in `less` | OK. Wheel NEEDS HUMAN (Task 4 row "Wheel") |
| `tmux` | `LANG=C.UTF-8 tmux -u`, Ctrl+B `%`: two panes split by the vertical rule, green status line, the new pane focused; `exit` twice returned to the shell | OK |
| 256 colours and truecolor | Inside tmux: 24-bit orange and 256-colour green (also Task 4's gradient bar) | OK |
| CJK and emoji | `中文` two cells each; `🙂` drawn from the system fallback font, two cells. (Without a UTF-8 locale on the server tmux prints `_` for them: server side, not the app) | OK |
| Resize | Task 4 (grid and `stty size` follow, 100 ms debounce) | OK. Drag-resize NEEDS HUMAN (Task 4 row) |
| Search | Task 4 | OK |
| Copy and paste | Task 4 (via messages) | NEEDS HUMAN with a real mouse and keyboard (Task 4 row "Copy and paste") |
| 50 MB of output | Task 4 (`head -c … \| base64` and 50 MB `cat`, UI kept ticking, shell stayed open) | OK |
| Network drop and reconnect | Covered by `connection_lost_banner_then_new_shell` and the Task 5 Docker idle tests (`--ignored`) | NEEDS HUMAN visually: with a terminal and a files tab of `shop-web` open, `docker compose -f docker/compose.yml restart sshd`; the terminal shows "Connection lost" with Reconnect, the files tab reconnects on its next action (open a folder) without pressing Reconnect; click Reconnect: "— reconnected —" and a working prompt |
| Remote edit with a conflict | Task 8 rows "Three saves in a row", "Conflict choice" | OK. Real GUI editor NEEDS HUMAN (Task 8 row) |
| Close with a pending edit | Task 8 rows "Close the last tab", "Close the window" | OK. "Upload and quit" with the network cut NEEDS HUMAN (Task 8 row) |
| Palette | Problem: with an empty query a busy files tab filled the 12 rows with its own actions and tabs, so Settings, "Open workspace…" and the sites needed typing; and every message re-ranked the rows. Fixed in `efc7fc5`: at most 5 own actions, 3 other tabs (not the active one), then Open workspace…, Settings, 4 sites, the rest; rows ranked only for the view, ↑/↓ and a run. Recaptured: files tab (sub/ selected) New folder F7, Rename F2, Delete Del, Download selection Ctrl ↵, New terminal tab, `shop-web · terminal`, Open workspace…, Settings, `shop-web`, `nas`, `blog`, `mail`; terminal tab Paste, Find in scrollback, New terminal tab, New files tab, Close tab, `shop-web · files`, then the same go-to rows | Fixed (`efc7fc5`). Real keyboard NEEDS HUMAN (Task 9 row) |
| Log | Task 6 rows; recaptured under the queue: three INFO lines (winit scale factor, `connecting`, `connected` with `site`, `protocol`, `endpoint`, `since_last_request_secs`), no iced/wgpu noise | OK |
| Remote toolbar at 1280 px | Problem: since `Edit F4` the remote toolbar dropped its key hints at the default 1280 px window (Task 8 row). The mockup has no pane toolbar; its key caps (`Ctrl K`) show keys are meant to be visible, and shortened labels would be jargon, so the labels stay and the flat text buttons lose 4 px of side padding each. Recaptured: at 1280 and 1240 px both toolbars show every label and hint ("Download Ctrl ↵  Edit F4  New folder F7  Rename F2  Delete Del  Permissions…"), compact below ~1240 px | Fixed (`56e9426`) |
| Toolbars at the 960 px minimum | Problem: the compact remote toolbar (no hints) clipped "Permissions…" in the 352 px panes of the 960×640 window, a regression from Plan 5 caused by Edit. Below a 400 px pane both toolbars now use short labels: remote "Download Edit Folder Rename Delete Perms…", local "Upload Folder Rename Delete". Recaptured at 960×640: all six remote actions unclipped with room to spare; at 1100 px the compact long labels fit; at 1280×800 unchanged (labels and hints) | Fixed (`01dd05e`) |
| Toast and notices | Kept notice (`paper_2` bar, mono path, outlined Dismiss) and toast "Uploaded `notes.txt`" (`dialog_box`: 1 px `rule_2`, 4 px corners, no shadow) recaptured together with the log panel; the only white fill in the edit flows is the main action of the close question | OK |
| Idle session, 10 minutes, the author's real server | Not reachable from here | NEEDS HUMAN: run the release build with the default filter (no `RUST_LOG`), open an FTP site from the real server, leave it idle 10+ minutes, then open a folder. It must work without a click. Read the INFO lines in the Log panel (status bar → Log) or the terminal: each `connection state changed` line names the state, the `reason` and `since_last_request_secs`, so any drop says why |
| Performance job | `perf.yml` not dispatched yet (the controller dispatches it after the push). Local `scripts/perf.sh` at `10e0947` (`docs/perf/README.md`): open 93 ms (< 500), idle 34.4 MB (< 80), 100k rows p95 0.68 ms (< 16), SFTP 1 GiB at 50 ms RTT 1.00 × `scp` (≥ 0.9), 10k × 4 KiB at 10 ms RTT SFTP 1.98 × `lftp` and FTP 1.001 × `lftp` (≤ 1.0, known gaps), progress ≤ 31 events in 3 s | OK locally, two known gaps recorded in the follow-ups. CI run pending |
