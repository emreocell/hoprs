# Folder compare — design

Status: approved by the author (2026-11-15) in conversation: "ok, isso mesmo", after the
proposal below was shown with a mockup. Third of the three community projects (after the pt-BR
UI and the contributor docs).

## Intent

The author wants to **see what is different between a local folder and a remote folder
before deciding anything** — not an automatic sync. hoprs shows the differences of the whole
tree under the two folders open in a file tab; the user picks items and sends them one way or
the other through the existing transfer queue. Nothing is transferred or deleted without a
click.

The comparison runs **automatically** when a file tab opens and whenever either pane changes
folder, because that is how the author wants to use it; since some users will not want it on
all the time, a toggle in the panel itself and the same setting in Settings turn the automatic
run off (then it runs only on demand).

## Goals

- Compare the local and the remote folder of a file tab **recursively**, in the background,
  without blocking navigation, transfers or the terminal.
- Show only what differs, grouped: only on this computer, only on the server, different.
- Let the user select items and **Upload selected** / **Download selected**; the files go
  through the transfer queue and replace the other side's version.
- Automatic comparison on by default, switchable in the panel and in Settings; a manual
  **Compare** in the toolbar and in the Ctrl+K palette always works.

## Non-goals (this version)

- Deleting what exists on one side only.
- Showing the contents of two versions side by side (a text diff).
- One-click "make the server match my folder" (mirror / sync in one direction).
- Comparing file contents (checksums): it would mean downloading everything.
- Comparing two remote sites, or two local folders.

All five go to the follow-ups.

## Anti-goals

- A comparison that slows the app down, holds the FTP control connection for long stretches
  so the panes stop listing, or keeps listing a huge tree nobody asked for.
- Any transfer, overwrite or delete the user did not click for.
- A panel that cannot be turned off.

## Approaches considered

1. **Recursive comparison in a panel of differences (chosen).** Walks both trees, shows only
   the differing items with relative paths. Works for real projects (changes deep in
   `src/components/`).
2. **Mark differences in the two panes, one level only.** Simple and fast, but blind to
   subfolders. Rejected by the author.
3. **Mark differences in the panes, recursively (badges on folders that contain changes).**
   Needs per-folder aggregation and still makes the user drill down to find each file;
   the flat list of differences answers "what changed?" in one look.

## Design

### Where the comparison lives

- The tree walk and the diff are a new module **`hoprs_remote::compare`**: it takes two
  `RemoteFs` (the local disk is already a `RemoteFs`, `LocalFs`) and two root paths, and
  produces the result below. No GUI, testable with `MemFs` and a temporary directory, no new
  crate edges.
- `hoprs-app` runs it from the backend (one comparison per file tab at a time; starting a new
  one cancels the old), streams progress to the UI, and owns the panel.

### What is compared

- Both trees are walked from the two open folders. Entries are matched by relative path,
  **case-sensitively** (a case-insensitive local file system is a known limit, see
  follow-ups).
- A **folder that exists on one side only** is reported once, as a folder (with the number of
  entries in it when known), and is **not walked**: one row, not one per file inside.
- A **file on both sides** is *different* when:
  - the sizes differ, or
  - both sides have a modification time and they differ by more than the tolerance:
    **2 seconds**, widened to **60 seconds** when the remote listing only has minute
    precision (FTP `LIST` without `MLSD`). It reads "local is newer" or "server is newer";
    equal times with different sizes read "size differs".
  - When either side has no modification time, only the size counts.
- **A file on one side and a folder of the same name on the other** is shown under different
  as "file vs folder" and cannot be transferred from the panel.
- **Symlinks** are listed as they are today and never followed; a symlink is compared by
  name only (present / absent), never transferred from the panel.
- **Ignored names** are skipped on both sides at any depth: default `.git`, `node_modules`,
  `.DS_Store`; editable in Settings as a comma-separated list of names, where `*` matches any
  run of characters (`*.log`). Matching is on the entry's name, not on the path.

### Cost and limits

