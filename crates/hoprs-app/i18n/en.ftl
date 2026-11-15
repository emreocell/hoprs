# hoprs interface text, English. This file is the reference for every translation.
#
# Format (Fluent, https://projectfluent.org):
#   key = text                 one message per key; keys are kebab-case.
#   # comment                  explains a message to translators; it is never shown.
#   { $name }                  a value the app fills in. Keep the variable names exactly
#                              as they are; the order and the words around them may change.
#   Plurals use a selector, one line per form, the line with * is the default:
#       site-count =
#           { $count ->
#               [one] { $count } site
#              *[other] { $count } sites
#           }
#   Keep technical names (SSH, SFTP, FTP, FTPS) as they are.
# pt-BR.ftl must define the same keys with the same variables.

app-title = hoprs
already-running = hoprs is already running
cannot-open-profiles = Cannot open the saved sites

# A number of saved sites: "1 site", "3 sites".
site-count =
    { $count ->
        [one] { $count } site
       *[other] { $count } sites
    }

# The settings page: the interface language, and its choices. The two language names are
# written in themselves, so they are the same in every catalog.
settings-language = Language
language-system = System
language-english = English
language-pt-br = Português (Brasil)

## Top bar

brand = hoprs
no-sessions = No open sessions
go-to-anything = Go to anything
go-to-anything-key = Ctrl K

## The Ctrl+K palette

palette-placeholder = Go to a tab, site or workspace, or run an action
palette-workspaces-placeholder = Open workspace
palette-no-results = No matches
palette-group-action = Action
palette-group-tab = Tab
palette-group-site = Site
palette-group-workspace = Workspace
palette-kind-terminal = terminal
palette-kind-files = files
palette-toggle-queue = Toggle queue
palette-toggle-log = Toggle log
palette-close-tab = Close tab
palette-open-workspace = Open workspace…
palette-edit = Edit in editor
palette-upload = Upload selection
palette-download = Download selection
palette-copy = Copy
palette-paste = Paste
palette-find = Find in scrollback
palette-key-copy = Ctrl Shift C
palette-key-paste = Ctrl Shift V
palette-key-find = Ctrl Shift F

## Sidebar

filter-sites = Filter sites
workspaces = Workspaces
all-sites = All sites
new = + New
no-workspaces = No workspaces yet
no-sites = No sites yet
import-filezilla = Import from FileZilla
settings = Settings
new-site = New site
no-matches = No matching sites
open-workspace = Open workspace
edit = Edit
site-missing = Site missing
cannot-load-sites = Can't load the saved sites
retry = Retry
unreadable-entry = Can't read this entry (written by a newer hoprs?)
open-profile-folder = Open sites.toml folder
# Opening the profile folder failed. $reason is the system's explanation.
cannot-open-folder = Can't open the folder: { $reason }
# A workspace's count when some of its sites are missing: "2 of 3 sites". When all are
# present, site-count is used instead.
workspace-site-count-partial =
    { $total ->
        [one] { $present } of { $total } site
       *[other] { $present } of { $total } sites
    }
# An unreadable entry that has no name: $number is its position (from 1), $file the file.
unreadable-name-fallback = Entry { $number } in { $file }

## Site form

site-form-new = New site
site-form-edit = Edit site
field-name = Name
field-name-hint = Defaults to the host
field-host = Host
field-port = Port
field-user = User
field-protocol = Protocol
field-password = Password
password-optional = Optional
password-replace = Type to replace
password-saved = Saved in the system keyring
forget-password = Forget the saved password
ssh-options = SSH
use-agent = Use ssh-agent
key-file = Key file
key-file-hint = Path to a private key (optional)
open-as = Open as
open-as-terminal = Terminal
open-as-files = Files
open-as-both = Terminal and files
ftp-options = FTP
ftp-mode = Transfer mode
ftp-passive = Passive
ftp-active = Active
nat-workaround = Ignore the passive address the server sends (NAT)
start-folders = Start folders
remote-dir = Remote folder
local-dir = Local folder
folder-hint = Optional
host-required = Host is required
port-invalid = Port must be a number from 1 to 65535
save = Save
saving = Saving…
cancel = Cancel
delete = Delete
keep = Keep
delete-site-confirm = Delete this site? It leaves every workspace and its saved password is deleted.

