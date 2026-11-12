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

| Region | Check | Result (Task 5, 2026-11-12, GNOME Wayland, 1280×800 @1x, captured with iced `window::screenshot`) |
| --- | --- | --- |
| Sidebar, loaded | Workspaces group: name + muted "N sites" count; expanded workspace on `paper_3` with its sites nested behind a 1 px `rule_2` line, "Site missing" inline (error colour) for a deleted site, white full-width "Open workspace" (32 px) plus a muted "Edit"; collapsed workspaces as plain rows. All sites: name left, protocol tag right in JetBrains Mono (`ssh + sftp`, `sftp`, `ftps`, `ftp`, `ssh`). "+ New" on both headings works | Pass. "Edit" next to "Open workspace" is an addition to the mockup (it has no edit affordance) |
| Sidebar, filter | "Filter sites" narrows All sites by name, host or user (any case) and Workspaces to those whose name or sites match; "No matching sites" when nothing matches | Pass |
| Sidebar, empty | "No workspaces yet"; "No sites yet" with the "New site" action | Pass |
| Sidebar, unreadable | Each unreadable entry by name (mono) or "Entry N in file", the warning "Can't read this entry (written by a newer hoprs?)" and "Open sites.toml folder" (opens the folder with `xdg-open`; a failure shows inline) | Pass (the opener itself not clicked in the capture) |
| Site form | Center area, no modal: Name, Protocol (`Protocol::label()` in mono), Host + Port (port follows the protocol until edited), User, Password (masked, "Saved in the system keyring" + "Forget the saved password" when one is stored), SSH options (ssh-agent, key file, open as + "Terminal tabs arrive in a later version") or FTP options (passive/active, NAT), start folders; inline errors in the error colour; Save (white) / Cancel / Delete; delete asks in place | Pass |
| Workspace form | Name, sites as checkboxes in `sites.toml` order with their mono tags, Save / Cancel / Delete with the in-place confirmation | Pass |
| Tabs (placeholder) | Open workspace adds one tab per site (name + mono tag, white underline on the active one); the center shows the site, `user@host:port` in mono and its state: Connecting, Connected, or the error with detail, the keyring note, Retry and Edit site | Pass. Task 6 replaces the center with the file panes |