- **One listing at a time on FTP** (listings share the control connection with the panes, so
  pane navigation gets its turn between them); up to **4 concurrent listings on SFTP**; the
  local walk runs off the UI thread.
- **Limit: 10 000 entries per side.** When a side reaches it, the walk pauses and the panel
  says how many items it has seen so far, with **Compare everything** (continues without a
  limit for this run) and the partial result shown meanwhile, marked as partial.
- The run is cancelled when the tab closes, a pane changes folder (a new run starts), the
  user presses ✕, or the connection drops (the panel then says the comparison stopped, with
  **Compare again**).
- A listing error in a subfolder (permission denied, vanished) does not stop the run: the
  folder is shown as "could not read" with the error text (through `strings::error`), and
  the rest continues.

### When it runs

- **Automatic on** (default): when a file tab has both panes listed after opening, and after
  either pane changes folder or is refreshed, with a 500 ms debounce so fast navigation does
  not start runs it cancels right away. Only while the site is connected.
- **Automatic off:** only the **Compare** toolbar button (both panes' toolbar area, label
  "Compare") and the palette action "Compare folders" start it.
- Turning the automatic toggle off in the panel saves the setting at once (it is the same
  setting as in Settings) and leaves the current panel open; turning it on starts a run.

### The panel

Between the file panes and the transfer queue, in the file tab, collapsible like the queue.

```
Differences  ~/proj ↔ /srv/app        [✓] Compare automatically   ✕
───────────────────────────────────────────────────────────────────
Only on this computer (2)
  [ ] src/components/nav.js                     3 KB
  [ ] docs/new.md                               1 KB
Only on the server (1)
  [ ] public/old.css                            8 KB
Different (2)
  [ ] src/app.js        local is newer       12 KB · 11 KB
  [ ] .env              server is newer       1 KB ·  2 KB
───────────────────────────────────────────────────────────────────
1 204 identical            [Upload selected]  [Download selected]
```

- Header: the two roots (the remote one in mono), the automatic toggle, ✕ (hides the panel
  for this tab until the next run: next folder change with automatic on, or **Compare**).
- While running: a progress line ("Comparing… 3 120 items") and the rows found so far; a
  **Stop** button.
- Groups with counts; empty groups are hidden. Each row: checkbox, relative path (mono),
  the status, size(s) (and dates on hover/tooltip). Group headers have a select-all
  checkbox. Rows are virtualised (the existing `virtual_list`), so tens of thousands of
  differences stay smooth.
- **Nothing different:** "The two folders are identical (1 204 items)".
- **Upload selected** is enabled when the selection contains items that exist locally
  (only-local and different); **Download selected** when it contains items that exist on the
  server (only-remote and different). Items that cannot go that way are left out of that
  action. Each becomes one queue item (a folder → one folder transfer), with the **overwrite**
  conflict policy for that item (the user chose it after seeing the difference), not the
  global default.
- Once queued, a row shows "queued" and its checkbox clears; when the transfer finishes the
  row is marked "done" (it stays until the next run). With automatic on, a run starts again
  when every transfer the panel queued has finished.