## Workspace form

workspace-form-new = New workspace
workspace-form-edit = Edit workspace
workspace-sites = Sites
workspace-no-sites = No sites yet. Create a site first.
name-required = Name is required
delete-workspace-confirm = Delete this workspace? Its sites are kept.

## Center

no-session-title = No session open
no-session-hint = Pick a site in the sidebar to connect.
keyring-unavailable = The system keyring was unavailable, so the saved password was not used.
edit-site = Edit site

## Tabs

close-tab = ×
# The tab strip's actions on a connected SSH tab: another tab on its session.
new-terminal-tab = New terminal tab
new-files-tab = New files tab
terminal-overflowed = Output was too fast and the shell was closed
terminal-reopen = Reopen
terminal-restart = Restart shell
terminal-shell-closed = Shell closed
terminal-open-failed = Can't open a shell
terminal-retry = Retry
terminal-waiting-for-connection = Waiting for the connection
terminal-input-dropped = Some input was dropped: the shell was not taking it fast enough
# Written into the scrollback when a new shell starts after a reconnect.
terminal-reconnected = — reconnected —
terminal-search-placeholder = Search scrollback
terminal-search-next = Next
terminal-search-previous = Previous
terminal-search-close = Close
terminal-search-no-match = No match
# "Shell exited (code N)": the shell ended and the server sent an exit code.
shell-exited-code = Shell exited (code { $code })
# The shell ended and the server sent no exit code.
shell-exited = Shell exited

## File panes
local = Local
col-name = Name
col-size = Size
col-modified = Modified
col-permissions = Permissions
sort-ascending = ↑
sort-descending = ↓
# The row that goes up one folder.
parent-dir = ..
# After a folder name, and between breadcrumb segments.
path-separator = /
# The breadcrumb's first segment for the home folder.
home-dir = ~
# The size column of a folder.
no-size = -
empty-folder = This folder is empty
reconnect = Reconnect
# Units of [`crate::ui::format::size`], base 1024.
size-unit-b = B
size-unit-kb = KB
size-unit-mb = MB
size-unit-gb = GB
size-unit-tb = TB
size-unit-pb = PB
size-unit-eb = EB
# Short month names for file dates.
month-1 = Jan
month-2 = Feb
month-3 = Mar
month-4 = Apr
month-5 = May
month-6 = Jun
month-7 = Jul
month-8 = Aug
month-9 = Sep
month-10 = Oct
month-11 = Nov
month-12 = Dec

## Queue
transfers = Transfers
queue-active = Active
queue-queued = Queued
queue-failed = Failed
queue-done = Done
queue-hide = Hide
queue-show = Show
no-transfers = No transfers
no-transfers-hint = Select files and press Upload or Download, drag them to the other pane, or drop them here.
queue-loading = Loading transfers…
nothing-active = Nothing running
nothing-queued = Nothing waiting
nothing-failed = No failures
nothing-done = Nothing finished yet
# Removes done, skipped, cancelled and failed rows (the engine's `clear_finished`).
clear-finished = Clear finished and failed
# [`CLEAR_FINISHED`] in a narrow window.
clear-finished-short = Clear finished

## Technical log panel
log = Log
log-error = Error
log-warn = Warn
log-info = Info
log-debug = Debug
filter-log = Filter log
# Copies the lines the filters show (all of them without a filter).
copy-shown = Copy shown
clear = Clear
clear-hint = Empties this panel only; new lines keep coming.
log-level-hint = Shows lines at least this severe. Only info and above are recorded unless hoprs starts with RUST_LOG (for example RUST_LOG=debug).
no-log-lines = Nothing logged yet
no-log-matches = No lines match
# The log panel's count: "1 line", "120 lines".
log-lines =
    { $count ->
        [one] { $count } line
       *[other] { $count } lines
    }
