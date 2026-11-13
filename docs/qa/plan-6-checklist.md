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
