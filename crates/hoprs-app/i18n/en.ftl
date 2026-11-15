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