upload-arrow = ↑
download-arrow = ↓
waiting-for-worker = Waiting for a worker
starting = Starting…
# A folder whose files are being copied.
transferring = Transferring…
# A failed row the user dismissed (still failed in the engine; Retry stays).
dismissed = Dismissed
paused = Paused
cancelled = Cancelled
transfer-done = Done
file-exists = File exists —
overwrite = Overwrite
resume = Resume
skip = Skip
pause = Pause
# The engine also makes the answer its policy for every later conflict of this run.
apply-to-all = Apply to all (this session)
site-paused = Paused — connection lost
drop-needs-site = Open a connected site to drop files into it
drop-pane-busy = The remote folder is busy or not listed yet; drop again in a moment
drop-unsupported = Can't upload this path
# After the total speed: " total, 4 workers".
total-workers =
    { $workers ->
        [one] total, { $workers } worker
       *[other] total, { $workers } workers
    }
# `policy-*` (lowercase) read inline in sentences; `settings-policy-*` (capitalised) are the settings picker's options.
policy-ask = ask
policy-overwrite = overwrite
policy-skip = skip
policy-overwrite-if-newer = overwrite if newer
policy-resume = resume
# In the queue head once an answer went "to all": "Conflicts: overwrite (this session)".
session-policy = Conflicts: { $name } (this session)
policy-if-newer = if newer
# The same in a narrow window, as a short mono tag: "overwrite: session".
session-policy-tag = { $name }: session
# The queue database could not be opened: the engine runs on an in-memory queue.
queue-unavailable = Transfer queue unavailable: { $reason }. Transfers this session won't survive a restart.
skip-exists = exists
skip-not-newer = not newer
skip-already-complete = already complete
# "Skipped: exists".
skipped = Skipped: { $reason }

## Status bar
not-connected = Not connected

## Status bar and connection states
connecting = Connecting…
connected = Connected
waiting-for-you = Waiting for you
closing = Saving tabs and closing…
close-workspace = Close tabs
reconnecting-now = Reconnecting…
# A workspace's saved folder is gone; the pane shows a folder above it instead.
saved-folder-gone = The saved folder { $saved } is gone; showing the nearest folder instead.
# "Reconnecting (2/5)".
reconnecting = Reconnecting ({ $attempt }/{ $of })
# "Disconnected — Connection lost".
disconnected = Disconnected — { $reason }
# How an SSH session logged in: "ed25519 via ssh-agent", "rsa key", "password"...
auth-none = no authentication
auth-agent = { $key_type } via ssh-agent
auth-key-file = { $key_type } key
auth-password = password
auth-keyboard-interactive = keyboard-interactive
# An FTP site's security, in place of a login method.
ftp-auth-ftp = FTP (unencrypted)
ftp-auth-ftps-explicit = FTPS explicit
ftp-auth-ftps-implicit = FTPS implicit
# "3 of 3 sites in Shop".
workspace-status = { $connected } of { $sites } in { $name }

## File actions
new-folder = New folder
rename = Rename
permissions = Permissions…
# Short toolbar labels for the narrowest panes (`ui::fit::Toolbar::Narrow`).
new-folder-short = Folder
permissions-short = Perms…
key-new-folder = F7
key-rename = F2
key-delete = Del
key-edit = F4
upload = Upload
download = Download
key-transfer = Ctrl ↵
# The name a new folder starts with.
new-folder-name = New folder
name-empty = The name can't be empty
name-invalid = The name can't contain / or be . or ..
name-invalid-windows = The name can't contain \ on this disk
name-spaces = The name can't start or end with a space
mixed-permissions = Mixed permissions: showing the first item's
name-taken = Something with this name is already here
dismiss = Dismiss
creating-folder = Creating folder…
renaming = Renaming…
changing-permissions = Changing permissions…
delete-folders-note = Folders are deleted with everything in them.
permissions-title = Permissions
perm-owner = Owner
perm-group = Group
perm-others = Others
perm-read = Read
perm-write = Write
perm-execute = Execute
perm-octal = Octal
octal-invalid = Three or four digits from 0 to 7
apply = Apply
# "Delete report.pdf?", "Delete 3 items?".
delete-question-one = Delete { $name }?
delete-question-many = Delete { $count } items?
# "and 4 more".
and-more = and { $count } more
# "Deleting… 12 items".
deleting =
    { $done ->
        [0] Deleting…
        [1] Deleting… 1 item
       *[other] Deleting… { $done } items
    }
