# Plan 5 visual checklist

Manual check against mockup v2 (`docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`), spec §11.
Run `cargo run -p hoprs-app` and compare each region. One line per region; later tasks append theirs.

| Region | Check | Result (Task 3, 2026-11-12, GNOME Wayland, 1280×800 @1x and 1920×986 @1.25x) |
| --- | --- | --- |
| Top bar | 40 px; brand "hoprs" in Inter semibold + muted version, 256 px wide with a 1 px right rule; "No open sessions" placeholder where tabs go; outlined "Go to anything" + `Ctrl K` key cap slot on the right; 1 px rule below | Pass. Tabs themselves arrive in Task 6, the palette in Plan 6 |
| Sidebar | 256 px, `paper_2`; "Filter sites" field (32 px, `rule_2` border, white on focus); Workspaces and All sites groups with 1 px rules, muted headings and "+ New"; empty states; footer "Import from FileZilla" / "Settings" pinned to the bottom above a rule | Pass. "+ New", Import and Settings render disabled (dim) until Tasks 5 and 11 wire them |
| Center | Ground `paper`; empty state "No session open" + hint, centred | Pass. File panes arrive in Task 6 |
| Queue | `rule_2` line on top; 36 px head: "Transfers", Active/Queued/Failed/Done with counts, white 2 px underline on the active tab, "Idle" total and Hide/Show; body "No transfers"; never taller than 210 px; collapsed shows only the head | Pass (expanded and collapsed both captured) |
| Status bar | 26 px, `paper_2`, 1 px rule on top, muted 12 px text, only under the right column (sidebar runs full height) | Pass. Connection details arrive in Task 10 |
