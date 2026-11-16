# UI polish: icons, settings tab, context menus, drag feedback, resizable panels — design

Status: approved by the author (2026-11-16) in conversation. The author listed six points
after using 1.2.0 on a real server; a visual audit added nine more (A–I below) and the author
said "todos devem entrar". The mockup `ui-v2.html` (visual companion, kept under
`.superpowers/brainstorm/`) was approved with one change: Phosphor **Regular** weight, not
Light ("prefiro o Regular, de resto está tudo ótimo").

## Intent

hoprs works, but it is all text: actions hide behind keyboard shortcuts and loose text
links, folders and files look alike, settings replace the whole center, the compare panel
takes space the author cannot give back, and a drag between panes shows nothing. The goal is
an app that is **obvious to use on first sight**, keeping the mockup v2 look (neutral
near-black, white accent, Inter + JetBrains Mono, 1 px rules, 4 px radius, no shadows).

## Goals

The author's six points:

1. **Icons** everywhere an action or an item kind is shown, from **Phosphor, Regular
   weight** (MIT).
2. **Settings open as a tab**, next to the connection tabs, instead of replacing the
   center.
3. **Right-click** on items opens a context menu with what can be done to them.
4. **Dragging** a file or folder between panes is **visible**: what is dragged, where it
   will land, what will happen.
5. **Sidebar sections collapse and expand** (Workspaces, All sites), and either can be
   **hidden**.
6. **The hoprs logo** at the top of the sidebar, **without the version** next to the name.

From the audit (all in):

- **A.** Disabled actions readable: today `rule_2` text on `paper` is about 1.6:1; it
  becomes about 3:1, and a disabled outline button gets a dashed border.
- **B.** Long names end in "…" and show the full name in a tooltip (file rows, compare rows,
  queue rows, sidebar names, tab titles, breadcrumbs that do not fit).
- **C.** Folders and files told apart by icon (folder vs file kinds), not only by the
  trailing "/".
- **D.** The per-row "Edit" text in the sidebar goes away: a pencil icon button appears on
  the hovered row, and Edit is in the row's context menu.
- **E.** The sidebar shows which sites are connected (a "plugs-connected" icon in the
  success colour and the name in `ink`), and which are connecting or failed.
- **F.** The compare panel is resizable (below), and its collapsed state is one summary
  line.
- **G.** The selected row is clearly visible: a lighter background plus a 2 px `ink` bar on
  the left edge; in the pane without focus the bar is `rule_2`.
- **H.** Click targets of at least 22×22 px for icon buttons (the tab ×, the queue row
  actions).
- **I.** The top bar's "New terminal tab" / "New files tab" become icon buttons with
  tooltips; the queue's Pause / Cancel / Clear / Hide and the status bar's Log likewise.

Resizable panels (the author's idea, refined in conversation):

- A **draggable divider** above the compare panel and another above the transfer queue.
  The pointer becomes a vertical-resize cursor over it; dragging resizes; **double-click
  resets** that panel to its default height.
- **Minimum heights:** the file panes never go below 5 rows; the compare panel never below
  its header plus 2 rows; the queue never below its header plus 1 row. When the window
  shrinks, the panels shrink to their minimums before the panes go below theirs.
- **Collapse** (compare) reduces the panel to one summary line, e.g.
  `Differences · 7 only here · 1 only on the server · 0 different`, with the automatic
  toggle, expand and close; expanding restores the height the user had. The queue's Hide
  keeps its current meaning.
- **Heights are saved** in the settings file and shared by all file tabs. The compare
  panel's default is about 30% of the area below the tab bar (smaller than today); the
  queue's default is today's height.

## Non-goals

- New file operations. Menus hold actions that already exist elsewhere in the app, plus
  **Copy path** (file rows) and **Close other tabs** (tab menu), which are small and
  obviously wanted.
- A light theme, theming, or icon customisation.
- Making the site and workspace forms or the FileZilla import into tabs (they stay center
  forms; only Settings moves).
- Drag and drop between tabs, to the sidebar, or out of the app to the OS file manager.
- Resizable sidebar width or pane columns.

## Anti-goals

- Anything that slows the file list: a 100k-row folder must still scroll at today's p95
  (`docs/perf/README.md`, 0.58 ms). Icons are glyphs of a font already loaded, not images
  per row.
- A context menu or drag that does something without the user's click (a drop is the
  click). A drop on an invalid target does nothing.
