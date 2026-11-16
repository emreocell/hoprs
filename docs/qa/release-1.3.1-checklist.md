# Release 1.3.1 checklist

One section per task; later tasks append theirs.

## Task 1: workspace groups (2026-11-16)

| Check | Result |
| --- | --- |
| An expanded workspace shows its sites and nothing else: no Open workspace button, no Close link | Pass: covered by the sidebar view change (no control left to render) |
| Right-click a workspace: Open all, Edit; Close all too while it is the open one; choosing them opens or closes the workspace | Pass: `site_and_workspace_menus` |
| The palette lists Open workspace and, while one is open, "Close all in (workspace name)" | Pass: `palette_lists_context_actions` |
| The open workspace is marked (green stack icon, "Workspace open" tooltip); the others are not | Pass: `the_open_workspace_is_marked` |
| The row, the menus, the palette and the mark look right in English and pt-BR, on a real display | NEEDS HUMAN: steps below |

### NEEDS HUMAN

1. **Groups.** Expand a workspace in the sidebar: only its sites show, no button under them.
   Click a site inside it: that site opens on its own, like any site.
2. **Menu.** Right-click the workspace: "Open all" and "Edit". Choose Open all: every site
   connects and the saved tabs come back; the workspace gets a green stack icon whose
   tooltip reads "Workspace open". Right-click it again: "Close all" is there too; choose it:
   the tabs are saved and closed, and the mark goes.
3. **Palette.** With the workspace open, press Ctrl+K and type "close": "Close all in
   (workspace name)" is listed and does the same as the menu item.
4. **pt-BR.** Switch Settings › Language to Português and repeat steps 2 and 3: "Abrir
   todos", "Fechar todos", "Workspace aberto", "Fechar todos em (nome do workspace)".

## Task 2: refresh after transfers (2026-11-16)

| Check | Result |
| --- | --- |
| A Done item re-lists the pane showing its destination folder (upload: remote, download: local), quietly | Pass: `done_item_relists_the_pane_showing_its_folder` |
| Another folder, the other side, another site, or a folder the pane left: no re-list | Pass: `other_folder_other_side_other_site_not_relisted` |
| Failed, cancelled and skipped items do not re-list | Pass: `failed_cancelled_skipped_do_not_relist` |
| A burst gives one listing per pane per 500 ms, never two at once | Pass: `burst_of_done_items_gives_one_listing`, `listing_in_flight_defers_the_relist` |
| Selection and scroll survive; an open rename editor holds the re-list back, typed text kept | Pass: `relist_keeps_selection_and_rename_editor` |
| The same remote folder in two tabs of a site re-lists in both, not in another site's tab | Pass: `same_remote_folder_in_two_tabs_both_relist` |
| Endings seen only in a snapshot (after a lag) re-list too | Pass: `done_seen_only_in_a_snapshot_relists` |
| Upload, download and a drag between panes show the file without Refresh, on SFTP and FTP | NEEDS HUMAN: steps below |

### NEEDS HUMAN

1. **SFTP.** Open an SSH site in a files tab. Select a local file and press Upload: the file
   appears in the remote pane within about a second, with no Refresh. Select a remote file
   and press Download: it appears in the local pane. Drag a file from one pane to the other:
   it appears in the target pane.
2. **FTP.** Repeat step 1 on an FTP (or FTPS) site.
3. **A burst.** Upload a folder with a few hundred small files into the remote folder shown:
   the remote pane updates as they land, without flicker; the log panel shows no listing
   storm (about two listings a second at most).
4. **Selection and editor.** Select a remote file, then upload another file into the same
   folder: the selection stays on the same file. Press F2 on a file and type a name, then
   let an upload finish into that folder: the typed name stays; after Escape, the new file
   appears.
