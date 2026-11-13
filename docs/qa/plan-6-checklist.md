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