- Shadows, gradients, coloured icons other than state colours (success, error), or a second
  accent. Icons are `muted` by default and `ink` on hover/active, like text buttons today.
- Keyboard users losing anything: every menu action stays reachable without a mouse
  (through the shortcuts and the Ctrl+K palette, as today).
- A new heavy dependency (iced's `svg` or full `image` features, a widget crate).

## Approaches considered

**Icons.**
1. **Icon font (chosen):** the Phosphor Regular TTF, subset to the glyphs used, embedded
   like Inter and JetBrains Mono, drawn as text. Cheap per row, scales with text, no new
   crate.
2. iced's `svg` feature with Phosphor SVGs: crisper control, but pulls `resvg` and its
   tree into the build for 16 px glyphs, and an SVG per row is costlier than a glyph.
3. PNG sprites: blurry at fractional scale factors, one file per size.

**Icon set.** Lucide was proposed first; Phosphor chosen by the author; weights Thin, Light
and Regular compared in the mockup; Regular chosen.

**Settings as a tab.** A tab kind with no site (chosen) vs. a modal or a side panel (the
mockup v2 rule is "no modal cascades", and the author asked for a tab).

**Resizing.** The author proposed dragging the panel's edge like a window; combined with the
existing Collapse, and the default auto-collapse proposed earlier was dropped, so nothing
the author asked for in 1.2 changes behaviour.

**Context menus, divider, drag ghost:** built from iced 0.14's own pieces (`mouse_area`
with right press / move / double click, `stack`, an absolutely positioned layer, mouse
interaction cursors), no third-party widget crate.

## Design

### Icons

- **Font:** Phosphor Regular (`@phosphor-icons/web` 2.1.x `Phosphor.ttf`, MIT), subset to
  the glyphs the app uses, committed under `crates/hoprs-app/assets/fonts/` with its licence
  file, loaded at startup with the other fonts. A script (`scripts/icons-subset.sh` or
  similar) regenerates the subset from a list of glyph names so adding an icon is one line;
  the generated font is committed (the build does not run Python).
- **A typed icon module** (`ui/icons.rs` or similar): one Rust item per glyph used
  (`Icon::Folder`, `Icon::DownloadSimple`, …) mapping to its codepoint; a helper renders it
  at a size and colour. No string lookups at runtime.
- **Sizes:** 16 px in rows, toolbars and menus; 15 px in the tab bar and sidebar rows; the
  same vertical alignment as the label next to it.
- **Glyphs (from the approved mockup):** folder, folder-plus, folder-simple-plus, file,
  file-text, file-code, file-html, file-image, file-pdf, file-zip, file-archive (by
  extension; unknown → file), arrow-bend-left-up (parent), upload-simple, download-simple,
  pencil-simple, pencil-simple-line, textbox (rename), trash, lock-key (permissions),
  lock-simple (encrypted connection), git-diff (compare), copy, arrows-clockwise (refresh),
  desktop (local), hard-drives (ftp/ftps site), terminal-window (ssh site, terminal tab),
  stack (workspace), plugs-connected, plug (disconnected) or warning (failed),
  magnifying-glass, plus, x, caret-down, caret-right, caret-up, dots-three, gear-six,
  sliders-horizontal, arrows-down-up, shield-check, scroll (log), broom (clear finished),
  pause, play, arrow-up, arrow-down (queue direction), arrows-left-right (different),
  files (multi-drag), warning-circle (errors). The builder may add a glyph where a place
  needs one; the list is the floor.
- **Where:** every toolbar action (icon + label, as in the mockup), every icon-only button
  (with a tooltip carrying the label and shortcut), file rows (kind icon), compare rows and
  group headers, queue rows (direction), sidebar rows (site kind or connection state),
  tabs (kind), breadcrumb side labels (desktop / hard-drives), status bar (connection,
  encryption), settings navigation.