# A delete that stopped after removing some items.
deleted-then =
    { $done ->
        [1] Deleted 1 item, then: { $error }
       *[other] Deleted { $done } items, then: { $error }
    }

## Security and credential dialogs
host-key-title = Unknown host key
host-key-intro = hoprs has not connected to this server before. Check the fingerprint with its administrator before you trust it.
trust-and-connect = Trust and connect
cert-untrusted-title = Untrusted certificate
cert-untrusted-intro = The server's certificate can't be verified. Check the fingerprint with its administrator before you trust it.
cert-changed-title = Certificate changed
cert-changed-intro = The server's certificate is not the one you trusted before. Someone may be intercepting the connection.
trust-certificate = Trust this certificate
label-host = Host
label-account = Account
label-key-type = Key type
label-fingerprint = Fingerprint (SHA-256)
label-reason = Reason
label-trusted-before = Trusted before
label-presented-now = Presented now
label-key-file = Key
password-title = Password
passphrase-title = Key passphrase
field-passphrase = Passphrase
kbd-title = Authentication
save-in-keyring = Save in the system keyring
save-in-keyring-hint = Saved only once the connection succeeds
connect = Connect
unlock = Unlock
continue = Continue
# Under a dialog, when others wait behind it: "1 more waiting".
dialogs-waiting = { $count } more waiting
# The warning of a host known with other key types.
other-key-types = This host is known with a different key type ({ $types }). Someone may be impersonating it.
# Why a certificate needs the user's decision.
cert-reason-expired = Expired
cert-reason-not-yet-valid = Not valid yet
cert-reason-wrong-name = Issued for a different name
cert-reason-unknown-issuer = Unknown authority (self-signed or untrusted issuer)

## Changed host key, in the tab
host-key-changed-title = The host key has changed
host-key-changed-body = The server presented a different key than the one recorded for it. That happens when the server was reinstalled, or when someone intercepts the connection. hoprs won't connect while the old entry is in known_hosts.
host-key-changed-session-body = The server presented a different key than the one this session verified when it connected. That happens when the server was reinstalled, or when someone intercepts the connection. A new session checks the key against known_hosts again.
remove-old-entry = Remove the old entry…
close-and-reconnect = Close and reconnect
reading-known-hosts = Reading known_hosts…
removing-known-hosts = Removing…
remove = Remove
connect-again = Connect again
no-known-hosts-entry = known_hosts has no entry for this host any more. Connect again to check the new key.
# "known_hosts line 3".
known-hosts-line = known_hosts line { $line }
# The removal question: "Remove 2 lines for [h]:2222 from known_hosts? …".
remove-known-hosts-question =
    { $lines ->
    [1] Remove 1 line for { $host } from known_hosts? The next connect asks you to trust the new key.
   *[other] Remove { $lines } lines for { $host } from known_hosts? The next connect asks you to trust the new key.
    }
# The warning for lines that also name other hosts.
also-covers-one-host = Line { $line } also covers other hosts (a pattern or a list of names): removing it forgets this key for them too.
also-covers-other-hosts = Lines { $lines } also cover other hosts (patterns or lists of names): removing them forgets these keys for them too.
# A connected tab whose site's host or port changed (an edit, a restore).
site-endpoint-changed = The site's host or port changed: this session was closed. Reconnect to use the new settings.

