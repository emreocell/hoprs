# Release 1.0 checklist

One section per task; later tasks append theirs.

## Task 1: icons and window icon (2026-11-13)

| Check | Result |
| --- | --- |
| 16 px and 24 px PNGs (rendered straight from the SVG) | Pass: the "h" with its arc is still recognisable and not blurred; viewed at 1:1 |
| 256 px PNG | Pass: dark rounded square, white "h", arc ending in a dot |
| Window class | Pass: under XWayland, `xprop WM_CLASS` of the running window = `"io.github.engviniciusbs.hoprs", "io.github.engviniciusbs.hoprs"` |
| `_NET_WM_ICON` of the window (XWayland, re-verified in fix round 1) | Pass: `xwininfo -root -tree` gave the client window `0x2600004` with class `io.github.engviniciusbs.hoprs`; `xprop -id 0x2600004 _NET_WM_ICON` prints `Icon (256 x 256):` (xprop does not dump the pixels). The first pass misread it as empty because the text is on the lines after the `=` |
| Icon visibly drawn in the title bar / dock / alt-tab | NEEDS HUMAN: no screenshot shows window chrome. Steps: `cargo run --release`, look at the icon in the dock and in alt-tab (no `.desktop` is installed yet, so it must come from the window). Under native Wayland the icon comes from the `.desktop` file (Task 2) |

## Task 5: package, install and release (2026-11-14)

Run in the container (Docker, Ubuntu 24.04 base): `scripts/package-deb.sh` then
`scripts/check-deb.sh target/deb-ubuntu-24.04/debian/hoprs_1.0.0-1_amd64.deb`. Exit code 0,
"deb check: all stages OK", 24 stages with `OK`.

| Check | Result |
| --- | --- |
| Package file name | Pass: `hoprs_1.0.0-1_amd64.deb` |
| Dependencies resolve on a clean Ubuntu 24.04, install with Depends only | Pass |
| Exact file list and modes of the package | Pass |
| `hoprs --version` | Pass: prints `hoprs 1.0.0` |
| `desktop-file-validate` | Pass |
| `appstreamcli validate` | Pass: exit 0 or 3 (3 = `url-homepage-missing` warning only, ruling: no repo URL while the repo is private) |
| No builder paths (`$HOME`, `~/.cargo`) in the binary | Pass |
| Startup probe under `xvfb-run`, tiny-skia and default renderer | Pass; default renderer RSS under 80 MB (no GPU render node, so tiny-skia is chosen) |
| Every loaded library comes from `Depends` | Pass |
| `apt-get remove`: installed files and docs gone, user data (`~/.config/hoprs`, `~/.local/share/hoprs`) intact | Pass |
| `purge`: nothing left, user data intact | Pass |
| `package` job in CI (`.github/workflows/ci.yml`) | Not run here: runs on the PR on GitHub |
| `release.yml` on a real tag | NEEDS HUMAN (below) |

### NEEDS HUMAN

1. **Install on your Ubuntu.** Build or download the package, then:
   `sha256sum -c SHA256SUMS` and `sudo apt install ./hoprs_1.0.0-1_amd64.deb`. Expect: apt
   installs without errors, `hoprs --version` prints `hoprs 1.0.0`, "hoprs" shows in the
   app menu. Launch it from the menu: the window opens and your sites are there.
2. **Icon in the app menu and the dock at 16, 24 and 48 px.** After step 1 (log out and in
   if the menu has not refreshed), check the icon in the app grid or menu, in the dock when
   running, and in alt-tab. To force a size, open the dock or panel settings and set the icon
   size to small, medium and large, or view
   `/usr/share/icons/hicolor/{16x16,24x24,48x48}/apps/io.github.engviniciusbs.hoprs.png`
   in an image viewer at 100%. Expect: a dark rounded square with a white "h", recognisable
   and not blurred at each size.
3. **Window grouping in the dock (`StartupWMClass`).** Run hoprs from the app menu, then open
   a second window or pin it: the running window must sit on the pinned launcher (one dock
   icon, not two), and the icon must be the hoprs one, not a generic cog. Check on Wayland
   (default session) and, if you can, on an X11 session.
4. **Uninstall without losing data.** `sudo apt remove hoprs`: the menu entry and `hoprs`
   command are gone; `ls ~/.config/hoprs ~/.local/share/hoprs` still lists your files and
   your saved passwords are still in the keyring (reinstall and open a site to confirm).
5. **The tag creates the Release (after the merge, with the author's confirmation).** On
   `main`: `git tag v1.0.0 && git push origin v1.0.0`. In the Actions tab `release.yml` runs:
   version check, build, draft Release, verify, publish. Expect a published (not draft, not
   prerelease) Release `v1.0.0` with `hoprs_1.0.0-1_amd64.deb`, `SHA256SUMS` and notes from
   `CHANGELOG.md`. Download both files from the Release page, run `sha256sum -c SHA256SUMS`,
   and install that downloaded `.deb` (step 1). If it fails, the workflow removes its own
   draft; delete the tag and retry.