- **File kind by extension** is a small table (images, pdf, archives, text, code, html);
  `.wpress` and other unknowns map to a sensible default (archive for known backup
  extensions is fine; unknown → file). Symlinks keep today's marker plus the file icon.

### Sidebar (points 5, 6, D, E)

- **Brand row:** the hoprs logo (from `assets/icon/hoprs.svg`; drawn from the embedded
  PNGs, see Constraints) at 22 px, "hoprs" in Inter SemiBold, **no version** (the version
  stays in Settings → General, and in `--version`). At the right, a `dots-three` icon
  button opening a small menu: **Show workspaces**, **Show all sites** (check items).
- **Sections:** a caret before the title collapses / expands the section; a collapsed
  section shows its count at the right instead of the + button. Hiding a section removes
  it entirely; hiding both shows a one-line hint to bring them back through the ⋯ menu.
- **Rows:** icon (site kind; workspace = stack), name with ellipsis, protocol label in mono.
  On hover the protocol label gives way to a pencil icon button (Edit). Right-click opens
  the row menu (below).
- **Connection state:** connected → `plugs-connected` in the success colour, name in `ink`;
  connecting → the kind icon with the busy treatment used elsewhere; failed → `warning`
  in the error colour with the error in the tooltip.
- **Footer:** "Import from FileZilla" with its icon (text kept, it is rare and needs words),
  and a `gear-six` icon button that opens the Settings tab.
- **Saved:** collapsed and hidden state of both sections, in the settings file.

### Settings as a tab (point 2)

- A **settings tab** lives in the tab bar like the others, with the `gear-six` icon and the
  title "Settings". It has no site, no connection and no status bar connection info.
- **Opening:** the sidebar gear, the palette action, and **Ctrl+,** (Cmd+, on macOS). If
  the tab is open, these focus it; there is at most one.
- **Closing:** its ×, or Ctrl+W when focused, like any tab.
- **Layout:** a left navigation (General, Transfers, Compare, Editing, Terminal, Backup)
  and the section's fields on the right, as in the mockup. Each section has a one-line
  description under its title. The fields and their behaviour are today's (saved at once);
  only the arrangement changes. General holds the language and the app version.
- **Not restored:** opening a workspace or restarting does not reopen it.
- The site form, workspace form, import and the "unsent edits" question stay center forms
  as today; while one of them is shown, the tab bar still shows the tabs (unchanged
  behaviour).

### Context menus (point 3)

A menu opens on right-click (and with **Shift+F10** or the Menu key on the focused item),
at the pointer (at the item for the keyboard), kept inside the window. It closes on Escape,
a click outside, a scroll, a window resize, or choosing an item. Arrow keys move, Enter
chooses. Items show their icon, label and shortcut (if any); unavailable items are shown
disabled, not hidden; destructive items (Delete) are in the error colour and last, after a
rule.

**Selection rule:** right-click on a row that is not selected selects it alone first;
right-click on a selected row keeps the selection, and the menu acts on all selected.

Menus (only actions that exist today, plus Copy path / Close other tabs):

- **Local file/folder row:** Upload (Ctrl+↵), Open (folders), Rename (F2), Copy path,
  Refresh (F5), New folder (F7) — rule — Delete (Del).
- **Remote file/folder row:** Download (Ctrl+↵), Edit (F4, files), Open (folders),
  Rename (F2), Permissions…, Copy path, Refresh (F5), New folder (F7) — rule — Delete (Del).
