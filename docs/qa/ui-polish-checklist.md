# Plan 13 checklist: UI polish

One run per release that touches the icons, context menus, drag and drop, or resizable
panels. AUTOMATED rows run in `cargo test --workspace`; NEEDS HUMAN rows need a person and a
real display.

## AUTOMATED

| Check | Result |
| --- | --- |
| Every icon is in the font, the font family is `Phosphor`, and `phosphor-glyphs.txt` matches the `Icon` enum (`crates/hoprs-app/src/ui/icons.rs` tests) | Pass |
| File icons by extension (`for_entry`) | Pass |
| Context menu model: opens at the last press, closes on Escape, outside click, resize and choice, blocked behind a dialog (`crates/hoprs-app/src/app/tests/menu.rs`); kept inside the window (`ui::menu` tests); press position published before the widget under it (`ui::pointer` tests) | Pass |
| Sidebar state saved: collapse and hide round-trip through `settings.toml`, invalid values read as shown, the ⋯ menu hides and shows a section (`settings` tests, `app/tests/import_settings.rs`); site state and icon by protocol (`ui::sidebar` tests) | Pass |
| Panel dividers: a drag resizes the compare panel and the queue from the press position, never below their minimum, saved on release or when the pointer leaves the window; a double-click resets and saves; heights round-trip through `settings.toml`; past the window's cap or the minimum a drag turns back at once; dragging up a short queue never lowers its maximum, dragging down shrinks it from the drawn height; a click or unchanged drag saves nothing; the queue's saved height is its maximum (two rows stay two rows); pointer moves are listened to only while resizing (`app/tests/panels.rs`, `ui::pointer` tests) | Pass |
| Drag feedback: a drag starts past 4 px, has a target only over the other pane of a connected site and says what a drop does (`app/tests/transfers.rs`); the chip shows the first dragged item and "+N" only once the drag started, and stays inside the window; the drop label splits out the folder (`ui::drag` tests) | Pass |
| Settings tab: one at most, selecting another tab keeps it, its × returns to the tab it came from (or the last), not saved with a workspace, blocked behind a dialog (`app/tests/tabs.rs`); settings saved and typed passwords forgotten when it is not shown (`app/tests/import_settings.rs`); no connection in the status bar (`app/tests/status.rs`); Ctrl+, opens it even from a focused terminal and sends nothing to the shell (`ui::files`, `ui::terminal`, `terminal::input` tests); palette rows (`app/tests/palette.rs`) | Pass |

**Screenshots (2026-11-16, release 1.3.0):** README images (`files`, `menu`, `drag`,
`settings`, `compare`, `terminal`, `palette`, `pt-br`) and the site's `main`, `split` and
`og.png` re-shot from a locally built `hoprs_1.3.0-1_amd64.deb` with
`LOCAL_DEB=… scripts/readme-shots.sh` and `LOCAL_DEB=… site/scripts/capture/run.sh`; each
image checked by eye (menu and drag visible, no leftover hover tint, Portuguese shot in
pt-BR). This is not a substitute for the rows below.

## NEEDS HUMAN

1. **Icon rendering.** Run hoprs at 100%, 150% and 200% scale on Linux, macOS and Windows.
   Expect: icons are crisp and sit on the same baseline as the label next to them.

2. **Disabled actions.** With nothing selected, look at the pane toolbars and the dialogs'
   outline buttons. Expect: enabled and disabled actions are told apart at a glance, and
   disabled outline buttons have a dashed outline.

3. **Logo.** Look at the top of the sidebar at 100% and 200% scale. Expect: the logo is
   sharp, aligned with "hoprs", and there is no version beside it.

4. **Context menu placement and closing.** Click the ⋯ button at the top of the sidebar.
   Expect: the menu opens at the pointer. Make the window small enough that the button sits
   near its right edge (or use a later menu near the right and bottom edges) and open it
   again. Expect: the menu flips to stay fully inside the window. Then check each way out:
   Escape, a click outside, a scroll and a window resize each close it, and nothing behind
   it reacts to that click.