## Import from FileZilla
import-title = Import from FileZilla
import-step-file = 1  File
import-step-folders = 2  Folders
import-step-report = 3  Report
import-file-label = FileZilla's site manager file
import-file-hint = Usually sitemanager.xml in FileZilla's configuration folder. Passwords go to the system keyring.
import-path-required = Choose the sitemanager.xml file
choose-file = Choose…
next = Next
back = Back
import-folders-question = Turn FileZilla folders into workspaces?
import-folders-hint = Yes: one workspace per folder, named by its path, with the folder's sites. No: the folder path goes in front of each site name ("Work/Alpha").
import-folders-yes = Yes, make workspaces
import-folders-no = No, keep them in the names
import-running = Importing…
import-imported = Imported
import-skipped = Skipped
import-workspaces-created = Workspaces created
import-workspaces-not-created = Workspaces not created
import-failures = Failures
import-none = None
import-failed-keep-source = Some entries could not be saved: keep sitemanager.xml and import again once fixed.
import-passwords-keep-source = The keyring refused the passwords of these sites: keep sitemanager.xml, it still has them.
delete-sitemanager = Delete sitemanager.xml
delete-sitemanager-confirm = Delete FileZilla's sitemanager.xml? FileZilla will no longer have these sites.
deleting-sitemanager = Deleting…
sitemanager-deleted = sitemanager.xml deleted
done = Done
pick-sitemanager = Choose FileZilla's sitemanager.xml
pick-filter-xml = XML files

## Settings
settings-title = Settings
settings-transfers = Transfers
settings-workers = Workers per site
settings-workers-hint = Parallel transfers per site, 1 to 16. Applies to the next connections.
settings-policy = When a file already exists
settings-policy-hint = Applies to the next transfers. Ask shows the choice in the queue.
settings-keepalive = Keepalive
settings-keepalive-hint = How often an idle connection is checked, so servers and routers keep it open. Applies to the next connections.
# A keepalive interval: "30 s".
seconds = { $secs } s
settings-unreadable = settings.toml can't be read, so the defaults apply. Changes last until hoprs closes and are not saved, to keep the file as it is: fix or delete it to save settings.
settings-save-failed = Can't save settings.toml
settings-terminal = Terminal
settings-font-size = Font size
settings-scrollback = Scrollback lines
settings-scrollback-hint = Applies to terminals opened from now on.
settings-theme = Theme
theme-hoprs-dark = hoprs dark
settings-policy-ask = Ask
settings-policy-overwrite = Overwrite
settings-policy-skip = Skip
settings-policy-overwrite-if-newer = Overwrite if newer
settings-policy-resume = Resume
decrease = −
increase = +

## Backup
backup = Backup
backup-hint = One encrypted file with every site, workspace and saved password. Keep the password: without it the backup can't be opened.
export-backup = Export backup…
restore-backup = Restore backup…
backup-file = File
backup-password = Password
backup-password-confirm = Password again
backup-path-required = Choose a file
backup-password-required = Enter a password
backup-passwords-differ = The passwords don't match
export = Export
export-overwrite-confirm = This file already exists. Overwrite it with the new backup?
restore = Restore
exporting = Encrypting the backup… this takes a few seconds
restoring = Decrypting the backup… this takes a few seconds
restore-mode = Mode
restore-merge = Merge
restore-merge-hint = Adds what is new; sites and workspaces you have stay.
restore-replace = Replace
restore-replace-hint = Everything becomes the backup: sites, workspaces and passwords not in it are removed.
restore-replace-confirm = Replace every site, workspace and saved password with the backup's?
replace = Replace
export-failed = The backup was not written
restore-failed = Nothing was restored
backup-secret-errors = Passwords not restored
backup-missing-secrets = Saved without their password (not in the keyring)
backup-skipped-unreadable = Not exported (unreadable entries)
backup-skipped-sites = Sites already here (kept)
backup-skipped-workspaces = Workspaces already here (kept)
backup-file-name = hoprs-backup.hoprs
pick-export = Save the hoprs backup
pick-restore = Choose a hoprs backup
# "Exported 12 sites, 3 workspaces and 9 passwords."
export-done = Exported { $sites }, { $workspaces } and { $secrets }.
workspace-count =
    { $count ->
    [one] { $count } workspace
   *[other] { $count } workspaces
    }
