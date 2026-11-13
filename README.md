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

## System dependencies

Linux (Ubuntu) is the v1 target. At run time hoprs needs:

- **A Wayland or X11 session.** iced draws through wgpu (Vulkan or OpenGL), so a working
  GPU driver (Mesa is enough) is required.
- **The Secret Service and a keyring daemon** (GNOME Keyring or KWallet) to save passwords
  and key passphrases. Without one, hoprs still connects but asks for the password every
  time.
- **The XDG desktop portal** (`xdg-desktop-portal` with a GNOME/GTK or KDE backend) for the
  "Choose…" file dialogs. `zenity` is used as a fallback.
- **`ssh-agent`** (optional), used when a site has "Use ssh-agent" ticked.

Building needs a Rust toolchain (stable) and a C compiler (`build-essential`) for the
crypto dependencies.

## Tests

```sh
cargo test --workspace --all-features
# Integration tests against the Docker test servers (OpenSSH, vsftpd):
docker compose -f docker/compose.yml up -d --build --wait
cargo test --workspace --all-features -- --ignored --test-threads=1
```
