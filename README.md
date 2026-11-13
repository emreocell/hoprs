# hoprs
A fast, native SSH/SFTP/FTP desktop client written in Rust.
Design spec: [docs/superpowers/specs/2026-11-11-hoprs-design.md](docs/superpowers/specs/2026-11-11-hoprs-design.md)

## Running

```sh
cargo run -p hoprs-app --release
```

Profiles live in `~/.config/hoprs` (`sites.toml`, `workspaces.toml`, `settings.toml`) and
the transfer queue in `~/.local/share/hoprs/queue.db`. Passwords never go in those files:
they are kept in the system keyring. Only one instance runs at a time.

## Terminal

An SSH site opens as files, terminal or both ("Open as" in the site form); its terminal
and files tabs share one connection, and "New terminal tab" / "New files tab" add more.
The terminal is `xterm-256color` with truecolor, keeps 10 000 lines of scrollback by
default, and its font size (10 to 24 px) and scrollback (1 000 to 100 000 lines) are in
Settings. When the connection drops, a bar offers "Reconnect" and a new shell opens.

## Remote editing

Select a remote file and press F4 (or "Edit"): hoprs downloads it to
`$XDG_RUNTIME_DIR/hoprs/edit/`, opens it with the editor set in Settings › Remote editing
(default `xdg-open`; for example `gedit` or `code --wait`, the file's path is added last)
and uploads it again each time you save (after 500 ms of quiet). If the file changed on
the server since you opened it, nothing is overwritten: the strip offers Overwrite, Keep
remote or Save a copy. Closing a tab or the window with unsent changes asks first, and
unsent changes are kept as copies in `~/.local/share/hoprs/recovered`.

## Keyboard shortcuts

| Key | Where | Does |
|---|---|---|
| Ctrl+K | anywhere (Ctrl+Shift+K in a focused terminal) | Go to anything: tabs, sites, workspaces, actions |
| Ctrl+Shift+C / Ctrl+Shift+V | terminal | Copy the selection / paste |
| Ctrl+Shift+F | terminal | Find in scrollback |
| F2 | file pane | Rename |
| F4 | remote pane | Edit in your editor |
| F7 | file pane | New folder |
| Del | file pane | Delete |
| Ctrl+Enter | file pane | Upload or download the selection |
| Tab | files tab | Switch pane |

Plain Ctrl+K in a focused terminal goes to the shell (it kills the rest of the line in
bash). The Log button in the status bar opens the technical log (no secrets, no terminal
contents).

## System dependencies

Linux (Ubuntu) is the v1 target. At run time hoprs needs:

- **A Wayland or X11 session.** iced draws through wgpu (Vulkan or OpenGL), so a working
  GPU driver (Mesa is enough) is required.
- **The Secret Service and a keyring daemon** (GNOME Keyring or KWallet) to save passwords
  and key passphrases. Without one, hoprs still connects but asks for the password every
  time.
- **The XDG desktop portal** (`xdg-desktop-portal` with a GNOME/GTK or KDE backend) for the
  "Choose…" file dialogs (through the `rfd` crate). `rfd` falls back to `zenity` when no
  portal answers; that fallback is untested.
- **`ssh-agent`** (optional), used when a site has "Use ssh-agent" ticked.
- **A text editor** for remote editing; `xdg-open` (from `xdg-utils`) by default.

Building needs a Rust toolchain (stable) and a C compiler (`build-essential`) for the
crypto dependencies.

## Tests

```sh
cargo test --workspace --all-features
# Integration tests against the Docker test servers (OpenSSH, vsftpd):
docker compose -f docker/compose.yml up -d --build --wait
cargo test --workspace --all-features -- --ignored --test-threads=1
```

Performance targets (spec §9) and their last numbers: [docs/perf/README.md](docs/perf/README.md).
