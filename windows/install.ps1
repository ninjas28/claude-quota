<#
.SYNOPSIS
    Builds Claude Quota, installs it, and offers to wire up the status line.

.DESCRIPTION
    The Windows counterpart of ../install.sh. Re-run it any time to update.
    To remove everything it did: .\install.ps1 -Uninstall

    Installs to %LOCALAPPDATA%\ClaudeQuota rather than Program Files: no
    administrator rights are needed, and -- more importantly -- the status line
    command string cannot be quoted (Claude Code runs it through Git Bash or
    PowerShell, and no quoting works in both), so the install path has to be one
    that survives unquoted.

.PARAMETER SkipBuild
    Install the binary that is already built instead of rebuilding.

.PARAMETER SkipStatusLine
    Do not touch ~/.claude/settings.json.

.PARAMETER LaunchAtLogin
    Register under the per-user Run key. You can also toggle this later from the
    tray menu or Settings.

.PARAMETER Uninstall
    Reverse all of it: restore the previous status line, remove the Run entry,
    the Start Menu shortcut, and the install directory.
#>
[CmdletBinding()]
param(
    [switch] $SkipBuild,
    [switch] $SkipStatusLine,
    [switch] $LaunchAtLogin,
    [switch] $Uninstall
)

$ErrorActionPreference = 'Stop'

$InstallDir = Join-Path $env:LOCALAPPDATA 'ClaudeQuota'
$InstalledExe = Join-Path $InstallDir 'claude-quota.exe'
$ShortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Claude Quota.lnk'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunValue = 'ClaudeQuota'

function Invoke-Quota {
    <#
    .SYNOPSIS
        Run a claude-quota subcommand and actually wait for it.

    .DESCRIPTION
        `& $exe args` does NOT wait for a windows-subsystem executable -- it
        returns the moment the process launches, with $LASTEXITCODE 0 whatever
        happens. That is not a style preference: uninstall used to run
        `remove-statusline` and then immediately kill the process, so the
        status line was never removed and nothing was printed to say so.

        Start-Process -Wait is the form that waits, and -NoNewWindow is what
        lets the subcommand's output reach the console.
    #>
    param([Parameter(Mandatory)] [string[]] $Arguments)

    $process = Start-Process $InstalledExe -ArgumentList $Arguments -NoNewWindow -Wait -PassThru
    return $process.ExitCode
}

function Stop-App {
    $running = Get-Process claude-quota -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host "==> Stopping the running copy"
        $running | Stop-Process -Force
        # The file stays locked for a moment after the process goes.
        Start-Sleep -Milliseconds 700
    }
}

if ($Uninstall) {
    Write-Host "==> Uninstalling Claude Quota"

    if (Test-Path $InstalledExe) {
        # Ask the binary to undo its own settings.json edit while it still
        # exists -- it is the only thing that knows what was there before.
        # This has to complete before Stop-App runs, hence Invoke-Quota.
        if ((Invoke-Quota remove-statusline) -ne 0) {
            Write-Warning "Could not restore the previous status line; check ~/.claude/settings.json."
        }
    } else {
        Write-Host "   (binary already gone; leaving settings.json alone)"
    }

    Stop-App

    if (Get-ItemProperty -Path $RunKey -Name $RunValue -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $RunKey -Name $RunValue
        Write-Host "   Removed the launch-at-login entry"
    }
    if (Test-Path $ShortcutPath) {
        Remove-Item $ShortcutPath
        Write-Host "   Removed the Start Menu shortcut"
    }
    if (Test-Path $InstallDir) {
        Remove-Item $InstallDir -Recurse -Force
        Write-Host "   Removed $InstallDir"
    }

    $settings = Join-Path $env:APPDATA 'ClaudeQuota'
    if (Test-Path $settings) {
        Write-Host "   Left your settings at $settings (delete it by hand if you want them gone)"
    }
    Write-Host "Done." -ForegroundColor Green
    return
}

# --- Build ------------------------------------------------------------------

if ($SkipBuild) {
    $built = $InstalledExe
    if (-not (Test-Path $built)) { Write-Error "-SkipBuild given but nothing is installed at $built" }
} else {
    $built = & (Join-Path $PSScriptRoot 'build.ps1') | Select-Object -Last 1
    if (-not (Test-Path $built)) { Write-Error "build did not produce a binary" }
}

# --- Install ----------------------------------------------------------------

Stop-App

if ($built -ne $InstalledExe) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item $built $InstalledExe -Force
    Write-Host "==> Installed to $InstalledExe"
}

if ($InstallDir -like '* *') {
    Write-Warning @"
Your install path contains a space:
  $InstallDir
Claude Code runs the status line through Git Bash or PowerShell, and no quoting
works in both, so the command is written unquoted. The installer will fall back
to the 8.3 short name; if that is unavailable it will tell you and skip.
"@
}

# --- Start Menu shortcut ----------------------------------------------------

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($ShortcutPath)
$shortcut.TargetPath = $InstalledExe
$shortcut.WorkingDirectory = $InstallDir
$shortcut.Description = 'Claude usage in the notification area'
$shortcut.Save()
Write-Host "==> Start Menu shortcut created"

# --- Status line ------------------------------------------------------------

if (-not $SkipStatusLine) {
    Write-Host "==> Status line"
    if ((Invoke-Quota install-statusline) -ne 0) {
        Write-Warning "Could not wire up the status line. The app still works using the usage API."
    }
}

# --- Launch at login --------------------------------------------------------

if ($LaunchAtLogin) {
    New-ItemProperty -Path $RunKey -Name $RunValue -Value "`"$InstalledExe`"" -PropertyType String -Force | Out-Null
    Write-Host "==> Will launch at login"
}

# --- Go ---------------------------------------------------------------------

Start-Process -FilePath $InstalledExe
Write-Host ""
Write-Host "Running." -ForegroundColor Green
Write-Host "Windows 11 hides new tray icons: click the ^ chevron at the left of the"
Write-Host "notification area and drag Claude Quota onto the taskbar to keep it visible."
Write-Host ""
Write-Host "Then send one message in Claude Code. rate_limits only appears after the"
Write-Host "first API response of a session, so there is nothing to show before that."
