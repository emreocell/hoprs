<#
.SYNOPSIS
Proves the hoprs Windows .msi installs, upgrades and uninstalls cleanly, and the
portable .zip runs.

.DESCRIPTION
Usage (on Windows, as an administrator: the .msi installs per machine):
  pwsh scripts/check-windows.ps1 -Msi <hoprs_X.Y.Z_windows_x64.msi> -Zip <hoprs_X.Y.Z_windows_x64.zip>

Checks, in order: the asset names; a file in %APPDATA%\hoprs before the install;
`msiexec /i` (quiet, verbose log) exits 0; %ProgramFiles%\hoprs\hoprs.exe prints
`hoprs <version>` and imports no VC++ runtime DLL (VCRUNTIME*, MSVCP*: the CRT is
linked statically, nothing to install first); the Start menu and desktop shortcuts point
at it; Add/Remove Programs has one per-machine (HKLM) entry with the MSI version
(numeric X.Y.Z); reinstalling the same .msi with REINSTALL=ALL REINSTALLMODE=vomus exits
0 (the upgrade path); `msiexec /x` exits 0; the exe, the shortcuts and the entry are gone
while the file in %APPDATA%\hoprs is still there; the .zip holds exactly hoprs.exe,
LICENSE-MIT, LICENSE-APACHE and README.txt, and its hoprs.exe prints the same version and
imports no VC++ runtime DLL; no builder paths in that exe (-BuilderPaths, default: this machine's %USERPROFILE% and the repository). Prints OK per check, throws
with the first failing one (exit code 1).
#>
param(
    [Parameter(Mandatory)][string]$Msi,
    [Parameter(Mandatory)][string]$Zip,
    [string[]]$BuilderPaths
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path -Parent $PSScriptRoot
if (-not $BuilderPaths) {
    $BuilderPaths = @($env:USERPROFILE, $repo)
}

function Ok([string]$name) { Write-Host "OK    $name" }
function Fail([string]$name, [string]$detail) {
    throw "FAIL  $name$(if ($detail) { ": $detail" })"
}
function Check([string]$name, [bool]$condition, [string]$detail) {
    if ($condition) { Ok $name } else { Fail $name $detail }
}

# [workspace.package] version from the repository's Cargo.toml (what
# scripts/workspace-version.sh prints).
function Get-WorkspaceVersion {
    $section = ''
    foreach ($line in Get-Content (Join-Path $repo 'Cargo.toml')) {
        if ($line -match '^\[') { $section = $line.Trim(); continue }
        if ($section -eq '[workspace.package]' -and $line -match '^version *= *"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw "FAIL  no version in [workspace.package] of $repo\Cargo.toml"
}

# msiexec with the given arguments, waited for; returns its exit code.
function Invoke-Msiexec([string[]]$arguments) {
    Write-Host "      msiexec $($arguments -join ' ')"
    $process = Start-Process -FilePath msiexec.exe -ArgumentList $arguments -Wait -PassThru
    return $process.ExitCode
}

function Show-LogTail([string]$log) {
    if (Test-Path $log) {
        Get-Content $log -Tail 40 | ForEach-Object { Write-Host "      $_" }
    }
}

# Where $needle occurs in the file (ignoring case), each as the printable ASCII string
# around it (up to 5, 300 characters each), to tell which part of the build put it there.
# Latin-1 maps each byte to one character, so IndexOf finds ASCII strings in a binary.
function Find-Strings([string]$file, [string]$needle) {
    $text = [System.Text.Encoding]::Latin1.GetString([System.IO.File]::ReadAllBytes($file))
    $found = @()
    $at = 0
    while ($found.Count -lt 5) {
        $at = $text.IndexOf($needle, $at, [System.StringComparison]::OrdinalIgnoreCase)
        if ($at -lt 0) { break }
        $start = $at
        while ($start -gt 0 -and $text[$start - 1] -ge ' ' -and $text[$start - 1] -le '~') { $start-- }
        $end = $at + $needle.Length
        while ($end -lt $text.Length -and $text[$end] -ge ' ' -and $text[$end] -le '~') { $end++ }
        $found += "'$($text.Substring($start, [Math]::Min($end - $start, 300)))'"
        $at = $end
    }
    return $found
}

# The VC++ runtime DLLs the exe imports: their names (VCRUNTIME140.dll, MSVCP140.dll,
# ...) sit in its import table as plain ASCII. Matched as whole DLL names, so a source
# path such as ...\vcruntime\src\... in the static CRT does not count. The 2015-2022
# redistributable is not on every Windows: package-windows.sh links the CRT statically
# (+crt-static), so there must be none.
function Check-NoVcRuntime([string]$exe, [string]$which) {
    $text = [System.Text.Encoding]::Latin1.GetString([System.IO.File]::ReadAllBytes($exe))
    $hits = @([regex]::Matches($text, '(?i)\b(vcruntime|msvcp)[0-9a-z_]*\.dll\b') |
        ForEach-Object { $_.Value } | Sort-Object -Unique)
    Check "$which imports no VC++ runtime DLL (VCRUNTIME*, MSVCP*)" ($hits.Count -eq 0) "found $($hits -join ', ')"
}

# The exe's --version, through a pipe: a GUI-subsystem program started from PowerShell
# is otherwise not waited for, and its output is lost.
function Get-VersionLine([string]$exe) {
    return (& $exe --version | Out-String).Trim()
}

# Add/Remove Programs entries named hoprs, in both registry views, per machine and per
# user.
function Get-ArpEntries {
    $keys = @(
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    # A function returning an empty array returns $null: callers wrap it in @().
    return @(Get-ItemProperty -Path $keys -ErrorAction SilentlyContinue |
        Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -eq 'hoprs' })
}

$version = Get-WorkspaceVersion
$msiVersion = ($version -split '-', 2)[0]

$Msi = (Resolve-Path $Msi).Path
$Zip = (Resolve-Path $Zip).Path
Check "asset name $(Split-Path -Leaf $Msi)" ((Split-Path -Leaf $Msi) -eq "hoprs_${version}_windows_x64.msi") "expected hoprs_${version}_windows_x64.msi"
Check "asset name $(Split-Path -Leaf $Zip)" ((Split-Path -Leaf $Zip) -eq "hoprs_${version}_windows_x64.zip") "expected hoprs_${version}_windows_x64.zip"

$installDir = Join-Path $env:ProgramFiles 'hoprs'
$installedExe = Join-Path $installDir 'hoprs.exe'
$shortcut = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\hoprs\hoprs.lnk'
# A per-machine install puts the desktop shortcut on the all-users (Public) desktop.
$desktopShortcut = Join-Path ([Environment]::GetFolderPath('CommonDesktopDirectory')) 'hoprs.lnk'
$dataDir = Join-Path $env:APPDATA 'hoprs'
$keep = Join-Path $dataDir 'keep.txt'
$work = Join-Path ([System.IO.Path]::GetTempPath()) "hoprs-check-$([guid]::NewGuid())"

# Before $work is created: nothing to clean up yet if this throws.
if (Test-Path $installedExe) {
    Fail 'no hoprs installed before the check' "$installedExe exists"
}
New-Item -ItemType Directory -Path $work | Out-Null

$createdDataDir = -not (Test-Path $dataDir)
$installed = $false
try {
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    $marker = "kept by the uninstall $([guid]::NewGuid())"
    Set-Content -Path $keep -Value $marker
    Ok "user data before the install: $keep"

    $log = Join-Path $work 'install.log'
    $code = Invoke-Msiexec @('/i', "`"$Msi`"", '/qn', '/l*v', "`"$log`"")
    if ($code -ne 0) { Show-LogTail $log }
    Check 'msiexec /i exits 0' ($code -eq 0) "exit code $code, log $log"
    $installed = $true

    Check "installed $installedExe" (Test-Path $installedExe -PathType Leaf)
    $line = Get-VersionLine $installedExe
    Check "installed hoprs.exe --version prints 'hoprs $version'" ($line -eq "hoprs $version") "got '$line'"
    Check-NoVcRuntime $installedExe 'installed hoprs.exe'

    Check "Start menu shortcut $shortcut" (Test-Path $shortcut -PathType Leaf)
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    Check 'Start menu shortcut targets the installed hoprs.exe' ($target -ieq $installedExe) "targets '$target'"
    Check "desktop shortcut $desktopShortcut" (Test-Path $desktopShortcut -PathType Leaf)
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($desktopShortcut).TargetPath
    Check 'desktop shortcut targets the installed hoprs.exe' ($target -ieq $installedExe) "targets '$target'"

    $entries = @(Get-ArpEntries)
    Check 'one Add/Remove Programs entry' ($entries.Count -eq 1) "found $($entries.Count)"
    Check 'the entry is per machine (HKLM)' ($entries[0].PSPath -like '*HKEY_LOCAL_MACHINE*') $entries[0].PSPath
    Check "the entry's DisplayVersion is $msiVersion" ($entries[0].DisplayVersion -eq $msiVersion) "got '$($entries[0].DisplayVersion)'"

    $log = Join-Path $work 'reinstall.log'
    $code = Invoke-Msiexec @('/i', "`"$Msi`"", 'REINSTALL=ALL', 'REINSTALLMODE=vomus', '/qn', '/l*v', "`"$log`"")
    if ($code -ne 0) { Show-LogTail $log }
    Check 'reinstall (REINSTALL=ALL REINSTALLMODE=vomus) exits 0' ($code -eq 0) "exit code $code, log $log"
    $line = Get-VersionLine $installedExe
    Check 'hoprs.exe --version after the reinstall' ($line -eq "hoprs $version") "got '$line'"
    Check 'still one Add/Remove Programs entry' (@(Get-ArpEntries).Count -eq 1) "found $(@(Get-ArpEntries).Count)"

    $log = Join-Path $work 'uninstall.log'
    $code = Invoke-Msiexec @('/x', "`"$Msi`"", '/qn', '/l*v', "`"$log`"")
    if ($code -ne 0) { Show-LogTail $log }
    Check 'msiexec /x exits 0' ($code -eq 0) "exit code $code, log $log"
    $installed = $false

    Check 'hoprs.exe gone after the uninstall' (-not (Test-Path $installedExe))
    Check 'Start menu shortcut gone after the uninstall' (-not (Test-Path $shortcut))
    Check 'desktop shortcut gone after the uninstall' (-not (Test-Path $desktopShortcut))
    Check 'Add/Remove Programs entry gone after the uninstall' (@(Get-ArpEntries).Count -eq 0) "found $(@(Get-ArpEntries).Count)"
    $kept = (Test-Path $keep) -and ((Get-Content $keep -Raw).Trim() -eq $marker)
    Check "user data kept by the uninstall: $keep" $kept

    $unzipped = Join-Path $work 'zip'
    Expand-Archive -Path $Zip -DestinationPath $unzipped
    $names = @(Get-ChildItem -Path $unzipped -Recurse -Force | ForEach-Object {
            $_.FullName.Substring($unzipped.Length + 1)
        } | Sort-Object)
    $expected = @('hoprs.exe', 'LICENSE-APACHE', 'LICENSE-MIT', 'README.txt') | Sort-Object
    Check '.zip holds hoprs.exe, LICENSE-MIT, LICENSE-APACHE, README.txt' (($names -join ',') -eq ($expected -join ',')) "holds $($names -join ', ')"
    $zipExe = Join-Path $unzipped 'hoprs.exe'
    $line = Get-VersionLine $zipExe
    Check ".zip's hoprs.exe --version prints 'hoprs $version'" ($line -eq "hoprs $version") "got '$line'"

    Check-NoVcRuntime $zipExe ".zip's hoprs.exe"

    # Both separators: rustc and the C compilers record paths either way.
    $hits = @()
    foreach ($path in $BuilderPaths) {
        if (-not $path) { continue }
        foreach ($form in @($path, $path.Replace('\', '/'))) {
            $hits += @(Find-Strings $zipExe $form)
        }
    }
    Check "no builder paths in hoprs.exe ($($BuilderPaths -join ', '))" ($hits.Count -eq 0) "found $($hits -join '; ')"

    Write-Host 'OK    all checks passed'
}
finally {
    if ($installed) {
        Invoke-Msiexec @('/x', "`"$Msi`"", '/qn') | Out-Null
    }
    Remove-Item -Force -ErrorAction SilentlyContinue $keep
    if ($createdDataDir) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $dataDir
    }
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $work
}