- **Double-click a row** (or Enter on it): both panes navigate to that item's parent folder
  (local and remote), with the item selected. With automatic on, this does **not** start a
  new run for the subfolder (navigation from the panel is "look closer", not "compare this
  folder instead"); the panel keeps its result until the user goes back to the compared
  folders, presses **Compare**, or changes folder by other means.
- Keyboard: the panel is focusable; arrows move, Space toggles, Enter opens.
- All text through the catalogs (en + pt-BR), sentence case, mockup v2 style (no shadows,
  1 px rules, inline errors).

### Settings

- **Compare folders automatically** (`compare_auto`, default `true`).
- **Ignore when comparing** (`compare_ignore`, default `.git, node_modules, .DS_Store`).
- Both in the settings file with the existing rules (unknown values kept on save, invalid
  ones read as the default).

### Engine support

- `TransferEngine` gains a way to enqueue with a per-item conflict policy (persisted as the
  item's existing `next_policy`), used by the panel with `Overwrite`. Existing callers keep
  using the global policy.

## Tests

- `compare` module (with `MemFs` on both sides and with `LocalFs` on a temp dir): only-local,
  only-remote, size differs, local newer, server newer, within tolerance = identical,
  minute-precision tolerance, missing mtime → size only, folder on one side reported once
  and not walked, file vs folder, symlink by name, ignored names at depth and with `*`,
  limit pause and continue, cancellation stops listing, a subfolder listing error is
  reported and the walk continues, FTP-style one-at-a-time listing (a `RemoteFs` double that
  fails the test if two listings overlap).
- App tests (FakeBackend): auto run on tab open and on folder change (debounced, previous run
  cancelled), no run with automatic off, toggle in the panel saves the setting, ✕ hides until
  next run, upload/download selected enqueue the right specs with `Overwrite`, rows go
  queued → done, auto re-run after the queued batch finishes, double-click navigates both
  panes without re-running, connection drop stops the run.
- Engine: an item enqueued with `Overwrite` overwrites an existing destination without a
  conflict question, also after a restart (persisted).
- Settings: both keys round-trip; defaults; unknown values.
- i18n: every new text in both catalogs (the existing coverage tests).
- Docker integration: a compare against the OpenSSH and vsftpd containers (SFTP and FTP)
  finds a planted difference.
- QA checklist rows (NEEDS HUMAN): a real project folder against a real server on SFTP and
  FTP, a large tree (limit prompt), the panel at 1280 px in both languages.

## Constraints

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` clean on every commit; tests pass on Linux, macOS and Windows.
- No new crate edges; no new dependencies unless unavoidable (a glob crate is not needed:
  `*` matching on names is a few lines).
- Every user-visible text in `crates/hoprs-app/i18n/en.ftl` and `pt-BR.ftl`; errors through
  `strings::error`.
- CHANGELOG under 1.2.0 (unreleased); README "Using hoprs" gains a "Compare folders" part;
  `docs/ROADMAP.md` moves folder compare to done-on-main.

## Builder-check rulings (2026-11-15, the controller's calls)

1. **Transferred files that keep an old mtime.** Some servers cannot set a file's
   modification time (FTP without `MFMT`), so a file just uploaded can still read "local is
   newer". The tab remembers, for this session, each file the panel transferred successfully,
   with both sizes after the transfer; on later runs such a file whose sizes still match and
   whose only difference is the mtime counts as identical. The memory is per tab and dropped
   when the tab closes. The automatic re-run after a batch happens once per batch, so it can
   never loop.
2. **Minute precision.** `RemoteFs` gains `fn mtime_precision(&self) -> Duration` with a
   default of 1 s; the FTP backend returns 60 s when it lists with `LIST` (no `MLSD`). The
   compare tolerance is `max(2 s, precision of either side)`.
3. **Per-item policy** applies to file items. A folder that exists on one side only cannot
   conflict and is queued with the normal policy.
4. **Row states:** queued → done, or failed (the error inline, through `strings::error`),
   or skipped (the reason), or cancelled (the row becomes selectable again). Rows map to queue
   items by the `TransferId` the enqueue returns; a folder item is done when the engine
   reports the folder item done.
5. **The limit** counts entries seen per side, ignored names excluded. A paused run holds no
   listing in flight. Fairness on FTP needs no session change: the walk issues one listing at
   a time and awaits it, so a pane's listing waits for at most one compare listing.
6. **Connections:** the comparison uses the tab's existing session (its `RemoteFs`); it never
   opens a connection of its own.
7. **Double-click on a one-sided item:** each pane goes to the item's parent folder if that
   folder exists on that side, otherwise to the nearest existing ancestor up to the compared
   root.
8. **Roots** are captured when a run starts; results are relative to them.

## Left to the builder

Data structures of the result and the progress stream; how the panel shares space with the
queue (sizes, collapse state); the exact debounce mechanics; how the per-item policy is
threaded through the engine API; copy wording within the meaning above.
