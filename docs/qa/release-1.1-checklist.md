# Release 1.1 checklist

One section per task; later tasks append theirs.

## Task 2: no console window on Windows (2026-11-14)

| Check | Result |
| --- | --- |
| `hoprs.exe --version` piped (grep for the `hoprs ` prefix) | Pass: CI, `windows-latest` |
| `hoprs --version` in a plain cmd window prints the version | NEEDS HUMAN: CI only covers the piped case. Steps: on Windows, open cmd.exe (and again PowerShell), run `hoprs.exe --version` from the install folder, expect the `hoprs <version>` line printed in that window |
| Double-clicking `hoprs.exe` opens the app with no console window | NEEDS HUMAN: on Windows, double-click the exe, expect only the app window |

## Task 11: version 1.1.0 and the docs (2026-11-14)

| Check | Result |
| --- | --- |
| `scripts/release-notes.sh 1.1.0` prints the section and the Installing block | Pass |
| `facts.test.ts` and `cargo test --workspace --all-features` with version 1.1.0 | Pass (see the task report) |
| Assets named as in the README: `hoprs_1.1.0-1_amd64.deb`, `hoprs_1.1.0_macos_universal.dmg`, `hoprs_1.1.0_windows_x64.msi`, `hoprs_1.1.0_windows_x64.zip`, `SHA256SUMS` | Pass: the `release.yml` dry run (`workflow_dispatch`) builds and checks all of them |
| Everything below needs a real Mac and a real Windows PC | NEEDS HUMAN |

### NEEDS HUMAN

Run these after the tag `v1.1.0` publishes the Release (steps 1 and 2 can use the dry run's
artifacts before that). Download from the Release page, never from a build folder.

1. **Install, per system.** Check the hash first. Linux: `sha256sum -c SHA256SUMS
   --ignore-missing`, then `sudo apt install ./hoprs_1.1.0-1_amd64.deb`. macOS:
   `shasum -a 256 -c SHA256SUMS --ignore-missing`, open the `.dmg`, drag hoprs to
   Applications. Windows: compare `(Get-FileHash .\hoprs_1.1.0_windows_x64.msi).Hash` with
   `SHA256SUMS`, run the `.msi` (per machine, UAC prompt); also unzip the `.zip` into a
   folder and run `hoprs.exe` from there. Expect: the app opens with the right icon and
   `hoprs --version` prints `hoprs 1.1.0`; on Windows the Start menu entry and the desktop
   shortcut exist, and `README.txt` and the licences are in the `.zip`.
2. **First launch past Gatekeeper (macOS).** Open hoprs from Applications: macOS refuses.
   System Settings, Privacy & Security, "Open Anyway", confirm. Expect: it opens, and the
   next launches need nothing. Repeat on a fresh download with
   `xattr -dr com.apple.quarantine /Applications/hoprs.app` instead. Check an Apple Silicon
   and an Intel Mac if you can (one universal build).
3. **First launch past SmartScreen (Windows).** Run the `.msi` and later `hoprs.exe` from
   the `.zip`: "Windows protected your PC", then "More info", "Run anyway". Expect: installs
   and opens. Also: double-click `hoprs.exe` shows no console window.
4. **Keyring save and read.** Add an ssh site with "Save password", close hoprs, reopen,
   connect. Expect: no password prompt. macOS: the entry is in Keychain Access under
   `hoprs/<site-id>`. Windows: Credential Manager, Windows Credentials, same name. Linux:
   Seahorse.
5. **Keychain "Always Allow" after an update (macOS).** With 1.1.0 installed and a saved
   password, install the next build over it, open a saved site. Expect: Keychain asks once
   per saved password; choose "Always Allow"; the next connection asks nothing. (Until a
   later release exists, install a dry-run `.dmg` over the release one.)
6. **ssh-agent.** macOS and Linux: `ssh-add` a key, connect to an ssh site with "Use
   ssh-agent". Windows: in PowerShell (admin) `Set-Service ssh-agent -StartupType Manual;
   Start-Service ssh-agent`, then `ssh-add $env:USERPROFILE\.ssh\id_ed25519`, then connect
   with "Use ssh-agent". Expect: connects without a password. Also try from a Git Bash
   window (MSYS `SSH_AUTH_SOCK`): it falls back to the OpenSSH pipe.
7. **FileZilla import.** With a real `sitemanager.xml` in `~/.config/filezilla` (macOS) or
   `%APPDATA%\FileZilla` (Windows), use "Import from FileZilla". Expect: the report lists
   the imported sites, which connect.
8. **F4 remote edit opens the default editor.** On a remote text file press F4 with the
   editor setting at `system`. macOS: opens in the default text editor (`open -t`). Windows:
   opens in the associated app; for a remote file with no extension, the Open With dialog
   is acceptable. Edit and save: the file uploads (status shows it). Also try
   `code.cmd --wait` as a custom editor on Windows.
9. **Command palette.** macOS: Cmd+K opens it; in a focused terminal tab Ctrl+Shift+K does,
   and plain Ctrl+K reaches the shell. Windows: Ctrl+K and Ctrl+Shift+K likewise.
10. **Upgrade keeps your sites.** Install 1.1.0, add a site and a workspace, then install
    the next release over it (Windows: run the newer `.msi`; macOS: replace the app).
    Expect: sites and workspaces are still there.
11. **Uninstall keeps data.** macOS: drag hoprs to the Bin. Windows: Settings, Apps,
    hoprs, Uninstall. Expect: the app is gone, `~/Library/Application Support/hoprs` or
    `%APPDATA%\hoprs` is still there with your files, and the saved passwords are still in
    Keychain / Credential Manager (reinstall and open a site to confirm).
12. **The `.app` icon at small sizes (macOS).** In Finder (list view and column view), the
    Dock and Cmd+Tab, the icon must be the dark rounded square with the white "h", sharp
    and not blank. The `.icns` stores the 16 and 32 px sizes as PNGs under `ic04` and
    `ic05`; if they show blank or blurry, that is a bug in `scripts/pack-icons.py`.
13. **The tag creates the Release (after the merge, with the author's confirmation).** On
    `main`: `git tag v1.1.0 && git push origin v1.1.0`. Expect a published (not draft, not
    prerelease) Release with the five assets and the notes from `CHANGELOG.md`, and the
    site deploying afterwards with the three download tabs working.