5. **Sidebar rows.** Hover several site and workspace rows. Expect: the pencil appears only
   on the hovered row (over the protocol tag or count) and opens its form; connected sites
   show a green plug and a white name; a site that failed shows a red warning, and hovering
   the icon shows the error. Collapse and hide both sections from the caret and the ⋯ menu,
   restart, and expect the same state.

6. **Panes and compare panel.** NEEDS HUMAN. Open a site. Expect: folders and files told
   apart at a glance (kind icons; `..` has its own); the selected row shows a 2 px bar in
   the focused pane (white) and in the other pane (grey); long names end in "…" with the
   full name on hover (and no tooltip on names that fit); a deep path in the path bar
   keeps its first and last folders with "…" between (hover shows the whole path). The
   toolbar buttons have icons; below about a 1480 px window the key hints go, and under about
   1320 px Permissions and Edit lose their labels first (the default 1280 px window shows
   Download, New folder, Rename and Delete with labels, in English and Portuguese, nothing
   clipped), and under about 1130 px every button is icons only with their label and key on hover. In the
   compare panel: group headers have icons, rows have kind icons and a selection bar, the
   collapse and close buttons are icons.

7. **Tabs, queue and status bar.** NEEDS HUMAN. Open an SSH site with a terminal and a
   files tab and start a few transfers. Expect: each tab has its kind icon, the name is cut
   with "…" past about 200 px (full name on hover), and the tab × and the queue row actions
   (pause, resume, cancel, retry, dismiss) are easy to hit (22 px). Every icon-only button
   (tab ×, new terminal and files tabs, clear finished, hide and show, log) shows a tooltip
   with its label. The status bar shows a green plug when connected, a red warning when the
   connection is lost, and a lock before the login method on SSH and FTPS but not on plain
   FTP. The Log button stays highlighted while the panel is open.

8. **Settings tab.** NEEDS HUMAN. Ctrl+, (Cmd+, on macOS) opens Settings from a file tab,
   from a focused terminal (nothing typed in the shell) and with no tabs. Expect: a
   "Settings" tab with a gear after the session tabs, selected; no queue under it and no
   connection in the status bar (the Log button stays). Press Ctrl+, again, or use the
   sidebar gear or Ctrl+K › Settings: the same tab is focused, never a second one. Click a
   session tab and back; then close Settings with its ×: the tab you came from is selected
   again.

9. **Settings sections.** NEEDS HUMAN. In the Settings tab click each of the six sections
   in the left list (General, Transfers, Folder compare, Remote editing, Terminal, Backup),
   in English and in Portuguese. Expect: the selected row has the white bar, each section
   shows its title, a one-line description and its fields, and each one fits at 960 x 600.

10. **Resizable panels.** NEEDS HUMAN. Open a files tab with the compare panel and a few
    transfers in the queue. Expect: the cursor turns into the vertical resize cursor over
    the top edge of the compare panel and of the queue, and a small grip shows there on
    hover; dragging either edge resizes it smoothly; a double-click on the edge resets it
    (compare about 30% of the tab, queue 210 px at most); the heights are the same after a
    restart. Make the window shorter: the panels shrink before the file panes lose their
    last five rows, and grow back when the window does. A terminal tab shows only the
    queue's edge; the settings tab has no queue. While dragging, the panel follows the
    pointer at once in both directions, also after going past its smallest or largest
    size; dragging up a queue with only a few rows does not shrink its maximum. Expected,
    not a bug: once a panel stops at a bound, the cursor may turn back into an arrow until
    the pointer is over the edge again (iced applies the resize cursor only over the 7 px
    band).
    Collapse the compare panel with its caret: one line is left with the differences icon, the
    three counts (only here, only on the server, different) or the progress while a run has
    no rows yet, the auto checkbox, expand and close; no resize edge; expanding brings back
    the height it had.