- **Empty area of a pane:** New folder, Refresh, Compare.
- **Compare row:** Upload / Download (as the panel's buttons allow for that item), Show in
  panes (today's double-click).
- **Queue row:** today's row actions (Pause/Resume, Cancel, Retry, …) for that row.
- **Sidebar site:** Open files (today's `ConnectSite`), Edit. (No Disconnect or Delete:
  neither exists as a sidebar action today; Delete stays in the site form.)
- **Sidebar workspace:** Open workspace, Edit.
- **Tab:** Close, Close other tabs.

Copy path copies the full path of the item (local: the OS path; remote: the remote path)
to the clipboard; with several selected, one per line.

### Drag feedback (point 4)

- A press on a row followed by a move of **more than 4 px** starts a drag (a press without
  that move stays a click; today's logic for where a drag may drop is kept).
- **Ghost:** a small chip that follows the pointer: kind icon, the first item's name (with
  ellipsis), and a count badge when several items are dragged ("+1", badge "2").
- **Source rows** are drawn at reduced opacity while dragging.
- **Target pane:** when the pointer is over a valid target, it gets a 1 px dashed `ink_2`
  outline and a faint wash, with a label at its bottom: "Download 2 items to ~/Downloads" /
  "Upload 1 item to /public_html/…" (the destination folder in mono, ellipsised).
- **Invalid target** (the source pane, a pane whose site is disconnected):
  no outline, the not-allowed cursor; dropping does nothing.
- **Files from the OS** hovered over the remote pane get the same outline and label
  ("Upload 3 files to …"; when the count is not known yet, "Upload files to …").
- Escape or leaving the window cancels the drag (today the window leave already does).

### Resizable compare panel and queue (point 4 of the conversation, F)

As in Goals. Details:

- The divider is a 7 px hit area with a 1 px rule and a small grip in the middle; hover
  shows the grip in `muted` and the resize cursor.
- Heights are stored as logical pixels: `compare_height`, `queue_height` (settings file;
  invalid → default, unknown keys kept, as for the other keys). They apply to every file
  tab. Heights are clamped at layout time to the minimums and the available space, never
  rewritten by a small window (the user's chosen height comes back when the window grows).
- The compare panel's collapsed summary line shows the three group counts with their
  icons, the automatic toggle, expand and close. While a run is going, it shows the
  progress text instead of the counts.
- Terminal tabs have only the queue divider.

### Visual fixes (A, B, G, H, I)

- **A:** a disabled text/icon button uses a grey of about 3:1 contrast on `paper` (a new
  palette token, e.g. `disabled`), and disabled outline buttons have a dashed border.
  Enabled vs disabled must be told apart at a glance in the toolbar of the mockup.
- **B:** ellipsis + tooltip on every single-line text that can overflow (list above).
  Tooltips use the existing tooltip style.
- **G:** selection as in Goals, in file panes and the compare list.
- **H:** icon buttons are at least 22×22 px hit area; the tab × included.
- **I:** as in Goals; every icon-only button has a tooltip with its label and shortcut.

### Text

All new labels (menu items, tooltips, the drop label, the summary line, settings section
titles and descriptions, "Show workspaces", …) in `en.ftl` and `pt-BR.ftl`, sentence case,
through the existing coverage tests. Counts use the catalogs' plural rules.

## Delivery

One plan in three parts, one branch and PR each, in this order:

1. **Visual base:** icon font and module, logo, sidebar (sections, hide, connection state,
   hover edit), icons across toolbars, rows, tabs, queue, status; A, B, C, G, H, I.
2. **Settings tab and resizable panels:** the site-less tab kind, settings layout with
   navigation, Ctrl+,; the two dividers, saved heights, the collapsed summary line.
3. **Interactions:** context menus everywhere listed (absorbing D's menu part), Copy path,
   Close other tabs; drag threshold, ghost, target outline and label, OS-drop label.

Each part ends with green CI on the three OSes, CHANGELOG under the next unreleased
version, and a QA checklist (`docs/qa/ui-polish-checklist.md`) with NEEDS HUMAN rows (real
mouse right-click and drag on Linux, macOS, Windows; HiDPI icon crispness; keyboard-only
menus).

**Screenshots (the author asked for this as its own step of the plan):** after part 3, the
README images (`docs/images/*.png`, `scripts/readme-shots.sh`) and the site images
(`site/public/shots/*`, `og.png`, `site/scripts/capture/`, with `shotAlt` in both site
languages) are regenerated to show the new UI: icons, the logo, a context menu open, a drag
in progress, the Settings tab. Both capture scripts today install the released `.deb`; they
gain an option to use a locally built `.deb` (`scripts/package-deb.sh`) so the images can be
made before the release. They land with the release that ships this UI (the release PR or
right after the tag, as with 1.2.0), so the site never shows UI the download does not have;
the site redeploys through `site.yml` once the release and `facts.ts` match. The README's
"Using hoprs" text is updated in the same step (right-click menus, Settings tab, resizable
panels).

## Tests

- **Icons:** every `Icon` variant's codepoint exists in the embedded font (a test reads
  the font's cmap); the extension → kind table.
- **Settings tab:** opening twice focuses one tab; Ctrl+, opens it; closing it; not
  restored with a workspace; a settings change made in the tab is saved as today.
- **Context menus:** right-click on an unselected row selects it alone; on a selected row
  keeps the selection; each menu offers the expected items with the right enabled state
  for local/remote, file/folder, connected/disconnected; Escape / outside click closes;
  choosing an item sends the same message as the toolbar or shortcut; Copy path text for
  one and several items.
- **Drag:** a press without a 4 px move does not start a drag; a drag over the other pane
  sets the target and its label; over the source pane, no target; drop on a valid target
  enqueues as today; Escape cancels.
- **Panels:** divider drag changes and saves the height; clamping to minimums; double-click
  resets; heights survive a restart (settings round-trip); invalid values read as the
  default; collapse/expand keeps the height.
- **Sidebar:** collapse/hide state saved and restored; hiding both shows the hint;
  connection state shown per site state.
- **i18n:** all new keys in both catalogs (existing coverage tests).
- **Perf:** the existing 100k-row bench stays within its target with icons in rows.

## Constraints

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features --
  -D warnings` clean on every commit; tests pass on Linux, macOS and Windows.
- Only `hoprs-app` changes (no new crate edges). No new dependency except, if needed for
  the logo, iced's `image-without-codecs` feature with the existing `png` decoder (the full
  `image` and `svg` features stay off). The Phosphor font is an asset, not a crate.
- Mockup v2 rules: neutral near-black, white accent, Inter for UI, JetBrains Mono for data,
  4 px radius, 1 px rules, no shadows, errors inline, no modal cascades.
- Every user-visible text in both catalogs; errors through `strings::error`.
- Fonts' licences shipped next to them (Phosphor: MIT), and listed wherever the other
  fonts' licences are listed (packaging, about).
- CHANGELOG, README (screenshots, "Using hoprs" mentions right-click and Settings tab),
  `docs/ROADMAP.md` kept in step.

## Builder-check rulings (2026-11-16, the controller's calls)

1. **Queue height.** The saved `queue_height` is the queue's **maximum**: the queue still
   fits its rows (as today) up to that height. Dragging the divider sets the maximum;
   double-click resets it to today's `QUEUE_MAX_HEIGHT`. An empty queue stays at its header
   plus the one-line empty state. No empty space is kept below few rows.
2. **Keyboard menus.** Shift+F10 / the Menu key open the menu only where the keyboard
   already has focus: the file panes and the compare list. Sidebar, tab and queue menus are
   mouse-only; their actions stay reachable through shortcuts and the palette. No new focus
   model in this work.
3. **Sidebar menus** hold only existing actions (see Context menus): no Disconnect, no
   Delete, no "open terminal for a site without a tab".
4. **Settings tab and the bottom area.** While the settings tab is selected, the compare
   panel and the queue are not shown and the status bar shows no connection. How the tab is
   modelled (a site-less `Tab` variant or a separate slot drawn in the tab bar) is the
   builder's choice; the user sees one tab bar.
5. **Ctrl+, in a terminal** opens Settings too (it sends no control code to a shell, so
   nothing is taken from the terminal).

## Left to the builder

Exact pixel values not fixed above (paddings, the ghost's offset from the pointer), how the
overlay layers are composed in iced, the file-kind table's exact extension lists, the name
of the disabled colour token, where the icon and menu modules live, and wording of tooltips
and menu labels within the meaning above.