password-count =
    { $count ->
    [one] { $count } password
   *[other] { $count } passwords
    }
# "Restored 2 sites, 1 workspace and 2 passwords."
restore-done = Restored { $sites }, { $workspaces } and { $secrets }.

## Remote editing
# The Edit action (remote pane).
edit-file = Edit
edit-no-folders = Folders can't be edited
edit-no-links = Links can't be edited
edit-too-large = Files over 50 MB can't be edited
waiting-for-connection = Waiting for connection…
remote-may-be-truncated = The server's copy may be incomplete until then.
changed-on-server = Changed on the server since you opened it
keep-remote = Keep remote
save-a-copy = Save a copy
reload-in-editor = Now the server's version: reload it in your editor.
upload-and-close = Upload pending changes and close
close-keep-copies = Close and keep local copies
upload-and-quit = Upload and quit
quit-keep-copies = Quit and keep local copies
checking-edits = Checking the remote edits…
stop-keep-copies = Stop and keep local copies
# While "Upload" runs (an upload under way has no bound but its protocol's timeouts).
uploading-edits = Uploading… (Stop to keep local copies instead)
# The kept notice, before a remote path: "Server copy of".
server-copy-of = Server copy of
# …after it.
may-be-incomplete = may be incomplete
# The kept notice after a close that did not ask (the site moved, "Close and
# reconnect"): the editor's file is no longer linked to the server.
kept-saves-stopped = The session closed: further saves in the editor won't be uploaded. Your changes are in the recovered copy; open the file again to go on editing.
edit-one-file = Select one file to edit
uploaded = Uploaded
editing = Editing
opening = Opening
close-workspace-edits-title = Some remote edits of this workspace are not uploaded
keeping-copies = Saving the local copies…
close-edits-title = Some remote edits are not uploaded
quit-edits-title = Some remote edits are not uploaded
quit-edits-hint = The window stays open until you choose. Nothing is deleted.
edit-unsent = not uploaded
edit-conflict = changed on the server
editor-command = Editor
# Windows' example names `code.cmd`: the command runs without a shell, which does not
# find `.cmd` shims from a bare name.
editor-command-hint-windows = system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: notepad++.exe, code.cmd --wait.
editor-command-hint-macos = system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: open -W -a "Visual Studio Code" (a full path also works).
editor-command-hint = system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: gedit, code --wait.
# Next to the editor field when it holds `system`.
editor-system-default = System default
editor-command-empty = Type a command
settings-editing = Remote editing
# "Upload failed: Permission denied".
upload-failed = Upload failed: { $reason }
# "Your version was saved to <path>".
your-version-at = Your version was saved to { $path }
# Where "keep" puts the copies: "Unsent changes are copied to <dir>".
copies-go-to = Unsent changes are copied to { $dir }
# "2 files could not be uploaded".
not-uploaded =
    { $count ->
    [1] 1 file could not be uploaded
   *[other] { $count } files could not be uploaded
    }
# The persistent notice after a close kept unsent edits: "Kept 2 unsent edits as
# recovered copies in" (the folder follows, in mono).
kept-notice =
    { $count ->
    [1] Kept 1 unsent edit as a recovered copy in
   *[other] Kept { $count } unsent edits as recovered copies in
    }
# The delete confirmation of a site with files open for editing.
delete-site-edits =
    { $count ->
    [1] 1 file of this site is open for editing: its unsent changes will be kept as a recovered copy, never deleted.
   *[other] { $count } files of this site are open for editing: their unsent changes will be kept as recovered copies, never deleted.
    }