11. **File pane menus.** NEEDS HUMAN. Right-click a file and a folder in each pane on
    Linux, macOS (also a two-finger click and Ctrl+click) and Windows. Expect: the menu opens
    at the pointer (kept inside the window near the edges) with icons, labels and keys:
    local rows Upload, Open (folders), Rename, Copy path, Refresh, New folder, a rule, then
    Delete in red; remote rows Download, Edit (files), Open (folders), Rename,
    Permissions…, Copy path, Refresh, New folder, rule, Delete. Unavailable items are dimmed,
    not hidden. A right-click on an unselected row selects it alone; on one of several
    selected rows the selection stays and Delete asks about all of them. A right-click
    below the rows gives New folder, Refresh, Compare. Copy path pastes the full path (one
    per line for several). Hovering an item lights it up with a white icon. F5 refreshes the
    focused pane. Select a row, then press Shift+F10 and the Menu key, in a short folder
    (no scrollbar), a long scrolled one, after Tab to the other pane and after resizing the
    window: the menu opens at the row each time; ↑/↓ move (skipping dimmed items, wrapping), Enter runs the item, Escape closes;
    another key (F7) closes the menu and acts. Opening Settings (Ctrl+,) or switching tabs
    closes an open menu.

12. **Copy path on Windows.** NEEDS HUMAN. On Windows, right-click a file in the local pane
    (for example under `Users\<you>\Documents`), choose Copy path and paste it into Explorer's
    address bar or Notepad. Expect: a native path with the drive and backslashes
    (`C:\Users\<you>\Documents\file.txt`) that Explorer opens; with several files selected,
    one such path per line. Also with a rename editor open in a pane, right-click a row:
    ↑/↓ move in the menu (not the editor's caret) and Enter runs the item (it does not
    commit the rename).

13. **Menus on compare rows, queue rows, the sidebar and tabs.** NEEDS HUMAN. Expect every
    menu in the spec's list where expected; keyboard reaches each action through shortcuts
    or the palette. Steps:
    - Compare: run a compare with rows in several groups. Right-click an unselected row: it
      is selected alone and the menu shows Upload, Download (each dimmed unless the panel's
      button of the same name is on) and Show in panes. Right-click one of several ticked
      rows: the ticks stay and Upload/Download act on all of them. Show in panes goes to the
      item, like a double-click. Click a row, move with ↑/↓, press Shift+F10 (and the Menu
      key) with the list short (no scrollbar) and scrolled: the menu opens at the row.
    - Queue: right-click an active row (Pause, Cancel), a paused one (Resume, Cancel), a
      failed one (Retry, Dismiss); a conflict row and a finished row open no menu (the
      conflict's answers stay in the row). An item on a row that finished meanwhile does
      nothing.
    - Sidebar: right-click a site (Open files, Edit; also a site inside an open workspace) and
      a workspace head (Open all, Edit; Close all too while it is the open one, marked by a green stack icon with an "Open" tooltip). No Disconnect, no Delete. Shift+F10 does
      nothing there (mouse only).
    - Tabs: right-click a session tab and the settings tab: Close tab, Close other tabs
      (dimmed when it is the only tab). Close other tabs keeps the clicked tab, selected,
      closes the rest (the settings tab too, unless it is the one kept); a site whose last tab
      has unsent remote edits asks first, in its tab.
    - Keyboard: Close tab, Open workspace, Close all in <workspace> (palette, while one is open), the sites, Upload/Download (compare panel
      buttons), the queue's actions (Tab to their buttons) stay reachable as before.

14. **Drag feedback.** NEEDS HUMAN. On Linux, macOS and Windows, open a connected site,
    select two files in the local pane and drag one of them. Expect: nothing moves until
    the pointer has gone a few pixels; then a chip follows the pointer with the first
    file's icon and name ("…" when long), "+1" and a badge "2", and stays inside the window
    near the edges; the two dragged rows are dimmed. Over the remote pane: a dashed grey
    outline, a faint wash and, near the bottom, "Upload 2 items to /the/folder" (the folder
    in mono, cut with "…" in a narrow window); the list does not move or shrink. Over the
    local pane itself, and over a pane whose site is disconnected: no outline, the
    not-allowed cursor, and releasing there does nothing. Drag remote files to the local
    pane: "Download N items to ~/…". Drag files from the OS file manager over the remote
    pane: "Upload N files to …" ("Upload files to …" until the system says how many).
    Press Escape during a drag, and drag out of the window: both cancel (no chip, no dimmed
    rows, nothing queued).
