# Release 1.1 checklist

One section per task; later tasks append theirs.

## Task 2: no console window on Windows (2026-11-14)

| Check | Result |
| --- | --- |
| `hoprs.exe --version` piped (grep for the `hoprs ` prefix) | Pass: CI, `windows-latest` |
| `hoprs --version` in a plain cmd window prints the version | NEEDS HUMAN: CI only covers the piped case. Steps: on Windows, open cmd.exe (and again PowerShell), run `hoprs.exe --version` from the install folder, expect the `hoprs <version>` line printed in that window |
| Double-clicking `hoprs.exe` opens the app with no console window | NEEDS HUMAN: on Windows, double-click the exe, expect only the app window |
